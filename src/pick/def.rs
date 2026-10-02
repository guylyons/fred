//! Jump to definition (`Space d`, `gd`) without a language server: grep
//! the project for lines that look like they define the word, in most
//! languages, and rank them.

use regex::Regex;

/// Keywords that introduce a function, type or module.
const DEFINES: &str = r"fn|def|func|function|class|struct|enum|trait|type|interface|module|mod|impl|union|typedef|namespace|package|object|record|protocol|extension|actor|data|newtype|alias|macro_rules!|macro|defun|defmacro|defn|defmodule|defp|defstruct|sub|proc|#define|#macro";
/// Keywords that introduce a variable or constant.
const BINDS: &str = r"const|static|let|var|val|local|my|our|readonly|export|global|declare|defvar|defparameter|defconst|defonce";
/// Words that may sit between the keyword and the name.
const MODS: &str = r"(?:(?:pub(?:\([^)]*\))?|mut|async|unsafe|static|inline|private|public|protected|final|abstract|override|virtual|export|default|extern|const|\*|&)\s*)*";
/// Words that start a statement, not a C-style declaration.
const NOT_TYPES: &[&str] = &[
    "return", "if", "else", "elif", "while", "for", "switch", "case", "await", "new", "throw",
    "yield", "print", "assert", "not", "and", "or", "in", "do", "delete", "typeof", "echo",
];

/// Patterns for `w`, best first.
fn tiers(w: &str) -> [String; 4] {
    let w = regex::escape(w);
    [
        format!(r"(?:^|[^\w#])(?:{DEFINES})\s+{MODS}(?:\w+\.)?{w}\b"),
        format!(r"(?:^|[^\w])(?:{BINDS})\s+{MODS}{w}\b"),
        [
            // Go methods: func (r T) Name(
            format!(r"\bfunc\s*\([^)]*\)\s*{w}\s*[(\[]"),
            // JS: name = function / (…) => / x =>, and name: function
            format!(r"\b{w}\s*[:=]\s*(?:async\s+)?(?:function\b|\([^)]*\)\s*=>|\w+\s*=>)"),
            // Methods: name(…) {
            format!(r"^\s*(?:(?:async|static|get|set|public|private|protected)\s+)*{w}\s*\([^)]*\)\s*(?::[^{{]*)?\{{"),
            // C-style: type name(…) with no `;` (a call or prototype)
            format!(r"^\s*(?:[\w:<>,*&\[\]~]+\s+)+[*&]*(?:[\w<>]+::)*{w}\s*\([^;]*$"),
            // HTML ids and CSS selectors
            format!(r#"\bid\s*=\s*["']{w}["']|^\s*[.#]{w}\b[^;]*[{{,]\s*$"#),
        ]
        .join("|"),
        // Top-level assignment, shell function, Makefile/YAML key.
        format!(r"^(?:export\s+)?{w}\s*(?:=[^=]|\(\)|::?(?:[^=]|$))"),
    ]
}

/// One regex matching any line that may define `w`, for the grep.
pub fn regex(w: &str) -> Result<Regex, String> {
    if w.is_empty() || !w.chars().all(crate::complete::index::is_word_char) {
        return Err(format!("not a name: {w}"));
    }
    crate::search::compile_exact(&tiers(w).join("|"))
}

/// The tiers compiled, for `tier`.
pub fn tier_regexes(w: &str) -> Vec<Regex> {
    tiers(w).iter().filter_map(|p| Regex::new(p).ok()).collect()
}

/// How good a definition `line` is (`res` from `tier_regexes`): 0 is
/// best; None if it isn't one after all (`return foo(x)` looks like a
/// C declaration).
pub fn tier(line: &str, res: &[Regex]) -> Option<u8> {
    let t = res.iter().position(|re| re.is_match(line))?;
    let first = line
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .find(|s| !s.is_empty());
    if t == 2 && first.is_some_and(|f| NOT_TYPES.contains(&f)) {
        return None;
    }
    Some(t as u8)
}

/// The word under (or after) byte `at` of `line`.
pub fn word_at(line: &str, at: usize) -> Option<&str> {
    use crate::complete::index::is_word_char;
    let at = line.floor_char_boundary(at);
    let start = line[at..].find(is_word_char)? + at;
    // On a word: back up to its start.
    let start = if start == at {
        line[..at]
            .rfind(|c| !is_word_char(c))
            .map_or(0, |i| i + line[i..].chars().next().unwrap().len_utf8())
    } else {
        start
    };
    let end = line[start..]
        .find(|c| !is_word_char(c))
        .map_or(line.len(), |i| start + i);
    Some(&line[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_definitions_in_many_languages() {
        let res = tier_regexes("foo");
        let defs = [
            ("pub fn foo(x: u8) {", 0),
            ("    pub(crate) async fn foo<T>(", 0),
            ("def foo(self):", 0),
            ("class foo:", 0),
            ("func foo(a int) error {", 0),
            ("export default function foo() {", 0),
            ("struct foo {", 0),
            ("type foo = string", 0),
            ("#define foo 3", 0),
            ("(defun foo (x)", 0),
            ("sub foo {", 0),
            ("  def self.foo", 0),
            ("let mut foo = 1;", 1),
            ("export const foo = () => 1", 1),
            ("local foo = {}", 1),
            ("func (s *Server) foo(ctx context.Context) {", 2),
            ("  foo: function (a) {", 2),
            ("  foo = (a, b) => a + b", 2),
            ("  async foo(a) {", 2),
            ("static int *foo(const char *s)", 2),
            ("std::string Widget::foo(int a) const {", 2),
            (r#"<div id="foo">"#, 2),
            (".foo {", 2),
            ("foo = 3", 3),
            ("foo() {", 2),
            ("foo:", 3),
        ];
        for (line, want) in defs {
            assert!(regex("foo").unwrap().is_match(line), "grep misses {line:?}");
            assert_eq!(tier(line, &res), Some(want), "{line:?}");
        }
        for line in [
            "    return foo(x);",
            "  if foo(x) {",
            "bar(foo)",
            "x = foo(1)",
            "fn foobar() {",
            "let x = foo;",
            "foo == 3",
        ] {
            let grep = regex("foo").unwrap().is_match(line);
            assert!(!grep || tier(line, &res).is_none(), "{line:?} counted");
        }
    }

    #[test]
    fn the_word_at_the_cursor() {
        assert_eq!(word_at("let foo_bar = 1", 6), Some("foo_bar"));
        assert_eq!(word_at("let foo_bar = 1", 4), Some("foo_bar"));
        assert_eq!(word_at("x(  y)", 1), Some("y"));
        assert_eq!(word_at("café = 1", 3), Some("café"));
        assert_eq!(word_at("x = ", 1), None);
        assert!(regex("a b").is_err());
    }
}
