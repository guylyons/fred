//! Operators and editing commands.

use super::motion::{self, Motion, word_end, word_fwd};
use crate::buffer::Edit;
use crate::editor::{Editor, Mode, Register};
use crate::text::{floor_grapheme, next_grapheme, prev_grapheme};

/// Largest text a counted put may create.
const MAX_PUT: usize = 16 * 1024 * 1024;

/// A span an operator acts on.
#[derive(Clone, Copy, Debug)]
pub enum Span {
    Lines(usize, usize),
    /// From (line, byte) up to, not including, (line, byte).
    Chars((usize, usize), (usize, usize)),
}

pub fn indent_of(s: &str) -> &str {
    &s[..s.len() - s.trim_start_matches([' ', '\t']).len()]
}

/// Insert `text` at the cursor and move the cursor after it.
pub fn insert_text(ed: &mut Editor, text: &str) {
    let start = ed.buf.pos_to_char(ed.cur.line, ed.cur.byte);
    ed.apply(Edit {
        start,
        end: start,
        text: text.into(),
    });
    let (l, mut b) = ed.buf.char_to_pos(start + text.chars().count());
    // The new text may have merged with the grapheme after it (a letter
    // before a combining mark): put the cursor after the whole cluster.
    let line = ed.buf.line(l);
    if floor_grapheme(&line, b) != b {
        b = next_grapheme(&line, floor_grapheme(&line, b));
    }
    ed.cur.line = l;
    ed.cur.byte = b;
}

/// Delete between two positions; returns the removed text.
pub fn delete_chars(ed: &mut Editor, from: (usize, usize), to: (usize, usize)) -> String {
    let s = ed.buf.pos_to_char(from.0, from.1);
    let e = ed.buf.pos_to_char(to.0, to.1);
    let removed = ed.buf.slice(s, e);
    if s < e {
        ed.apply(Edit {
            start: s,
            end: e,
            text: String::new(),
        });
    }
    removed
}

/// Replace lines `lo..=hi` with `with`.
pub fn splice_lines(ed: &mut Editor, at: usize, remove: usize, with: &[String]) {
    if let Some((edit, _)) = ed.buf.splice_edit(at, remove, with) {
        ed.apply(edit);
    }
}

