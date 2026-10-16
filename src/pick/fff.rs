//! File ranking as in fff (https://github.com/dmtrKovalenko/fff): frecency,
//! the file a query led to before, git state, nearness to the current
//! file, typo tolerance and query filters. fff's formulas and constants,
//! on fred's own matcher and plain state files (no index, no database).

use super::fuzzy::Fuzzy;
use crate::git::FileState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DAY: u64 = 86400;
/// ln 2 / 10: an access counts half as much after ten days.
const DECAY: f64 = 0.0693;
const HISTORY_DAYS: u64 = 30;
const MAX_STAMPS: usize = 128;
/// Points for a changed file, by how long ago it was written.
const MODIFIED: [(i64, u64); 5] = [
    (16, 60 * 2),
    (8, 60 * 15),
    (4, 60 * 60),
    (2, DAY),
    (1, DAY * 7),
];
/// A query's file is boosted from its third pick on.
const MIN_COMBO: u32 = 3;
const COMBO_BOOST: i32 = 100;
const MAX_QUERIES: usize = 1000;
/// Typos are tried when fewer files than this match as typed.
const TYPO_BELOW: usize = 20;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Write and rename, so two freds never leave a half-written file.
fn save(file: &Path, text: String) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// `stamp stamp …<TAB>path` per line: when each file was opened.
fn stamps(file: &Path) -> Vec<(PathBuf, Vec<u64>)> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return vec![];
    };
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(s, p)| {
            let s = s.split(' ').filter_map(|t| t.parse().ok()).collect();
            (PathBuf::from(p), s)
        })
        .collect()
}

/// `path` was opened at `now`. Errors are ignored, as for recent files.
pub fn track(file: &Path, path: &Path, now: u64) {
    let Ok(path) = std::path::absolute(path) else {
        return;
    };
    let cutoff = now.saturating_sub(HISTORY_DAYS * DAY);
    let mut list = stamps(file);
    if !list.iter().any(|e| e.0 == path) {
        list.push((path.clone(), vec![]));
    }
    for (p, s) in &mut list {
        s.retain(|&t| t >= cutoff);
        if *p == path {
            let over = (s.len() + 1).saturating_sub(MAX_STAMPS);
            s.drain(..over);
            s.push(now);
        }
    }
    let text = list
        .iter()
        .filter(|(_, s)| !s.is_empty())
        .filter_map(|(p, s)| Some((p.to_str().filter(|p| !p.contains('\n'))?, s)))
        .map(|(p, s)| {
            let s: Vec<String> = s.iter().map(u64::to_string).collect();
            format!("{}\t{p}\n", s.join(" "))
        })
        .collect();
    save(file, text);
}

/// Each access decays by its age; past ten the sum grows slowly.
fn access_score(stamps: &[u64], now: u64) -> i64 {
    let cutoff = now.saturating_sub(HISTORY_DAYS * DAY);
    let total: f64 = stamps
        .iter()
        .filter(|&&t| t >= cutoff)
        .map(|&t| (-DECAY * now.saturating_sub(t) as f64 / DAY as f64).exp())
        .sum();
    let total = if total <= 10.0 {
        total
    } else {
        10.0 + (total - 10.0).sqrt()
    };
    total.round() as i64
}

/// Access scores of the files under `root`, by relative path.
pub fn access_scores(file: &Path, root: &Path, now: u64) -> HashMap<String, i64> {
    stamps(file)
        .iter()
        .filter_map(|(p, s)| {
            let rel = p.strip_prefix(root).ok()?.to_str()?.to_string();
            Some((rel, access_score(s, now))).filter(|e| e.1 > 0)
        })
        .collect()
}

/// Points for a changed file written `age` seconds ago, interpolated
/// between the thresholds.
pub fn modification_score(age: u64) -> i64 {
    for (i, &(points, within)) in MODIFIED.iter().enumerate() {
        if age > within {
            continue;
        }
        if i == 0 || age == within {
            return points;
        }
        let (prev_points, prev_within) = MODIFIED[i - 1];
        let span = (within - prev_within) as i64;
        return prev_points - (prev_points - points) * (age - prev_within) as i64 / span;
    }
    0
}

