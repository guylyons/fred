//! Pickers: files (`Space p`), grep (`Space g`), lines of this file
//! (`Space k`), find-file browsing (`Space j`, `fred DIR`), and recent
//! files (`Space r`).

pub mod browse;
pub mod def;
pub mod files;
pub mod fuzzy;
pub mod grep;
pub mod lines;
pub mod recent;

use crate::buffer::Buffer;
use crate::editor::{CmdLine, Editor, Mode};
use crate::ex::ExEffect;
use crate::key::{Key, KeyCode};
use files::Files;
use grep::Grep;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Grep starts this long after the last keystroke.
const DEBOUNCE: Duration = Duration::from_millis(100);
/// Rows kept per query (more than any window shows).
const LIMIT: usize = 500;

/// What the pickers search, shared across `:e` (which replaces the editor).
#[derive(Debug, Default)]
pub struct Project {
    /// Where to look for the repo: the edited file's directory.
    dir: Option<PathBuf>,
    pub recent_file: Option<PathBuf>,
    files: Mutex<Option<Arc<Files>>>,
    pub grep: Arc<Grep>,
    /// `git status` per repository root, loading in the background (for
    /// the dots in the file pickers), and "one has just loaded".
    status: Mutex<HashMap<PathBuf, Arc<Mutex<Option<Status>>>>>,
    status_loaded: Arc<AtomicBool>,
}

type Status = HashMap<String, crate::git::FileState>;

impl Project {
    pub fn new(dir: Option<PathBuf>, recent_file: Option<PathBuf>) -> Project {
        Project {
            dir,
            recent_file,
            ..Project::default()
        }
    }

    /// The file list, walked on first use.
    pub fn files(&self) -> Arc<Files> {
        let mut f = self.files.lock().unwrap();
        Arc::clone(f.get_or_insert_with(|| {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let dir = self.dir.clone().unwrap_or_else(|| cwd.clone());
            Files::spawn(files::project_root(&dir, &cwd), false)
        }))
    }

    /// Walk again, honoring ignore files or (`all`) not.
    fn set_all(&self, all: bool) {
        let root = self.files().root.clone();
        *self.files.lock().unwrap() = Some(Files::spawn(root, all));
    }

    /// `path`'s state in `git status` (for a directory: of anything inside
    /// it, modified first). None outside git, unchanged, or still loading.
    pub fn file_state(&self, path: &Path, dir: bool) -> Option<crate::git::FileState> {
        use crate::git::FileState;
        let root = crate::complete::nearby::repo_root(if dir { path } else { path.parent()? })?;
        let rel = path.strip_prefix(&root).ok()?.to_str()?.to_string();
        let slot = {
            let mut m = self.status.lock().ok()?;
            let slot = m.entry(root.clone()).or_insert_with(|| {
                let slot: Arc<Mutex<Option<Status>>> = Arc::default();
                let (out, loaded) = (Arc::clone(&slot), Arc::clone(&self.status_loaded));
                std::thread::spawn(move || {
                    let st = crate::git::status(&root).unwrap_or_default();
                    if let Ok(mut o) = out.lock() {
                        *o = Some(st);
                    }
                    loaded.store(true, Ordering::Release);
                });
                slot
            });
            Arc::clone(slot)
        };
        let st = slot.lock().ok()?;
        let st = st.as_ref()?;
        if !dir {
            return st.get(&rel).copied();
        }
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        let mut inside = st
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix))
            .map(|(_, s)| *s);
        let first = inside.next()?;
        Some(
            if first == FileState::Modified || inside.any(|s| s == FileState::Modified) {
                FileState::Modified
            } else {
                FileState::New
            },
        )
    }

    /// Forget `git status`: a file picker opening reads it fresh.
    fn reset_status(&self) {
        if let Ok(mut m) = self.status.lock() {
            m.clear();
        }
    }

    /// Recent files in this project: relative path → rank (0 = newest).
    fn recent(&self) -> HashMap<String, usize> {
        let Some(f) = &self.recent_file else {
            return HashMap::new();
        };
        let root = &self.files().root.clone();
        recent::load(f)
            .iter()
            .filter_map(|p| p.strip_prefix(root).ok()?.to_str().map(str::to_string))
            .enumerate()
            .map(|(i, p)| (p, i))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Files,
    Grep,
    Lines,
    Browse,
    Recent,
    Buffers,
    /// Lines of every buffer (`Space B`).
    AllLines,
    /// Definitions of the word at the cursor (`Space d`, `gd`).
    Def,
}

