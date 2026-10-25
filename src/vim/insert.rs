//! Insert-mode keys.

use super::ops::{delete_chars, indent_of, insert_text};
use crate::complete::{self, Popup, index::is_word_char};
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use crate::text::{col_of_byte, next_grapheme, prev_grapheme, word_start_before};

pub fn insert_key(ed: &mut Editor, k: Key) {
    if super::select::insert_key(ed, k) {
        return;
    }
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
        match ed.nearby.words().lock() {
            Ok(nearby) => complete::candidates(&text, &mut ed.word_index, l, &nearby),
            Err(_) => complete::candidates(&text, &mut ed.word_index, l, &[]),
        }
    };
    ed.popup = (!items.is_empty()).then_some(Popup {
        items,
        sel: None,
        start,
        typed: text,
    });
}

/// Replace mode: type `c` over the character under the cursor.
fn replace_char(ed: &mut Editor, c: char) {
    let (l, b) = ed.cur.pos();
    let line = ed.buf.line(l);
    let old = (b < line.len()).then(|| line[b..next_grapheme(&line, b)].to_string());
    if old.is_some() {
        delete_chars(ed, (l, b), (l, next_grapheme(&line, b)));
        ed.cur.byte = b;
    }
    let mut s = [0u8; 4];
    insert_text(ed, c.encode_utf8(&mut s));
    if let Some(stack) = ed.vim.replace.as_mut() {
        stack.push(old);
    }
}

/// An Insert-mode key that takes the next key too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    /// CTRL-R: a register's text.
    Reg,
    /// CTRL-V: the next key literally, or a character by its code.
    Literal(Literal),
    /// CTRL-G.
    CtrlG,
}

/// CTRL-O ran its command: back to Insert mode.
pub(crate) fn after_ctrl_o(ed: &mut Editor) {
    if ed.mode != Mode::Normal || !ed.vim.pending.is_empty() {
        return;
    }
    let Some((eol, line)) = ed.vim.insert_once.take() else {
        return;
    };
    // Back after the last character if Insert mode was there (on the same
    // line) or the command wants a column further right (`$`).
    let (l, b) = ed.cur.pos();
    let s = ed.buf.line(l);
    let col = col_of_byte(&s, b, ed.tabstop());
    if b < s.len()
        && next_grapheme(&s, b) == s.len()
        && ((eol && l == line) || ed.cur.want_col > col)
    {
        ed.cur.byte = s.len();
    }
    ed.mode = Mode::Insert;
    ed.undo.begin(ed.cur.pos());
}

/// The key after CTRL-R, CTRL-V or CTRL-G.
fn pending_key(ed: &mut Editor, p: Pending, k: Key) {
    match p {
        Pending::Reg => {
            let c = match k.code {
                // CTRL-R CTRL-R, CTRL-O, CTRL-P {reg}: the same here.
                KeyCode::Char('r' | 'o' | 'p') if k.ctrl => {
                    ed.vim.ins_pending = Some(Pending::Reg);
                    return;
                }
                KeyCode::Char(c) if !k.ctrl && !k.alt => c,
                _ => return,
            };
            if let Some(r) = super::register::get(ed, c) {
                let text = if r.linewise {
                    format!("{}\n", r.text)
                } else {
                    r.text
                };
                insert_text(ed, &text);
            }
        }
        Pending::Literal(st) => literal_key(ed, st, k),
        Pending::CtrlG => {
            let (l, _) = ed.cur.pos();
            let col = col_of_byte(
                &ed.buf.line(ed.vim.insert_start.0.min(ed.line_count() - 1)),
                ed.vim.insert_start.1,
                ed.tabstop(),
            );
            let to = |ed: &mut Editor, nl: usize| {
                let s = ed.buf.line(nl);
                ed.cur.line = nl;
                ed.cur.byte = crate::text::byte_of_col(&s, col, ed.tabstop());
                ed.vim.insert_start = ed.cur.pos();
            };
            match (k.code, k.ctrl) {
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) if l + 1 < ed.line_count() => {
                    to(ed, l + 1)
                }
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) if l > 0 => to(ed, l - 1),
                // A new undoable change starts here.
                (KeyCode::Char('u'), false) => {
                    let p = ed.cur.pos();
                    ed.undo.end(p);
                    ed.undo.begin(p);
                }
                _ => {}
            }
        }
    }
}

