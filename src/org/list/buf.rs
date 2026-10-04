//! A small Emacs buffer model for porting org-list.el literally: the text
//! as one string, point and markers as byte positions, folds that move
//! with edits. Org list code reads and edits through these primitives;
//! `super::load`/`super::store` map it to Fred's line buffer.

use crate::org::fold::Spec;
use regex::Regex;

/// Match data: group ranges as absolute byte positions.
#[derive(Clone, Debug, Default)]
pub struct Caps(pub Vec<Option<(usize, usize)>>);

impl Caps {
    pub fn get(&self, i: usize) -> Option<(usize, usize)> {
        self.0.get(i).copied().flatten()
    }
    pub fn beg(&self, i: usize) -> Option<usize> {
        self.get(i).map(|g| g.0)
    }
    pub fn end(&self, i: usize) -> Option<usize> {
        self.get(i).map(|g| g.1)
    }
    pub fn str<'a>(&self, b: &'a Buf, i: usize) -> Option<&'a str> {
        self.get(i).map(|(s, e)| &b.s[s..e])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker(usize);

/// A folded region: from the beginning of its first hidden line to the
/// end of its last one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fold {
    pub spec: Spec,
    pub beg: usize,
    pub end: usize,
}

#[derive(Default)]
pub struct Buf {
    pub s: String,
    pub pt: usize,
    marks: Vec<Option<(usize, bool)>>,
    pub folds: Vec<Fold>,
    /// The last `message`, shown by the caller.
    pub msg: Option<String>,
    /// Lines whose statistics cookies were updated (tags to re-align).
    pub cookie_lines: Vec<usize>,
}

fn is_ws(c: u8, set: &[u8]) -> bool {
    set.contains(&c)
}

impl Buf {
    pub fn new(s: impl Into<String>, pt: usize) -> Buf {
        let s = s.into();
        let pt = pt.min(s.len());
        Buf {
            s,
            pt,
            ..Buf::default()
        }
    }

    pub fn len(&self) -> usize {
        self.s.len()
    }

    pub fn is_empty(&self) -> bool {
        self.s.is_empty()
    }

    pub fn bol_at(&self, p: usize) -> usize {
        self.s[..p.min(self.s.len())]
            .rfind('\n')
            .map_or(0, |i| i + 1)
    }

    pub fn eol_at(&self, p: usize) -> usize {
        let p = p.min(self.s.len());
        self.s[p..].find('\n').map_or(self.s.len(), |i| p + i)
    }

    pub fn bol(&self) -> usize {
        self.bol_at(self.pt)
    }

    pub fn eol(&self) -> usize {
        self.eol_at(self.pt)
    }

    pub fn bolp(&self) -> bool {
        self.pt == self.bol()
    }

    pub fn eobp(&self) -> bool {
        self.pt >= self.s.len()
    }

    pub fn goto(&mut self, p: usize) {
        self.pt = p.min(self.s.len());
    }

    /// forward-line.
    pub fn forward_line(&mut self, n: i64) {
        if n > 0 {
            for _ in 0..n {
                let e = self.eol();
                if e >= self.s.len() {
                    self.pt = self.s.len();
                    return;
                }
                self.pt = e + 1;
            }
        } else {
            self.pt = self.bol();
            for _ in 0..-n {
                if self.pt == 0 {
                    return;
                }
                self.pt = self.bol_at(self.pt - 1);
            }
        }
    }

    /// line-beginning-position N.
    pub fn lbp(&self, n: i64) -> usize {
        let mut p = self.bol();
        if n > 1 {
            for _ in 1..n {
                let e = self.eol_at(p);
                if e >= self.s.len() {
                    return self.s.len();
                }
                p = e + 1;
            }
        } else {
            for _ in n..1 {
                if p == 0 {
                    return 0;
                }
                p = self.bol_at(p - 1);
            }
        }
        p
    }

    /// The 1-based line number of `p` (org-current-line).
    pub fn line_number(&self, p: usize) -> usize {
        self.s[..p.min(self.s.len())].matches('\n').count() + 1
    }

    pub fn char_after(&self) -> Option<u8> {
        self.s.as_bytes().get(self.pt).copied()
    }

    /// looking-at: `re` must start with `\A`.
    pub fn looking_at(&self, re: &Regex) -> Option<Caps> {
        self.looking_at_pos(re, self.pt)
    }

