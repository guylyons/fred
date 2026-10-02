//! `Space k`: lines of the current buffer, swiper-style; `Space B`: of
//! every buffer.

use super::Row;
use regex::Regex;
use ropey::Rope;
use std::path::{Path, PathBuf};

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
pub fn search(query: &str, text: &Rope, from: usize) -> Result<Found, String> {
    Ok(search_in(&terms(query)?, text, from, None, Path::new("")))
}

/// [`search`] in every buffer of `bufs` (name, path, text), the first one
/// (the cursor's, `from` applying to it) nearest the prompt; rows are
/// `name:12: line`.
pub fn search_all(
    query: &str,
    bufs: &[(String, PathBuf, Rope)],
    from: usize,
) -> Result<Found, String> {
    let terms = terms(query)?;
    let mut all = Found {
        rows: vec![],
        sel: 0,
    };
    for (i, (name, path, text)) in bufs.iter().enumerate() {
        let f = search_in(&terms, text, from, Some(name), path);
        if i == 0 {
            all.sel = f.sel;
        }
        all.rows.extend(f.rows);
    }
    Ok(all)
}

fn terms(query: &str) -> Result<Vec<Regex>, String> {
    query
        .split_whitespace()
        .map(crate::search::compile)
        .collect()
}

fn search_in(terms: &[Regex], text: &Rope, from: usize, name: Option<&str>, path: &Path) -> Found {
    let n = text.len_lines();
    let width = n.to_string().len();
    let mut rows = vec![];
    // ponytail: rescans the whole buffer per keystroke; fine to ~100k lines.
    for (l, line) in text.lines().enumerate().take(n) {
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
        let head = match name {
            Some(name) => format!("{name}:{}: ", l + 1),
            None => format!("{:>width$}: ", l + 1),
        };
        hl.sort_unstable();
        rows.push(Row {
            hl: hl
                .into_iter()
                .filter(|&(_, b)| b <= shown.len())
                .map(|(a, b)| (head.len() + a, head.len() + b))
                .collect(),
            text: format!("{head}{shown}"),
            path: path.to_path_buf(),
            line: l,
            col: col.unwrap_or(0),
            code: Some(head.len()),
        });
    }
    let sel = rows
        .iter()
        .position(|r| r.line >= from)
        .unwrap_or(rows.len().saturating_sub(1));
    rows.reverse();
    let sel = rows.len().saturating_sub(1 + sel);
    Found { rows, sel }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(q: &str, text: &str, from: usize) -> (Vec<usize>, usize) {
        let f = search(q, &Rope::from_str(text), from).unwrap();
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
        let f = search("b.r foo", &Rope::from_str("x\nfoo bar\n"), 0).unwrap();
        let r = &f.rows[0];
        assert_eq!(r.text, "2: foo bar");
        let hl: Vec<&str> = r.hl.iter().map(|&(a, b)| &r.text[a..b]).collect();
        assert_eq!(hl, ["foo", "bar"]);
        assert_eq!((r.line, r.col), (1, 0));
        assert!(search("(", &Rope::from_str("x"), 0).is_err());
    }

    #[test]
    fn every_buffer_this_one_nearest_the_prompt() {
        let buf =
            |name: &str, text: &str| (name.to_string(), PathBuf::from(name), Rope::from_str(text));
        let bufs = [buf("a.rs", "x\nfoo 1\nfoo 2"), buf("b.rs", "foo 3\ny")];
        let f = search_all("foo", &bufs, 2).unwrap();
        let texts: Vec<&str> = f.rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["a.rs:3: foo 2", "a.rs:2: foo 1", "b.rs:1: foo 3"]);
        assert_eq!(f.sel, 0);
        assert_eq!(f.rows[2].path, PathBuf::from("b.rs"));
        assert!(search_all("(", &bufs, 0).is_err());
    }
}