/// CTRL-V's state, as Neovim's `get_literal()`: `x`, `o`, `u` or `U`
/// (typed at any point) pick the base; the code so far and its digits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Literal {
    hex: bool,
    octal: bool,
    unicode: Option<char>,
    cc: u32,
    digits: usize,
}

/// A key after CTRL-V.
/// What a key after CTRL-V did.
pub(crate) enum LiteralStep {
    /// More digits may come.
    More(Literal),
    /// The text to insert, then a key to handle as usual.
    Done(String, Option<Key>),
}

impl Literal {
    /// The next key after CTRL-V (Insert mode and the command line).
    pub(crate) fn step(mut self, k: Key) -> LiteralStep {
        let mut stop = None;
        match k.char() {
            None => stop = Some(k),
            Some('x' | 'X') => self.hex = true,
            Some('o' | 'O') => self.octal = true,
            Some(u @ ('u' | 'U')) => self.unicode = Some(u),
            Some(c) => {
                let base = if self.hex || self.unicode.is_some() {
                    16
                } else if self.octal {
                    8
                } else {
                    10
                };
                match c.to_digit(base) {
                    Some(d) => {
                        self.cc = self.cc.saturating_mul(base).saturating_add(d);
                        self.digits += 1;
                    }
                    None => stop = Some(k),
                }
            }
        }
        if stop.is_none() {
            if self.cc > 255 && self.unicode.is_none() {
                self.cc = 255;
            }
            let done = match self.unicode {
                _ if self.hex => self.digits >= 2,
                Some('u') => self.digits >= 4,
                Some(_) => self.digits >= 8,
                None => self.digits >= 3,
            };
            if !done {
                return LiteralStep::More(self);
            }
        }
        if self.digits == 0 {
            // No code: the key itself.
            let text = stop.map_or_else(String::new, |k| super::register::keys_to_text(&[k]));
            return LiteralStep::Done(text, None);
        }
        let text = char::from_u32(self.cc)
            .filter(|&c| c != '\0')
            .map_or_else(String::new, String::from);
        LiteralStep::Done(text, stop)
    }
}

fn literal_key(ed: &mut Editor, st: Literal, k: Key) {
    match st.step(k) {
        LiteralStep::More(st) => ed.vim.ins_pending = Some(Pending::Literal(st)),
        LiteralStep::Done(text, then) => {
            insert_text(ed, &text);
            // The key that ended the code does what it does.
            if let Some(k) = then {
                edit_key(ed, k);
            }
        }
    }
}

/// CTRL-T and CTRL-D: shift the line one 'shiftwidth', keeping the cursor
/// on the same text.
fn shift_line(ed: &mut Editor, left: bool) {
    let (l, b) = ed.cur.pos();
    let before = ed.buf.line_len(l);
    super::ops::shift(ed, l, l, left, 1);
    let after = ed.buf.line_len(l);
    ed.cur.line = l;
    ed.cur.byte = (b + after).saturating_sub(before).min(after);
    let ind = indent_of(&ed.buf.line(l)).len();
    if b < before && ed.cur.byte < ind {
        ed.cur.byte = ind;
    }
    ed.msg = None;
}