/// `count<TAB>root<TAB>path<TAB>query` per line, newest first: the file
/// each query last led to in a project, and how many times in a row.
fn queries(file: &Path) -> Vec<(u32, String, String, String)> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return vec![];
    };
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, '\t');
            let n = f.next()?.parse().ok()?;
            Some((n, f.next()?.into(), f.next()?.into(), f.next()?.into()))
        })
        .collect()
}

/// `query` in the project at `root` was answered with `path`.
pub fn track_query(file: &Path, root: &Path, query: &str, path: &Path) {
    let query = query.trim();
    let (Some(root), Some(path)) = (root.to_str(), path.to_str()) else {
        return;
    };
    if query.is_empty() || [root, path, query].iter().any(|s| s.contains(['\t', '\n'])) {
        return;
    }
    let mut list = queries(file);
    let old = list
        .iter()
        .position(|e| e.1 == root && e.3 == query)
        .map(|i| list.remove(i));
    // The same file again counts up; another one starts over.
    let n = old.filter(|o| o.2 == path).map_or(1, |o| o.0 + 1);
    list.insert(0, (n, root.into(), path.into(), query.into()));
    list.truncate(MAX_QUERIES);
    let text = list
        .iter()
        .map(|(n, r, p, q)| format!("{n}\t{r}\t{p}\t{q}\n"))
        .collect();
    save(file, text);
}

/// The file `query` last led to under `root` (relative), and its count.
pub fn last_query(file: &Path, root: &Path, query: &str) -> Option<(String, u32)> {
    let (root_s, query) = (root.to_str()?, query.trim());
    let e = queries(file)
        .into_iter()
        .find(|e| e.1 == root_s && e.3 == query)?;
    let rel = Path::new(&e.2)
        .strip_prefix(root)
        .ok()?
        .to_str()?
        .to_string();
    Some((rel, e.0))
}

enum Filter {
    /// `*.rs`
    Ext(String),
    /// `src/`: a directory on the way to the file.
    Dir(String),
    /// `status:modified`, `status:untracked`, `status:unmodified`.
    Status(Option<FileState>),
    /// `**/*_test.go`
    Glob(ignore::overrides::Override),
    /// `type:rust`
    Type(ignore::types::Types),
}

impl Filter {
    /// `grep`: a pattern may hold `*`, `?`, `[` and `{`, so no bare globs.
    fn parse(t: &str, grep: bool) -> Option<Filter> {
        if let Some(ext) = t.strip_prefix("*.")
            && !ext.is_empty()
            && !ext.contains(['*', '?', '[', '{', '/'])
        {
            return Some(Filter::Ext(ext.to_lowercase()));
        }
        if grep && !t.starts_with("**/") {
            // Only `**/…` globs: anything else is the pattern's.
        } else if t.contains(['*', '?', '[', '{']) {
            let mut b = ignore::overrides::OverrideBuilder::new("/");
            b.add(t).ok()?;
            return b.build().ok().map(Filter::Glob);
        }
        if let Some(v) = t.strip_prefix("status:").or_else(|| t.strip_prefix("git:")) {
            return match v {
                "" => None,
                v if "modified".starts_with(v) || "staged".starts_with(v) => {
                    Some(Filter::Status(Some(FileState::Modified)))
                }
                v if "untracked".starts_with(v) => Some(Filter::Status(Some(FileState::New))),
                v if "unmodified".starts_with(v) => Some(Filter::Status(None)),
                _ => None,
            };
        }
        if let Some(name) = t.strip_prefix("type:") {
            let mut b = ignore::types::TypesBuilder::new();
            b.add_defaults();
            b.select(name);
            return b.build().ok().map(Filter::Type);
        }
        let dir = t.strip_suffix('/')?.trim_matches('/');
        let plain = dir
            .chars()
            .all(|c| c.is_alphanumeric() || "/._-".contains(c));
        (!dir.is_empty() && (plain || !grep)).then(|| Filter::Dir(dir.to_lowercase()))
    }

    fn keeps(&self, f: &str, status: &HashMap<String, FileState>) -> bool {
        match self {
            Filter::Ext(e) => f
                .rsplit_once('.')
                .is_some_and(|(_, x)| x.eq_ignore_ascii_case(e)),
            Filter::Dir(d) => format!("/{}", f.to_lowercase()).contains(&format!("/{d}/")),
            Filter::Status(s) => status.get(f).copied() == *s,
            Filter::Glob(g) => g.matched(Path::new("/").join(f), false).is_whitelist(),
            Filter::Type(t) => t.matched(f, false).is_whitelist(),
        }
    }
}

