//! Syntax highlighting: tree-sitter for the bundled languages
//! (`crate::treesitter`), syntect, cached per line, for the rest.

use crate::buffer::Buffer;
use crate::treesitter::doc::TsDoc;
use crate::treesitter::langs::{self, LangSpec};
use ratatui::style::{Color, Modifier, Style};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use syntect::highlighting::{
    self as sh, FontStyle, HighlightState, RangedHighlightIterator, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

const MAX_FILE: usize = 10 * 1024 * 1024;
const MAX_LINE: usize = 20_000;

/// Styled byte ranges of one line.
pub type LineStyles = Vec<(Style, Range<usize>)>;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

pub(crate) fn find_theme(name: &str) -> Option<Theme> {
    let own = [
        ("material-dark", include_str!("../assets/themes/material-dark.tmTheme")),
        ("material-deep-ocean", include_str!("../assets/themes/material-deep-ocean.tmTheme")),
    ];
    if let Some((_, src)) = own.iter().find(|(n, _)| name.eq_ignore_ascii_case(n)) {
        return ThemeSet::load_from_reader(&mut std::io::Cursor::new(src)).ok();
    }
    let set = two_face::theme::extra();
    two_face::theme::EmbeddedLazyThemeSet::theme_names()
        .iter()
        .find(|t| t.as_name().eq_ignore_ascii_case(name))
        .map(|t| set.get(*t).clone())
        .or_else(|| ThemeSet::load_defaults().themes.get(name).cloned())
}

pub struct Highlighter {
    theme: Arc<Theme>,
    truecolor: bool,
    syntax: String,
    /// Parser state at the start of each line, for a prefix of the file.
    states: Vec<(ParseState, HighlightState)>,
    /// Why highlighting is off for this file, if it is.
    pub disabled: Option<String>,
    /// The last `styles` call ran out of time before the visible lines.
    incomplete: bool,
    /// The file's tree, when its language is bundled; syntect is not used then.
    ts: Option<TsDoc>,
}

impl Highlighter {
    pub fn new(theme: &str, truecolor: bool) -> Result<Highlighter, String> {
        let theme = find_theme(theme).ok_or_else(|| format!("unknown theme: {theme}"))?;
        Ok(Highlighter {
            theme: Arc::new(theme),
            truecolor,
            syntax: "Plain Text".into(),
            states: vec![],
            disabled: None,
            incomplete: false,
            ts: None,
        })
    }

    /// A highlighter for another buffer: same theme, nothing cached.
    pub fn blank(&self) -> Highlighter {
        Highlighter {
            theme: Arc::clone(&self.theme),
            truecolor: self.truecolor,
            syntax: "Plain Text".into(),
            states: vec![],
            disabled: None,
            incomplete: false,
            ts: None,
        }
    }

    /// Visible lines are still waiting to be highlighted: draw again soon.
    pub fn incomplete(&self) -> bool {
        self.incomplete
    }

    pub fn syntax_name(&self) -> &str {
        &self.syntax
    }

    fn syntax_ref(&self) -> &'static SyntaxReference {
        let ss = syntaxes();
        ss.find_syntax_by_name(&self.syntax)
            .unwrap_or_else(|| ss.find_syntax_plain_text())
    }

    /// Pick the language for a file and reset the cache.
    pub fn set_file(&mut self, path: Option<&Path>, buf: &Buffer) {
        let ss = syntaxes();
        let first = if buf.len_lines() > 0 {
            buf.line(0)
        } else {
            String::new()
        };
        let by_path = path.and_then(|p| {
            let ext = p.extension().and_then(|e| e.to_str());
            let name = p.file_name().and_then(|n| n.to_str());
            ext.and_then(|e| ss.find_syntax_by_extension(e))
                .or_else(|| name.and_then(|n| ss.find_syntax_by_extension(n)))
        });
        let syn = by_path
            .or_else(|| ss.find_syntax_by_first_line(&first))
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let spec = langs::for_file(path, &first);
        self.syntax = spec.map_or_else(|| syn.name.clone(), |s| s.name.into());
        self.disabled = if buf.len_bytes() > MAX_FILE {
            Some("file too large to highlight".into())
        } else if buf.rope().lines().any(|l| l.len_bytes() > MAX_LINE) {
            Some("line too long to highlight".into())
        } else {
            None
        };
        self.states.clear();
        self.ts = spec
            .filter(|_| self.disabled.is_none())
            .map(|spec| TsDoc::new(spec, Arc::clone(&self.theme), self.truecolor));
    }

    /// The file's bundled language; None on the syntect path.
    pub fn ts_spec(&self) -> Option<&'static LangSpec> {
        self.ts.as_ref().map(TsDoc::spec)
    }

    /// The file's syntax tree and its language, brought up to date with
    /// `buf` within `budget`; None for languages syntect highlights, and
    /// while the parse isn't finished.
    pub fn ts_tree(
        &mut self,
        buf: &Buffer,
        budget: Duration,
    ) -> Option<(&'static LangSpec, &tree_sitter::Tree)> {
        let ts = self.ts.as_mut()?;
        let spec = ts.spec();
        ts.tree(buf, budget).map(|tree| (spec, tree))
    }

    /// Text from `line` on has changed.
    pub fn invalidate(&mut self, line: usize) {
        self.states.truncate(line + 1);
    }

    fn ensure_start(&mut self, hl: &sh::Highlighter) {
        if self.states.is_empty() {
            self.states.push((
                ParseState::new(self.syntax_ref()),
                HighlightState::new(hl, ScopeStack::new()),
            ));
        }
    }

    /// Parse one line from its start state; returns its styles and the next state.
    fn parse(
        &self,
        hl: &sh::Highlighter,
        line_no: usize,
        buf: &Buffer,
    ) -> Result<(LineStyles, (ParseState, HighlightState)), String> {
        let (mut ps, mut hs) = self.states[line_no].clone();
        let text = buf.line(line_no);
        if text.len() > MAX_LINE {
            return Err("line too long to highlight".into());
        }
        let mut line = text;
        line.push('\n');
        let ops = ps
            .parse_line(&line, syntaxes())
            .map_err(|e| e.to_string())?;
        let end = line.len() - 1;
        let spans = RangedHighlightIterator::new(&mut hs, &ops, &line, hl)
            .filter_map(|(st, _, r)| {
                let r = r.start.min(end)..r.end.min(end);
                (!r.is_empty()).then(|| (to_style(st, self.truecolor), r))
            })
            .collect();
        Ok((spans, (ps, hs)))
    }

    /// Styles for `lines`. Lines whose start state could not be reached within
    /// `budget` are `None` (draw them plain; a later call will catch up).
    pub fn styles(
        &mut self,
        buf: &Buffer,
        lines: Range<usize>,
        budget: Duration,
    ) -> Vec<Option<LineStyles>> {
        let mut out = vec![None; lines.len()];
        self.incomplete = false;
        if self.disabled.is_some() {
            return out;
        }
        if let Some(ts) = &mut self.ts {
            let rope = buf.rope();
            let long = |l| rope.line_to_byte(l + 1) - rope.line_to_byte(l) > MAX_LINE + 1;
            if (lines.start..lines.end.min(rope.len_lines())).any(long) {
                self.disabled = Some("line too long to highlight".into());
                self.ts = None;
                return out;
            }
            let (out, incomplete) = ts.styles(buf, lines, budget);
            self.incomplete = incomplete;
            return out;
        }
        let started = Instant::now();
        let theme = Arc::clone(&self.theme);
        let hl = sh::Highlighter::new(&theme);
        self.ensure_start(&hl);
        let n = buf.len_lines();
        let want = lines.start.min(n);
        while self.states.len() <= want && self.states.len() < n {
            if started.elapsed() >= budget {
                self.incomplete = true;
                return out;
            }
            let i = self.states.len() - 1;
            match self.parse(&hl, i, buf) {
                Ok((_, next)) => self.states.push(next),
                Err(e) => {
                    self.disabled = Some(e);
                    return vec![None; lines.len()];
                }
            }
        }
        for l in lines.start..lines.end.min(n) {
            if l >= self.states.len() {
                break;
            }
            match self.parse(&hl, l, buf) {
                Ok((spans, next)) => {
                    if self.states.len() == l + 1 {
                        self.states.push(next);
                    }
                    out[l - lines.start] = Some(spans);
                }
                Err(e) => {
                    self.disabled = Some(e);
                    return vec![None; lines.len()];
                }
            }
        }
        out
    }

    /// Some visible lines are waiting to be highlighted: draw again soon
    /// (for callers that ask for several separate ranges in one frame).
    pub fn set_incomplete(&mut self) {
        self.incomplete = true;
    }

    /// Styles for one line of another file (a grep hit), parsed on its own
    /// with the language its name suggests. A line inside a multi-line
    /// comment or string can come out wrong.
    pub fn one_line(&self, path: &Path, text: &str) -> Option<LineStyles> {
        if text.len() > MAX_LINE {
            return None;
        }
        let ss = syntaxes();
        let ext = path.extension().and_then(|e| e.to_str());
        let name = path.file_name().and_then(|n| n.to_str());
        let syn = ext
            .and_then(|e| ss.find_syntax_by_extension(e))
            .or_else(|| name.and_then(|n| ss.find_syntax_by_extension(n)))?;
        let theme = Arc::clone(&self.theme);
        let hl = sh::Highlighter::new(&theme);
        let mut ps = ParseState::new(syn);
        let mut hs = HighlightState::new(&hl, ScopeStack::new());
        let line = format!("{text}\n");
        let ops = ps.parse_line(&line, ss).ok()?;
        let end = text.len();
        Some(
            RangedHighlightIterator::new(&mut hs, &ops, &line, &hl)
                .filter_map(|(st, _, r)| {
                    let r = r.start.min(end)..r.end.min(end);
                    (!r.is_empty()).then(|| (to_style(st, self.truecolor), r))
                })
                .collect(),
        )
    }

}

