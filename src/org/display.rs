//! What the renderer draws for an Org line (Emacs overlays and display
//! properties): org-indent prefixes and hidden leading stars, org-num
//! numbers, and, except on the cursor line, descriptive links and hidden
//! markers (face.rs), custom timestamps (org-display-custom-times) and
//! pretty entities (org-pretty-entities).
//!
//! Every change is an edit of the line text; [`Shown::map`] takes a
//! byte of the line to its displayed position, for the cursor, and
//! [`source_byte`] takes a displayed position back, for mouse clicks.
//! The cursor line only gets insertions and same-width replacements, so
//! its cursor is exact; the other transforms show the real text while
//! the cursor is on the line (as Fred's link concealment does).

use super::syntax::{self, Settings};
use crate::editor::Editor;
use crate::highlight::LineStyles;
use ratatui::style::Style;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// One display change: `range` of the line shown as `text`.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    /// The inserted text's style (None: unstyled, or the replaced text's).
    pub style: Option<Style>,
}

/// Per-buffer computed prefixes and numbers, by buffer version.
#[derive(Default)]
pub struct Cache(RefCell<Option<Cached>>);

/// (buffer version, numbers on, indentation on, info).
type Cached = (u64, bool, bool, Rc<Info>);

#[derive(Default)]
struct Info {
    num: Vec<Option<String>>,
    indent: Vec<String>,
}

fn info(ed: &Editor, num: bool, indent: bool) -> Rc<Info> {
    let v = ed.buf.version;
    if let Some(o) = &ed.org
        && let Some((ver, n, i, x)) = &*o.display.0.borrow()
        && *ver == v
        && *n == num
        && *i == indent
    {
        return Rc::clone(x);
    }
    let st = super::settings(ed);
    let lines: Vec<String> = (0..ed.line_count()).map(|l| ed.buf.line(l)).collect();
    let x = Rc::new(Info {
        num: if num {
            super::num::numbering(&lines, &st, ed.path.as_deref())
        } else {
            vec![]
        },
        indent: if indent {
            super::indent::prefixes(&lines, &st)
        } else {
            vec![]
        },
    });
    if let Some(o) = &ed.org {
        *o.display.0.borrow_mut() = Some((v, num, indent, Rc::clone(&x)));
    }
    x
}

/// The display edits of line `l` (raw text `line`).
pub fn edits(ed: &Editor, l: usize, line: &str, st: &Settings, cursor: bool) -> Vec<Edit> {
    let num = super::num::on(ed);
    let indent = super::indent::on(ed);
    let mut out: Vec<Edit> = vec![];
    if num || indent {
        let x = info(ed, num, indent);
        if let Some(p) = x.indent.get(l).filter(|p| !p.is_empty()) {
            out.push(Edit {
                range: 0..0,
                text: p.clone(),
                style: None,
            });
        }
        if indent
            && super::indent::hide_stars(ed)
            && let Some(n) = syntax::level(line)
            && n > 1
        {
            out.push(Edit {
                range: 0..n - 1,
                text: " ".repeat(n - 1),
                style: None,
            });
        }
        if let Some(Some(s)) = x.num.get(l)
            && let Some(n) = syntax::level(line)
        {
            let at = (n + 1).min(line.len());
            let style = super::face::level_style(st, n);
            out.push(Edit {
                range: at..at,
                text: s.clone(),
                style: Some(style),
            });
        }
    }
    if !cursor {
        for r in super::face::conceal_ranges(line, st) {
            out.push(Edit {
                range: r,
                text: String::new(),
                style: None,
            });
        }
        if super::time::display_custom_times(ed) {
            for (r, t) in super::time::stamp::custom_overlays(line) {
                out.push(Edit {
                    range: r,
                    text: t,
                    style: None,
                });
            }
        }
        for (r, t) in super::entities::overlays(ed, line) {
            out.push(Edit {
                range: r,
                text: t,
                style: None,
            });
        }
    }
    // Sorted by start, insertions first; overlapping later edits dropped.
    let mut sorted: Vec<Edit> = vec![];
    let mut all = out;
    all.sort_by_key(|e| (e.range.start, !e.range.is_empty()));
    for e in all {
        if sorted
            .iter()
            .any(|x| x.range.start < e.range.end && e.range.start < x.range.end)
        {
            continue;
        }
        sorted.push(e);
    }
    sorted
}

/// A displayed line.
pub struct Shown {
    pub text: String,
    pub styles: Option<LineStyles>,
    /// Displayed position of each byte of the line (and its end).
    pub map: Vec<usize>,
}

