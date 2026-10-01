//! Syntax highlighting with syntect, cached per line.

use crate::buffer::Buffer;
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

fn find_theme(name: &str) -> Option<Theme> {
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
        })
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
        self.syntax = syn.name.clone();
        self.disabled = if buf.len_bytes() > MAX_FILE {
            Some("file too large to highlight".into())
        } else if buf.rope().lines().any(|l| l.len_bytes() > MAX_LINE) {
            Some("line too long to highlight".into())
        } else {
            None
        };
        self.states.clear();
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
                (!r.is_empty()).then(|| (self.convert(st), r))
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

    fn convert(&self, st: sh::Style) -> Style {
        let mut s = Style::default();
        if let Some(c) = to_color(st.foreground, self.truecolor) {
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
