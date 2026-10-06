//! org-footnote.el: footnote references and definitions. Creation
//! (org-footnote-new with every org-footnote-auto-label mode and inline
//! definitions), the footnote section or local placement, navigation
//! with the mark ring, sorting, renumbering, normalization, deletion,
//! org-footnote-auto-adjust and org-footnote-action's menu; plus
//! org-edit-footnote-reference (org-src.el) in a special buffer.
//!
//! The functions work on an [`EBuf`] the way the Lisp works on the
//! buffer (positions, markers); the parse tree comes from the element
//! parser, reparsed when the text changes.

use super::buf::EBuf;
use super::element::{self, NodeId, Tree, Type};
use super::sexp::Sexp;
use super::syntax::Settings;
use super::{Prefix, fold, links};
use crate::editor::Editor;
use regex::Regex;

/// org-footnote-re: group 1 or 3 is the label, group 2 the inline colon.
pub const FN_RE: &str = r"\[fn:(?:([-_\w]+)?(:)|([-_\w]+)\])";
/// org-footnote-definition-re.
pub const DEF_RE: &str = r"(?m)^\[fn:([-_\w]+)\]";

fn re(s: &str) -> Regex {
    Regex::new(s).expect("a valid footnote regexp")
}

// ---- options ----

/// org-footnote-section (None: define footnotes locally).
pub fn section(st: &Settings) -> Option<String> {
    match st.opt("org-footnote-section") {
        None => Some("Footnotes".into()),
        Some(Sexp::Str(s) | Sexp::Sym(s)) if s != "nil" => Some(s),
        _ => None,
    }
}

fn auto_label(st: &Settings) -> String {
    st.opt_sym("org-footnote-auto-label", "t")
}

fn auto_adjust(st: &Settings) -> String {
    st.opt_sym("org-footnote-auto-adjust", "nil")
}

/// A footnote reference (org-footnote-at-reference-p): label, start,
/// end (before trailing blanks) and the inline definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    pub label: Option<String>,
    pub beg: usize,
    pub end: usize,
    pub def: Option<String>,
}

/// A definition (org-footnote-at-definition-p).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    pub label: String,
    pub beg: usize,
    pub end: usize,
    pub contents: String,
}

/// org-footnote-normalize-label.
pub fn normalize_label(label: &str) -> Option<String> {
    match label.trim() {
        "" => None,
        t if t.starts_with("fn:") => Some(label.get(3..).unwrap_or("").to_owned()),
        _ => Some(label.to_owned()),
    }
}

/// The buffer being worked on, with its parse tree.
pub struct F<'a> {
    pub b: EBuf,
    pub st: &'a Settings,
    cache: Option<(String, Tree)>,
}

/// Skip `set` backward from `pos`.
fn skip_back(s: &str, mut pos: usize, set: &str) -> usize {
    while pos > 0 {
        match s[..pos].chars().next_back() {
            Some(c) if set.contains(c) => pos -= c.len_utf8(),
            _ => break,
        }
    }
    pos
}

/// count-lines.
fn count_lines(s: &str, a: usize, b: usize) -> usize {
    let (a, b) = (a.min(b), a.max(b));
    let n = s[a..b].matches('\n').count();
    if a != b && !(b == 0 || s.as_bytes()[b - 1] == b'\n') {
        n + 1
    } else {
        n
    }
}

