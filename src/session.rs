//! One editing session: the editor plus its file, swap file and file effects.

use crate::buffer::{Buffer, LineEnding};
use crate::config::Config;
use crate::editor::{Editor, Mode};
use crate::ex::addr::Range;
use crate::ex::{BufCmd, ExEffect};
use crate::fileio::{self, FileStamp};
use crate::key::Key;
use crate::swap::{self, SwapInfo};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SWAP_IDLE: Duration = Duration::from_secs(1);
const SWAP_EDITS: usize = 200;

/// What our swap file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwapState {
    /// We have no swap file.
    None,
    /// A lock with no unsaved text: "a fred has this file open".
    Clean,
    /// Unsaved text as of this buffer version.
    Dirty(u64),
}

/// What to do about a file's existing swap file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapChoice {
    Recover,
    Delete,
    ReadOnly,
    Cancel,
}

/// `:e` waiting for the user to decide about the new file's swap file.
#[derive(Clone, Debug)]
pub struct PendingEdit {
    pub path: PathBuf,
    pub info: SwapInfo,
    pub then: Option<Goto>,
}

/// Where a picker result lands once its file is open.
#[derive(Clone, Debug)]
pub struct Goto {
    pub line: usize,
    pub col: usize,
    pub pattern: Option<String>,
}

/// A buffer not being shown: its editor and file state.
struct Parked {
    ed: Editor,
    stamp: Option<FileStamp>,
    swap_path: PathBuf,
    lossy: bool,
    no_swap: bool,
    swap_state: SwapState,
    /// When it was last left ([`Session::clock`]).
    used: u64,
}

pub struct Session {
    pub ed: Editor,
    pub stamp: Option<FileStamp>,
    pub swap_dir: PathBuf,
    pub swap_path: PathBuf,
    /// Summary of the last successful write of the buffer's own file.
    pub written: Option<String>,
    pub quit: bool,
    /// A different file was loaded (`:e`); the caller resets per-file state.
    pub reloaded: bool,
    /// Another fred owns this file's swap; never write it.
    pub no_swap: bool,
    /// `:e` into a file with a swap file: the caller asks what to do.
    pub pending_edit: Option<PendingEdit>,
    /// `:!cmd` waiting for the app to hand it the terminal.
    pub pending_shell: Option<String>,
    /// `:ai`: the lines and the prompt, for the app to ask Claude.
    pub pending_ai: Option<(crate::ex::addr::Range, String)>,
    lossy: bool,
    cfg: Config,
    seen_version: u64,
    last_change: Option<Instant>,
    swap_state: SwapState,
    swap_error_shown: bool,
    /// Every open buffer, numbered from 1. The one being edited lives in
    /// the fields above; its slot (`cur`) is None.
    bufs: Vec<Option<Parked>>,
    cur: usize,
    /// Counts buffer switches, to order buffers by last use.
    clock: u64,
}

fn make_editor(buf: Buffer, cfg: &Config) -> Editor {
    let mut ed = Editor::new(buf);
    ed.tabstop = cfg.tabstop;
    ed.autocomplete = cfg.autocomplete;
    ed.clipboard = cfg.clipboard;
    ed.win_height = cfg.height;
    ed
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(p),
    }
}

fn human_size(n: usize) -> String {
    match n {
        n if n < 1024 => format!("{n}B"),
        n if n < 1024 * 1024 => format!("{:.1}K", n as f64 / 1024.0),
        n => format!("{:.1}M", n as f64 / (1024.0 * 1024.0)),
    }
}

/// `"name" 12L, 340B written`
pub fn summary(name: &Path, data: &[u8]) -> String {
    let mut lines = data.iter().filter(|&&b| b == b'\n').count();
    if !data.is_empty() && !data.ends_with(b"\n") {
        lines += 1;
    }
    format!(
        "\"{}\" {lines}L, {} written",
        name.display(),
        human_size(data.len())
    )
}

struct Opened {
    ed: Editor,
    stamp: Option<FileStamp>,
    lossy: bool,
}

fn open_file(path: Option<&Path>, cfg: &Config) -> Result<Opened, String> {
    let Some(p) = path else {
        let buf = Buffer::from_text("");
        return Ok(Opened {
            ed: make_editor(buf, cfg),
            stamp: None,
            lossy: false,
        });
    };
    let l = fileio::load(p)?;
    let lossy = l.notice.as_deref() == Some("not valid UTF-8");
    let (lines, bytes) = (l.buf.len_lines(), l.buf.len_bytes());
    let mut ed = make_editor(l.buf, cfg);
    ed.path = Some(p.to_path_buf());
    ed.readonly = l.readonly;
    let name = p.display();
    match l.notice.as_deref() {
        Some("[new]") => ed.set_msg(format!("\"{name}\" [new]")),
        Some(n) => ed.set_msg(format!("\"{name}\" {n}")),
        None => ed.set_msg(format!("\"{name}\" {lines}L, {}", human_size(bytes))),
    }
    Ok(Opened {
        ed,
        stamp: l.stamp,
        lossy,
    })
}

/// A swap file at `swap` that the user should decide about: one held by a
/// live fred, or one with unsaved text. A clean lock or a copy of the file
/// left by a dead fred is removed instead.
fn leftover(swap: &Path, buf: &Buffer) -> Option<SwapInfo> {
    if !swap.exists() {
        return None;
    }
    let info = swap::read(swap).ok()?;
    if swap::is_mine(&info) {
        return None;
    }
    if !swap::owner_alive(&info) && (info.clean || info.text.as_bytes() == buf.to_bytes()) {
        swap::remove(swap);
        return None;
    }
    Some(info)
}

/// Remove the swap file at `swap` if it is ours.
fn release(swap: &Path) {
    if let Ok(info) = swap::read_head(swap)
        && swap::is_mine(&info)
    {
        swap::remove(swap);
    }
}

fn buf_name(ed: &Editor) -> String {
    ed.path
        .as_ref()
        .map_or_else(|| "[No Name]".into(), |p| p.display().to_string())
}

