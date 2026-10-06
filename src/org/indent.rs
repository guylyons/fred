//! org-indent.el: org-indent-mode's virtual indentation. Each line gets
//! the line-prefix upstream computes (text under a heading of N stars is
//! shifted by N + (N-1)×(org-indent-indentation-per-level − 1) columns
//! and the boundary character; headings get hidden stars); leading stars
//! are hidden (org-indent-mode-turns-on-hiding-stars) and
//! org-adapt-indentation is off while the mode is on
//! (org-indent-mode-turns-off-org-adapt-indentation). The renderer
//! draws the prefixes (display.rs).
//!
//! Adaptation: wrapped continuation rows do not get the wrap-prefix.

use super::syntax::{self, Settings};
use super::{Prefix, options};
use crate::editor::Editor;

/// org-indent-mode in this buffer (toggled, else `#+STARTUP: indent`,
/// else org-startup-indented).
pub fn on(ed: &Editor) -> bool {
    match ed.org.as_ref().and_then(|o| o.indent_mode) {
        Some(v) => v,
        None => ed.org.is_some() && super::settings(ed).opt_bool("org-startup-indented", false),
    }
}

/// org-indent-mode keeps org-adapt-indentation off here.
pub fn adapt_off(ed: &Editor) -> bool {
    on(ed)
        && options::bool("org-indent-mode-turns-off-org-adapt-indentation", true)
        && options::string("org-adapt-indentation", "nil").trim_start_matches('\'')
            != "headline-data"
}

/// org-indent-mode hides leading stars.
pub fn hide_stars(ed: &Editor) -> bool {
    on(ed) && options::bool("org-indent-mode-turns-on-hiding-stars", true)
}

/// What a line is for org-indent-add-properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Heading(usize),
    Task(usize),
    /// Text (or an item) under a heading of this many stars.
    Text(usize),
    /// Headline data with org-adapt-indentation `headline-data`.
    Data(usize),
}

/// org-indent--compute-prefixes, one entry: the heading-prefix width.
fn indentation(per: usize, n: usize) -> usize {
    if n <= 1 { 0 } else { (per - 1) * (n - 1) }
}

/// The line prefixes of LINES (None when org-indent-indentation-per-level
/// is 0). Hidden stars are spaces.
pub fn prefixes(lines: &[String], st: &Settings) -> Vec<String> {
    let per = options::int("org-indent-indentation-per-level", 2).max(0) as usize;
    if per == 0 {
        return vec![String::new(); lines.len()];
    }
    let boundary = options::string("org-indent-boundary-char", " ")
        .chars()
        .next()
        .unwrap_or(' ');
    let headline_data =
        options::string("org-adapt-indentation", "nil").trim_start_matches('\'') == "headline-data";
    let task = super::inlinetask::min_level_stars(st);
    let first_star = options::bool("org-inlinetask-show-first-star", false);
    let mut level = 0;
    let mut out = Vec::with_capacity(lines.len());
    let mut in_drawer = false;
    for (i, l) in lines.iter().enumerate() {
        let kind = match syntax::level(l) {
            Some(n) if n >= task => Kind::Task(n),
            Some(n) => {
                level = n;
                Kind::Heading(n)
            }
            None => {
                let t = l.trim();
                let data = headline_data
                    && (super::ctx::is_planning(l)
                        || t.starts_with("CLOCK:")
                        || t.eq_ignore_ascii_case(":PROPERTIES:")
                        || (in_drawer && t.starts_with(':')));
                if t.eq_ignore_ascii_case(":PROPERTIES:") {
                    in_drawer = true;
                } else if t.eq_ignore_ascii_case(":END:") {
                    in_drawer = false;
                }
                let _ = i;
                if data {
                    Kind::Data(level)
                } else {
                    Kind::Text(level)
                }
            }
        };
        out.push(match kind {
            Kind::Heading(n) | Kind::Data(n) => " ".repeat(indentation(per, n)),
            Kind::Task(n) => {
                if n <= 1 {
                    String::new()
                } else if first_star {
                    format!("*{}", " ".repeat(indentation(per, n).saturating_sub(1)))
                } else {
                    " ".repeat(indentation(per, n))
                }
            }
            Kind::Text(n) => {
                let mut s = " ".repeat(n + indentation(per, n));
                if n > 0 {
                    s.push(boundary);
                }
                s
            }
        });
    }
    out
}

/// org-indent-mode and org-indent-indent-buffer.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-indent-mode" => {
            if ed.org.is_none() {
                return Some(Err("Not in Org mode".into()));
            }
            let new = match arg {
                Prefix::None => !on(ed),
                Prefix::Num(n) => n > 0,
                Prefix::Minus => false,
                Prefix::U(_) => true,
            };
            if let Some(o) = &mut ed.org {
                o.indent_mode = Some(new);
            }
            ed.set_msg(format!(
                "Org-Indent mode {} in current buffer",
                if new { "enabled" } else { "disabled" }
            ));
            Ok(())
        }
        "org-indent-indent-buffer" => {
            if ed.org.is_none() {
                return Some(Err("Not in Org mode".into()));
            }
            // The prefixes follow the text on every redisplay.
            ed.set_msg("Indentation of buffer set.");
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Vec<String> {
        let st = syntax::settings(text.lines(), None);
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        prefixes(&lines, &st)
    }

    #[test]
    fn prefixes_spec() {
        assert_eq!(
            p("top\n* A\ntext\n** B\n- item\n*** C\nx"),
            ["", "", "  ", " ", "    ", "  ", "      "]
        );
        options::put("org-indent-indentation-per-level", toml::Value::Integer(3));
        assert_eq!(p("* A\nt\n** B\nt"), ["", "  ", "  ", "     "]);
        options::put("org-indent-indentation-per-level", toml::Value::Integer(0));
        assert_eq!(p("* A\nt"), ["", ""]);
        options::remove("org-indent-indentation-per-level");
        options::put("org-indent-boundary-char", toml::Value::String("|".into()));
        assert_eq!(p("* A\nt"), ["", " |"]);
        options::remove("org-indent-boundary-char");
        // Inline tasks.
        let t = "*".repeat(15);
        assert_eq!(p(&format!("* A\n{t} T\nx")), ["", &" ".repeat(14), "  "]);
    }

    #[test]
    fn toggle_and_adapt() {
        let mut e = super::super::tests::org("* H\nx", "");
        crate::org::run(&mut e, "org-indent-mode", Prefix::None);
        assert!(on(&e) && adapt_off(&e) && hide_stars(&e));
        assert_eq!(
            e.msg.as_ref().unwrap().0,
            "Org-Indent mode enabled in current buffer"
        );
        crate::org::run(&mut e, "org-indent-mode", Prefix::None);
        assert!(!on(&e));
        let e = super::super::tests::org("#+STARTUP: indent\n* H", "");
        assert!(on(&e));
    }
}
