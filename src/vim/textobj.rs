//! Neovim's cursor walking (`inc()`/`dec()`), word and sentence motions,
//! bracket matching (`%`) and the text objects (`iw`, `as`, `ip`, `i(`,
//! `i"`…), ported from `textobject.c` and `search.c` of the pinned source.

use crate::buffer::Buffer;
use crate::text::{next_grapheme, prev_grapheme};

/// (line, byte). A walker may stand on a line's end (Vim's NUL).
pub type Pos = (usize, usize);

/// What a text object covers: from `start` to `end`, where `end` is the
/// cursor. In Visual mode `start` is the new Visual start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obj {
    pub start: Pos,
    pub end: Pos,
    pub inclusive: bool,
    pub linewise: bool,
}

/// A cursor that moves as Neovim's `inc()` and `dec()` do.
#[derive(Clone)]
pub(crate) struct W<'a> {
    buf: &'a Buffer,
    pub p: Pos,
    line: String,
    /// `W`, `B`, `E`: every non-blank is one class.
    pub big: bool,
}

const WHITE: [char; 2] = [' ', '\t'];

impl<'a> W<'a> {
    pub fn new(buf: &'a Buffer, p: Pos) -> Self {
        W {
            buf,
            p,
            line: buf.line(p.0),
            big: false,
        }
    }

    pub fn set(&mut self, p: Pos) {
        if p.0 != self.p.0 {
            self.line = self.buf.line(p.0);
        }
        self.p = (p.0, p.1.min(self.line.len()));
    }

    fn last_line(&self) -> usize {
        self.buf.len_lines() - 1
    }

    /// The character here; None at the end of the line.
    pub fn ch(&self) -> Option<char> {
        self.line.get(self.p.1..).and_then(|s| s.chars().next())
    }

    fn white(&self) -> bool {
        self.ch().is_some_and(|c| WHITE.contains(&c))
    }

    fn line_empty(&self, l: usize) -> bool {
        self.buf.line_len(l) == 0
    }

    /// Forward one character: 0 within the line, 2 onto its end, 1 onto
    /// the next line, -1 at the end of the buffer.
    pub fn inc(&mut self) -> i32 {
        if self.p.1 < self.line.len() {
            self.p.1 = next_grapheme(&self.line, self.p.1);
            return if self.p.1 < self.line.len() { 0 } else { 2 };
        }
        if self.p.0 < self.last_line() {
            self.set((self.p.0 + 1, 0));
            return 1;
        }
        -1
    }

    /// Back one character: 0 within the line, 1 onto the previous line's
    /// end, -1 at the start of the buffer.
    pub fn dec(&mut self) -> i32 {
        if self.p.1 > 0 {
            self.p.1 = prev_grapheme(&self.line, self.p.1);
            return 0;
        }
        if self.p.0 > 0 {
            let l = self.p.0 - 1;
            self.set((l, usize::MAX));
            return 1;
        }
        -1
    }

    /// `inc()` that skips line ends.
    pub fn incl(&mut self) -> i32 {
        let mut r = self.inc();
        if r >= 1 && self.p.1 != 0 {
            r = self.inc();
        }
        r
    }

    /// `dec()` that skips line ends.
    pub fn decl(&mut self) -> i32 {
        let mut r = self.dec();
        if r == 1 && self.p.1 != 0 {
            r = self.dec();
        }
        r
    }

    fn oneleft(&mut self) -> bool {
        if self.p.1 == 0 {
            return false;
        }
        self.p.1 = prev_grapheme(&self.line, self.p.1);
        true
    }

    /// Character class: 0 blank, 1 punctuation, 2+ word characters.
    pub fn cls(&self) -> u8 {
        match self.ch() {
            None | Some(' ' | '\t') => 0,
            Some(c) => match class(c) {
                0 => 0,
                _ if self.big => 1,
                k => k,
            },
        }
    }

    fn skip_chars(&mut self, c: u8, fwd: bool) -> bool {
        while self.cls() == c {
            if (if fwd { self.inc() } else { self.dec() }) == -1 {
                return true;
            }
        }
        false
    }

    fn back_in_line(&mut self) {
        let s = self.cls();
        while self.p.1 > 0 {
            self.dec();
            if self.cls() != s {
                self.inc();
                break;
            }
        }
    }

    fn find_first_blank(&mut self) {
        while self.decl() != -1 {
            if !self.white() {
                self.incl();
                break;
            }
        }
    }

