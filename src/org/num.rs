//! org-num.el: org-num-mode numbers headlines (`1.2 ` after the stars),
//! with org-num-max-level, org-num-skip-commented, -footnotes, -tags and
//! -unnumbered. The renderer draws the numbers (see display.rs); they
//! are recomputed when the text changes, as org-num--verify does.
//!
//! Adaptation: org-num-format-function is the default format (a custom
//! Lisp function cannot run); org-num-face nil uses the headline's face.

use super::props::{Doc, Inherit};
use super::sexp::Sexp;
use super::syntax::{self, Settings};
use super::{Prefix, options};
use crate::editor::Editor;

/// org-num-mode in this buffer (toggled, else `#+STARTUP: num`, else
/// org-startup-numerated).
pub fn on(ed: &Editor) -> bool {
    match ed.org.as_ref().and_then(|o| o.num_mode) {
        Some(v) => v,
        None => ed.org.is_some() && super::settings(ed).opt_bool("org-startup-numerated", false),
    }
}

/// org-num-default-format.
pub fn default_format(numbering: &[i64]) -> String {
    let parts: Vec<String> = numbering.iter().map(i64::to_string).collect();
    format!("{} ", parts.join("."))
}

/// org-num-skip-tags.
fn skip_tags() -> Vec<String> {
    match super::sexp::option("org-num-skip-tags") {
        Some(v) => v
            .list()
            .unwrap_or(&[])
            .iter()
            .filter_map(|x| x.str().map(str::to_owned))
            .collect(),
        None => vec![],
    }
}

/// The numbers of LINES: for each headline line the display string, or
/// None (not a numbered headline).
pub fn numbering(
    lines: &[String],
    st: &Settings,
    path: Option<&std::path::Path>,
) -> Vec<Option<String>> {
    let odd = st.opt_bool("org-odd-levels-only", false);
    let max_stars = match st.opt("org-num-max-level") {
        Some(Sexp::Int(n)) if n >= 0 => {
            let n = n as usize;
            if odd { (2 * n).saturating_sub(1) } else { n }
        }
        _ => super::inlinetask::min_level_stars(st).saturating_sub(1),
    };
    let skip_commented = options::bool("org-num-skip-commented", false);
    let skip_footnotes = options::bool("org-num-skip-footnotes", false);
    let skip_unnumbered = options::bool("org-num-skip-unnumbered", false);
    let tags = skip_tags();
    let section = super::footnote::section(st);
    let doc = Doc::new(lines, st, path);
    let mut skip_level: Option<usize> = None;
    let mut numbering: Vec<i64> = vec![];
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let stars = syntax::level(l)?;
            if stars > max_stars {
                return None;
            }
            let level = if odd { stars.div_ceil(2) } else { stars };
            let h = syntax::headline(l, st)?;
            let title = if h.commented {
                let c = options::string("org-comment-string", "COMMENT");
                let t = h.title(l);
                if t.is_empty() { c } else { format!("{c} {t}") }
            } else {
                h.title(l).to_owned()
            };
            let skip = (skip_footnotes && section.as_deref() == Some(title.as_str()))
                || (skip_commented && h.commented)
                || (!tags.is_empty() && h.tags.iter().any(|t| tags.contains(t)))
                || (skip_unnumbered
                    && doc
                        .get(Some(i), "UNNUMBERED", Inherit::Selective, false)
                        .is_some_and(|v| v != "nil"));
            // org-num--current-numbering.
            if skip_level.is_some_and(|s| level > s) {
                return None;
            }
            if skip {
                skip_level = Some(level);
                return None;
            }
            skip_level = None;
            let last = numbering.len();
            if numbering.is_empty() {
                numbering = vec![0; level];
                numbering[level - 1] = 1;
            } else if level == last {
                numbering[level - 1] += 1;
            } else if level < last {
                numbering.truncate(level);
                numbering[level - 1] += 1;
            } else {
                numbering.resize(level, 0);
                numbering[level - 1] = 1;
            }
            Some(default_format(&numbering))
        })
        .collect()
}

/// org-num-mode (ARG > 0 on, <= 0 off, none toggles).
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    if name != "org-num-mode" {
        return None;
    }
    if ed.org.is_none() {
        return Some(Err(
            "Cannot activate headline numbering outside Org mode".into()
        ));
    }
    let new = match arg {
        Prefix::None => !on(ed),
        Prefix::Num(n) => n > 0,
        Prefix::Minus => false,
        Prefix::U(_) => true,
    };
    if let Some(o) = &mut ed.org {
        o.num_mode = Some(new);
    }
    ed.set_msg(format!(
        "Org-Num mode {} in current buffer",
        if new { "enabled" } else { "disabled" }
    ));
    Some(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(text: &str) -> Vec<String> {
        let st = syntax::settings(text.lines(), None);
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        numbering(&lines, &st, None).into_iter().flatten().collect()
    }

    #[test]
    fn numbering_spec() {
        // Ported from test-org-num/update.
        assert_eq!(nums("* H1\n* H2"), ["1 ", "2 "]);
        assert_eq!(
            nums("* H\n** H1\n** H2\n* H3"),
            ["1 ", "1.1 ", "1.2 ", "2 "]
        );
        assert_eq!(nums("** H\n* H2"), ["0.1 ", "1 "]);
        assert_eq!(nums("* H\n*** H1"), ["1 ", "1.0.1 "]);
        assert_eq!(nums("#+STARTUP: odd\n* H\n*** H1"), ["1 ", "1.1 "]);
        options::put("org-num-max-level", toml::Value::Integer(1));
        assert_eq!(nums("* H\n** H1\n* H2"), ["1 ", "2 "]);
        options::remove("org-num-max-level");
        options::put("org-num-skip-commented", toml::Value::Boolean(true));
        assert_eq!(nums("* H\n* COMMENT H2\n** H3\n* H4"), ["1 ", "2 "]);
        options::remove("org-num-skip-commented");
        options::put("org-num-skip-footnotes", toml::Value::Boolean(true));
        assert_eq!(nums("* H\n* Footnotes\n* H2"), ["1 ", "2 "]);
        options::remove("org-num-skip-footnotes");
        options::put(
            "org-num-skip-tags",
            toml::Value::Array(vec![toml::Value::String("foo".into())]),
        );
        assert_eq!(nums("* H\n* H2 :foo:\n** H3\n* H4"), ["1 ", "2 "]);
        options::remove("org-num-skip-tags");
        options::put("org-num-skip-unnumbered", toml::Value::Boolean(true));
        assert_eq!(
            nums("* H\n* H2\n:PROPERTIES:\n:UNNUMBERED: t\n:END:\n** H3\n* H4"),
            ["1 ", "2 "]
        );
        assert_eq!(
            nums("* H\n* H2\n:PROPERTIES:\n:UNNUMBERED: nil\n:END:\n* H4"),
            ["1 ", "2 ", "3 "]
        );
        options::remove("org-num-skip-unnumbered");
        // Inline tasks are not numbered.
        assert_eq!(nums("* H\n*************** Task\n* H2"), ["1 ", "2 "]);
    }

    #[test]
    fn toggle() {
        let mut e = super::super::tests::org("* H", "");
        assert!(!on(&e));
        crate::org::run(&mut e, "org-num-mode", Prefix::None);
        assert!(on(&e));
        assert_eq!(
            e.msg.as_ref().unwrap().0,
            "Org-Num mode enabled in current buffer"
        );
        crate::org::run(&mut e, "org-num-mode", Prefix::Minus);
        assert!(!on(&e));
        let e = super::super::tests::org("#+STARTUP: num\n* H", "");
        assert!(on(&e));
    }
}
