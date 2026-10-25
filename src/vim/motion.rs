//! vim motions: where the cursor goes, and how an operator treats the span.

use super::textobj::{self, W};
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
    /// `ge`, `gE`.
    WordEndBack(bool),
    /// `H`, `M`, `L`: a line of the window.
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    /// `|`: screen column N.
    Column,
    /// `g_`: last non-blank, N-1 lines down.
    LastNonBlank,
    /// `_`: first non-blank, N-1 lines down.
    FirstNonBlankDown,
    /// `%`: the matching bracket, or N% of the file with a count.
    Percent,
    /// `)` and `(`.
    Sentence(bool),
    /// `]]` (fwd, '{'), `][` (fwd, '}'), `[[`, `[]`.
    Section {
        fwd: bool,
        what: char,
    },
    /// `[(`, `[{`, `])`, `]}`: an unmatched bracket.
    Unmatched(char),
    /// `*`, `#` (`fwd` false), `g*`, `g#` (`whole` false).
    Star {
        fwd: bool,
        whole: bool,
    },
    /// `go`: byte N of the buffer.
    Byte,
    /// `iw`, `a(`…: a text object (in Visual mode and after an operator).
    Object {
        include: bool,
        kind: char,
    },
    /// `` `a ``: a mark's exact position.
    MarkExact(char),
}