/// One result: what to show and where it leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    /// Byte ranges of `text` to highlight.
    pub hl: Vec<(usize, usize)>,
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    /// Where the code starts in `text` (after `12: ` or `path:12: `), for
    /// syntax highlighting; None for rows that are just paths.
    pub code: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub kind: Kind,
    pub query: CmdLine,
    /// Best first.
    pub rows: Vec<Row>,
    pub sel: usize,
    /// Status line text: counts, or why grep can't run.
    pub status: String,
    pub err: bool,
    /// Browse: files below are still being listed (no match isn't final).
    pub searching: bool,
    recent: HashMap<String, usize>,
    /// What `rows` were built from. Files: query, file count, walk done.
    seen_files: Option<(String, usize, bool)>,
    /// Grep: generation, hits, done.
    seen_grep: (u64, usize, bool),
    /// Grep: the query being (or last) run, and when the query last changed.
    started: Option<String>,
    changed: Instant,
    /// Lines: the cursor's line when the picker opened.
    origin: usize,
    /// Browse: the directory listed last (with dotfiles?) and its entries,
    /// and every file below it once a name is being typed.
    listing: Option<(PathBuf, bool, Result<Vec<browse::Entry>, String>)>,
    walk: Option<Arc<Files>>,
    /// Recent: the files, newest first, as shown (`~/…`) and where they
    /// are. Buffers: the same, by last use, and each one's number.
    recent_files: Vec<(String, PathBuf, usize)>,
    /// AllLines: each buffer's name, path and text, this one first.
    buf_texts: Vec<(String, PathBuf, ropey::Rope)>,
    /// Def: the file searched from (its definitions rank first), and
    /// whether to jump straight to a clear winner (until a key is typed).
    from: Option<PathBuf>,
    jump: bool,
    /// Def: each row's tier and "in `from`", for the jump.
    ranks: Vec<(u8, bool)>,
}