    pub fn looking_at_pos(&self, re: &Regex, p: usize) -> Option<Caps> {
        let c = re.captures(&self.s[p.min(self.s.len())..])?;
        Some(Caps(
            c.iter()
                .map(|m| m.map(|m| (m.start() + p, m.end() + p)))
                .collect(),
        ))
    }

    pub fn looking_p(&self, re: &Regex) -> bool {
        re.is_match(&self.s[self.pt..])
    }

    /// re-search-forward to `bound`: point after the match.
    pub fn re_search_forward(&mut self, re: &Regex, bound: usize) -> Option<Caps> {
        let bound = bound.min(self.s.len());
        if self.pt > bound {
            return None;
        }
        let c = re.captures_at(&self.s[..bound], self.pt)?;
        let caps = Caps(c.iter().map(|m| m.map(|m| (m.start(), m.end()))).collect());
        self.pt = caps.end(0).unwrap_or(self.pt);
        Some(caps)
    }

    /// re-search-backward for a regexp anchored at line beginnings (`re`
    /// starts with `\A`): point at the match.
    pub fn re_search_backward(&mut self, re: &Regex, bound: usize) -> Option<Caps> {
        let mut ls = self.bol();
        loop {
            if ls < bound {
                return None;
            }
            if let Some(c) = self.looking_at_pos(re, ls)
                && c.end(0).is_some_and(|e| e <= self.pt)
            {
                self.pt = ls;
                return Some(c);
            }
            if ls == 0 {
                return None;
            }
            ls = self.bol_at(ls - 1);
        }
    }

    pub fn skip_backward(&mut self, set: &str) {
        let set = set.as_bytes();
        while self.pt > 0 && is_ws(self.s.as_bytes()[self.pt - 1], set) {
            self.pt -= 1;
        }
    }

    pub fn skip_forward(&mut self, set: &str) {
        let set = set.as_bytes();
        while self.pt < self.s.len() && is_ws(self.s.as_bytes()[self.pt], set) {
            self.pt += 1;
        }
    }

    pub fn sub(&self, a: usize, b: usize) -> &str {
        &self.s[a.min(self.s.len())..b.min(self.s.len())]
    }

    // ---- edits ----

    fn shift_insert(&mut self, at: usize, n: usize, before_markers: bool) {
        for (p, adv) in self.marks.iter_mut().flatten() {
            if *p > at || (*p == at && (*adv || before_markers)) {
                *p += n;
            }
        }
        for f in &mut self.folds {
            if f.beg >= at {
                f.beg += n;
            }
            if f.end > at || (f.end == at && before_markers) {
                f.end += n;
            }
        }
    }

    fn insert_inner(&mut self, t: &str, before_markers: bool) {
        let at = self.pt;
        self.s.insert_str(at, t);
        self.shift_insert(at, t.len(), before_markers);
        self.pt = at + t.len();
    }

    /// insert at point; point moves after the text.
    pub fn insert(&mut self, t: &str) {
        self.insert_inner(t, false);
    }

    pub fn insert_before_markers(&mut self, t: &str) {
        self.insert_inner(t, true);
    }

    /// Insert at `p` without moving point unless it is after `p`.
    pub fn insert_at(&mut self, p: usize, t: &str) {
        let pt = self.pt;
        self.pt = p;
        self.insert(t);
        self.pt = if pt > p { pt + t.len() } else { pt };
    }

    pub fn delete(&mut self, a: usize, b: usize) {
        let (a, b) = (a.min(b), b.max(a).min(self.s.len()));
        if a >= b {
            return;
        }
        self.s.replace_range(a..b, "");
        self.folds.retain(|f| !(a <= f.beg && f.end <= b));
        let n = b - a;
        let adj = |p: &mut usize| {
            if *p >= b {
                *p -= n;
            } else if *p > a {
                *p = a;
            }
        };
        adj(&mut self.pt);
        for (p, _) in self.marks.iter_mut().flatten() {
            adj(p);
        }
        for f in &mut self.folds {
            adj(&mut f.beg);
            adj(&mut f.end);
        }
    }

    pub fn delete_extract(&mut self, a: usize, b: usize) -> String {
        let t = self.sub(a, b).to_owned();
        self.delete(a, b);
        t
    }