/// A typed query: what to match fuzzily, what to keep, where to land.
#[derive(Default)]
pub struct Query {
    pub text: String,
    /// Each with "not" (`!*.rs`).
    filters: Vec<(bool, Filter)>,
    /// `name:12` or `name:12:3`: the line and column (from 1) to open at.
    pub at: Option<(usize, usize)>,
}

impl Query {
    pub fn parse(q: &str) -> Query {
        let (mut out, mut words) = Query::split(q, false);
        if let Some(last) = words.last_mut()
            && let Some((name, at)) = location(last)
        {
            *last = name;
            out.at = Some(at);
        }
        out.text = words.join(" ");
        out
    }

    /// A grep query: the same filters (`*.rs`, `src/`, `type:rust`,
    /// `status:modified`, `**/…`), the rest being the pattern as typed.
    pub fn parse_grep(q: &str) -> Query {
        let (mut out, words) = Query::split(q, true);
        if out.filters.is_empty() {
            out.text = q.to_string();
        } else {
            out.text = words.join(" ");
        }
        out
    }

    fn split(q: &str, grep: bool) -> (Query, Vec<&str>) {
        let mut out = Query::default();
        let mut words = vec![];
        for t in q.split_whitespace() {
            let (not, body) = t.strip_prefix('!').map_or((false, t), |b| (true, b));
            match Filter::parse(body, grep) {
                Some(f) => out.filters.push((not, f)),
                None => words.push(t),
            }
        }
        (out, words)
    }

    /// Whether the file at `f` (relative) passes every filter.
    pub fn keeps(&self, f: &str, status: &HashMap<String, FileState>) -> bool {
        self.filters
            .iter()
            .all(|(not, x)| x.keeps(f, status) != *not)
    }
}

/// `name:12` → (`name`, (12, 1)); `name:12:3` → (`name`, (12, 3)).
fn location(t: &str) -> Option<(&str, (usize, usize))> {
    let num = |s: &str| s.parse::<usize>().ok().filter(|&n| n > 0);
    let (head, last) = t.rsplit_once(':')?;
    let last = num(last)?;
    if let Some((name, line)) = head.rsplit_once(':')
        && let Some(line) = num(line)
        && !name.is_empty()
    {
        return Some((name, (line, last)));
    }
    (!head.is_empty()).then_some((head, (last, 1)))
}

/// What is known about the project's files, by relative path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// Decayed count of recent opens.
    pub access: HashMap<String, i64>,
    pub status: HashMap<String, FileState>,
    /// Points for changed files written lately.
    pub modified: HashMap<String, i64>,
    /// How many of the last few commits touched the file.
    pub commits: HashMap<String, i16>,
    /// Rank among recently opened files (0 = newest): breaks ties.
    pub recent: HashMap<String, usize>,
    /// The file being edited: its neighbours rank higher, itself lower.
    pub current: Option<String>,
}

impl Meta {
    /// Take in `git status`: changed files score by when they were written.
    pub fn set_status(&mut self, root: &Path, status: HashMap<String, FileState>, now: u64) {
        // shortcut: stats at most 2000 changed files, raise if a huge
        // untracked tree should rank by age too.
        self.modified = status
            .keys()
            .take(2000)
            .filter_map(|f| {
                let t = std::fs::metadata(root.join(f)).ok()?.modified().ok()?;
                let t = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
                Some((f.clone(), modification_score(now.saturating_sub(t)))).filter(|e| e.1 > 0)
            })
            .collect();
        self.status = status;
    }

    /// Files worth reading first, best first: used often, or changed.
    pub fn likely(&self) -> Vec<String> {
        let mut all: Vec<&String> = self.access.keys().chain(self.status.keys()).collect();
        all.sort_by_key(|f| (std::cmp::Reverse(self.frecency(f)), f.as_str()));
        all.dedup();
        all.into_iter().cloned().collect()
    }

