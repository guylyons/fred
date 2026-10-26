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
    if let Some((edit, inserted)) = ed.buf.splice_edit(at, remove, with) {
        ed.apply(edit);
        if with.is_empty() {
            ed.buf.deleted_lines(at, remove, inserted);
        }
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
    ed.vim.numbered_delete = matches!(
        motion,
        Some(
            Motion::Percent
                | Motion::Sentence(_)
                | Motion::SearchNext { .. }
                | Motion::ParaFwd
                | Motion::ParaBack
        )
    );
    let Some(motion) = motion else {
        // `2dd` on the last line fails, as in Vim.
        if count.is_some_and(|c| c > 1) && start.0 + 1 >= n {
            return None;
        }
        let hi = (start.0 + count.unwrap_or(1).max(1) - 1).min(n - 1);
        return Some(Span::Lines(start.0, hi));
    };
    if let Motion::Gn(fwd) = motion {
        let (s, e) = motion::gn_match(ed, fwd)?;
        let l = ed.buf.line(e.0);
        return Some(Span::Chars(s, (e.0, next_grapheme(&l, e.1.min(l.len())))));
    }
    if let Motion::Object { include, kind } = motion {
        let o = super::textobj::object(
            &ed.buf,
            start,
            count.unwrap_or(1),
            include,
            kind,
            None,
            false,
        )?;
        if o.linewise {
            return Some(Span::Lines(o.start.0, o.end.0));
        }
        let (from, mut to) = if o.end < o.start {
            (o.end, o.start)
        } else {
            (o.start, o.end)
        };
        // Tag blocks keep an end in column 0 (Neovim's CA_NO_ADJ_OP_END).
        if kind == 't' {
            if o.inclusive {
                let l = ed.buf.line(to.0);
                to.1 = next_grapheme(&l, to.1.min(l.len()));
            }
            return Some(Span::Chars(from, to));
        }
        return Some(char_span(ed, from, to, o.inclusive));
    }
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
                want: None,
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
                want: None,
            }
        }
        // Neovim's default matchit plugin makes `d50%` characterwise and
        // exclusive.
        Motion::Percent if count.is_some() => {
            let mut t = motion::target(ed, motion, count)?;
            let l = ed.buf.line(t.line);
            t.byte = crate::text::byte_of_col(&l, ed.cur.want_col, ed.tabstop()).min(l.len());
            t.linewise = false;
            t.keep_col = false;
            t.inclusive = false;
            t
        }
        m => motion::target(ed, m, count)?,
    };
    let mut t = t;
    match ed.vim.force {
        Some('V') => return Some(Span::Lines(start.0.min(t.line), start.0.max(t.line))),
        // `v`: a linewise motion becomes exclusive charwise; a charwise one
        // switches inclusive and exclusive.
        Some('v') if t.linewise => {
            t.linewise = false;
            t.inclusive = false;
            if t.keep_col {
                let l = ed.buf.line(t.line);
                t.byte = crate::text::byte_of_col(&l, ed.cur.want_col, ed.tabstop()).min(l.len());
            }
        }
        Some('v') => t.inclusive = !t.inclusive,
        _ => {}
    }
    if t.linewise {
        return Some(Span::Lines(start.0.min(t.line), start.0.max(t.line)));
    }
    let (from, to) = if (t.line, t.byte) < start {
        ((t.line, t.byte), start)
    } else {
        (start, (t.line, t.byte))
    };
    Some(char_span(ed, from, to, t.inclusive))
}

/// A characterwise span. An exclusive one ending in column 0 stops at the
/// previous line's end instead, and becomes linewise if it starts in the
/// indent (as in Vi).
fn char_span(
    ed: &Editor,
    mut from: (usize, usize),
    mut to: (usize, usize),
    inclusive: bool,
) -> Span {
    if inclusive {
        let l = ed.buf.line(to.0);
        to.1 = next_grapheme(&l, to.1.min(l.len()));
    } else if to.0 > from.0 && to.1 == 0 {
        to = (to.0 - 1, ed.buf.line_len(to.0 - 1));
        if from.1 <= ed.first_nonblank(from.0) {
            return Span::Lines(from.0, to.0);
        }
    }
    from.1 = floor_grapheme(&ed.buf.line(from.0), from.1);
    Span::Chars(from, to)
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
    apply(ed, op, sp, true);
}