impl<'a> F<'a> {
    pub fn new(b: EBuf, st: &'a Settings) -> F<'a> {
        F { b, st, cache: None }
    }

    fn ensure(&mut self) {
        if self.cache.as_ref().is_none_or(|(s, _)| *s != self.b.s) {
            let t = element::parse_buffer(&self.b.s, self.st);
            self.cache = Some((self.b.s.clone(), t));
        }
    }

    fn tree(&self) -> &Tree {
        &self.cache.as_ref().expect("parsed").1
    }

    /// org-element-context at `pos`.
    fn context(&mut self, pos: usize) -> Option<NodeId> {
        self.ensure();
        element::context(&self.b.s, self.tree(), pos)
    }

    /// org-element-at-point at `pos`.
    fn element(&mut self, pos: usize) -> Option<NodeId> {
        self.ensure();
        element::at_point(&self.b.s, self.tree(), pos)
    }

    /// line-beginning-position 2 from `pos`.
    fn lbp2(&self, pos: usize) -> usize {
        let e = self.b.eol_at(pos);
        if e < self.b.len() { e + 1 } else { e }
    }

    // ---- predicates ----

    /// org-footnote-at-reference-p.
    pub fn at_reference(&mut self) -> Option<Ref> {
        let pt = self.b.pt;
        let c = self.context(pt)?;
        let t = self.tree();
        if t.kind(c) != Type::FootnoteReference {
            return None;
        }
        let end = skip_back(&self.b.s, t.end(c)?, " \t");
        if pt >= end {
            return None;
        }
        let inline = t.str(c, "type") == Some("inline");
        let def = if inline {
            Some(self.b.s[t.contents_begin(c)?..t.contents_end(c)?].to_owned())
        } else {
            None
        };
        Some(Ref {
            label: t.str(c, "label").map(str::to_owned),
            beg: t.begin(c)?,
            end,
            def,
        })
    }

    /// org-footnote-at-definition-p.
    pub fn at_definition(&mut self) -> Option<Def> {
        let e = self.element(self.b.pt)?;
        let t = self.tree();
        let d = t.lineage_of(e, &[Type::FootnoteDefinition], true)?;
        let label = t.str(d, "label")?.to_owned();
        let beg = t.post_affiliated(d).or(t.begin(d))?;
        let end = self.lbp2(skip_back(&self.b.s, t.end(d)?, " \r\t\n"));
        let contents = match (t.contents_begin(d), t.contents_end(d)) {
            (Some(a), Some(z)) => self.b.s[a..z].trim().to_owned(),
            _ => String::new(),
        };
        Some(Def {
            label,
            beg,
            end,
            contents,
        })
    }

    /// org-footnote--allow-reference-p.
    pub fn allow_reference(&mut self, inline: bool) -> bool {
        if !inline && self.b.bolp() {
            return false;
        }
        let pt = self.b.pt;
        let Some(c) = self.context(pt) else {
            return true;
        };
        let t = self.tree();
        let s = &self.b.s;
        if t.post_affiliated(c).is_some_and(|p| pt < p) {
            return false;
        }
        let ty = t.kind(c);
        match ty {
            Type::Paragraph => return true,
            Type::VerseBlock => {
                return t.contents_begin(c).is_some_and(|b| pt >= b)
                    && t.contents_end(c).is_some_and(|e| pt < e);
            }
            Type::Headline | Type::Inlinetask => {
                let line = self.b.line();
                if super::syntax::level(line).is_none() {
                    return true;
                }
                let end_re = crate::org_re!(r"(?i)^\*+ END[ \t]*$");
                if end_re.is_match(line) {
                    return false;
                }
                let Some(h) = super::syntax::headline(line, self.st) else {
                    return false;
                };
                if h.title.is_empty() {
                    return false;
                }
                let col = pt - self.b.bol();
                let tags = h
                    .tags_range
                    .as_ref()
                    .and_then(|r| line[r.clone()].find(':').map(|i| r.start + i));
                return col >= h.title.start && tags.is_none_or(|t| col < t);
            }
            _ => {}
        }
        let end = t.end(c).unwrap_or(s.len());
        let after = skip_back(s, end, " \r\t\n");
        let after = if t.class(c) == element::Class::Object {
            after
        } else {
            self.lbp2(after)
        };
        if pt >= after {
            return true;
        }
        if ty == Type::FootnoteDefinition {
            return self.b.char_after(pt).is_some_and(char::is_whitespace);
        }
        if t.class(c) == element::Class::Element {
            return false;
        }
        if t.begin(c) == Some(pt) {
            return true;
        }
        if ty == Type::Link {
            return false;
        }
        if ty == Type::TableCell {
            return t
                .contents_end(c)
                .is_some_and(|ce| skip_back(s, pt, " \t") <= ce);
        }
        match (t.contents_begin(c), t.contents_end(c)) {
            (Some(a), Some(z)) => pt >= a && pt <= z,
            _ => false,
        }
    }

    /// org-preserve-local-variables: run `f` with a trailing
    /// `# Local Variables:` block removed, then put it back at the end.
    pub fn preserve_local_variables<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let r = crate::org_re!(
            r"(?im)^[ \t]*#[ \t]+Local Variables:(?s:.+)^[ \t]*#[ \t]+End:[ \t]*\n?"
        );
        let from = self.b.len().saturating_sub(3000);
        let from = (from..=self.b.len())
            .find(|&i| self.b.s.is_char_boundary(i))
            .unwrap_or(0);
        let found = r
            .find_at(&self.b.s.clone(), from)
            .map(|m| (m.start(), m.end()));
        let Some((a, z)) = found else {
            return f(self);
        };
        let block = self.b.delete(a, z);
        let before = self.b.s.clone();
        let out = f(self);
        let save = self.b.mark(self.b.pt);
        if self.b.s == before {
            self.b.goto(a);
        } else {
            self.b.goto(self.b.len());
            if !self.b.bolp() {
                self.b.insert("\n");
            }
        }
        self.b.insert(&block);
        self.b.goto(self.b.marker(save));
        out
    }

    // ---- internal helpers ----

    /// org-back-over-empty-lines.
    pub fn back_over_empty_lines(&mut self) -> usize {
        let pos = self.b.pt;
        let heading = super::structure::alist_option(
            "org-blank-before-new-entry",
            "heading",
            Sexp::Sym("auto".into()),
        );
        if heading.truthy() {
            self.b.skip_chars_backward(" \t\n\r", None);
        } else if !self.b.eobp() {
            self.b.forward_line(-1);
        }
        self.b.forward_line(1);
        self.b.goto(self.b.pt.min(pos));
        count_lines(&self.b.s, self.b.pt, pos)
    }

    /// outline-next-heading with org-with-limited-levels (inline tasks
    /// are not headings).
    fn next_heading_limited(&mut self) {
        let max = super::inlinetask::min_level_stars(self.st);
        loop {
            if !self.b.next_heading() {
                return;
            }
            if self.b.current_level().is_some_and(|l| l < max) {
                return;
            }
        }
    }

    /// org-footnote--goto-local-insertion-point.
    fn goto_local_insertion_point(&mut self) {
        self.next_heading_limited();
        self.b.skip_chars_backward(" \t\n", None);
        if !self.b.bobp() {
            self.b.forward_line(1);
        }
        if !self.b.bolp() {
            self.b.insert("\n");
        }
    }

    /// org-footnote--clear-footnote-section.
    fn clear_section(&mut self) {
        let Some(sec) = section(self.st) else {
            return;
        };
        let r = re(&format!(r"(?m)^\*+ +{}[ \t]*$", regex::escape(&sec)));
        self.b.goto(0);
        while let Some((s, _)) = self.b.re_search_forward(&r, None) {
            self.b.goto(s);
            let e = self.b.end_of_subtree(true);
            self.b.delete(s, e);
            self.b.goto(s);
        }
        self.b.goto(self.b.len());
        self.b.skip_chars_backward(" \r\t\n", None);
        if !self.b.bobp() {
            self.b.forward_line(1);
            if self.b.eolp() {
                self.b.insert("\n");
            }
        }
        let (p, e) = (self.b.pt, self.b.len());
        self.b.delete(p, e);
        let heading = super::structure::alist_option(
            "org-blank-before-new-entry",
            "heading",
            Sexp::Sym("auto".into()),
        );
        if heading.truthy() {
            let save = self.b.pt;
            let n = self.back_over_empty_lines();
            self.b.goto(save);
            if n == 0 {
                self.b.insert("\n");
            }
        }
        self.b.insert(&format!("* {sec}\n"));
    }

