//! An Emacs-like buffer (text, point, positions) for porting org.el code
//! closely. Commands load the editor's text, edit it like Lisp would, and
//! the changed lines are spliced back into Fred's buffer as one undoable
//! change; folds follow the line changes.

use crate::editor::Editor;
use regex::Regex;

pub struct EBuf {
    /// The text, lines joined with `\n` (no trailing newline added).
    pub s: String,
    /// Point: a byte offset into `s`.
    pub pt: usize,
    /// The text when loaded, for writing back only what changed.
    orig: String,
    orig_pt: usize,
}

impl EBuf {
    pub fn new(s: &str, pt: usize) -> EBuf {
        EBuf {
            s: s.to_owned(),
            pt: pt.min(s.len()),
            orig: s.to_owned(),
            orig_pt: pt,
        }
    }

    /// The editor's text with point at the cursor.
    pub fn load(ed: &Editor) -> EBuf {
        let n = ed.line_count();
        let mut s = String::new();
        let mut pt = 0;
        for l in 0..n {
            if l == ed.cur.line {
                pt = s.len() + ed.cur.byte;
            }
            s.push_str(&ed.buf.line(l));
            if l + 1 < n {
                s.push('\n');
            }
        }
        EBuf::new(&s, pt)
    }

    /// Write changed lines back (one splice) and move the cursor to point.
    pub fn store(self, ed: &mut Editor) {
        if self.s != self.orig {
            let old: Vec<&str> = self.orig.split('\n').collect();
            let new: Vec<&str> = self.s.split('\n').collect();
            let pre = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
            let max_suf = old.len().min(new.len()) - pre;
            let suf = old
                .iter()
                .rev()
                .zip(new.iter().rev())
                .take(max_suf)
                .take_while(|(a, b)| a == b)
                .count();
            let with: Vec<String> = new[pre..new.len() - suf].iter().map(|s| s.to_string()).collect();
            let remove = old.len() - suf - pre;
            if remove == 0 && with.is_empty() {
                // Nothing (cannot happen when texts differ).
            } else if remove == 0 && pre == old.len() {
                // Appending after the last line.
                super::splice(ed, old.len(), 0, &with);
            } else {
                super::splice(ed, pre, remove, &with);
            }
        }
        if self.s != self.orig || self.pt != self.orig_pt {
            let (l, b) = self.line_col(self.pt);
            ed.set_cursor(l, b);
        }
    }

    pub fn len(&self) -> usize {
        self.s.len()
    }

    pub fn is_empty(&self) -> bool {
        self.s.is_empty()
    }

