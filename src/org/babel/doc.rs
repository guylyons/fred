//! The text Babel works on: buffer lines plus file and point, and the
//! Org elements Babel needs (org-element subset on lines): source
//! blocks, babel calls, inline objects, results and named data, with
//! affiliated keywords and post-blank as org-element defines them.

use crate::org::syntax;
use std::path::PathBuf;

/// A buffer's lines (each ends in a newline), file and point.
#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub lines: Vec<String>,
    /// Absolute file name, when the buffer visits one.
    pub path: Option<PathBuf>,
    /// (line, byte).
    pub point: (usize, usize),
    /// Positions that follow edits (markers).
    pub(super) marks: Vec<Option<(usize, usize)>>,
    /// Bumped by every edit (buffer-chars-modified-tick).
    pub tick: u64,
}

impl Doc {
    pub fn new(text: &str, path: Option<PathBuf>) -> Doc {
        let text = text.strip_suffix('\n').unwrap_or(text);
        Doc {
            lines: if text.is_empty() {
                vec![]
            } else {
                text.split('\n').map(str::to_owned).collect()
            },
            path,
            ..Doc::default()
        }
    }

    pub fn text(&self) -> String {
        let mut s = self.lines.join("\n");
        if !self.lines.is_empty() {
            s.push('\n');
        }
        s
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line(&self, l: usize) -> &str {
        self.lines.get(l).map_or("", String::as_str)
    }

    /// The directory of the file (default-directory).
    pub fn dir(&self) -> PathBuf {
        self.path
            .as_deref()
            .and_then(|p| p.parent().map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    pub fn mark(&mut self, pos: (usize, usize)) -> usize {
        self.marks.push(Some(pos));
        self.marks.len() - 1
    }

    pub fn marker(&self, id: usize) -> (usize, usize) {
        self.marks.get(id).copied().flatten().unwrap_or((0, 0))
    }

    pub fn unmark(&mut self, id: usize) {
        if let Some(m) = self.marks.get_mut(id) {
            *m = None;
        }
    }

    /// Replace `remove` lines at `at` with `with`; point and markers follow.
    pub fn splice(&mut self, at: usize, remove: usize, with: Vec<String>) {
        let at = at.min(self.lines.len());
        let remove = remove.min(self.lines.len() - at);
        let added = with.len();
        self.lines.splice(at..at + remove, with);
        self.tick += 1;
        let fix = |p: &mut (usize, usize)| {
            if p.0 >= at + remove {
                p.0 = p.0 + added - remove;
            } else if p.0 >= at {
                *p = (at, 0);
            }
        };
        fix(&mut self.point);
        for m in self.marks.iter_mut().flatten() {
            fix(m);
        }
    }

    pub fn insert(&mut self, at: usize, with: Vec<String>) {
        self.splice(at, 0, with);
    }

    /// Replace a line's text (markers on it keep their line).
    pub fn set(&mut self, l: usize, text: String) {
        if self.lines[l] != text {
            self.lines[l] = text;
            self.tick += 1;
        }
    }
}

/// `#+KEY: value` with KEY upcased.
pub fn keyword(line: &str) -> Option<(String, &str)> {
    syntax::keyword_line(line)
}

fn blank(l: &str) -> bool {
    l.trim().is_empty()
}

/// Affiliated keyword names, translated (org-element-keyword-translation-alist).
fn affiliated(key: &str) -> Option<&'static str> {
    Some(match key {
        "NAME" | "DATA" | "LABEL" | "RESNAME" | "SOURCE" | "SRCNAME" | "TBLNAME" => "NAME",
        "RESULTS" | "RESULT" => "RESULTS",
        "HEADER" | "HEADERS" => "HEADER",
        "CAPTION" => "CAPTION",
        "PLOT" => "PLOT",
        k if k.starts_with("ATTR_") => "ATTR",
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Heading,
    Planning,
    PropertyDrawer,
    Drawer,
    /// `#+begin_NAME` blocks other than the ones below (lower-case name).
    Block(String),
    Src,
    Example,
    Export,
    Fixed,
    Table,
    List,
    Keyword,
    Call,
    Latex,
    Comment,
    Rule,
    Paragraph,
}

/// An element: lines `begin..end` (end after the blank lines that follow).
#[derive(Clone, Debug)]
pub struct El {
    pub kind: Kind,
    /// First affiliated keyword line.
    pub begin: usize,
    /// The element's own first line.
    pub post: usize,
    /// The line after its last non-blank line.
    pub stop: usize,
    /// The line after its post-blank.
    pub end: usize,
    /// `#+name:` (TBLNAME and friends).
    pub name: Option<String>,
    /// `#+header:` values, in order.
    pub headers: Vec<String>,
    /// `#+results` keywords: (hash, value) of the affiliated RESULTS.
    pub results: Option<(Option<String>, String)>,
}

impl El {
    pub fn is(&self, k: &Kind) -> bool {
        &self.kind == k
    }
}

fn block_name(l: &str) -> Option<String> {
    let t = l.trim_start();
    let rest = t
        .get(..8)
        .filter(|p| p.eq_ignore_ascii_case("#+begin_"))
        .map(|_| &t[8..])?;
    let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

fn is_block_end(l: &str, name: &str) -> bool {
    let t = l.trim();
    t.len() == 6 + name.len()
        && t[..6].eq_ignore_ascii_case("#+end_")
        && t[6..].eq_ignore_ascii_case(name)
}

fn drawer_name(l: &str) -> Option<&str> {
    let t = l.trim();
    let n = t.strip_prefix(':')?.strip_suffix(':')?;
    (!n.is_empty()
        && n.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_'))
    .then_some(n)
}

pub fn is_fixed(l: &str) -> bool {
    let t = l.trim_start();
    t == ":" || t.starts_with(": ")
}

pub fn is_table_line(l: &str) -> bool {
    l.trim_start().starts_with('|')
}

/// A list item's indentation (columns), if `l` starts one.
fn item_indent(l: &str) -> Option<usize> {
    crate::org::ctx::item_bullet(l).map(|_| super::value::indentation(l))
}

fn starts_element(lines: &[String], i: usize) -> bool {
    let l = &lines[i];
    syntax::level(l).is_some()
        || block_name(l).is_some()
        || keyword(l).is_some()
        || is_fixed(l)
        || is_table_line(l)
        || item_indent(l).is_some()
        || l.trim_start().starts_with("\\begin{")
        || is_comment(l)
        || drawer_name(l).is_some_and(|_| drawer_close(lines, i).is_some())
        || is_rule(l)
}

fn is_comment(l: &str) -> bool {
    let t = l.trim_start();
    t == "#" || t.starts_with("# ")
}

fn is_rule(l: &str) -> bool {
    let t = l.trim();
    t.len() >= 5 && t.bytes().all(|b| b == b'-')
}

fn drawer_close(lines: &[String], i: usize) -> Option<usize> {
    (i + 1..lines.len())
        .take_while(|&j| syntax::level(&lines[j]).is_none())
        .find(|&j| lines[j].trim().eq_ignore_ascii_case(":END:"))
}

/// Parse the element starting at line `i` (not blank): its kind and the
/// line after it (before post-blank).
fn parse_one(lines: &[String], i: usize, after_heading: bool, in_list: bool) -> (Kind, usize) {
    let l = &lines[i];
    let n = lines.len();
    if syntax::level(l).is_some() {
        return (Kind::Heading, i + 1);
    }
    if after_heading && crate::org::ctx::is_planning(l) {
        return (Kind::Planning, i + 1);
    }
    if let Some(name) = block_name(l)
        && let Some(e) = (i + 1..n)
            .take_while(|&j| syntax::level(&lines[j]).is_none())
            .find(|&j| is_block_end(&lines[j], &name))
    {
        let kind = match name.as_str() {
            "src" => Kind::Src,
            "example" => Kind::Example,
            "export" => Kind::Export,
            _ => Kind::Block(name),
        };
        return (kind, e + 1);
    }
    if let Some(d) = drawer_name(l)
        && let Some(e) = drawer_close(lines, i)
    {
        let kind = if d.eq_ignore_ascii_case("PROPERTIES") {
            Kind::PropertyDrawer
        } else {
            Kind::Drawer
        };
        return (kind, e + 1);
    }
    if let Some((k, _)) = keyword(l) {
        if k == "CALL" {
            return (Kind::Call, i + 1);
        }
        if k == "TBLFM" {
            return (Kind::Keyword, i + 1);
        }
        return (Kind::Keyword, i + 1);
    }
    if is_fixed(l) {
        let e = (i..n).find(|&j| !is_fixed(&lines[j])).unwrap_or(n);
        return (Kind::Fixed, e);
    }
    if is_table_line(l) {
        let mut e = (i..n).find(|&j| !is_table_line(&lines[j])).unwrap_or(n);
        while e < n && keyword(&lines[e]).is_some_and(|(k, _)| k == "TBLFM") {
            e += 1;
        }
        return (Kind::Table, e);
    }
    if let Some(ind) = item_indent(l).filter(|_| !in_list) {
        // A list ends at two blank lines or a line indented at most like
        // its first item that is not an item.
        let mut j = i + 1;
        let mut last = i + 1;
        let mut blanks = 0;
        while j < n {
            let x = &lines[j];
            if blank(x) {
                blanks += 1;
                if blanks >= 2 {
                    break;
                }
                j += 1;
                continue;
            }
            if syntax::level(x).is_some() {
                break;
            }
            let xi = super::value::indentation(x);
            if xi <= ind && item_indent(x).is_none_or(|k| k < ind) {
                break;
            }
            // Blocks inside items are skipped whole.
            if let Some(name) = block_name(x)
                && let Some(e) = (j + 1..n).find(|&k| is_block_end(&lines[k], &name))
            {
                j = e;
            }
            blanks = 0;
            j += 1;
            last = j;
        }
        return (Kind::List, last);
    }
    if l.trim_start().starts_with("\\begin{") {
        let env: String = l.trim_start()[7..]
            .chars()
            .take_while(|&c| c != '}')
            .collect();
        let close = format!("\\end{{{env}}}");
        if let Some(e) = (i..n).find(|&j| lines[j].contains(&close)) {
            return (Kind::Latex, e + 1);
        }
    }
    if is_comment(l) {
        let e = (i..n).find(|&j| !is_comment(&lines[j])).unwrap_or(n);
        return (Kind::Comment, e);
    }
    if is_rule(l) {
        return (Kind::Rule, i + 1);
    }
    let mut e = i + 1;
    while e < n && !blank(&lines[e]) && !starts_element(lines, e) {
        e += 1;
    }
    (Kind::Paragraph, e)
}

/// The elements of lines `from..to` (a section or the whole document).
pub fn elements(lines: &[String], from: usize, to: usize) -> Vec<El> {
    elements_in(lines, from, to, false)
}

/// Elements of `from..to`; inside a list, item lines start paragraphs.
fn elements_in(lines: &[String], from: usize, to: usize, in_list: bool) -> Vec<El> {
    let mut out = vec![];
    let mut i = from;
    let mut after_heading = false;
    while i < to {
        if blank(&lines[i]) {
            i += 1;
            continue;
        }
        // Affiliated keywords, attached when an element follows directly.
        let begin = i;
        let mut name = None;
        let mut headers = vec![];
        let mut results = None;
        let mut j = i;
        while j < to
            && let Some((k, v)) = keyword(&lines[j])
            && let Some(a) = affiliated(raw_key(&lines[j]).as_str()).or_else(|| affiliated(&k))
        {
            match a {
                "NAME" => name = Some(v.to_owned()),
                "HEADER" => headers.push(v.to_owned()),
                "RESULTS" => results = Some((result_hash(&lines[j]), v.to_owned())),
                _ => {}
            }
            j += 1;
        }
        let attached = j > i
            && j < to
            && !blank(&lines[j])
            && keyword(&lines[j]).is_none_or(|(k, _)| k == "CALL");
        let post = if attached { j } else { i };
        if !attached {
            name = None;
            headers.clear();
            results = None;
        }
        let (kind, stop) = parse_one(lines, post, after_heading, in_list);
        let stop = stop.min(to);
        after_heading = kind == Kind::Heading || (after_heading && kind == Kind::Planning);
        let mut end = stop;
        while end < to && blank(&lines[end]) {
            end += 1;
        }
        // A heading's element is its own line: blank lines after it begin
        // the section.
        if kind == Kind::Heading {
            end = stop;
        }
        out.push(El {
            kind,
            begin,
            post,
            stop,
            end,
            name,
            headers,
            results,
        });
        i = end;
    }
    out
}

/// The key of a keyword line as written (before translation), upcased.
fn raw_key(l: &str) -> String {
    let t = l.trim_start();
    let rest = &t[2..];
    let end = rest.find([':', '[']).unwrap_or(rest.len());
    rest[..end].to_ascii_uppercase()
}

/// The hash in `#+RESULTS[hash]:`.
pub fn result_hash(l: &str) -> Option<String> {
    let t = l.trim_start();
    let rest = t
        .get(..9)
        .filter(|p| p.eq_ignore_ascii_case("#+results"))
        .map(|_| &t[9..])?;
    let inner = rest.strip_prefix('[')?;
    let close = inner.find(']')?;
    let mut h = &inner[..close];
    // Optional "(YYYY-MM-DD HH:MM:SS) " time stamp.
    if h.starts_with('(')
        && let Some(p) = h.find(") ")
    {
        h = &h[p + 2..];
    }
    (!h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| h.to_owned())
}

/// org-babel-result-regexp at the start of `l`: `#+results[...]:`.
pub fn is_result_line(l: &str) -> bool {
    let t = l.trim_start();
    if !t
        .get(..9)
        .is_some_and(|p| p.eq_ignore_ascii_case("#+results"))
    {
        return false;
    }
    let rest = &t[9..];
    if rest.starts_with(':') {
        return true;
    }
    rest.starts_with('[') && result_hash(l).is_some() && rest.find("]:").is_some()
}

/// The value after `#+RESULTS...:`.
pub fn result_value(l: &str) -> &str {
    let t = l.trim_start();
    match t.find(':') {
        Some(i) if is_result_line(l) => t[i + 1..].trim(),
        _ => "",
    }
}

/// The start of the section around `l` and its end (the next heading).
pub fn section(lines: &[String], l: usize) -> (usize, usize) {
    let start = (0..=l.min(lines.len().saturating_sub(1)))
        .rev()
        .find(|&i| syntax::level(&lines[i]).is_some());
    let end = (l + 1..lines.len())
        .find(|&i| syntax::level(&lines[i]).is_some())
        .unwrap_or(lines.len());
    (start.unwrap_or(0), end)
}

/// org-element-at-point at line `l` (the heading line for headings).
pub fn element_at(lines: &[String], l: usize) -> Option<El> {
    if l >= lines.len() {
        return None;
    }
    let (s, e) = section(lines, l);
    let mut el = elements(lines, s, e)
        .into_iter()
        .find(|el| el.begin <= l && l < el.end.max(el.stop))?;
    // Descend into containers.
    while let Some((a, b, in_list)) = inner(&el)
        && l >= a
        && l < b
        && let Some(x) = elements_in(lines, a, b, in_list)
            .into_iter()
            .find(|x| x.begin <= l && l < x.end.max(x.stop))
    {
        el = x;
    }
    Some(el)
}

/// The lines a container element's children live in.
fn inner(el: &El) -> Option<(usize, usize, bool)> {
    match &el.kind {
        Kind::List => Some((el.post, el.stop, true)),
        Kind::Drawer => Some((el.post + 1, el.stop - 1, false)),
        Kind::Block(n) if !matches!(n.as_str(), "comment" | "verse") => {
            Some((el.post + 1, el.stop - 1, false))
        }
        _ => None,
    }
}

/// Every element of the document, containers before their contents.
pub fn all_elements(lines: &[String]) -> Vec<El> {
    let mut out = vec![];
    fn walk(lines: &[String], els: Vec<El>, out: &mut Vec<El>) {
        for el in els {
            let kids = inner(&el).map(|(a, b, in_list)| elements_in(lines, a, b, in_list));
            out.push(el);
            if let Some(k) = kids {
                walk(lines, k, out);
            }
        }
    }
    walk(lines, elements(lines, 0, lines.len()), &mut out);
    out
}

/// What is at a position (org-element-context subset).
#[derive(Clone, Debug)]
pub enum Context {
    Src(El),
    Call(El),
    InlineSrc(Inline, El),
    InlineCall(Inline, El),
    Other(El),
}

// ---- inline objects ----

/// An inline object: `src_LANG[headers]{body}` or `call_NAME[h](args)[h]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline {
    pub line: usize,
    pub start: usize,
    /// Byte after the object.
    pub end: usize,
    pub call: bool,
    /// Language or called name.
    pub name: String,
    pub inside: Option<String>,
    pub body: String,
    pub end_header: Option<String>,
}

fn paired(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
    }
    None
}

/// Inline src blocks and babel calls on a line.
pub fn inlines(line_no: usize, l: &str) -> Vec<Inline> {
    let mut out = vec![];
    let mut from = 0;
    while from < l.len() {
        let next = [l[from..].find("src_"), l[from..].find("call_")]
            .into_iter()
            .flatten()
            .min();
        let Some(off) = next else { break };
        let start = from + off;
        from = start + 1;
        if l[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let call = l[start..].starts_with("call_");
        let rest = &l[start + if call { 5 } else { 4 }..];
        let stop: &[char] = if call {
            &[' ', '\t', '[', '(']
        } else {
            &[' ', '\t', '[', '{']
        };
        let nlen = rest.find(stop).unwrap_or(rest.len());
        if nlen == 0 {
            continue;
        }
        let name = rest[..nlen].to_owned();
        let mut p = start + (if call { 5 } else { 4 }) + nlen;
        let mut inside = None;
        if l[p..].starts_with('[') {
            let Some(e) = paired(&l[p..], '[', ']') else {
                continue;
            };
            inside = Some(l[p + 1..p + e - 1].to_owned());
            p += e;
        }
        let (open, close) = if call { ('(', ')') } else { ('{', '}') };
        if !l[p..].starts_with(open) {
            continue;
        }
        let Some(e) = paired(&l[p..], open, close) else {
            continue;
        };
        let body = l[p + 1..p + e - 1].to_owned();
        p += e;
        let mut end_header = None;
        if call
            && l[p..].starts_with('[')
            && let Some(e) = paired(&l[p..], '[', ']')
        {
            end_header = Some(l[p + 1..p + e - 1].to_owned());
            p += e;
        }
        out.push(Inline {
            line: line_no,
            start,
            end: p,
            call,
            name,
            inside,
            body,
            end_header,
        });
        from = p;
    }
    out
}

/// Elements whose text may hold inline objects.
pub fn holds_objects(k: &Kind) -> bool {
    matches!(
        k,
        Kind::Paragraph | Kind::Heading | Kind::List | Kind::Table
    )
}

/// The `{{{results(...)}}}` macro on line `l` at or after `from`
/// (whitespace only before it): its byte range.
pub fn results_macro(l: &str, from: usize) -> Option<(usize, usize)> {
    let rest = &l[from..];
    let ws = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let at = from + ws;
    if !l[at..].starts_with("{{{results(") {
        return None;
    }
    let close = l[at..].find(")}}}")?;
    Some((at, at + close + 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    #[test]
    fn elements_with_affiliated_keywords() {
        let l = lines(
            "#+name: a\n#+header: :var x=1\n#+begin_src sh\necho\n#+end_src\n\n#+RESULTS[ab12]: a\n: 1\n\n| a |\n|---|\n#+TBLFM: x\npara\ntext\n",
        );
        let els = all_elements(&l);
        assert_eq!(els[0].kind, Kind::Src);
        assert_eq!(
            (els[0].begin, els[0].post, els[0].stop, els[0].end),
            (0, 2, 5, 6)
        );
        assert_eq!(els[0].name.as_deref(), Some("a"));
        assert_eq!(els[0].headers, vec![":var x=1"]);
        assert_eq!(els[1].kind, Kind::Fixed);
        assert_eq!(els[1].results, Some((Some("ab12".into()), "a".into())));
        assert_eq!((els[2].kind.clone(), els[2].stop), (Kind::Table, 12));
        assert_eq!((els[3].kind.clone(), els[3].stop), (Kind::Paragraph, 14));
        let l = lines("#+RESULTS:\n\n: 1");
        let els = all_elements(&l);
        assert_eq!(els[0].kind, Kind::Keyword);
        assert!(is_result_line("#+RESULTS[(2020-01-01 10:00:00) ab]: x"));
        assert_eq!(result_value("#+results: x"), "x");
    }

    #[test]
    fn inline_objects() {
        let v = inlines(
            0,
            "a src_sh[:results raw]{echo {1}} b call_f[:x 1](y=2)[:results html] c",
        );
        assert_eq!(v.len(), 2);
        assert_eq!(
            (
                v[0].name.as_str(),
                v[0].inside.as_deref(),
                v[0].body.as_str()
            ),
            ("sh", Some(":results raw"), "echo {1}")
        );
        assert!(v[1].call);
        assert_eq!(
            (v[1].body.as_str(), v[1].end_header.as_deref()),
            ("y=2", Some(":results html"))
        );
        assert!(inlines(0, "xsrc_sh{a}").is_empty());
        assert_eq!(results_macro("x {{{results(=1=)}}} y", 1), Some((2, 20)));
    }
}