    /// org-footnote--set-label: point at the reference or definition start.
    fn set_label(&mut self, label: &str) {
        self.b.goto(self.b.pt + 4);
        if self.b.char_after(self.b.pt) == Some(':') {
            self.b.insert(label);
        } else if let Some(m) = crate::org_re!(r"^[-_\w]+").find(&self.b.s[self.b.pt..]) {
            let (a, z) = (self.b.pt, self.b.pt + m.end());
            self.b.replace(a, z, label);
            self.b.goto(a + label.len());
        }
    }

    /// org-footnote--collect-references: (label, begin marker, top-level,
    /// inline size), in deep-reading order.
    fn collect_references(&mut self, anonymous: bool) -> Vec<Cell> {
        let r = if anonymous {
            re(FN_RE)
        } else {
            re(r"\[fn:[-_\w]+[\]:]")
        };
        let mut refs: Vec<Cell> = vec![];
        let mut nested: Vec<(String, Vec<Option<String>>)> = vec![];
        let mut from = 0;
        while let Some(m) = r.find_at(&self.b.s.clone(), from) {
            from = m.end();
            if self.b.s[..m.end()].ends_with(']') && self.b.bol_at(m.start()) == m.start() {
                continue;
            }
            let Some(o) = self.context(m.end() - 1) else {
                continue;
            };
            let t = self.tree();
            if t.kind(o) != Type::FootnoteReference {
                continue;
            }
            let label = t.str(o, "label").map(str::to_owned);
            let begin = t.begin(o).unwrap_or(m.start());
            let size = (t.str(o, "type") == Some("inline"))
                .then(|| t.contents_end(o).unwrap_or(0) - t.contents_begin(o).unwrap_or(0));
            let d = t
                .lineage_of(o, &[Type::FootnoteDefinition], false)
                .and_then(|d| t.str(d, "label").map(str::to_owned));
            let mk = self.b.mark(begin);
            refs.push(Cell {
                label: label.clone(),
                begin: mk,
                pos: begin,
                top: d.is_none(),
                size,
            });
            if let Some(dl) = d {
                match nested.iter_mut().find(|(k, _)| *k == dl) {
                    Some((_, v)) => v.push(label),
                    None => nested.push((dl, vec![label])),
                }
            }
        }
        // Nested footnotes come right after their parent definition's label.
        let mut ordered: Vec<Cell> = vec![];
        fn add(
            r: &Cell,
            allow_nested: bool,
            refs: &[Cell],
            nested: &[(String, Vec<Option<String>>)],
            ordered: &mut Vec<Cell>,
        ) {
            if !(allow_nested || r.top) {
                return;
            }
            ordered.push(r.clone());
            let kids = r
                .label
                .as_ref()
                .and_then(|l| nested.iter().find(|(k, _)| k == l))
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            for l in kids {
                // (assoc l references): the last such reference.
                let Some(x) = refs.iter().rev().find(|c| c.label == l) else {
                    continue;
                };
                if !ordered.contains(x) {
                    add(x, true, refs, nested, ordered);
                }
            }
        }
        for r in &refs {
            add(r, false, &refs, &nested, &mut ordered);
        }
        ordered
    }

    /// org-footnote--collect-definitions: (label, definition), the last
    /// one first (as the Lisp pushes them).
    fn collect_definitions(&mut self, delete: bool) -> Vec<(String, String)> {
        let r = re(DEF_RE);
        let mut defs = vec![];
        let mut seen: Vec<String> = vec![];
        self.b.goto(0);
        while let Some((_, e)) = self.b.re_search_forward(&r, None) {
            let Some(el) = self.element(e - 1) else {
                continue;
            };
            let t = self.tree();
            if t.kind(el) != Type::FootnoteDefinition {
                continue;
            }
            let Some(label) = t.str(el, "label").map(str::to_owned) else {
                continue;
            };
            if seen.contains(&label) {
                continue;
            }
            seen.push(label.clone());
            let (b0, e0) = (t.begin(el).unwrap_or(0), t.end(el).unwrap_or(0));
            let bs = skip_back(&self.b.s, b0, " \r\t\n");
            let beg = if bs == 0 { 0 } else { self.lbp2(bs) };
            let end = self.lbp2(skip_back(&self.b.s, e0, " \r\t\n"));
            defs.push((label, self.b.s[beg..end].trim().to_owned()));
            self.b.goto(end);
            if delete {
                self.b.delete(beg, end);
            }
        }
        defs.reverse();
        defs
    }

    // ---- getters ----

    /// org-footnote-get-definition.
    pub fn get_definition(&mut self, label: &str) -> Option<Def> {
        let label = normalize_label(label)?;
        let q = regex::escape(&label);
        let r = re(&format!(r"(?m)^\[fn:{q}\]|.\[fn:{q}:"));
        let mut from = 0;
        while let Some(m) = r.find_at(&self.b.s.clone(), from) {
            from = m.end();
            let Some(d) = self.context(m.end() - 1) else {
                continue;
            };
            let t = self.tree();
            if !matches!(
                t.kind(d),
                Type::FootnoteDefinition | Type::FootnoteReference
            ) {
                continue;
            }
            let contents = match (t.contents_begin(d), t.contents_end(d)) {
                (Some(a), Some(z)) => self.b.s[a..z]
                    .trim_end_matches([' ', '\t', '\n'])
                    .to_owned(),
                _ => String::new(),
            };
            return Some(Def {
                label,
                beg: t.begin(d)?,
                end: t.end(d)?,
                contents,
            });
        }
        None
    }