    /// `w`: false if already at the end of the buffer. With `eol` the last
    /// word stops at the end of its line (for operators).
    pub fn fwd_word(&mut self, mut count: usize, eol: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            let last = self.p.0 == self.last_line();
            let i = self.inc();
            if i == -1 || (i >= 1 && last) {
                return false;
            }
            if i >= 1 && eol && count == 0 {
                return true;
            }
            if sclass != 0 {
                while self.cls() == sclass {
                    let i = self.inc();
                    if i == -1 || (i >= 1 && eol && count == 0) {
                        return true;
                    }
                }
            }
            while self.cls() == 0 {
                if self.p.1 == 0 && self.line.is_empty() {
                    break;
                }
                let i = self.inc();
                if i == -1 || (i >= 1 && eol && count == 0) {
                    return true;
                }
            }
        }
        true
    }

    /// `b`. With `stop`, on the start of a word moves one word less.
    pub fn bck_word(&mut self, mut count: usize, mut stop: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            if self.dec() == -1 {
                return false;
            }
            'finished: {
                if !stop || sclass == self.cls() || sclass == 0 {
                    while self.cls() == 0 {
                        if self.p.1 == 0 && self.line_empty(self.p.0) {
                            break 'finished;
                        }
                        if self.dec() == -1 {
                            return true;
                        }
                    }
                    if self.skip_chars(self.cls(), false) {
                        return true;
                    }
                }
                self.inc();
            }
            stop = false;
        }
        true
    }

    /// `e`. With `stop`, on the end of a word moves one word less; with
    /// `empty`, stops at an empty line.
    pub fn end_word(&mut self, mut count: usize, mut stop: bool, empty: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            if self.inc() == -1 {
                return false;
            }
            'finished: {
                if self.cls() == sclass && sclass != 0 {
                    if self.skip_chars(sclass, true) {
                        return false;
                    }
                } else if !stop || sclass == 0 {
                    while self.cls() == 0 {
                        if empty && self.p.1 == 0 && self.line_empty(self.p.0) {
                            break 'finished;
                        }
                        if self.inc() == -1 {
                            return false;
                        }
                    }
                    if self.skip_chars(self.cls(), true) {
                        return false;
                    }
                }
                self.dec();
            }
            stop = false;
        }
        true
    }

    /// `ge`. With `eol`, stops at the end of a line.
    pub fn bckend_word(&mut self, mut count: usize, eol: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            let i = self.dec();
            if i == -1 {
                return false;
            }
            if eol && i == 1 {
                return true;
            }
            if sclass != 0 {
                while self.cls() == sclass {
                    let i = self.dec();
                    if i == -1 || (eol && i == 1) {
                        return true;
                    }
                }
            }
            while self.cls() == 0 {
                if self.p.1 == 0 && self.line_empty(self.p.0) {
                    break;
                }
                let i = self.dec();
                if i == -1 || (eol && i == 1) {
                    return true;
                }
            }
        }
        true
    }
}

/// Neovim's `utf_class()` with the default 'iskeyword'.
fn class(c: char) -> u8 {
    match c {
        ' ' | '\t' | '\0' | '\u{a0}' => 0,
        c if c.is_alphanumeric() || c == '_' => 2,
        c if (c as u32) < 0x100 => 1,
        c if c.is_whitespace() => 0,
        c if (c as u32) >= 0x1f000 => 3,
        _ => 1,
    }
}

const PARA: &[u8] = b"IPLPPPQPP TPHPLIPpLpItpplpipbp";
const SECTIONS: &[u8] = b"SHNHH HUnhsh";

/// Is `s` (after a line's leading `.`) an nroff macro named in `opt`?
fn inmacro(opt: &[u8], s: &[u8]) -> bool {
    let at = |v: &[u8], i: usize| v.get(i).copied().unwrap_or(0);
    let s0 = at(s, 0);
    let s1 = if s0 == 0 { 0 } else { at(s, 1) };
    let mut i = 0;
    while at(opt, i) != 0 {
        let (m0, m1) = (at(opt, i), at(opt, i + 1));
        if (m0 == s0 || (m0 == b' ' && (s0 == 0 || s0 == b' ')))
            && (m1 == s1 || ((m1 == 0 || m1 == b' ') && (s0 == 0 || s1 == 0 || s1 == b' ')))
        {
            return true;
        }
        if m1 == 0 {
            return false;
        }
        i += 2;
    }
    false
}

/// Does line `l` start a paragraph or section (`para` = '\0') or only a
/// section (`para` = '{' or '}')? With `both`, a `}` line too.
pub(crate) fn start_ps(buf: &Buffer, l: usize, para: char, both: bool) -> bool {
    let s = buf.line(l);
    let first = s.chars().next().unwrap_or('\0');
    if first == para || first == '\x0c' || (both && first == '}') {
        return true;
    }
    first == '.'
        && (inmacro(SECTIONS, &s.as_bytes()[1..])
            || (para == '\0' && inmacro(PARA, &s.as_bytes()[1..])))
}