pub(crate) fn to_style(st: sh::Style, truecolor: bool) -> Style {
    let mut s = Style::default();
    if let Some(c) = to_color(st.foreground, truecolor) {
        s = s.fg(c);
    }
    if st.font_style.contains(FontStyle::BOLD) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if st.font_style.contains(FontStyle::ITALIC) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if st.font_style.contains(FontStyle::UNDERLINE) {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    s
}

/// syntect color → terminal color. bat's `ansi` theme stores palette indexes
/// with alpha 0 and "default color" with alpha 1.
pub fn to_color(c: sh::Color, truecolor: bool) -> Option<Color> {
    match c.a {
        0 => Some(Color::Indexed(c.r)),
        1 => None,
        _ if truecolor => Some(Color::Rgb(c.r, c.g, c.b)),
        _ => Some(Color::Indexed(rgb_to_256(c.r, c.g, c.b))),
    }
}

fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let level = |v: u8| ((v as u16 * 5 + 127) / 255) as u8;
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = 16 + 36 * ri + 6 * gi + bi;
    if r.abs_diff(g) < 10 && g.abs_diff(b) < 10 && ri == gi && gi == bi {
        let avg = (r as u16 + g as u16 + b as u16) / 3;
        if avg > 8 && avg < 238 {
            return 232 + ((avg - 8) / 10).min(23) as u8;
        }
    }
    cube
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Edit};
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn one_line_uses_the_file_names_language() {
        let h = Highlighter::new("ansi", false).unwrap();
        let st = h.one_line(Path::new("src/x.rs"), "fn main() {}").unwrap();
        assert!(
            st.iter().any(|(s, r)| r.start == 0 && s.fg.is_some()),
            "{st:?}"
        );
        assert!(
            h.one_line(Path::new("notes.zzz-unknown"), "fn main")
                .is_none()
        );
    }

    #[test]
    fn material_dark_colors_keywords_purple() {
        let h = Highlighter::new("Material-Dark", true).unwrap();
        let st = h.one_line(Path::new("x.rs"), "fn main() {}").unwrap();
        let purple = Some(Color::Rgb(0xc7, 0x92, 0xea));
        assert!(
            st.iter().any(|(s, r)| r.start == 0 && s.fg == purple),
            "{st:?}"
        );
    }

    #[test]
    fn deep_ocean_matches_nvim_styles() {
        let h = Highlighter::new("material-deep-ocean", true).unwrap();
        let st = h.one_line(Path::new("x.rs"), "fn main() { a + 1 } // hi").unwrap();
        let has = |at: usize, rgb: (u8, u8, u8), m: Modifier| {
            st.iter().any(|(s, r)| {
                r.contains(&at) && s.fg == Some(Color::Rgb(rgb.0, rgb.1, rgb.2)) && s.add_modifier == m
            })
        };
        assert!(has(0, (0xc7, 0x92, 0xea), Modifier::BOLD), "{st:?}");
        assert!(has(3, (0x82, 0xaa, 0xff), Modifier::BOLD | Modifier::ITALIC), "{st:?}");
        assert!(has(14, (0x89, 0xdd, 0xff), Modifier::empty()), "{st:?}"); // an operator is not a bold keyword
        assert!(has(22, (0x46, 0x4b, 0x5d), Modifier::ITALIC), "{st:?}");
    }

    #[test]
    fn material_dark_colors_comment_delimiters_as_comment() {
        let h = Highlighter::new("Material-Dark", true).unwrap();
        for (path, text) in [("x.rs", "// hi"), ("x.php", "<?php # hi"), ("x.c", "/* hi */")] {
            let st = h.one_line(Path::new(path), text).unwrap();
            let at = text.find(['/', '#']).unwrap();
            let mut comment = st.iter().filter(|(_, r)| r.start >= at);
            let grey = Some(Color::Rgb(0x61, 0x61, 0x61));
            let n = comment.clone().count();
            assert!(n > 0 && comment.all(|(s, _)| s.fg == grey), "{path}: {st:?}");
        }
    }

    const LONG: Duration = Duration::from_secs(5);

    fn hl() -> Highlighter {
        Highlighter::new("ansi", false).unwrap()
    }

    #[test]
    fn detects_language() {
        let mut h = hl();
        let b = Buffer::from_text("x");
        h.set_file(Some(Path::new("x.rs")), &b);
        assert_eq!(h.syntax_name(), "Rust");
        h.set_file(Some(Path::new("Makefile")), &b);
        assert_eq!(h.syntax_name(), "Makefile");
        h.set_file(None, &Buffer::from_text("#!/usr/bin/env python3\nprint(1)"));
        assert_eq!(h.syntax_name(), "Python");
        h.set_file(Some(Path::new("x.unknownext")), &b);
        assert_eq!(h.syntax_name(), "Plain Text");
    }

    #[test]
    fn unknown_theme_is_error() {
        assert!(Highlighter::new("no-such-theme", false).is_err());
        assert!(Highlighter::new("Nord", true).is_ok());
    }

    #[test]
    fn highlights_spans() {
        let mut h = hl();
        let b = Buffer::from_text("fn main() {}");
        h.set_file(Some(Path::new("a.rs")), &b);
        let s = h.styles(&b, 0..1, LONG);
        let spans = s[0].as_ref().unwrap();
        assert!(spans.len() > 1);
        assert_eq!(spans.last().unwrap().1.end, "fn main() {}".len());
    }

    #[test]
    fn multiline_state_is_invalidated_by_edits() {
        let mut h = hl();
        let mut b = Buffer::from_text("/*\nfoo\n*/\nlet");
        h.set_file(Some(Path::new("a.rs")), &b);
        let before = h.styles(&b, 1..2, LONG)[0].clone().unwrap();
        // Turn the block comment opener into a line comment.
        b.apply(Edit {
            start: 0,
            end: 2,
            text: "//".into(),
        });
        h.invalidate(b.take_dirty_from().unwrap());
        let after = h.styles(&b, 1..2, LONG)[0].clone().unwrap();
        assert_ne!(before, after, "line 1 is no longer inside a comment");
    }

    #[test]
    fn zero_budget_returns_none_for_unparsed_lines() {
        let mut h = hl();
        let text = (0..50)
            .map(|i| format!("let x{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let b = Buffer::from_text(&text);
        h.set_file(Some(Path::new("a.rs")), &b);
        assert!(h.styles(&b, 40..41, Duration::ZERO)[0].is_none());
        assert!(h.styles(&b, 40..41, LONG)[0].is_some());
    }

    #[test]
    fn huge_lines_disable_highlighting() {
        let mut h = hl();
        let b = Buffer::from_text(&"x".repeat(20_001));
        h.set_file(Some(Path::new("a.rs")), &b);
        assert!(h.disabled.is_some());
        assert!(h.styles(&b, 0..1, LONG)[0].is_none());
    }

    #[test]
    fn unbundled_languages_still_use_syntect() {
        let mut h = Highlighter::new("material-dark", true).unwrap();
        let b = Buffer::from_text("main :: IO ()\nmain = pure ()");
        h.set_file(Some(Path::new("x.hs")), &b);
        assert_eq!(h.syntax_name(), "Haskell");
        assert!(h.ts.is_none());
        assert!(h.styles(&b, 0..1, LONG)[0].as_ref().unwrap().iter().any(|(s, _)| s.fg.is_some()));
    }

    #[test]
    fn every_theme_colours_every_bundled_language() {
        // One sample per LangSpec (markdown-inline is reached through Markdown).
        // Markdown's is a fence, not a heading: GitHub and Sublime Snazzy leave headings
        // plain (with syntect too), and colour only a fence's language name.
        let samples: &[(&str, &str)] = &[("a.rs", "fn main() {}"), ("a.ts", "const x: number = 1;"), ("a.tsx", "const x = <div/>;"),
            ("a.js", "const x = 1;"), ("a.py", "def f(): pass"), ("a.go", "package main"), ("a.c", "int main(void) { return 0; }"),
            ("a.cpp", "class A {};"), ("a.swift", "func f() {}"), ("a.lua", "local x = 1"), ("a.rb", "def f; end"),
            ("A.java", "class A {}"), ("a.sh", "if true; then echo; fi"), ("a.html", "<p class=\"x\">hi</p>"), ("a.css", "a { color: red; }"),
            ("a.json", "{\"a\": 1}"), ("a.toml", "a = 1"), ("a.yaml", "a: 1"), ("a.md", "```rust"), ("a.ex", "defmodule A do end"),
            ("a.php", "<?php function f() {}"), ("a.sql", "SELECT 1;"), ("a.zig", "const x = 1;"), ("Makefile", "all:\n\techo"),
            ("Dockerfile", "FROM alpine")];
        let names: Vec<String> = two_face::theme::EmbeddedLazyThemeSet::theme_names().iter().map(|t| t.as_name().to_string())
            .chain(["material-dark".to_string(), "material-deep-ocean".to_string()]).collect();
        for theme in &names {
            for (path, text) in samples {
                let mut h = Highlighter::new(theme, true).unwrap();
                let b = Buffer::from_text(text);
                h.set_file(Some(Path::new(path)), &b);
                assert!(h.ts.is_some(), "{path} is a bundled language");
                let st = h.styles(&b, 0..1, LONG);
                assert!(st[0].as_ref().is_some_and(|l| l.iter().any(|(s, _)| s.fg.is_some() || !s.add_modifier.is_empty())), "{theme} {path}");
                // Stronger than "has a colour": the theme's plain-text colour alone doesn't count.
                let plain = to_style(sh::Highlighter::new(&h.theme).get_default(), true);
                assert!(st[0].as_ref().unwrap().iter().any(|(s, _)| *s != plain), "{theme} {path}: nothing but plain text");
            }
        }
        assert_eq!(samples.len(), langs::LANGS.len() - 1, "a sample per language but markdown-inline");
    }

    #[test]
    fn deep_ocean_styles_hold_with_tree_sitter() {
        let mut h = Highlighter::new("material-deep-ocean", true).unwrap();
        let b = Buffer::from_text("fn main() { a + 1 } // hi");
        h.set_file(Some(Path::new("x.rs")), &b);
        let st = h.styles(&b, 0..1, LONG)[0].clone().unwrap();
        let has = |at: usize, rgb: (u8, u8, u8), m: Modifier| {
            st.iter().any(|(s, r)| {
                r.contains(&at) && s.fg == Some(Color::Rgb(rgb.0, rgb.1, rgb.2)) && s.add_modifier == m
            })
        };
        assert!(has(0, (0xc7, 0x92, 0xea), Modifier::BOLD), "{st:?}");
        assert!(has(3, (0x82, 0xaa, 0xff), Modifier::BOLD | Modifier::ITALIC), "{st:?}");
        assert!(has(22, (0x46, 0x4b, 0x5d), Modifier::ITALIC), "{st:?}");
    }

    #[test]
    fn ts_tree_is_for_bundled_languages_only() {
        let mut h = hl();
        let b = Buffer::from_text("fn main() {}");
        h.set_file(Some(Path::new("a.rs")), &b);
        let (spec, tree) = h.ts_tree(&b, LONG).unwrap();
        assert_eq!((spec.name, tree.root_node().kind()), ("Rust", "source_file"));
        h.set_file(Some(Path::new("a.hs")), &b);
        assert!(h.ts_tree(&b, LONG).is_none());
    }

    #[test]
    fn color_conversion() {
        use ratatui::style::Color;
        use syntect::highlighting::Color as C;
        assert_eq!(
            to_color(
                C {
                    r: 3,
                    g: 0,
                    b: 0,
                    a: 0
                },
                false
            ),
            Some(Color::Indexed(3))
        );
        assert_eq!(
            to_color(
                C {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 1
                },
                false
            ),
            None
        );
        assert_eq!(
            to_color(
                C {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255
                },
                true
            ),
            Some(Color::Rgb(255, 0, 0))
        );
        assert_eq!(
            to_color(
                C {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255
                },
                false
            ),
            Some(Color::Indexed(196))
        );
    }
}