    /// org-footnote-all-labels.
    pub fn all_labels(&mut self) -> Vec<String> {
        let r = re(FN_RE);
        let mut all: Vec<String> = vec![];
        let mut from = 0;
        while let Some(m) = r.find_at(&self.b.s.clone(), from) {
            from = m.end();
            let Some(c) = self.context(m.end() - 1) else {
                continue;
            };
            let t = self.tree();
            if matches!(
                t.kind(c),
                Type::FootnoteDefinition | Type::FootnoteReference
            ) && let Some(l) = t.str(c, "label")
                && !all.iter().any(|x| x == l)
            {
                all.insert(0, l.to_owned());
            }
        }
        all
    }

    /// org-footnote-get-next-reference from point.
    pub fn next_reference(&mut self, label: Option<&str>, backward: bool) -> Option<Ref> {
        let r = match label {
            Some(l) => re(&format!(r"\[fn:{}[\]:]", regex::escape(l))),
            None => re(FN_RE),
        };
        let save = self.b.pt;
        let out = loop {
            let found = if backward {
                if self.b.pt == 0 {
                    None
                } else {
                    // Matches starting strictly before point.
                    self.b.pt -= 1;
                    self.b.re_search_backward(&r, None)
                }
            } else {
                self.b.re_search_forward(&r, None)
            };
            if found.is_none() {
                break None;
            }
            if !backward {
                self.b.pt -= 1;
            }
            if let Some(x) = self.at_reference() {
                break Some(x);
            }
            if !backward {
                self.b.pt += 1;
            }
        };
        self.b.goto(save);
        out
    }

    // ---- editing ----

    /// org-end-of-meta-data with FULL t, from a heading.
    fn end_of_meta_data(&mut self) {
        self.b.back_to_heading();
        self.b.forward_line(1);
        let planning = crate::org_re!(r"^[ \t]*(DEADLINE:|SCHEDULED:|CLOSED:)");
        if planning.is_match(self.b.line()) {
            self.b.forward_line(1);
        }
        let drawer = crate::org_re!(
            r"(?i)^[ \t]*:PROPERTIES:[ \t]*\n(?:[ \t]*:\S+:(?:[ \t].*)?[ \t]*\n)*?[ \t]*:END:[ \t]*(?:\n|$)"
        );
        if let Some(m) = drawer.find(&self.b.s[self.b.pt..]) {
            let e = self.b.pt + m.end();
            self.b.goto(e);
            if !self.b.bolp() {
                self.b.forward_line(1);
            }
        }
        if self.b.at_heading() {
            return;
        }
        let end = {
            let mut c = EBuf::new(&self.b.s, self.b.pt);
            c.next_heading();
            c.pt
        };
        let blank_or_clock = crate::org_re!(r"^(?:[ \t]*$|[ \t]*CLOCK:)");
        let any_drawer = crate::org_re!(r"^[ \t]*:[-_\w]+:[ \t]*$");
        let end_re = crate::org_re!(r"(?im)^[ \t]*:END:[ \t]*$");
        while !self.b.eobp() {
            let l = self.b.line();
            if blank_or_clock.is_match(l) {
                self.b.forward_line(1);
            } else if any_drawer.is_match(l) {
                match self.b.re_search_forward(end_re, Some(end)) {
                    Some(_) => {
                        self.b.forward_line(1);
                    }
                    None => return,
                }
            } else {
                return;
            }
        }
    }

    /// org-footnote-create-definition: the definition's start; point stays.
    pub fn create_definition(&mut self, label: &str) -> usize {
        self.preserve_local_variables(|x| x.create_definition_1(label))
    }

    fn create_definition_1(&mut self, label: &str) -> usize {
        let label = normalize_label(label).unwrap_or_default();
        let save = self.b.mark(self.b.pt);
        match section(self.st) {
            None => self.goto_local_insertion_point(),
            Some(sec) => {
                let r = re(&format!(r"(?m)^\*+[ \t]+{}[ \t]*$", regex::escape(&sec)));
                match r.find(&self.b.s.clone()) {
                    Some(m) => {
                        self.b.goto(m.end());
                        self.end_of_meta_data();
                        if !self.b.bolp() {
                            self.b.insert("\n");
                        }
                    }
                    None => self.clear_section(),
                }
            }
        }
        if self.back_over_empty_lines() == 0 {
            self.b.insert("\n");
        }
        self.b.insert(&format!("[fn:{label}] \n"));
        let start = self.b.lbp(0);
        self.b.goto(self.b.marker(save));
        start
    }

    /// org-footnote-delete-references.
    fn delete_references(&mut self, label: &str) -> usize {
        let save = self.b.mark(self.b.pt);
        self.b.goto(0);
        let mut n = 0;
        while let Some(r) = self.next_reference(Some(label), false) {
            self.b.goto(r.beg);
            self.b.delete(r.beg, r.end);
            n += 1;
        }
        self.b.goto(self.b.marker(save));
        n
    }

    /// org-footnote-delete-definitions.
    fn delete_definitions(&mut self, label: &str) -> usize {
        let save = self.b.mark(self.b.pt);
        let r = re(&format!(r"(?m)^\[fn:{}\]", regex::escape(label)));
        self.b.goto(0);
        let mut n = 0;
        while self.b.re_search_forward(&r, None).is_some() {
            if let Some(d) = self.at_definition() {
                let bs = skip_back(&self.b.s, d.beg, " \r\t\n");
                let a = if bs == 0 { 0 } else { self.lbp2(bs) };
                let es = skip_back(&self.b.s, d.end, " \r\t\n");
                let z = if es == 0 { 0 } else { self.lbp2(es) };
                self.b.delete(a, z);
                n += 1;
            }
        }
        self.b.goto(self.b.marker(save));
        n
    }

