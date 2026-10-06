//! Insert-mode keys.

use super::ops::{delete_chars, indent_of, insert_text};
use crate::complete::{self, Popup, index::is_word_char};
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use crate::text::{col_of_byte, next_grapheme, prev_grapheme, word_start_before};

pub fn insert_key(ed: &mut Editor, k: Key) {
    if let Some(r) = ed.vim.recording.as_mut() {
        r.push(k);
    }
    let is_j = k == Key::ch('j');
    if std::mem::replace(&mut ed.vim.after_j, is_j) && is_j {
        // `jj` = Esc: drop the first `j` (still just before the cursor).
        let (l, b) = ed.cur.pos();
        if ed.buf.line(l)[..b].ends_with('j') {
            delete_chars(ed, (l, b - 1), (l, b));
            ed.cur.byte = b - 1;
            ed.popup = None;
            leave(ed);
            return;
        }
    }
    if popup_key(ed, k) {
        return;
    }
    edit_key(ed, k);
    let typed = match k.code {
        KeyCode::Char(c) if !k.ctrl && !k.alt => {
            is_word_char(c) || matches!(c, '/' | '.' | '~' | '-')
        }
        KeyCode::Backspace => true,
        _ => false,
    };
    if typed && ed.opts.bool(crate::options::Opt::Autocomplete) && ed.mode == Mode::Insert {
        refresh_popup(ed, false);
    } else {
        ed.popup = None;
    }
}

/// Keys that drive an open popup (or open one with Ctrl-N/Ctrl-P).
fn popup_key(ed: &mut Editor, k: Key) -> bool {
    let next = k == Key::new(KeyCode::Tab) || k == Key::ctrl('n');
    let prev = k == Key::new(KeyCode::BackTab) || k == Key::ctrl('p');
    if next || prev {
        if ed.popup.is_none() {
            if k.code == KeyCode::Tab {
                return false;
            }
            refresh_popup(ed, true);
            if ed.popup.is_none() {
                return true;
            }
        }
        cycle(ed, next);
        return true;
    }
    if k.is(KeyCode::Enter)
        && let Some(p) = &ed.popup
    {
        let accepted = p.sel.is_some();
        ed.popup = None;
        return accepted;
    }
    false
}

fn cycle(ed: &mut Editor, next: bool) {
    let Some(p) = ed.popup.as_mut() else { return };
    let n = p.items.len();
    p.sel = match (p.sel, next) {
        (None, true) => Some(0),
        (Some(i), true) => (i + 1 < n).then_some(i + 1),
        (None, false) => Some(n - 1),
        (Some(0), false) => None,
        (Some(i), false) => Some(i - 1),
    };
    let text = p
        .sel
        .map_or_else(|| p.typed.clone(), |i| p.items[i].clone());
    let start = p.start;
    let l = ed.cur.line;
    delete_chars(ed, (l, start), (l, ed.cur.byte));
    ed.cur.byte = start;
    insert_text(ed, &text);
}

/// Recompute completions for the token before the cursor.
fn refresh_popup(ed: &mut Editor, manual: bool) {
    let (l, b) = ed.cur.pos();
    let line = ed.buf.line(l);
    let Some((start, text, is_path)) = complete::token_before(&line, b) else {
        ed.popup = None;
        return;
    };
    if !manual && text.chars().count() < 2 {
        ed.popup = None;
        return;
    }
    let items: Vec<String> = if is_path {
        let cwd = std::env::current_dir().unwrap_or_default();
        complete::path::complete(&text, &cwd)
            .into_iter()
            .filter(|p| *p != text)
            .take(complete::MAX_ITEMS)
            .collect()
    } else {
        ed.word_index.ensure(&ed.buf, false);
        match ed.nearby.lock() {
            Ok(nearby) => complete::candidates(&text, &ed.word_index, l, &nearby),
            Err(_) => complete::candidates(&text, &ed.word_index, l, &[]),
        }
    };
    ed.popup = (!items.is_empty()).then_some(Popup {
        items,
        sel: None,
        start,
        typed: text,
    });
}

fn edit_key(ed: &mut Editor, k: Key) {
    let (l, b) = ed.cur.pos();
    let line = ed.buf.line(l);
    match k.code {
        KeyCode::Esc | KeyCode::Char('c') if k.code == KeyCode::Esc || k.ctrl => leave(ed),
        KeyCode::Enter => {
            let indent = indent_of(&line[..b]).to_string();
            insert_text(ed, &format!("\n{indent}"));
        }
        KeyCode::Backspace | KeyCode::Char('h') if k.code == KeyCode::Backspace || k.ctrl => {
            if b > 0 {
                delete_chars(ed, (l, prev_grapheme(&line, b)), (l, b));
                ed.cur.byte = prev_grapheme(&line, b);
            } else if l > 0 {
                let pl = ed.buf.line_len(l - 1);
                delete_chars(ed, (l - 1, pl), (l, 0));
                ed.cur.line = l - 1;
                ed.cur.byte = pl;
            }
        }
        KeyCode::Delete => {
            if b < line.len() {
                delete_chars(ed, (l, b), (l, next_grapheme(&line, b)));
            } else if l + 1 < ed.line_count() {
                delete_chars(ed, (l, b), (l + 1, 0));
            }
        }
        KeyCode::Char('w' | 'u') if k.ctrl => {
            let mut start = match k.code {
                KeyCode::Char('w') => word_start_before(&line, b),
                // CTRL-U keeps the indent, unless already back at it.
                _ => match indent_of(&line).len() {
                    i if i < b => i,
                    _ => 0,
                },
            };
            // Typed text goes first: stop where this insert began.
            let (sl, sb) = ed.vim.insert_start;
            if sl == l && start < sb && sb < b {
                start = sb;
            }
            if start < b {
                delete_chars(ed, (l, start), (l, b));
                ed.cur.byte = start;
            }
        }
        KeyCode::Tab => {
            if ed.indent_spaces == 0 {
                insert_text(ed, "\t");
            } else {
                let col = col_of_byte(&line, b, ed.tabstop());
                let n = ed.indent_spaces - col % ed.indent_spaces;
                insert_text(ed, &" ".repeat(n));
            }
        }
        KeyCode::Left => ed.set_cursor(l, prev_grapheme(&line, b)),
        KeyCode::Right => ed.set_cursor(l, next_grapheme(&line, b)),
        KeyCode::Home => ed.set_cursor(l, 0),
        KeyCode::End => ed.set_cursor(l, line.len()),
        KeyCode::Up if l > 0 => ed.set_line_keep_col(l - 1),
        KeyCode::Down => ed.set_line_keep_col(l + 1),
        KeyCode::Char(c) if !k.ctrl && !k.alt => {
            let mut s = [0u8; 4];
            insert_text(ed, c.encode_utf8(&mut s));
        }
        _ => {}
    }
}

/// Leave Insert mode: close the undo group and finish dot-repeat recording.
pub fn leave(ed: &mut Editor) {
    ed.mode = Mode::Normal;
    ed.vim.after_j = false;
    let line = ed.buf.line(ed.cur.line);
    ed.cur.byte = prev_grapheme(&line, ed.cur.byte);
    let b = ed.cur.byte;
    ed.set_cursor(ed.cur.line, b);
    ed.undo.end(ed.cur.pos());
    if let Some(r) = ed.vim.recording.take() {
        ed.vim.finish_recording(r);
    }
}