/// `)` and `(`: the start of the `count`-th next or previous sentence.
pub(crate) fn findsent(buf: &Buffer, start: Pos, fwd: bool, count: usize) -> Option<Pos> {
    let mut w = W::new(buf, start);
    let step = |w: &mut W| if fwd { w.incl() } else { w.decl() };
    let mut noskip = false;
    let mut count = count;
    while count > 0 {
        count -= 1;
        let prev = w.p;
        'found: {
            if w.ch().is_none() {
                // On an empty line: skip to a non-empty one.
                while step(&mut w) != -1 && w.ch().is_none() {}
                if fwd {
                    break 'found;
                }
            } else if fwd && w.p.1 == 0 && start_ps(buf, w.p.0, '\0', false) {
                if w.p.0 == w.last_line() {
                    return None;
                }
                w.set((w.p.0 + 1, 0));
                break 'found;
            } else if !fwd {
                w.decl();
            }
            // Back to the previous non-white, non-punctuation character.
            let mut found_dot = false;
            while let Some(c) = w
                .ch()
                .filter(|&c| WHITE.contains(&c) || ".!?)]\"'".contains(c))
            {
                let mut t = w.clone();
                if t.decl() == -1 || (t.line_empty(t.p.0) && fwd) || found_dot {
                    break;
                }
                if ".!?".contains(c) {
                    found_dot = true;
                }
                if ")]\"'".contains(c) && !t.ch().is_some_and(|tc| ".!?)]\"'".contains(tc)) {
                    break;
                }
                w.decl();
            }
            let start_line = w.p.0;
            loop {
                let Some(c) = w
                    .ch()
                    .filter(|_| !(w.p.1 == 0 && start_ps(buf, w.p.0, '\0', false)))
                else {
                    if !fwd && w.p.0 != start_line {
                        w.set((w.p.0 + 1, w.p.1));
                    }
                    break;
                };
                if matches!(c, '.' | '!' | '?') {
                    let mut t = w.clone();
                    let mut r;
                    loop {
                        r = t.inc();
                        if r == -1 || !t.ch().is_some_and(|x| ")]\"'".contains(x)) {
                            break;
                        }
                    }
                    if r == -1 || t.ch().is_none_or(|x| WHITE.contains(&x)) {
                        w = t;
                        if w.ch().is_none() {
                            w.inc();
                        }
                        break;
                    }
                }
                if step(&mut w) == -1 {
                    if count > 0 {
                        return None;
                    }
                    noskip = true;
                    break;
                }
            }
        }
        while !noskip && w.white() {
            if w.incl() == -1 {
                break;
            }
        }
        if w.p == prev {
            // Didn't move: advance one character and try again.
            if step(&mut w) == -1 {
                if count > 0 {
                    return None;
                }
                break;
            }
            count += 1;
        }
    }
    Some(w.p)
}

/// The brackets `%` matches.
pub(crate) const MATCHPAIRS: &[(char, char)] = &[('(', ')'), ('{', '}'), ('[', ']')];
/// The brackets text objects match.
const OBJECT_PAIRS: &[(char, char)] = &[('(', ')'), ('{', '}'), ('[', ']'), ('<', '>')];

fn backslashes_before(line: &[u8], col: usize) -> usize {
    line[..col]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
}

fn char_at(line: &[u8], col: usize) -> Option<char> {
    std::str::from_utf8(line.get(col..)?)
        .ok()
        .and_then(|s| s.chars().next())
}

