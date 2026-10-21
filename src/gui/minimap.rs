//! The minimap: each buffer line's shape as colored runs of display columns.
use super::proto;
use crate::editor::Editor;
use crate::highlight::{Highlighter, LineStyles};
use crate::text::{grapheme_width, is_plain};
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

/// `[col, len, color]` runs per line.
pub type Lines = Vec<Vec<(u16, u16, Option<u32>)>>;

/// Columns drawn per line; the rest is cut.
const COLS: usize = 96;
/// Bigger files get no minimap.
const MAX_LINES: usize = 50_000;
/// Lines highlighted per call, so the budget is checked between them.
const CHUNK: usize = 64;

/// Every line of `ed` as runs of non-blank display columns, colored by
/// `hl` as far as it gets within `budget`. `None` over 50 000 lines.
// ponytail: re-highlights the whole file each time (spans are not cached),
// so past `budget` lines stay uncolored; cache per-line runs if that shows.
pub fn build(ed: &Editor, hl: &mut Highlighter, budget: Duration) -> Option<Lines> {
    let n = ed.buf.len_lines();
    if n > MAX_LINES {
        return None;
    }
    // Generated buffers: their shape only.
    let plain =
        ed.magit.is_some() || ed.dired.is_some() || ed.term.is_some() || ed.org_view.is_some();
    let started = Instant::now();
    let ts = ed.tabstop();
    let mut out = Vec::with_capacity(n);
    let mut text = ed.buf.rope().lines();
    for start in (0..n).step_by(CHUNK) {
        let end = (start + CHUNK).min(n);
        let left = budget.saturating_sub(started.elapsed());
        let styles = if plain || left.is_zero() {
            vec![None; end - start]
        } else {
            hl.styles(&ed.buf, start..end, left)
        };
        for st in styles {
            let l: std::borrow::Cow<str> = text.next().map_or("".into(), Into::into);
            out.push(line(l.strip_suffix('\n').unwrap_or(&l), st.as_ref(), ts));
        }
    }
    Some(out)
}

fn line(text: &str, styles: Option<&LineStyles>, ts: usize) -> Vec<(u16, u16, Option<u32>)> {
    // Printable ASCII is a byte a column, without the grapheme walk.
    if is_plain(text) {
        return runs(
            text.bytes().enumerate().map(|(b, c)| (b, 1, c == b' ')),
            styles,
        );
    }
    let mut col = 0;
    let cells = text.grapheme_indices(true).map(|(b, g)| {
        let w = grapheme_width(g, col, ts);
        col += w;
        (b, w, g.chars().all(char::is_whitespace))
    });
    runs(cells, styles)
}

/// Runs of non-blank cells, given each cell's byte, width and blankness.
fn runs(
    cells: impl Iterator<Item = (usize, usize, bool)>,
    styles: Option<&LineStyles>,
) -> Vec<(u16, u16, Option<u32>)> {
    let mut runs: Vec<(u16, u16, Option<u32>)> = vec![];
    let (mut col, mut span, mut blank) = (0, 0, true);
    for (b, w, ws) in cells {
        if col >= COLS {
            break;
        }
        if ws {
            (col, blank) = (col + w, true);
            continue;
        }
        let color = styles.and_then(|st| {
            while st.get(span).is_some_and(|s| s.1.end <= b) {
                span += 1;
            }
            st.get(span)
                .filter(|s| s.1.start <= b)
                .and_then(|s| s.0.fg)
                .and_then(proto::color)
        });
        let len = w.min(COLS - col) as u16;
        match runs.last_mut() {
            Some(r) if !blank && r.2 == color => r.1 += len,
            _ => runs.push((col as u16, len, color)),
        }
        (col, blank) = (col + w, false);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use std::path::Path;

    const LONG: Duration = Duration::from_secs(5);

    fn ed(text: &str) -> Editor {
        let mut ed = Editor::new(Buffer::from_text(text));
        crate::options::set(&mut ed.opts, "ts=4", crate::options::Level::Both).unwrap();
        ed
    }

    fn plain(text: &str) -> Lines {
        let mut hl = Highlighter::new("ansi", false).unwrap();
        build(&ed(text), &mut hl, LONG).unwrap()
    }

    fn shape(lines: &Lines) -> Vec<Vec<(u16, u16)>> {
        lines
            .iter()
            .map(|l| l.iter().map(|&(c, n, _)| (c, n)).collect())
            .collect()
    }

    #[test]
    fn runs_skip_whitespace_and_expand_tabs() {
        // The tab at column 4 runs to the next stop: `c` is at 8.
        assert_eq!(
            shape(&plain("  ab\tc\n\nx")),
            [vec![(2, 2), (8, 1)], vec![], vec![(0, 1)]]
        );
    }

    #[test]
    fn long_lines_are_cut_at_96_columns() {
        let l = &plain(&"x".repeat(200))[0];
        assert_eq!(shape(&vec![l.clone()]), [vec![(0, 96)]]);
        // A word straddling the edge is cut at it; one past it is gone.
        let text = format!("{} abcd efg", "y".repeat(93));
        assert_eq!(shape(&plain(&text))[0], [(0, 93), (94, 2)]);
    }

    #[test]
    fn huge_files_have_no_minimap() {
        let mut hl = Highlighter::new("ansi", false).unwrap();
        let text = "x\n".repeat(50_001);
        assert!(build(&ed(&text), &mut hl, LONG).is_none());
        let text = "x\n".repeat(49_999);
        assert!(build(&ed(&text), &mut hl, LONG).is_some());
    }

    #[test]
    fn rust_keywords_carry_their_color() {
        let mut hl = Highlighter::new("material-dark", true).unwrap();
        let e = ed("fn main() {}");
        hl.set_file(Some(Path::new("x.rs")), &e.buf);
        let lines = build(&e, &mut hl, LONG).unwrap();
        assert_eq!(lines[0][0], (0, 2, Some(0x100_0000 | 0xC792EA)));
    }

    #[test]
    fn lines_past_the_budget_have_no_color() {
        let mut hl = Highlighter::new("material-dark", true).unwrap();
        let e = ed("fn a() {}\nfn b() {}");
        hl.set_file(Some(Path::new("x.rs")), &e.buf);
        let lines = build(&e, &mut hl, Duration::ZERO).unwrap();
        assert_eq!(lines[1][0], (0, 2, None));
    }
}
