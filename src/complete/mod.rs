//! Completion: buffer words, nearby files, paths, and the `:` line.

pub mod index;
pub mod nearby;
pub mod path;

use index::{WordIndex, is_word_char};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const MAX_ITEMS: usize = 8;

/// The completion menu shown in Insert mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    pub items: Vec<String>,
    pub sel: Option<usize>,
    /// Byte where the completed token starts on the cursor line.
    pub start: usize,
    /// What the user typed (restored when cycling past the last item).
    pub typed: String,
}

/// `word` completes `prefix` (`lower`: the prefix lowercased, for no case).
fn matches(word: &str, prefix: &str, lower: Option<&str>) -> bool {
    word != prefix
        && match lower {
            None => word.starts_with(prefix),
            // Char by char: lowercasing every word in a big file per key
            // was most of a keystroke's time.
            Some(lower) => {
                let w = &word.as_bytes()[..lower.len().min(word.len())];
                if lower.is_ascii() && w.is_ascii() {
                    return w.len() == lower.len() && w.eq_ignore_ascii_case(lower.as_bytes());
                }
                let mut w = word.chars().flat_map(char::to_lowercase);
                lower.chars().all(|c| w.next() == Some(c))
            }
        }
}

/// Completions for `prefix`: buffer words nearest the cursor first, then
/// words from nearby files. Smart case; at most `MAX_ITEMS`.
pub fn candidates(
    prefix: &str,
    idx: &mut WordIndex,
    cursor_line: usize,
    nearby: &[String],
) -> Vec<String> {
    let lower = (!prefix.chars().any(char::is_uppercase)).then(|| prefix.to_lowercase());
    let lower = lower.as_deref();
    // Every indexed word that can match, when a shorter prefix's matches
    // are known: those it still matches, and words from lines edited
    // since. Finding them all ends the search, so typing a word that's
    // nowhere else doesn't read the whole file per key.
    let fresh = std::mem::take(&mut idx.fresh);
    let known: Option<HashSet<String>> = idx
        .memo
        .take()
        .filter(|(p, _)| prefix.starts_with(p.as_str()))
        .map(|(_, ws)| {
            ws.into_iter()
                .chain(fresh)
                .filter(|w| matches(w, prefix, lower))
                .collect()
        });
    let mut best: HashMap<&str, usize> = HashMap::new();
    // Outward from the cursor, a distance at a time: a word's first
    // sighting is its nearest, and once a whole distance leaves enough
    // words, farther ones can't rank. Not the whole file per key.
    let len = idx.len_lines();
    for dist in 0..=cursor_line.max(len) {
        if known.as_ref().is_some_and(|k| k.len() == best.len()) {
            break;
        }
        let below = cursor_line.checked_sub(dist).filter(|_| dist > 0);
        for l in below.into_iter().chain(Some(cursor_line + dist)) {
            for w in idx.line_words(l) {
                if matches(w, prefix, lower) {
                    best.entry(w).or_insert(dist);
                }
            }
        }
        if best.len() >= MAX_ITEMS {
            break;
        }
    }
    // Fewer than a menu's worth: the search was exhaustive.
    let memo = (best.len() < MAX_ITEMS).then(|| {
        (
            prefix.to_string(),
            best.keys().map(|w| w.to_string()).collect(),
        )
    });
    let mut ranked: Vec<(&str, usize)> = best.into_iter().collect();
    ranked.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));
    let mut out: Vec<String> = ranked
        .into_iter()
        .map(|(w, _)| w.to_string())
        .take(MAX_ITEMS)
        .collect();
    for w in nearby {
        if out.len() >= MAX_ITEMS {
            break;
        }
        if matches(w, prefix, lower) && !out.contains(w) {
            out.push(w.clone());
        }
    }
    idx.memo = memo;
    out
}

/// Characters that end a path token.
fn path_stop(c: char) -> bool {
    c.is_whitespace() || "'\"`()[]{}<>,;=:".contains(c)
}

fn after(s: &str, i: usize) -> usize {
    i + s[i..].chars().next().map_or(1, char::len_utf8)
}

/// The token ending at `byte`: (start byte, text, is_path).
pub fn token_before(line: &str, byte: usize) -> Option<(usize, String, bool)> {
    let before = &line[..byte];
    let pstart = before.rfind(path_stop).map_or(0, |i| after(before, i));
    let ptok = &before[pstart..];
    if ptok.contains('/') || ptok.starts_with('~') {
        return Some((pstart, ptok.to_string(), true));
    }
    let wstart = before
        .rfind(|c: char| !is_word_char(c))
        .map_or(0, |i| after(before, i));
    let w = &before[wstart..];
    (!w.is_empty()).then(|| (wstart, w.to_string(), false))
}