impl Picker {
    fn new(kind: Kind, project: &Project, origin: usize) -> Picker {
        Picker {
            kind,
            query: CmdLine::default(),
            rows: vec![],
            sel: 0,
            status: String::new(),
            err: false,
            searching: false,
            recent: if kind == Kind::Files {
                project.recent()
            } else {
                HashMap::new()
            },
            seen_files: None,
            seen_grep: (0, 0, false),
            started: None,
            changed: Instant::now(),
            origin,
            listing: None,
            walk: None,
            recent_files: vec![],
            buf_texts: vec![],
            from: None,
            jump: false,
            ranks: vec![],
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self.kind {
            Kind::Files => "find> ",
            Kind::Grep => "grep> ",
            Kind::Lines => "lines> ",
            Kind::Browse => "find file: ",
            Kind::Recent => "recent> ",
            Kind::Buffers => "buffer> ",
            Kind::AllLines => "all lines> ",
            Kind::Def => "definition> ",
        }
    }

    /// Bring `rows` up to date; true if anything shown changed.
    fn update(&mut self, project: &Project, buf: &Buffer, now: Instant) -> bool {
        match self.kind {
            Kind::Recent | Kind::Buffers => {
                let q = &self.query.text;
                if self.seen_files.as_ref().is_some_and(|s| s.0 == *q) {
                    return false;
                }
                self.seen_files = Some((q.clone(), 0, true));
                self.sel = 0;
                let names: Vec<String> = self.recent_files.iter().map(|f| f.0.clone()).collect();
                let rank: HashMap<String, usize> = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.clone(), i))
                    .collect();
                let (idx, matched) = fuzzy::rank(q, &names, &rank, LIMIT);
                let mut fz = fuzzy::Fuzzy::new(q);
                self.rows = idx
                    .iter()
                    .map(|&i| Row {
                        hl: if q.is_empty() {
                            vec![]
                        } else {
                            fz.ranges(&names[i])
                        },
                        text: names[i].clone(),
                        path: self.recent_files[i].1.clone(),
                        line: self.recent_files[i].2,
                        col: 0,
                        code: None,
                    })
                    .collect();
                self.status = format!("{matched}/{}", names.len());
                true
            }
            Kind::Browse => {
                let q = self.query.text.clone();
                let (d, name) = browse::split(&q);
                let dir = browse::resolve(d);
                let hidden = name.starts_with('.');
                if self
                    .listing
                    .as_ref()
                    .is_none_or(|l| (&l.0, l.1) != (&dir, hidden))
                {
                    self.listing = Some((dir.clone(), hidden, browse::list(&dir, hidden)));
                }
                // Typing a name searches every file below, as consult does.
                let walk = (!name.is_empty()).then(|| {
                    if self.walk.as_ref().is_none_or(|w| w.root != dir) {
                        self.walk = Some(Files::spawn(dir.clone(), false));
                    }
                    Arc::clone(self.walk.as_ref().unwrap())
                });
                let now = Some((
                    q.clone(),
                    walk.as_ref().map_or(0, |w| w.len()),
                    walk.as_ref().is_none_or(|w| w.done()),
                ));
                if now == self.seen_files {
                    return false;
                }
                if self.seen_files.as_ref().is_none_or(|s| s.0 != q) {
                    self.sel = 0;
                }
                self.seen_files = now;
                let entries = match &self.listing.as_ref().unwrap().2 {
                    Ok(e) => e,
                    Err(e) => {
                        self.rows.clear();
                        self.status = e.clone();
                        self.err = true;
                        return true;
                    }
                };
                // Directories here (to go into), then files: all of them
                // below when searching, else just this directory's.
                let mut cands: Vec<String> = entries
                    .iter()
                    .filter(|e| e.dir)
                    .map(|e| format!("{}/", e.name))
                    .collect();
                match &walk {
                    Some(w) if !hidden => w.with(|l| cands.extend(l.iter().cloned())),
                    _ => cands.extend(entries.iter().filter(|e| !e.dir).map(|e| e.name.clone())),
                }
                let mut fz = fuzzy::Fuzzy::new(name);
                let mut hits: Vec<(u32, usize)> = cands
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| fz.score(c).map(|s| (s, i)))
                    .collect();
                let matched = hits.len();
                // Ties: shorter paths (nearer this directory) first.
                hits.sort_by_key(|&(s, i)| (std::cmp::Reverse(s), cands[i].len(), i));
                self.rows = hits
                    .iter()
                    .take(LIMIT)
                    .map(|&(_, i)| Row {
                        hl: if name.is_empty() {
                            vec![]
                        } else {
                            fz.ranges(&cands[i])
                        },
                        path: dir.join(cands[i].trim_end_matches('/')),
                        text: cands[i].clone(),
                        line: 0,
                        col: 0,
                        code: None,
                    })
                    .collect();
                self.sel = self.sel.min(self.rows.len().saturating_sub(1));
                self.err = false;
                self.searching = walk.as_ref().is_some_and(|w| !w.done());
                let more = match &walk {
                    Some(w) if w.truncated() => "+",
                    Some(w) if !w.done() => "…",
                    _ => "",
                };
                self.status = if self.rows.is_empty() && !name.is_empty() && more.is_empty() {
                    format!("new file: {name}")
                } else {
                    format!("{matched}/{}{more}", cands.len())
                };
                true
            }
            Kind::Lines | Kind::AllLines => {
                let q = &self.query.text;
                if self.seen_files.as_ref().is_some_and(|s| s.0 == *q) {
                    return false;
                }
                self.seen_files = Some((q.clone(), 0, true));
                let (found, total) = if self.kind == Kind::Lines {
                    (lines::search(q, buf.rope(), self.origin), buf.len_lines())
                } else {
                    let total = self.buf_texts.iter().map(|b| b.2.len_lines()).sum();
                    (lines::search_all(q, &self.buf_texts, self.origin), total)
                };
                match found {
                    Ok(f) => {
                        self.status = format!("{}/{total} lines", f.rows.len());
                        self.err = false;
                        self.rows = f.rows;
                        self.sel = f.sel;
                    }
                    // Keep the last results; say why they didn't change.
                    Err(e) => {
                        self.status = e;
                        self.err = true;
                    }
                }
                true
            }
            Kind::Files => {
                let files = project.files();
                let n = files.len();
                let q = &self.query.text;
                let now = Some((q.clone(), n, files.done()));
                if now == self.seen_files {
                    return false;
                }
                if self.seen_files.as_ref().is_none_or(|s| s.0 != *q) {
                    self.sel = 0;
                }
                self.seen_files = now;
                let (idx, matched) = files.with(|l| fuzzy::rank(q, l, &self.recent, LIMIT));
                let mut fz = fuzzy::Fuzzy::new(q);
                self.rows = files.with(|l| {
                    idx.iter()
                        .map(|&i| Row {
                            hl: if q.is_empty() {
                                vec![]
                            } else {
                                fz.ranges(&l[i])
                            },
                            text: l[i].clone(),
                            path: files.root.join(&l[i]),
                            line: 0,
                            col: 0,
                            code: None,
                        })
                        .collect()
                });
                let more = if files.truncated() {
                    "+ (list truncated)"
                } else if files.done() {
                    ""
                } else {
                    "…"
                };
                self.status = format!("{matched}/{n}{more}{}", all_tag(&files));
                self.sel = self.sel.min(self.rows.len().saturating_sub(1));
                true
            }
            Kind::Grep | Kind::Def => {
                let files = project.files();
                let q = self.query.text.clone();
                let mut changed = false;
                if self.started.as_deref() != Some(&q) && now >= self.changed + DEBOUNCE {
                    self.started = Some(q.clone());
                    if q.is_empty() {
                        project.grep.cancel();
                        self.rows.clear();
                        self.status.clear();
                        self.err = false;
                        self.seen_grep = (0, 0, false);
                        return true;
                    }
                    let re = if self.kind == Kind::Def {
                        def::regex(&q)
                    } else {
                        crate::search::compile(&q)
                    };
                    match re {
                        Ok(re) => {
                            self.err = false;
                            project.grep.start(re, Arc::clone(&files));
                        }
                        Err(e) => {
                            // Keep the last results; say why they didn't change.
                            self.status = e;
                            self.err = true;
                            return true;
                        }
                    }
                    changed = true;
                }
                let r = project.grep.results();
                if self.err || r.generation == 0 || q.is_empty() {
                    return changed;
                }
                let now = (r.generation, r.hits.len(), r.done);
                if now == self.seen_grep && !changed {
                    return false;
                }
                if r.generation != self.seen_grep.0 {
                    self.sel = 0;
                }
                self.seen_grep = now;
                if self.kind == Kind::Def {
                    // Best tier, then this file, then files of its type.
                    let res = def::tier_regexes(&q);
                    let from = self.from.as_deref();
                    let ext = from.and_then(Path::extension);
                    let mut hits: Vec<_> = r
                        .hits
                        .iter()
                        .filter_map(|h| {
                            let row = grep_row(h, &files.root);
                            let t = def::tier(&h.text, &res)?;
                            let here = from == Some(row.path.as_path());
                            let other_type = row.path.extension() != ext;
                            Some(((t, !here, other_type), row))
                        })
                        .collect();
                    hits.sort_by_key(|h| h.0);
                    hits.truncate(LIMIT);
                    self.ranks = hits.iter().map(|((t, away, _), _)| (*t, !away)).collect();
                    self.rows = hits.into_iter().map(|(_, row)| row).collect();
                } else {
                    self.rows = r
                        .hits
                        .iter()
                        .take(LIMIT)
                        .map(|h| grep_row(h, &files.root))
                        .collect();
                }
                let n = self.rows.len();
                let what = if self.kind == Kind::Def {
                    "definitions"
                } else {
                    "matches"
                };
                self.status = match (r.capped, r.done) {
                    (true, _) => format!("{n}+ {what}"),
                    (_, true) => format!("{n} {what}"),
                    _ => format!("{n} {what}…"),
                } + all_tag(&files);
                self.sel = self.sel.min(self.rows.len().saturating_sub(1));
                true
            }
        }
    }
}

