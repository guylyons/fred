//! Dired: a directory as a read-only buffer, one entry per line, as in
//! Emacs. Vim motions and search work as anywhere; dired's keys act on the
//! entry under the cursor, on a Visual-line range, or on the marked entries.
//!
//! `Enter` opens, `-`/`^` goes up, `m`/`u`/`U`/`t` mark, unmark, unmark all,
//! toggle; `d` flags for deletion and `x` deletes the flagged; `D` deletes,
//! `R` renames/moves, `C` copies, `+` makes a directory, `M` chmods, `T`
//! touches, `!` runs a shell command on the files, `%` marks by regexp,
//! `s` sorts by name/time, `(` hides details, `gh` hides dotfiles, `gr`
//! re-reads. `i` edits the names in place; `:w` renames (wdired).

use crate::editor::{Editor, Mode};
use crate::ex::ExEffect;
use crate::key::{Key, KeyCode};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Lines before the first entry: the directory's name.
const HEADER: usize = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    /// A directory, or a link to one.
    dir: bool,
    /// A symlink's target.
    link: Option<String>,
    size: u64,
    mtime: Option<SystemTime>,
    mode: u32,
}

/// What the `@` command line is asking for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ask {
    Delete(Vec<PathBuf>),
    Rename(Vec<PathBuf>),
    Copy(Vec<PathBuf>),
    Mkdir,
    Chmod(Vec<PathBuf>),
    Shell(Vec<PathBuf>),
    MarkRegex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dired {
    pub dir: PathBuf,
    entries: Vec<Entry>,
    /// `*` marked or `D` flagged for deletion, by name.
    marks: HashMap<String, char>,
    details: bool,
    by_time: bool,
    hide_dots: bool,
    /// `i`: the names are being edited; `:w` renames.
    pub editing: bool,
    ask: Option<Ask>,
}

/// Show `dir` in `ed` (a new listing, or another directory in this one),
/// the cursor on `focus` if it names an entry.
pub fn visit(ed: &mut Editor, dir: &Path, focus: Option<&str>) -> Result<(), String> {
    let dir = fs::canonicalize(dir)
        .map_err(|e| format!("{}: {}", dir.display(), crate::fileio::err_msg(&e)))?;
    let mut d = ed.dired.take().map_or_else(
        || Dired {
            dir: dir.clone(),
            entries: vec![],
            marks: HashMap::new(),
            details: true,
            by_time: false,
            hide_dots: false,
            editing: false,
            ask: None,
        },
        |b| *b,
    );
    if d.dir != dir {
        d.marks.clear();
    }
    d.dir = dir.clone();
    d.entries = match read(&d) {
        Ok(e) => e,
        Err(e) => {
            ed.dired = Some(Box::new(d));
            return Err(e);
        }
    };
    ed.dired = Some(Box::new(d));
    ed.path = Some(dir);
    ed.readonly = true;
    redraw(ed);
    let line = focus.and_then(|f| line_of(ed, f)).unwrap_or(HEADER + 1);
    goto(ed, line);
    Ok(())
}

