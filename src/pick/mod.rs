//! Pickers: files (`Space p`), grep (`Space g`), lines of this file (`Space k`).

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
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self.kind {
            Kind::Files => "find> ",
            Kind::Grep => "grep> ",
            Kind::Lines => "lines> ",
        }
    }

    /// Bring `rows` up to date; true if anything shown changed.
    fn update(&mut self, project: &Project, buf: &Buffer, now: Instant) -> bool {
        match self.kind {
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

/// `Space p` / `Space g` / `Space k`.
pub fn open(ed: &mut Editor, kind: Kind) {
    let mut p = Picker::new(kind, &ed.project, ed.cur.line);
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
