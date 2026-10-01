//! Fuzzy ranking of file paths (nucleo, Helix's matcher).

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::collections::HashMap;

pub struct Fuzzy {
    matcher: Matcher,
    pattern: Pattern,
    buf: Vec<char>,
}

impl Fuzzy {
    /// Smart case: a capital letter makes the query case-sensitive.
    pub fn new(query: &str) -> Fuzzy {
        Fuzzy {
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
            pattern: Pattern::parse(query, CaseMatching::Smart, Normalization::Smart),
            buf: vec![],
        }
    }

    pub fn score(&mut self, s: &str) -> Option<u32> {
        self.pattern
            .score(Utf32Str::new(s, &mut self.buf), &mut self.matcher)
    }

    /// Byte ranges of the matched characters in `s`.
    pub fn ranges(&mut self, s: &str) -> Vec<(usize, usize)> {
        let mut idx = vec![];
        self.pattern
            .indices(Utf32Str::new(s, &mut self.buf), &mut self.matcher, &mut idx);
        idx.sort_unstable();
        idx.dedup();
        let mut idx = idx.into_iter().peekable();
        s.char_indices()
            .enumerate()
            .filter(|(i, _)| idx.next_if_eq(&(*i as u32)).is_some())
            .map(|(_, (b, c))| (b, b + c.len_utf8()))
            .collect()
    }
}

/// Indices into `files`, best first, at most `limit`, and how many matched.
/// An empty query lists recent files (by `recent` rank) then the rest in
/// order; otherwise ties in score go to the more recent, then the shorter.
pub fn rank(
    query: &str,
    files: &[String],
    recent: &HashMap<String, usize>,
    limit: usize,
) -> (Vec<usize>, usize) {
    let rec = |i: usize| recent.get(&files[i]).copied().unwrap_or(usize::MAX);
    if query.trim().is_empty() {
        let mut out: Vec<usize> = (0..files.len()).filter(|&i| rec(i) != usize::MAX).collect();
        out.sort_by_key(|&i| rec(i));
        let n = out.len();
        out.extend(
            (0..files.len())
                .filter(|&i| rec(i) == usize::MAX)
                .take(limit.saturating_sub(n)),
        );
        out.truncate(limit);
        return (out, files.len());
    }
    let mut fz = Fuzzy::new(query);
    // ponytail: scores and sorts every match per keystroke; fine to ~200k files.
    let mut hits: Vec<(u32, usize)> = files
        .iter()
        .enumerate()
        .filter_map(|(i, f)| fz.score(f).map(|s| (s, i)))
        .collect();
    let n = hits.len();
    hits.sort_unstable_by_key(|&(s, i)| (std::cmp::Reverse(s), rec(i), files[i].len(), i));
    (hits.into_iter().take(limit).map(|(_, i)| i).collect(), n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fuzzy_finds_the_file() {
        let files = v(&[
            "src/search.rs",
            "src/session.rs",
            "src/swap.rs",
            "README.md",
        ]);
        let (top, n) = rank("sesrs", &files, &HashMap::new(), 10);
        assert_eq!(files[top[0]], "src/session.rs");
        assert!(n >= 1 && n < files.len());
        // Smart case: a capital requires a capital.
        assert_eq!(rank("readme", &files, &HashMap::new(), 10).1, 1);
        assert_eq!(rank("Readme", &files, &HashMap::new(), 10).1, 0);
    }

    #[test]
    fn recent_first_and_breaks_ties() {
        let files = v(&["a/x.rs", "b/x.rs", "c.rs"]);
        let recent = HashMap::from([("c.rs".to_string(), 0), ("b/x.rs".to_string(), 1)]);
        let (top, n) = rank("", &files, &recent, 10);
        assert_eq!((top, n), (vec![2, 1, 0], 3));
        assert_eq!(rank("", &files, &recent, 2).0, [2, 1]);
        assert_eq!(rank("x.rs", &files, &recent, 10).0[0], 1);
    }

    #[test]
    fn ranges_are_byte_ranges() {
        let mut fz = Fuzzy::new("ab");
        assert_eq!(fz.ranges("éaxb"), [(2, 3), (4, 5)]);
    }
}
