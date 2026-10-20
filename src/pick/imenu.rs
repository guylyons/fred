//! `Space i`: this buffer's functions, types, variables and headings, as
//! Emacs's imenu, from the same patterns as `Space d`. Pick one to go there.

use super::def::{BINDS, DEFINES, MODS, NOT_TYPES};
use crate::editor::Editor;
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub line: usize,
    pub name: String,
    pub kind: &'static str,
}

/// What a defining keyword makes.
fn kind_of(keyword: &str) -> &'static str {
    match keyword {
        "class" | "struct" | "enum" | "trait" | "type" | "interface" | "union" | "typedef"
        | "object" | "record" | "protocol" | "data" | "newtype" | "alias" | "actor"
        | "defstruct" => "type",
        "module" | "mod" | "namespace" | "package" | "defmodule" => "module",
        "impl" | "extension" => "impl",
        "macro_rules!" | "macro" | "defmacro" | "#define" | "#macro" => "macro",
        "const" | "static" | "readonly" | "defconst" => "constant",
        _ if BINDS.split('|').any(|b| b == keyword) => "variable",
        _ => "function",
    }
}

struct Patterns {
    /// `impl<T> Display for Foo`: the whole head is its name.
    imp: Regex,
    define: Regex,
    /// Bindings only at the top level: a function's locals aren't listed.
    bind: Regex,
    /// Go methods, JS `name = (…) =>`, `name(…) {` methods, C-style.
    other: [Regex; 4],
    heading: Regex,
}

/// Patterns with the name in group `n` (and the keyword in `k`).
fn patterns() -> &'static Patterns {
    static RES: OnceLock<Patterns> = OnceLock::new();
    RES.get_or_init(|| {
        let re = |p: &str| Regex::new(&p.replace("NAME", r"(?P<n>[\w$]+)")).unwrap();
        Patterns {
            imp: re(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?(?P<k>impl)\b(?P<n>[^{;]*)"),
            define: re(&format!(r"(?:^|[^\w#])(?P<k>{DEFINES})\s+{MODS}(?:\w+\.)?NAME")),
            bind: re(&format!(r"^(?:(?:pub(?:\([^)]*\))?|export)\s+)?(?P<k>{BINDS})\s+{MODS}NAME")),
            other: [
                re(r"\bfunc\s*\([^)]*\)\s*NAME\s*[(\[]"),
                re(r"\bNAME\s*[:=]\s*(?:async\s+)?(?:function\b|\([^)]*\)\s*=>|\w+\s*=>)"),
                re(r"^\s*(?:(?:async|static|get|set|public|private|protected)\s+)*NAME\s*\([^)]*\)\s*(?::[^{]*)?\{"),
                re(r"^\s*(?:[\w:<>,*&\[\]~]+\s+)+[*&]*(?:[\w<>]+::)*NAME\s*\([^;]*$"),
            ],
            heading: re(r"^(?P<k>#{1,6}|\*+)\s+(?P<n>.+)"),
        }
    })
}

/// The entries in `lines`; `headings` for Markdown and Org, which list
/// those instead.
pub fn entries<'a>(lines: impl Iterator<Item = &'a str>, headings: bool) -> Vec<Entry> {
    let Patterns {
        imp,
        define,
        bind,
        other,
        heading,
    } = patterns();
    let mut out = vec![];
    for (line, text) in lines.enumerate() {
        let t = text.trim_start();
        if headings {
            if let Some(c) = heading.captures(text) {
                let depth = c["k"].len();
                let name = format!("{}{}", "  ".repeat(depth - 1), c["n"].trim());
                out.push(Entry {
                    line,
                    name,
                    kind: "heading",
                });
            }
            continue;
        }
        if ["//", "/*", "* ", "--", ";", "#!"]
            .iter()
            .any(|c| t.starts_with(c))
            || t.starts_with('#') && !t.starts_with("#define") && !t.starts_with("#macro")
        {
            continue;
        }
        let (name, kind) = if let Some(c) = imp.captures(text) {
            (without_generics(c["n"].trim()).to_string(), "impl")
        } else if let Some(c) = define.captures(text).or_else(|| bind.captures(text)) {
            (c["n"].to_string(), kind_of(&c["k"]))
        } else if let Some(c) = other.iter().find_map(|re| re.captures(text)) {
            let first = t
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("");
            // A field (`view: View::default(),`) isn't a declaration.
            if NOT_TYPES.contains(&first)
                || t.split_whitespace()
                    .next()
                    .is_some_and(|w| w.ends_with(':'))
            {
                continue;
            }
            (c["n"].to_string(), "function")
        } else {
            continue;
        };
        out.push(Entry { line, name, kind });
    }
    out
}