/// Marks a list that ignores `.gitignore` (`Ctrl-o`).
fn all_tag(files: &Files) -> &'static str {
    if files.all { " [all]" } else { "" }
}

fn grep_row(h: &grep::Hit, root: &std::path::Path) -> Row {
    let indent = h.text.len() - h.text.trim_start().len();
    let head = format!("{}:{}: ", h.path, h.line + 1);
    let shift = |b: usize| head.len() + b.saturating_sub(indent).min(h.text.len() - indent);
    Row {
        hl: vec![(shift(h.col), shift(h.end))],
        text: format!("{head}{}", &h.text[indent..]),
        path: root.join(&h.path),
        line: h.line,
        col: h.col,
        code: Some(head.len()),
    }
}

/// Find-file starting in `dir` (`Space j`, `fred DIR`, `:e DIR`).
pub fn browse(ed: &mut Editor, dir: &std::path::Path) {
    open(ed, Kind::Browse);
    if let Mode::Pick(p) = &mut ed.mode {
        set_query(p, browse::show(dir));
        p.update(&ed.project, &ed.buf, Instant::now());
    }
}

/// `Space d` / `gd`: find the definition of the word at the cursor,
/// jumping straight there when one stands out.
pub fn definition(ed: &mut Editor) {
    let line = ed.buf.line(ed.cur.line);
    let Some(w) = def::word_at(&line, ed.cur.byte).map(str::to_string) else {
        return ed.set_err("no name under the cursor".to_string());
    };
    let from = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
    open(ed, Kind::Def);
    if let Mode::Pick(p) = &mut ed.mode {
        set_query(p, w);
        p.from = from;
        p.jump = true;
        // No debounce: the word is already typed.
        p.changed = Instant::now() - DEBOUNCE;
        p.update(&ed.project, &ed.buf, Instant::now());
    }
}