/// Dired's handling of `k` in Normal or Visual-line mode; false leaves it
/// to vim.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    let Some(d) = ed.dired.as_ref() else {
        return false;
    };
    // `gr` and `gh` follow vim's pending `g` (`gr` also ends editing).
    if ed.vim.pending.len() == 1 && ed.vim.pending[0].char() == Some('g') {
        let done = match k.char() {
            Some('r') => {
                ed.vim.pending.clear();
                revert(ed);
                true
            }
            Some('h') if !d.editing => {
                ed.vim.pending.clear();
                let hide = !d.hide_dots;
                with(ed, |d| d.hide_dots = hide);
                refresh(ed);
                true
            }
            _ => false,
        };
        return done;
    }
    if d.editing || !ed.vim.pending.is_empty() || k.ctrl || k.alt {
        return false;
    }
    let (lo, hi) = match ed.mode {
        Mode::VisualLine { anchor } => (anchor.min(ed.cur.line), anchor.max(ed.cur.line)),
        Mode::Normal => (ed.cur.line, ed.cur.line),
        _ => return false,
    };
    let visual = matches!(ed.mode, Mode::VisualLine { .. });
    match k.code {
        KeyCode::Enter => open_entry(ed),
        KeyCode::Char(c) => match c {
            '-' | '^' => up(ed),
            'm' => mark(ed, lo, hi, visual, Some('*')),
            'd' => mark(ed, lo, hi, visual, Some('D')),
            'u' => mark(ed, lo, hi, visual, None),
            'U' => {
                with(ed, |d| d.marks.clear());
                redraw(ed);
            }
            't' => toggle(ed),
            'x' => {
                let flagged = paths_marked(ed, 'D');
                if flagged.is_empty() {
                    ed.set_err("no files flagged for deletion (d flags)");
                } else {
                    ask_delete(ed, flagged);
                }
            }
            'D' => {
                let t = targets(ed, lo, hi, visual);
                ask_delete(ed, t);
            }
            'R' => ask_paths(ed, lo, hi, visual, "Rename", Ask::Rename),
            'C' => ask_paths(ed, lo, hi, visual, "Copy", Ask::Copy),
            'M' => {
                let t = targets(ed, lo, hi, visual);
                if !t.is_empty() {
                    let p = format!("Change mode of {} to (octal): ", what(&t));
                    ask(ed, Ask::Chmod(t), &p, "");
                }
            }
            '!' => {
                let t = targets(ed, lo, hi, visual);
                if !t.is_empty() {
                    let p = format!("! on {} (* = the files): ", what(&t));
                    ask(ed, Ask::Shell(t), &p, "");
                }
            }
            '+' => ask(ed, Ask::Mkdir, "Create directory: ", ""),
            '%' => ask(ed, Ask::MarkRegex, "Mark (regexp): ", ""),
            'T' => {
                let t = targets(ed, lo, hi, visual);
                let n = t.len();
                let r = t.iter().try_for_each(|p| touch(p));
                done(ed, r.map(|()| format!("touched {n}")));
            }
            's' => {
                let t = ed.dired.as_ref().is_some_and(|d| !d.by_time);
                with(ed, |d| d.by_time = t);
                refresh(ed);
                ed.set_msg(if t {
                    "sorted by time"
                } else {
                    "sorted by name"
                });
            }
            '(' => {
                let show = ed.dired.as_ref().is_some_and(|d| !d.details);
                with(ed, |d| d.details = show);
                redraw(ed);
            }
            'i' => {
                with(ed, |d| d.editing = true);
                ed.readonly = false;
                ed.set_msg("editing names: :w renames, gr discards");
            }
            // Vim's editing keys: the listing is read-only.
            'a' | 'A' | 'I' | 'o' | 'O' | 'c' | 'S' | 'r' | 'p' | 'P' | 'J' | 'X' | '.' | '>'
            | '<' | '=' | '~' => {
                ed.set_err("dired is read-only (i edits names)");
            }
            _ => return false,
        },
        _ => return false,
    }
    if visual {
        ed.mode = Mode::Normal;
    }
    true
}

/// The answer typed on the `@` command line.
pub fn answer(ed: &mut Editor, text: &str) {
    let Some(a) = ed.dired.as_mut().and_then(|d| d.ask.take()) else {
        return;
    };
    let dir = ed.dired.as_ref().map(|d| d.dir.clone()).unwrap_or_default();
    let text = text.trim();
    // A new name in this directory: the cursor goes to it.
    let focus = text
        .split('/')
        .next()
        .filter(|f| !f.is_empty() && !text.starts_with('~'));
    let r: Result<String, String> = match a {
        Ask::Delete(ps) => {
            if !matches!(text, "y" | "yes") {
                return ed.set_msg("nothing deleted");
            }
            ps.iter()
                .try_for_each(|p| remove(p))
                .map(|()| format!("deleted {}", what(&ps)))
        }
        Ask::Rename(ps) => {
            transfer("mv", &ps, &target(&dir, text)).map(|()| format!("moved {}", what(&ps)))
        }
        Ask::Copy(ps) => {
            transfer("cp", &ps, &target(&dir, text)).map(|()| format!("copied {}", what(&ps)))
        }
        Ask::Mkdir => fs::create_dir_all(dir.join(text))
            .map(|()| format!("created {text}"))
            .map_err(|e| crate::fileio::err_msg(&e)),
        Ask::Chmod(ps) => match u32::from_str_radix(text, 8) {
            Ok(m) => ps
                .iter()
                .try_for_each(|p| {
                    fs::set_permissions(p, fs::Permissions::from_mode(m))
                        .map_err(|e| format!("{}: {}", p.display(), crate::fileio::err_msg(&e)))
                })
                .map(|()| format!("mode {text}: {}", what(&ps))),
            Err(_) => Err(format!("not an octal mode: {text}")),
        },
        Ask::Shell(ps) => {
            if text.is_empty() {
                return;
            }
            ed.pending_effect = Some(ExEffect::Shell(shell_line(&dir, text, &ps)));
            return;
        }
        Ask::MarkRegex => match regex::Regex::new(text) {
            Ok(re) => {
                let mut n = 0;
                with(ed, |d| {
                    for e in d
                        .entries
                        .iter()
                        .filter(|e| e.name != ".." && re.is_match(&e.name))
                    {
                        d.marks.insert(e.name.clone(), '*');
                        n += 1;
                    }
                });
                redraw(ed);
                return ed.set_msg(format!("marked {n}"));
            }
            Err(e) => Err(e
                .to_string()
                .lines()
                .last()
                .unwrap_or("bad regexp")
                .to_string()),
        },
    };
    done(ed, r);
    if let Some(l) = focus.and_then(|f| line_of(ed, f)) {
        goto(ed, l);
    }
}