impl Session {
    /// Open `path` (or an unnamed buffer). Returns a leftover swap file, if any.
    pub fn open(
        path: Option<PathBuf>,
        cfg: &Config,
        swap_dir: &Path,
    ) -> Result<(Session, Option<SwapInfo>), String> {
        // `fred DIR`: an empty buffer, browsing DIR.
        let browse = path.clone().filter(|p| p.is_dir());
        let path = if browse.is_some() { None } else { path };
        let o = open_file(path.as_deref(), cfg)?;
        let swap_path = swap::swap_path_in(swap_dir, path.as_deref());
        let mut s = Session {
            seen_version: o.ed.buf.version,
            ed: o.ed,
            stamp: o.stamp,
            swap_dir: swap_dir.to_path_buf(),
            swap_path,
            written: None,
            quit: false,
            reloaded: false,
            no_swap: false,
            pending_edit: None,
            pending_shell: None,
            pending_ai: None,
            lossy: o.lossy,
            cfg: cfg.clone(),
            last_change: None,
            swap_state: SwapState::None,
            swap_error_shown: false,
            bufs: vec![None],
            cur: 0,
            clock: 0,
        };
        let dir = browse.clone().or_else(|| {
            s.ed.path
                .as_deref()
                .and_then(|p| std::path::absolute(p).ok())
                .and_then(|p| p.parent().map(Path::to_path_buf))
        });
        s.ed.project = std::sync::Arc::new(crate::pick::Project::new(dir, Some(s.recent_file())));
        s.arrived();
        if let Some(d) = &browse {
            crate::pick::browse(&mut s.ed, d);
        }
        let info = s.leftover_swap();
        Ok((s, info))
    }

    /// `~/.local/state/fred/recent`, beside the swap directory.
    fn recent_file(&self) -> PathBuf {
        self.swap_dir.with_file_name("recent")
    }

    /// A file was opened: put it first among recent files, go back to
    /// where the cursor was when it was last left, and fetch its staged
    /// version for the git marks.
    fn arrived(&mut self) {
        let Some(p) = self.ed.path.clone() else {
            return;
        };
        self.ed.git = crate::git::Gutter::load(&p);
        let rf = self.recent_file();
        if let Some((line, col)) = crate::pick::recent::position(&rf, &p) {
            self.ed.set_cursor(line, col);
            self.ed.clamp_cursor();
        }
        crate::pick::recent::record(&rf, &p, None);
    }

    /// Remember where the cursor is in this file, for next time.
    pub fn remember_place(&self) {
        if let Some(p) = &self.ed.path {
            let pos = (self.ed.cur.line, self.ed.cur.byte);
            crate::pick::recent::record(&self.recent_file(), p, Some(pos));
        }
    }

    fn leftover_swap(&mut self) -> Option<SwapInfo> {
        leftover(&self.swap_path, &self.ed.buf)
    }

    pub fn handle_key(&mut self, k: Key) {
        self.ed.handle_key(k);
        if let Some(eff) = self.ed.pending_effect.take() {
            self.perform(eff);
        }
    }

    pub fn perform(&mut self, eff: ExEffect) {
        match eff {
            ExEffect::None => {}
            ExEffect::Write {
                path,
                force,
                range,
                then_quit,
            } => {
                if self.write(path, force, range) && then_quit {
                    if self.ed.buf.modified {
                        // Wrote a copy; the buffer itself is still unsaved.
                        self.ed
                            .set_err("unsaved changes (q! to discard, wq to save)");
                    } else {
                        self.quit_if_all_saved();
                    }
                }
            }
            ExEffect::WriteIfModifiedQuit => {
                if !self.ed.buf.modified || self.write(None, false, None) {
                    self.quit_if_all_saved();
                }
            }
            ExEffect::Quit { force } => {
                if force {
                    self.quit = true;
                } else if self.ed.buf.modified {
                    self.ed
                        .set_err("unsaved changes (q! to discard, wq to save)");
                } else {
                    self.quit_if_all_saved();
                }
            }
            ExEffect::Buffer { cmd, arg, force } => self.buffer(cmd, &arg, force),
            ExEffect::Edit { path, force } => self.edit(path.as_deref(), force),
            ExEffect::Shell(cmd) => self.pending_shell = Some(cmd),
            ExEffect::Ai { range, prompt } => self.pending_ai = Some((range, prompt)),
            ExEffect::Pwd => match std::env::current_dir() {
                Ok(d) => self.ed.set_msg(d.display().to_string()),
                Err(e) => self.ed.set_err(fileio::err_msg(&e)),
            },
            ExEffect::Cd(dir) => self.cd(dir.as_deref()),
            ExEffect::Open {
                path,
                line,
                col,
                pattern,
            } => {
                // A file from Space p/r/j has no target: the remembered
                // position applies. A grep match does.
                let then = (pattern.is_some() || line > 0 || col > 0).then_some(Goto {
                    line,
                    col,
                    pattern,
                });
                self.open_pick(path, then)
            }
        }
    }

    fn write(&mut self, path: Option<String>, force: bool, range: Option<Range>) -> bool {
        if self.lossy {
            self.ed
                .set_err("file is not valid UTF-8; writing it would change it");
            return false;
        }
        let target = path.as_deref().map(expand_tilde);
        match (target, range) {
            (None, Some(_)) => {
                self.ed.set_err("partial write needs a file name");
                false
            }
            (None, None) => match self.ed.path.clone() {
                Some(p) => self.write_own(&p, force),
                None => {
                    self.ed.set_err("no file name (use :w name)");
                    false
                }
            },
            (Some(t), None) if self.ed.path.is_none() => {
                self.ed.path = Some(t.clone());
                self.stamp = None;
                let ok = self.write_own(&t, force);
                if ok {
                    // The lock moves from the unnamed swap to the file's.
                    let locked = self.swap_state != SwapState::None;
                    self.release_swap();
                    self.swap_path = swap::swap_path_in(&self.swap_dir, Some(&t));
                    if locked {
                        self.lock();
                    }
                } else {
                    self.ed.path = None;
                }
                ok
            }
            (Some(t), None) if self.is_own_file(&t) => {
                let own = self.ed.path.clone().unwrap_or(t);
                self.write_own(&own, force)
            }
            (Some(t), range) => {
                let data = match range {
                    Some(r) => self.range_bytes(r),
                    None => self.ed.buf.to_bytes(),
                };
                match fileio::write(&t, &data, None, force) {
                    Ok(_) => {
                        self.ed.set_msg(summary(&t, &data));
                        true
                    }
                    Err(e) => {
                        self.ed.set_err(e);
                        false
                    }
                }
            }
        }
    }

    /// `t` names the buffer's own file, perhaps by another path.
    fn is_own_file(&self, t: &Path) -> bool {
        self.ed
            .path
            .as_deref()
            .is_some_and(|own| swap::canonical(own) == swap::canonical(t))
    }

    fn range_bytes(&self, r: Range) -> Vec<u8> {
        let nl = self.ed.buf.line_ending.as_str();
        let mut s = String::new();
        for l in r.start..=r.end {
            s.push_str(&self.ed.buf.line(l));
            s.push_str(nl);
        }
        s.into_bytes()
    }