impl Motion {
    /// Jumps set the previous context mark and go in the jumplist.
    pub fn is_jump(self) -> bool {
        matches!(
            self,
            Motion::GotoLine
                | Motion::FileStart
                | Motion::Percent
                | Motion::Sentence(_)
                | Motion::ParaFwd
                | Motion::ParaBack
                | Motion::Section { .. }
                | Motion::SearchNext { .. }
                | Motion::Star { .. }
                | Motion::ScreenTop
                | Motion::ScreenMiddle
                | Motion::ScreenBottom
                | Motion::Mark(_)
                | Motion::MarkExact(_)
                | Motion::Unmatched(_)
                | Motion::Byte
        )
    }
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
    /// `|`: remember this screen column.
    pub want: Option<usize>,
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
            want: None,
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
    let keep = |l: usize| {
        let mut t = Target::at(l, 0).linewise();
        t.keep_col = true;
        t
    };
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
            // At the end of the buffer `w` can't move: it fails (operators
            // use their own span).
            let clamped = if p.1 >= buf.line_len(p.0) {
                (p.0, ed.last_grapheme(p.0))
            } else {
                p
            };
            if clamped == (l, b) {
                return None;
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
        // 'startofline' is off: these keep the column.
        Motion::GotoLine => keep(count.map_or(n - 1, |c| c.clamp(1, n) - 1)),
        Motion::FileStart => keep(count.map_or(0, |c| c.clamp(1, n) - 1)),
        Motion::WordEndBack(big) => {
            let mut w = W::new(buf, (l, b));
            w.big = big;
            if !w.bckend_word(n_count, false) {
                return None;
            }
            Target::at(w.p.0, w.p.1).inclusive()
        }
        Motion::ScreenTop | Motion::ScreenMiddle | Motion::ScreenBottom => {
            let (top, rows) = ed
                .viewport
                .map_or((0, ed.win_height), |v| (v.view.top, v.rows));
            let rows = rows.max(1);
            let top = top.min(n - 1);
            let bottom = (top + rows).min(n) - 1;
            // Fred's view keeps 2 lines of context ('scrolloff').
            let so = 2.min((rows - 1) / 2);
            let t = match m {
                Motion::ScreenTop => {
                    let so = if top > 0 { so } else { 0 };
                    (top + (n_count - 1).max(so)).min(bottom)
                }
                Motion::ScreenBottom => {
                    let so = if bottom + 1 < n { so } else { 0 };
                    bottom.saturating_sub((n_count - 1).max(so)).max(top)
                }
                _ => top + (bottom - top + 2) / 2 - 1,
            };
            keep(t)
        }
        Motion::Column => {
            let col = n_count - 1;
            let mut t = Target::at(l, crate::text::byte_of_col(&line, col, ed.tabstop()));
            t.want = Some(col);
            t
        }
        Motion::LastNonBlank => {
            let nl = l + n_count - 1;
            if nl >= n {
                return None;
            }
            let s = buf.line(nl);
            let end = s.trim_end_matches([' ', '\t']).len();
            let mut t = Target::at(nl, prev_grapheme(&s, end.max(1).min(s.len())));
            if end == 0 {
                t.byte = 0;
            }
            t.inclusive()
        }
        Motion::FirstNonBlankDown => {
            let nl = l + n_count - 1;
            if nl >= n {
                return None;
            }
            fnb(ed, nl)
        }
        Motion::Percent => match count {
            Some(c) if c > 100 => return None,
            Some(c) => keep(((n * c).div_ceil(100)).clamp(1, n) - 1),
            None => {
                let p = textobj::find_match(buf, (l, b), None, None, textobj::MATCHPAIRS, true)?;
                Target::at(p.0, p.1).inclusive()
            }
        },
        Motion::Sentence(fwd) => {
            let p = textobj::findsent(buf, (l, b), fwd, n_count)?;
            at_text(ed, p)
        }
        Motion::Section { fwd, what } => {
            let (p, incl) = findpar(buf, l, fwd, n_count, what, false)?;
            let t = Target::at(p.0, p.1);
            if incl { t.inclusive() } else { t }
        }
        Motion::Unmatched(c) => {
            let fwd = matches!(c, ')' | '}');
            let mut p = (l, b);
            let mut found = None;
            for _ in 0..n_count {
                match textobj::find_match(buf, p, Some(c), Some(fwd), textobj::MATCHPAIRS, true) {
                    Some(q) => {
                        found = Some(q);
                        p = q;
                    }
                    None => break,
                }
            }
            let p = found?;
            Target::at(p.0, p.1)
        }
        Motion::Star { fwd, whole } => {
            let Some((start, end)) = ident_at(&line, b) else {
                ed.set_err("E348: No string under cursor");
                return None;
            };
            let word = &line[start..end];
            let special: &[char] = if fwd {
                &['/', '.', '*', '~', '[', '^', '$', '\\']
            } else {
                &['/', '?', '.', '*', '~', '[', '^', '$', '\\']
            };
            let mut pat = String::from("\\C");
            let kw = |c: Option<char>| c.is_some_and(|c| class_of(c) >= 2);
            if whole && kw(word.chars().next()) {
                pat.push_str("\\<");
            }
            for c in word.chars() {
                if special.contains(&c) {
                    pat.push('\\');
                }
                pat.push(c);
            }
            if whole && kw(word.chars().last()) {
                pat.push_str("\\>");
            }
            ed.last_pat = Some(pat);
            ed.last_search_fwd = fwd;
            let saved = ed.cur.byte;
            ed.cur.byte = start;
            let r = ed.search_target_n(false, n_count);
            ed.cur.byte = saved;
            let (pl, pb) = r?;
            Target::at(pl, pb)
        }
        Motion::Byte => {
            let rope = buf.rope();
            let off = count.unwrap_or(1).max(1) - 1;
            let off = off.min(rope.len_bytes().saturating_sub(1));
            let nl = rope.byte_to_line(off).min(n - 1);
            let b = off - rope.line_to_byte(nl);
            let s = buf.line(nl);
            Target::at(nl, crate::text::floor_grapheme(&s, b.min(s.len())))
        }
        // Handled by `ops::span` and Visual mode.
        Motion::Object { .. } => return None,
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
        Motion::Mark(c) | Motion::MarkExact(c) => match super::marks::get(ed, c) {
            Some((ml, mb)) if matches!(m, Motion::MarkExact(_)) => Target::at(ml, mb),
            Some((ml, _)) => fnb(ed, ml),
            None => {
                ed.set_err("E20: Mark not set");
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

/// A position from `findsent` as a target: not on a line's end (Neovim's
/// `adjust_cursor`), where an operator then includes the last character.
fn at_text(ed: &Editor, p: (usize, usize)) -> Target {
    let len = ed.buf.line_len(p.0);
    if p.1 >= len && len > 0 {
        Target::at(p.0, ed.last_grapheme(p.0)).inclusive()
    } else {
        Target::at(p.0, p.1)
    }
}

fn class_of(c: char) -> u8 {
    class(c.encode_utf8(&mut [0; 4]), false)
}

/// The identifier under or after byte `b` (else the non-blank string), as
/// Neovim's `find_ident_under_cursor` finds it for `*` and `#`.
fn ident_at(line: &str, b: usize) -> Option<(usize, usize)> {
    let cls = |i: usize| class(grapheme_at(line, i), false);
    for want_ident in [true, false] {
        let mut col = b.min(line.len());
        while col < line.len() {
            let c = cls(col);
            if c != 0 && (!want_ident || c != 1) {
                break;
            }
            col = next_grapheme(line, col);
        }
        if col >= line.len() {
            continue;
        }
        let this = cls(col);
        while col > 0 && cls(prev_grapheme(line, col)) == this {
            col = prev_grapheme(line, col);
        }
        let mut end = next_grapheme(line, col);
        while end < line.len()
            && if want_ident {
                cls(end) == this
            } else {
                cls(end) != 0
            }
        {
            end = next_grapheme(line, end);
        }
        return Some((col, end));
    }
    None
}

/// Neovim's `findpar()` for sections: the line after `count` section
/// starts (`what` '{' or '}'). Returns the position and whether the motion
/// becomes inclusive (it reached the last line's last character).
fn findpar(
    buf: &Buffer,
    line: usize,
    fwd: bool,
    count: usize,
    what: char,
    both: bool,
) -> Option<((usize, usize), bool)> {
    let n = buf.len_lines();
    let mut curr = line;
    for left in (0..count).rev() {
        let mut did_skip = false;
        let mut first = true;
        loop {
            if buf.line_len(curr) != 0 {
                did_skip = true;
            }
            if !first && did_skip && textobj::start_ps(buf, curr, what, both) {
                break;
            }
            first = false;
            let next = if fwd {
                curr.checked_add(1).filter(|&c| c < n)
            } else {
                curr.checked_sub(1)
            };
            match next {
                Some(c) => curr = c,
                None if left > 0 => return None,
                None => break,
            }
        }
    }
    if both && buf.line(curr).starts_with('}') {
        curr = (curr + 1).min(n - 1);
    }
    if curr == n - 1 && what != '}' && fwd {
        let s = buf.line(curr);
        if !s.is_empty() {
            return Some(((curr, prev_grapheme(&s, s.len())), true));
        }
    }
    Some(((curr, 0), false))
}
