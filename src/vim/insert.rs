//! Insert-mode keys.

use super::ops::{delete_chars, indent_of, insert_text};
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use crate::text::{col_of_byte, next_grapheme, prev_grapheme};

pub fn insert_key(ed: &mut Editor, k: Key) {
    if let Some(r) = ed.vim.recording.as_mut() {
        r.push(k);
    }
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
        KeyCode::Char('w') if k.ctrl => {
            let before = &line[..b];
            let trimmed = before.trim_end_matches([' ', '\t']);
            let word = |c: char| c.is_alphanumeric() || c == '_';
            let start = match trimmed.chars().last() {
                None => 0,
                Some(c) if word(c) => trimmed.trim_end_matches(word).len(),
                Some(c) => trimmed.len() - c.len_utf8(),
            };
            if b > 0 {
                delete_chars(ed, (l, start), (l, b));
                ed.cur.byte = start;
            }
        }
        KeyCode::Char('u') if k.ctrl => {
            delete_chars(ed, (l, 0), (l, b));
            ed.cur.byte = 0;
        }
        KeyCode::Tab => {
            if ed.indent_spaces == 0 {
                insert_text(ed, "\t");
            } else {
                let col = col_of_byte(&line, b, ed.tabstop);
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
    let line = ed.buf.line(ed.cur.line);
    ed.cur.byte = prev_grapheme(&line, ed.cur.byte);
    let b = ed.cur.byte;
    ed.set_cursor(ed.cur.line, b);
    ed.undo.end(ed.cur.pos());
    if let Some(r) = ed.vim.recording.take() {
        ed.vim.last_change = r;
    }
}