    fn write_own(&mut self, p: &Path, force: bool) -> bool {
        if self.ed.readonly && !force {
            self.ed.set_err("read-only (w! to override)");
            return false;
        }
        let data = self.ed.buf.to_bytes();
        match fileio::write(p, &data, self.stamp.as_ref(), force) {
            Ok(st) => {
                self.stamp = Some(st);
                self.ed.mark_saved();
                let msg = summary(p, &data);
                self.ed.set_msg(msg.clone());
                self.written = Some(msg);
                if self.swap_state != SwapState::None {
                    self.write_lock();
                }
                true
            }
            Err(e) => {
                self.ed.set_err(e);
                false
            }
        }
    }

    /// `:e`: reload this file, or open (or go to) another one; this one
    /// stays open as a buffer (unless `:e!` discards its changes).
    fn edit(&mut self, path: Option<&str>, force: bool) {
        let p = match (path, &self.ed.path) {
            (Some(path), _) => expand_tilde(path),
            (None, Some(own)) => own.clone(),
            (None, None) => {
                self.ed.set_err("no file name");
                return;
            }
        };
        if self.ed.buf.modified && !force && self.is_own_file(&p) {
            self.ed.set_err("unsaved changes (e! to discard)");
            return;
        }
        let before = self.cur;
        self.edit_path(p, None);
        // `:e! other` discards this buffer's changes, as in vim: close it.
        if force && self.cur != before && self.ed_at(before).buf.modified {
            self.buffer(BufCmd::Delete, &(before + 1).to_string(), true);
        }
    }

    /// `:cd [dir]`: change directory (home without one). Relative paths,
    /// `Space p` and `Space g` follow.
    fn cd(&mut self, dir: Option<&str>) {
        let to = dir.map_or_else(|| expand_tilde("~/"), expand_tilde);
        // Pin the files' paths first: `src/x.rs` must not come to mean a
        // file in the new directory.
        let files: Vec<Option<PathBuf>> = self
            .editors_mut()
            .map(|ed| ed.path.as_deref().and_then(|p| std::path::absolute(p).ok()))
            .collect();
        if let Err(e) = std::env::set_current_dir(&to) {
            self.ed
                .set_err(format!("{}: {}", to.display(), fileio::err_msg(&e)));
            return;
        }
        let cwd = std::env::current_dir().unwrap_or(to);
        for (ed, f) in self.editors_mut().zip(files) {
            if let Some(f) = f {
                ed.path = Some(f.strip_prefix(&cwd).map(Path::to_path_buf).unwrap_or(f));
            }
        }
        self.ed.project = std::sync::Arc::new(crate::pick::Project::new(
            Some(cwd.clone()),
            Some(self.recent_file()),
        ));
        self.ed.set_msg(cwd.display().to_string());
    }

    /// Open a picker result (this file stays open as a buffer).
    fn open_pick(&mut self, path: PathBuf, then: Option<Goto>) {
        self.ed.project.grep.cancel();
        if self.is_own_file(&path) {
            self.ed.mode = Mode::Normal;
            self.go(then);
            return;
        }
        // Shown on the status line: relative to where fred runs, if inside it.
        let path = std::env::current_dir()
            .ok()
            .and_then(|c| path.strip_prefix(c).ok().map(Path::to_path_buf))
            .unwrap_or(path);
        self.edit_path(path, then);
        // Still here: the open failed or waits on a swap question.
        if matches!(self.ed.mode, Mode::Pick(_)) {
            self.ed.mode = Mode::Normal;
        }
    }

    fn edit_path(&mut self, p: PathBuf, then: Option<Goto>) {
        if p.is_dir() {
            crate::pick::browse(&mut self.ed, &p);
            return;
        }
        if let Some(i) = self.find(&p) {
            self.show(i);
            self.go(then);
            return;
        }
        let o = match open_file(Some(&p), &self.cfg) {
            Ok(o) => o,
            Err(e) => {
                self.ed.set_err(format!("{}: {e}", p.display()));
                return;
            }
        };
        let new_swap = swap::swap_path_in(&self.swap_dir, Some(&p));
        if new_swap != self.swap_path
            && let Some(info) = leftover(&new_swap, &o.ed.buf)
        {
            self.pending_edit = Some(PendingEdit {
                path: p,
                info,
                then,
            });
            return;
        }
        self.switch_to(o, new_swap);
        self.lock();
        self.go(then);
    }

    fn go(&mut self, then: Option<Goto>) {
        let Some(g) = then else { return };
        self.ed.set_cursor(g.line, g.col);
        if let Some(p) = g.pattern.filter(|p| !p.is_empty()) {
            self.ed.last_pat = Some(p);
            self.ed.last_search_fwd = true;
        }
    }

    /// Make `o` the buffer being edited, with its swap file at `new_swap`:
    /// a new buffer, or in place of this one when reloading it (or when
    /// this one is an empty unnamed one).
    fn switch_to(&mut self, o: Opened, new_swap: PathBuf) {
        let blank = self.ed.path.is_none() && !self.ed.buf.modified && self.ed.buf.len_bytes() == 0;
        if new_swap == self.swap_path || blank {
            self.replace(o, new_swap);
            return;
        }
        self.bufs.push(Some(Parked {
            ed: o.ed,
            stamp: o.stamp,
            swap_path: new_swap,
            lossy: o.lossy,
            no_swap: false,
            swap_state: SwapState::None,
            used: 0,
        }));
        self.show(self.bufs.len() - 1);
        self.arrived();
    }

    /// Put `o` in place of the buffer being edited.
    fn replace(&mut self, o: Opened, new_swap: PathBuf) {
        if new_swap != self.swap_path {
            self.release_swap();
        }
        self.remember_place();
        let mut ed = o.ed;
        ed.inherit(&mut self.ed);
        self.ed = ed;
        self.arrived();
        self.stamp = o.stamp;
        self.lossy = o.lossy;
        self.no_swap = false;
        self.seen_version = self.ed.buf.version;
        self.last_change = None;
        self.swap_path = new_swap;
        self.reloaded = true;
    }

    /// Finish an `:e` that was waiting on [`SwapChoice`].
    pub fn resolve_edit(&mut self, choice: SwapChoice) {
        let Some(PendingEdit { path, info, then }) = self.pending_edit.take() else {
            return;
        };
        if choice == SwapChoice::Cancel {
            self.ed.set_msg(format!("still editing {}", self.name()));
            return;
        }
        let o = match open_file(Some(&path), &self.cfg) {
            Ok(o) => o,
            Err(e) => {
                self.ed.set_err(format!("{}: {e}", path.display()));
                return;
            }
        };
        let new_swap = swap::swap_path_in(&self.swap_dir, Some(&path));
        self.switch_to(o, new_swap);
        self.go(then);
        match choice {
            SwapChoice::Recover => {
                self.recover(info);
                self.lock();
            }
            SwapChoice::Delete => {
                swap::remove(&self.swap_path);
                self.lock();
            }
            _ => {
                self.ed.readonly = true;
                self.no_swap = true;
                self.ed
                    .set_msg(format!("\"{}\" opened read-only", path.display()));
            }
        }
    }