/// Full `:` lines that Tab can complete `text` to.
pub fn cmdline_candidates(text: &str, cwd: &Path) -> Vec<String> {
    if text.chars().all(|c| c.is_ascii_alphabetic()) {
        let mut names: Vec<String> = crate::ex::cmd::COMMANDS
            .iter()
            // Names typed as one symbol (`:&`, `:>`) aren't completed.
            .filter(|c| c.name.starts_with(text) && c.name != text)
            .filter(|c| c.name.starts_with(|ch: char| ch.is_ascii_alphabetic()))
            // One that needs an argument comes with the space before it.
            .map(|c| match c.flags & crate::ex::cmd::NEEDARG {
                0 => c.name.to_string(),
                _ => format!("{} ", c.name),
            })
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        return names;
    }
    let Some((cmd, arg)) = text.split_once(' ') else {
        return vec![];
    };
    use crate::ex::cmd::{FILE, OPTION, lookup};
    let flags = lookup(cmd.trim_end_matches('!')).map_or(0, |c| c.flags);
    if flags & OPTION != 0 {
        return option_names(cmd, arg);
    }
    if flags & FILE == 0 {
        return vec![];
    }
    let arg = arg.trim_start();
    path::complete(arg, cwd)
        .into_iter()
        .map(|p| format!("{cmd} {p}"))
        .collect()
}