fn lines_text(ed: &Editor, lo: usize, hi: usize) -> String {
    (lo..=hi)
        .map(|l| ed.buf.line(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The span `op` covers for `motion` (None = whole lines, `dd`).
pub fn span(
    ed: &mut Editor,
    op: char,
    count: Option<usize>,
    motion: Option<Motion>,
) -> Option<Span> {
    let n = ed.line_count();
    let start = ed.cur.pos();
    let Some(motion) = motion else {
        let hi = (start.0 + count.unwrap_or(1).max(1) - 1).min(n - 1);
        return Some(Span::Lines(start.0, hi));
    };
    let line = ed.buf.line(start.0);
    let on_word = start.1 < line.len() && !line[start.1..].starts_with(char::is_whitespace);
    let t = match motion {
        Motion::WordFwd(big) if op == 'c' && on_word => {
            // `cw` changes to the end of the word, like `ce` but never past it.
            let mut p = start;
            for i in 0..count.unwrap_or(1).max(1) {
                let l = ed.buf.line(p.0);
                let nb = next_grapheme(&l, p.1);
                let same = nb < l.len() && {
                    let a = &l[p.1..nb];
                    let b = &l[nb..next_grapheme(&l, nb)];
                    word_class(a, big) == word_class(b, big)
                };
                if i > 0 || same {
                    p = word_end(&ed.buf, p, big);
                }
            }
            motion::Target {
                line: p.0,
                byte: p.1,
                linewise: false,
                inclusive: true,
                keep_col: false,
                eol: false,
            }
        }
        Motion::WordFwd(big) => {
            let mut p = start;
            let steps = count.unwrap_or(1).max(1);
            for _ in 0..steps - 1 {
                p = word_fwd(&ed.buf, p, big).0;
            }
            let (q, crossed) = word_fwd(&ed.buf, p, big);
            // The last word moved over ends the span at its line's end.
            let end = if crossed && q.0 > start.0 || crossed && p.0 == start.0 {
                (p.0, ed.buf.line_len(p.0))
            } else {
                q
            };
            let end = if end < start { start } else { end };
            motion::Target {
                line: end.0,
                byte: end.1,
                linewise: false,
                inclusive: false,
                keep_col: false,
                eol: false,
            }
        }
        m => motion::target(ed, m, count)?,
    };
    if t.linewise {
        return Some(Span::Lines(start.0.min(t.line), start.0.max(t.line)));
    }
    let (mut from, mut to) = if (t.line, t.byte) < start {
        ((t.line, t.byte), start)
    } else {
        (start, (t.line, t.byte))
    };
    if t.inclusive {
        let l = ed.buf.line(to.0);
        to.1 = next_grapheme(&l, to.1);
    } else if to.0 > from.0 && to.1 == 0 {
        // An exclusive motion ending at column 0 stops at the previous line's end.
        to = (to.0 - 1, ed.buf.line_len(to.0 - 1));
        if from.1 <= ed.first_nonblank(from.0) {
            return Some(Span::Lines(from.0, to.0));
        }
    }
    from.1 = floor_grapheme(&ed.buf.line(from.0), from.1);
    Some(Span::Chars(from, to))
}

fn word_class(g: &str, big: bool) -> u8 {
    match g.chars().next() {
        Some(c) if c.is_whitespace() => 0,
        Some(_) if big => 1,
        Some(c) if c.is_alphanumeric() || c == '_' => 2,
        _ => 1,
    }
}

/// Apply operator `op` (d, c, y) to a span.
pub fn apply_op(ed: &mut Editor, op: char, sp: Span) {
    match sp {
        Span::Lines(lo, hi) => {
            set_reg(
                ed,
                Register {
                    text: lines_text(ed, lo, hi),
                    linewise: true,
                },
            );
            match op {
                'y' => {
                    if lo < ed.cur.line {
                        ed.set_cursor(lo, ed.cur.byte);
                    }
                }
                'd' => {
                    splice_lines(ed, lo, hi - lo + 1, &[]);
                    let l = lo.min(ed.line_count() - 1);
                    ed.set_cursor(l, ed.first_nonblank(l));
                }
                _ => {
                    let indent = indent_of(&ed.buf.line(lo)).to_string();
                    splice_lines(ed, lo, hi - lo + 1, std::slice::from_ref(&indent));
                    ed.mode = Mode::Insert;
                    ed.set_cursor(lo, indent.len());
                }
            }
        }
        Span::Chars(from, to) => {
            if op == 'y' {
                let s = ed.buf.pos_to_char(from.0, from.1);
                let e = ed.buf.pos_to_char(to.0, to.1);
                set_reg(
                    ed,
                    Register {
                        text: ed.buf.slice(s, e),
                        linewise: false,
                    },
                );
                ed.set_cursor(from.0, from.1);
                return;
            }
            let removed = delete_chars(ed, from, to);
            set_reg(
                ed,
                Register {
                    text: removed,
                    linewise: false,
                },
            );
            if op == 'c' {
                ed.mode = Mode::Insert;
            }
            ed.cur.line = from.0;
            ed.cur.byte = from.1;
            ed.clamp_cursor();
            let b = ed.cur.byte;
            ed.set_cursor(from.0, b);
        }
    }
}

/// Set the register, and the system clipboard with it.
pub(crate) fn set_reg(ed: &mut Editor, reg: Register) {
    if ed.clipboard {
        crate::clipboard::set(&reg);
    }
    ed.reg = reg;
}

pub fn put(ed: &mut Editor, count: usize, after: bool) {
    // Something copied in another app since our last yank wins.
    if ed.clipboard
        && let Some(c) = crate::clipboard::get()
        && c != crate::clipboard::to_clip(&ed.reg)
    {
        ed.reg = crate::clipboard::from_clip(&c);
    }
    let reg = ed.reg.clone();
    if reg.text.is_empty() && !reg.linewise {
        return;
    }
    let count = count.max(1).min(MAX_PUT / (reg.text.len() + 1)).max(1);
    if reg.linewise {
        let one: Vec<String> = reg.text.split('\n').map(String::from).collect();
        let lines: Vec<String> = std::iter::repeat_n(one, count).flatten().collect();
        let at = if after { ed.cur.line + 1 } else { ed.cur.line };
        splice_lines(ed, at, 0, &lines);
        ed.set_cursor(at, ed.first_nonblank(at));
    } else {
        let text = reg.text.repeat(count);
        let line = ed.buf.line(ed.cur.line);
        let byte = if after && !line.is_empty() {
            next_grapheme(&line, ed.cur.byte)
        } else {
            ed.cur.byte
        };
        ed.cur.byte = byte;
        let start = ed.cur.pos();
        insert_text(ed, &text);
        if text.contains('\n') {
            ed.set_cursor(start.0, start.1);
        } else {
            let l = ed.buf.line(ed.cur.line);
            let b = prev_grapheme(&l, ed.cur.byte);
            ed.set_cursor(ed.cur.line, b);
        }
    }
}

/// `J`: join `count` lines (at least two) starting at `lo`.
pub fn join(ed: &mut Editor, lo: usize, count: usize) -> bool {
    let joins = count.max(2) - 1;
    if lo + 1 >= ed.line_count() {
        return false;
    }
    let mut col = 0;
    for _ in 0..joins {
        if lo + 1 >= ed.line_count() {
            break;
        }
        let cur = ed.buf.line(lo);
        let next = ed.buf.line(lo + 1);
        let next = next.trim_start();
        let sep = if cur.is_empty()
            || next.is_empty()
            || cur.ends_with([' ', '\t'])
            || next.starts_with(')')
        {
            ""
        } else {
            " "
        };
        col = cur.len();
        let joined = format!("{cur}{sep}{next}");
        splice_lines(ed, lo, 2, &[joined]);
    }
    ed.set_cursor(lo, col);
    true
}

/// `r{c}` over `count` graphemes.
pub fn replace_chars(ed: &mut Editor, count: usize, c: char) -> bool {
    let line = ed.buf.line(ed.cur.line);
    let mut end = ed.cur.byte;
    for _ in 0..count.max(1) {
        if end >= line.len() {
            return false;
        }
        end = next_grapheme(&line, end);
    }
    let start = ed.cur.pos();
    delete_chars(ed, start, (start.0, end));
    let text: String = std::iter::repeat_n(c, count.max(1)).collect();
    insert_text(ed, &text);
    let l = ed.buf.line(ed.cur.line);
    let b = prev_grapheme(&l, ed.cur.byte);
    ed.set_cursor(ed.cur.line, b);
    true
}

/// Open a line below (or above) with the current indentation and enter Insert.
pub fn open_line(ed: &mut Editor, below: bool) {
    let indent = indent_of(&ed.buf.line(ed.cur.line)).to_string();
    let at = if below { ed.cur.line + 1 } else { ed.cur.line };
    splice_lines(ed, at, 0, std::slice::from_ref(&indent));
    ed.mode = Mode::Insert;
    ed.cur.line = at;
    ed.cur.byte = indent.len();
}