/// Def, when the grep is done: jump to a clear winner, or say there is
/// nothing; else leave the list up.
fn auto_jump(ed: &mut Editor) {
    let Mode::Pick(p) = &mut ed.mode else {
        return;
    };
    let r = ed.project.grep.results();
    if p.kind != Kind::Def || !p.jump || p.err || !r.done || r.generation != p.seen_grep.0 {
        return;
    }
    drop(r);
    p.jump = false;
    let clear = match p.ranks.as_slice() {
        [] => {
            let msg = format!("no definition of {} found", p.query.text);
            ed.mode = Mode::Normal;
            return ed.set_err(msg);
        }
        [_] => true,
        [a, b, ..] => a != b,
    };
    if clear {
        let r = &p.rows[0];
        ed.pending_effect = Some(ExEffect::Open {
            path: r.path.clone(),
            line: r.line,
            col: r.col,
            pattern: Some(format!(r"\b{}\b", p.query.text)),
        });
    }
}

/// `Space b` / `:ls`: pick from `list` (name, path, buffer number), most
/// recently used first.
pub fn buffers(ed: &mut Editor, list: Vec<(String, PathBuf, usize)>) {
    open(ed, Kind::Buffers);
    if let Mode::Pick(p) = &mut ed.mode {
        p.recent_files = list;
        p.seen_files = None;
        p.update(&ed.project, &ed.buf, Instant::now());
    }
}

/// `Space B`: search the lines of `bufs` (name, path, text), this one first.
pub fn all_lines(ed: &mut Editor, bufs: Vec<(String, PathBuf, ropey::Rope)>) {
    open(ed, Kind::AllLines);
    if let Mode::Pick(p) = &mut ed.mode {
        p.buf_texts = bufs;
        p.seen_files = None;
        p.update(&ed.project, &ed.buf, Instant::now());
    }
}

fn set_query(p: &mut Picker, q: String) {
    p.query.cursor = q.len();
    p.query.text = q;
}

/// `Space p` / `Space g` / `Space k`.
pub fn open(ed: &mut Editor, kind: Kind) {
    if matches!(kind, Kind::Files | Kind::Browse | Kind::Recent) {
        ed.project.reset_status();
    }
    let mut p = Picker::new(kind, &ed.project, ed.cur.line);
    if kind == Kind::Recent {
        // Not the file being edited, and not files since deleted.
        let me = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
        let list = ed
            .project
            .recent_file
            .as_deref()
            .map(recent::load)
            .unwrap_or_default();
        p.recent_files = list
            .into_iter()
            .filter(|f| Some(f) != me.as_ref() && f.is_file())
            .map(|f| (browse::tilde(&f), f, 0))
            .collect();
    }
    p.update(&ed.project, &ed.buf, Instant::now());
    ed.mode = Mode::Pick(Box::new(p));
}

