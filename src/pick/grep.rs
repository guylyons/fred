//! Regex grep over the project's files on a background thread.

use super::files::Files;
use crate::complete::nearby::read_text;
use regex::Regex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const MAX_HITS: usize = 1000;
const MAX_FILE: u64 = 1024 * 1024;
/// Longest line kept for display, in bytes.
const MAX_TEXT: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Relative to the project root.
    pub path: String,
    /// 0-based line, and the match's byte range in the full line.
    pub line: usize,
    pub col: usize,
    pub end: usize,
    /// The line, cut to `MAX_TEXT` bytes.
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Results {
    pub generation: u64,
    pub hits: Vec<Hit>,
    pub done: bool,
    pub capped: bool,
}

/// One grep at a time: starting another (or cancelling) stops the last.
#[derive(Debug, Default)]
pub struct Grep {
    generation: AtomicU64,
    results: Mutex<Results>,
}

impl Grep {
    /// Start grepping `files` for `re`; returns the new generation.
    pub fn start(self: &Arc<Self>, re: Regex, files: Arc<Files>) -> u64 {
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.results.lock().unwrap() = Results {
            generation: g,
            ..Results::default()
        };
        let me = Arc::clone(self);
        std::thread::spawn(move || me.run(g, &re, &files));
        g
    }

    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn current(&self, g: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == g
    }

    fn run(&self, g: u64, re: &Regex, files: &Files) {
        let mut i = 0;
        let mut found = 0;
        loop {
            if !self.current(g) {
                return;
            }
            // The file list may still be growing.
            let next = files.with(|l| l.get(i).cloned());
            let Some(rel) = next else {
                if files.done() && i >= files.len() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            i += 1;
            let Some(text) = read_text(&files.root.join(&rel), MAX_FILE) else {
                continue;
            };
            let mut hits = vec![];
            for (n, line) in text.lines().enumerate() {
                if let Some(m) = re.find(line) {
                    hits.push(Hit {
                        path: rel.clone(),
                        line: n,
                        col: m.start(),
                        end: m.end(),
                        text: line[..line.floor_char_boundary(MAX_TEXT)].to_string(),
                    });
                    if found + hits.len() == MAX_HITS {
                        break;
                    }
                }
            }
            found += hits.len();
            let mut r = self.results.lock().unwrap();
            if r.generation != g {
                return;
            }
            r.hits.append(&mut hits);
            if found == MAX_HITS {
                r.capped = true;
                break;
            }
        }
        let mut r = self.results.lock().unwrap();
        if r.generation == g {
            r.done = true;
        }
    }

    pub fn results(&self) -> std::sync::MutexGuard<'_, Results> {
        self.results.lock().unwrap()
    }

    #[cfg(test)]
    pub fn wait(&self, g: u64) -> Results {
        loop {
            {
                let r = self.results();
                if r.generation != g || r.done {
                    return r.clone();
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Files>) {
        let dir = tempfile::tempdir().unwrap();
        for (p, t) in files {
            let p = dir.path().join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, t).unwrap();
        }
        let f = Files::spawn(dir.path().to_path_buf(), false);
        (dir, f)
    }

    #[test]
    fn finds_lines_and_skips_binary() {
        let (_d, files) = project(&[
            ("a.rs", "fn main() {}\n    let x = 1;\nfn other() {}\n"),
            ("bin", "fn \0 binary"),
            ("b/c.txt", "nothing here\n"),
        ]);
        let g = Arc::new(Grep::default());
        let r = g.wait(g.start(Regex::new(r"fn \w+").unwrap(), files));
        let got: Vec<_> = r
            .hits
            .iter()
            .map(|h| (h.path.as_str(), h.line, h.col, h.end))
            .collect();
        assert_eq!(got, [("a.rs", 0, 0, 7), ("a.rs", 2, 0, 8)]);
        assert!(r.done && !r.capped);
    }

    #[test]
    fn caps_hits() {
        let text = "x\n".repeat(MAX_HITS + 50);
        let (_d, files) = project(&[("a", &text), ("b", &text)]);
        let g = Arc::new(Grep::default());
        let r = g.wait(g.start(Regex::new("x").unwrap(), files));
        assert_eq!(r.hits.len(), MAX_HITS);
        assert!(r.capped);
    }

    #[test]
    fn a_new_grep_replaces_the_old() {
        let (_d, files) = project(&[("a", "one\ntwo\n")]);
        let g = Arc::new(Grep::default());
        let old = g.start(Regex::new("one").unwrap(), Arc::clone(&files));
        let new = g.start(Regex::new("two").unwrap(), files);
        assert!(new > old);
        let r = g.wait(new);
        assert_eq!(r.generation, new);
        assert_eq!(r.hits.len(), 1);
        assert_eq!(r.hits[0].text, "two");
    }
}
