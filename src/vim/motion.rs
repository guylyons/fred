//! vim motions: where the cursor goes, and how an operator treats the span.

use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::text::{next_grapheme, prev_grapheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordFwd(bool),
    WordBack(bool),
    WordEnd(bool),
    LineStart,
    FirstNonBlank,
    LineEnd,
    /// `G`: last line, or line N with a count.
    GotoLine,
    /// `gg`: first line, or line N with a count.
    FileStart,
    NextLine,
    PrevLine,
    ParaFwd,
    ParaBack,
    Find {
        kind: char,
        ch: char,
    },
    RepeatFind {
        reverse: bool,
    },
    SearchNext {
        reverse: bool,
    },
    Mark(char),
}

/// Where a motion lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub line: usize,
    pub byte: usize,
    pub linewise: bool,
    pub inclusive: bool,
    /// j/k: keep the remembered column.
    pub keep_col: bool,
    /// `$`: remember "end of line" as the column.
    pub eol: bool,
}

impl Target {
    fn at(line: usize, byte: usize) -> Target {
        Target {
            line,
            byte,
            linewise: false,
            inclusive: false,
            keep_col: false,
            eol: false,
        }
    }
    fn inclusive(mut self) -> Target {
        self.inclusive = true;
        self
    }
    fn linewise(mut self) -> Target {
        self.linewise = true;
        self
    }
}

/// Word class: 0 blank, 1 punctuation, 2 word chars (`big` merges 1 and 2).
fn class(g: &str, big: bool) -> u8 {
    match g.chars().next() {
        None => 0,
        Some(c) if c.is_whitespace() => 0,
        Some(_) if big => 1,
        Some(c) if c.is_alphanumeric() || c == '_' => 2,
        Some(_) => 1,
    }
}

fn grapheme_at(line: &str, b: usize) -> &str {
    &line[b..next_grapheme(line, b)]
}

/// One `w` step. Returns the new position and whether it crossed a line.
pub fn word_fwd(buf: &Buffer, (mut l, mut b): (usize, usize), big: bool) -> ((usize, usize), bool) {
    let n = buf.len_lines();
    let mut line = buf.line(l);
    let start_line = l;
    if b < line.len() {
        let c = class(grapheme_at(&line, b), big);
        if c != 0 {
            while b < line.len() && class(grapheme_at(&line, b), big) == c {
                b = next_grapheme(&line, b);
            }
        }
    }
    loop {
        while b < line.len() && class(grapheme_at(&line, b), big) == 0 {
            b = next_grapheme(&line, b);
        }
        if b < line.len() {
            return ((l, b), l != start_line);
        }
        if l + 1 >= n {
            return ((l, line.len()), l != start_line);
        }
        l += 1;
        b = 0;
        line = buf.line(l);
        if line.is_empty() {
            return ((l, 0), true);
        }
    }
}

pub fn word_back(buf: &Buffer, (mut l, mut b): (usize, usize), big: bool) -> (usize, usize) {
    let mut line = buf.line(l);
    loop {
        if b == 0 {
            if l == 0 {
                return (0, 0);
            }
            l -= 1;
            line = buf.line(l);
            b = line.len();
            if line.is_empty() {
                return (l, 0);
            }
            continue;
        }
        b = prev_grapheme(&line, b);
        if class(grapheme_at(&line, b), big) != 0 {
            break;
        }
    }
    let c = class(grapheme_at(&line, b), big);
    while b > 0 && class(grapheme_at(&line, prev_grapheme(&line, b)), big) == c {
        b = prev_grapheme(&line, b);
    }
    (l, b)
}

pub fn word_end(buf: &Buffer, pos: (usize, usize), big: bool) -> (usize, usize) {
    let n = buf.len_lines();
    let (mut l, mut b) = pos;
    let mut line = buf.line(l);
    b = next_grapheme(&line, b);
    loop {
        if b >= line.len() {
            if l + 1 >= n {
                return pos;
            }
            l += 1;
            b = 0;
            line = buf.line(l);
            continue;
        }
        if class(grapheme_at(&line, b), big) == 0 {
            b = next_grapheme(&line, b);
            continue;
        }
        break;
    }
    let c = class(grapheme_at(&line, b), big);
    loop {
        let nb = next_grapheme(&line, b);
        if nb >= line.len() || class(grapheme_at(&line, nb), big) != c {
            return (l, b);
        }
        b = nb;
    }
}

fn find_in_line(
    line: &str,
    b: usize,
    kind: char,
    ch: char,
    count: usize,
    repeat: bool,
) -> Option<usize> {
    let target = ch.to_string();
    let mut found = 0;
    match kind {
        'f' | 't' => {
            let mut p = next_grapheme(line, b);
            if repeat && kind == 't' && p < line.len() && grapheme_at(line, p) == target {
                p = next_grapheme(line, p);
            }
            while p < line.len() {
                if grapheme_at(line, p) == target {
                    found += 1;
                    if found == count {
                        return Some(if kind == 't' {
                            prev_grapheme(line, p)
                        } else {
                            p
                        });
                    }
                }
                p = next_grapheme(line, p);
            }
            None
        }
        _ => {
            let mut p = b;
            if repeat && kind == 'T' && p > 0 && grapheme_at(line, prev_grapheme(line, p)) == target
            {
                p = prev_grapheme(line, p);
            }
            while p > 0 {
                p = prev_grapheme(line, p);
                if grapheme_at(line, p) == target {
                    found += 1;
                    if found == count {
                        return Some(if kind == 'T' {
                            next_grapheme(line, p)
                        } else {
                            p
                        });
                    }
                }
            }
            None
        }
    }
}

