//! The right-click menu: Neovim's PopUp menu
//! (`runtime/lua/vim/_core/defaults.lua`) with 'mousemodel' "popup_setpos".

use crate::clipboard;
use crate::editor::{Editor, Mode, Register};
use crate::key::{Key, KeyCode};
use crate::vim::{self, ops};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    OpenUrl,
    Definition,
    Separator,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

const N: u8 = 1;
const V: u8 = 2;
const I: u8 = 4;

/// Neovim's items, in its order, with the modes each shows in. Its Inspect,
/// diagnostics and "How-to disable mouse" items need what Fred doesn't
/// have (highlight inspection, LSP, `:help`).
const ITEMS: &[(Item, &str, u8)] = &[
    (Item::OpenUrl, "Open in web browser", N | V | I),
    (Item::Definition, "Go to definition", N | V | I),
    (Item::Separator, "", N | V | I),
    (Item::Cut, "Cut", V),
    (Item::Copy, "Copy", V),
    (Item::Paste, "Paste", N | V | I),
    (Item::Delete, "Delete", V),
    (Item::SelectAll, "Select All", N | V | I),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    pub items: Vec<(Item, &'static str)>,
    /// The highlighted item.
    pub sel: Option<usize>,
    /// The screen cell it opened at (the click).
    pub at: (u16, u16),
    url: Option<String>,
}

/// Open the menu for the cursor's place and mode at screen cell `at`.
pub fn open(ed: &mut Editor, at: (u16, u16)) {
    let mode = match ed.mode {
        Mode::Normal => N,
        Mode::Visual { .. } | Mode::VisualLine { .. } => V,
        Mode::Insert => I,
        _ => return,
    };
    let line = ed.buf.line(ed.cur.line);
    let url = url_at(&line, ed.cur.byte).map(String::from);
    // Like Neovim's, Go to definition needs something to look up.
    let word = crate::pick::def::word_at(&line, ed.cur.byte).is_some()
        && ed.magit.is_none()
        && ed.dired.is_none();
    let mut items: Vec<(Item, &'static str)> = ITEMS
        .iter()
        .filter(|&&(item, _, modes)| {
            modes & mode != 0
                && match item {
                    Item::OpenUrl => url.is_some(),
                    Item::Definition => word,
                    // Generated Git text and directory listings aren't edited here.
                    Item::Cut | Item::Paste | Item::Delete => !ed.generated() && ed.dired.is_none(),
                    _ => true,
                }
        })
        .map(|&(item, label, _)| (item, label))
        .collect();
    // No separator at either end.
    while items.first().is_some_and(|i| i.0 == Item::Separator) {
        items.remove(0);
    }
    while items.last().is_some_and(|i| i.0 == Item::Separator) {
        items.pop();
    }
    ed.menu = (!items.is_empty()).then_some(Menu {
        items,
        sel: None,
        at,
        url,
    });
}

/// A key while the menu is open; false when it closed the menu without
/// using the key.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    let Some(m) = ed.menu.as_mut() else {
        return false;
    };
    let step = |m: &mut Menu, down: bool| {
        let n = m.items.len();
        let mut i = m.sel.unwrap_or(if down { n - 1 } else { 0 });
        loop {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if m.items[i].0 != Item::Separator {
                break;
            }
        }
        m.sel = Some(i);
    };
    match k.code {
        KeyCode::Down | KeyCode::Tab => step(m, true),
        KeyCode::Char('n' | 'j') if k.ctrl => step(m, true),
        KeyCode::Up | KeyCode::BackTab => step(m, false),
        KeyCode::Char('p' | 'k') if k.ctrl => step(m, false),
        KeyCode::Enter => match m.sel {
            Some(i) => {
                let item = m.items[i].0;
                choose(ed, item);
            }
            None => ed.menu = None,
        },
        KeyCode::Esc => ed.menu = None,
        _ if k == Key::ctrl('c') => ed.menu = None,
        _ => {
            ed.menu = None;
            return false;
        }
    }
    true
}