    /// org-footnote-delete: the message.
    pub fn delete(&mut self, label: Option<String>) -> Result<String, String> {
        self.preserve_local_variables(|x| x.delete_1(label))
    }

    fn delete_1(&mut self, label: Option<String>) -> Result<String, String> {
        let label = match label {
            Some(l) => l,
            None => match self.at_reference() {
                Some(Ref { label: Some(l), .. }) => l,
                Some(r) => {
                    self.b.delete(r.beg, r.end);
                    return Ok("Anonymous footnote removed".into());
                }
                None => match self.at_definition() {
                    Some(d) => d.label,
                    None => return Err("Don't know which footnote to remove".into()),
                },
            },
        };
        let nref = self.delete_references(&label);
        let ndef = self.delete_definitions(&label);
        self.auto_adjust_maybe();
        Ok(format!(
            "{ndef} definition(s) of and {nref} reference(s) of footnote {label} removed"
        ))
    }

    /// org-footnote-renumber-fn:N.
    pub fn renumber(&mut self) {
        let save = self.b.mark(self.b.pt);
        let numeric = |l: &str| !l.is_empty() && l.bytes().all(|c| c.is_ascii_digit());
        let refs: Vec<Cell> = self
            .collect_references(false)
            .into_iter()
            .filter(|r| r.label.as_deref().is_some_and(numeric))
            .collect();
        let mut c = 0;
        let mut alist: Vec<(String, String)> = vec![];
        for r in &refs {
            let l = r.label.clone().unwrap_or_default();
            if !alist.iter().any(|(k, _)| *k == l) {
                c += 1;
                alist.push((l, c.to_string()));
            }
        }
        let get = |l: &str, alist: &[(String, String)]| {
            alist.iter().find(|(k, _)| k == l).map(|(_, v)| v.clone())
        };
        for r in &refs {
            self.b.goto(self.b.marker(r.begin));
            let new = get(r.label.as_deref().unwrap_or(""), &alist).unwrap_or_default();
            self.set_label(&new);
        }
        let r = crate::org_re!(r"(?m)^\[fn:([0-9]+)\]");
        self.b.goto(0);
        while let Some(caps) = r.captures_at(&self.b.s.clone(), self.b.pt) {
            let g = caps.get(1).unwrap();
            let new = get(g.as_str(), &alist).unwrap_or_else(|| {
                c += 1;
                c.to_string()
            });
            self.b.replace(g.start(), g.end(), &new);
            self.b.goto(g.start() + new.len() + 1);
        }
        self.b.goto(self.b.marker(save));
    }

    /// org-footnote-sort.
    pub fn sort(&mut self) {
        let refs = self.collect_references(false);
        self.preserve_local_variables(|x| x.sort_1(refs))
    }

    fn sort_1(&mut self, refs: Vec<Cell>) {
        let save = self.b.mark(self.b.pt);
        let defs = self.collect_definitions(true);
        self.clear_section();
        let sec = section(self.st).is_some();
        let mut inserted: Vec<String> = vec![];
        for cell in &refs {
            let label = cell.label.clone().unwrap_or_default();
            if inserted.contains(&label) || cell.size.is_some() {
                continue;
            }
            inserted.push(label.clone());
            if !(sec || !cell.top) {
                self.b.goto(self.b.marker(cell.begin));
                self.goto_local_insertion_point();
            }
            let def = defs
                .iter()
                .find(|(k, _)| *k == label)
                .map(|(_, d)| d.clone())
                .unwrap_or_else(|| format!("[fn:{label}] DEFINITION NOT FOUND."));
            self.b.insert(&format!("\n{def}\n"));
        }
        for (label, def) in &defs {
            if !inserted.contains(label) {
                self.b.insert(&format!("\n{def}\n"));
            }
        }
        self.b.goto(self.b.marker(save));
    }

    /// org-footnote-normalize.
    pub fn normalize(&mut self) {
        self.preserve_local_variables(F::normalize_1)
    }