    fn name(&self) -> String {
        buf_name(&self.ed)
    }

    /// Show buffer `i`, leaving the current one open (and its unsaved
    /// text in its swap file).
    fn show(&mut self, i: usize) {
        if i == self.cur {
            // Picked from the buffer list: still close it.
            self.ed.mode = Mode::Normal;
            return;
        }
        self.remember_place();
        self.write_swap();
        let mut p = self.bufs[i].take().expect("a parked buffer");
        p.ed.inherit(&mut self.ed);
        self.ed.mode = Mode::Normal;
        std::mem::swap(&mut self.ed, &mut p.ed);
        std::mem::swap(&mut self.stamp, &mut p.stamp);
        std::mem::swap(&mut self.swap_path, &mut p.swap_path);
        std::mem::swap(&mut self.lossy, &mut p.lossy);
        std::mem::swap(&mut self.no_swap, &mut p.no_swap);
        std::mem::swap(&mut self.swap_state, &mut p.swap_state);
        self.clock += 1;
        p.used = self.clock;
        self.bufs[self.cur] = Some(p);
        self.cur = i;
        self.seen_version = self.ed.buf.version;
        self.last_change = None;
        self.reloaded = true;
    }

    /// The other buffer open on `path`.
    fn find(&self, path: &Path) -> Option<usize> {
        let want = swap::canonical(path);
        self.bufs.iter().position(|b| {
            b.as_ref()
                .and_then(|b| b.ed.path.as_deref())
                .is_some_and(|p| swap::canonical(p) == want)
        })
    }

    fn ed_at(&self, i: usize) -> &Editor {
        self.bufs[i].as_ref().map_or(&self.ed, |b| &b.ed)
    }

    fn editors_mut(&mut self) -> impl Iterator<Item = &mut Editor> {
        let parked = self.bufs.iter_mut().flatten().map(|b| &mut b.ed);
        std::iter::once(&mut self.ed).chain(parked)
    }

    /// Quit, unless another buffer has unsaved changes.
    fn quit_if_all_saved(&mut self) {
        let unsaved = (0..self.bufs.len()).find(|&i| self.ed_at(i).buf.modified);
        match unsaved {
            Some(i) => self.ed.set_err(format!(
                "buffer {} ({}) has unsaved changes (:b{0} to see it, q! to discard)",
                i + 1,
                buf_name(self.ed_at(i))
            )),
            None => self.quit = true,
        }
    }

    /// A buffer by `:b` argument: a number, `#` (the one used before this
    /// one), its file name or part of its path, or nothing (this one).
    fn buffer_arg(&self, arg: &str) -> Result<usize, String> {
        if arg.is_empty() {
            return Ok(self.cur);
        }
        if arg == "#" {
            return self
                .bufs
                .iter()
                .enumerate()
                .filter_map(|(i, b)| Some((b.as_ref()?.used, i)))
                .max()
                .map(|(_, i)| i)
                .ok_or_else(|| "no other buffer".into());
        }
        if let Ok(n) = arg.parse::<usize>() {
            return match n {
                1.. if n <= self.bufs.len() => Ok(n - 1),
                _ => Err(format!("no buffer {n}")),
            };
        }
        let mut hits: Vec<usize> = (0..self.bufs.len())
            .filter(|&i| buf_name(self.ed_at(i)).contains(arg))
            .collect();
        // `:b main.rs` means main.rs, not also src/domain.rs.
        let exact = |i: &usize| {
            let p = self.ed_at(*i).path.as_deref();
            p.and_then(Path::file_name).is_some_and(|n| n == arg)
        };
        if hits.len() > 1 && hits.iter().any(exact) {
            hits.retain(exact);
        }
        match hits[..] {
            [i] => Ok(i),
            [] => Err(format!("no buffer matches {arg}")),
            _ => Err(format!("more than one buffer matches {arg}")),
        }
    }

    fn buffer(&mut self, cmd: BufCmd, arg: &str, force: bool) {
        let n = self.bufs.len();
        let target = match cmd {
            BufCmd::Go | BufCmd::Delete => match self.buffer_arg(arg) {
                Ok(i) => i,
                Err(e) => return self.ed.set_err(e),
            },
            BufCmd::Next => (self.cur + 1) % n,
            BufCmd::Prev => (self.cur + n - 1) % n,
            BufCmd::List => return self.list_buffers(),
            BufCmd::Search => return self.search_buffers(),
        };
        if cmd != BufCmd::Delete {
            return self.show(target);
        }
        if self.ed_at(target).buf.modified && !force {
            return self.ed.set_err(format!(
                "buffer {} has unsaved changes (bd! to discard)",
                target + 1
            ));
        }
        if n == 1 {
            // The last buffer: an empty one takes its place.
            let o = open_file(None, &self.cfg).expect("an empty buffer");
            self.replace(o, swap::swap_path_in(&self.swap_dir, None));
            self.lock();
            return;
        }
        if target == self.cur {
            let to = self.buffer_arg("#").unwrap_or((target + 1) % n);
            self.show(to);
        }
        if let Some(b) = self.bufs.remove(target)
            && !b.no_swap
        {
            release(&b.swap_path);
        }
        if target < self.cur {
            self.cur -= 1;
        }
    }

    /// `Space B`: search the lines of every buffer, this one first, then
    /// the others by last use (unnamed ones but this one left out).
    fn search_buffers(&mut self) {
        let abs = |ed: &Editor| ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
        let mut parked: Vec<&Parked> = self
            .bufs
            .iter()
            .flatten()
            .filter(|b| b.ed.path.is_some())
            .collect();
        parked.sort_by_key(|b| std::cmp::Reverse(b.used));
        let list = std::iter::once(&self.ed)
            .chain(parked.into_iter().map(|b| &b.ed))
            .map(|ed| {
                (
                    buf_name(ed),
                    abs(ed).unwrap_or_default(),
                    ed.buf.rope().clone(),
                )
            })
            .collect();
        crate::pick::all_lines(&mut self.ed, list);
    }