/// Apply EDITS to LINE (bytes past `raw_len`, like the fold ellipsis, are
/// kept as they are).
pub fn apply(line: &str, raw_len: usize, edits: &[Edit], styles: Option<LineStyles>) -> Shown {
    let n = line.len();
    let mut out = String::with_capacity(n + 16);
    let mut map = vec![0usize; n + 1];
    let mut inserted = vec![0usize; n + 1];
    let mut extra: LineStyles = vec![];
    let mut e = 0;
    let mut i = 0;
    while i <= n {
        // Insertions at i, then a replacement starting at i.
        let mut replaced: Option<usize> = None;
        while e < edits.len() && edits[e].range.start == i && i <= raw_len {
            let ed = &edits[e];
            e += 1;
            let at = out.len();
            out.push_str(&ed.text);
            if let Some(s) = ed.style {
                extra.push((s, at..out.len()));
            }
            if ed.range.is_empty() {
                inserted[i] += ed.text.len();
            } else {
                for j in ed.range.clone() {
                    if j <= n {
                        map[j] = at;
                    }
                }
                replaced = Some(ed.range.end);
                break;
            }
        }
        if let Some(end) = replaced {
            i = end;
            continue;
        }
        if i == n {
            map[n] = out.len();
            break;
        }
        map[i] = out.len();
        let c = line[i..].chars().next().unwrap();
        out.push(c);
        for k in 1..c.len_utf8() {
            map[i + k] = map[i];
        }
        i += c.len_utf8();
    }
    let styles = styles.map(|v| {
        let mut v: LineStyles = v
            .into_iter()
            .map(|(s, r)| {
                let a = map[r.start.min(n)];
                let z = map[r.end.min(n)] - inserted[r.end.min(n)];
                (s, a..z.max(a))
            })
            .filter(|(_, r)| r.start < r.end)
            .collect();
        v.extend(extra);
        v
    });
    Shown {
        text: out,
        styles,
        map,
    }
}

/// The line as drawn: `line` is the buffer line (maybe with the fold
/// ellipsis appended), `styles` its faces.
pub fn show(
    ed: &Editor,
    l: usize,
    line: String,
    styles: Option<LineStyles>,
    st: &Settings,
    cursor: bool,
) -> Shown {
    let raw = ed.buf.line(l);
    let raw_len = raw.len().min(line.len());
    let edits = edits(ed, l, &line[..raw_len], st, cursor);
    if edits.is_empty() {
        let map = (0..=line.len()).collect();
        return Shown {
            text: line,
            styles,
            map,
        };
    }
    apply(&line, raw_len, &edits, styles)
}

/// The displayed text of line `l` and its map (mouse clicks).
pub fn shown(ed: &Editor, l: usize) -> Shown {
    let st = super::settings(ed);
    show(ed, l, ed.buf.line(l), None, &st, l == ed.cur.line)
}

/// The byte of the line shown at displayed position `x`.
pub fn source_byte(map: &[usize], x: usize) -> usize {
    (0..map.len()).rev().find(|&i| map[i] <= x).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::Prefix;
    use crate::org::tests::org;

    #[test]
    fn apply_maps_bytes() {
        let e = [
            Edit {
                range: 0..0,
                text: "  ".into(),
                style: None,
            },
            Edit {
                range: 2..4,
                text: "β".into(),
                style: None,
            },
        ];
        let s = apply("a \\b c", 6, &e, Some(vec![(Style::default(), 2..4)]));
        assert_eq!(s.text, "  a β c");
        assert_eq!(s.map[0], 2);
        assert_eq!(s.map[4], 2 + 2 + "β".len());
        assert_eq!(s.styles.unwrap()[0].1, 4..6);
        assert_eq!(source_byte(&s.map, 0), 0);
        assert_eq!(source_byte(&s.map, 4), 3);
    }

    #[test]
    fn num_indent_entities_and_times() {
        let mut e = org("* H\n** H1\ntext \\alpha\n* H2", "");
        crate::org::run(&mut e, "org-num-mode", Prefix::None);
        let st = crate::org::settings(&e);
        assert_eq!(show(&e, 0, e.buf.line(0), None, &st, true).text, "* 1 H");
        assert_eq!(
            show(&e, 1, e.buf.line(1), None, &st, false).text,
            "** 1.1 H1"
        );
        crate::org::run(&mut e, "org-indent-mode", Prefix::None);
        let s = show(&e, 1, e.buf.line(1), None, &st, true);
        assert_eq!(s.text, "  * 1.1 H1");
        // The cursor at the title's first byte.
        assert_eq!(s.map[3], 8);
        assert_eq!(
            show(&e, 2, e.buf.line(2), None, &st, false).text,
            "    text \\alpha"
        );
        crate::org::run(&mut e, "org-toggle-pretty-entities", Prefix::None);
        assert_eq!(
            show(&e, 2, e.buf.line(2), None, &st, false).text,
            "    text α"
        );
        // The cursor line shows the source.
        assert_eq!(
            show(&e, 2, e.buf.line(2), None, &st, true).text,
            "    text \\alpha"
        );
        // Custom timestamps.
        let mut e = org("x <2026-10-04 Sun>", "");
        crate::org::run(&mut e, "org-toggle-time-stamp-overlays", Prefix::None);
        let st = crate::org::settings(&e);
        let t = show(&e, 0, e.buf.line(0), None, &st, false).text;
        assert_ne!(t, "x <2026-10-04 Sun>");
    }
}
