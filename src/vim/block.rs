//! Visual-block mode (CTRL-V): the rectangle and what operators do to it,
//! after Neovim's `block_prep()` and the blockwise `op_*` functions.
//! Characters partly in the block (a tab across its edge) count as in it.

use super::ops::{self, splice_lines};
use crate::editor::{Editor, Mode, Register};
use crate::text::{byte_of_col, col_of_byte, display_width, grapheme_width, next_grapheme};

/// The block: lines `lo..=hi`, screen columns `left..=right`, or to each
/// line's end (`eol`, after `$`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub lo: usize,
    pub hi: usize,
    pub left: usize,
    pub right: usize,
    pub eol: bool,
}

/// What Insert mode started by `I`, `A`, `c` or `C` types again on the
/// block's other lines when it ends.
#[derive(Clone, Debug)]
pub(crate) struct Insert {
    lo: usize,
    hi: usize,
    /// The column it goes in on each line.
    col: usize,
    /// Each line's end instead (`$A`, `C`).
    eol: bool,
    /// Short lines get spaces up to the column (`A`); else they're left.
    pad: bool,
    /// Where typing began on the first line, and that line's length then.
    start: usize,
    pre_len: usize,
    /// The block's left column, where the cursor ends up.
    left: usize,
}

/// A position's first and last screen column.
fn cols(ed: &Editor, (l, b): (usize, usize)) -> (usize, usize) {
    let line = ed.buf.line(l);
    let ts = ed.tabstop();
    let s = col_of_byte(&line, b, ts);
    let w = if b < line.len() {
        grapheme_width(&line[b..next_grapheme(&line, b)], s, ts).max(1)
    } else {
        1
    };
    (s, s + w - 1)
}

/// The block being selected, if Visual-block mode is on.
pub fn rect(ed: &Editor) -> Option<Rect> {
    let Mode::Visual { anchor } = ed.mode else {
        return None;
    };
    if !ed.vim.block {
        return None;
    }
    let cur = ed.cur.pos();
    let (a0, a1) = cols(ed, anchor);
    let (c0, c1) = cols(ed, cur);
    Some(Rect {
        lo: anchor.0.min(cur.0),
        hi: anchor.0.max(cur.0),
        left: a0.min(c0),
        right: a1.max(c1),
        eol: ed.cur.want_col == usize::MAX,
    })
}

/// The bytes of `line` in the block; None if the line ends before it.
pub fn part(line: &str, r: &Rect, ts: usize) -> Option<(usize, usize)> {
    if display_width(line, ts, 0) <= r.left {
        return None;
    }
    let mut start = None;
    let mut end = line.len();
    let mut col = 0;
    let mut b = 0;
    while b < line.len() {
        let nb = next_grapheme(line, b);
        let w = grapheme_width(&line[b..nb], col, ts).max(1);
        if start.is_none() && col + w > r.left {
            start = Some(b);
        }
        if !r.eol && col > r.right {
            end = b;
            break;
        }
        col += w;
        b = nb;
    }
    Some((start?, end))
}

/// The block's text on each line (empty for short lines).
fn texts(ed: &Editor, r: &Rect) -> Vec<String> {
    let ts = ed.tabstop();
    (r.lo..=r.hi)
        .map(|l| {
            let line = ed.buf.line(l);
            part(&line, r, ts).map_or_else(String::new, |(a, b)| line[a..b].to_string())
        })
        .collect()
}

fn store(ed: &mut Editor, texts: Vec<String>, yank: bool) {
    let r = Register {
        text: texts.join("\n"),
        linewise: false,
        block: true,
    };
    super::register::store(ed, r, yank, false);
}

/// Replace each line's block part with what `f` makes of it.
fn edit_parts(ed: &mut Editor, r: &Rect, f: impl Fn(&str) -> String) {
    let ts = ed.tabstop();
    let lines: Vec<String> = (r.lo..=r.hi)
        .map(|l| {
            let line = ed.buf.line(l);
            match part(&line, r, ts) {
                Some((a, b)) => format!("{}{}{}", &line[..a], f(&line[a..b]), &line[b..]),
                None => line,
            }
        })
        .collect();
    splice_lines(ed, r.lo, lines.len(), &lines);
}

/// Back to Normal mode at the block's top left.
fn top_left(ed: &mut Editor, r: &Rect) {
    ed.mode = Mode::Normal;
    ed.vim.block = false;
    let line = ed.buf.line(r.lo);
    let b = byte_of_col(&line, r.left, ed.tabstop()).min(line.len());
    ed.set_cursor(r.lo, b);
}