    fn frecency(&self, f: &str) -> i32 {
        let get = |m: &HashMap<String, i64>| m.get(f).copied().unwrap_or(0) as i32;
        get(&self.access) + get(&self.modified) * 4
    }

    /// Minus the directories between the current file's and `f`'s (≤ 20).
    fn distance(&self, f: &str) -> i32 {
        let Some(cur) = &self.current else { return 0 };
        let dir = |p: &str| p.rsplit_once('/').map_or("", |d| d.0).to_string();
        let (a, b) = (dir(cur), dir(f));
        let a: Vec<&str> = a.split('/').filter(|c| !c.is_empty()).collect();
        let common = a
            .iter()
            .zip(b.split('/'))
            .take_while(|(x, y)| *x == y)
            .count();
        -((a.len() - common).min(20) as i32)
    }
}

/// Files that open a module or program: a small boost when only their
/// directory matched.
fn entry_point(name: &str) -> bool {
    matches!(
        name,
        "mod.rs"
            | "lib.rs"
            | "main.rs"
            | "index.js"
            | "index.jsx"
            | "index.ts"
            | "index.tsx"
            | "index.mjs"
            | "index.cjs"
            | "index.vue"
            | "__init__.py"
            | "__main__.py"
            | "main.go"
            | "main.c"
            | "index.php"
            | "main.rb"
            | "index.rb"
    )
}

/// One result: the file, and what of the query it matched if not all.
pub struct Hit {
    pub idx: usize,
    pub typo: Option<String>,
}

/// Indices into `files`, best first, at most `limit`, and how many matched.
/// With nothing to match, the files by frecency.
pub fn rank(
    q: &Query,
    files: &[String],
    meta: &Meta,
    combo: Option<&(String, u32)>,
    limit: usize,
) -> (Vec<Hit>, usize) {
    let rec = |i: usize| meta.recent.get(&files[i]).copied().unwrap_or(usize::MAX);
    let changed = |f: &str| meta.status.contains_key(f);
    let commits = |f: &str| meta.commits.get(f).copied().unwrap_or(0) as i32;
    let kept = (0..files.len()).filter(|&i| q.keeps(&files[i], &meta.status));
    if q.text.is_empty() {
        let mut all: Vec<(i32, usize)> = kept
            .map(|i| {
                let f = &files[i];
                let mut fr = meta.frecency(f);
                let git = if changed(f) { fr * 15 / 100 } else { 0 };
                if meta.current.as_deref() == Some(f) {
                    fr = 0;
                }
                (fr + git + commits(f), i)
            })
            .collect();
        let n = all.len();
        all.sort_by_key(|&(s, i)| (std::cmp::Reverse(s), rec(i), i));
        let hits = all.into_iter().take(limit);
        return (hits.map(|(_, idx)| Hit { idx, typo: None }).collect(), n);
    }
    // The first word decides what counts as matching the file's name.
    let needle = q.text.split(' ').next().unwrap_or_default();
    let has_dir = q.text.contains('/');
    let mut name_fz = Fuzzy::new(needle);
    let mut score = |base: i32, f: &str| {
        let name = f.rsplit_once('/').map_or(f, |n| n.1);
        let frecency = base.saturating_mul(meta.frecency(f)) / 100;
        let git = if changed(f) { base * 15 / 100 } else { 0 };
        let filename = if needle.eq_ignore_ascii_case(name) {
            base / 5 * 2
        } else if !has_dir && name_fz.score(name).is_some() {
            (base / 6).min(30)
        } else if (5..=11).contains(&name.len()) && entry_point(name) {
            base * 5 / 100
        } else {
            0
        };
        let current = if meta.current.as_deref() == Some(f) {
            -(base / 4)
        } else {
            0
        };
        let combo = match combo {
            Some((p, n)) if p == f && *n >= MIN_COMBO => *n as i32 * COMBO_BOOST,
            Some((p, n)) if p == f => *n as i32 * 5,
            _ => 0,
        };
        // A pasted path: the more of it the file's path ends with, the better.
        let aligned = if has_dir && needle.len() > 10 {
            let same = f
                .bytes()
                .rev()
                .zip(needle.bytes().rev())
                .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
                .count();
            let cover = (same * 100 / needle.len()) as i32;
            if same > 10 && cover >= 30 {
                base * cover / 100
            } else {
                0
            }
        } else {
            0
        };
        base + frecency + git + commits(f) + meta.distance(f) + filename + current + combo + aligned
    };
    let mut fz = Fuzzy::new(&q.text);
    let mut hits: Vec<(i32, usize)> = vec![];
    let mut missed = vec![];
    for i in kept {
        match fz.score(&files[i]) {
            Some(s) => hits.push((score(s as i32, &files[i]), i)),
            None => missed.push(i),
        }
    }
    // Typos: files that lack a few of the query's letters, fewest first.
    let mut slips: Vec<(u32, usize)> = match Lcs::for_names(&q.text) {
        Some(lcs) if hits.len() < TYPO_BELOW => {
            let near = missed.iter().map(|&i| (lcs.missing(&files[i]), i));
            near.filter(|&(m, _)| m <= lcs.allowed).collect()
        }
        _ => vec![],
    };
    slips.sort_unstable_by_key(|&(m, i)| (m, files[i].len(), i));
    // shortcut: the 2000 nearest are scored; raise if a huge tree hides a
    // better two-typo match behind that many.
    slips.truncate(2000);
    let mut hits: Vec<(i32, usize, Option<String>)> =
        hits.into_iter().map(|(s, i)| (s, i, None)).collect();
    let mut fzs: HashMap<String, Fuzzy> = HashMap::new();
    for (m, i) in slips {
        // Match what the file does have of the query, as a query.
        let had = kept_letters(&q.text, &files[i]);
        let fz = fzs.entry(had.clone()).or_insert_with(|| Fuzzy::new(&had));
        if let Some(s) = fz.score(&files[i]) {
            // Each typo costs: one never outranks the file matched as typed.
            let base = s as i32 * 3 / (3 + m as i32);
            hits.push((score(base, &files[i]), i, Some(had)));
        }
    }
    let n = hits.len();
    hits.sort_unstable_by_key(|(s, i, _)| (std::cmp::Reverse(*s), rec(*i), files[*i].len(), *i));
    let hits = hits.into_iter().take(limit);
    (hits.map(|(_, idx, typo)| Hit { idx, typo }).collect(), n)
}