    pub fn marker(&mut self, p: usize, advance: bool) -> Marker {
        self.marks.push(Some((p.min(self.s.len()), advance)));
        Marker(self.marks.len() - 1)
    }

    pub fn mpos(&self, m: Marker) -> usize {
        self.marks[m.0].map_or(0, |(p, _)| p)
    }

    pub fn set_marker(&mut self, m: Marker, p: usize) {
        if let Some((q, _)) = &mut self.marks[m.0] {
            *q = p;
        }
    }

    pub fn free(&mut self, m: Marker) {
        self.marks[m.0] = None;
    }

    /// save-excursion.
    pub fn excursion<T>(&mut self, f: impl FnOnce(&mut Buf) -> T) -> T {
        let m = self.marker(self.pt, false);
        let r = f(self);
        self.pt = self.mpos(m);
        self.free(m);
        r
    }

    // ---- columns ----

    /// org-current-text-indentation of the line at `p` (tabs to 8).
    pub fn ind_at(&self, p: usize) -> usize {
        let mut col = 0;
        for c in self.s[self.bol_at(p)..].bytes() {
            match c {
                b' ' => col += 1,
                b'\t' => col = (col / 8 + 1) * 8,
                _ => break,
            }
        }
        col
    }

    pub fn ind(&self) -> usize {
        self.ind_at(self.pt)
    }

    /// indent-line-to: point keeps its place relative to the text.
    pub fn indent_line_to(&mut self, col: usize) {
        let bol = self.bol();
        let ws = self.s[bol..]
            .bytes()
            .take_while(|c| *c == b' ' || *c == b'\t')
            .count();
        if self.ind() == col && self.s[bol..bol + ws].bytes().all(|c| c == b' ') {
            if self.pt < bol + ws {
                self.pt = bol + ws;
            }
            return;
        }
        let at_text = self.pt <= bol + ws;
        let m = self.marker(self.pt, false);
        self.delete(bol, bol + ws);
        self.pt = bol;
        self.insert(&" ".repeat(col));
        if !at_text {
            self.pt = self.mpos(m);
        }
        self.free(m);
    }

    /// current-column.
    pub fn column(&self) -> usize {
        col_of(&self.s[self.bol()..self.pt])
    }

    /// org-move-to-column / move-to-column.
    pub fn move_to_column(&mut self, col: usize) {
        let bol = self.bol();
        let eol = self.eol();
        let mut c = 0;
        let mut p = bol;
        for (i, ch) in self.s[bol..eol].char_indices() {
            if c >= col {
                break;
            }
            c = if ch == '\t' { (c / 8 + 1) * 8 } else { c + 1 };
            p = bol + i + ch.len_utf8();
        }
        self.pt = p;
    }

    /// Lines `0..` of the folds, for an editor.
    pub fn line_of(&self, p: usize) -> usize {
        self.line_number(p) - 1
    }

    /// org-invisible-p at `p`.
    pub fn invisible(&self, p: usize) -> bool {
        self.folds.iter().any(|f| f.beg <= p && p <= f.end)
    }
}

/// The display column at the end of `s` (tabs to 8).
pub fn col_of(s: &str) -> usize {
    s.chars()
        .fold(0, |c, ch| if ch == '\t' { (c / 8 + 1) * 8 } else { c + 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_and_point_follow_edits() {
        let mut b = Buf::new("abc\ndef", 5);
        let m = b.marker(4, false);
        let a = b.marker(4, true);
        b.insert_at(4, "XY");
        assert_eq!((b.pt, b.mpos(m), b.mpos(a)), (7, 4, 6));
        b.delete(0, 5);
        assert_eq!((b.s.as_str(), b.pt, b.mpos(m)), ("Ydef", 2, 0));
        b.goto(0);
        b.forward_line(1);
        assert_eq!(b.pt, 4);
        let re = Regex::new(r"\A[a-z]+").unwrap();
        let mut b = Buf::new("x\nfoo\nbar\n", 9);
        assert_eq!(b.re_search_backward(&re, 0).and_then(|c| c.beg(0)), Some(6));
        assert_eq!(b.re_search_backward(&re, 0).and_then(|c| c.beg(0)), Some(2));
        assert_eq!(b.lbp(2), 6);
        assert_eq!(b.lbp(0), 0);
    }
}
