//! Regex grep over the project's files on a background thread.

use super::fff::Loose;
use super::files::Files;
use crate::complete::nearby::read_text;
use regex::Regex;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const MAX_HITS: usize = 1000;
/// More from one file would crowd out the rest (fff's limit).
const MAX_PER_FILE: usize = 100;
/// File text kept in memory between searches.
const MAX_CACHED: usize = 512 * 1024 * 1024;
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
    /// Fuzzy: a near miss, not the pattern itself.
    pub loose: bool,
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
    /// What files held when last read, so that each keystroke searches
    /// memory, not the disk (fff keeps its files' contents too).
    cache: Mutex<Cache>,
}

#[derive(Debug, Default)]
struct Cache {
    files: HashMap<String, Cached>,
    /// Entries checked against the disk since `fresh` was last called.
    epoch: u64,
    bytes: usize,
}

#[derive(Debug)]
struct Cached {
    len: u64,
    mtime: Option<SystemTime>,
    epoch: u64,
    /// None for binary or oversized files.
    text: Option<Arc<str>>,
}

/// What to look for, in which files, and which of them to read first.
pub struct Search {
    pub re: Regex,
    /// Fuzzy: also lines with this pattern's letters close together, give
    /// or take a couple.
    pub fuzzy: Option<String>,
    pub keep: Box<dyn Fn(&str) -> bool + Send + Sync>,
    /// Files most likely wanted (used often, changed): their matches lead.
    pub first: Vec<String>,
}

impl Search {
    pub fn regex(re: Regex) -> Search {
        Search {
            re,
            fuzzy: None,
            keep: Box::new(|_| true),
            first: vec![],
        }
    }
}

impl Grep {
    /// Start grepping `files` for `re`; returns the new generation.
    pub fn start(self: &Arc<Self>, re: Regex, files: Arc<Files>) -> u64 {
        self.search(Search::regex(re), files)
    }

    pub fn search(self: &Arc<Self>, search: Search, files: Arc<Files>) -> u64 {
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.results.lock().unwrap() = Results {
            generation: g,
            ..Results::default()
        };
        let me = Arc::clone(self);
        std::thread::spawn(move || me.run(g, &search, &files));
        g
    }

    /// Check cached files against the disk again (a grep is opening).
    pub fn fresh(&self) {
        self.cache.lock().unwrap().epoch += 1;
    }

    /// The text of `rel`, from memory if it is unchanged.
    fn text(&self, root: &Path, rel: &str) -> Option<Arc<str>> {
        let path = root.join(rel);
        let epoch = {
            let c = self.cache.lock().unwrap();
            match c.files.get(rel) {
                Some(e) if e.epoch == c.epoch => return e.text.clone(),
                _ => c.epoch,
            }
        };
        let m = std::fs::metadata(&path).ok()?;
        {
            let mut c = self.cache.lock().unwrap();
            if let Some(e) = c.files.get_mut(rel)
                && m.len() == e.len
                && m.modified().ok() == e.mtime
            {
                e.epoch = epoch;
                return e.text.clone();
            }
        }
        let text: Option<Arc<str>> = read_text(&path, MAX_FILE).map(Arc::from);
        let size = text.as_ref().map_or(0, |t| t.len());
        let mut c = self.cache.lock().unwrap();
        if let Some(old) = c.files.remove(rel) {
            c.bytes -= old.text.map_or(0, |t| t.len());
        }
        // shortcut: past the budget, files are read each time; evict if
        // huge trees should stay fast too.
        if c.bytes + size <= MAX_CACHED {
            c.bytes += size;
            let e = Cached {
                len: m.len(),
                mtime: m.modified().ok(),
                epoch,
                text: text.clone(),
            };
            c.files.insert(rel.to_string(), e);
        }
        text
    }