/// Neovim's `findmatchlimit()` for brackets. With `initc`, the unmatched
/// bracket of its pair in its direction (backward for an opening one) or
/// in direction `forward`; without, the match of the bracket under or
/// after the cursor. `smart` skips brackets in quotes (`cpo` without `%`).
pub(crate) fn find_match(
    buf: &Buffer,
    cur: Pos,
    initc: Option<char>,
    forward: Option<bool>,
    pairs: &[(char, char)],
    smart: bool,
) -> Option<Pos> {
    let mut pos = cur;
    let mut line = buf.line(pos.0).into_bytes();
    let (initc, findc, backwards, match_escaped) = match initc {
        Some(c) => {
            let &(open, close) = pairs.iter().find(|p| p.0 == c || p.1 == c)?;
            let (i, f, back) = if c == open {
                (close, open, true)
            } else {
                (open, close, false)
            };
            (i, f, forward.map_or(back, |f| !f), 0)
        }
        None => {
            if pos.1 >= line.len() && pos.1 > 0 {
                pos.1 = prev_grapheme(std::str::from_utf8(&line).ok()?, pos.1);
            }
            let (i, f, back) = loop {
                let c = char_at(&line, pos.1)?;
                if let Some(&(open, close)) = pairs.iter().find(|p| p.0 == c || p.1 == c) {
                    break if c == open {
                        (open, close, false)
                    } else {
                        (close, open, true)
                    };
                }
                pos.1 += c.len_utf8();
            };
            (i, f, back, backslashes_before(&line, pos.1) & 1)
        }
    };
    let n = buf.len_lines();
    let mut count = 0i32;
    let mut do_quotes: i32 = -1;
    let mut inquote = false;
    let mut start_in_quotes: Option<bool> = None;
    loop {
        if backwards {
            if pos.1 == 0 {
                if pos.0 == 0 {
                    break;
                }
                pos.0 -= 1;
                line = buf.line(pos.0).into_bytes();
                pos.1 = line.len();
                do_quotes = -1;
            } else {
                pos.1 -= 1;
                while pos.1 > 0 && line[pos.1] & 0xc0 == 0x80 {
                    pos.1 -= 1;
                }
            }
        } else if pos.1 >= line.len() {
            if pos.0 + 1 >= n {
                break;
            }
            pos.0 += 1;
            line = buf.line(pos.0).into_bytes();
            pos.1 = 0;
            do_quotes = -1;
        } else {
            pos.1 += char_at(&line, pos.1).map_or(1, char::len_utf8);
        }
        if !smart {
            do_quotes = 0;
        } else if do_quotes == -1 {
            // Brackets in quotes are skipped only when the line has an
            // even number of quotes (not counting \" and '"').
            let mut at_start = do_quotes;
            let mut i = 0;
            while i < line.len() {
                if i == pos.1 + backwards as usize {
                    at_start = do_quotes & 1;
                }
                if line[i] == b'"'
                    && (i == 0 || line[i - 1] != b'\'' || line.get(i + 1) != Some(&b'\''))
                {
                    do_quotes += 1;
                }
                if line[i] == b'\\' && i + 1 < line.len() {
                    i += 1;
                }
                i += 1;
            }
            do_quotes &= 1;
            if do_quotes == 0 {
                inquote = false;
                if line.last() == Some(&b'\\') {
                    do_quotes = 1;
                    if start_in_quotes.is_none() {
                        inquote = true;
                        start_in_quotes = Some(true);
                    } else if backwards {
                        inquote = true;
                    }
                }
                if pos.0 > 0 && buf.line(pos.0 - 1).ends_with('\\') {
                    do_quotes = 1;
                    if start_in_quotes.is_none() {
                        inquote = at_start != 0;
                        if inquote {
                            start_in_quotes = Some(true);
                        }
                    } else if !backwards {
                        inquote = true;
                    }
                }
            }
        }
        if start_in_quotes.is_none() {
            start_in_quotes = Some(false);
        }
        match char_at(&line, pos.1) {
            None => {
                if pos.1 == 0 || line[pos.1 - 1] != b'\\' {
                    inquote = false;
                    start_in_quotes = Some(false);
                }
            }
            Some('"') => {
                if do_quotes != 0 && backslashes_before(&line, pos.1).is_multiple_of(2) {
                    inquote = !inquote;
                    start_in_quotes = Some(false);
                }
            }
            Some(c) => {
                // Skip 'x' and '\x'.
                if c == '\'' && smart {
                    let at = |i: usize| line.get(i).copied().unwrap_or(0);
                    if backwards {
                        if pos.1 > 1 && at(pos.1 - 2) == b'\'' {
                            pos.1 -= 2;
                            continue;
                        }
                        if pos.1 > 2 && at(pos.1 - 2) == b'\\' && at(pos.1 - 3) == b'\'' {
                            pos.1 -= 3;
                            continue;
                        }
                    } else if at(pos.1 + 1) != 0 {
                        if at(pos.1 + 1) == b'\\' && at(pos.1 + 2) != 0 && at(pos.1 + 3) == b'\'' {
                            pos.1 += 3;
                            continue;
                        }
                        if at(pos.1 + 2) == b'\'' {
                            pos.1 += 2;
                            continue;
                        }
                    }
                }
                if (!inquote || start_in_quotes == Some(true))
                    && (c == initc || c == findc)
                    && backslashes_before(&line, pos.1) & 1 == match_escaped
                {
                    if c == initc {
                        count += 1;
                    } else if count == 0 {
                        return Some(pos);
                    } else {
                        count -= 1;
                    }
                }
            }
        }
    }
    None
}