    /// `:ls`: a picker of the buffers, most recently used first.
    fn list_buffers(&mut self) {
        let mut list: Vec<(u64, String, PathBuf, usize)> = (0..self.bufs.len())
            .map(|i| {
                let ed = self.ed_at(i);
                let used = self.bufs[i].as_ref().map_or(0, |b| b.used);
                let plus = if ed.buf.modified { " [+]" } else { "" };
                let path = ed.path.clone().unwrap_or_default();
                let path = std::path::absolute(&path).unwrap_or(path);
                (
                    used,
                    format!("{} {}{plus}", i + 1, buf_name(ed)),
                    path,
                    i + 1,
                )
            })
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.0));
        let list = list.into_iter().map(|(_, t, p, n)| (t, p, n)).collect();
        crate::pick::buffers(&mut self.ed, list);
    }

    /// Replace the buffer with a swap file's text.
    pub fn recover(&mut self, info: SwapInfo) {
        let mut ed = make_editor(Buffer::from_text(&info.text), &self.cfg);
        ed.inherit(&mut self.ed);
        ed.path = self.ed.path.clone();
        ed.readonly = self.ed.readonly;
        ed.saved_state = u64::MAX;
        ed.buf.modified = true;
        ed.set_msg("recovered unsaved changes; :w to save them");
        self.ed = ed;
        self.seen_version = self.ed.buf.version;
    }

    /// Delete a leftover swap file the user chose not to recover.
    pub fn discard_swap(&mut self) {
        swap::remove(&self.swap_path);
    }

    /// Take this file's swap file for the session: a clean lock, or the
    /// unsaved text right away (after recovering it).
    pub fn lock(&mut self) {
        if self.ed.buf.modified {
            self.write_swap();
        } else {
            self.write_lock();
        }
    }

    /// Keep the swap file up to date: write unsaved text once the user
    /// pauses (or after many edits), and go back to a clean lock on save.
    pub fn maybe_swap(&mut self, now: Instant) {
        let v = self.ed.buf.version;
        if v != self.seen_version {
            self.seen_version = v;
            self.last_change = Some(now);
        }
        if self.no_swap {
            return;
        }
        if !self.ed.buf.modified {
            if matches!(self.swap_state, SwapState::Dirty(_)) {
                self.write_lock();
            }
            return;
        }
        if self.swap_state == SwapState::Dirty(v) {
            return;
        }
        let idle = self
            .last_change
            .is_some_and(|t| now.duration_since(t) >= SWAP_IDLE);
        if idle || self.ed.buf.edits_since_swap >= SWAP_EDITS || self.last_change.is_none() {
            self.write_swap();
        }
    }

    /// May we write this file's swap? Not if another live fred owns it.
    fn may_write_swap(&mut self) -> bool {
        if self.no_swap {
            return false;
        }
        match swap::read_head(&self.swap_path) {
            Ok(info) if !swap::is_mine(&info) && swap::owner_alive(&info) => {
                self.no_swap = true;
                self.ed.set_err(format!(
                    "fred (pid {}) has this file open; not saving swap files",
                    info.pid
                ));
                false
            }
            _ => true,
        }
    }

    /// Write the swap now if there is anything unsaved (crash, signal).
    pub fn write_swap(&mut self) {
        if !self.ed.buf.modified || !self.may_write_swap() {
            return;
        }
        let text = String::from_utf8_lossy(&self.ed.buf.to_bytes()).into_owned();
        let r = swap::write(&self.swap_path, self.ed.path.as_deref(), &text);
        if self.swap_written(r) {
            self.swap_state = SwapState::Dirty(self.ed.buf.version);
            self.ed.buf.edits_since_swap = 0;
        }
    }

    /// Write a clean lock (no unsaved text).
    fn write_lock(&mut self) {
        if !self.may_write_swap() {
            return;
        }
        let r = swap::write_clean(&self.swap_path, self.ed.path.as_deref());
        if self.swap_written(r) {
            self.swap_state = SwapState::Clean;
        }
    }

    fn swap_written(&mut self, r: Result<(), String>) -> bool {
        match r {
            Ok(()) => true,
            Err(e) => {
                if !self.swap_error_shown {
                    self.swap_error_shown = true;
                    self.ed.set_err(e);
                }
                false
            }
        }
    }

    /// Remove our swap file (never one another fred wrote).
    fn release_swap(&mut self) {
        release(&self.swap_path);
        self.swap_state = SwapState::None;
    }

    /// Normal exit: the swap files are no longer needed.
    pub fn cleanup(&mut self) {
        if !self.no_swap {
            self.release_swap();
        }
        for b in self.bufs.iter().flatten().filter(|b| !b.no_swap) {
            release(&b.swap_path);
        }
    }

    /// `+LINE` on the command line.
    pub fn goto_line(&mut self, line: Option<usize>) {
        let n = self.ed.line_count();
        let l = line.map_or(n - 1, |l| l.clamp(1, n) - 1);
        let b = self.ed.first_nonblank(l);
        self.ed.set_cursor(l, b);
    }

    pub fn line_ending_name(&self) -> &'static str {
        match self.ed.buf.line_ending {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::key::parse_keys;
    use std::fs;
    use std::time::{Duration, Instant};

    struct T {
        dir: tempfile::TempDir,
        s: Session,
    }

    impl T {
        fn open(name: Option<&str>, content: Option<&str>) -> T {
            let dir = tempfile::tempdir().unwrap();
            let path = name.map(|n| dir.path().join(n));
            if let (Some(p), Some(c)) = (&path, content) {
                fs::write(p, c).unwrap();
            }
            let (s, _) = Session::open(path, &Config::default(), &dir.path().join("swap")).unwrap();
            T { dir, s }
        }
        fn keys(&mut self, k: &str) {
            for key in parse_keys(k) {
                self.s.handle_key(key);
            }
        }
        fn file(&self, name: &str) -> String {
            fs::read_to_string(self.dir.path().join(name)).unwrap()
        }
        fn msg(&self) -> String {
            self.s.ed.msg.clone().map(|m| m.0).unwrap_or_default()
        }
    }

    #[test]
    fn new_file_notice_and_write() {
        let mut t = T::open(Some("f.txt"), None);
        assert!(t.msg().contains("[new]"), "{}", t.msg());
        t.keys("ihi<Esc>:w<Enter>");
        assert_eq!(t.file("f.txt"), "hi\n");
        assert!(t.msg().ends_with("1L, 3B written"), "{}", t.msg());
        assert!(!t.s.ed.buf.modified);
        assert!(t.s.written.is_some());
    }

    #[test]
    fn write_detects_external_change() {
        let mut t = T::open(Some("f"), Some("a\n"));
        fs::write(t.dir.path().join("f"), "theirs\n").unwrap();
        t.keys("x:w<Enter>");
        assert!(t.msg().contains("changed on disk"), "{}", t.msg());
        assert_eq!(t.file("f"), "theirs\n");
        t.keys(":w!<Enter>");
        assert_eq!(t.file("f"), "\n");
    }

    #[test]
    fn quit_rules() {
        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys("x:q<Enter>");
        assert!(!t.s.quit);
        assert!(t.msg().contains("unsaved changes"));
        t.keys(":q!<Enter>");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "abc\n");

        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys("x:wq<Enter>");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "bc\n");

        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys(":x<Enter>");
        assert!(t.s.quit);
        assert!(
            t.s.written.is_none(),
            ":x on an unmodified buffer doesn't write"
        );
    }

    #[test]
    fn wq_to_another_file_keeps_unsaved_work() {
        let mut t = T::open(Some("f"), Some("one\ntwo\n"));
        let part = t.dir.path().join("part");
        let other = t.dir.path().join("other");
        t.keys(&format!("GoNEW WORK<Esc>:1,2wq {}<Enter>", part.display()));
        assert!(!t.s.quit, "the buffer still has unsaved changes");
        assert!(t.msg().contains("unsaved changes"), "{}", t.msg());
        assert_eq!(t.file("part"), "one\ntwo\n");
        t.keys(&format!(":wq {}<Enter>", other.display()));
        assert!(!t.s.quit);
        assert_eq!(t.file("other"), "one\ntwo\nNEW WORK\n");
        t.keys(":wq<Enter>");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "one\ntwo\nNEW WORK\n");
    }

    #[test]
    fn write_copy_and_partial() {
        let mut t = T::open(Some("f"), Some("one\ntwo\nthree\n"));
        let copy = t.dir.path().join("copy");
        let part = t.dir.path().join("part");
        t.keys(&format!("x:w {}<Enter>", copy.display()));
        assert_eq!(t.file("copy"), "ne\ntwo\nthree\n");
        assert!(
            t.s.ed.buf.modified,
            "writing a copy doesn't save the buffer"
        );
        t.keys(&format!(":w {}<Enter>", copy.display()));
        assert!(t.msg().contains("file exists"), "{}", t.msg());
        t.keys(&format!(":2,3w {}<Enter>", part.display()));
        assert_eq!(t.file("part"), "two\nthree\n");
        t.keys(":2,3w<Enter>");
        assert!(t.msg().contains("file name"), "{}", t.msg());
    }

    #[test]
    fn unnamed_buffer() {
        let mut t = T::open(None, None);
        t.keys("ihello<Esc>:w<Enter>");
        assert!(t.msg().contains("no file name"), "{}", t.msg());
        let p = t.dir.path().join("named.txt");
        t.keys(&format!(":w {}<Enter>", p.display()));
        assert_eq!(fs::read_to_string(&p).unwrap(), "hello\n");
        assert_eq!(t.s.ed.path.as_deref(), Some(p.as_path()));
        assert!(!t.s.ed.buf.modified);
    }

    #[test]
    fn edit_bang_reloads_the_current_file() {
        let mut t = T::open(Some("f"), Some("one\n"));
        t.keys("x:e<Enter>");
        assert!(t.msg().contains("unsaved changes"), "{}", t.msg());
        t.keys(":e!<Enter>");
        assert_eq!(t.s.ed.buf.text(), "one");
        assert!(!t.s.ed.buf.modified);
        // The usual answer to "file changed on disk": reload it.
        fs::write(t.dir.path().join("f"), "theirs\n").unwrap();
        t.keys(":e<Enter>");
        assert_eq!(t.s.ed.buf.text(), "theirs");
        let mut u = T::open(None, None);
        u.keys(":e!<Enter>");
        assert!(u.msg().contains("no file name"), "{}", u.msg());
    }

    #[test]
    fn zz_and_zq() {
        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys("xZZ");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "bc\n");
        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys("xZQ");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "abc\n");
    }

    #[test]
    fn writing_the_same_file_by_another_name_saves_it() {
        let mut t = T::open(Some("f"), Some("abc\n"));
        let same = t.dir.path().join(".").join("f");
        t.keys(&format!("x:w {}<Enter>", same.display()));
        assert!(!t.s.ed.buf.modified, "{}", t.msg());
        assert_eq!(t.file("f"), "bc\n");
    }

    #[test]
    fn edit_other_file_keeps_this_one_as_a_buffer() {
        let mut t = T::open(Some("a"), Some("aaa\n"));
        fs::write(t.dir.path().join("b"), "bbb\n").unwrap();
        let b = t.dir.path().join("b");
        t.keys(&format!("x:e {}<Enter>", b.display()));
        assert_eq!(t.s.ed.buf.text(), "bbb");
        assert!(t.s.reloaded);
        // a's unsaved text is safe in its swap file while it's hidden.
        let a_swap = swap::swap_path_in(&t.dir.path().join("swap"), Some(&t.dir.path().join("a")));
        assert_eq!(swap::read(&a_swap).unwrap().text, "aa\n");
        // Quitting asks about it; Ctrl-^ goes back to it, unsaved text and all.
        t.keys(":q<Enter>");
        assert!(!t.s.quit);
        assert!(t.msg().contains("buffer 1"), "{}", t.msg());
        t.keys("<C-^>");
        assert_eq!(t.s.ed.buf.text(), "aa");
        assert!(t.s.ed.buf.modified);
        // :e of an open file goes to that buffer (no reload).
        t.keys(&format!("x:e {}<Enter>", b.display()));
        assert_eq!(t.s.ed.buf.text(), "bbb");
        t.keys(":b1<Enter>");
        assert_eq!(t.s.ed.buf.text(), "a");
    }

    #[test]
    fn buffer_commands() {
        let mut t = T::open(Some("a"), Some("aaa\n"));
        for n in ["b", "c"] {
            fs::write(t.dir.path().join(n), format!("{n}\n")).unwrap();
            t.keys(&format!(":e {}<Enter>", t.dir.path().join(n).display()));
        }
        let text = |t: &T| t.s.ed.buf.text();
        assert_eq!(text(&t), "c");
        t.keys(":bn<Enter>");
        assert_eq!(text(&t), "aaa");
        t.keys(":bp<Enter>:bp<Enter>");
        assert_eq!(text(&t), "b");
        t.keys(":b c<Enter>");
        assert_eq!(text(&t), "c");
        t.keys(":b#<Enter>");
        assert_eq!(text(&t), "b");
        t.keys(":b9<Enter>");
        assert!(t.msg().contains("no buffer 9"), "{}", t.msg());
        // Deleting this buffer goes to the one used before it.
        t.keys("x:bd<Enter>");
        assert!(t.msg().contains("unsaved"), "{}", t.msg());
        t.keys(":bd!<Enter>");
        assert_eq!(text(&t), "c");
        t.keys(":bn<Enter>");
        assert_eq!(text(&t), "aaa");
        // :ls lists them, the one used last first; Enter goes there.
        t.keys(":ls<Enter>");
        let Mode::Pick(p) = &t.s.ed.mode else {
            panic!("no picker")
        };
        assert!(p.rows[0].text.starts_with("2 ") && p.rows[0].text.ends_with("/c"));
        t.keys("<Enter>");
        assert_eq!((text(&t), &t.s.ed.mode), ("c".into(), &Mode::Normal));
        // Deleting the last buffer leaves an empty one.
        t.keys(":bd<Enter>:bd<Enter>");
        assert_eq!((text(&t), t.s.ed.path.clone()), ("".into(), None));
        t.keys(":q<Enter>");
        assert!(t.s.quit);
    }

    #[test]
    fn invalid_utf8_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bin");
        fs::write(&p, [b'a', 0xff, b'\n']).unwrap();
        let (mut s, _) = Session::open(
            Some(p.clone()),
            &Config::default(),
            &dir.path().join("swap"),
        )
        .unwrap();
        assert!(s.ed.readonly);
        for k in parse_keys(":w!<Enter>") {
            s.handle_key(k);
        }
        assert_eq!(fs::read(&p).unwrap(), [b'a', 0xff, b'\n']);
        assert!(s.ed.msg.unwrap().1);
    }

    #[test]
    fn swap_is_a_lock_that_holds_unsaved_text() {
        let mut t = T::open(Some("f"), Some("a\n"));
        let sp = t.s.swap_path.clone();
        t.s.lock();
        let info = crate::swap::read(&sp).unwrap();
        assert!(info.clean, "the lock exists from the start, with no text");
        let now = Instant::now();
        t.keys("ixyz<Esc>");
        t.s.maybe_swap(now);
        assert!(crate::swap::read(&sp).unwrap().clean, "not yet idle");
        t.s.maybe_swap(now + Duration::from_millis(1100));
        let info = crate::swap::read(&sp).unwrap();
        assert!(!info.clean);
        assert_eq!(info.text, "xyza\n");
        t.keys(":w<Enter>");
        t.s.maybe_swap(now + Duration::from_secs(3));
        assert!(
            crate::swap::read(&sp).unwrap().clean,
            "saved: back to a clean lock"
        );
        t.s.cleanup();
        assert!(!sp.exists(), "a normal exit removes it");
    }

    #[test]
    fn never_removes_or_overwrites_someone_elses_swap() {
        let mut t = T::open(Some("f"), Some("a\n"));
        let sp = t.s.swap_path.clone();
        // Another (live) owner's swap appears for our file.
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        crate::swap::write(&sp, Some(&t.dir.path().join("f")), "theirs").unwrap();
        let text = fs::read_to_string(&sp).unwrap().replacen(
            &format!("\"pid\":{}", std::process::id()),
            &format!("\"pid\":{}", child.id()),
            1,
        );
        fs::write(&sp, text).unwrap();
        t.keys("ixyz<Esc>");
        t.s.write_swap();
        t.s.cleanup();
        let info = crate::swap::read(&sp).unwrap();
        assert_eq!(info.pid, child.id());
        assert_eq!(info.text, "theirs");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_clean_lock_left_by_a_dead_fred_is_removed_silently() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, "x\n").unwrap();
        let swap_dir = dir.path().join("swap");
        let sp = crate::swap::swap_path_in(&swap_dir, Some(&p));
        crate::swap::write_clean(&sp, Some(&p)).unwrap();
        let text = fs::read_to_string(&sp).unwrap().replacen(
            &format!("\"pid\":{}", std::process::id()),
            "\"pid\":999999",
            1,
        );
        fs::write(&sp, text).unwrap();
        let (_, info) = Session::open(Some(p), &Config::default(), &swap_dir).unwrap();
        assert!(info.is_none());
        assert!(!sp.exists());
    }

    #[test]
    fn edit_into_a_file_with_a_swap_asks_first() {
        let mut t = T::open(Some("a"), Some("aaa\n"));
        let b = t.dir.path().join("b");
        fs::write(&b, "bbb\n").unwrap();
        let sp = crate::swap::swap_path_in(&t.dir.path().join("swap"), Some(&b));
        crate::swap::write(&sp, Some(&b), "bbb recovered\n").unwrap();
        let text = fs::read_to_string(&sp).unwrap().replacen(
            &format!("\"pid\":{}", std::process::id()),
            "\"pid\":999999",
            1,
        );
        fs::write(&sp, text).unwrap();
        t.keys(&format!(":e {}<Enter>", b.display()));
        assert!(t.s.pending_edit.is_some(), "the choice is the user's");
        assert_eq!(
            t.s.ed.buf.text(),
            "aaa",
            "nothing switches before the answer"
        );
        t.s.resolve_edit(SwapChoice::Cancel);
        assert_eq!(t.s.ed.buf.text(), "aaa");
        t.keys(&format!(":e {}<Enter>", b.display()));
        t.s.resolve_edit(SwapChoice::Recover);
        assert_eq!(t.s.ed.buf.text(), "bbb recovered");
        assert!(t.s.ed.buf.modified);
        assert!(!t.s.no_swap, "the recovered text stays protected");
        let info = crate::swap::read(&sp).unwrap();
        assert_eq!(
            (info.pid, info.text.as_str()),
            (std::process::id(), "bbb recovered\n")
        );
    }

    #[test]
    fn swap_written_after_many_edits() {
        let mut t = T::open(Some("f"), Some("a\n"));
        t.keys(&"ix<Esc>".repeat(200));
        t.s.maybe_swap(Instant::now());
        assert!(t.s.swap_path.exists());
    }

    #[test]
    fn stale_swap_is_offered_and_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, "old\n").unwrap();
        let swap_dir = dir.path().join("swap");
        let sp = crate::swap::swap_path_in(&swap_dir, Some(&p));
        crate::swap::write(&sp, Some(&p), "recovered\r\n").unwrap();
        // Pretend the owner was a dead process.
        let text = fs::read_to_string(&sp).unwrap().replacen(
            &format!("\"pid\":{}", std::process::id()),
            "\"pid\":999999",
            1,
        );
        fs::write(&sp, text).unwrap();
        let (mut s, info) = Session::open(Some(p.clone()), &Config::default(), &swap_dir).unwrap();
        let info = info.expect("swap offered");
        assert!(!crate::swap::owner_alive(&info));
        s.recover(info);
        assert!(s.ed.buf.modified);
        for k in parse_keys(":w<Enter>") {
            s.handle_key(k);
        }
        assert_eq!(fs::read_to_string(&p).unwrap(), "recovered\r\n");
    }

    #[test]
    fn opening_a_pick_goes_to_the_match() {
        let mut t = T::open(Some("a"), Some("one\n"));
        let b = t.dir.path().join("b");
        fs::write(&b, "x\n  needle here\n").unwrap();
        let project = std::sync::Arc::clone(&t.s.ed.project);
        t.s.perform(ExEffect::Open {
            path: b.clone(),
            line: 1,
            col: 2,
            pattern: Some("needle".into()),
        });
        assert_eq!(t.s.ed.path.as_ref(), Some(&b));
        assert_eq!(t.s.ed.cur.pos(), (1, 2));
        assert_eq!(t.s.ed.last_pat.as_deref(), Some("needle"));
        assert!(std::sync::Arc::ptr_eq(&project, &t.s.ed.project));
        let recent = crate::pick::recent::load(&t.dir.path().join("recent"));
        assert_eq!(recent, [b, t.dir.path().join("a")]);
    }

    #[test]
    fn space_shift_b_searches_every_buffer() {
        let mut t = T::open(Some("a"), Some("one\nneedle a\n"));
        let b = t.dir.path().join("b");
        fs::write(&b, "x\ny\nneedle b\n").unwrap();
        t.keys(&format!(":e {}<Enter>:b1<Enter>", b.display()));
        t.keys(" Bneedle");
        let Mode::Pick(p) = &t.s.ed.mode else {
            panic!("no picker")
        };
        let rows: Vec<&str> = p.rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].ends_with("a:2: needle a") && rows[1].ends_with("b:3: needle b"));
        // Up to b's match: Enter goes to that buffer and line.
        t.keys("<Up><Enter>");
        assert_eq!((t.s.ed.path.as_ref(), t.s.ed.cur.pos()), (Some(&b), (2, 0)));
        assert_eq!(t.s.ed.last_pat.as_deref(), Some("needle"));
        // This buffer's line, with nothing typed: the cursor just moves.
        t.keys(" B<Enter>");
        assert_eq!((t.s.ed.path.as_ref(), t.s.ed.cur.pos()), (Some(&b), (2, 0)));
        t.keys("gg B<Down><Down><Enter>");
        assert_eq!(t.s.ed.cur.pos(), (2, 0));
        // Space b lists the buffers themselves, like :ls.
        t.keys(" b");
        let Mode::Pick(p) = &t.s.ed.mode else {
            panic!("no buffer list")
        };
        let rows: Vec<&str> = p.rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        t.keys("<Esc>:ls<Enter>");
        let Mode::Pick(q) = &t.s.ed.mode else {
            panic!("no :ls list")
        };
        assert_eq!(q.rows.len(), 2);
        // Enter picks the previous buffer; picking the one shown closes the list.
        let a = t.dir.path().join("a");
        t.keys("<Enter>");
        assert_eq!(
            (t.s.ed.path.as_ref(), &t.s.ed.mode),
            (Some(&a), &Mode::Normal)
        );
        t.keys(":ls<Enter><Up><Enter>");
        assert_eq!(
            (t.s.ed.path.as_ref(), &t.s.ed.mode),
            (Some(&a), &Mode::Normal)
        );
    }

    #[test]
    fn a_pick_keeps_unsaved_changes_in_their_buffer() {
        let mut t = T::open(Some("a"), Some("one\n"));
        fs::create_dir(t.dir.path().join(".git")).unwrap();
        fs::write(t.dir.path().join("b"), "two\n").unwrap();
        t.keys("x p");
        let ready = Instant::now();
        while crate::pick::tick(&mut t.s.ed) || ready.elapsed() < Duration::from_millis(200) {
            std::thread::sleep(Duration::from_millis(5));
        }
        t.keys("b<Enter>");
        assert_eq!(t.s.ed.path, Some(t.dir.path().join("b")));
        assert_eq!(t.s.ed.mode, Mode::Normal);
        t.keys("<C-^>");
        assert_eq!(t.s.ed.path, Some(t.dir.path().join("a")));
        assert_eq!(t.s.ed.buf.text(), "ne");
    }

    #[test]
    fn a_pick_that_cannot_open_closes_the_picker() {
        let mut t = T::open(Some("a"), Some("one\n"));
        fs::create_dir(t.dir.path().join(".git")).unwrap();
        t.keys(" g");
        assert!(matches!(t.s.ed.mode, Mode::Pick(_)));
        t.s.perform(ExEffect::Open {
            path: t.dir.path().join("a/x"),
            line: 0,
            col: 0,
            pattern: None,
        });
        assert_eq!(t.s.ed.mode, Mode::Normal);
        assert!(t.s.ed.msg.as_ref().is_some_and(|m| m.1), "{:?}", t.s.ed.msg);
        assert_eq!(t.s.ed.path, Some(t.dir.path().join("a")));
    }

    #[test]
    fn remembers_where_you_were_in_each_file() {
        let mut t = T::open(Some("a"), Some("one\ntwo\nthree\nfour line\n"));
        let b = t.dir.path().join("b");
        fs::write(&b, "x\ny\n").unwrap();
        let open = |t: &mut T, p: PathBuf, line, col, pattern: Option<&str>| {
            t.s.perform(ExEffect::Open {
                path: p,
                line,
                col,
                pattern: pattern.map(str::to_string),
            })
        };
        t.keys("3jw");
        assert_eq!(t.s.ed.cur.pos(), (3, 5));
        open(&mut t, b.clone(), 0, 0, None);
        assert_eq!(t.s.ed.cur.pos(), (0, 0));
        t.keys("j");
        // Back to a (a file pick: no target), where we left it.
        let a = t.dir.path().join("a");
        open(&mut t, a, 0, 0, None);
        assert_eq!(t.s.ed.cur.pos(), (3, 5));
        // A grep match is a target: it wins over the remembered place.
        open(&mut t, b.clone(), 0, 0, Some("x"));
        assert_eq!(t.s.ed.cur.pos(), (0, 0));
        // A new session (quit and reopen) starts where this one left off.
        t.keys("j");
        t.s.remember_place();
        let (s2, _) =
            Session::open(Some(b), &Config::default(), &t.dir.path().join("swap")).unwrap();
        assert_eq!(s2.ed.cur.pos(), (1, 0));
    }

    #[test]
    fn a_remembered_place_past_the_end_is_clamped() {
        let t = T::open(Some("a"), Some("one\ntwo\n"));
        let a = t.dir.path().join("a");
        crate::pick::recent::record(&t.dir.path().join("recent"), &a, Some((40, 99)));
        let (s2, _) =
            Session::open(Some(a), &Config::default(), &t.dir.path().join("swap")).unwrap();
        assert_eq!(s2.ed.cur.pos(), (1, 2));
    }
}