    fn normalize_1(&mut self) {
        let save = self.b.mark(self.b.pt);
        let mut n = 0usize;
        let mut translations: Vec<(String, String)> = vec![];
        let mut definitions: Vec<(Key, String)> = vec![];
        let refs = self.collect_references(true);
        let mut keys: Vec<Key> = vec![];
        for cell in &refs {
            let (key, new) = match &cell.label {
                None => {
                    n += 1;
                    (Key::Anon(n), n.to_string())
                }
                Some(l) => match translations.iter().find(|(k, _)| k == l) {
                    Some((_, v)) => (Key::Named(l.clone()), v.clone()),
                    None => {
                        n += 1;
                        translations.push((l.clone(), n.to_string()));
                        (Key::Named(l.clone()), n.to_string())
                    }
                },
            };
            self.b.goto(self.b.marker(cell.begin));
            self.set_label(&new);
            if let Some(size) = cell.size {
                let p = self.b.pt;
                let z = (p + size + 1).min(self.b.len());
                let text = self.b.delete(p, z);
                let def = format!("[fn:{new}] {}", text.get(1..).unwrap_or("").trim());
                definitions.push((key.clone(), def));
            }
            keys.push(key);
        }
        for (l, d) in self.collect_definitions(true) {
            definitions.push((Key::Named(l), d));
        }
        self.clear_section();
        let sec = section(self.st).is_some();
        let mut inserted: Vec<Key> = vec![];
        let first_label = crate::org_re!(r"^\[fn:(.*?)\]");
        for (cell, key) in refs.iter().zip(&keys) {
            if !(sec || !cell.top) {
                self.b.goto(self.b.marker(cell.begin));
                self.goto_local_insertion_point();
            }
            if inserted.contains(key) {
                continue;
            }
            inserted.push(key.clone());
            let stored = definitions
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, d)| d.clone());
            let new = match key {
                Key::Anon(i) => i.to_string(),
                Key::Named(l) => translations
                    .iter()
                    .find(|(k, _)| k == l)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default(),
            };
            let text = match stored {
                None => format!("[fn:{new}] DEFINITION NOT FOUND."),
                Some(d) if matches!(key, Key::Anon(_)) => d,
                Some(d) => first_label
                    .replace(&d, |_: &regex::Captures| format!("[fn:{new}]"))
                    .into_owned(),
            };
            self.b.insert(&format!("\n{text}\n"));
        }
        let def_re = re(DEF_RE);
        for (key, def) in &definitions {
            if !inserted.contains(key) {
                n += 1;
                let d = def_re.replace_all(def, format!("[fn:{n}]").as_str());
                self.b.insert(&format!("\n{d}\n"));
            }
        }
        self.b.goto(self.b.marker(save));
    }

    /// org-footnote-auto-adjust-maybe.
    pub fn auto_adjust_maybe(&mut self) {
        let aa = auto_adjust(self.st);
        if matches!(aa.as_str(), "t" | "renumber") {
            self.renumber();
        }
        if matches!(aa.as_str(), "t" | "sort") {
            let label = self.at_definition().map(|d| d.label);
            self.sort();
            if let Some(l) = label {
                let r = re(&format!(r"(?m)^\[fn:{}\]", regex::escape(&l)));
                self.b.goto(0);
                if self.b.re_search_forward(&r, None).is_some() {
                    self.b.insert(" ");
                    // just-one-space
                    let mut a = self.b.pt;
                    while a > 0 && matches!(self.b.char_before(a), Some(' ' | '\t')) {
                        a -= 1;
                    }
                    let mut z = self.b.pt;
                    while matches!(self.b.char_after(z), Some(' ' | '\t')) {
                        z += 1;
                    }
                    self.b.delete(a, z);
                    self.b.goto(a);
                    self.b.insert(" ");
                }
            }
        }
    }
}

/// A collected reference.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell {
    label: Option<String>,
    /// Marker handle of the reference start.
    begin: usize,
    /// The start when collected (identity, as the Lisp marker's).
    pos: usize,
    top: bool,
    size: Option<usize>,
}

/// org-footnote-normalize's labels: named, or anonymous by number.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Named(String),
    Anon(usize),
}

// ---- editor commands ----

fn with<R>(ed: &mut Editor, f: impl FnOnce(&mut F) -> R) -> R {
    let st = super::settings(ed);
    let mut x = F::new(EBuf::load_emacs(ed), &st);
    let r = f(&mut x);
    x.b.store_emacs(ed);
    r
}

/// org-footnote-goto-definition.
pub fn goto_definition(
    ed: &mut Editor,
    label: &str,
    location: Option<usize>,
) -> Result<(), String> {
    let label = normalize_label(label).unwrap_or_default();
    let st = super::settings(ed);
    let mut x = F::new(EBuf::load_emacs(ed), &st);
    let start = match location {
        Some(p) => p,
        None => x
            .get_definition(&label)
            .map(|d| d.beg)
            .ok_or_else(|| format!("Cannot find definition of footnote {label}"))?,
    };
    let (l, _) = x.b.line_col(start);
    if let Some((lo, hi)) = fold::narrowed(ed)
        && (l < lo || l > hi)
    {
        return Err("Definition is outside narrowed part of buffer".into());
    }
    links::mark_ring_push(ed);
    let r = re(&format!(r"^\[fn:{}[\]:]", regex::escape(&label)));
    let pos = r.find(&x.b.s[start..]).map_or(start, |m| start + m.end());
    let (l, b) = x.b.line_col(pos);
    ed.set_cursor(l, b);
    fold::show_context_for(ed, l, "link-search");
    ed.set_msg("Edit definition and go back with `C-c &' or, if unique, with `C-c C-c'.");
    Ok(())
}

/// org-footnote-goto-previous-reference.
pub fn goto_previous_reference(ed: &mut Editor, label: &str) -> Result<(), String> {
    let label = normalize_label(label).unwrap_or_default();
    let st = super::settings(ed);
    let mut x = F::new(EBuf::load_emacs(ed), &st);
    let r = x
        .next_reference(Some(&label), true)
        .or_else(|| x.next_reference(Some(&label), false))
        .ok_or_else(|| format!("Cannot find reference of footnote {label:?}"))?;
    let (l, b) = x.b.line_col(r.beg);
    if let Some((lo, hi)) = fold::narrowed(ed)
        && (l < lo || l > hi)
    {
        return Err("Reference is outside narrowed part of buffer".into());
    }
    links::mark_ring_push(ed);
    ed.set_cursor(l, b);
    fold::show_context_for(ed, l, "link-search");
    Ok(())
}

/// 32 random bits as hex (`(format "%x" (abs (random)))`).
fn random_label() -> String {
    let mut bytes = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    }
    let v = u32::from_le_bytes(bytes) ^ (super::now() as u32);
    format!("{v:x}")
}