/// `"_d`: delete into the black hole register, leaving the others alone.
pub fn delete_black_hole(ed: &mut Editor, sp: Span) {
    apply(ed, 'd', sp, false);
}

fn apply(ed: &mut Editor, op: char, sp: Span, to_reg: bool) {
    match op {
        '<' | '>' => {
            let (lo, hi) = match sp {
                Span::Lines(lo, hi) => (lo, hi),
                Span::Chars(a, b) => (a.0, b.0),
            };
            shift(ed, lo, hi, op == '<', 1);
            return;
        }
        '~' | 'u' | 'U' | '?' => {
            change_case(ed, op, sp);
            return;
        }
        'q' | 'w' => {
            let (lo, hi) = match sp {
                Span::Lines(lo, hi) => (lo, hi),
                Span::Chars(a, b) => (a.0, b.0),
            };
            format(ed, lo, hi, op == 'w');
            return;
        }
        _ => {}
    }
    match sp {
        Span::Lines(lo, hi) => {
            if to_reg {
                let r = Register {
                    text: lines_text(ed, lo, hi),
                    linewise: true,
                    block: false,
                };
                super::register::store(ed, r, op == 'y', true);
            }
            match op {
                'y' => {
                    super::marks::set(ed, '[', (lo, 0));
                    let end = ed.last_grapheme(hi);
                    super::marks::set(ed, ']', (hi, end));
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
                if to_reg {
                    let r = Register {
                        text: ed.buf.slice(s, e),
                        linewise: false,
                        block: false,
                    };
                    super::register::store(ed, r, true, false);
                }
                super::marks::set(ed, '[', from);
                let l = ed.buf.line(to.0);
                super::marks::set(ed, ']', (to.0, prev_grapheme(&l, to.1.min(l.len()))));
                ed.set_cursor(from.0, from.1);
                return;
            }
            let removed = delete_chars(ed, from, to);
            if to_reg {
                let r = Register {
                    text: removed,
                    linewise: false,
                    block: false,
                };
                let numbered = ed.vim.numbered_delete;
                super::register::store(ed, r, false, numbered);
            }
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
    if ed.clipboard() && !ed.vim.keep_clipboard {
        crate::clipboard::set(&reg);
    }
    ed.reg = reg;
}

pub fn put(ed: &mut Editor, count: usize, after: bool) {
    put_with(ed, count, after, false);
}

/// `p` and `P`, or `gp` and `gP` (`past`: the cursor goes just after the
/// new text), from the register `"x` named, else the unnamed one.
pub fn put_with(ed: &mut Editor, count: usize, after: bool, past: bool) {
    let named = ed.vim.register.filter(|&c| c != '"');
    let reg = match named {
        Some(c) => match super::register::get(ed, c) {
            Some(r) => r,
            None => {
                ed.set_err(format!("E353: Nothing in register {c}"));
                return;
            }
        },
        None => {
            // Something copied in another app since our last yank wins.
            if ed.clipboard()
                && let Some(c) = crate::clipboard::get()
                && c != crate::clipboard::to_clip(&ed.reg)
            {
                ed.reg = crate::clipboard::from_clip(&c);
            }
            ed.reg.clone()
        }
    };
    if reg.text.is_empty() && !reg.linewise {
        return;
    }
    let count = count.max(1).min(MAX_PUT / (reg.text.len() + 1)).max(1);
    if reg.block {
        return super::block::put(ed, &reg, count, after);
    }
    if reg.linewise {
        let one: Vec<String> = reg.text.split('\n').map(String::from).collect();
        let lines: Vec<String> = std::iter::repeat_n(one, count).flatten().collect();
        let at = if after { ed.cur.line + 1 } else { ed.cur.line };
        let n = lines.len();
        splice_lines(ed, at, 0, &lines);
        if past {
            ed.set_cursor(at + n, 0);
        } else {
            ed.set_cursor(at, ed.first_nonblank(at));
        }
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
        if past {
            let (l, b) = ed.cur.pos();
            ed.set_cursor(l, b);
        } else if text.contains('\n') {
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
    join_with(ed, lo, count, true)
}

/// `J`, or `gJ` (`spaces` false: the lines are joined as they are).
pub fn join_with(ed: &mut Editor, lo: usize, count: usize, spaces: bool) -> bool {
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
        col = cur.len();
        let next = ed.buf.line(lo + 1);
        let joined = if spaces {
            crate::text::join_lines(&cur, &next)
        } else {
            cur.clone() + &next
        };
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

/// `>` and `<`: shift lines `lo..=hi` by `times` 'shiftwidth's. Empty
/// lines stay as they are.
pub fn shift(ed: &mut Editor, lo: usize, hi: usize, left: bool, times: usize) {
    let (sw, ts) = (ed.shiftwidth(), ed.tabstop());
    let et = ed.opts.bool(crate::options::Opt::Expandtab);
    let lines: Vec<String> = (lo..=hi)
        .map(|l| {
            let s = ed.buf.line(l);
            if s.is_empty() {
                return s;
            }
            let ind = indent_of(&s);
            let cols = crate::text::display_width(ind, ts, 0);
            let amount = sw.saturating_mul(times);
            let new = if left {
                cols.saturating_sub(amount)
            } else {
                cols + amount
            };
            let indent = if et {
                " ".repeat(new)
            } else {
                "\t".repeat(new / ts) + &" ".repeat(new % ts)
            };
            indent + &s[ind.len()..]
        })
        .collect();
    splice_lines(ed, lo, hi - lo + 1, &lines);
    // 'nostartofline': keep the remembered column.
    ed.set_line_keep_col(lo);
    ed.clamp_cursor();
    let n = hi - lo + 1;
    if n > 2 {
        let dir = if left { "<" } else { ">" };
        let s = if times == 1 { "" } else { "s" };
        ed.set_msg(format!("{n} lines {dir}ed {times} time{s}"));
    }
}

/// `g~`, `gu`, `gU`, `g?` (named `~ u U ?`) over a span.
fn change_case(ed: &mut Editor, op: char, sp: Span) {
    let conv = |s: &str| -> String {
        s.chars()
            .flat_map(|c| -> Vec<char> {
                match op {
                    'u' => c.to_lowercase().collect(),
                    'U' => c.to_uppercase().collect(),
                    '?' => vec![match c {
                        'a'..='m' | 'A'..='M' => (c as u8 + 13) as char,
                        'n'..='z' | 'N'..='Z' => (c as u8 - 13) as char,
                        c => c,
                    }],
                    _ if c.is_lowercase() => c.to_uppercase().collect(),
                    _ => c.to_lowercase().collect(),
                }
            })
            .collect()
    };
    match sp {
        Span::Lines(lo, hi) => {
            let lines: Vec<String> = (lo..=hi).map(|l| conv(&ed.buf.line(l))).collect();
            let changed = (lo..=hi).zip(&lines).any(|(l, s)| ed.buf.line(l) != *s);
            if changed {
                splice_lines(ed, lo, hi - lo + 1, &lines);
            }
            if lo != ed.cur.line {
                ed.set_line_keep_col(lo);
            } else {
                ed.clamp_cursor();
            }
        }
        Span::Chars(from, to) => {
            let s = ed.buf.pos_to_char(from.0, from.1);
            let e = ed.buf.pos_to_char(to.0, to.1);
            let old = ed.buf.slice(s, e);
            let new = conv(&old);
            if new != old {
                ed.apply(Edit {
                    start: s,
                    end: e,
                    text: new,
                });
            }
            ed.set_cursor(from.0, from.1);
        }
    }
}

/// `~` (without 'tildeop'): switch the case of `count` characters and move
/// past them.
pub fn tilde(ed: &mut Editor, count: usize) -> bool {
    let line = ed.buf.line(ed.cur.line);
    if line.is_empty() {
        return false;
    }
    let start = ed.cur.pos();
    let mut end = start.1;
    for _ in 0..count.max(1) {
        if end >= line.len() {
            break;
        }
        end = next_grapheme(&line, end);
    }
    change_case(ed, '~', Span::Chars(start, (start.0, end)));
    let l = ed.buf.line(start.0);
    let b = if end >= l.len() {
        prev_grapheme(&l, l.len())
    } else {
        end
    };
    ed.set_cursor(start.0, b);
    true
}

/// Neovim's `vim_str2nr()` for CTRL-A: the prefix (`x`, `X`, `b`, `B`, `0`
/// for octal, 0 for decimal), the bytes taken (with a leading `-`), the
/// value and whether it overflowed. `maxlen` 0 means no limit.
fn str2nr(s: &[u8], bin: bool, oct: bool, hex: bool, maxlen: usize) -> (u8, usize, u64, bool) {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut i = 0;
    let mut maxlen = maxlen;
    if at(0) == b'-' {
        i = 1;
        maxlen = maxlen.saturating_sub(1);
    }
    let start = i;
    let mut pre = 0u8;
    if at(i) == b'0' && at(i + 1) != b'8' && at(i + 1) != b'9' && (maxlen == 0 || maxlen > 1) {
        let p = at(i + 1);
        let room = maxlen == 0 || maxlen > 2;
        let hex_pre = hex && matches!(p, b'x' | b'X') && at(i + 2).is_ascii_hexdigit();
        let bin_pre = bin && matches!(p, b'b' | b'B') && matches!(at(i + 2), b'0' | b'1');
        if (hex_pre || bin_pre) && room {
            pre = p;
            i += 2;
        } else if oct {
            // "0", "08" and "0129" are not octal.
            let mut j = i + 1;
            while at(j).is_ascii_digit() {
                if at(j) > b'7' {
                    pre = 0;
                    break;
                }
                pre = b'0';
                j += 1;
            }
        }
    }
    let base: u64 = match pre {
        b'x' | b'X' => 16,
        b'b' | b'B' => 2,
        b'0' => 8,
        _ => 10,
    };
    let limit = if maxlen == 0 {
        usize::MAX
    } else {
        start + maxlen
    };
    let (mut n, mut overflow) = (0u64, false);
    while i < limit {
        let Some(d) = (at(i) as char).to_digit(base as u32) else {
            break;
        };
        match n.checked_mul(base).and_then(|v| v.checked_add(d as u64)) {
            Some(v) => n = v,
            None => {
                n = u64::MAX;
                overflow = true;
            }
        }
        i += 1;
    }
    (pre, i, n, overflow)
}

/// CTRL-A and CTRL-X (`sub`): Neovim's `do_addsub()` on line `l` from byte
/// `col`, within `length` bytes in Visual mode (`linewise`: a Visual-line
/// selection). Returns whether the number changed; in Normal mode the
/// cursor ends on its last character.
pub fn addsub(
    ed: &mut Editor,
    l: usize,
    col: usize,
    length: Option<usize>,
    linewise: bool,
    sub: bool,
    amount: u64,
) -> bool {
    let nf = ed.opts.str(crate::options::Opt::Nrformats);
    let has = |s: &str| nf.split(',').any(|i| i == s);
    let (do_hex, do_oct, do_bin) = (has("hex"), has("octal"), has("bin"));
    let (do_alpha, do_unsigned, do_blank) = (has("alpha"), has("unsigned"), has("blank"));
    let line = ed.buf.line(l);
    let p = line.as_bytes();
    let at = |i: usize| p.get(i).copied().unwrap_or(0);
    let back1 = |mut c: usize| {
        c -= 1;
        while c > 0 && p[c] & 0xc0 == 0x80 {
            c -= 1;
        }
        c
    };
    let bdigit = |b: u8| matches!(b, b'0' | b'1');
    let alpha = |b: u8| do_alpha && b.is_ascii_alphabetic();
    let white = |b: u8| matches!(b, b' ' | b'\t');
    let visual = length.is_some();
    let mut length = length.unwrap_or(0) as isize;
    let orig = col;
    let mut col = col;
    if col >= p.len() {
        return false;
    }
    let (mut negative, mut was_positive, mut blank_unsigned) = (false, true, false);
    if !visual {
        // On a hex or binary number, after its "0x"?
        if do_bin {
            while col > 0 && bdigit(at(col)) {
                col = back1(col);
            }
        }
        if do_hex {
            while col > 0 && at(col).is_ascii_hexdigit() {
                col = back1(col);
            }
        }
        let hexstart = |c: usize| {
            c > 0
                && matches!(at(c), b'x' | b'X')
                && at(c - 1) == b'0'
                && at(c + 1).is_ascii_hexdigit()
        };
        let binstart = |c: usize| {
            c > 0 && matches!(at(c), b'b' | b'B') && at(c - 1) == b'0' && bdigit(at(c + 1))
        };
        if do_bin && do_hex && !hexstart(col) {
            col = orig;
            while col > 0 && at(col).is_ascii_digit() {
                col = back1(col);
            }
        }
        if (do_hex && hexstart(col)) || (do_bin && binstart(col)) {
            col = back1(col);
        } else {
            col = orig;
            while at(col) != 0 && !at(col).is_ascii_digit() && !alpha(at(col)) {
                col += 1;
            }
            while col > 0 && at(col - 1).is_ascii_digit() && !alpha(at(col)) {
                col -= 1;
            }
        }
    } else {
        while at(col) != 0 && length > 0 && !at(col).is_ascii_digit() && !alpha(at(col)) {
            let n = line[col..].chars().next().map_or(1, char::len_utf8);
            col += n;
            length -= n as isize;
        }
        if length <= 0 {
            return false;
        }
        if col > orig && at(col - 1) == b'-' && !do_unsigned {
            if do_blank && col >= 2 && !white(at(col - 2)) {
                blank_unsigned = true;
            } else {
                negative = true;
                was_positive = false;
            }
        }
    }
    let first = at(col);
    if !first.is_ascii_digit() && !alpha(first) {
        ed.vim.failed = true;
        return false;
    }
    if alpha(first) {
        let (lo, hi) = if first.is_ascii_uppercase() {
            (b'A', b'Z')
        } else {
            (b'a', b'z')
        };
        let ord = (first - lo) as u64;
        let new = if sub {
            if ord < amount {
                lo
            } else {
                first - amount as u8
            }
        } else if 25 - ord < amount {
            hi
        } else {
            first + amount as u8
        };
        let s = ed.buf.pos_to_char(l, col);
        ed.apply(Edit {
            start: s,
            end: s + 1,
            text: (new as char).to_string(),
        });
        if !visual {
            ed.set_cursor(l, col);
        }
        return true;
    }
    if col > 0 && at(col - 1) == b'-' && !visual && !do_unsigned {
        if do_blank && col >= 2 && !white(at(col - 2)) {
            blank_unsigned = true;
        } else {
            col -= 1;
            negative = true;
        }
    }
    let maxlen = if visual && !linewise {
        length as usize
    } else {
        0
    };
    let (pre, len, mut n, overflow) = str2nr(&p[col..], do_bin, do_oct, do_hex, maxlen);
    let mut length = len as isize;
    if pre != 0 && negative {
        col += 1;
        length -= 1;
        negative = false;
    }
    let subtract = sub ^ negative;
    let oldn = n;
    if !overflow {
        n = if subtract {
            n.wrapping_sub(amount)
        } else {
            n.wrapping_add(amount)
        };
    }
    if pre == 0 {
        if subtract {
            if n > oldn {
                n = 1u64.wrapping_add(!n);
                negative ^= true;
            }
        } else if n < oldn {
            n = !n;
            negative ^= true;
        }
        if n == 0 {
            negative = false;
        }
    }
    if (do_unsigned || blank_unsigned) && negative {
        n = if subtract { 0 } else { u64::MAX };
        negative = false;
    }
    if visual && !was_positive && !negative && col > 0 {
        col -= 1;
        length += 1;
    }
    let todel = length.max(0) as usize;
    if at(col) == b'-' {
        length -= 1;
    }
    for &c in &p[col..(col + todel).min(p.len())] {
        if c.is_ascii_alphabetic() {
            ed.vim.hexupper = c.is_ascii_uppercase();
        }
    }
    let mut out = String::new();
    if negative && (!visual || was_positive) {
        out.push('-');
    }
    if pre != 0 {
        out.push('0');
        length -= 1;
    }
    if matches!(pre, b'b' | b'B' | b'x' | b'X') {
        out.push(pre as char);
        length -= 1;
    }
    let digits = match pre {
        b'b' | b'B' if n == 0 => String::new(),
        b'b' | b'B' => format!("{n:b}"),
        0 => n.to_string(),
        b'0' => format!("{n:o}"),
        _ if ed.vim.hexupper => format!("{n:X}"),
        _ => format!("{n:x}"),
    };
    length -= digits.len() as isize;
    // Keep the number's width when it had leading zeros.
    if first == b'0' && !(do_oct && pre == 0) {
        for _ in 0..length.max(0) {
            out.push('0');
        }
    }
    out.push_str(&digits);
    let s = ed.buf.pos_to_char(l, col);
    let e = ed.buf.pos_to_char(l, (col + todel).min(p.len()));
    ed.apply(Edit {
        start: s,
        end: e,
        text: out.clone(),
    });
    if !visual {
        ed.set_cursor(l, col + out.len().saturating_sub(1));
    }
    true
}

/// Where to break `line` for 'textwidth' `tw`: the last run of blanks
/// before byte `limit` that leaves at most `tw` columns before it (or, if
/// the first word is longer, the first run). The run's start and end.
pub(crate) fn break_at(line: &str, limit: usize, tw: usize, ts: usize) -> Option<(usize, usize)> {
    let indent = indent_of(line).len();
    let mut runs = vec![];
    let mut i = indent;
    let b = line.as_bytes();
    while i < limit.min(line.len()) {
        if matches!(b[i], b' ' | b'\t') {
            let s = i;
            while i < line.len() && matches!(b[i], b' ' | b'\t') {
                i += 1;
            }
            runs.push((s, i));
        } else {
            i += 1;
        }
    }
    let fits = |s: usize| crate::text::display_width(&line[..s], ts, 0) <= tw;
    runs.iter()
        .rev()
        .find(|r| fits(r.0))
        .or(runs.first())
        .copied()
}

/// `gq` and `gw` (`keep`: the cursor stays): each paragraph of lines
/// `lo..=hi` joined and broken again at 'textwidth' (79 if it is 0), with
/// the first line's indent.
fn format(ed: &mut Editor, lo: usize, hi: usize, keep: bool) {
    let tw = match ed.opts.num(crate::options::Opt::Textwidth) {
        0 => 79,
        n => n as usize,
    };
    let ts = ed.tabstop();
    let saved = ed.cur.pos();
    let hi = hi.min(ed.line_count() - 1);
    let blank = |s: &str| s.trim().is_empty();
    let mut out: Vec<String> = vec![];
    let mut l = lo;
    while l <= hi {
        let line = ed.buf.line(l);
        if blank(&line) {
            out.push(line);
            l += 1;
            continue;
        }
        let mut joined = line;
        l += 1;
        while l <= hi && !blank(&ed.buf.line(l)) {
            joined = crate::text::join_lines(&joined, &ed.buf.line(l));
            l += 1;
        }
        let indent = indent_of(&joined).to_string();
        loop {
            if crate::text::display_width(&joined, ts, 0) <= tw {
                break;
            }
            let Some((s, e)) = break_at(&joined, joined.len(), tw, ts) else {
                break;
            };
            if e >= joined.len() {
                joined.truncate(s);
                break;
            }
            out.push(joined[..s].to_string());
            joined = indent.clone() + &joined[e..];
        }
        out.push(joined);
    }
    let n = out.len();
    splice_lines(ed, lo, hi - lo + 1, &out);
    if keep {
        let l = saved.0.min(ed.line_count() - 1);
        ed.set_cursor(l, saved.1);
    } else {
        let last = lo + n - 1;
        ed.set_cursor(last, ed.first_nonblank(last));
    }
}

/// `]p` (`after`), `[p`, `[P`, `]P`: put a linewise register with its
/// indent changed to the cursor line's (others as `p` and `P`).
pub fn put_indented(ed: &mut Editor, count: usize, after: bool) {
    let reg = match ed.vim.register.filter(|&c| c != '"') {
        Some(c) => super::register::get(ed, c),
        None => Some(ed.reg.clone()),
    };
    let Some(reg) = reg.filter(|r| r.linewise) else {
        return put(ed, count, after);
    };
    let ts = ed.tabstop();
    let et = ed.opts.bool(crate::options::Opt::Expandtab);
    let want = crate::text::display_width(indent_of(&ed.buf.line(ed.cur.line)), ts, 0);
    let lines: Vec<&str> = reg.text.split('\n').collect();
    let first = lines
        .first()
        .map_or(0, |l| crate::text::display_width(indent_of(l), ts, 0));
    let indent = |cols: usize| {
        if et {
            " ".repeat(cols)
        } else {
            "\t".repeat(cols / ts) + &" ".repeat(cols % ts)
        }
    };
    let one: Vec<String> = lines
        .iter()
        .map(|l| {
            if l.is_empty() {
                return String::new();
            }
            let cols = crate::text::display_width(indent_of(l), ts, 0);
            indent((cols + want).saturating_sub(first)) + l.trim_start_matches([' ', '\t'])
        })
        .collect();
    let all: Vec<String> = std::iter::repeat_n(one, count.max(1)).flatten().collect();
    let at = if after { ed.cur.line + 1 } else { ed.cur.line };
    splice_lines(ed, at, 0, &all);
    ed.set_cursor(at, ed.first_nonblank(at));
}