    /// (line, byte in line) of `pos`.
    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.s.len());
        let l = self.s[..pos].matches('\n').count();
        (l, pos - self.bol_at(pos))
    }

    /// The position of line `l`'s start.
    pub fn pos_of_line(&self, l: usize) -> usize {
        if l == 0 {
            return 0;
        }
        self.s
            .match_indices('\n')
            .nth(l - 1)
            .map_or(self.s.len(), |(i, _)| i + 1)
    }

    pub fn line_number(&self) -> usize {
        self.line_col(self.pt).0
    }

    pub fn goto(&mut self, pos: usize) {
        self.pt = pos.min(self.s.len());
    }

    pub fn bol_at(&self, pos: usize) -> usize {
        self.s[..pos.min(self.s.len())].rfind('\n').map_or(0, |i| i + 1)
    }

    pub fn eol_at(&self, pos: usize) -> usize {
        let pos = pos.min(self.s.len());
        self.s[pos..].find('\n').map_or(self.s.len(), |i| pos + i)
    }

    pub fn bol(&self) -> usize {
        self.bol_at(self.pt)
    }

    pub fn eol(&self) -> usize {
        self.eol_at(self.pt)
    }

    /// line-beginning-position N (1 = this line).
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

    /// line-end-position N.
    pub fn lep(&self, n: i64) -> usize {
        let p = self.lbp(n);
        if n < 1 && p == 0 && self.bol() == 0 {
            return 0;
        }
        self.eol_at(p)
    }

    pub fn bolp(&self) -> bool {
        self.pt == self.bol()
    }

    pub fn eolp(&self) -> bool {
        self.pt == self.eol()
    }

    pub fn bobp(&self) -> bool {
        self.pt == 0
    }

    pub fn eobp(&self) -> bool {
        self.pt >= self.s.len()
    }

    /// forward-line: returns the lines it could not move.
    pub fn forward_line(&mut self, n: i64) -> i64 {
        if n >= 1 {
            for i in 0..n {
                let e = self.eol();
                if e >= self.s.len() {
                    self.pt = self.s.len();
                    return n - i;
                }
                self.pt = e + 1;
            }
            0
        } else {
            self.pt = self.bol();
            for i in 0..-n {
                if self.pt == 0 {
                    return -n - i;
                }
                self.pt = self.bol_at(self.pt - 1);
            }
            0
        }
    }

    pub fn beginning_of_line(&mut self) {
        self.pt = self.bol();
    }

    pub fn end_of_line(&mut self) {
        self.pt = self.eol();
    }

    pub fn line(&self) -> &str {
        &self.s[self.bol()..self.eol()]
    }

    pub fn line_at(&self, pos: usize) -> &str {
        &self.s[self.bol_at(pos)..self.eol_at(pos)]
    }

    pub fn char_after(&self, pos: usize) -> Option<char> {
        self.s.get(pos..).and_then(|r| r.chars().next())
    }

    pub fn char_before(&self, pos: usize) -> Option<char> {
        self.s.get(..pos).and_then(|r| r.chars().next_back())
    }

    pub fn substring(&self, a: usize, b: usize) -> &str {
        &self.s[a.min(b)..b.max(a).min(self.s.len())]
    }

    /// Insert at point, moving point after it.
    pub fn insert(&mut self, t: &str) {
        self.s.insert_str(self.pt, t);
        self.pt += t.len();
    }

    /// Insert at `pos`; point moves along when it is after `pos`.
    pub fn insert_at(&mut self, pos: usize, t: &str) {
        self.s.insert_str(pos, t);
        if self.pt > pos {
            self.pt += t.len();
        }
    }

    /// delete-region; point adjusts like an Emacs marker.
    pub fn delete(&mut self, a: usize, b: usize) -> String {
        let (a, b) = (a.min(b), b.max(a).min(self.s.len()));
        let out: String = self.s.drain(a..b).collect();
        if self.pt > b {
            self.pt -= b - a;
        } else if self.pt > a {
            self.pt = a;
        }
        out
    }

    /// Replace `a..b` with `t`, point after the replacement if it was inside.
    pub fn replace(&mut self, a: usize, b: usize, t: &str) {
        let old_pt = self.pt;
        self.delete(a, b);
        self.s.insert_str(a, t);
        if old_pt >= b {
            self.pt = old_pt - (b - a) + t.len();
        } else if old_pt > a {
            self.pt = a + t.len();
        }
    }

    /// looking-at: the match of `re` anchored at point.
    pub fn looking_at<'a>(&'a self, re: &Regex) -> Option<regex::Captures<'a>> {
        re.captures_at(&self.s, self.pt).filter(|c| c.get(0).unwrap().start() == self.pt)
    }

    /// Whether the line at point matches `re` from its start (org-match-line).
    pub fn match_line<'a>(&'a self, re: &Regex) -> Option<regex::Captures<'a>> {
        let b = self.bol();
        re.captures_at(&self.s[..self.eol()], b)
            .filter(|c| c.get(0).unwrap().start() == b)
    }

    /// re-search-forward: point moves to the match end.
    pub fn re_search_forward(&mut self, re: &Regex, bound: Option<usize>) -> Option<(usize, usize)> {
        let limit = bound.unwrap_or(self.s.len()).min(self.s.len());
        if self.pt > limit {
            return None;
        }
        let m = re.find_at(&self.s[..limit], self.pt)?;
        self.pt = m.end();
        Some((m.start(), m.end()))
    }

    /// re-search-backward: the last match starting before point; point
    /// moves to its start.
    pub fn re_search_backward(&mut self, re: &Regex, bound: Option<usize>) -> Option<(usize, usize)> {
        let lo = bound.unwrap_or(0);
        let mut found = None;
        // Matches anchored at each line start or anywhere: scan candidates.
        let mut start = lo;
        while start <= self.pt {
            match re.find_at(&self.s, start) {
                Some(m) if m.start() <= self.pt && m.start() >= lo => {
                    found = Some((m.start(), m.end()));
                    start = m.start() + self.s[m.start()..].chars().next().map_or(1, char::len_utf8);
                }
                _ => break,
            }
        }
        let (a, b) = found?;
        self.pt = a;
        Some((a, b))
    }

    pub fn skip_chars_forward(&mut self, set: &str, bound: Option<usize>) {
        let limit = bound.unwrap_or(self.s.len());
        while self.pt < limit {
            match self.char_after(self.pt) {
                Some(c) if set.contains(c) => self.pt += c.len_utf8(),
                _ => break,
            }
        }
    }

    pub fn skip_chars_backward(&mut self, set: &str, bound: Option<usize>) {
        let limit = bound.unwrap_or(0);
        while self.pt > limit {
            match self.char_before(self.pt) {
                Some(c) if set.contains(c) => self.pt -= c.len_utf8(),
                _ => break,
            }
        }
    }

    /// current-column (characters; tabs at 8).
    pub fn current_column(&self) -> usize {
        column(&self.s[self.bol()..self.pt])
    }

    /// move-to-column.
    pub fn move_to_column(&mut self, col: usize) {
        self.pt = self.bol();
        let mut c = 0;
        while c < col && self.pt < self.eol() {
            let ch = self.char_after(self.pt).unwrap();
            c = if ch == '\t' { (c / 8 + 1) * 8 } else { c + 1 };
            self.pt += ch.len_utf8();
        }
    }

    // ---- outline ----

    pub fn at_heading_at(&self, pos: usize) -> Option<usize> {
        super::syntax::level(self.line_at(pos))
    }

    /// org-at-heading-p.
    pub fn at_heading(&self) -> bool {
        self.at_heading_at(self.pt).is_some()
    }

    /// org-back-to-heading (point to the heading's start), false before
    /// the first heading.
    pub fn back_to_heading(&mut self) -> bool {
        let mut p = self.bol();
        loop {
            if self.at_heading_at(p).is_some() {
                self.pt = p;
                return true;
            }
            if p == 0 {
                return false;
            }
            p = self.bol_at(p - 1);
        }
    }

    /// org-current-level.
    pub fn current_level(&self) -> Option<usize> {
        let mut p = self.bol();
        loop {
            if let Some(n) = self.at_heading_at(p) {
                return Some(n);
            }
            if p == 0 {
                return None;
            }
            p = self.bol_at(p - 1);
        }
    }

    /// outline-next-heading: the start of the next heading line, or the
    /// end; true when found.
    pub fn next_heading(&mut self) -> bool {
        let mut p = self.eol();
        while p < self.s.len() {
            p += 1;
            if self.at_heading_at(p).is_some() {
                self.pt = p;
                return true;
            }
            p = self.eol_at(p);
        }
        self.pt = self.s.len();
        false
    }

    /// outline-previous-heading.
    pub fn previous_heading(&mut self) -> bool {
        let mut p = self.bol();
        while p > 0 {
            p = self.bol_at(p - 1);
            if self.at_heading_at(p).is_some() {
                self.pt = p;
                return true;
            }
        }
        self.pt = 0;
        false
    }

    /// org-end-of-subtree (invisible-ok assumed); with `to_heading` the
    /// start of the next heading, else the end of the last non-blank text.
    pub fn end_of_subtree(&mut self, to_heading: bool) -> usize {
        let lvl = if self.back_to_heading() {
            self.at_heading_at(self.pt).unwrap()
        } else {
            0
        };
        let mut p = self.eol();
        let mut end = self.s.len();
        while p < self.s.len() {
            p += 1;
            if let Some(n) = self.at_heading_at(p)
                && (lvl == 0 || n <= lvl)
            {
                end = p;
                break;
            }
            if lvl == 0 {
                // Before the first heading: point-max.
            }
            p = self.eol_at(p);
        }
        if lvl == 0 {
            end = self.s.len();
        }
        self.pt = end;
        if !to_heading && self.char_before(self.pt) == Some('\n') {
            self.pt -= 1;
            self.skip_chars_backward("\n\r\t ", None);
        }
        self.pt
    }

    /// org-up-heading-safe: to the parent heading; its level.
    pub fn up_heading_safe(&mut self) -> Option<usize> {
        let save = self.pt;
        if !self.back_to_heading() {
            self.pt = save;
            return None;
        }
        let lvl = self.at_heading_at(self.pt).unwrap();
        let mut p = self.pt;
        while p > 0 {
            p = self.bol_at(p - 1);
            if let Some(n) = self.at_heading_at(p)
                && n < lvl
            {
                self.pt = p;
                return Some(n);
            }
        }
        self.pt = save;
        None
    }

    /// org-previous-line-empty-p.
    pub fn previous_line_empty(&self) -> bool {
        let b = self.bol();
        b > 0 && self.line_at(b - 1).trim().is_empty()
    }
}