fn edit_key(ed: &mut Editor, k: Key) {
    if let Some(p) = ed.vim.ins_pending.take() {
        return pending_key(ed, p, k);
    }
    let (l, b) = ed.cur.pos();
    let line = ed.buf.line(l);
    if k.ctrl && !k.alt {
        match k.code {
            KeyCode::Char('r') => return ed.vim.ins_pending = Some(Pending::Reg),
            KeyCode::Char('v' | 'q') => {
                return ed.vim.ins_pending = Some(Pending::Literal(Literal::default()));
            }
            KeyCode::Char('g') => return ed.vim.ins_pending = Some(Pending::CtrlG),
            // The last inserted text (CTRL-@ also leaves Insert mode).
            KeyCode::Char(c @ ('a' | '@' | ' ')) => {
                let text = ed.vim.last_insert.clone();
                insert_text(ed, &text);
                if c != 'a' {
                    leave(ed);
                }
                return;
            }
            KeyCode::Char('t') => return shift_line(ed, false),
            KeyCode::Char('d') => {
                // `0 CTRL-D` and `^ CTRL-D` delete all the indent.
                let typed = &line[..b];
                let rest = typed.trim_start_matches([' ', '\t']);
                if rest == "0" || rest == "^" {
                    delete_chars(ed, (l, 0), (l, b));
                    ed.cur.byte = 0;
                    return;
                }
                return shift_line(ed, true);
            }
            // The character below or above the cursor.
            KeyCode::Char(c @ ('e' | 'y')) => {
                let other = if c == 'e' { l + 1 } else { l.wrapping_sub(1) };
                if other < ed.line_count() {
                    let s = ed.buf.line(other);
                    let col = col_of_byte(&line, b, ed.tabstop());
                    let ob = crate::text::byte_of_col(&s, col, ed.tabstop());
                    if ob < s.len() && col_of_byte(&s, ob, ed.tabstop()) == col {
                        let g = s[ob..next_grapheme(&s, ob)].to_string();
                        insert_text(ed, &g);
                    }
                }
                return;
            }
            KeyCode::Char('j' | 'm') => return edit_key(ed, Key::new(KeyCode::Enter)),
            KeyCode::Char('i') => return edit_key(ed, Key::new(KeyCode::Tab)),
            KeyCode::Char('[') => return edit_key(ed, Key::new(KeyCode::Esc)),
            // One Normal-mode command, then back.
            KeyCode::Char('o') => {
                let eol = b >= line.len() && !line.is_empty();
                ed.undo.end(ed.cur.pos());
                ed.mode = Mode::Normal;
                ed.vim.after_j = false;
                if eol {
                    ed.cur.byte = prev_grapheme(&line, b);
                }
                ed.vim.insert_once = Some((eol, l));
                return;
            }
            KeyCode::Left => {
                let p = super::motion::word_back(&ed.buf, (l, b), false);
                return ed.set_cursor(p.0, p.1);
            }
            KeyCode::Right => {
                let p = super::motion::word_fwd(&ed.buf, (l, b), false).0;
                ed.cur.line = p.0;
                ed.cur.byte = p.1;
                return;
            }
            KeyCode::Home => return ed.set_cursor(0, 0),
            KeyCode::End => {
                let last = ed.line_count() - 1;
                ed.cur.line = last;
                ed.cur.byte = ed.buf.line_len(last);
                return;
            }
            _ => {}
        }
    }
    if k.shift && !k.ctrl {
        match k.code {
            KeyCode::Left => return edit_key(ed, Key::with(KeyCode::Left, "C")),
            KeyCode::Right => return edit_key(ed, Key::with(KeyCode::Right, "C")),
            KeyCode::Up => return edit_key(ed, Key::new(KeyCode::PageUp)),
            KeyCode::Down => return edit_key(ed, Key::new(KeyCode::PageDown)),
            _ => {}
        }
    }
    if ed.vim.replace.is_some() {
        match k.code {
            KeyCode::Char(c) if !k.ctrl && !k.alt => return replace_char(ed, c),
            KeyCode::Backspace | KeyCode::Char('h') if k.code == KeyCode::Backspace || k.ctrl => {
                match ed.vim.replace.as_mut().and_then(Vec::pop) {
                    // Before the replaced text: only move left.
                    None => {
                        if b > 0 {
                            ed.cur.byte = prev_grapheme(&line, b);
                        }
                        return;
                    }
                    // Put back what the character replaced.
                    Some(Some(old)) => {
                        let p = prev_grapheme(&line, b);
                        delete_chars(ed, (l, p), (l, b));
                        ed.cur.byte = p;
                        insert_text(ed, &old);
                        ed.cur.byte = p;
                        return;
                    }
                    // It was added: delete it, as in Insert mode.
                    Some(None) => {}
                }
            }
            KeyCode::Enter => {
                if let Some(stack) = ed.vim.replace.as_mut() {
                    stack.push(None);
                }
            }
            _ => {}
        }
    }
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
            if !ed.opts.bool(crate::options::Opt::Expandtab) {
                insert_text(ed, "\t");
            } else {
                let sw = ed.shiftwidth();
                let col = col_of_byte(&line, b, ed.tabstop());
                insert_text(ed, &" ".repeat(sw - col % sw));
            }
        }
        KeyCode::Left => ed.set_cursor(l, prev_grapheme(&line, b)),
        KeyCode::Right => ed.set_cursor(l, next_grapheme(&line, b)),
        KeyCode::Home => ed.set_cursor(l, 0),
        KeyCode::End => ed.set_cursor(l, line.len()),
        KeyCode::Up if l > 0 => ed.set_line_keep_col(l - 1),
        KeyCode::Down => ed.set_line_keep_col(l + 1),
        KeyCode::PageUp => ed.set_line_keep_col(l.saturating_sub(ed.win_height.max(1))),
        KeyCode::PageDown => ed.set_line_keep_col(l + ed.win_height.max(1)),
        KeyCode::Char(c) if !k.ctrl && !k.alt => {
            let mut s = [0u8; 4];
            insert_text(ed, c.encode_utf8(&mut s));
            if !c.is_whitespace() {
                auto_wrap(ed);
            }
        }
        _ => {}
    }
}