/// How many of a query's letters a path lacks, in order: the longest
/// common subsequence, a machine word at a time (Hyyrö's bit-vectors).
struct Lcs {
    /// For each byte, the query positions holding it (letters as lowercase).
    masks: [u64; 256],
    len: u32,
    /// Typos forgiven: fff's a quarter of the letters, two to six, but
    /// one for four or five letters and none below (two typos in four
    /// letters would match nearly anything).
    allowed: u32,
}

impl Lcs {
    /// For file names: None for queries matched only as typed.
    fn for_names(query: &str) -> Option<Lcs> {
        // shortcut: one word; split into words if multi-word queries
        // should forgive typos too.
        let n = query.len();
        if n < 4 || query.contains(' ') {
            return None;
        }
        Lcs::new(query, if n < 6 { 1 } else { (n / 4).clamp(2, 6) as u32 })
    }

    /// None unless `query` is 1 to 64 ASCII characters.
    fn new(query: &str, allowed: u32) -> Option<Lcs> {
        let n = query.len();
        if !(1..=64).contains(&n) || !query.is_ascii() {
            return None;
        }
        let mut masks = [0u64; 256];
        for (i, b) in query.bytes().enumerate() {
            masks[b.to_ascii_lowercase() as usize] |= 1 << i;
        }
        Some(Lcs {
            masks,
            len: n as u32,
            allowed,
        })
    }

    fn missing(&self, path: &str) -> u32 {
        let mut v = !0u64;
        for b in path.bytes() {
            let u = v & self.masks[b.to_ascii_lowercase() as usize];
            v = v.wrapping_add(u) | (v - u);
        }
        let all = !0u64 >> (64 - self.len);
        self.len - (!v & all).count_ones()
    }
}

/// Fuzzy grep, by fff's rules: a line matches when it has the pattern's
/// letters in order, give or take two, close together.
pub struct Loose {
    query: String,
    lcs: Lcs,
    /// One matcher per set of letters a line turned out to have.
    fzs: HashMap<String, Fuzzy>,
}