/// A key in Visual-block mode. False if it's not a block command.
pub(crate) fn key(ed: &mut Editor, c: char, count: Option<usize>) -> bool {
    let Some(r) = rect(ed) else {
        return false;
    };
    let edits = !matches!(c, 'y' | 'o' | 'O' | 'v' | 'V');
    if edits && ed.generated() {
        ed.mode = Mode::Normal;
        ed.vim.block = false;
        ed.set_err("generated Git buffer is read-only");
        return true;
    }
    let ts = ed.tabstop();
    match c {
        'v' => ed.vim.block = false,
        'V' => {
            ed.vim.block = false;
            if let Mode::Visual { anchor } = ed.mode {
                ed.mode = Mode::VisualLine { anchor: anchor.0 };
            }
        }
        'o' => {
            if let Mode::Visual { anchor } = ed.mode {
                ed.mode = Mode::Visual {
                    anchor: ed.cur.pos(),
                };
                ed.set_cursor(anchor.0, anchor.1);
            }
        }
        // The other corner on the same line.
        'O' => {
            if let Mode::Visual { anchor } = ed.mode {
                let (ac, _) = cols(ed, anchor);
                let (cc, _) = cols(ed, ed.cur.pos());
                let al = ed.buf.line(anchor.0);
                let new_anchor = (anchor.0, byte_of_col(&al, cc, ts).min(al.len()));
                let cl = ed.buf.line(ed.cur.line);
                let b = byte_of_col(&cl, ac, ts).min(cl.len());
                ed.mode = Mode::Visual { anchor: new_anchor };
                ed.set_cursor(ed.cur.line, b);
            }
        }
        'y' => {
            store(ed, texts(ed, &r), true);
            top_left(ed, &r);
        }
        'd' | 'x' | 'D' => {
            let r = Rect {
                eol: r.eol || c == 'D',
                ..r
            };
            let t = texts(ed, &r);
            ed.undo.begin(ed.cur.pos());
            store(ed, t, false);
            edit_parts(ed, &r, |_| String::new());
            top_left(ed, &r);
            ed.clamp_cursor();
            ed.undo.end(ed.cur.pos());
        }
        'c' | 's' | 'C' => {
            let r = Rect {
                eol: r.eol || c == 'C',
                ..r
            };
            let t = texts(ed, &r);
            ed.undo.begin(ed.cur.pos());
            store(ed, t, false);
            edit_parts(ed, &r, |_| String::new());
            top_left(ed, &r);
            start_insert(ed, &r, r.left, false, false);
        }
        'I' | 'A' => {
            ed.undo.begin(ed.cur.pos());
            let (col, pad) = if c == 'I' {
                (r.left, false)
            } else {
                (r.right + 1, true)
            };
            let eol = c == 'A' && r.eol;
            top_left(ed, &r);
            // `A` on a short first line: spaces up to the column.
            let line = ed.buf.line(r.lo);
            let w = display_width(&line, ts, 0);
            let b = if eol {
                line.len()
            } else if w < col {
                ops::splice_lines(ed, r.lo, 1, &[format!("{line}{}", " ".repeat(col - w))]);
                ed.buf.line_len(r.lo)
            } else {
                byte_of_col(&line, col, ts).min(line.len())
            };
            ed.cur.byte = b;
            start_insert(ed, &r, col, eol, pad);
        }
        'r' => return false,
        '~' | 'u' | 'U' | '?' => {
            let conv = |s: &str| -> String {
                s.chars()
                    .flat_map(|ch| -> Vec<char> {
                        match c {
                            'u' => ch.to_lowercase().collect(),
                            'U' => ch.to_uppercase().collect(),
                            '?' => vec![match ch {
                                'a'..='m' | 'A'..='M' => (ch as u8 + 13) as char,
                                'n'..='z' | 'N'..='Z' => (ch as u8 - 13) as char,
                                ch => ch,
                            }],
                            _ if ch.is_lowercase() => ch.to_uppercase().collect(),
                            _ => ch.to_lowercase().collect(),
                        }
                    })
                    .collect()
            };
            ed.undo.begin(ed.cur.pos());
            edit_parts(ed, &r, conv);
            top_left(ed, &r);
            ed.undo.end(ed.cur.pos());
        }
        '>' | '<' => {
            ed.undo.begin(ed.cur.pos());
            shift(ed, &r, c == '<', count.unwrap_or(1).max(1));
            top_left(ed, &r);
            ed.undo.end(ed.cur.pos());
        }
        'J' => {
            ed.mode = Mode::Normal;
            ed.vim.block = false;
            ed.undo.begin(ed.cur.pos());
            ops::join(ed, r.lo, (r.hi - r.lo + 1).max(2));
            ed.undo.end(ed.cur.pos());
        }
        _ => return false,
    }
    true
}

/// Visual-block `r{c}`.
pub(crate) fn replace(ed: &mut Editor, c: char) {
    let Some(r) = rect(ed) else {
        return;
    };
    ed.undo.begin(ed.cur.pos());
    edit_parts(ed, &r, |s| {
        use unicode_segmentation::UnicodeSegmentation;
        std::iter::repeat_n(c, s.graphemes(true).count()).collect()
    });
    top_left(ed, &r);
    ed.undo.end(ed.cur.pos());
}