/// `:w` while editing names: rename each entry whose name changed.
pub fn save(ed: &mut Editor) {
    let Some(d) = ed.dired.as_ref().filter(|d| d.editing) else {
        return ed.set_err("dired: nothing to write (i edits names)");
    };
    let r = renames(ed, d).and_then(|plan| {
        let n = plan.len();
        apply_renames(&d.dir, &plan).map(|()| format!("renamed {n}"))
    });
    if r.is_ok() {
        with(ed, |d| d.editing = false);
        ed.readonly = true;
    }
    done(ed, r);
}

/// Re-read the directory (after a shell command, say).
pub fn refresh(ed: &mut Editor) {
    let Some(d) = ed.dired.as_ref() else { return };
    let here = entry_at(ed, ed.cur.line).map(|e| e.name.clone());
    let (dir, line) = (d.dir.clone(), ed.cur.line);
    match read(d) {
        Ok(entries) => {
            with(ed, |d| {
                d.marks.retain(|n, _| entries.iter().any(|e| &e.name == n));
                d.entries = entries;
            });
            redraw(ed);
            let line = here.and_then(|h| line_of(ed, &h)).unwrap_or(line);
            goto(ed, line);
        }
        Err(e) => ed.set_err(format!("{}: {e}", dir.display())),
    }
}

// ---- reading and drawing ----

fn read(d: &Dired) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(&d.dir).map_err(|e| crate::fileio::err_msg(&e))?;
    let mut entries: Vec<Entry> = rd
        .filter_map(Result::ok)
        .map(|e| entry(&e.path(), e.file_name().to_string_lossy().into_owned()))
        .filter(|e| !(d.hide_dots && e.name.starts_with('.')))
        .collect();
    if d.by_time {
        entries.sort_by(|a, b| b.mtime.cmp(&a.mtime).then(a.name.cmp(&b.name)));
    } else {
        entries.sort_by(|a, b| a.name.cmp(&b.name));
    }
    if d.dir.parent().is_some() {
        entries.insert(0, entry(&d.dir.join(".."), "..".into()));
    }
    Ok(entries)
}

fn entry(path: &Path, name: String) -> Entry {
    let lm = fs::symlink_metadata(path).ok();
    let link = lm
        .as_ref()
        .filter(|m| m.file_type().is_symlink())
        .and_then(|_| fs::read_link(path).ok())
        .map(|t| t.display().to_string());
    Entry {
        dir: path.is_dir(),
        link,
        size: lm.as_ref().map_or(0, fs::Metadata::len),
        mtime: lm.as_ref().and_then(|m| m.modified().ok()),
        mode: lm.as_ref().map_or(0, |m| m.permissions().mode()),
        name,
    }
}

/// Where names start on a line: after the mark and the details.
fn name_col(ed: &Editor) -> usize {
    match ed.dired.as_ref() {
        Some(d) if d.details => 2 + 10 + 1 + 6 + 1 + 12 + 1,
        _ => 2,
    }
}

fn line_text(d: &Dired, e: &Entry) -> String {
    let mark = d.marks.get(&e.name).copied().unwrap_or(' ');
    let slash = if e.dir && e.link.is_none() { "/" } else { "" };
    let link = e
        .link
        .as_ref()
        .map_or(String::new(), |t| format!(" -> {t}"));
    if d.details {
        format!(
            "{mark} {} {:>6} {} {}{slash}{link}",
            mode_str(e),
            human(e.size),
            date(e.mtime),
            e.name
        )
    } else {
        format!("{mark} {}{slash}{link}", e.name)
    }
}