fn blank(buf: &Buffer, l: usize) -> bool {
    buf.line_len(l) == 0
}

/// Where `m` takes the cursor from its current position. `None` = can't move.
pub fn target(ed: &mut Editor, m: Motion, count: Option<usize>) -> Option<Target> {
    let n_count = count.unwrap_or(1).max(1);
    let (l, b) = ed.cur.pos();
    let buf = &ed.buf;
    let n = buf.len_lines();
    let line = buf.line(l);
    let fnb = |ed: &Editor, l: usize| Target::at(l, ed.first_nonblank(l)).linewise();
    let t = match m {
        Motion::Left => {
            if b == 0 {
                return None;
            }
            let mut p = b;
            for _ in 0..n_count {
                p = prev_grapheme(&line, p);
            }
            Target::at(l, p)
        }
        Motion::Right => {
            if line.is_empty() {
                return None;
            }
            let mut p = b;
            for _ in 0..n_count {
                p = next_grapheme(&line, p);
            }
            Target::at(l, p)
        }
        Motion::Up | Motion::Down => {
            let nl = if m == Motion::Down {
                if l + 1 >= n {
                    return None;
                }
                if ed.folds.is_empty() {
                    (l + n_count).min(n - 1)
                } else {
                    ed.folds.down(l, n_count, n)
                }
            } else {
                if l == 0 {
                    return None;
                }
                if ed.folds.is_empty() {
                    l.saturating_sub(n_count)
                } else {
                    ed.folds.up(l, n_count)
                }
            };
            if nl == l {
                return None;
            }
            let mut t = Target::at(nl, 0).linewise();
            t.keep_col = true;
            t
        }
        Motion::NextLine => {
            if l + 1 >= n {
                return None;
            }
            fnb(ed, (l + n_count).min(n - 1))
        }
        Motion::PrevLine => {
            if l == 0 {
                return None;
            }
            fnb(ed, l.saturating_sub(n_count))
        }
        Motion::WordFwd(big) => {
            let mut p = (l, b);
            for _ in 0..n_count {
                p = word_fwd(buf, p, big).0;
            }
            Target::at(p.0, p.1)
        }
        Motion::WordBack(big) => {
            let mut p = (l, b);
            for _ in 0..n_count {
                p = word_back(buf, p, big);
            }
            Target::at(p.0, p.1)
        }
        Motion::WordEnd(big) => {
            let mut p = (l, b);
            for _ in 0..n_count {
                p = word_end(buf, p, big);
            }
            Target::at(p.0, p.1).inclusive()
        }
        Motion::LineStart => Target::at(l, 0),
        Motion::FirstNonBlank => Target::at(l, ed.first_nonblank(l)),
        Motion::LineEnd => {
            let nl = (l + n_count - 1).min(n - 1);
            let mut t = Target::at(nl, ed.last_grapheme(nl)).inclusive();
            t.eol = true;
            t
        }
        Motion::GotoLine => fnb(ed, count.map_or(n - 1, |c| c.clamp(1, n) - 1)),
        Motion::FileStart => fnb(ed, count.map_or(0, |c| c.clamp(1, n) - 1)),
        Motion::ParaFwd => {
            let mut p = l;
            for _ in 0..n_count {
                while p < n && blank(buf, p) {
                    p += 1;
                }
                while p < n && !blank(buf, p) {
                    p += 1;
                }
                if p >= n {
                    let last = n - 1;
                    return Some(Target::at(last, buf.line_len(last)).inclusive_end(ed, last));
                }
            }
            Target::at(p, 0)
        }
        Motion::ParaBack => {
            let mut p = l;
            for _ in 0..n_count {
                while p > 0 && blank(buf, p) {
                    p -= 1;
                }
                while p > 0 && !blank(buf, p) {
                    p -= 1;
                }
            }
            Target::at(p, 0)
        }
        Motion::Find { kind, ch } => {
            ed.vim.last_find = Some((kind, ch));
            find_target(&line, l, b, kind, ch, n_count, false)?
        }
        Motion::RepeatFind { reverse } => {
            let (kind, ch) = ed.vim.last_find?;
            let kind = if reverse {
                match kind {
                    'f' => 'F',
                    'F' => 'f',
                    't' => 'T',
                    _ => 't',
                }
            } else {
                kind
            };
            find_target(&line, l, b, kind, ch, n_count, true)?
        }
        Motion::SearchNext { reverse } => {
            let (pl, pb) = ed.search_target_n(reverse, n_count)?;
            Target::at(pl, pb)
        }
        Motion::Mark(c) => match ed.marks.get(&c) {
            Some(&ml) => fnb(ed, ml.min(n - 1)),
            None => {
                ed.set_err("mark not set");
                return None;
            }
        },
    };
    Some(t)
}

impl Target {
    /// `}` at end of file: land on the last character, inclusive.
    fn inclusive_end(self, ed: &Editor, line: usize) -> Target {
        let mut t = Target::at(line, ed.last_grapheme(line));
        t.inclusive = ed.buf.line_len(line) > 0;
        t
    }
}

fn find_target(
    line: &str,
    l: usize,
    b: usize,
    kind: char,
    ch: char,
    count: usize,
    repeat: bool,
) -> Option<Target> {
    let p = find_in_line(line, b, kind, ch, count, repeat)?;
    let t = Target::at(l, p);
    Some(if matches!(kind, 'f' | 't') {
        t.inclusive()
    } else {
        t
    })
}