/// Columns of `s` (tabs at 8).
pub fn column(s: &str) -> usize {
    let mut c = 0;
    for ch in s.chars() {
        c = if ch == '\t' { (c / 8 + 1) * 8 } else { c + 1 };
    }
    c
}

/// A regex compiled once per call site.
#[macro_export]
macro_rules! org_re {
    ($re:expr) => {{
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new($re).unwrap())
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_and_motion() {
        let mut b = EBuf::new("* a\nbody\n** b\n* c", 5);
        assert_eq!(b.line(), "body");
        assert_eq!(b.lbp(2), 9);
        assert_eq!(b.lep(0), 3);
        assert!(b.back_to_heading());
        assert_eq!(b.pt, 0);
        b.goto(10);
        assert_eq!(b.end_of_subtree(true), 14);
        b.goto(0);
        assert_eq!(b.end_of_subtree(true), 14);
        b.goto(10);
        assert_eq!(b.up_heading_safe(), Some(1));
        b.goto(0);
        assert!(b.next_heading());
        assert_eq!(b.pt, 9);
        let re = Regex::new(r"(?m)^\*+ ").unwrap();
        b.goto(b.len());
        assert_eq!(b.re_search_backward(&re, None), Some((14, 16)));
        b.replace(14, 15, "**");
        assert_eq!(b.s, "* a\nbody\n** b\n** c");
    }
}