/// Dired+ field coloring, using entry metadata rather than parsing filenames.
/// Edited names can move the fields, so wdired stays unstyled until re-read.
pub(crate) fn styles(ed: &Editor, line: usize) -> crate::highlight::LineStyles {
    use ratatui::style::{Color, Modifier, Style};
    let Some(d) = ed.dired.as_ref().filter(|d| !d.editing) else {
        return vec![];
    };
    let text = ed.buf.line(line);
    if line < HEADER {
        return vec![(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            0..text.len(),
        )];
    }
    let Some(e) = entry_at(ed, line) else {
        return vec![];
    };
    let suffix_len = usize::from(e.dir && e.link.is_none())
        + e.link.as_ref().map_or(0, |target| 4 + target.len());
    let Some(name) = text.len().checked_sub(e.name.len() + suffix_len) else {
        return vec![];
    };
    let end = name + e.name.len();
    // Embedded newlines can split an entry across buffer lines.
    if name < if d.details { 33 } else { 2 } || text.get(name..end) != Some(e.name.as_str()) {
        return vec![];
    }
    let mark = d.marks.get(&e.name).copied();
    let mut spans = vec![];
    if let Some(mark) = mark {
        let color = if mark == 'D' {
            Color::Red
        } else {
            Color::Yellow
        };
        spans.push((
            Style::default().fg(color).add_modifier(Modifier::BOLD),
            0..1,
        ));
    }
    let kind = if e.link.is_some() {
        Color::Blue
    } else if e.dir {
        Color::Cyan
    } else if e.mode & 0o111 != 0 {
        Color::Green
    } else if e.name.starts_with('.') {
        Color::DarkGray
    } else {
        Color::Gray
    };
    if d.details {
        for (i, c) in mode_str(e).chars().enumerate() {
            let color = match c {
                'd' | 'l' => kind,
                'r' => Color::Green,
                'w' => Color::Yellow,
                'x' => Color::Red,
                _ => Color::DarkGray,
            };
            spans.push((Style::default().fg(color), 2 + i..3 + i));
        }
        spans.push((Style::default().fg(Color::Yellow), 13..name - 14));
        spans.push((Style::default().fg(Color::Blue), name - 13..name - 1));
    }
    if mark == Some('D') {
        spans.push((Style::default().fg(Color::Red), name..text.len()));
    } else {
        // A leading dot is a hidden basename, not a file extension.
        let extension = (!e.dir && e.link.is_none())
            .then(|| e.name.rfind('.').filter(|&i| i > 0 && i + 1 < e.name.len()))
            .flatten();
        let split = extension.map_or(end, |i| name + i);
        spans.push((Style::default().fg(kind), name..split));
        if split < end {
            let compressed = matches!(
                e.name[split - name + 1..].to_ascii_lowercase().as_str(),
                "gz" | "bz2" | "xz" | "zst" | "zip" | "tgz" | "7z" | "tar"
            );
            spans.push((
                Style::default().fg(if compressed {
                    Color::Magenta
                } else {
                    Color::Green
                }),
                split..end,
            ));
        }
        if suffix_len > 0 {
            spans.push((
                Style::default().fg(if e.link.is_some() {
                    Color::DarkGray
                } else {
                    kind
                }),
                end..text.len(),
            ));
        }
    }
    spans
}

/// Put the listing in the buffer: not an edit (no undo, not modified).
fn redraw(ed: &mut Editor) {
    let Some(d) = ed.dired.as_ref() else { return };
    let mut lines = vec![format!("  {}:", crate::pick::browse::tilde(&d.dir))];
    lines.extend(d.entries.iter().map(|e| line_text(d, e)));
    let (line, n) = (ed.cur.line, ed.buf.len_lines());
    crate::vim::ops::splice_lines(ed, 0, n, &lines);
    ed.undo = crate::undo::Undo::default();
    ed.mark_saved();
    goto(ed, line);
}

fn goto(ed: &mut Editor, line: usize) {
    let line = line
        .min(ed.line_count() - 1)
        .max(HEADER.min(ed.line_count() - 1));
    let col = name_col(ed).min(ed.buf.line(line).len());
    ed.set_cursor(line, col);
}

fn entry_at(ed: &Editor, line: usize) -> Option<&Entry> {
    let d = ed.dired.as_ref()?;
    line.checked_sub(HEADER).and_then(|i| d.entries.get(i))
}

fn line_of(ed: &Editor, name: &str) -> Option<usize> {
    let d = ed.dired.as_ref()?;
    d.entries
        .iter()
        .position(|e| e.name == name)
        .map(|i| i + HEADER)
}

fn with(ed: &mut Editor, f: impl FnOnce(&mut Dired)) {
    if let Some(d) = ed.dired.as_mut() {
        f(d);
    }
}

// ---- keys ----

fn open_entry(ed: &mut Editor) {
    let Some(e) = entry_at(ed, ed.cur.line).cloned() else {
        return;
    };
    if e.name == ".." {
        return up(ed);
    }
    let path = ed
        .dired
        .as_ref()
        .map(|d| d.dir.join(&e.name))
        .unwrap_or_default();
    if e.dir {
        if let Err(err) = visit(ed, &path, None) {
            ed.set_err(err);
        }
    } else {
        ed.pending_effect = Some(ExEffect::Open {
            path,
            line: 0,
            col: 0,
            pattern: None,
        });
    }
}

