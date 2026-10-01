//! `Space k`: lines of the current buffer, swiper-style.

use super::Row;
use crate::buffer::Buffer;
use regex::Regex;
use std::path::PathBuf;

/// Longest part of a line shown, in bytes.
const MAX_TEXT: usize = 400;

pub struct Found {
    /// Last line first, so the list reads top to bottom on screen.
    pub rows: Vec<Row>,
    /// The first match at or after `from` (else the last match).
    pub sel: usize,
}

/// Lines matching every space-separated word of `query` (each a regex with
/// `/`'s smart case), in any order. An empty query lists every line.
pub fn search(query: &str, buf: &Buffer, from: usize) -> Result<Found, String> {
    let terms = query
        .split_whitespace()
        .map(crate::search::compile)
        .collect::<Result<Vec<Regex>, String>>()?;
    let n = buf.len_lines();
    let width = n.to_string().len();
    let mut rows = vec![];
    // ponytail: rescans the whole buffer per keystroke; fine to ~100k lines.
    for (l, line) in buf.rope().lines().enumerate().take(n) {
        let line = line.to_string();
        let line = line.trim_end_matches(['\n', '\r']);
        let mut hl = vec![];
        let mut col = None;
        let all = terms.iter().all(|t| match t.find(line) {
            Some(m) => {
                col = Some(col.map_or(m.start(), |c: usize| c.min(m.start())));
                hl.push((m.start(), m.end()));
                true
            }
            None => false,
        });
        if !all {
            continue;
        }
        let shown = &line[..line.floor_char_boundary(MAX_TEXT)];
        let head = format!("{:>width$}: ", l + 1);
        hl.sort_unstable();
        rows.push(Row {
            hl: hl
                .into_iter()
                .filter(|&(_, b)| b <= shown.len())
                .map(|(a, b)| (head.len() + a, head.len() + b))
                .collect(),
            text: format!("{head}{shown}"),
            path: PathBuf::new(),
            line: l,
            col: col.unwrap_or(0),
        });
    }
    let sel = rows
        .iter()
        .position(|r| r.line >= from)
        .unwrap_or(rows.len().saturating_sub(1));
    rows.reverse();
    let sel = rows.len().saturating_sub(1 + sel);
    Ok(Found { rows, sel })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(q: &str, text: &str, from: usize) -> (Vec<usize>, usize) {
        let f = search(q, &Buffer::from_text(text), from).unwrap();
        (f.rows.iter().map(|r| r.line).collect(), f.sel)
    }

    #[test]
    fn all_words_in_any_order() {
        let t = "fn draw()\nlet x = draw;\nfn main()\ndraw fn here";
        assert_eq!(lines("fn draw", t, 0).0, [3, 0]);
        assert_eq!(lines("draw", t, 0).0, [3, 1, 0]);
        assert_eq!(lines("", t, 0).0, [3, 2, 1, 0]);
        // Smart case per word.
        assert_eq!(lines("Draw", t, 0).0, Vec::<usize>::new());
    }

    #[test]
    fn starts_at_the_first_match_from_the_cursor() {
        let t = "a\nx\na\nx\na";
        // Matches are lines 4, 2, 0 (reversed); from line 1 the next is 2.
        assert_eq!(lines("a", t, 1), (vec![4, 2, 0], 1));
        assert_eq!(lines("a", t, 0), (vec![4, 2, 0], 2));
        // Past the last match: the last one.
        assert_eq!(lines("x", t, 4), (vec![3, 1], 0));
    }

    #[test]
    fn rows_show_numbers_and_highlight_matches() {
        let f = search("b.r foo", &Buffer::from_text("x\nfoo bar\n"), 0).unwrap();
        let r = &f.rows[0];
        assert_eq!(r.text, "2: foo bar");
        let hl: Vec<&str> = r.hl.iter().map(|&(a, b)| &r.text[a..b]).collect();
        assert_eq!(hl, ["foo", "bar"]);
        assert_eq!((r.line, r.col), (1, 0));
        assert!(search("(", &Buffer::from_text("x"), 0).is_err());
    }
}