impl Loose {
    /// None for patterns that can only be matched as typed (not ASCII, or
    /// over 64 characters).
    pub fn new(query: &str) -> Option<Loose> {
        // A third of the letters may be off, two at most.
        let lcs = Lcs::new(query, (query.len() / 3).min(2) as u32)?;
        Some(Loose {
            query: query.to_string(),
            lcs,
            fzs: HashMap::new(),
        })
    }

    /// The byte range of the match in `line`.
    pub fn find(&mut self, line: &str) -> Option<(usize, usize)> {
        let n = self.query.len();
        let missing = self.lcs.missing(line);
        if missing > self.lcs.allowed {
            return None;
        }
        let had = if missing == 0 {
            self.query.clone()
        } else {
            kept_letters(&self.query, line)
        };
        let fz = match self.fzs.get_mut(&had) {
            Some(fz) => fz,
            None => self.fzs.entry(had.clone()).or_insert(Fuzzy::literal(&had)),
        };
        let at = fz.ranges(line);
        let (first, last) = (at.first()?.0, at.last()?.1);
        // Not scattered: within three times the pattern's length, mostly
        // unbroken (delimiters in `some_long_name` are allowed for), and
        // denser when letters are missing.
        let span = last - first;
        let gaps = at.windows(2).filter(|w| w[1].0 != w[0].1).count();
        let dense = if at.len() >= n { 45 } else { 65 };
        let close = span <= n * 3 && at.len() * 100 / span >= dense && gaps <= (n / 3).max(2);
        (at.len() + self.lcs.allowed as usize >= n && close).then_some((first, last))
    }
}