/// `:set` lines completing the last word of `args` to an option name,
/// keeping a `no` or `inv` before it.
fn option_names(cmd: &str, args: &str) -> Vec<String> {
    let (before, word) = args.rsplit_once(' ').map_or(("", args), |(b, w)| (b, w));
    if word.contains(['=', ':', '!', '&', '?']) {
        return vec![];
    }
    let (prefix, name) = ["no", "inv", ""]
        .into_iter()
        .find_map(|p| Some((p, word.strip_prefix(p)?)))
        .unwrap_or(("", word));
    let head = match before {
        "" => format!("{cmd} "),
        b => format!("{cmd} {b} "),
    };
    crate::options::OPTIONS
        .iter()
        .filter(|d| d.name.starts_with(name) && d.name != name)
        .filter(|d| prefix.is_empty() || matches!(d.default, crate::options::DefaultValue::Bool(_)))
        .map(|d| format!("{head}{prefix}{}", d.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Edit};
    use crate::editor::tests::ed;
    use index::WordIndex;

    #[test]
    fn ranking_and_smart_case() {
        let b = Buffer::from_text("alpha\nalphabet\nzzz\nAlpine\nalp");
        let mut i = WordIndex::default();
        i.ensure(&b, true);
        assert_eq!(
            candidates("alp", &mut i, 4, &[]),
            vec!["Alpine", "alphabet", "alpha"]
        );
        assert_eq!(candidates("Alp", &mut i, 4, &[]), vec!["Alpine"]);
        let nearby = vec!["alpaca".to_string(), "alpha".to_string()];
        assert_eq!(
            candidates("alp", &mut i, 4, &nearby),
            vec!["Alpine", "alphabet", "alpha", "alpaca"]
        );
    }

    #[test]
    fn typing_on_reuses_the_last_search() {
        // The memo says "zq" matched nothing: "zqz" doesn't look again,
        // however far away (here, planted behind its back) a match is.
        let mut i = WordIndex::default();
        i.ensure(
            &Buffer::from_text(&format!("{}zqzq", "abc\n".repeat(50))),
            true,
        );
        i.memo = Some(("zq".into(), vec![]));
        assert_eq!(candidates("zqz", &mut i, 0, &[]), Vec::<String>::new());
        // Honestly: what a fresh search would find, as words are typed,
        // deleted and the cursor moves.
        let mut b =
            Buffer::from_text("alpha Alpine alphabet\nzeta zebra\n\nzealot alps\nALPACA zest\n");
        let mut i = WordIndex::default();
        i.ensure(&b, true);
        let mut seed = 3usize;
        for _ in 0..400 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let len = b.len_chars();
            let (start, end, text) = match (seed >> 40) % 8 {
                0 => ((seed >> 8) % (len + 1), (seed >> 8) % (len + 1), " "),
                1 if len > 0 => (len - 1, len, ""),
                2 => (len, len, "\n"),
                n => (len, len, ["a", "l", "Z", "e"][n % 4]),
            };
            b.apply(Edit {
                start,
                end,
                text: text.into(),
            });
            let last = b.len_lines() - 1;
            let line = b.line(last);
            let Some((_, prefix, _)) = token_before(&line, line.len()) else {
                continue;
            };
            i.ensure(&b, false);
            let got = candidates(&prefix, &mut i, last, &[]);
            let mut fresh = WordIndex::default();
            fresh.ensure(&b, true);
            assert_eq!(
                got,
                candidates(&prefix, &mut fresh, last, &[]),
                "{prefix:?} in {b:?}"
            );
        }
    }

    #[test]
    fn same_ranking_as_scanning_everything() {
        // The old whole-file scan: each match's nearest line, then nearest
        // first, ties alphabetical, then nearby files' words.
        fn naive(prefix: &str, b: &Buffer, cur: usize, nearby: &[String]) -> Vec<String> {
            let cs = prefix.chars().any(char::is_uppercase);
            let ok = |w: &str| {
                w != prefix
                    && if cs {
                        w.starts_with(prefix)
                    } else {
                        w.to_lowercase().starts_with(&prefix.to_lowercase())
                    }
            };
            let mut best: HashMap<String, usize> = HashMap::new();
            for l in 0..b.len_lines() {
                for w in index::words(&b.line(l)).filter(|w| ok(w)) {
                    let d = best.entry(w.to_string()).or_insert(usize::MAX);
                    *d = (*d).min(l.abs_diff(cur));
                }
            }
            let mut r: Vec<_> = best.into_iter().collect();
            r.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
            let mut out: Vec<String> = r.into_iter().map(|(w, _)| w).take(MAX_ITEMS).collect();
            for w in nearby {
                if out.len() < MAX_ITEMS && ok(w) && !out.contains(w) {
                    out.push(w.clone());
                }
            }
            out
        }
        let mut seed = 11usize;
        let mut next = |n: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let nearby: Vec<String> = ["abba", "Abed", "abcab", "bab"].map(String::from).into();
        for _ in 0..60 {
            // Few letters: many matches (well over a menu) and many ties.
            let text: String = (0..300)
                .map(|_| match next(9) {
                    0 => '\n',
                    1 => ' ',
                    n => "abcAB c"[n - 2..].chars().next().unwrap(),
                })
                .collect();
            let b = Buffer::from_text(&text);
            let mut i = WordIndex::default();
            i.ensure(&b, true);
            for _ in 0..20 {
                let prefix: String = (0..1 + next(3)).map(|_| ['a', 'b', 'A'][next(3)]).collect();
                let cur = next(b.len_lines() + 2);
                assert_eq!(
                    candidates(&prefix, &mut i, cur, &nearby),
                    naive(&prefix, &b, cur, &nearby),
                    "{prefix:?} at {cur} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn caps_at_eight() {
        let text = (0..20)
            .map(|i| format!("word{i:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut i = WordIndex::default();
        i.ensure(&Buffer::from_text(&text), true);
        assert_eq!(candidates("wo", &mut i, 0, &[]).len(), 8);
    }

    #[test]
    fn token_before_cursor() {
        assert_eq!(
            token_before("let foo_ba", 10),
            Some((4, "foo_ba".to_string(), false))
        );
        assert_eq!(
            token_before("open(\"./src/ma", 14),
            Some((6, "./src/ma".to_string(), true))
        );
        assert_eq!(
            token_before("x ~/Do", 6),
            Some((2, "~/Do".to_string(), true))
        );
        assert_eq!(token_before("x ", 2), None);
    }

    #[test]
    fn popup_flow() {
        let e = ed("hello\n", "Gohe<Tab><Esc>");
        assert_eq!(e.buf.text(), "hello\nhello");
        let e = ed("hello\n", "Gohe<Enter><Esc>");
        assert_eq!(e.buf.text(), "hello\nhe\n");
        let e = ed("hello help\n", "Goh<C-n><C-n><Esc>");
        assert_eq!(e.buf.text(), "hello help\nhelp");
        let e = ed("hello help\n", "Gohel<Tab><Tab><Tab><Esc>");
        assert_eq!(
            e.buf.text(),
            "hello help\nhel",
            "cycling past the end restores what was typed"
        );
        let e = ed("hello\n", "Gohe<Tab><Enter>x<Esc>");
        assert_eq!(
            e.buf.text(),
            "hello\nhellox",
            "Enter accepts a selection without a newline"
        );
        let e = ed("hello\n", "Gohe");
        assert!(e.popup.is_some());
        let e = ed("hello\n", "Gohe<Esc>");
        assert!(e.popup.is_none());
        let e = ed("hello\n", "Gohe ");
        assert!(e.popup.is_none());
    }

    #[test]
    fn autocomplete_off_still_allows_ctrl_n() {
        let mut e = crate::editor::Editor::new(Buffer::from_text("hello"));
        e.run_ex("set noautocomplete");
        for k in crate::key::parse_keys("ohe") {
            e.handle_key(k);
        }
        assert!(e.popup.is_none());
        e.handle_key(crate::key::Key::ctrl('n'));
        assert_eq!(e.buf.line(1), "hello");
    }

    #[test]
    fn cmdline_completion() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("notes.txt"), "").unwrap();
        std::fs::create_dir(d.path().join("notebook")).unwrap();
        let c = cmdline_candidates("w no", d.path());
        assert_eq!(c, vec!["w notebook/", "w notes.txt"]);
        assert_eq!(
            cmdline_candidates("e! notes", d.path()),
            vec!["e! notes.txt"]
        );
        assert_eq!(cmdline_candidates("wr", d.path()), vec!["write"]);
        assert_eq!(cmdline_candidates("set ta", d.path()), vec!["set tabstop"]);
        assert_eq!(
            cmdline_candidates("se ts=4 nowr", d.path()),
            vec!["se ts=4 nowrap"]
        );
        assert!(cmdline_candidates("set note", d.path()).is_empty());
        assert!(cmdline_candidates("set ts=", d.path()).is_empty());
        assert!(cmdline_candidates("s/x/y/", d.path()).is_empty());
    }
}
