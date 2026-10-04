//! Completion: buffer words, nearby files, paths, and the `:` line.

pub mod index;
pub mod nearby;
pub mod path;

use index::{WordIndex, is_word_char};
use std::collections::HashMap;
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

fn matches(word: &str, prefix: &str, case_sensitive: bool) -> bool {
    word != prefix
        && if case_sensitive {
            word.starts_with(prefix)
        } else {
            word.to_lowercase().starts_with(&prefix.to_lowercase())
        }
}

/// Completions for `prefix`: buffer words nearest the cursor first, then
/// words from nearby files. Smart case; at most `MAX_ITEMS`.
pub fn candidates(
    prefix: &str,
    idx: &WordIndex,
    cursor_line: usize,
    nearby: &[String],
) -> Vec<String> {
    let cs = prefix.chars().any(char::is_uppercase);
    let mut best: HashMap<&str, usize> = HashMap::new();
    for l in 0..idx.len_lines() {
        let dist = l.abs_diff(cursor_line);
        for w in idx.line_words(l) {
            if matches(w, prefix, cs) {
                let d = best.entry(w).or_insert(dist);
                *d = (*d).min(dist);
            }
        }
    }
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
        if matches(w, prefix, cs) && !out.contains(w) {
            out.push(w.clone());
        }
    }
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

const COMMANDS: &[&str] = &[
    "ai",
    "bdelete",
    "bnext",
    "bprevious",
    "buffer",
    "buffers",
    "cd",
    "d",
    "edit",
    "explain",
    "files",
    "g",
    "j",
    "ls",
    "m",
    "pwd",
    "quit",
    "read",
    "s",
    "t",
    "v",
    "write",
    "wq",
    "xit",
];

/// Full `:` lines that Tab can complete `text` to.
pub fn cmdline_candidates(text: &str, cwd: &Path) -> Vec<String> {
    if text.chars().all(|c| c.is_ascii_alphabetic()) {
        return COMMANDS
            .iter()
            .filter(|c| c.starts_with(text) && **c != text)
            .map(|c| c.to_string())
            .collect();
    }
    let Some((cmd, arg)) = text.split_once(' ') else {
        return vec![];
    };
    let name = cmd.trim_end_matches('!');
    if !matches!(
        name,
        "w" | "write" | "wq" | "e" | "edit" | "r" | "read" | "cd"
    ) {
        return vec![];
    }
    let arg = arg.trim_start();
    path::complete(arg, cwd)
        .into_iter()
        .map(|p| format!("{cmd} {p}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::editor::tests::ed;
    use index::WordIndex;

    #[test]
    fn ranking_and_smart_case() {
        let b = Buffer::from_text("alpha\nalphabet\nzzz\nAlpine\nalp");
        let mut i = WordIndex::default();
        i.ensure(&b, true);
        assert_eq!(
            candidates("alp", &i, 4, &[]),
            vec!["Alpine", "alphabet", "alpha"]
        );
        assert_eq!(candidates("Alp", &i, 4, &[]), vec!["Alpine"]);
        let nearby = vec!["alpaca".to_string(), "alpha".to_string()];
        assert_eq!(
            candidates("alp", &i, 4, &nearby),
            vec!["Alpine", "alphabet", "alpha", "alpaca"]
        );
    }

    #[test]
    fn caps_at_eight() {
        let text = (0..20)
            .map(|i| format!("word{i:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut i = WordIndex::default();
        i.ensure(&Buffer::from_text(&text), true);
        assert_eq!(candidates("wo", &i, 0, &[]).len(), 8);
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
        e.autocomplete = false;
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
        assert!(cmdline_candidates("s/x/y/", d.path()).is_empty());
    }
}