/// `iw`, `aw`, `iW`, `aW`. `visual` is the Visual start when one is active.
pub(crate) fn word(
    buf: &Buffer,
    cur: Pos,
    mut count: usize,
    include: bool,
    big: bool,
    visual: Option<Pos>,
) -> Option<Obj> {
    let mut w = W::new(buf, cur);
    w.big = big;
    let mut inclusive = true;
    let mut include_white = false;
    let mut start = cur;
    let mut vis = visual;
    if visual.is_none_or(|v| v == cur) {
        w.back_in_line();
        start = w.p;
        if (w.cls() == 0) == include {
            if !w.end_word(1, true, true) {
                return None;
            }
        } else {
            w.fwd_word(1, true);
            if w.p.1 == 0 {
                w.decl();
            } else {
                w.oneleft();
            }
            include_white = include;
        }
        if vis.is_some() {
            vis = Some(start);
        }
        count -= 1;
    }
    while count > 0 {
        inclusive = true;
        if vis.is_some_and(|v| w.p < v) {
            if w.decl() == -1 {
                return None;
            }
            if include != (w.cls() != 0) {
                if !w.bck_word(1, true) {
                    return None;
                }
            } else {
                if !w.bckend_word(1, true) {
                    return None;
                }
                w.incl();
            }
        } else {
            if w.incl() == -1 {
                return None;
            }
            if include != (w.cls() == 0) {
                if !w.fwd_word(1, true) && count > 1 {
                    return None;
                }
                if !w.oneleft() {
                    inclusive = false;
                }
            } else if !w.end_word(1, true, true) {
                return None;
            }
        }
        count -= 1;
    }
    if include_white && (w.cls() != 0 || (w.p.1 == 0 && !inclusive)) {
        // No white space after the word: take the white space before it
        // instead (but not indent).
        let end = w.p;
        w.set(start);
        if w.oneleft() {
            w.back_in_line();
            if w.cls() == 0 && w.p.1 > 0 {
                if vis.is_some() {
                    vis = Some(w.p);
                } else {
                    start = w.p;
                }
            }
        }
        w.set(end);
    }
    Some(Obj {
        start: vis.unwrap_or(start),
        end: w.p,
        inclusive,
        linewise: false,
    })
}

fn sent_forward(buf: &Buffer, w: &mut W, mut count: usize, mut at_start: bool) {
    while count > 0 {
        count -= 1;
        if let Some(p) = findsent(buf, w.p, true, 1) {
            w.set(p);
        }
        if at_start {
            w.find_first_blank();
        }
        if count == 0 || at_start {
            w.decl();
        }
        at_start = !at_start;
    }
}

/// `is` and `as`.
pub(crate) fn sentence(
    buf: &Buffer,
    cur: Pos,
    count: usize,
    include: bool,
    visual: Option<Pos>,
) -> Option<Obj> {
    let fs = |w: &mut W, fwd: bool| {
        if let Some(p) = findsent(buf, w.p, fwd, 1) {
            w.set(p);
        }
    };
    let mut start = cur;
    let mut pos = W::new(buf, cur);
    let mut w = W::new(buf, cur);
    fs(&mut w, true);
    if let Some(v) = visual
        && start != v
    {
        return Some(sent_extend(buf, start, v, pos, w, count, include));
    }
    while pos.white() {
        pos.incl();
    }
    let start_blank = pos.p == w.p;
    if start_blank {
        let mut s = W::new(buf, start);
        s.find_first_blank();
        start = s.p;
    } else {
        fs(&mut w, false);
        start = w.p;
    }
    let ncount = if include {
        count * 2
    } else {
        count - start_blank as usize
    };
    if ncount > 0 {
        sent_forward(buf, &mut w, ncount, true);
    } else {
        w.decl();
    }
    if include {
        if start_blank {
            w.find_first_blank();
            if w.white() {
                w.decl();
            }
        } else if !w.white() {
            let mut s = W::new(buf, start);
            s.find_first_blank();
            start = s.p;
        }
    }
    if let Some(v) = visual {
        if start == w.p {
            return Some(sent_extend(buf, start, v, pos, w, count, include));
        }
        return Some(Obj {
            start,
            end: w.p,
            inclusive: true,
            linewise: false,
        });
    }
    let inclusive = w.incl() == -1;
    Some(Obj {
        start,
        end: w.p,
        inclusive,
        linewise: false,
    })
}