fn up(ed: &mut Editor) {
    let Some(dir) = ed.dired.as_ref().map(|d| d.dir.clone()) else {
        return;
    };
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
        return;
    };
    let name = name.to_string_lossy().into_owned();
    if let Err(e) = visit(ed, parent, Some(&name)) {
        ed.set_err(e);
    }
}

/// Mark lines `lo..=hi` (`None` unmarks); in Normal mode, move down after.
fn mark(ed: &mut Editor, lo: usize, hi: usize, visual: bool, m: Option<char>) {
    let names: Vec<String> = (lo..=hi)
        .filter_map(|l| entry_at(ed, l))
        .filter(|e| e.name != "..")
        .map(|e| e.name.clone())
        .collect();
    with(ed, |d| {
        for n in names {
            match m {
                Some(c) => d.marks.insert(n, c),
                None => d.marks.remove(&n),
            };
        }
    });
    redraw(ed);
    if !visual {
        goto(ed, ed.cur.line + 1);
    }
}

/// `t`: marked become unmarked and unmarked marked (flags stay).
fn toggle(ed: &mut Editor) {
    with(ed, |d| {
        for e in d.entries.iter().filter(|e| e.name != "..") {
            match d.marks.get(&e.name) {
                Some('*') => {
                    d.marks.remove(&e.name);
                }
                None => {
                    d.marks.insert(e.name.clone(), '*');
                }
                _ => {}
            }
        }
    });
    redraw(ed);
}

fn paths_marked(ed: &Editor, c: char) -> Vec<PathBuf> {
    let Some(d) = ed.dired.as_ref() else {
        return vec![];
    };
    d.entries
        .iter()
        .filter(|e| d.marks.get(&e.name) == Some(&c))
        .map(|e| d.dir.join(&e.name))
        .collect()
}

/// What an operation acts on: the Visual range, else the `*` marked
/// entries, else the one under the cursor.
fn targets(ed: &Editor, lo: usize, hi: usize, visual: bool) -> Vec<PathBuf> {
    let Some(d) = ed.dired.as_ref() else {
        return vec![];
    };
    let marked = paths_marked(ed, '*');
    if !visual && !marked.is_empty() {
        return marked;
    }
    (lo..=hi)
        .filter_map(|l| entry_at(ed, l))
        .filter(|e| e.name != "..")
        .map(|e| d.dir.join(&e.name))
        .collect()
}

/// The marked entries, else the one under the cursor (magit-dired's
/// dired-get-marked-files).
pub fn selection(ed: &Editor) -> Vec<PathBuf> {
    targets(ed, ed.cur.line, ed.cur.line, false)
}
/// Only the `*` marked entries.
pub fn marked(ed: &Editor) -> Vec<PathBuf> {
    paths_marked(ed, '*')
}

fn what(ps: &[PathBuf]) -> String {
    match ps {
        [p] => p
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        _ => format!("{} files", ps.len()),
    }
}

fn ask(ed: &mut Editor, a: Ask, prompt: &str, text: &str) {
    with(ed, |d| d.ask = Some(a));
    ed.open_prompt(prompt, text);
}

fn ask_delete(ed: &mut Editor, ps: Vec<PathBuf>) {
    if ps.is_empty() {
        return;
    }
    let dirs = ps.iter().any(|p| p.is_dir() && !p.is_symlink());
    let p = format!(
        "Delete {}{}? (y/n) ",
        what(&ps),
        if dirs {
            " (directories recursively)"
        } else {
            ""
        }
    );
    ask(ed, Ask::Delete(ps), &p, "");
}

fn ask_paths(
    ed: &mut Editor,
    lo: usize,
    hi: usize,
    visual: bool,
    verb: &str,
    a: fn(Vec<PathBuf>) -> Ask,
) {
    let t = targets(ed, lo, hi, visual);
    if t.is_empty() {
        return;
    }
    // One file: start from its name; several: from this directory.
    let start = match &t[..] {
        [p] => p.file_name().map(|n| n.to_string_lossy().into_owned()),
        _ => None,
    }
    .unwrap_or_default();
    let p = format!("{verb} {} to: ", what(&t));
    ask(ed, a(t), &p, &start);
}

/// Report how an operation went and show the directory as it is now.
fn done(ed: &mut Editor, r: Result<String, String>) {
    refresh(ed);
    match r {
        Ok(m) => ed.set_msg(m),
        Err(e) => ed.set_err(e),
    }
}

fn revert(ed: &mut Editor) {
    with(ed, |d| d.editing = false);
    ed.readonly = true;
    refresh(ed);
}

// ---- file operations ----

/// A typed destination: `~/x`, absolute, or relative to the listing.
fn target(dir: &Path, text: &str) -> PathBuf {
    match text.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None => dir.join(text),
    }
}