/// org-footnote-new.
pub fn new(ed: &mut Editor) -> Result<(), String> {
    let st = super::settings(ed);
    let all = F::new(EBuf::load_emacs(ed), &st).all_labels();
    let mode = auto_label(&st);
    match mode.as_str() {
        "anonymous" => insert_new(ed, None, &all),
        "random" => insert_new(ed, Some(random_label()), &all),
        "t" => insert_new(ed, Some(unique_label(&all)), &all),
        _ => {
            let propose = if mode == "confirm" {
                unique_label(&all)
            } else {
                String::new()
            };
            super::read(
                ed,
                "Label (leave empty for anonymous): ",
                &propose,
                move |ed, ans| {
                    ed.undo.begin(ed.cur.pos());
                    if let Err(e) = insert_new(ed, normalize_label(&ans), &all) {
                        ed.set_err(e);
                    }
                    ed.undo.end(ed.cur.pos());
                },
            );
            Ok(())
        }
    }
}

/// org-footnote-unique-label.
pub fn unique_label(current: &[String]) -> String {
    let mut n = 1;
    while current.iter().any(|l| *l == n.to_string()) {
        n += 1;
    }
    n.to_string()
}

fn insert_new(ed: &mut Editor, label: Option<String>, all: &[String]) -> Result<(), String> {
    let st = super::settings(ed);
    let inline = st.opt_bool("org-footnote-define-inline", false);
    let refuse = || Err("Cannot insert a footnote here".to_owned());
    let mut x = F::new(EBuf::load_emacs(ed), &st);
    let Some(label) = label else {
        if !x.allow_reference(true) {
            return refuse();
        }
        x.b.insert("[fn::]");
        x.b.pt -= 1;
        x.b.store_emacs(ed);
        return Ok(());
    };
    if all.contains(&label) {
        if !x.allow_reference(false) {
            return refuse();
        }
        x.b.insert(&format!("[fn:{label}]"));
        x.b.store_emacs(ed);
        ed.set_msg("New reference to existing note");
        return Ok(());
    }
    if inline {
        if !x.allow_reference(true) {
            return refuse();
        }
        x.b.insert(&format!("[fn:{label}:]"));
        x.b.pt -= 1;
        x.auto_adjust_maybe();
        x.b.store_emacs(ed);
        return Ok(());
    }
    if !x.allow_reference(false) {
        return refuse();
    }
    x.b.insert(&format!("[fn:{label}]"));
    let p = x.create_definition(&label);
    x.b.store_emacs(ed);
    let ok = goto_definition(ed, &label, Some(p)).is_ok();
    with(ed, |x| x.auto_adjust_maybe());
    if !ok {
        edit_reference(ed)?;
    }
    Ok(())
}