/// `is`/`as` with a Visual area of more than one character: extend it.
fn sent_extend(
    buf: &Buffer,
    start: Pos,
    vis: Pos,
    mut pos: W,
    mut w: W,
    mut count: usize,
    include: bool,
) -> Obj {
    let fs = |w: &mut W, fwd: bool| {
        if let Some(p) = findsent(buf, w.p, fwd, 1) {
            w.set(p);
        }
    };
    if include {
        count *= 2;
    }
    if start < vis {
        // The cursor is at the start of the Visual area.
        let mut at_start = true;
        pos.decl();
        while pos.p < w.p {
            if !pos.white() {
                at_start = false;
                break;
            }
            pos.incl();
        }
        if !at_start {
            fs(&mut w, false);
            if w.p == start {
                at_start = true;
            } else {
                fs(&mut w, true);
            }
        }
        while count > 0 {
            count -= 1;
            if at_start {
                w.find_first_blank();
            }
            if !at_start || (!include && !w.white()) {
                fs(&mut w, false);
            }
            at_start = !at_start;
        }
    } else {
        pos.incl();
        let mut at_start = true;
        if pos.p != w.p {
            at_start = false;
            while pos.p < w.p {
                if !pos.white() {
                    at_start = true;
                    break;
                }
                pos.incl();
            }
            if at_start {
                fs(&mut w, false);
            } else {
                w.set(start);
            }
        }
        sent_forward(buf, &mut w, count, at_start);
    }
    Obj {
        start: vis,
        end: w.p,
        inclusive: true,
        linewise: false,
    }
}

/// `ip` and `ap`. In Visual mode `visual` is the Visual start; the result
/// is linewise.
pub(crate) fn paragraph(
    buf: &Buffer,
    cur: Pos,
    count: usize,
    include: bool,
    visual: Option<Pos>,
    visual_lines: bool,
) -> Option<Obj> {
    // 1-based line numbers, as upstream.
    let n = buf.len_lines();
    let white = |l: usize| buf.line(l - 1).chars().all(|c| WHITE.contains(&c));
    let ps = |l: usize| start_ps(buf, l - 1, '\0', false);
    let mut start_l = cur.0 + 1;
    let extend = |mut start_l: usize, vis_l: usize| -> Option<Obj> {
        let back = start_l < vis_l;
        let edge = if back { 1 } else { n };
        let step = |l: usize| if back { l - 1 } else { l + 1 };
        let mut ok = true;
        for _ in 0..count {
            if start_l == edge {
                ok = false;
                break;
            }
            let mut prev_white = None;
            for _ in 0..2 {
                start_l = step(start_l);
                let start_white = white(start_l);
                if prev_white == Some(start_white) {
                    start_l = if back { start_l + 1 } else { start_l - 1 };
                    break;
                }
                while start_l != edge
                    && start_white == white(step(start_l))
                    && (start_white || !ps(if back { start_l } else { start_l + 1 }))
                {
                    start_l = step(start_l);
                }
                if !include || start_l == edge {
                    break;
                }
                prev_white = Some(start_white);
            }
        }
        ok.then_some(Obj {
            start: visual?,
            end: (start_l - 1, 0),
            inclusive: true,
            linewise: true,
        })
    };
    if let Some(v) = visual
        && v.0 + 1 != start_l
    {
        return extend(start_l, v.0 + 1);
    }
    let white_in_front = white(start_l);
    while start_l > 1 {
        if white_in_front {
            if !white(start_l - 1) {
                break;
            }
        } else if white(start_l - 1) || ps(start_l) {
            break;
        }
        start_l -= 1;
    }
    let mut end_l = start_l;
    while end_l <= n && white(end_l) {
        end_l += 1;
    }
    end_l -= 1;
    let mut i = count - (!include && white_in_front) as usize;
    let mut do_white = false;
    while i > 0 {
        i -= 1;
        if end_l == n {
            return None;
        }
        if !include {
            do_white = white(end_l + 1);
        }
        if include || !do_white {
            end_l += 1;
            while end_l < n && !white(end_l + 1) && !ps(end_l + 1) {
                end_l += 1;
            }
        }
        if i == 0 && white_in_front && include {
            break;
        }
        if include || do_white {
            while end_l < n && white(end_l + 1) {
                end_l += 1;
            }
        }
    }
    if !white_in_front && !white(end_l) && include {
        while start_l > 1 && white(start_l - 1) {
            start_l -= 1;
        }
    }
    if visual.is_some() && visual_lines && start_l == cur.0 + 1 {
        return extend(start_l, cur.0 + 1);
    }
    Some(Obj {
        start: (start_l - 1, 0),
        end: (end_l - 1, 0),
        inclusive: true,
        linewise: true,
    })
}