fn remove(p: &Path) -> Result<(), String> {
    let r = if p.is_dir() && !p.is_symlink() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    };
    r.map_err(|e| format!("{}: {}", p.display(), crate::fileio::err_msg(&e)))
}

/// `mv`/`cp -R` the files to `dest` (a directory when there are several),
/// never over an existing file.
fn transfer(tool: &str, ps: &[PathBuf], dest: &Path) -> Result<(), String> {
    if ps.is_empty() {
        return Ok(());
    }
    let into = dest.is_dir();
    if ps.len() > 1 && !into {
        return Err(format!("{}: not a directory", dest.display()));
    }
    for p in ps {
        let to = if into {
            dest.join(p.file_name().unwrap_or_default())
        } else {
            dest.to_path_buf()
        };
        if to.symlink_metadata().is_ok() {
            return Err(format!("{} exists", to.display()));
        }
    }
    let mut c = std::process::Command::new(tool);
    if tool == "cp" {
        c.arg("-R");
    }
    let out = c
        .arg("--")
        .args(ps)
        .arg(dest)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{tool}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().next().unwrap_or("failed").to_string())
    }
}

fn touch(p: &Path) -> Result<(), String> {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .and_then(|f| f.set_modified(SystemTime::now()))
        .map_err(|e| format!("{}: {}", p.display(), crate::fileio::err_msg(&e)))
}

/// `cmd` run in `dir` on the files: in place of a lone `*`, else after it.
fn shell_line(dir: &Path, cmd: &str, ps: &[PathBuf]) -> String {
    let files: Vec<String> = ps
        .iter()
        .map(|p| crate::shell::quote(&p.file_name().unwrap_or_default().to_string_lossy()))
        .collect();
    let files = files.join(" ");
    let words: Vec<&str> = cmd.split(' ').collect();
    let cmd = if words.contains(&"*") {
        words
            .iter()
            .map(|w| if *w == "*" { files.as_str() } else { w })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        format!("{cmd} {files}")
    };
    format!(
        "cd {} && {cmd}",
        crate::shell::quote(&dir.to_string_lossy())
    )
}

/// wdired: the entries whose line now names something else, as (old, new).
fn renames(ed: &Editor, d: &Dired) -> Result<Vec<(String, String)>, String> {
    if ed.line_count() != HEADER + d.entries.len() {
        return Err("lines were added or removed: only names can change (gr discards)".into());
    }
    let col = name_col(ed);
    let mut plan = vec![];
    for (i, e) in d.entries.iter().enumerate() {
        let line = ed.buf.line(HEADER + i);
        let mut new = line.get(col..).unwrap_or("").to_string();
        if let Some(t) = &e.link
            && let Some(n) = new.strip_suffix(&format!(" -> {t}"))
        {
            new = n.to_string();
        }
        if e.dir && e.link.is_none() {
            new = new.strip_suffix('/').unwrap_or(&new).to_string();
        }
        if new.is_empty() {
            return Err(format!("line {}: empty name", HEADER + i + 1));
        }
        if new != e.name && e.name != ".." {
            plan.push((e.name.clone(), new));
        }
    }
    Ok(plan)
}

/// Rename in two steps (through temporary names) so swaps work, never over
/// a file that isn't itself being renamed away.
fn apply_renames(dir: &Path, plan: &[(String, String)]) -> Result<(), String> {
    for (_, new) in plan {
        let taken = dir.join(new).symlink_metadata().is_ok();
        if taken && !plan.iter().any(|(old, _)| old == new) {
            return Err(format!("{new} exists"));
        }
    }
    let tmp = |i: usize| dir.join(format!(".fred-rename-{}-{i}", std::process::id()));
    let err = |e: std::io::Error| crate::fileio::err_msg(&e);
    for (i, (old, _)) in plan.iter().enumerate() {
        fs::rename(dir.join(old), tmp(i)).map_err(err)?;
    }
    for (i, (_, new)) in plan.iter().enumerate() {
        fs::rename(tmp(i), dir.join(new)).map_err(err)?;
    }
    Ok(())
}

// ---- formatting ----