/// org-footnote-action.
pub fn action(ed: &mut Editor, special: bool) -> Result<(), String> {
    let st = super::settings(ed);
    let mut x = F::new(EBuf::load_emacs(ed), &st);
    let pt = x.b.pt;
    let ctx = if special { None } else { x.context(pt) };
    if let Some(c) = ctx {
        let t = x.tree();
        let end = t.end(c).unwrap_or(x.b.len());
        if pt > skip_back(&x.b.s, end, " \t") {
            return new(ed);
        }
        match t.kind(c) {
            Type::FootnoteReference => {
                let Some(label) = t.str(c, "label").map(str::to_owned) else {
                    let cb = t.contents_begin(c).unwrap_or(pt);
                    let (l, b) = x.b.line_col(cb);
                    ed.set_cursor(l, b);
                    return Ok(());
                };
                if let Some(d) = x.get_definition(&label) {
                    return goto_definition(ed, &label, Some(d.beg));
                }
                super::yes_or_no(
                    ed,
                    &format!("No definition for {label}.  Create one? "),
                    move |ed| {
                        ed.undo.begin(ed.cur.pos());
                        let p = with(ed, |x| x.create_definition(&label));
                        if goto_definition(ed, &label, Some(p)).is_err()
                            && let Err(e) = edit_reference(ed)
                        {
                            ed.set_err(e);
                        }
                        ed.undo.end(ed.cur.pos());
                    },
                );
                return Ok(());
            }
            Type::FootnoteDefinition => {
                let label = t.str(c, "label").unwrap_or("").to_owned();
                return goto_previous_reference(ed, &label);
            }
            _ => {}
        }
    }
    if special || !x.allow_reference(false) {
        let entries = [
            ("s", "sort"),
            ("r", "renumber fn:N"),
            ("S", "renumber and sort"),
            ("n", "normalize"),
            ("d", "delete"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        super::menu(
            ed,
            "Footnotes: [s]ort | [r]enumber fn:N | [S]=r+s | [n]ormalize | [d]elete",
            entries,
            |ed, k| {
                ed.undo.begin(ed.cur.pos());
                let r = match k.as_str() {
                    "s" => {
                        with(ed, |x| x.sort());
                        Ok(())
                    }
                    "r" => {
                        with(ed, |x| x.renumber());
                        Ok(())
                    }
                    "S" => {
                        with(ed, |x| {
                            x.renumber();
                            x.sort();
                        });
                        Ok(())
                    }
                    "n" => {
                        with(ed, |x| x.normalize());
                        Ok(())
                    }
                    _ => delete_cmd(ed, None),
                };
                if let Err(e) = r {
                    ed.set_err(e);
                }
                ed.undo.end(ed.cur.pos());
            },
        );
        return Ok(());
    }
    new(ed)
}

fn delete_cmd(ed: &mut Editor, label: Option<String>) -> Result<(), String> {
    let msg = with(ed, |x| x.delete(label))?;
    ed.set_msg(msg);
    Ok(())
}

/// org-edit-footnote-reference: edit the definition of the reference at
/// point in a `*Edit footnote [LABEL]*` buffer (C-c ' writes it back,
/// C-c C-k aborts). The label part stays as upstream's read-only prefix:
/// changing it aborts the write-back.
pub fn edit_reference(ed: &mut Editor) -> Result<(), String> {
    let st = super::settings(ed);
    let mut x = F::new(EBuf::load_emacs(ed), &st);
    let pt = x.b.pt;
    let c = x
        .context(pt)
        .filter(|&c| {
            let t = x.tree();
            t.kind(c) == Type::FootnoteReference
                && t.begin(c).is_some_and(|b| pt >= b)
                && t.end(c)
                    .is_some_and(|e| pt < e - t.post_blank(c) || pt == b_of(t, c))
        })
        .ok_or("Not on a footnote reference")?;
    let label = x
        .tree()
        .str(c, "label")
        .map(str::to_owned)
        .ok_or("Cannot edit remotely anonymous footnotes")?;
    let (prefix, contents, inline) = definition_text(&mut x, &label)?;
    let name = format!("*Edit footnote [{label}]*");
    let path = ed.path.clone();
    let cursor = (0, prefix.len().min(contents.len()));
    super::effect(ed, move |s| {
        let origin = s.org_current();
        s.org_special(
            &name,
            &contents,
            cursor,
            Box::new(move |s, text, abort| {
                if abort {
                    return;
                }
                let apply = move |ed: &mut Editor| {
                    let Some(body) = text.strip_prefix(prefix.as_str()) else {
                        return ed.set_err("Cannot edit footnote label");
                    };
                    let mut body = body.to_owned();
                    if inline {
                        if body.ends_with(']') {
                            body.pop();
                        }
                        if crate::org_re!(r"\n[ \t]*\n").is_match(&body) {
                            return ed.set_err("Inline definitions cannot contain blank lines");
                        }
                    }
                    let st = super::settings(ed);
                    let mut x = F::new(EBuf::load_emacs(ed), &st);
                    let Some((a, z, in_cell)) = definition_body(&mut x, &label) else {
                        return ed.set_err("Source buffer disappeared.  Aborting");
                    };
                    if in_cell {
                        body = body.replace('\n', " ");
                    }
                    ed.undo.begin(ed.cur.pos());
                    let save = x.b.pt;
                    x.b.replace(a, z, &body);
                    x.b.goto(save.min(x.b.len()));
                    x.b.store_emacs(ed);
                    ed.undo.end(ed.cur.pos());
                };
                match &path {
                    Some(p) => {
                        let _ = s.org_with_file(p, apply);
                    }
                    None => s.org_with_buffer(origin.min(s.org_buffer_count() - 1), apply),
                }
            }),
        );
    });
    Ok(())
}

fn b_of(t: &Tree, c: NodeId) -> usize {
    t.begin(c).unwrap_or(0)
}

/// The definition of LABEL as the edit buffer shows it: (label prefix,
/// whole text, inline).
fn definition_text(x: &mut F, label: &str) -> Result<(String, String, bool), String> {
    let d = x
        .get_definition(label)
        .ok_or_else(|| format!("Cannot find definition of footnote {label}"))?;
    let r = re(&format!(r"^\[fn:{}[\]:]", regex::escape(label)));
    let after = r.find(&x.b.s[d.beg..]).map_or(d.beg, |m| d.beg + m.end());
    let el = x.context(after.saturating_sub(1)).ok_or("No definition")?;
    let t = x.tree();
    let inline = t.kind(el) == Type::FootnoteReference;
    let start = t.post_affiliated(el).or(t.begin(el)).unwrap_or(d.beg);
    let end = if inline {
        t.contents_end(el).map_or(x.b.len(), |e| e + 1)
    } else {
        match t.contents_end(el) {
            Some(e) => e,
            None => x.b.eol_at(start),
        }
    };
    let contents = x.b.s[start..end].to_owned();
    let pre = if inline {
        crate::org_re!(r"^\[fn:.*?:").find(&contents)
    } else {
        crate::org_re!(r"^.*?\]").find(&contents)
    };
    let prefix = pre.map_or(String::new(), |m| contents[..m.end()].to_owned());
    Ok((prefix, contents, inline))
}

/// The editable part of LABEL's definition: (start, end, in a table cell).
fn definition_body(x: &mut F, label: &str) -> Option<(usize, usize, bool)> {
    let d = x.get_definition(label)?;
    let r = re(&format!(r"^\[fn:{}[\]:]", regex::escape(label)));
    let after = d.beg + r.find(&x.b.s[d.beg..])?.end();
    let el = x.context(after - 1)?;
    let t = x.tree();
    let in_cell = t.lineage_of(el, &[Type::TableCell], false).is_some();
    if t.kind(el) == Type::FootnoteReference {
        return Some((after, t.contents_end(el)?, in_cell));
    }
    let end = t.contents_end(el).unwrap_or_else(|| x.b.eol_at(after));
    Some((after, end.max(after), in_cell))
}

/// A label argument: from [`super::call_with`], else read.
fn with_label(
    ed: &mut Editor,
    f: impl FnOnce(&mut Editor, &str) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    if let Some(l) = super::take_call_arg() {
        return f(ed, &l);
    }
    super::read(ed, "Label: ", "", move |ed, l| {
        if let Err(e) = f(ed, &l) {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// The footnote commands.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-footnote-action" => action(ed, !arg.is_none()),
        "org-footnote-new" => new(ed),
        "org-footnote-normalize" => {
            with(ed, |x| x.normalize());
            Ok(())
        }
        "org-footnote-renumber-fn:N" => {
            with(ed, |x| x.renumber());
            Ok(())
        }
        "org-footnote-sort" => {
            with(ed, |x| x.sort());
            Ok(())
        }
        "org-footnote-delete" => delete_cmd(ed, super::take_call_arg()),
        "org-footnote-goto-definition" => with_label(ed, |ed, l| goto_definition(ed, l, None)),
        "org-footnote-goto-previous-reference" => with_label(ed, goto_previous_reference),
        "org-edit-footnote-reference" => edit_reference(ed),
        _ => return None,
    })
}

#[cfg(test)]
pub(crate) mod tests;