pub fn pick_key(ed: &mut Editor, k: Key) {
    let Mode::Pick(p) = &mut ed.mode else {
        return;
    };
    match k.code {
        KeyCode::Esc => close(ed),
        KeyCode::Char('c') if k.ctrl => close(ed),
        KeyCode::Enter
            if p.kind == Kind::AllLines
                && p.rows.get(p.sel).is_some_and(|r| {
                    !r.path.as_os_str().is_empty()
                        && ed
                            .path
                            .as_deref()
                            .and_then(|e| std::path::absolute(e).ok())
                            .as_ref()
                            != Some(&r.path)
                }) =>
        {
            let r = &p.rows[p.sel];
            ed.pending_effect = Some(ExEffect::Open {
                path: r.path.clone(),
                line: r.line,
                col: r.col,
                // Always a target (even line 1); an empty pattern isn't a search.
                pattern: Some(
                    p.query
                        .text
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                ),
            });
        }
        KeyCode::Enter if matches!(p.kind, Kind::Lines | Kind::AllLines) => {
            if let Some(r) = p.rows.get(p.sel) {
                let (line, col) = (r.line, r.col);
                // The first word becomes the search, so `n` finds the next.
                if let Some(w) = p.query.text.split_whitespace().next() {
                    ed.last_pat = Some(w.to_string());
                    ed.last_search_fwd = true;
                }
                ed.mode = Mode::Normal;
                ed.set_cursor(line, col);
            }
        }
        KeyCode::Enter | KeyCode::Tab if p.kind == Kind::Browse => {
            let (d, name) = browse::split(&p.query.text);
            let (d, name) = (d.to_string(), name.to_string());
            let target = match p.rows.get(p.sel) {
                // Enter on a directory lists it (dired), as in Emacs.
                Some(r) if r.text.ends_with('/') && k.code == KeyCode::Enter => r.path.clone(),
                // Tab on a directory goes in.
                Some(r) if r.text.ends_with('/') => {
                    set_query(p, format!("{d}{}", r.text));
                    p.update(&ed.project, &ed.buf, Instant::now());
                    return;
                }
                // Tab on a file completes its name.
                Some(r) if k.code == KeyCode::Tab => {
                    set_query(p, format!("{d}{}", r.text));
                    p.update(&ed.project, &ed.buf, Instant::now());
                    return;
                }
                Some(r) => r.path.clone(),
                // Nothing matches (and the search is done): a new file.
                None if !name.is_empty() && k.code == KeyCode::Enter && !p.searching => {
                    browse::resolve(&d).join(name)
                }
                None => return,
            };
            ed.pending_effect = Some(ExEffect::Open {
                path: target,
                line: 0,
                col: 0,
                pattern: None,
            });
        }
        KeyCode::Backspace if p.kind == Kind::Browse && browse::up(&p.query.text).is_some() => {
            let q = browse::up(&p.query.text).unwrap();
            set_query(p, q);
            p.update(&ed.project, &ed.buf, Instant::now());
        }
        KeyCode::Enter if p.kind == Kind::Buffers => {
            if let Some(r) = p.rows.get(p.sel) {
                ed.pending_effect = Some(ExEffect::Buffer {
                    cmd: crate::ex::BufCmd::Go,
                    arg: r.line.to_string(),
                    force: false,
                });
            }
        }
        KeyCode::Enter => {
            if let Some(r) = p.rows.get(p.sel) {
                ed.pending_effect = Some(ExEffect::Open {
                    path: r.path.clone(),
                    line: r.line,
                    col: r.col,
                    pattern: match p.kind {
                        Kind::Grep => Some(p.query.text.clone()),
                        Kind::Def => Some(format!(r"\b{}\b", p.query.text)),
                        _ => None,
                    },
                });
            }
        }
        // Search ignored files too (or stop): Drupal core, vendor/, ….
        KeyCode::Char('o') if k.ctrl && matches!(p.kind, Kind::Files | Kind::Grep | Kind::Def) => {
            ed.project.set_all(!ed.project.files().all);
            p.started = None;
            p.update(&ed.project, &ed.buf, Instant::now());
        }
        // Candidates run downward from the prompt.
        KeyCode::Down => p.sel = (p.sel + 1).min(p.rows.len().saturating_sub(1)),
        KeyCode::Char('n') if k.ctrl => p.sel = (p.sel + 1).min(p.rows.len().saturating_sub(1)),
        KeyCode::Up => p.sel = p.sel.saturating_sub(1),
        KeyCode::Char('p') if k.ctrl => p.sel = p.sel.saturating_sub(1),
        _ => {
            let before = p.query.text.clone();
            p.query.edit(k);
            if p.kind == Kind::Browse
                && let Some(q) = browse::restart(&p.query.text)
            {
                set_query(p, q);
            }
            if p.query.text != before {
                p.jump = false;
                p.changed = Instant::now();
                p.update(&ed.project, &ed.buf, p.changed);
            }
        }
    }
}

fn close(ed: &mut Editor) {
    ed.project.grep.cancel();
    ed.mode = Mode::Normal;
}

/// Background results or the grep debounce: true if the picker changed.
pub fn tick(ed: &mut Editor) -> bool {
    match &mut ed.mode {
        Mode::Pick(p) => {
            // `git status` arriving brings the dots.
            let loaded = ed.project.status_loaded.swap(false, Ordering::AcqRel);
            let changed = p.update(&ed.project, &ed.buf, Instant::now()) || loaded;
            auto_jump(ed);
            changed
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