/// Close the menu and do what `item` does.
pub fn choose(ed: &mut Editor, item: Item) {
    let Some(m) = ed.menu.take() else { return };
    match item {
        Item::Separator => {}
        Item::OpenUrl => {
            if let Some(url) = m.url {
                match crate::shell::browse(&url) {
                    Ok(()) => ed.set_msg(format!("Opened {url}")),
                    Err(e) => ed.set_err(e),
                }
            }
        }
        Item::Definition => {
            to_normal(ed);
            ed.handle_key(Key::ch('g'));
            ed.handle_key(Key::ch('d'));
        }
        // `"+x`, `"+y`, `"_x`.
        Item::Cut | Item::Copy | Item::Delete => {
            let Some(span) = vim::selection(ed, &ed.mode) else {
                return;
            };
            ed.mode = Mode::Normal;
            ed.undo.begin(ed.cur.pos());
            match item {
                Item::Cut => ops::apply_op(ed, 'd', span),
                Item::Copy => ops::apply_op(ed, 'y', span),
                _ => ops::delete_black_hole(ed, span),
            }
            ed.undo.end(ed.cur.pos());
            if item != Item::Delete {
                clipboard::set(&ed.reg);
            }
        }
        Item::Paste => paste(ed),
        Item::SelectAll => {
            let last = ed.line_count() - 1;
            if let Mode::Visual { .. } = ed.mode {
                // `gg0oG$`.
                let end = crate::text::prev_grapheme(&ed.buf.line(last), ed.buf.line_len(last));
                ed.mode = Mode::Visual { anchor: (0, 0) };
                ed.set_cursor(last, end);
            } else {
                // `ggVG` (from Insert, after leaving it).
                to_normal(ed);
                ed.mode = Mode::VisualLine { anchor: 0 };
                ed.set_cursor(last, ed.first_nonblank(last));
            }
        }
    }
}

/// Paste: `"+gP` in Normal and Insert mode, `"+P` over a selection. The
/// unnamed register is left as it was.
fn paste(ed: &mut Editor) {
    let Some(text) = clipboard::get() else {
        return ed.set_err("nothing in the clipboard");
    };
    let reg = clipboard::from_clip(&text);
    if reg.text.is_empty() && !reg.linewise {
        return;
    }
    let insert = ed.mode == Mode::Insert;
    if !insert {
        ed.undo.begin(ed.cur.pos());
    }
    match vim::selection(ed, &ed.mode) {
        None => put_before(ed, &reg, None),
        Some(span @ ops::Span::Lines(lo, _)) => {
            ed.mode = Mode::Normal;
            ops::delete_black_hole(ed, span);
            // Even characters go in as lines in place of lines.
            put_before(ed, &reg, Some(lo));
        }
        Some(span @ ops::Span::Chars(from, _)) => {
            ed.mode = Mode::Normal;
            ops::delete_black_hole(ed, span);
            // Where the text was, even past the line's last character.
            (ed.cur.line, ed.cur.byte) = from;
            if reg.linewise {
                // Lines in place of characters split the line around them.
                ops::insert_text(ed, &format!("\n{}\n", reg.text));
                ed.set_cursor(from.0 + 1, 0);
            } else {
                // `P` leaves the cursor on the text's last character.
                ops::insert_text(ed, &reg.text);
                let l = ed.buf.line(ed.cur.line);
                ed.cur.byte = crate::text::prev_grapheme(&l, ed.cur.byte);
            }
        }
    }
    if !insert {
        ed.clamp_cursor();
        ed.undo.end(ed.cur.pos());
    }
}

/// `gP`: put `reg` before the cursor and move just past it; `lines_at`
/// puts it as whole lines above that line.
fn put_before(ed: &mut Editor, reg: &Register, lines_at: Option<usize>) {
    if !reg.linewise && lines_at.is_none() {
        return ops::insert_text(ed, &reg.text);
    }
    let lines: Vec<String> = reg.text.split('\n').map(String::from).collect();
    let at = lines_at.unwrap_or(ed.cur.line);
    // Over every line, the empty one the delete left is replaced.
    let emptied = lines_at.is_some() && ed.line_count() == 1 && ed.buf.line(0).is_empty();
    ops::splice_lines(ed, at, usize::from(emptied), &lines);
    let after = (at + lines.len()).min(ed.line_count() - 1);
    ed.set_cursor(after, 0);
}

/// Leave Insert or Visual mode for Normal.
fn to_normal(ed: &mut Editor) {
    match ed.mode {
        Mode::Insert => vim::insert::leave(ed),
        Mode::Visual { .. } | Mode::VisualLine { .. } => ed.mode = Mode::Normal,
        _ => {}
    }
}

/// The http(s) URL around `byte` in `line`.
pub fn url_at(line: &str, byte: usize) -> Option<&str> {
    let stop = |c: char| c.is_whitespace() || "<>\"'`()[]{}".contains(c);
    let start = line[..byte.min(line.len())].rfind(stop).map_or(0, |i| {
        i + line[i..].chars().next().map_or(1, char::len_utf8)
    });
    let end = line[start..].find(stop).map_or(line.len(), |i| start + i);
    let word = line[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
    (word.starts_with("http://") || word.starts_with("https://")).then_some(word)
}

#[cfg(test)]
mod tests;