fn mode_str(e: &Entry) -> String {
    let kind = if e.link.is_some() {
        'l'
    } else if e.dir {
        'd'
    } else {
        '-'
    };
    let mut s = String::from(kind);
    for shift in [6, 3, 0] {
        let b = (e.mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// `ls -h` style: `512`, `4.0K`, `13M`, `1.8G`.
fn human(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["", "K", "M", "G", "T"] {
        if v < 1024.0 || unit == "T" {
            return match unit {
                "" => format!("{n}"),
                _ if v < 10.0 => format!("{v:.1}{unit}"),
                _ => format!("{v:.0}{unit}"),
            };
        }
        v /= 1024.0;
    }
    unreachable!()
}

/// `Oct  2 11:43` this half-year, `Oct  2  2024` before: 12 columns.
fn date(t: Option<SystemTime>) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let Some(secs) = t
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as libc::time_t)
    else {
        return " ".repeat(12);
    };
    // SAFETY: `tm` is plain data that localtime_r fills in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call.
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return " ".repeat(12);
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as libc::time_t);
    let month = MONTHS[tm.tm_mon.clamp(0, 11) as usize];
    if (now - secs).abs() < 182 * 24 * 3600 {
        format!(
            "{month} {:>2} {:02}:{:02}",
            tm.tm_mday, tm.tm_hour, tm.tm_min
        )
    } else {
        format!("{month} {:>2}  {}", tm.tm_mday, tm.tm_year + 1900)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::key::parse_keys;

    fn keys(ed: &mut Editor, s: &str) {
        for k in parse_keys(s) {
            ed.handle_key(k);
        }
    }

    fn setup() -> (tempfile::TempDir, Editor) {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        for f in ["a.txt", "b.txt", "c.log", ".hidden"] {
            fs::write(d.path().join(f), f).unwrap();
        }
        let mut ed = Editor::new(Buffer::from_text(""));
        visit(&mut ed, d.path(), None).unwrap();
        (d, ed)
    }

    fn names(ed: &Editor) -> Vec<String> {
        let d = ed.dired.as_ref().unwrap();
        d.entries.iter().map(|e| e.name.clone()).collect()
    }

    /// Put the cursor on `name`'s line.
    fn on(ed: &mut Editor, name: &str) {
        let l = line_of(ed, name).unwrap();
        goto(ed, l);
    }

    #[test]
    fn lists_and_navigates() {
        let (d, mut ed) = setup();
        assert_eq!(
            names(&ed),
            ["..", ".hidden", "a.txt", "b.txt", "c.log", "sub"]
        );
        assert!(ed.buf.line(0).ends_with(':'));
        assert!(ed.buf.line(6).ends_with(" sub/"));
        assert!(!ed.buf.modified && ed.readonly);
        // The cursor sits on names; j keeps it there.
        // The cursor starts on the first entry after `..`, on its name; j
        // keeps it on names.
        assert_eq!(&ed.buf.line(ed.cur.line)[ed.cur.byte..], ".hidden");
        keys(&mut ed, "j");
        assert_eq!(&ed.buf.line(ed.cur.line)[ed.cur.byte..], "a.txt");
        // Into a directory and back up, landing on it.
        on(&mut ed, "sub");
        keys(&mut ed, "<Enter>");
        assert!(ed.dired.as_ref().unwrap().dir.ends_with("sub"));
        keys(&mut ed, "-");
        assert_eq!(entry_at(&ed, ed.cur.line).unwrap().name, "sub");
        // A file opens.
        on(&mut ed, "a.txt");
        keys(&mut ed, "<Enter>");
        assert!(matches!(
            ed.pending_effect.take(),
            Some(ExEffect::Open { path, .. }) if path == fs::canonicalize(d.path()).unwrap().join("a.txt")
        ));
        // Vim's editing keys don't touch the listing.
        keys(&mut ed, "ox<Esc>p");
        assert!(!ed.buf.modified);
        // gh hides dotfiles, ( hides details, s sorts by time.
        keys(&mut ed, "gh");
        assert!(!names(&ed).contains(&".hidden".to_string()));
        keys(&mut ed, "(");
        assert_eq!(ed.buf.line(1), "  ../");
        keys(&mut ed, "s");
        assert!(ed.dired.as_ref().unwrap().by_time);
    }

    #[test]
    fn marks_and_deletes() {
        let (d, mut ed) = setup();
        on(&mut ed, "a.txt");
        keys(&mut ed, "mm");
        assert!(ed.buf.line(line_of(&ed, "a.txt").unwrap()).starts_with('*'));
        assert!(ed.buf.line(line_of(&ed, "b.txt").unwrap()).starts_with('*'));
        keys(&mut ed, "U");
        assert!(ed.dired.as_ref().unwrap().marks.is_empty());
        // Flag with d, delete the flagged with x after a yes.
        on(&mut ed, "a.txt");
        keys(&mut ed, "dd");
        keys(&mut ed, "x");
        assert!(matches!(ed.mode, Mode::Command(_)));
        keys(&mut ed, "n<Enter>");
        assert!(d.path().join("a.txt").exists(), "no: nothing deleted");
        keys(&mut ed, "xy<Enter>");
        assert!(!d.path().join("a.txt").exists() && !d.path().join("b.txt").exists());
        assert_eq!(names(&ed), ["..", ".hidden", "c.log", "sub"]);
        // D on a directory deletes it, recursively.
        fs::write(d.path().join("sub/x"), "x").unwrap();
        on(&mut ed, "sub");
        keys(&mut ed, "Dyes<Enter>");
        assert!(!d.path().join("sub").exists());
        // `..` is never a target.
        on(&mut ed, "..");
        keys(&mut ed, "m");
        assert!(ed.dired.as_ref().unwrap().marks.is_empty());
    }

    #[test]
    fn visual_range_and_regexp_marks() {
        let (_d, mut ed) = setup();
        on(&mut ed, "a.txt");
        keys(&mut ed, "Vjm");
        assert_eq!(ed.mode, Mode::Normal);
        let marks = &ed.dired.as_ref().unwrap().marks;
        assert!(marks.contains_key("a.txt") && marks.contains_key("b.txt") && marks.len() == 2);
        keys(&mut ed, "U%\\.log$<Enter>");
        let marks = &ed.dired.as_ref().unwrap().marks;
        assert_eq!(marks.keys().collect::<Vec<_>>(), ["c.log"]);
        keys(&mut ed, "t");
        let marks = &ed.dired.as_ref().unwrap().marks;
        assert!(!marks.contains_key("c.log") && marks.contains_key("a.txt"));
    }

    #[test]
    fn rename_copy_mkdir_chmod_touch() {
        let (d, mut ed) = setup();
        on(&mut ed, "a.txt");
        // R starts from the name.
        keys(&mut ed, "R");
        let Mode::Command(cl) = &ed.mode else {
            panic!()
        };
        assert_eq!(cl.text, "a.txt");
        keys(&mut ed, "<C-u>z.txt<Enter>");
        assert!(d.path().join("z.txt").exists() && !d.path().join("a.txt").exists());
        assert_eq!(entry_at(&ed, ed.cur.line).unwrap().name, "z.txt");
        // Never over an existing file.
        on(&mut ed, "z.txt");
        keys(&mut ed, "R<C-u>b.txt<Enter>");
        assert!(ed.msg.as_ref().unwrap().1, "{:?}", ed.msg);
        assert!(d.path().join("z.txt").exists());
        // Marked files copy (and move) into a directory.
        on(&mut ed, "b.txt");
        keys(&mut ed, "mm");
        keys(&mut ed, "Csub<Enter>");
        assert!(d.path().join("sub/b.txt").exists() && d.path().join("b.txt").exists());
        assert!(d.path().join("sub/c.log").exists());
        keys(&mut ed, "+new/deep<Enter>");
        assert!(d.path().join("new/deep").is_dir());
        on(&mut ed, "z.txt");
        keys(&mut ed, "U");
        keys(&mut ed, "M600<Enter>");
        let mode = fs::metadata(d.path().join("z.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        keys(&mut ed, "Mxyz<Enter>");
        assert!(ed.msg.as_ref().unwrap().1);
        // ! runs in the directory, on the files.
        on(&mut ed, "z.txt");
        keys(&mut ed, "!wc -c * | sort<Enter>");
        let Some(ExEffect::Shell(cmd)) = ed.pending_effect.take() else {
            panic!()
        };
        assert!(
            cmd.starts_with("cd ") && cmd.ends_with("&& wc -c z.txt | sort"),
            "{cmd}"
        );
    }

    #[test]
    fn wdired_renames_by_editing_names() {
        let (d, mut ed) = setup();
        keys(&mut ed, "i");
        assert!(ed.dired.as_ref().unwrap().editing);
        // Now vim edits: swap a.txt and b.txt, rename c.log.
        let la = line_of(&ed, "a.txt").unwrap();
        let col = name_col(&ed);
        let set = |ed: &mut Editor, l: usize, name: &str| {
            let old = ed.buf.line(l);
            crate::vim::ops::splice_lines(ed, l, 1, &[format!("{}{name}", &old[..col])]);
        };
        set(&mut ed, la, "b.txt");
        set(&mut ed, la + 1, "a.txt");
        set(&mut ed, la + 2, "d.log");
        save(&mut ed);
        assert!(!ed.dired.as_ref().unwrap().editing, "{:?}", ed.msg);
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "b.txt");
        assert_eq!(fs::read_to_string(d.path().join("b.txt")).unwrap(), "a.txt");
        assert!(d.path().join("d.log").exists() && !d.path().join("c.log").exists());
        // Adding a line is refused; gr discards.
        keys(&mut ed, "i");
        crate::vim::ops::splice_lines(&mut ed, 1, 0, &["oops".to_string()]);
        save(&mut ed);
        assert!(ed.dired.as_ref().unwrap().editing && ed.msg.as_ref().unwrap().1);
        keys(&mut ed, "<Esc>gr");
        assert!(!ed.dired.as_ref().unwrap().editing && !ed.buf.modified);
        assert_eq!(names(&ed).len(), 6);
    }
}