/// 'textwidth': typing past it breaks the line at a blank before it.
fn auto_wrap(ed: &mut Editor) {
    let tw = ed.opts.num(crate::options::Opt::Textwidth) as usize;
    if tw == 0 {
        return;
    }
    let ts = ed.tabstop();
    loop {
        let (l, b) = ed.cur.pos();
        let line = ed.buf.line(l);
        if crate::text::display_width(&line[..b], ts, 0) <= tw {
            return;
        }
        let Some((s, e)) = super::ops::break_at(&line, b, tw, ts) else {
            return;
        };
        if e > b {
            return;
        }
        let indent = indent_of(&line).to_string();
        delete_chars(ed, (l, s), (l, e));
        ed.cur.line = l;
        ed.cur.byte = s;
        insert_text(ed, &format!("\n{indent}"));
        let nb = ed.cur.byte + (b - e);
        ed.cur.byte = nb;
    }
}

/// Leave Insert mode: type the text again for a count, close the undo
/// group and finish dot-repeat recording.
pub fn leave(ed: &mut Editor) {
    if let Some((cmd, n)) = ed.vim.insert_repeat.take() {
        let keys = match &ed.vim.recording {
            Some(r) => r[ed.vim.recording_split.min(r.len())..].to_vec(),
            None if ed.vim.replaying => {
                let k = &ed.vim.last_change;
                k[ed.vim.last_change_split.unwrap_or(k.len()).min(k.len())..].to_vec()
            }
            None => vec![],
        };
        let text = super::typed_text(&keys);
        for _ in 1..n {
            if matches!(cmd, 'o' | 'O') {
                super::ops::open_line(ed, true);
                insert_text(ed, &text);
            } else if cmd == 'R' {
                for c in text.chars() {
                    replace_char(ed, c);
                }
            } else {
                insert_text(ed, &text);
            }
        }
    }
    ed.vim.replace = None;
    ed.vim.insert_stop = Some(ed.cur.pos());
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