/// The letters of `query` (ASCII) that `path` has, in order: one longest
/// common subsequence, the latest letters preferred (the file's name).
fn kept_letters(query: &str, path: &str) -> String {
    let (q, p) = (query.as_bytes(), path.as_bytes());
    let same = |i: usize, j: usize| q[i].eq_ignore_ascii_case(&p[j]);
    // best[i][j]: the longest for q[i..] and p[j..].
    let mut best = vec![vec![0u16; p.len() + 1]; q.len() + 1];
    for i in (0..q.len()).rev() {
        for j in (0..p.len()).rev() {
            best[i][j] = if same(i, j) {
                best[i + 1][j + 1] + 1
            } else {
                best[i + 1][j].max(best[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < q.len() && j < p.len() {
        if same(i, j) && best[i][j] == best[i + 1][j + 1] + 1 {
            out.push(q[i] as char);
            (i, j) = (i + 1, j + 1);
        } else if best[i + 1][j] >= best[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    fn top(q: &str, files: &[String], meta: &Meta, combo: Option<&(String, u32)>) -> Vec<String> {
        let (hits, _) = rank(&Query::parse(q), files, meta, combo, 10);
        hits.iter().map(|h| files[h.idx].clone()).collect()
    }

    #[test]
    fn frecency_decays_and_saturates() {
        let now = 1_000 * DAY;
        assert_eq!(access_score(&[], now), 0);
        assert_eq!(access_score(&[now], now), 1);
        // Ten days halves it; a month ago is forgotten.
        assert_eq!(access_score(&[now - 10 * DAY; 4], now), 2);
        assert_eq!(access_score(&[now - 31 * DAY; 9], now), 0);
        // Past ten, growth is the square root.
        assert_eq!(access_score(&[now; 26], now), 14);
        assert_eq!(modification_score(60), 16);
        assert_eq!(modification_score(60 * 15), 8);
        assert_eq!(modification_score(DAY * 8), 0);
        assert!((9..16).contains(&modification_score(60 * 5)));
    }

    #[test]
    fn tracks_opens_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("state/frecency");
        let now = 1_000 * DAY;
        track(&f, Path::new("/p/a"), now - 40 * DAY);
        track(&f, Path::new("/p/b"), now - 1);
        for _ in 0..3 {
            track(&f, Path::new("/p/sub/c"), now);
        }
        let s = access_scores(&f, Path::new("/p"), now);
        // The old open of `a` is dropped, not just outscored.
        assert_eq!(s, HashMap::from([("b".into(), 1), ("sub/c".into(), 3)]));
        assert_eq!(stamps(&f).len(), 2);
        for i in 0..200 {
            track(&f, Path::new("/p/b"), now + i);
        }
        assert_eq!(stamps(&f)[0].1.len(), MAX_STAMPS);
    }

    #[test]
    fn remembers_what_a_query_led_to() {
        let dir = tempfile::tempdir().unwrap();
        let (f, root) = (dir.path().join("queries"), Path::new("/p"));
        assert_eq!(last_query(&f, root, "ses"), None);
        track_query(&f, root, "ses", Path::new("/p/src/session.rs"));
        track_query(&f, root, " ses ", Path::new("/p/src/session.rs"));
        assert_eq!(
            last_query(&f, root, "ses"),
            Some(("src/session.rs".into(), 2))
        );
        // Another project, another memory; another file starts over.
        assert_eq!(last_query(&f, Path::new("/q"), "ses"), None);
        track_query(&f, root, "ses", Path::new("/p/src/search.rs"));
        assert_eq!(
            last_query(&f, root, "ses"),
            Some(("src/search.rs".into(), 1))
        );
        track_query(&f, root, "", Path::new("/p/x"));
        assert_eq!(queries(&f).len(), 1);
    }

    #[test]
    fn query_filters_and_locations() {
        let files = v(&[
            "src/session.rs",
            "src/ui/render.rs",
            "docs/session.md",
            "tests/session_test.rs",
        ]);
        let mut meta = Meta::default();
        meta.status
            .insert("docs/session.md".into(), FileState::Modified);
        let t = |q: &str| {
            let mut t = top(q, &files, &meta, None);
            t.sort();
            t
        };
        assert_eq!(t("*.md"), ["docs/session.md"]);
        assert_eq!(
            t("session *.rs"),
            ["src/session.rs", "tests/session_test.rs"]
        );
        assert_eq!(t("session !*.rs"), ["docs/session.md"]);
        assert_eq!(t("ui/"), ["src/ui/render.rs"]);
        assert_eq!(t("session !src/ !docs/"), ["tests/session_test.rs"]);
        assert_eq!(t("status:modified"), ["docs/session.md"]);
        assert_eq!(t("git:m"), ["docs/session.md"]);
        assert_eq!(t("status:unmodified").len(), 3);
        assert_eq!(t("**/*_test.rs"), ["tests/session_test.rs"]);
        assert_eq!(t("type:markdown"), ["docs/session.md"]);
        let q = Query::parse("render:12");
        assert_eq!((q.text.as_str(), q.at), ("render", Some((12, 1))));
        let q = Query::parse("*.rs render.rs:12:3");
        assert_eq!((q.text.as_str(), q.at), ("render.rs", Some((12, 3))));
        assert_eq!(Query::parse("a:b").at, None);
        // Grep keeps its pattern whole: only clear filters are taken out.
        let q = Query::parse_grep(r"fn \w+\(.*\) *.rs !tests/");
        assert_eq!(q.text, r"fn \w+\(.*\)");
        assert!(q.keeps("src/a.rs", &meta.status) && !q.keeps("tests/a.rs", &meta.status));
        assert_eq!(
            Query::parse_grep("a  b? [x]* http://").text,
            "a  b? [x]* http://"
        );
    }

    #[test]
    fn ranking_follows_fff() {
        let files = v(&[
            "src/pick/mod.rs",
            "src/pick/model.rs",
            "src/ui/model.rs",
            "docs/model.md",
        ]);
        let mut meta = Meta::default();
        // The file's name matching beats a directory's.
        assert!(top("model.rs", &files, &meta, None)[0].ends_with("/model.rs"));
        // Nearer the file being edited.
        meta.current = Some("src/pick/grep.rs".into());
        assert_eq!(top("model.rs", &files, &meta, None)[0], "src/pick/model.rs");
        // The current file itself sinks.
        meta.current = Some("src/pick/model.rs".into());
        assert_eq!(top("model.rs", &files, &meta, None)[0], "src/ui/model.rs");
        // Opened often.
        meta.current = None;
        meta.access.insert("docs/model.md".into(), 10);
        assert_eq!(top("model", &files, &meta, None)[0], "docs/model.md");
        // With nothing typed: by frecency, then as listed.
        assert_eq!(top("", &files, &meta, None)[0], "docs/model.md");
        // Picked for this query three times running.
        let combo = ("src/ui/model.rs".to_string(), 3);
        assert_eq!(
            top("model", &files, &meta, Some(&combo))[0],
            "src/ui/model.rs"
        );
        let once = ("src/ui/model.rs".to_string(), 1);
        assert_eq!(top("model", &files, &meta, Some(&once))[0], "docs/model.md");
    }

    #[test]
    fn typos_still_find_the_file() {
        let files = v(&[
            "src/session.rs",
            "src/search.rs",
            "src/controller.rs",
            "README.md",
        ]);
        let meta = Meta::default();
        let found = |q: &str| {
            let (hits, n) = rank(&Query::parse(q), &files, &meta, None, 10);
            let top = hits
                .first()
                .map(|h| (files[h.idx].as_str(), h.typo.clone()));
            (n, top)
        };
        // Swapped letters: the file has all but one of them, in order.
        assert_eq!(
            found("sesison"),
            (1, Some(("src/session.rs", Some("sesson".into()))))
        );
        // Two slips in a longer word.
        let (n, top) = found("cnotrolelr");
        assert_eq!((n, top.unwrap().0), (1, "src/controller.rs"));
        // A wrong letter, and a doubled one.
        assert_eq!(found("sezsion").1.unwrap().0, "src/session.rs");
        assert_eq!(found("reaadme").1.unwrap().0, "README.md");
        // Too far off, or too short to guess at.
        assert_eq!(found("cxnxrxlxer").0, 0);
        assert_eq!(found("xq").0, 0);
        assert_eq!(found("sqz").0, 0);
        // As typed wins over a slip.
        let files = v(&["src/sesion.rs", "src/session.rs"]);
        let (hits, n) = rank(&Query::parse("sesion"), &files, &meta, None, 10);
        assert_eq!((n, hits[0].typo.is_none()), (2, true));
    }

    #[test]
    fn loose_lines() {
        let find = |q: &str, line: &str| {
            let at = Loose::new(q).unwrap().find(line)?;
            Some(line[at.0..at.1].to_string())
        };
        // Letters in between, as long as they stay close.
        assert_eq!(
            find("UserController", "x = UserAuthController::new()").unwrap(),
            "UserAuthController"
        );
        assert_eq!(
            find("flvencodepicture", "int ff_flv_encode_picture_header(void)").unwrap(),
            "flv_encode_picture"
        );
        assert_eq!(find("total", "t = (o, t); a = l"), None);
        // Up to two slips: swapped, wrong or extra letters.
        assert_eq!(
            find("shcema", "CREATE TABLE schema_info").unwrap(),
            "schema"
        );
        assert_eq!(
            find("recieve_mesage", "fn receive_message(&self)").unwrap(),
            "receive_message"
        );
        assert_eq!(
            find("!importnat", "color: red !important;").unwrap(),
            "!important"
        );
        assert_eq!(find("fn mian", "pub fn main() {").unwrap(), "fn main");
        // Three are too many, and short patterns forgive less.
        assert_eq!(find("rxcxevx_message", "fn receive_message(&self)"), None);
        assert_eq!(find("ab", "b a"), None);
        assert_eq!(find("fo", "foo").unwrap(), "fo");
        assert!(Loose::new("naïve").is_none());
    }

    #[test]
    fn counts_missing_letters() {
        let lcs = |q: &str, p: &str| Lcs::new(q, 0).unwrap().missing(p);
        assert_eq!(lcs("session", "src/session.rs"), 0);
        assert_eq!(lcs("SESISON", "src/session.rs"), 1);
        assert_eq!(lcs("zzzz", "src/session.rs"), 4);
        assert_eq!(lcs(&"a".repeat(64), &"a".repeat(70)), 0);
        assert_eq!(lcs(&"ab".repeat(32), "aaaa"), 60);
        let allowed = |n: usize| Lcs::for_names(&"a".repeat(n)).map(|l| l.allowed);
        assert_eq!(
            [3, 4, 5, 6, 11, 12, 24, 40].map(allowed),
            [
                None,
                Some(1),
                Some(1),
                Some(2),
                Some(2),
                Some(3),
                Some(6),
                Some(6)
            ]
        );
        assert_eq!(kept_letters("sesison", "src/session.rs"), "sesson");
        assert_eq!(kept_letters("xyz", "abc"), "");
    }
}