/// Shift the text right of the block's left edge by 'shiftwidth's.
fn shift(ed: &mut Editor, r: &Rect, left: bool, times: usize) {
    let (ts, sw) = (ed.tabstop(), ed.shiftwidth());
    let et = ed.opts.bool(crate::options::Opt::Expandtab);
    let lines: Vec<String> = (r.lo..=r.hi)
        .map(|l| {
            let line = ed.buf.line(l);
            let Some((a, _)) = part(&line, r, ts) else {
                return line;
            };
            let ws = line[a..].len() - line[a..].trim_start_matches([' ', '\t']).len();
            if line.len() == a + ws {
                return line;
            }
            let start = col_of_byte(&line, a, ts);
            let end = col_of_byte(&line, a + ws, ts);
            let total = if left {
                (end - start).saturating_sub(sw * times)
            } else {
                end - start + sw * times
            };
            // Tabs only from a tab stop on.
            let pad = if et {
                " ".repeat(total)
            } else {
                let mut s = String::new();
                let mut c = start;
                while c + (ts - c % ts) <= start + total && ts - c % ts > 1 {
                    s.push('\t');
                    c += ts - c % ts;
                }
                s + &" ".repeat(start + total - c)
            };
            format!("{}{pad}{}", &line[..a], &line[a + ws..])
        })
        .collect();
    splice_lines(ed, r.lo, lines.len(), &lines);
}

fn start_insert(ed: &mut Editor, r: &Rect, col: usize, eol: bool, pad: bool) {
    ed.mode = Mode::Insert;
    ed.vim.block_insert = Some(Insert {
        lo: r.lo,
        hi: r.hi,
        col,
        eol,
        pad,
        start: ed.cur.byte,
        pre_len: ed.buf.line_len(ed.cur.line),
        left: r.left,
    });
}

/// Insert mode is ending: type the first line's new text on the others.
pub(crate) fn finish_insert(ed: &mut Editor) -> Option<(usize, usize)> {
    let bi = ed.vim.block_insert.take()?;
    let ts = ed.tabstop();
    let (l, b) = ed.cur.pos();
    if l != bi.lo {
        return None;
    }
    // Afterwards the cursor goes to the block's top left.
    let back = {
        let line = ed.buf.line(bi.lo);
        (bi.lo, byte_of_col(&line, bi.left, ts).min(line.len()))
    };
    // Joined away lines leave nothing to copy into (Vim checks the
    // block's last line still exists).
    if bi.hi >= ed.line_count() {
        return Some(back);
    }
    // As op_insert(): the first line grew by some bytes; that many from
    // where typing began are the text, wherever the cursor is now.
    let first = ed.buf.line(l);
    let n = first.len().checked_sub(bi.pre_len).filter(|&n| n > 0)?;
    // ponytail: Vim copies the bytes even mid-character; Fred skips that.
    let typed = first.get(bi.start..bi.start + n)?.to_string();
    let lines: Vec<String> = (bi.lo + 1..=bi.hi)
        .map(|li| {
            let line = ed.buf.line(li);
            if bi.eol {
                return line + &typed;
            }
            let w = display_width(&line, ts, 0);
            if w < bi.col {
                return if bi.pad {
                    format!("{line}{}{typed}", " ".repeat(bi.col - w))
                } else {
                    line
                };
            }
            let at = byte_of_col(&line, bi.col, ts).min(line.len());
            format!("{}{typed}{}", &line[..at], &line[at..])
        })
        .collect();
    if !lines.is_empty() {
        splice_lines(ed, bi.lo + 1, lines.len(), &lines);
        ed.cur.line = l;
        ed.cur.byte = b;
    }
    Some(back)
}

/// `p` and `P` of a blockwise register: its lines go in at the cursor's
/// column on this and the following lines (added if needed), padded to
/// the block's width where text follows.
pub(crate) fn put(ed: &mut Editor, reg: &Register, count: usize, after: bool) {
    let ts = ed.tabstop();
    let pieces: Vec<&str> = reg.text.split('\n').collect();
    let width = pieces
        .iter()
        .map(|p| display_width(p, ts, 0))
        .max()
        .unwrap_or(0);
    let (l, b) = ed.cur.pos();
    let line = ed.buf.line(l);
    let col = if after && !line.is_empty() {
        cols(ed, (l, b)).1 + 1
    } else {
        col_of_byte(&line, b, ts)
    };
    let mut out = vec![];
    for (i, piece) in pieces.iter().enumerate() {
        let li = l + i;
        let s = if li < ed.line_count() {
            ed.buf.line(li)
        } else {
            String::new()
        };
        let w = display_width(&s, ts, 0);
        let (at, before) = if w < col {
            (s.len(), " ".repeat(col - w))
        } else {
            (byte_of_col(&s, col, ts).min(s.len()), String::new())
        };
        let padded = format!("{piece}{}", " ".repeat(width - display_width(piece, ts, 0)));
        let mut text = padded.repeat(count.max(1));
        if at >= s.len() {
            text = text.trim_end_matches(' ').to_string();
            if text.is_empty() && !piece.is_empty() {
                text = piece.to_string();
            }
        }
        out.push(format!("{}{before}{text}{}", &s[..at], &s[at..]));
    }
    let existing = (ed.line_count() - l).min(out.len());
    splice_lines(ed, l, existing, &out);
    let first = ed.buf.line(l);
    ed.set_cursor(l, byte_of_col(&first, col, ts).min(first.len()));
}