    /// Wait up to `max` for search `g` to finish: most do, and showing the
    /// whole answer at once doesn't flicker.
    pub fn settle(&self, g: u64, max: Duration) {
        let start = Instant::now();
        while start.elapsed() < max {
            let r = self.results();
            if r.generation != g || r.done {
                return;
            }
            drop(r);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn current(&self, g: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == g
    }

    /// Search on every core; results still come in file order.
    fn run(&self, g: u64, search: &Search, files: &Files) {
        // The whole text at once: `^` and `$` at each line's ends.
        let re = Regex::new(&format!("(?mR){}", search.re.as_str()));
        let re = re.as_ref().unwrap_or(&search.re);
        // Only listed files, so only once the list is whole.
        let first: Vec<String> = if files.done() {
            files.with(|l| {
                let listed: HashSet<&str> = l.iter().map(String::as_str).collect();
                let first = search.first.iter().filter(|f| listed.contains(f.as_str()));
                first.cloned().collect()
            })
        } else {
            vec![]
        };
        let led: HashSet<&str> = first.iter().map(String::as_str).collect();
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        // Files searched, by position, until those before them are done:
        // (published up to, waiting).
        let waiting: Mutex<(usize, BTreeMap<usize, Vec<Hit>>)> = Mutex::default();
        let gone = || stop.load(Ordering::Relaxed) || !self.current(g);
        // The file list may still be growing: wait for position `i`.
        let nth = |i: usize| loop {
            if gone() {
                return None;
            }
            let item = match first.get(i) {
                Some(f) => Some(f.clone()),
                None => files.with(|l| l.get(i - first.len()).cloned()),
            };
            match item {
                Some(rel) => return Some(rel),
                None if files.done() => return None,
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        let work = || {
            let mut loose = search.fuzzy.as_deref().and_then(Loose::new);
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(rel) = nth(i) else { return };
                let skip = (i >= first.len() && led.contains(rel.as_str())) || !(search.keep)(&rel);
                let hits = match self.text(&files.root, &rel) {
                    Some(text) if !skip => scan(&text, re, loose.as_mut(), &rel, MAX_PER_FILE),
                    _ => vec![],
                };
                let mut w = waiting.lock().unwrap();
                w.1.insert(i, hits);
                let mut out = vec![];
                let (done, ready) = &mut *w;
                while let Some(h) = ready.remove(done) {
                    *done += 1;
                    out.extend(h);
                }
                if out.is_empty() {
                    continue;
                }
                let mut r = self.results.lock().unwrap();
                if r.generation != g {
                    return stop.store(true, Ordering::Relaxed);
                }
                let room = MAX_HITS - r.hits.len();
                if out.len() >= room {
                    out.truncate(room);
                    r.capped = true;
                    stop.store(true, Ordering::Relaxed);
                }
                r.hits.append(&mut out);
            }
        };
        std::thread::scope(|s| {
            for _ in 0..workers() {
                s.spawn(work);
            }
        });
        let mut r = self.results.lock().unwrap();
        if r.generation == g {
            r.done = true;
        }
    }

    /// Read every file into memory ahead of the first search.
    pub fn warm(self: &Arc<Self>, files: Arc<Files>) {
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            while !files.done() {
                std::thread::sleep(Duration::from_millis(10));
            }
            let next = AtomicUsize::new(0);
            std::thread::scope(|s| {
                for _ in 0..workers() {
                    s.spawn(|| {
                        while let Some(rel) =
                            files.with(|l| l.get(next.fetch_add(1, Ordering::Relaxed)).cloned())
                        {
                            me.text(&files.root, &rel);
                        }
                    });
                }
            });
        });
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

/// The lines of `text` that match, at most `cap`: the regex over the whole
/// text (as ripgrep: far faster than line by line), then for fuzzy grep the
/// near misses, line by line.
fn scan(text: &str, re: &Regex, loose: Option<&mut Loose>, rel: &str, cap: usize) -> Vec<Hit> {
    let hit = |line: usize, text: &str, (col, end): (usize, usize), loose: bool| Hit {
        path: rel.to_string(),
        line,
        col,
        end,
        text: text[..text.floor_char_boundary(MAX_TEXT)].to_string(),
        loose,
    };
    let mut hits = vec![];
    // Lines are counted only up to each match.
    let (mut at, mut line, mut counted) = (0, 0, 0);
    while hits.len() < cap && at <= text.len() {
        let Some(m) = re.find_at(text, at) else {
            break;
        };
        let start = text[..m.start()].rfind('\n').map_or(0, |i| i + 1);
        let end = text[m.start()..]
            .find('\n')
            .map_or(text.len(), |i| m.start() + i);
        line += text.as_bytes()[counted..start]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        counted = start;
        let l = text[start..end].trim_end_matches('\r');
        // One that runs into the next line (`\s`, `[^x]`) is tried within this one.
        let span = if m.end() <= start + l.len() {
            Some((m.start() - start, m.end() - start))
        } else {
            re.find(l).map(|m| (m.start(), m.end()))
        };
        if let Some(span) = span {
            hits.push(hit(line, l, span, false));
        }
        at = end + 1;
    }
    let Some(loose) = loose else {
        return hits;
    };
    let exact: HashSet<usize> = hits.iter().map(|h| h.line).collect();
    for (n, l) in text.lines().enumerate() {
        if hits.len() >= cap {
            break;
        }
        if !exact.contains(&n)
            && let Some(span) = loose.find(&l[..l.floor_char_boundary(MAX_TEXT)])
        {
            hits.push(hit(n, l, span, true));
        }
    }
    hits.sort_by_key(|h| h.line);
    hits
}

/// Threads to search with: the cores, up to eight.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get().min(8))
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
    fn filters_fuzzy_lines_and_likely_files_first() {
        let (_d, files) = project(&[
            (
                "a.rs",
                "struct UserAuthController;\nlet u = controller(user);\n",
            ),
            ("b.md", "the UserController\n"),
            ("z.rs", "UserController::new()\n"),
        ]);
        files.wait();
        let g = Arc::new(Grep::default());
        let search = Search {
            re: Regex::new("UserController").unwrap(),
            fuzzy: Some("UserController".into()),
            keep: Box::new(|f| f.ends_with(".rs")),
            first: vec!["z.rs".into(), "gone.rs".into()],
        };
        let r = g.wait(g.search(search, files));
        let got: Vec<_> = r.hits.iter().map(|h| (h.path.as_str(), h.line)).collect();
        // The scattered letters on a.rs's second line are no match.
        assert_eq!(got, [("z.rs", 0), ("a.rs", 0)]);
        assert_eq!((r.hits[1].col, r.hits[1].end), (7, 25));
    }

    #[test]
    fn scans_whole_texts_line_by_line_results() {
        let re = |p: &str| Regex::new(&format!("(?mR){p}")).unwrap();
        let text = "ab\r\nx foo\r\n\nfoo foo\nend";
        let got = |p: &str| -> Vec<_> {
            let hits = scan(text, &re(p), None, "f", 10);
            hits.into_iter()
                .map(|h| (h.line, h.col, h.end, h.text))
                .collect()
        };
        // One hit a line, the first match in it, lines counted from 0.
        assert_eq!(
            got("foo"),
            [(1, 2, 5, "x foo".into()), (3, 0, 3, "foo foo".into())]
        );
        assert_eq!(got("^foo$"), []);
        assert_eq!(
            got("foo$"),
            [(1, 2, 5, "x foo".into()), (3, 4, 7, "foo foo".into())]
        );
        assert_eq!(got("^end$"), [(4, 0, 3, "end".into())]);
        // Never across lines.
        assert_eq!(got(r"b\s+x"), []);
        assert_eq!(
            got(r"foo\s*"),
            [(1, 2, 5, "x foo".into()), (3, 0, 4, "foo foo".into())]
        );
        assert_eq!(scan(text, &re("o"), None, "f", 1).len(), 1);
    }

    #[test]
    fn caps_hits() {
        // At most 100 a file, 1000 in all.
        let text = "x\n".repeat(MAX_PER_FILE + 50);
        let names: Vec<String> = (0..12).map(|i| format!("f{i:02}")).collect();
        let list: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), text.as_str())).collect();
        let (_d, files) = project(&list);
        let g = Arc::new(Grep::default());
        let r = g.wait(g.start(Regex::new("x").unwrap(), files));
        assert_eq!(r.hits.len(), MAX_HITS);
        assert_eq!(
            r.hits.iter().filter(|h| h.path == "f00").count(),
            MAX_PER_FILE
        );
        assert!(r.capped);
    }

    #[test]
    fn remembers_texts_until_they_change() {
        let (d, files) = project(&[("a", "old\n")]);
        let g = Arc::new(Grep::default());
        assert_eq!(
            g.wait(g.start(Regex::new("old").unwrap(), Arc::clone(&files)))
                .hits
                .len(),
            1
        );
        // Within one picker the cache answers; a new one checks the disk.
        std::fs::write(d.path().join("a"), "new text\n").unwrap();
        assert_eq!(
            g.wait(g.start(Regex::new("old").unwrap(), Arc::clone(&files)))
                .hits
                .len(),
            1
        );
        g.fresh();
        assert_eq!(
            g.wait(g.start(Regex::new("old").unwrap(), Arc::clone(&files)))
                .hits
                .len(),
            0
        );
        assert_eq!(
            g.wait(g.start(Regex::new("new").unwrap(), files))
                .hits
                .len(),
            1
        );
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
