//! Pickers: files (`Space p`), grep (`Space g`), lines of this file
//! (`Space k`), find-file browsing (`Space j`, `fred DIR`), and recent
//! files (`Space r`).

pub mod browse;
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
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
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
    files: OnceLock<Arc<Files>>,
    pub grep: Arc<Grep>,
}

impl Project {
    pub fn new(dir: Option<PathBuf>, recent_file: Option<PathBuf>) -> Project {
        Project {
            dir,
            recent_file,
            ..Project::default()
        }
    }

    /// The file list, walked on first use.
    pub fn files(&self) -> &Arc<Files> {
        self.files.get_or_init(|| {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let dir = self.dir.clone().unwrap_or_else(|| cwd.clone());
            Files::spawn(files::project_root(&dir, &cwd))
        })
    }

    /// Recent files in this project: relative path → rank (0 = newest).
    fn recent(&self) -> HashMap<String, usize> {
        let Some(f) = &self.recent_file else {
            return HashMap::new();
        };
        let root = &self.files().root;
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
    /// Recent: the files, newest first, as shown (`~/…`) and where they are.
    recent_files: Vec<(String, PathBuf)>,
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
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self.kind {
            Kind::Files => "find> ",
            Kind::Grep => "grep> ",
            Kind::Lines => "lines> ",
            Kind::Browse => "find file: ",
            Kind::Recent => "recent> ",
        }
    }

    /// Bring `rows` up to date; true if anything shown changed.
    fn update(&mut self, project: &Project, buf: &Buffer, now: Instant) -> bool {
        match self.kind {
            Kind::Recent => {
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
                        line: 0,
                        col: 0,
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
                        self.walk = Some(Files::spawn(dir.clone()));
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
            Kind::Lines => {
                let q = &self.query.text;
                if self.seen_files.as_ref().is_some_and(|s| s.0 == *q) {
                    return false;
                }
                self.seen_files = Some((q.clone(), 0, true));
                match lines::search(q, buf, self.origin) {
                    Ok(f) => {
                        self.status = format!("{}/{} lines", f.rows.len(), buf.len_lines());
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
                self.status = format!("{matched}/{n}{more}");
                self.sel = self.sel.min(self.rows.len().saturating_sub(1));
                true
            }
            Kind::Grep => {
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
                    match crate::search::compile(&q) {
                        Ok(re) => {
                            self.err = false;
                            project.grep.start(re, Arc::clone(files));
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
                self.rows = r
                    .hits
                    .iter()
                    .take(LIMIT)
                    .map(|h| grep_row(h, &files.root))
                    .collect();
                let n = r.hits.len();
                self.status = match (r.capped, r.done) {
                    (true, _) => format!("{n}+ matches"),
                    (_, true) => format!("{n} matches"),
                    _ => format!("{n} matches…"),
                };
                self.sel = self.sel.min(self.rows.len().saturating_sub(1));
                true
            }
        }
    }
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

fn set_query(p: &mut Picker, q: String) {
    p.query.cursor = q.len();
    p.query.text = q;
}

/// `Space p` / `Space g` / `Space k`.
pub fn open(ed: &mut Editor, kind: Kind) {
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
            .map(|f| (browse::tilde(&f), f))
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
        KeyCode::Enter if p.kind == Kind::Lines => {
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
                // A directory: go in.
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
        KeyCode::Enter => {
            if let Some(r) = p.rows.get(p.sel) {
                ed.pending_effect = Some(ExEffect::Open {
                    path: r.path.clone(),
                    line: r.line,
                    col: r.col,
                    pattern: (p.kind == Kind::Grep).then(|| p.query.text.clone()),
                });
            }
        }
        // Best is at the bottom: Up goes to worse matches.
        KeyCode::Up => p.sel = (p.sel + 1).min(p.rows.len().saturating_sub(1)),
        KeyCode::Char('p') if k.ctrl => p.sel = (p.sel + 1).min(p.rows.len().saturating_sub(1)),
        KeyCode::Down => p.sel = p.sel.saturating_sub(1),
        KeyCode::Char('n') if k.ctrl => p.sel = p.sel.saturating_sub(1),
        _ => {
            let before = p.query.text.clone();
            p.query.edit(k);
            if p.kind == Kind::Browse
                && let Some(q) = browse::restart(&p.query.text)
            {
                set_query(p, q);
            }
            if p.query.text != before {
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
        Mode::Pick(p) => p.update(&ed.project, &ed.buf, Instant::now()),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