/// Is the cursor in the line's indent (with `extra` columns to spare)?
fn inindent(w: &W, extra: usize) -> bool {
    let indent = w.line.len() - w.line.trim_start_matches(WHITE).len();
    indent >= w.p.1 + extra
}

/// `i(`, `a{`, `ib`… for the bracket pair `what`/`other`.
pub(crate) fn block(
    buf: &Buffer,
    cur: Pos,
    count: usize,
    include: bool,
    what: char,
    other: char,
    visual: Option<Pos>,
) -> Option<Obj> {
    let fm =
        |p: Pos, c: char, fwd: Option<bool>| find_match(buf, p, Some(c), fwd, OBJECT_PAIRS, false);
    let mut w = W::new(buf, cur);
    let (mut old_start, mut old_end) = (cur, cur);
    match visual {
        Some(v) if v != cur => {
            if v < cur {
                old_start = v;
                w.set(v);
            } else {
                old_end = v;
            }
        }
        _ => {
            if what == '{' {
                while inindent(&w, 1) {
                    if w.inc() != 0 {
                        break;
                    }
                }
            }
            if w.ch() == Some(what) {
                w.p.1 += 1;
            }
        }
    }
    // The enclosing block, or else the next one.
    let dir = if fm(w.p, what, None).is_some() {
        None
    } else {
        Some(true)
    };
    let mut start = w.p;
    for _ in 0..count {
        start = fm(w.p, what, dir)?;
        w.set(start);
    }
    let mut end_pos = fm(w.p, other, None)?;
    w.set(end_pos);
    let mut sw = W::new(buf, start);
    let mut sol = false;
    // Exclude the brackets (and an indent before the closing one); in
    // Visual mode, take the next block out while that is no bigger.
    let mut exclude = !include;
    while exclude {
        sw.incl();
        sol = w.p.1 == 0;
        w.decl();
        while inindent(&w, 1) {
            sol = true;
            if w.decl() != 0 {
                break;
            }
        }
        if visual.is_some() && sw.p == end_pos {
            return None;
        }
        // Not bigger than the Visual area: take the next block out.
        if visual.is_some() && sw.p >= old_start && old_end >= w.p && sw.p != w.p {
            w.set(old_start);
            w.decl();
            let s = fm(w.p, what, None)?;
            sw.set(s);
            w.set(s);
            end_pos = fm(w.p, other, None)?;
            w.set(end_pos);
        } else {
            exclude = false;
        }
    }
    let start = sw.p;
    if visual.is_some() {
        if sol && w.ch().is_some() {
            w.inc();
        }
        return Some(Obj {
            start,
            end: w.p,
            inclusive: true,
            linewise: false,
        });
    }
    let mut inclusive = false;
    if sol {
        w.incl();
    } else if start <= w.p {
        inclusive = true;
    } else {
        w.set(start);
    }
    Some(Obj {
        start,
        end: w.p,
        inclusive,
        linewise: false,
    })
}

fn next_quote(line: &[u8], mut col: usize, q: u8, escape: bool) -> Option<usize> {
    loop {
        let c = *line.get(col)?;
        if escape && c == b'\\' {
            col += 1;
            line.get(col)?;
        } else if c == q {
            return Some(col);
        }
        col += char_at(line, col).map_or(1, char::len_utf8);
    }
}

fn prev_quote(line: &[u8], mut col: usize, q: u8, escape: bool) -> usize {
    while col > 0 {
        col -= 1;
        while col > 0 && line[col] & 0xc0 == 0x80 {
            col -= 1;
        }
        let n = if escape {
            backslashes_before(line, col)
        } else {
            0
        };
        if n & 1 == 1 {
            col -= n;
        } else if line[col] == q {
            break;
        }
    }
    col
}