/// `<T: Clone> Display for X<T>` without its leading `<…>`.
fn without_generics(head: &str) -> &str {
    if !head.starts_with('<') {
        return head;
    }
    let mut depth = 0;
    for (i, c) in head.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if depth == 1 => return head[i + 1..].trim_start(),
            '>' => depth -= 1,
            _ => {}
        }
    }
    head
}

/// `Space i`: pick from this buffer's entries.
pub fn open(ed: &mut Editor) {
    let headings = ed
        .path
        .as_deref()
        .and_then(|p| p.extension())
        .is_some_and(|e| matches!(e.to_str(), Some("md" | "markdown" | "org")))
        || ed.org.is_some();
    let text = ed.buf.text();
    let found = entries(text.lines(), headings);
    if found.is_empty() {
        return ed.set_err("nothing to list here");
    }
    let w = found
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(48);
    let row = |e: &Entry| format!("{:<w$}  {:<8}  {}", e.name, e.kind, e.line + 1);
    let rows: Vec<String> = found.iter().map(row).collect();
    crate::pick::choose(ed, "symbols", rows, "");
    ed.org_require_match = true;
    ed.org_then = Some(Box::new(|ed, picked| {
        // The row ends with its line number.
        if let Some(n) = picked
            .rsplit(' ')
            .next()
            .and_then(|n| n.parse::<usize>().ok())
        {
            let line = (n - 1).min(ed.line_count() - 1);
            ed.set_cursor(line, ed.first_nonblank(line));
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(text: &str, headings: bool) -> Vec<(usize, String, &'static str)> {
        entries(text.lines(), headings)
            .into_iter()
            .map(|e| (e.line, e.name, e.kind))
            .collect()
    }

    #[test]
    fn rust() {
        let text = "\
//! fn not_this
use x;
pub const MAX: usize = 3;
static mut N: u8 = 0;
pub struct Point { x: i32 }
enum Dir { Up }
impl<T: Clone> Display for Point<T> {
    pub fn new() -> Self {
        let local = 1;
        helper(local)
    }
}
pub(crate) async fn run() {}
mod tests;
macro_rules! m { () => {} }
";
        assert_eq!(
            list(text, false),
            [
                (2, "MAX".into(), "constant"),
                (3, "N".into(), "constant"),
                (4, "Point".into(), "type"),
                (5, "Dir".into(), "type"),
                (6, "Display for Point<T>".into(), "impl"),
                (7, "new".into(), "function"),
                (12, "run".into(), "function"),
                (13, "tests".into(), "module"),
                (14, "m".into(), "macro"),
            ]
        );
    }

    #[test]
    fn other_languages() {
        let py = "import os\nX = 1\nd = {\n    key: f(1),\n}\nclass A:\n    def f(self):\n        y = 2\ndef g():\n    return h(1)\n";
        assert_eq!(
            list(py, false),
            [
                (5, "A".into(), "type"),
                (6, "f".into(), "function"),
                (8, "g".into(), "function")
            ]
        );
        let js = "const a = 1;\nexport function b() {}\nconst c = (x) => x;\nclass D {\n  e(x) {\n    if (x) {\n";
        assert_eq!(
            list(js, false),
            [
                (0, "a".into(), "constant"),
                (1, "b".into(), "function"),
                (2, "c".into(), "constant"),
                (3, "D".into(), "type"),
                (4, "e".into(), "function"),
            ]
        );
        let go = "package main\nfunc (s *S) Get(k string) int {\n";
        assert_eq!(
            list(go, false),
            [(0, "main".into(), "module"), (1, "Get".into(), "function")]
        );
    }

    #[test]
    fn headings() {
        let md = "# Title\ntext\n## Install\n### From source\n";
        assert_eq!(
            list(md, true),
            [
                (0, "Title".into(), "heading"),
                (2, "  Install".into(), "heading"),
                (3, "    From source".into(), "heading"),
            ]
        );
    }
}

#[cfg(test)]
mod keys {
    use crate::editor::Mode;
    use crate::editor::tests::ed;

    #[test]
    fn space_i_jumps_to_the_pick() {
        let mut e = ed("fn alpha() {}\n\nstruct Beta;\nfn gamma() {}\n", " i");
        let Mode::Pick(p) = &e.mode else {
            panic!("{:?}", e.mode)
        };
        assert_eq!(p.rows.len(), 3);
        for k in crate::key::parse_keys("beta<Enter>") {
            e.handle_key(k);
        }
        assert_eq!((e.mode.clone(), e.cur.line), (Mode::Normal, 2));
    }
}