/// `i"`, `a'`, `` i` ``…: a quoted string on the cursor's line.
pub(crate) fn quote(
    buf: &Buffer,
    cur: Pos,
    count: usize,
    include: bool,
    q: u8,
    visual: Option<Pos>,
) -> Option<Obj> {
    let line = buf.line(cur.0).into_bytes();
    let at = |i: usize| line.get(i).copied().unwrap_or(0);
    let c0 = cur.1;
    let (mut vis_empty, mut vis_bef, mut inside, mut selected_quote) = (true, false, false, false);
    if let Some(v) = visual {
        if v.0 != cur.0 {
            return None;
        }
        vis_bef = v < cur;
        vis_empty = v == cur;
        if !vis_empty {
            let (mut i, end);
            if vis_bef {
                inside = v.1 > 0 && at(v.1 - 1) == q && at(c0) != 0 && at(c0 + 1) == q;
                (i, end) = (v.1, c0);
            } else {
                inside = c0 > 0 && at(c0 - 1) == q && at(v.1) != 0 && at(v.1 + 1) == q;
                (i, end) = (c0, v.1);
            }
            while i <= end && at(i) != 0 {
                i += 1;
                if at(i - 1) == q {
                    selected_quote = true;
                    break;
                }
            }
        }
    }
    let (mut col_start, mut col_end);
    if !vis_empty && at(c0) == q {
        // Already selecting, on a quote: the next quoted string.
        if vis_bef {
            col_start = next_quote(&line, c0 + 1, q, false)?;
            match next_quote(&line, col_start + 1, q, true) {
                Some(e) => col_end = e,
                None => (col_start, col_end) = (c0, col_start),
            }
        } else {
            col_end = prev_quote(&line, c0, q, false);
            if at(col_end) != q {
                return None;
            }
            col_start = prev_quote(&line, col_end, q, true);
            if at(col_start) != q {
                (col_start, col_end) = (col_end, c0);
            }
        }
    } else if at(c0) == q || !vis_empty {
        // On a quote (opening or closing?): scan from the line's start.
        let first_col = if vis_empty {
            Some(c0)
        } else if vis_bef {
            next_quote(&line, c0, q, false)
        } else {
            Some(prev_quote(&line, c0, q, false))
        };
        let first_col = first_col?;
        let mut from = 0;
        loop {
            let s = next_quote(&line, from, q, false).filter(|&s| s <= first_col)?;
            let e = next_quote(&line, s + 1, q, true)?;
            if first_col <= e {
                (col_start, col_end) = (s, e);
                break;
            }
            from = e + 1;
        }
    } else {
        col_start = prev_quote(&line, c0, q, true);
        if at(col_start) != q {
            col_start = next_quote(&line, col_start, q, false)?;
        }
        col_end = next_quote(&line, col_start + 1, q, true)?;
    }
    if include {
        if matches!(at(col_end + 1), b' ' | b'\t') {
            while matches!(at(col_end + 1), b' ' | b'\t') {
                col_end += 1;
            }
        } else {
            while col_start > 0 && matches!(at(col_start - 1), b' ' | b'\t') {
                col_start -= 1;
            }
        }
    }
    if !include && count < 2 && (vis_empty || !inside) {
        col_start += 1;
    }
    let mut vis = visual;
    if let Some(v) = visual
        && (vis_empty
            || (vis_bef
                && !selected_quote
                && (inside || (at(v.1) != q && (v.1 == 0 || at(v.1 - 1) != q)))))
    {
        vis = Some((cur.0, col_start));
    }
    let mut w = W::new(buf, (cur.0, col_end));
    let inclusive = (include || count > 1 || (!vis_empty && inside)) && w.inc() == 2;
    if let Some(v) = visual {
        if vis_empty || vis_bef {
            w.dec();
        } else {
            if inside || (!selected_quote && at(v.1) != q && (at(v.1) == 0 || at(v.1 + 1) != q)) {
                w.dec();
                vis = Some(w.p);
            }
            w.set((cur.0, col_start));
        }
        return Some(Obj {
            start: vis?,
            end: w.p,
            inclusive: true,
            linewise: false,
        });
    }
    Some(Obj {
        start: (cur.0, col_start),
        end: w.p,
        inclusive,
        linewise: false,
    })
}

/// The text object `include` (`a`) or not (`i`), named by `kind`.
pub(crate) fn object(
    buf: &Buffer,
    cur: Pos,
    count: usize,
    include: bool,
    kind: char,
    visual: Option<Pos>,
    visual_lines: bool,
) -> Option<Obj> {
    let count = count.max(1);
    match kind {
        'w' | 'W' => word(buf, cur, count, include, kind == 'W', visual),
        's' => sentence(buf, cur, count, include, visual),
        'p' => paragraph(buf, cur, count, include, visual, visual_lines),
        'b' | '(' | ')' => block(buf, cur, count, include, '(', ')', visual),
        'B' | '{' | '}' => block(buf, cur, count, include, '{', '}', visual),
        '[' | ']' => block(buf, cur, count, include, '[', ']', visual),
        '<' | '>' => block(buf, cur, count, include, '<', '>', visual),
        '"' | '\'' | '`' => quote(buf, cur, count, include, kind as u8, visual),
        _ => None,
    }
}
