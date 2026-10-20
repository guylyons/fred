//! One editing session: the editor plus its file, swap file and file effects.

use crate::buffer::{Buffer, LineEnding};
use crate::config::Config;
use crate::editor::{Editor, Mode};
use crate::ex::addr::Range;
use crate::ex::{BufCmd, ExEffect};
use crate::fileio::{self, FileStamp};
use crate::highlight::Highlighter;
use crate::key::Key;
use crate::options::Options;
use crate::swap::{self, SwapInfo};
use crate::ui::View;
use crate::window::{Node, Placed, Window};
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) mod magit;
mod occur;
mod org;
mod term;
mod windows;
pub use org::Finish;

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
    /// Its highlighting, kept while parked; None until first shown.
    hl: Option<Highlighter>,
    /// Its scroll when it was last left.
    view: View,
}

/// Claude's `fred_keys` being typed, a few keys a frame, so you watch.
pub struct Typing {
    keys: std::collections::VecDeque<Key>,
    per_frame: usize,
    reply: crate::remote::Reply,
}

pub struct Session {
    pub ed: Editor,
    /// Claude typing (see `tick_typing`); your keys wait until it's done.
    pub typing: Option<Typing>,
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
    pub pending_git: Option<crate::magit::repo::GitInvocation>,
    pub git_busy: bool,
    /// magit-commit-add-log: the entry (file, defun) for the draft being opened.
    pending_add_log: Option<(String, Option<String>)>,
    /// magit-refresh-verbose, and a one-shot magit-profile-refresh-buffer:
    /// report how long a refresh took and how many Git calls it made.
    refresh_verbose: bool,
    profile_once: Option<(std::time::Instant, usize)>,
    /// magit-wip-mode.
    pub wip_mode: bool,
    /// magit--disable-save-buffers: the buffers were just offered for saving.
    saving_done: bool,
    /// magit-save-repository-buffers: None never, Some(true) ask, Some(false)
    /// save without asking (tests never save unless they ask to).
    pub save_buffers: Option<bool>,
    /// magit-repolist-find-file-other-frame: the repositories to open in.
    repolist_files: Option<Vec<PathBuf>>,
    /// magit-auto-revert-mode: reload unmodified repository files that a Git
    /// command changed.
    pub auto_revert: bool,
    /// magit-process-buffer's log: each terminal Git command and its result.
    pub git_log: Vec<(PathBuf, String, Result<(), String>)>,
    magit_job: Option<magit::Job>,
    magit_picker_repo: Option<crate::magit::repo::Repo>,
    magit_drafts: std::collections::HashMap<
        PathBuf,
        (
            crate::magit::repo::Repo,
            crate::magit::CommitMode,
            Vec<String>,
        ),
    >,
    /// A swap write running in the background, and the version it holds.
    swap_job: Option<(u64, std::thread::JoinHandle<Result<(), String>>)>,
    /// `:ai` / `:explain`: the lines, the prompt and whether it explains.
    pub pending_ai: Option<(crate::ex::addr::Range, String, bool)>,
    lossy: bool,
    cfg: Config,
    seen_version: u64,
    last_change: Option<Instant>,
    swap_state: SwapState,
    swap_error_shown: bool,
    /// Every open buffer, numbered from 1. The one being edited lives in
    /// the fields above; its slot (`cur`) is None.
    bufs: Vec<Option<Parked>>,
    pub cur: usize,
    /// Counts buffer switches, to order buffers by last use.
    clock: u64,
    /// Every window, and how the screen is divided among them.
    pub windows: Vec<Window>,
    pub tree: Node,
    /// The active window's id: it shows the live buffer, `ed`.
    pub active: usize,
    /// The window `CTRL-W p` goes back to.
    prev_win: Option<usize>,
    /// `Space w`: the windows show their numbers until the next key.
    pub picking_window: bool,
    /// `Space w c`: the next digit is how many windows to lay out.
    grid_pending: bool,
    /// The which-key popup is up: `Space` was pressed a while ago.
    pub which_key: bool,
    next_win_id: usize,
    /// The windows' area on screen, set by the app every draw.
    pub area: Rect,
    /// The live buffer's highlighting (the app sets the theme).
    pub hl: Highlighter,
    /// A window's lines were left unhighlighted last frame: draw again.
    pub hl_incomplete: bool,
}

fn make_editor(buf: Buffer, opts: Options, cfg: &Config) -> Editor {
    let mut ed = Editor::new(buf);
    ed.opts = opts;
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
    // A sum of 0/1 vectorizes; `filter(..).count()` crawled on a 2 GB file.
    let mut lines: usize = data.iter().map(|&b| usize::from(b == b'\n')).sum();
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

fn open_file(path: Option<&Path>, opts: Options, cfg: &Config) -> Result<Opened, String> {
    let Some(p) = path else {
        let buf = Buffer::from_text("");
        return Ok(Opened {
            ed: make_editor(buf, opts, cfg),
            stamp: None,
            lossy: false,
        });
    };
    let l = fileio::load(p)?;
    let lossy = l.notice.as_deref() == Some("not valid UTF-8");
    let (lines, bytes) = (l.buf.len_lines(), l.buf.len_bytes());
    let mut ed = make_editor(l.buf, opts, cfg);
    ed.path = Some(p.to_path_buf());
    ed.readonly = l.readonly;
    crate::org::attach(&mut ed);
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
    if let Some(v) = &ed.magit {
        return format!("[{}]", v.title());
    }
    if ed.occur.is_some() {
        return "[*Occur*]".into();
    }
    if ed.claude.is_some() {
        return "[*claude*]".into();
    }
    if ed.term.is_some() {
        return "[terminal]".into();
    }
    if let Some(n) = &ed.org_buffer_name {
        return format!("[{n}]");
    }
    ed.path
        .as_ref()
        .map_or_else(|| "[No Name]".into(), |p| p.display().to_string())
}

impl Session {
    /// Git work is running in the background.
    pub fn working(&self) -> bool {
        self.magit_job.is_some() || self.git_busy || crate::claude::busy(&self.ed)
    }

    /// Open `path` (or an unnamed buffer). Returns a leftover swap file, if any.
    pub fn open(
        path: Option<PathBuf>,
        cfg: &Config,
        swap_dir: &Path,
    ) -> Result<(Session, Option<SwapInfo>), String> {
        // `fred DIR`: an empty buffer, browsing DIR.
        crate::org::options::set(cfg.org.0.clone());
        let browse = path.clone().filter(|p| p.is_dir());
        let path = if browse.is_some() { None } else { path };
        // Config errors were reported when it was parsed.
        let opts = cfg.options().unwrap_or_default();
        let o = open_file(path.as_deref(), opts, cfg)?;
        let swap_path = swap::swap_path_in(swap_dir, path.as_deref());
        let mut s = Session {
            typing: None,
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
            pending_git: None,
            git_log: vec![],
            git_busy: false,
            pending_add_log: None,
            refresh_verbose: crate::magit::options::flag("magit-refresh-verbose", false),
            profile_once: None,
            wip_mode: crate::magit::options::flag("magit-wip-mode", false),
            repolist_files: None,
            saving_done: false,
            save_buffers: match crate::magit::options::value("magit-save-repository-buffers") {
                Some(toml::Value::Boolean(false)) => None,
                Some(toml::Value::String(s)) if s == "dontask" => Some(false),
                None if cfg!(test) => None,
                _ => Some(true),
            },
            auto_revert: crate::magit::options::flag("magit-auto-revert-mode", true),
            magit_job: None,
            magit_picker_repo: None,
            magit_drafts: std::collections::HashMap::new(),
            swap_job: None,
            pending_ai: None,
            lossy: o.lossy,
            cfg: cfg.clone(),
            last_change: None,
            swap_state: SwapState::None,
            swap_error_shown: false,
            bufs: vec![None],
            cur: 0,
            clock: 0,
            windows: vec![Window {
                id: 1,
                buf: 0,
                cur: Default::default(),
                view: View::default(),
                prev: None,
            }],
            tree: Node::Leaf(1),
            active: 1,
            prev_win: None,
            picking_window: false,
            grid_pending: false,
            which_key: false,
            next_win_id: 2,
            area: Rect::new(0, 0, 80, 23),
            hl: Highlighter::new("ansi", false)?,
            hl_incomplete: false,
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
            s.open_dired(d, None);
        }
        let info = s.leftover_swap();
        s.attach_git_commit_mode();
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
        if self.ed.generated() {
            return;
        }
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
        // Each open counts toward the file's frecency in `Space p`.
        let now = crate::pick::fff::now();
        crate::pick::fff::track(&rf.with_file_name("frecency"), &p, now);
    }

    /// Remember where the cursor is in this file, for next time.
    pub fn remember_place(&self) {
        if self.ed.generated() {
            return;
        }
        if let Some(p) = &self.ed.path {
            let pos = (self.ed.cur.line, self.ed.cur.byte);
            crate::pick::recent::record(&self.recent_file(), p, Some(pos));
        }
    }

    fn leftover_swap(&mut self) -> Option<SwapInfo> {
        leftover(&self.swap_path, &self.ed.buf)
    }

    pub fn handle_key(&mut self, k: Key) {
        self.which_key = false;
        // Terminal mode's keys go to the shell (`term_key`), not here: these
        // are Claude's or replayed ones, and must not reach the shell.
        if self.in_terminal() {
            return;
        }
        if std::mem::take(&mut self.grid_pending) {
            match k.char().and_then(|c| c.to_digit(10)).filter(|&n| n > 0) {
                Some(n) => self.grid(n as usize),
                None => self.ed.msg = None,
            }
            return;
        }
        if std::mem::take(&mut self.picking_window) {
            self.ed.msg = None;
            if k == Key::ch('c') {
                self.grid_pending = true;
                return self.ed.set_msg("how many windows? 1-9");
            }
            // A digit picks a window; any other key just puts the numbers away.
            if let Some(n) = k.char().and_then(|c| c.to_digit(10)).filter(|&n| n > 0)
                && let Some(&id) = self.tree.order().get(n as usize - 1)
            {
                self.focus(id);
            }
            return;
        }
        // The approval dialog takes `y` and `n` in Normal mode, from whichever
        // buffer you're in: typed text (Insert, `:`, `/`) never answers it. Never
        // from Claude's own typing either (it would approve itself).
        let normal = self.ed.mode == Mode::Normal && self.ed.vim.pending.is_empty();
        if normal
            && self.typing.is_none()
            && self.editors_mut().any(|ed| crate::claude::answer(ed, k))
        {
            self.ed.msg = None;
            return;
        }
        if self.ed.claude.is_some() {
            let files = self.open_files();
            if let Some(c) = &mut self.ed.claude {
                c.files = files;
            }
        }
        let zapping = self.ed.zap.is_some();
        self.ed.handle_key(k);
        if !zapping && self.ed.zap.is_some() {
            self.zap_windows();
        }
        // A chat is never "unsaved".
        if self.ed.claude.is_some() {
            self.ed.buf.modified = false;
        }
        // Effects can ask for more effects (Org work across buffers).
        for _ in 0..16 {
            match self.ed.pending_effect.take() {
                Some(eff) => self.perform(eff),
                None => break,
            }
        }
    }

    pub fn perform(&mut self, eff: ExEffect) {
        match eff {
            // The editor applies these itself (`Editor::run_ex`).
            ExEffect::None | ExEffect::Set { .. } | ExEffect::Help(_) => {}
            ExEffect::Magit(a) => self.magit_action(a),
            ExEffect::Org(e) => {
                if let Some(f) = e.take() {
                    f(self);
                }
            }
            ExEffect::Write { .. } if self.ed.dired.is_some() => crate::dired::save(&mut self.ed),
            ExEffect::Write { .. } if self.ed.occur.is_some() => self.occur_write(),
            ExEffect::Occur { pat, range } => self.occur(&pat, range),
            ExEffect::OccurRevert => self.occur_revert(),
            ExEffect::Claude(a) => {
                self.claude();
                crate::claude::act(&mut self.ed, a);
            }
            ExEffect::Write {
                path,
                force,
                range,
                then_quit,
            } => {
                if self.write(path, force, range) && then_quit {
                    if self.windows.len() > 1 {
                        self.close_active();
                    } else if self.ed.buf.modified {
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
                    if self.windows.len() > 1 {
                        self.close_active();
                    } else {
                        self.quit_if_all_saved();
                    }
                }
            }
            // In a split, `:q` closes the window; the buffer stays, hidden.
            ExEffect::Quit { .. } if self.windows.len() > 1 => self.close_active(),
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
            ExEffect::Window { cmd, count } => self.wincmd(cmd, count),
            ExEffect::ZapTo { win, line, byte } => {
                self.focus(win);
                self.ed.set_cursor(line, byte);
            }
            ExEffect::Close { .. } => self.close_active(),
            ExEffect::Only => self.only(),
            ExEffect::QuitAll { force, write } => {
                if force {
                    self.quit = true;
                } else if !write || self.write_all() {
                    self.quit_if_all_saved();
                }
            }
            ExEffect::Split {
                vertical,
                what,
                size,
            } => self.split(vertical, what, size),
            ExEffect::Edit { path, force } => self.edit(path.as_deref(), force),
            ExEffect::Shell(cmd) => match crate::shell::guard() {
                Ok(()) => self.pending_shell = Some(cmd),
                Err(e) => self.ed.set_err(e),
            },
            ExEffect::Terminal { split } => match crate::shell::guard() {
                Ok(()) => self.terminal(split),
                Err(e) => self.ed.set_err(e),
            },
            ExEffect::Ai {
                range,
                prompt,
                explain,
            } => self.pending_ai = Some((range, prompt, explain)),
            ExEffect::Pwd => match std::env::current_dir() {
                Ok(d) => self.ed.set_msg(d.display().to_string()),
                Err(e) => self.ed.set_err(fileio::err_msg(&e)),
            },
            ExEffect::Cd(dir) => self.cd(dir.as_deref()),
            ExEffect::Reveal(dir) => self.browse(&dir.to_string_lossy()),
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
        if self.ed.generated() {
            self.ed.set_err("generated Git buffer is read-only");
            return false;
        }
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
                // magit-wip-after-save-mode.
                if self.wip_mode {
                    self.wip_after_save(p);
                }
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
        if path.is_none() && self.ed.blob.is_some() {
            self.ed.set_err("blob buffer: use gr to refresh");
            return;
        }
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

    /// Vim's 'autoread': reload the current file when it changed on disk and
    /// the buffer has no unsaved changes. True when it reloaded.
    pub fn auto_read(&mut self) -> bool {
        let ed = &self.ed;
        let stale = !ed.buf.modified
            && !ed.generated()
            && ed.commit_repo.is_none()
            && ed.path.as_ref().is_some_and(|p| {
                // A deleted file keeps its buffer.
                p.is_file() && fileio::changed_on_disk(p, self.stamp.as_ref())
            });
        if !stale {
            return false;
        }
        let (line, byte) = (self.ed.cur.line, self.ed.cur.byte);
        self.edit(None, false);
        self.ed.set_cursor(line, byte);
        self.ed.set_msg("file changed on disk: reloaded");
        true
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
            // The working directory itself is `.`, not an empty (new) file.
            .map(|p| {
                if p.as_os_str().is_empty() {
                    ".".into()
                } else {
                    p
                }
            })
            .unwrap_or(path);
        self.edit_path(path, then);
        // Still here: the open failed or waits on a swap question.
        if matches!(self.ed.mode, Mode::Pick(_)) {
            self.ed.mode = Mode::Normal;
        }
    }

    fn edit_path(&mut self, p: PathBuf, then: Option<Goto>) {
        if p.is_dir() {
            // A directory's target is the name to put the cursor on.
            return self.open_dired(&p, then.and_then(|g| g.pattern).as_deref());
        }
        if let Some(i) = self.find(&p) {
            self.show(i);
            self.go(then);
            return;
        }
        let o = match open_file(Some(&p), self.ed.opts.for_new_buffer(), &self.cfg) {
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

    /// A dired buffer for `dir` (the one already open, if any), the cursor
    /// on the entry named `focus`: else on the file (or subdirectory) you
    /// came from, else where it already was.
    fn open_dired(&mut self, dir: &Path, focus: Option<&str>) {
        let root = swap::canonical(dir);
        let from = focus.map(str::to_owned).or_else(|| {
            let p = swap::canonical(self.ed.path.as_deref()?);
            let first = p.strip_prefix(&root).ok()?.components().next()?;
            Some(first.as_os_str().to_string_lossy().into_owned())
        });
        let focus = from.as_deref();
        if let Some(i) = self.find(dir) {
            self.show(i);
            let kept = from.clone().or_else(|| crate::dired::current(&self.ed));
            let focus = kept.as_deref();
            if let Err(e) = crate::dired::visit(&mut self.ed, dir, focus) {
                self.ed.set_err(e);
            }
            return;
        }
        let mut ed = make_editor(
            Buffer::from_text(""),
            self.ed.opts.for_new_buffer(),
            &self.cfg,
        );
        if let Err(e) = crate::dired::visit(&mut ed, dir, focus) {
            return self.ed.set_err(e);
        }
        let swap = swap::swap_path_in(&self.swap_dir, ed.path.as_deref());
        let o = Opened {
            ed,
            stamp: None,
            lossy: false,
        };
        self.switch_to(o, swap);
        // A listing: no swap file, no git marks, not a remembered place.
        self.no_swap = true;
        self.ed.git = crate::git::Gutter::default();
        if let Err(e) = crate::dired::visit(&mut self.ed, dir, focus) {
            self.ed.set_err(e);
        }
    }

    /// Make `o` the buffer being edited, with its swap file at `new_swap`:
    /// a new buffer, or in place of this one when reloading it (or when
    /// this one is an empty unnamed one).
    fn switch_to(&mut self, o: Opened, new_swap: PathBuf) {
        let shown_elsewhere = self
            .windows
            .iter()
            .any(|w| w.id != self.active && w.buf == self.cur);
        let blank = o.ed.magit.is_none()
            && !shown_elsewhere
            && self.ed.path.is_none()
            && !self.ed.buf.modified
            && self.ed.buf.len_bytes() == 0;
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
            hl: None,
            view: View::default(),
        }));
        self.show(self.bufs.len() - 1);
        self.arrived();
        self.attach_git_commit_mode();
    }

    /// Put `o` in place of the buffer being edited.
    fn replace(&mut self, o: Opened, new_swap: PathBuf) {
        self.clock += 1; // Invalidate reads started for the buffer being replaced.
        self.wait_swap();
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
        self.hl.set_file(
            self.ed.path.as_deref().or(self.ed.syntax_path.as_deref()),
            &self.ed.buf,
        );
        self.win_mut().view = View::default();
        self.attach_git_commit_mode();
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
        let o = match open_file(Some(&path), self.ed.opts.for_new_buffer(), &self.cfg) {
            Ok(o) => o,
            Err(e) => {
                self.ed.set_err(format!("{}: {e}", path.display()));
                return;
            }
        };
        let new_swap = swap::swap_path_in(&self.swap_dir, Some(&path));
        self.switch_to(o, new_swap);
        self.go(then);
        self.attach_commit_repo();
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

    /// Show buffer `i` in the active window, leaving the current one open
    /// (and its unsaved text in its swap file). The window keeps each
    /// buffer's scroll.
    fn show(&mut self, i: usize) {
        if i == self.cur {
            // Picked from the buffer list: still close it.
            self.ed.mode = Mode::Normal;
            return;
        }
        let from = self.cur;
        let incoming = self.bufs[i].as_ref().map(|p| p.view).unwrap_or_default();
        self.make_live(i);
        let out = self.win().view;
        if let Some(p) = self.bufs[from].as_mut() {
            p.view = out;
        }
        let w = self.win_mut();
        w.view = incoming;
        w.prev = Some(from);
        w.buf = i;
    }

    /// Make buffer `i` the live one, `ed`; nothing about windows.
    fn make_live(&mut self, i: usize) {
        self.remember_place();
        self.write_swap();
        let mut p = self.bufs[i].take().expect("a parked buffer");
        let hl = p.hl.take().unwrap_or_else(|| {
            let mut h = self.hl.blank();
            h.set_file(
                p.ed.path.as_deref().or(p.ed.syntax_path.as_deref()),
                &p.ed.buf,
            );
            h
        });
        p.hl = Some(std::mem::replace(&mut self.hl, hl));
        // Text that changed while parked (Claude's replies, a refresh).
        if let Some(d) = p.ed.buf.take_dirty_from() {
            self.hl.invalidate(d);
        }
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
        if self.ed.magit.as_ref().is_some_and(|view| view.dirty) {
            self.magit_action(crate::magit::Action::Refresh);
        }
    }

    /// Feed keys in Vim notation, as tests type them.
    #[cfg(test)]
    pub fn handle_keys(&mut self, k: &str) {
        for key in crate::key::parse_keys(k) {
            self.handle_key(key);
        }
    }

    /// Each window on screen, the active one first: its buffer's editor
    /// (with the window's cursor in it, clamped to the text), its view, its
    /// buffer's highlighting, where it is, and whether it is active.
    pub fn each_window(
        &mut self,
        mut f: impl FnMut(&Editor, &mut View, &mut Highlighter, Placed, bool),
    ) {
        let mut placed = self.tree.layout(self.area);
        placed.sort_by_key(|p| p.id != self.active);
        for p in placed {
            let Some(w) = self.windows.iter_mut().find(|w| w.id == p.id) else {
                continue;
            };
            let active = p.id == self.active;
            let (ed, hl) = if w.buf == self.cur {
                (&mut self.ed, &mut self.hl)
            } else {
                let Some(park) = self.bufs[w.buf].as_mut() else {
                    continue;
                };
                let hl = park.hl.get_or_insert_with(|| {
                    let mut h = self.hl.blank();
                    h.set_file(
                        park.ed.path.as_deref().or(park.ed.syntax_path.as_deref()),
                        &park.ed.buf,
                    );
                    h
                });
                if let Some(d) = park.ed.buf.take_dirty_from() {
                    hl.invalidate(d);
                }
                (&mut park.ed, hl)
            };
            if active {
                f(ed, &mut w.view, hl, p, true);
                continue;
            }
            let saved = ed.cur;
            ed.set_cursor(w.cur.line, w.cur.byte);
            ed.cur.want_col = w.cur.want_col;
            w.cur = ed.cur;
            f(ed, &mut w.view, hl, p, false);
            ed.cur = saved;
        }
    }

    /// The active window.
    pub fn win(&self) -> &Window {
        self.windows
            .iter()
            .find(|w| w.id == self.active)
            .expect("the active window")
    }

    pub fn win_mut(&mut self) -> &mut Window {
        let id = self.active;
        self.windows
            .iter_mut()
            .find(|w| w.id == id)
            .expect("the active window")
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
            let o =
                open_file(None, self.ed.opts.for_new_buffer(), &self.cfg).expect("an empty buffer");
            self.replace(o, swap::swap_path_in(&self.swap_dir, None));
            self.lock();
            return;
        }
        self.close_windows_on(target);
        if target == self.cur {
            let to = self.buffer_arg("#").unwrap_or((target + 1) % n);
            self.show(to);
        }
        self.drop_buffer(target);
    }

    /// Close the windows showing buffer `i`, except the last window.
    fn close_windows_on(&mut self, i: usize) {
        let on: Vec<usize> = self
            .windows
            .iter()
            .filter(|w| w.buf == i)
            .map(|w| w.id)
            .collect();
        for id in on {
            if self.windows.len() > 1 {
                let _ = self.close_window(id);
            }
        }
    }

    /// Forget parked buffer `target`: its windows close, and every buffer
    /// number above it (windows', Magit's way back) shifts down.
    fn drop_buffer(&mut self, target: usize) {
        self.close_windows_on(target);
        if let Some(b) = self.bufs.remove(target)
            && !b.no_swap
        {
            release(&b.swap_path);
        }
        if target < self.cur {
            self.cur -= 1;
        }
        for w in &mut self.windows {
            if w.buf > target {
                w.buf -= 1;
            }
            w.prev = w
                .prev
                .filter(|&p| p != target)
                .map(|p| if p > target { p - 1 } else { p });
        }
        self.clock += 1;
        for ed in self.editors_mut() {
            if let Some(view) = &mut ed.magit {
                if view.return_to == target {
                    view.return_to = usize::MAX;
                } else if view.return_to > target && view.return_to != usize::MAX {
                    view.return_to -= 1;
                }
            }
        }
    }

    /// `:claude`: show the chat (one per fred, as `*Occur*`), ready to type.
    fn claude(&mut self) {
        let chat = (0..self.bufs.len()).find(|&i| self.ed_at(i).claude.is_some());
        // Its own pane, below the one you're in: the one it has, if it's open.
        if chat != Some(self.cur) {
            match self.windows.iter().find(|w| Some(w.buf) == chat) {
                Some(w) => self.focus(w.id),
                None => self.split_at(false, true, crate::ex::SplitWhat::Same, None),
            }
        }
        match chat {
            Some(i) if i == self.cur => {}
            Some(i) => self.show(i),
            None => {
                let mut ed = make_editor(
                    Buffer::from_text("> "),
                    self.ed.opts.for_new_buffer(),
                    &self.cfg,
                );
                let cmd = match self.cfg.claude_permissions.as_str() {
                    "" => self.cfg.claude_command.clone(),
                    mode => format!("{} --permission-mode {mode}", self.cfg.claude_command),
                };
                ed.claude = Some(Box::new(crate::claude::Chat::new(&cmd)));
                ed.syntax_path = Some(PathBuf::from("claude.md"));
                let swap = self.swap_dir.join(format!("claude-{}", self.bufs.len()));
                self.bufs.push(Some(Parked {
                    ed,
                    stamp: None,
                    swap_path: swap,
                    lossy: false,
                    no_swap: true,
                    swap_state: SwapState::None,
                    used: 0,
                    hl: None,
                    view: View::default(),
                }));
                self.show(self.bufs.len() - 1);
            }
        }
        let last = self.ed.line_count() - 1;
        self.ed.set_cursor(last, 0);
        self.handle_key(Key::ch('A'));
    }

    /// The other buffers' files, for the chat: `path:line`, the one used
    /// last first.
    fn open_files(&self) -> String {
        let mut parked: Vec<&Parked> = self.bufs.iter().flatten().collect();
        parked.sort_by_key(|b| std::cmp::Reverse(b.used));
        let mut out = String::new();
        for ed in parked.iter().map(|b| &b.ed).filter(|ed| !ed.generated()) {
            let Some(p) = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok()) else {
                continue;
            };
            let unsaved = if ed.buf.modified { " unsaved" } else { "" };
            out += &format!("{}:{}{unsaved}\n", p.display(), ed.cur.line + 1);
        }
        out
    }

    /// A tool call from Claude, through `fred --mcp` (see `remote`). Keys
    /// are typed over the next frames (`tick_typing`); approvals wait for
    /// you; the rest is answered now.
    pub fn remote(&mut self, tool: &str, args: &serde_json::Value, reply: crate::remote::Reply) {
        let now = match tool {
            "fred_approve" => return self.approve(args, reply),
            "fred_keys" => match self.start_typing(args) {
                Ok(keys) => {
                    // All typed in about 3s (100 frames), however many.
                    let per_frame = keys.len().div_ceil(100).max(1);
                    self.typing = Some(Typing {
                        keys,
                        per_frame,
                        reply,
                    });
                    return;
                }
                Err(e) => Err(e),
            },
            "fred_state" => Ok(self.state()),
            "fred_read" => self.read_buffer(args),
            _ => Err(format!("unknown tool: {tool}")),
        };
        let _ = reply.send(now);
    }

    fn start_typing(
        &self,
        args: &serde_json::Value,
    ) -> Result<std::collections::VecDeque<Key>, String> {
        let keys = args["keys"].as_str().ok_or("keys is required")?;
        // Else a `y` typed by Claude would approve Claude.
        if self.asking() {
            return Err("fred is asking the user to allow a tool; try again after".into());
        }
        Ok(crate::key::parse_keys(keys).into())
    }

    /// Type a frame's worth of Claude's keys; the last one answers Claude
    /// with what fred shows. True while typing.
    pub fn tick_typing(&mut self) -> bool {
        let Some(t) = &mut self.typing else {
            return false;
        };
        let n = t.per_frame.min(t.keys.len());
        let keys: Vec<Key> = t.keys.drain(..n).collect();
        for k in keys {
            if self.quit || self.typing.is_none() {
                break;
            }
            self.type_key(k);
        }
        if self.quit || self.typing.as_ref().is_some_and(|t| t.keys.is_empty()) {
            let t = self.typing.take().unwrap();
            // The lines around the cursor too, so a slipped edit shows at once.
            let l = self.ed.cur.line + 1;
            let near = self.read_buffer(
                &serde_json::json!({"start": l.saturating_sub(2).max(1), "end": l + 2}),
            );
            let _ = t.reply.send(Ok(self.state() + &near.unwrap_or_default()));
        }
        true
    }

    /// One of Claude's keys. Text in Insert mode goes in as typed, like a
    /// paste: no autoindent doubling Claude's indent, no completion popup
    /// taking its Enter.
    fn type_key(&mut self, k: Key) {
        use crate::key::KeyCode;
        let plain = !k.ctrl && !k.alt;
        let text = match k.code {
            KeyCode::Char(c) if plain => Some(c.to_string()),
            KeyCode::Enter if plain => Some("\n".into()),
            KeyCode::Tab if plain => Some("\t".into()),
            _ => None,
        };
        let file = self.ed.claude.is_none();
        let was_insert = self.ed.mode == Mode::Insert;
        // In the chat buffer Claude may only leave it (`:b 1<CR>`): any other
        // key would type into your prompt, or Enter send it.
        if !file
            && k.code != KeyCode::Esc
            && !matches!(self.ed.mode, Mode::Command(_))
            && !(self.ed.mode == Mode::Normal && plain && k.code == KeyCode::Char(':'))
        {
            return self.refuse_typing("refused: that key would edit the [*claude*] chat buffer; leave it first with `:b N<CR>` or `:e path<CR>`");
        }
        let before = (!file).then(|| self.ed.buf.clone());
        match text {
            Some(t) if was_insert && file => return self.ed.paste(&t),
            _ => {
                crate::shell::CLAUDE_TYPING.set(true);
                self.handle_key(k);
                crate::shell::CLAUDE_TYPING.set(false);
            }
        }
        // A `:` command that edited the chat anyway (`:s`, `:d`) is undone.
        if let Some(b) =
            before.filter(|b| self.ed.claude.is_some() && b.version != self.ed.buf.version)
        {
            let end = self.ed.buf.len_chars();
            self.ed.apply(crate::buffer::Edit {
                start: 0,
                end,
                text: b.text(),
            });
            return self.refuse_typing(
                "refused: that edited the [*claude*] chat buffer, so fred undid it",
            );
        }
        // `o`, `O`, `cc`, `S` autoindent the new line; Claude sends its own.
        if file && !was_insert && self.ed.mode == Mode::Insert {
            let line = self.ed.buf.line(self.ed.cur.line).to_string();
            if !line.is_empty() && line.trim().is_empty() && self.ed.cur.byte == line.len() {
                let start = self.ed.buf.line_to_char(self.ed.cur.line);
                let end = start + line.chars().count();
                self.ed.apply(crate::buffer::Edit {
                    start,
                    end,
                    text: String::new(),
                });
                self.ed.cur.byte = 0;
            }
        }
    }

    /// Ctrl-C while Claude types: the rest is dropped, and Claude told.
    pub fn stop_typing(&mut self) {
        if let Some(t) = self.typing.take() {
            let left = t.keys.len();
            let msg = format!(
                "the user stopped you (Ctrl-C), {left} keys short; fred_state shows where fred is"
            );
            let _ = t.reply.send(Err(msg));
        }
    }

    /// Claude's keys stopped early, and Claude told why.
    fn refuse_typing(&mut self, why: &str) {
        if let Some(t) = self.typing.take() {
            let _ = t
                .reply
                .send(Err(format!("{why}; fred_state shows where fred is")));
        }
    }

    /// `fred_approve`: Claude, in a chat, asks to use a tool. The answer
    /// waits for `y` or `n`.
    pub fn approve(
        &mut self,
        args: &serde_json::Value,
        reply: std::sync::mpsc::Sender<Result<String, String>>,
    ) {
        match self.editors_mut().find(|ed| crate::claude::live(ed)) {
            Some(ed) => crate::claude::ask(ed, args, reply),
            None => {
                let _ = reply.send(Err("no :claude chat in fred is waiting".into()));
            }
        }
    }

    fn asking(&self) -> bool {
        let parked = self.bufs.iter().flatten().map(|b| &b.ed);
        std::iter::once(&self.ed)
            .chain(parked)
            .any(|ed| crate::claude::question(ed).is_some())
    }

    /// What fred shows, for Claude: mode, this buffer, message, buffers.
    fn state(&self) -> String {
        let label = |ed: &Editor| match ed.path.as_deref().and_then(|p| std::path::absolute(p).ok())
        {
            Some(p) if !ed.generated() => p.display().to_string(),
            _ => buf_name(ed),
        };
        let unsaved = |ed: &Editor| if ed.buf.modified { ", unsaved" } else { "" };
        let ed = &self.ed;
        let mut out = format!(
            "mode: {}\nbuffer {}: {}, line {} col {} of {} lines{}\n",
            crate::ui::render::mode_name(&ed.mode),
            self.cur + 1,
            label(ed),
            ed.cur.line + 1,
            crate::text::col_of_byte(&ed.buf.line(ed.cur.line), ed.cur.byte, ed.tabstop()) + 1,
            ed.line_count(),
            unsaved(ed),
        );
        if let Some((m, _)) = &ed.msg {
            out += &format!("message: {m}\n");
        }
        let mut list: Vec<usize> = (0..self.bufs.len()).collect();
        list.sort_by_key(|&i| {
            std::cmp::Reverse(self.bufs[i].as_ref().map_or(u64::MAX, |b| b.used))
        });
        out += "buffers, most recently used first:\n";
        for i in list {
            let ed = self.ed_at(i);
            let here = if i == self.cur { "*" } else { " " };
            out += &format!(
                "{here}{} {}:{}{}\n",
                i + 1,
                label(ed),
                ed.cur.line + 1,
                unsaved(ed)
            );
        }
        out
    }

    /// Numbered lines of a buffer, at most `READ_MAX` a call. A `path` is
    /// opened (and shown, at `start`) first, as `:e` would.
    fn read_buffer(&mut self, args: &serde_json::Value) -> Result<String, String> {
        const READ_MAX: usize = 500;
        if let Some(p) = args["path"].as_str() {
            if self.typing.is_some() || self.asking() {
                return Err("fred is busy; try again".into());
            }
            let line = args["start"].as_u64().unwrap_or(1).max(1) as usize - 1;
            let path = PathBuf::from(p);
            if !path.is_file() {
                return Err(format!(
                    "{p} isn't a file (to make one: fred_keys `:e {p}<CR>`)"
                ));
            }
            self.ed.mode = Mode::Normal;
            self.edit_path(
                path.clone(),
                Some(Goto {
                    line,
                    col: 0,
                    pattern: None,
                }),
            );
            if self.pending_edit.is_some() {
                return Err(format!(
                    "{p} has a swap file; the user is being asked what to do"
                ));
            }
            if !self.is_own_file(&path) {
                return Err(self
                    .ed
                    .msg
                    .clone()
                    .map_or(format!("can't open {p}"), |m| m.0));
            }
        }
        let i = match args["buffer"].as_u64() {
            Some(n) => (n as usize)
                .checked_sub(1)
                .filter(|&i| i < self.bufs.len())
                .ok_or(format!("no buffer {n} (fred_state lists them)"))?,
            None => self.cur,
        };
        let ed = self.ed_at(i);
        let n = ed.line_count();
        let start = args["start"].as_u64().unwrap_or(1).max(1) as usize;
        if start > n {
            return Err(format!("line {start} is past the end ({n} lines)"));
        }
        let end = args["end"]
            .as_u64()
            .map_or(n, |e| e as usize)
            .clamp(start, n)
            .min(start + READ_MAX - 1);
        let mut out = format!("buffer {}: lines {start}-{end} of {n}\n", i + 1);
        for l in start - 1..end {
            out += &format!("{}\t{}\n", l + 1, ed.buf.line(l));
        }
        Ok(out)
    }

    /// Claude's replies, into every chat (shown or not); the spinner.
    pub fn tick_claude(&mut self) -> bool {
        let mut changed = false;
        let mut finished = None;
        for ed in self.editors_mut() {
            let was = crate::claude::busy(ed);
            changed |= crate::claude::tick(ed);
            if was && !crate::claude::busy(ed) {
                finished = Some(ed.msg.clone());
            }
        }
        // The turn is over: back to the chat, where it says how it went.
        if let Some(msg) = finished
            && self.ed.claude.is_none()
        {
            self.claude();
            self.ed.msg = msg;
            changed = true;
        }
        let working = self.editors_mut().find_map(|ed| crate::claude::working(ed));
        changed |= working.is_some();
        self.ed.claude_working = working;
        let asked = self
            .editors_mut()
            .find_map(|ed| crate::claude::question(ed));
        if let Some(s) = asked {
            self.ed.set_msg(s);
            changed = true;
        }
        changed
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
        let mut ed = make_editor(
            Buffer::from_text(&info.text),
            self.ed.opts.for_new_buffer(),
            &self.cfg,
        );
        ed.inherit(&mut self.ed);
        ed.path = self.ed.path.clone();
        ed.readonly = self.ed.readonly;
        ed.commit_repo = self.ed.commit_repo.clone();
        ed.commit_mode = self.ed.commit_mode.clone();
        ed.commit_args = self.ed.commit_args.clone();
        crate::magit::sync_commit_options(&mut ed);
        ed.saved_state = u64::MAX;
        ed.buf.modified = true;
        ed.set_msg("recovered unsaved changes; :w to save them");
        self.ed = ed;
        self.seen_version = self.ed.buf.version;
    }

    /// Delete a leftover swap file the user chose not to recover.
    pub fn discard_swap(&mut self) {
        self.wait_swap();
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
        if self.swap_job.as_ref().is_some_and(|j| j.1.is_finished()) {
            self.wait_swap();
        }
        let v = self.ed.buf.version;
        if v != self.seen_version {
            self.seen_version = v;
            self.last_change = Some(now);
        }
        if self.no_swap || self.ed.generated() {
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
            self.start_swap();
        }
    }

    /// Write the swap in the background: for a big file that takes
    /// seconds, and typing shouldn't wait. One write at a time.
    fn start_swap(&mut self) {
        if self.swap_job.is_some() || !self.may_write_swap() {
            return;
        }
        let text = self.ed.buf.snapshot();
        let (swap, file) = (self.swap_path.clone(), self.ed.path.clone());
        let job = std::thread::spawn(move || swap::write(&swap, file.as_deref(), &text));
        self.swap_job = Some((self.ed.buf.version, job));
    }

    /// Wait for the background swap write, if one is running.
    pub fn wait_swap(&mut self) {
        let Some((v, job)) = self.swap_job.take() else {
            return;
        };
        let r = job
            .join()
            .unwrap_or_else(|_| Err("swap: write failed".into()));
        if self.swap_written(r) {
            self.swap_state = SwapState::Dirty(v);
            self.ed.buf.edits_since_swap = 0;
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
        self.wait_swap();
        if !self.ed.buf.modified || !self.may_write_swap() {
            return;
        }
        let r = swap::write(
            &self.swap_path,
            self.ed.path.as_deref(),
            &self.ed.buf.snapshot(),
        );
        if self.swap_written(r) {
            self.swap_state = SwapState::Dirty(self.ed.buf.version);
            self.ed.buf.edits_since_swap = 0;
        }
    }

    /// Write a clean lock (no unsaved text).
    fn write_lock(&mut self) {
        // A dirty write still running would land on top of the lock.
        self.wait_swap();
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
        self.wait_swap();
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
        fn cmdline_text(&self) -> String {
            match &self.s.ed.mode {
                Mode::Command(cl) => cl.text.clone(),
                other => panic!("no command line: {other:?}"),
            }
        }
    }

    #[test]
    fn quit_in_split_closes_window_and_keeps_buffer() {
        let mut t = T::open(Some("a"), Some("a\n"));
        let b = t.dir.path().join("b");
        fs::write(&b, "b\n").unwrap();
        t.keys(&format!(":split {}<Enter>ix<Esc>:q<Enter>", b.display()));
        assert!(!t.s.quit);
        assert_eq!(t.s.windows.len(), 1);
        assert_eq!(t.s.ed.buf.text(), "a");
        assert!((0..t.s.bufs.len()).any(|i| t.s.ed_at(i).buf.text() == "xb"));
    }

    #[test]
    fn close_last_window_errors() {
        let mut t = T::open(Some("a"), Some("a\n"));
        t.keys("<C-w>c");
        assert_eq!(t.msg(), "? E444: Cannot close last window");
        t.keys(":close<Enter>");
        assert_eq!(t.msg(), "? E444: Cannot close last window");
    }

    #[test]
    fn only_and_hide() {
        let mut t = T::open(Some("a"), Some("a\n"));
        t.keys(":split<Enter>:vsplit<Enter>:only<Enter>");
        assert_eq!(t.s.windows.len(), 1);
        assert_eq!(t.s.tree.order(), vec![t.s.active]);
        t.keys(":split<Enter>:hide<Enter>");
        assert_eq!(t.s.windows.len(), 1);
    }

    #[test]
    fn qall_checks_every_buffer() {
        let mut t = T::open(Some("a"), Some("a\n"));
        let b = t.dir.path().join("b");
        t.keys(&format!(
            ":split {}<Enter>ix<Esc><C-w>j:qa<Enter>",
            b.display()
        ));
        assert!(!t.s.quit);
        t.keys(":wqa<Enter>");
        assert!(t.s.quit);
        assert_eq!(t.file("b"), "x\n");
    }

    #[test]
    fn bdelete_closes_its_windows() {
        let mut t = T::open(Some("a"), Some("a\n"));
        let b = t.dir.path().join("b");
        fs::write(&b, "b\n").unwrap();
        t.keys(&format!(":split {}<Enter>:bd<Enter>", b.display()));
        assert_eq!(t.s.windows.len(), 1);
        assert_eq!(t.s.ed.buf.text(), "a");
    }

    /// Every window shows a buffer that exists, and drawing them works.
    fn windows_valid(t: &mut T) {
        let n = t.s.bufs.len();
        assert!(t.s.windows.iter().all(|w| w.buf < n), "{:?}", t.s.windows);
        assert_eq!(t.s.win().buf, t.s.cur);
        t.s.each_window(|_, _, _, _, _| {});
    }

    #[test]
    fn org_finish_with_another_window_split() {
        let mut t = T::open(Some("a"), Some("a\n"));
        t.s.org_special("*Org Note*", "note\n", (0, 0), Box::new(|_, _, _| {}));
        let c = t.dir.path().join("c");
        t.keys(&format!(":split {}<Enter><C-w>j", c.display()));
        t.s.org_finish(false);
        windows_valid(&mut t);
        assert_eq!(t.s.ed.buf.text(), "a");
    }

    #[test]
    fn org_finish_shown_in_two_windows() {
        let mut t = T::open(Some("a"), Some("a\n"));
        t.s.org_special("*Org Note*", "note\n", (0, 0), Box::new(|_, _, _| {}));
        t.keys(":split<Enter>");
        t.s.org_finish(false);
        windows_valid(&mut t);
        assert_eq!(t.s.windows.len(), 1);
    }

    #[test]
    fn focus_clamps_normal_cursor_to_the_line() {
        let mut t = T::open(Some("a"), Some("abcdef\n"));
        t.keys(":split<Enter>$<C-w>j0lld$<C-w>kx");
        assert_eq!(t.s.ed.buf.text(), "a");
    }

    #[test]
    fn split_opens_above_and_moves_there() {
        let mut t = T::open(Some("a"), Some("1\n2\n3\n"));
        t.keys("3G:split<Enter>");
        assert_eq!(t.s.tree.order(), vec![t.s.active, t.s.windows[0].id]);
        assert_eq!(t.s.ed.cur.line, 2);
    }

    #[test]
    fn space_w_number_picks_a_window() {
        let mut t = T::open(Some("a"), Some("1\n2\n3\n"));
        t.keys("<C-w>s<C-w>s");
        let order = t.s.tree.order();
        t.keys(" w");
        assert!(t.s.picking_window);
        t.keys("3");
        assert_eq!((t.s.active, t.s.picking_window), (order[2], false));
        // Any other key puts the numbers away and does nothing else.
        t.keys(" wx");
        assert_eq!(
            (t.s.active, t.s.ed.buf.text().as_str()),
            (order[2], "1\n2\n3")
        );
    }

    #[test]
    fn space_w_c_lays_out_a_grid() {
        let mut t = T::open(Some("a"), Some("1\n"));
        let rects = |t: &T| {
            let mut r: Vec<_> =
                t.s.tree
                    .layout(t.s.area)
                    .into_iter()
                    .map(|p| p.rect)
                    .collect();
            r.sort_by_key(|r| (r.y, r.x));
            r
        };
        // 4: two rows of two, the same size near enough.
        t.keys(" wc4");
        let r = rects(&t);
        assert_eq!(r.len(), 4);
        assert_eq!((r[0].y, r[1].y, r[2].y, r[3].y), (0, 0, r[2].y, r[2].y));
        assert!(r[2].y > 0 && r[0].width.abs_diff(r[1].width) <= 1);
        assert!(r[0].height.abs_diff(r[2].height) <= 1);
        // It starts over: 3 is two above one, then 9 is three by three.
        t.keys(" wc3");
        let r = rects(&t);
        assert_eq!(r.len(), 3);
        assert_eq!(r[2].width, t.s.area.width);
        t.keys(" wc9");
        let r = rects(&t);
        assert_eq!(r.len(), 9);
        let widths: Vec<u16> = r.iter().map(|r| r.width).collect();
        assert!(
            widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1,
            "{widths:?}"
        );
        let tops: std::collections::BTreeSet<u16> = r.iter().map(|r| r.y).collect();
        assert_eq!(tops.len(), 3);
        // Anything but a digit: nothing changes.
        t.keys(" wcx");
        assert_eq!(rects(&t).len(), 9);
        assert_eq!(t.s.ed.buf.text(), "1");
    }

    #[test]
    fn ctrl_w_moves_keep_each_cursor() {
        let mut t = T::open(Some("a"), Some("1\n2\n3\n4\n5\n"));
        t.keys("3G<C-w>s5G<C-w>j");
        assert_eq!(t.s.ed.cur.line, 2);
        t.keys("<C-w>k");
        assert_eq!(t.s.ed.cur.line, 4);
        t.keys("<C-w><C-j>");
        assert_eq!(t.s.ed.cur.line, 2);
        t.keys("<C-w>p");
        assert_eq!(t.s.ed.cur.line, 4);
    }

    #[test]
    fn vsplit_file_and_window_numbers() {
        let mut t = T::open(Some("a"), Some("a\n"));
        let b = t.dir.path().join("b");
        fs::write(&b, "b\n").unwrap();
        t.keys(&format!(":vsplit {}<Enter>", b.display()));
        assert_eq!(t.s.ed.buf.text(), "b");
        t.keys(":wincmd l<Enter>");
        assert_eq!(t.s.ed.buf.text(), "a");
        t.keys("1<C-w>w");
        assert_eq!(t.s.ed.buf.text(), "b");
    }

    #[test]
    fn split_count_sets_height() {
        let mut t = T::open(Some("a"), Some("a\n"));
        t.keys(":5split<Enter>");
        let p = t.s.tree.layout(t.s.area);
        assert_eq!(p[0].rect.height, 6); // 5 text rows + status line
    }

    #[test]
    fn split_too_small() {
        let mut t = T::open(Some("a"), Some("a\n"));
        t.s.area = ratatui::layout::Rect::new(0, 0, 80, 3);
        t.keys(":split<Enter>");
        // fred marks every error with "? ".
        assert_eq!(t.msg(), "? E36: Not enough room");
        assert_eq!(t.s.windows.len(), 1);
    }

    #[test]
    fn new_and_sbuffer() {
        let mut t = T::open(Some("a"), Some("a\n"));
        t.keys(":new<Enter>");
        assert_eq!(t.s.ed.buf.text(), "");
        t.keys(":sbuffer 1<Enter>");
        assert_eq!(t.s.ed.buf.text(), "a");
        assert_eq!(t.s.windows.len(), 3);
    }

    #[test]
    fn scroll_survives_buffer_switch() {
        let mut t = T::open(Some("a"), Some(&"x\n".repeat(200)));
        t.keys("150G");
        t.s.win_mut().view.top = 140;
        t.keys(":e b<Enter>:b 1<Enter>");
        assert_eq!(t.s.win().view.top, 140);
        assert_eq!(t.s.ed.cur.line, 149);
    }

    #[test]
    fn one_window_at_start() {
        let t = T::open(Some("a"), Some("1\n"));
        assert_eq!(t.s.tree.order(), vec![t.s.active]);
        assert_eq!(t.s.win().buf, 0);
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
    fn auto_read_reloads_unmodified_buffers_only() {
        let mut t = T::open(Some("f"), Some("one\ntwo\n"));
        assert!(!t.s.auto_read());
        t.keys("j");
        fs::write(t.dir.path().join("f"), "uno\ndos\ntres\n").unwrap();
        assert!(t.s.auto_read());
        assert_eq!(t.s.ed.buf.text(), "uno\ndos\ntres");
        assert_eq!(t.s.ed.cur.line, 1);
        assert!(!t.s.auto_read());
        // Unsaved changes win: `:w` reports the conflict instead.
        t.keys("x");
        fs::write(t.dir.path().join("f"), "theirs\n").unwrap();
        assert!(!t.s.auto_read());
        assert_eq!(t.s.ed.buf.text(), "uno\nos\ntres");
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
        // Written in the background.
        t.s.wait_swap();
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
        crate::swap::write(
            &sp,
            Some(&t.dir.path().join("f")),
            &crate::buffer::Buffer::from_text("theirs").snapshot(),
        )
        .unwrap();
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
        crate::swap::write(
            &sp,
            Some(&b),
            &crate::buffer::Buffer::from_text("bbb recovered\n").snapshot(),
        )
        .unwrap();
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
    fn dired_opens_on_the_file_you_came_from() {
        let mut t = T::open(Some("m.txt"), Some("hi\n"));
        let dir = t.dir.path().to_path_buf();
        for f in ["a.txt", "z.txt"] {
            fs::write(dir.join(f), "x").unwrap();
        }
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/inner.txt"), "x").unwrap();
        let here = |t: &T| crate::dired::current(&t.s.ed).unwrap_or_default();
        t.keys(&format!(":e {}<Enter>", dir.display()));
        assert_eq!(here(&t), "m.txt");
        // From deeper down: the subdirectory that leads there.
        t.keys(&format!(
            ":e {}<Enter>",
            dir.join("sub/inner.txt").display()
        ));
        t.keys(&format!(":e {}<Enter>", dir.display()));
        assert_eq!(here(&t), "sub");
        // From elsewhere: wherever the cursor was.
        t.keys("k");
        let was = here(&t);
        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("o"), "x").unwrap();
        t.keys(&format!(":e {}<Enter>", other.path().join("o").display()));
        t.keys(&format!(":e {}<Enter>", dir.display()));
        assert_eq!(here(&t), was);
    }

    #[test]
    fn occur_starts_on_m_s_o_and_on_a_selection() {
        let mut t = T::open(Some("f.txt"), Some("foo\nbar\nfoo\n"));
        t.keys("<M-s>ofoo<Enter>");
        assert_eq!(buf_name(&t.s.ed), "[*Occur*]");
        assert_eq!(t.s.ed.line_count(), 3);
        // From Visual: only the selected lines.
        t.keys(":b 1<Enter>jVj<M-s>ofoo<Enter>");
        assert_eq!(
            t.s.ed.buf.line(0),
            "1 match for \"foo\" in buffer: f.txt within region: 2-3"
        );
        // Space o stays the Org prefix.
        t.keys(":b 1<Enter> o");
        assert_eq!(t.s.ed.vim.pending, [Key::ch(' '), Key::ch('o')]);
    }

    #[test]
    fn occur_lists_visits_reverts_and_edits_back() {
        let mut t = T::open(Some("f.txt"), Some("let foo = 1;\nbar\nfoo(foo);\n"));
        t.keys(":occur foo<Enter>");
        let lines: Vec<String> = (0..t.s.ed.line_count())
            .map(|l| t.s.ed.buf.line(l))
            .collect();
        assert_eq!(
            lines[0],
            "3 matches in 2 lines for \"foo\" in buffer: f.txt"
        );
        assert_eq!(&lines[1..], ["      1:let foo = 1;", "      3:foo(foo);"]);
        assert_eq!(buf_name(&t.s.ed), "[*Occur*]");
        // Read-only until i; Enter goes to the match, n finds the next.
        t.keys("dd");
        assert_eq!(t.s.ed.line_count(), 3);
        t.keys("j<Enter>");
        assert!(t.s.ed.path.as_deref().is_some_and(|p| p.ends_with("f.txt")));
        assert_eq!((t.s.ed.cur.line, t.s.ed.cur.byte), (2, 0));
        t.keys("n");
        assert_eq!((t.s.ed.cur.line, t.s.ed.cur.byte), (2, 4));
        // A second :occur reuses the buffer; gr sees the file's new text.
        t.keys("ggOfoo<Esc>:occur foo<Enter>");
        assert_eq!(t.s.bufs.len(), 2);
        assert_eq!(t.s.ed.buf.line(1), "      1:foo");
        // occur-edit: i, change a line, :w puts it in the file's buffer.
        t.keys("i");
        t.keys("jA // hi<Esc>:w<Enter>");
        assert!(t.msg().contains("changed 1 line"), "{}", t.msg());
        assert!(!t.s.swap_path.exists());
        t.keys(":b 1<Enter>");
        assert_eq!(t.s.ed.buf.line(1), "let foo = 1; // hi");
        assert!(t.s.ed.buf.modified);
        t.keys("u");
        assert_eq!(t.s.ed.buf.line(1), "let foo = 1;");
        t.keys(":occur nothing<Enter>");
        assert!(t.msg().contains("no matches"), "{}", t.msg());
    }

    #[test]
    fn dired_buffers() {
        let mut t = T::open(Some("f.txt"), Some("hi\n"));
        let d = t.dir.path().join("d");
        fs::create_dir(&d).unwrap();
        fs::write(d.join("old.txt"), "x").unwrap();
        // Space - lists the file's directory, cursor on the file.
        t.keys(" -");
        assert!(t.s.ed.dired.is_some());
        assert!(t.s.ed.buf.line(t.s.ed.cur.line).ends_with("f.txt"));
        // :e on a directory too; wdired renames on :w.
        t.keys(&format!(":e {}<Enter>", d.display()));
        assert!(t.s.ed.buf.line(t.s.ed.cur.line).ends_with("old.txt"));
        // Space j finds files in the listed directory, not its parent.
        t.keys(" j");
        let Mode::Pick(p) = &t.s.ed.mode else {
            panic!("no picker")
        };
        let listed = &t.s.ed.dired.as_ref().unwrap().dir;
        assert_eq!(p.query.text, crate::pick::browse::show(listed));
        t.keys("<Esc>");
        t.keys("icwnew<Esc>:w<Enter>");
        assert!(d.join("new.txt").exists(), "{:?}", t.s.ed.msg);
        assert!(!t.s.ed.buf.modified);
        // A listing has no swap file, even while names are edited.
        t.keys("icwzzz<Esc>");
        t.s.maybe_swap(Instant::now() + Duration::from_secs(5));
        t.s.wait_swap();
        assert!(!t.s.swap_path.exists());
        t.keys("gr");
        // Enter on a file opens it as a buffer.
        t.keys("<Enter>");
        assert_eq!(
            t.s.ed.path.as_deref().and_then(Path::file_name).unwrap(),
            "new.txt"
        );
    }

    #[test]
    fn swap_written_after_many_edits() {
        let mut t = T::open(Some("f"), Some("a\n"));
        t.keys(&"ix<Esc>".repeat(200));
        t.s.maybe_swap(Instant::now());
        t.s.wait_swap();
        assert!(t.s.swap_path.exists());
    }

    #[test]
    fn stale_swap_is_offered_and_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, "old\n").unwrap();
        let swap_dir = dir.path().join("swap");
        let sp = crate::swap::swap_path_in(&swap_dir, Some(&p));
        crate::swap::write(
            &sp,
            Some(&p),
            &crate::buffer::Buffer::from_text("recovered\r\n").snapshot(),
        )
        .unwrap();
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
    fn opening_the_working_directory_lists_it() {
        let mut t = T::open(Some("a"), Some("one\n"));
        let cwd = std::env::current_dir().unwrap();
        t.s.perform(ExEffect::Open {
            path: cwd.clone(),
            line: 0,
            col: 0,
            pattern: None,
        });
        let d = t.s.ed.dired.as_ref().expect("dired, not a new file");
        assert_eq!(d.dir, fs::canonicalize(cwd).unwrap());
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
        // Down to b's match: Enter goes to that buffer and line.
        t.keys("<Down><Enter>");
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
        t.keys(":ls<Enter><Down><Enter>");
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
    fn the_claude_chat_knows_the_open_files() {
        let mut t = T::open(Some("a"), Some("1\n2\n3\n"));
        fs::write(t.dir.path().join("b"), "x\n").unwrap();
        let d = std::path::absolute(t.dir.path()).unwrap();
        t.keys(&format!("jj:e {}/b<Enter>x:claude<Enter>", d.display()));
        let files = &t.s.ed.claude.as_ref().unwrap().files;
        let want = format!("{0}/b:1 unsaved\n{0}/a:3\n", d.display());
        assert_eq!(files, &want);
    }

    #[test]
    fn claude_is_answered_in_normal_mode_but_not_by_its_own_typing() {
        let mut t = T::open(Some("a"), Some("1\n"));
        // The chat is in Insert mode at its prompt, as after sending one.
        t.keys(":claude<Enter>");
        assert_eq!(t.s.ed.mode, Mode::Insert);
        let (tx, rx) = std::sync::mpsc::channel();
        let args = serde_json::json!({ "tool_name": "Bash", "input": { "command": "ls" } });
        crate::claude::ask(&mut t.s.ed, &args, tx);
        // Text being typed is not an answer: Esc first.
        t.keys("y");
        assert!(rx.try_recv().is_err());
        t.keys("<Esc>y");
        assert!(rx.try_recv().unwrap().unwrap().contains("allow"));
        assert!(
            t.s.ed.buf.text().contains("  ⎿  Allowed"),
            "{}",
            t.s.ed.buf.text()
        );
        let (tx, rx) = std::sync::mpsc::channel();
        crate::claude::ask(&mut t.s.ed, &args, tx);
        let (ktx, _krx) = std::sync::mpsc::channel();
        t.s.typing = Some(Typing {
            keys: "n".chars().map(Key::ch).collect(),
            per_frame: 1,
            reply: ktx,
        });
        t.s.tick_typing();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn claude_shows_it_is_working_and_comes_back_to_the_chat_when_done() {
        let mut t = T::open(Some("a"), Some("1\n"));
        t.s.cfg.claude_command =
            r#"sleep 0.2; echo '{"type":"result","duration_ms":1000,"total_cost_usd":0}'"#.into();
        t.s.cfg.claude_permissions = String::new();
        t.keys(":claude<Enter>hi<Enter><Esc>:b 1<Enter>");
        t.s.tick_claude();
        assert!(t.s.ed.claude.is_none());
        assert!(
            t.s.ed
                .claude_working
                .as_deref()
                .unwrap()
                .contains("Claude is working… 0s")
        );
        let start = std::time::Instant::now();
        while t.s.ed.claude.is_none() && start.elapsed().as_secs() < 5 {
            t.s.tick_claude();
        }
        assert!(t.s.ed.claude.is_some());
        assert_eq!(t.s.ed.claude_working, None);
        assert_eq!(t.s.ed.msg.as_ref().unwrap().0, "done in 1s · $0.00");
    }

    #[test]
    fn the_chat_opens_in_a_pane_below_and_jj_leaves_insert() {
        let mut t = T::open(Some("a"), Some("1\n"));
        t.s.cfg.claude_command = "true".into();
        t.s.cfg.claude_permissions = String::new();
        let file_win = t.s.active;
        t.keys(" cc");
        assert_eq!(t.s.windows.len(), 2);
        assert!(t.s.ed.claude.is_some());
        let chat_win = t.s.active;
        // Below the file.
        let rect = |t: &T, id| {
            t.s.tree
                .layout(t.s.area)
                .into_iter()
                .find(|p| p.id == id)
                .unwrap()
                .rect
        };
        assert!(rect(&t, chat_win).y > rect(&t, file_win).y);
        assert_eq!(t.s.ed.mode, Mode::Insert);
        t.keys("jj");
        assert_eq!(t.s.ed.mode, Mode::Normal);
        assert_eq!(t.s.ed.buf.text(), "> ");
        // From the file again: back to the chat's pane, not a third one.
        t.s.focus(file_win);
        t.keys(" cc");
        assert_eq!((t.s.windows.len(), t.s.active), (2, chat_win));
    }

    fn claude_settle(t: &mut T) {
        let start = std::time::Instant::now();
        while crate::claude::live(&t.s.ed) && start.elapsed().as_secs() < 5 {
            t.s.tick_claude();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn claude_reports_back_after_the_turn_when_a_background_command_ends() {
        let mut t = T::open(Some("a"), Some("1\n"));
        let r = r#"echo '{"type":"result","duration_ms":1000,"total_cost_usd":0}'"#;
        let text = r#"echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"sync finished"}}}'"#;
        t.s.cfg.claude_command = format!("{r}; sleep 0.3; {text}; {r}");
        t.s.cfg.claude_permissions = String::new();
        t.keys(":claude<Enter>hi<Enter>");
        claude_settle(&mut t);
        assert!(
            t.s.ed
                .buf
                .slice(0, t.s.ed.buf.len_chars())
                .contains("sync finished")
        );
        assert!(
            !t.s.ed
                .buf
                .slice(0, t.s.ed.buf.len_chars())
                .contains("exited without answering")
        );
    }

    #[test]
    fn claude_takes_each_prompt_in_the_same_process() {
        let mut t = T::open(Some("a"), Some("1\n"));
        use std::os::unix::fs::PermissionsExt;
        let claude = t.dir.path().join("claude");
        // Answers each stdin line with its own pid, and how many it has read.
        fs::write(
            &claude,
            "#!/bin/sh\nn=0\nwhile read l; do n=$((n+1))\n\
             echo '{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"'$$-$n'\"}}}'\n\
             echo '{\"type\":\"result\",\"duration_ms\":0,\"total_cost_usd\":0}'\ndone\n",
        )
        .unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        t.s.cfg.claude_command = claude.display().to_string();
        t.s.cfg.claude_permissions = String::new();
        let turn = |t: &mut T, keys: &str| {
            t.keys(keys);
            let start = std::time::Instant::now();
            while crate::claude::busy(&t.s.ed) && start.elapsed().as_secs() < 5 {
                t.s.tick_claude();
            }
        };
        turn(&mut t, ":claude<Enter>one<Enter>");
        assert!(crate::claude::live(&t.s.ed));
        turn(&mut t, "two<Enter>");
        let chat = t.s.ed.buf.slice(0, t.s.ed.buf.len_chars());
        let pid = chat
            .split("-1")
            .next()
            .unwrap()
            .rsplit(|c: char| !c.is_ascii_digit())
            .next();
        assert!(chat.contains(&format!("{}-2", pid.unwrap())), "{chat}");
    }

    #[test]
    fn claude_drives_fred_through_the_remote_tools() {
        let mut t = T::open(Some("a"), Some("1\n2\n3\n"));
        fs::write(t.dir.path().join("b"), "x\ny\n").unwrap();
        let d = std::path::absolute(t.dir.path())
            .unwrap()
            .display()
            .to_string();
        let keys = serde_json::json!({ "keys": format!("jdd:e {d}/b<CR>") });
        let state = call(&mut t.s, "fred_keys", keys).unwrap();
        assert!(
            state.starts_with(&format!(
                "mode: NORMAL\nbuffer 2: {d}/b, line 1 col 1 of 2 lines\n"
            )),
            "{state}"
        );
        assert!(
            state.ends_with(&format!(
                "*2 {d}/b:1\n 1 {d}/a:2, unsaved\nbuffer 2: lines 1-2 of 2\n1\tx\n2\ty\n"
            )),
            "{state}"
        );
        let mut read = |args| call(&mut t.s, "fred_read", args);
        assert_eq!(
            read(serde_json::json!({ "buffer": 1 })).unwrap(),
            "buffer 1: lines 1-2 of 2\n1\t1\n2\t3\n"
        );
        assert_eq!(
            read(serde_json::json!({ "start": 2, "end": 9 })).unwrap(),
            "buffer 2: lines 2-2 of 2\n2\ty\n"
        );
        assert!(read(serde_json::json!({ "buffer": 3 })).is_err());
        assert!(read(serde_json::json!({ "start": 5 })).is_err());
        assert!(call(&mut t.s, "nope", serde_json::json!({})).is_err());
    }

    /// A tool call, typed through to the end as the event loop would.
    fn call(s: &mut Session, tool: &str, args: serde_json::Value) -> Result<String, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        s.remote(tool, &args, tx);
        while s.tick_typing() {}
        rx.recv().unwrap()
    }

    #[test]
    fn claude_types_a_frame_at_a_time_and_text_goes_in_as_sent() {
        let mut t = T::open(Some("a"), Some("fn x() {\n    w();\n}\n"));
        let (tx, rx) = std::sync::mpsc::channel();
        let keys = serde_json::json!({ "keys": "<Esc>jo    y();<CR>    z();<Esc>" });
        t.s.remote("fred_keys", &keys, tx);
        // One key a frame: after three, only the `o` has landed.
        for _ in 0..3 {
            t.s.tick_typing();
        }
        assert_eq!(t.s.ed.buf.text(), "fn x() {\n    w();\n\n}");
        assert!(rx.try_recv().is_err());
        while t.s.tick_typing() {}
        assert!(rx.recv().unwrap().unwrap().starts_with("mode: NORMAL"));
        // The indent as sent, not doubled by autoindent.
        assert_eq!(
            t.s.ed.buf.text(),
            "fn x() {\n    w();\n    y();\n    z();\n}"
        );
        t.keys("u");
        assert_eq!(t.s.ed.buf.text(), "fn x() {\n    w();\n}");
    }

    #[test]
    fn ctrl_c_stops_claude_typing() {
        let mut t = T::open(Some("a"), Some("1\n"));
        let (tx, rx) = std::sync::mpsc::channel();
        t.s.remote("fred_keys", &serde_json::json!({ "keys": "xxxx" }), tx);
        t.s.tick_typing();
        t.s.stop_typing();
        assert!(!t.s.tick_typing());
        assert!(rx.recv().unwrap().unwrap_err().contains("3 keys short"));
    }

    #[test]
    fn shell_commands_typed_by_claude_are_refused() {
        let mut t = T::open(Some("a"), Some("1\n"));
        let (tx, rx) = std::sync::mpsc::channel();
        let keys = serde_json::json!({ "keys": ":!touch x<CR>:r !echo hi<CR>" });
        t.s.remote("fred_keys", &keys, tx);
        while t.s.tick_typing() {}
        assert!(
            rx.recv()
                .unwrap()
                .unwrap()
                .contains("refuses shell commands")
        );
        assert!(t.s.pending_shell.is_none());
        assert_eq!(t.s.ed.buf.text(), "1");
        // The user's own are run.
        t.keys(":r !echo hi<Enter>");
        assert_eq!(t.s.ed.buf.text(), "1\nhi");
    }

    #[test]
    fn claude_cannot_type_into_the_chat_buffer() {
        let mut t = T::open(Some("a"), Some("1\n"));
        t.s.ed.claude = Some(Box::new(crate::claude::Chat::new("true")));
        let keys = |k: &str| serde_json::json!({ "keys": k });
        // `o` would type into the prompt; the rest is never typed.
        let err = call(&mut t.s, "fred_keys", keys("<Esc>ohi<CR><Esc>")).unwrap_err();
        assert!(err.contains("[*claude*]"), "{err}");
        assert_eq!(t.s.ed.buf.text(), "1");
        // A `:` command that edits it is undone.
        let err = call(&mut t.s, "fred_keys", keys(":s/1/2/<CR>")).unwrap_err();
        assert!(err.contains("undid"), "{err}");
        assert_eq!(t.s.ed.buf.text(), "1");
    }

    #[test]
    fn fred_read_opens_a_path_in_front_of_you() {
        let mut t = T::open(Some("a"), Some("1\n"));
        fs::write(t.dir.path().join("b"), "x\ny\nz\n").unwrap();
        let b = t.dir.path().join("b").display().to_string();
        let got = call(
            &mut t.s,
            "fred_read",
            serde_json::json!({ "path": b, "start": 2 }),
        );
        assert_eq!(got.unwrap(), "buffer 2: lines 2-3 of 3\n2\ty\n3\tz\n");
        assert!(t.s.is_own_file(Path::new(&b)));
        assert_eq!(t.s.ed.cur.line, 1);
        let missing = t.dir.path().join("nope").display().to_string();
        assert!(
            call(
                &mut t.s,
                "fred_read",
                serde_json::json!({ "path": missing })
            )
            .is_err()
        );
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
    fn magit_repo(t: &T) {
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(t.dir.path())
            .args(["init", "-q", "-b", "main"])
            .status()
            .unwrap();
        assert!(st.success());
    }
    fn magit_wait(t: &mut T) {
        for _ in 0..500 {
            t.s.tick_magit();
            if t.s.ed.magit.is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("status did not open: {:?}", t.s.ed.msg);
    }
    #[test]
    fn magit_preserves_unsaved_source_buffer() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        t.keys("iunsaved <Esc> ms");
        magit_wait(&mut t);
        assert!(t.s.no_swap);
        assert!(!t.s.ed.buf.modified);
        t.keys("q");
        assert_eq!(t.s.ed.buf.line(0), "unsaved original");
        assert!(t.s.ed.buf.modified);
        assert_eq!(t.file("f.txt"), "original\n");
    }
    #[test]
    fn magit_views_are_readonly_and_have_no_swap() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        t.keys(" ms");
        magit_wait(&mut t);
        let before = t.s.ed.buf.to_bytes();
        t.keys("idd<Esc>:s/Head/Broken/<Enter>:w<Enter>");
        assert_eq!(t.s.ed.buf.to_bytes(), before);
        assert!(!t.s.swap_path.exists());
    }
    #[test]
    fn late_magit_result_does_not_replace_another_buffer() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        t.keys(" ms");
        t.keys(":e another<Enter>");
        for _ in 0..100 {
            t.s.tick_magit();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(t.s.ed.magit.is_none());
        assert!(t.s.ed.path.as_ref().unwrap().ends_with("another"));
    }

    #[test]
    fn magit_visual_edits_are_blocked() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        t.keys(" ms");
        magit_wait(&mut t);
        let before = t.s.ed.buf.to_bytes();
        t.keys("Vd");
        assert_eq!(t.s.ed.buf.to_bytes(), before);
    }
    #[test]
    fn magit_refresh_retains_expansion_and_selection() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys(" ms");
        magit_wait(&mut t);
        let row = t.s.ed.magit.as_ref().unwrap().rows.iter().position(|r| matches!(&r.action,Some(crate::magit::RowAction::File(path,..)) if path == Path::new("f.txt"))).unwrap();
        t.s.ed.set_cursor(row, 0);
        t.keys("<Tab>");
        for _ in 0..100 {
            t.s.tick_magit();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(t.s.ed.buf.to_bytes().windows(9).any(|w| w == b"+original"));
        let selected = t.s.ed.magit.as_ref().unwrap().action_at(t.s.ed.cur.line);
        t.keys(" ms");
        for _ in 0..100 {
            t.s.tick_magit();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(t.s.ed.buf.to_bytes().windows(9).any(|w| w == b"+original"));
        assert_eq!(
            t.s.ed.magit.as_ref().unwrap().action_at(t.s.ed.cur.line),
            selected
        );
    }

    fn magit_settle(t: &mut T) {
        for _ in 0..500 {
            t.s.tick_magit();
            if t.s.magit_job.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("Git job did not finish");
    }
    #[test]
    fn magit_commit_draft_survives_failed_hook_and_resumes() {
        use std::os::unix::fs::PermissionsExt;
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some());
        let path = t.s.ed.path.clone().unwrap();
        t.keys("ifirst message<Esc>:w<Enter>");
        assert_eq!(fs::read_to_string(&path).unwrap(), "first message\n");
        let hook = t.dir.path().join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        t.keys(" mcc");
        let inv = t.s.pending_git.take().unwrap();
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_err());
        t.s.finish_git(inv, result);
        assert_eq!(t.s.ed.buf.line(0), "first message");
        assert_eq!(fs::read_to_string(&path).unwrap(), "first message\n");
        fs::remove_file(hook).unwrap();
        t.keys(" mcc");
        let inv = t.s.pending_git.take().unwrap();
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok());
        t.s.finish_git(inv, result);
        magit_settle(&mut t);
        assert!(t.s.ed.magit.is_some());
        assert_eq!(fs::read(&path).unwrap(), b"");
        fs::write(t.dir.path().join("f.txt"), "next\n").unwrap();
        // magit-commit-assert: a draft needs something to commit.
        crate::magit::repo::Repo::discover(t.dir.path())
            .unwrap()
            .stage_file(Path::new("f.txt"))
            .unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        t.keys("inext message<Esc>:w<Enter>");
        assert_eq!(fs::read_to_string(&path).unwrap(), "next message\n");
    }

    #[test]
    fn magit_file_log_preserves_source_and_refreshes_its_filter() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "file initial"]).unwrap();
        fs::write(t.dir.path().join("other"), "unrelated\n").unwrap();
        repo.stage_file(Path::new("other")).unwrap();
        repo.read(&["commit", "-qm", "unrelated"]).unwrap();
        t.keys("iunsaved <Esc> mL");
        magit_settle(&mut t);
        assert!(t.s.ed.magit.is_some(), "{}", t.msg());
        assert!(t.s.ed.buf.text().contains("file initial"));
        assert!(!t.s.ed.buf.text().contains("unrelated"));
        fs::write(t.dir.path().join("f.txt"), "changed\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "file later"]).unwrap();
        t.keys("gr");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("file later"));
        assert!(!t.s.ed.buf.text().contains("unrelated"));
        t.keys("j<Enter>");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.line(1).starts_with("Author:     "),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys("qq");
        assert!(t.s.ed.magit.is_none());
        assert_eq!(t.s.ed.buf.line(0), "unsaved original");
    }

    #[test]
    fn magit_log_menu_inherits_file_filter_and_follow_option() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "before rename"]).unwrap();
        repo.read(&["mv", "f.txt", "renamed"]).unwrap();
        repo.read(&["commit", "-qm", "rename"]).unwrap();
        fs::write(t.dir.path().join("other"), "unrelated\n").unwrap();
        repo.stage_file(Path::new("other")).unwrap();
        repo.read(&["commit", "-qm", "unrelated"]).unwrap();
        t.keys(&format!(
            ":e {}<Enter> ml-f<Esc> mL",
            t.dir.path().join("renamed").display()
        ));
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.text().contains("before rename"),
            "{}",
            t.s.ed.buf.text()
        );
        assert!(!t.s.ed.buf.text().contains("unrelated"));
        t.keys("gr");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("before rename"));
        t.keys(" mll");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.text().contains("before rename"),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys(" ml-fl");
        magit_settle(&mut t);
        assert!(!t.s.ed.buf.text().contains("before rename"));
        // h from a file log keeps its file (buffer arguments); from the
        // source buffer it logs all of HEAD.
        for _ in 0..4 {
            if t.s.ed.magit.is_some() {
                t.keys("q");
            }
        }
        assert!(t.s.ed.magit.is_none());
        // magit-log-head is on level 0 of the menu: by name.
        t.keys(":Magit magit-log-head<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("unrelated"));
    }

    #[test]
    fn magit_log_menu_reads_option_values_and_limits_in_the_buffer() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        for i in 0..3 {
            fs::write(t.dir.path().join("f.txt"), format!("{i}\n")).unwrap();
            repo.stage_file(Path::new("f.txt")).unwrap();
            repo.read(&["commit", "-qm", &format!("commit {i}")])
                .unwrap();
        }
        // The seeded -n256 is unset by its key, then read from the minibuffer.
        t.keys(" ml-n-n2<Enter>-Fcommit<Enter>l");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(
            text.contains("commit 2") && text.contains("commit 1"),
            "{text}"
        );
        assert!(!text.contains("commit 0"), "{text}");
        assert!(
            matches!(&t.s.ed.magit.as_ref().unwrap().kind, crate::magit::Kind::Log(_, args) if args.iter().any(|a| a == "--grep=commit")),
            "{text}"
        );
        t.keys("+");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("commit 0"));
        t.keys("=");
        magit_settle(&mut t);
        let crate::magit::Kind::Log(_, args) = &t.s.ed.magit.as_ref().unwrap().kind else {
            panic!()
        };
        assert!(crate::magit::log::limit(args).is_none(), "{args:?}");
        // C-c C-n moves to the parent commit's row.
        let row = |t: &T, c: &str| t.s.ed.buf.text().lines().position(|l| l.contains(c));
        t.keys(&format!(":{}<Enter>", row(&t, "commit 2").unwrap() + 1));
        t.keys("<C-c><C-n>");
        assert_eq!(Some(t.s.ed.cur.line), row(&t, "commit 1"));
        // + with no limit sets the default limit (magit-log-set-commit-limit).
        t.keys("+");
        magit_settle(&mut t);
        let crate::magit::Kind::Log(_, args) = &t.s.ed.magit.as_ref().unwrap().kind else {
            panic!()
        };
        assert_eq!(crate::magit::log::limit(args), Some(256));
        t.keys("=");
        magit_settle(&mut t);
        // The menu opened from the log buffer shows the buffer's arguments,
        // and values typed into it stay (not reseeded from the buffer).
        t.keys(" ml-Afred<Enter>");
        assert_eq!(
            t.s.ed
                .magit_values
                .get(&('l', "--author="))
                .map(String::as_str),
            Some("fred")
        );
        t.keys("-A");
        assert!(!t.s.ed.magit_values.contains_key(&('l', "-n")));
        assert_eq!(
            t.s.ed
                .magit_values
                .get(&('l', "--grep="))
                .map(String::as_str),
            Some("commit")
        );
    }

    #[test]
    fn magit_stage_and_unstage_a_whole_section_from_its_heading() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        fs::write(t.dir.path().join("g.txt"), "g\n").unwrap();
        repo.read(&["add", "."]).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        fs::write(t.dir.path().join("g.txt"), "h\n").unwrap();
        t.keys(" ms");
        magit_settle(&mut t);
        let heading = |t: &T, name: &str| {
            let text = t.s.ed.buf.text();
            text.lines()
                .position(|l| l.starts_with(name))
                .unwrap_or_else(|| panic!("{name}: {text}"))
        };
        t.keys(&format!(":{}<Enter>s", heading(&t, "Unstaged changes") + 1));
        magit_settle(&mut t);
        let staged = repo.read(&["diff", "--cached", "--name-only"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&staged), "f.txt\ng.txt\n");
        t.keys(&format!(":{}<Enter>u", heading(&t, "Staged changes") + 1));
        magit_settle(&mut t);
        assert!(
            repo.read(&["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn magit_menus_take_key_sequences_and_save_their_arguments() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "first"]).unwrap();
        // magit-branch-configure's "a m" is two keys.
        t.keys(":Magit magit-branch-configure<Enter>am");
        magit_settle(&mut t);
        let merge = repo.read(&["config", "branch.autoSetupMerge"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&merge).trim(), "always");
        // C-x C-s saves the log menu's arguments; a later session starts
        // from them instead of the defaults.
        t.keys(" ml-n");
        t.keys("<Esc>");
        t.keys(" ml<C-x><C-s>");
        assert_eq!(t.s.ed.msg.as_ref().map(|m| m.0.as_str()), Some("Saved"));
        let saved = crate::magit::menu_arguments(&t.s.ed, 'l');
        t.s.ed.magit_options.clear();
        t.s.ed.magit_values.clear();
        t.keys(" ml<Esc>");
        assert_eq!(crate::magit::menu_arguments(&t.s.ed, 'l'), saved);
        // magit-revision-jump replaces the status jump menu in a commit.
        t.keys(" mdc");
        magit_settle(&mut t);
        t.keys("HEAD<Enter>");
        magit_settle(&mut t);
        t.keys(":Magit magit-status-jump<Enter>");
        assert_eq!(
            t.s.ed.magit_menu,
            Some('^'),
            "{:?} {:?}",
            t.s.ed.magit.as_ref().map(|v| &v.kind),
            t.s.ed.msg
        );
    }

    #[test]
    fn magit_prompts_keep_a_history_per_question() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        t.keys(" ml-Ffirst<Enter><Esc>");
        t.keys(" ml-Fsecond<Enter><Esc>");
        // The next --grep= reading offers them, newest first (M-p, M-n).
        t.keys(" ml-F<M-p>");
        assert_eq!(t.cmdline_text(), "second");
        t.keys("<M-p>");
        assert_eq!(t.cmdline_text(), "first");
        t.keys("<M-n><M-n>");
        assert_eq!(t.cmdline_text(), "");
        t.keys("<Esc>");
    }

    #[test]
    fn magit_menu_values_last_one_invocation_unless_set() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let args = |t: &T| crate::magit::menu_arguments(&t.s.ed, 'p');
        // A toggled argument is gone when the menu opens again.
        t.keys(" mp-F");
        assert!(args(&t).contains(&"--force".to_owned()));
        t.keys("<Esc> mp");
        assert!(!args(&t).contains(&"--force".to_owned()));
        // C-x s sets it for the session.
        t.keys("-F<C-x>s");
        t.keys(" mp");
        assert!(args(&t).contains(&"--force".to_owned()));
        t.keys("<Esc>");
        // The commit menu's :value (--verbose) is where it starts.
        t.keys(" mc");
        assert_eq!(crate::magit::menu_arguments(&t.s.ed, 'C'), ["--verbose"]);
    }

    #[test]
    fn magit_options_configured_per_test_reach_git_workers() {
        // magit-status-file-list-limit is read where the status is built,
        // on the Git worker thread.
        crate::magit::options::set_for_test(
            "magit-status-file-list-limit = 1\nmagit-section-initial-visibility-alist = { untracked = \"show\" }",
        );
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        for f in ["a", "b", "c"] {
            fs::write(t.dir.path().join(f), f).unwrap();
        }
        t.keys(" ms");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        // Shown (not hidden) by the visibility option, one file listed.
        let n: usize = text
            .lines()
            .find_map(|l| {
                l.strip_prefix("Untracked files (")?
                    .strip_suffix(')')?
                    .parse()
                    .ok()
            })
            .expect(&text);
        assert!(
            text.contains(&format!("{} files not listed", n - 1)),
            "{text}"
        );
    }

    #[test]
    fn magit_diff_buffer_folds_files_like_upstream() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        fs::write(t.dir.path().join("gone.txt"), "gone\n").unwrap();
        repo.read(&["add", "."]).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        fs::remove_file(t.dir.path().join("gone.txt")).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        // magit-diff-unstaged: f.txt open, the deleted file folded.
        t.keys(":Magit magit-diff-unstaged<Enter>");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(
            text.contains("modified   f.txt\n@@ -1 +1 @@\n-one\n+two"),
            "{text}"
        );
        let row = text
            .lines()
            .position(|l| l == "deleted    gone.txt…")
            .expect(&text);
        t.keys(&format!(":{}<Enter><Tab>", row + 1));
        assert_eq!(t.s.ed.buf.line(row), "deleted    gone.txt");
        assert_eq!(t.s.ed.buf.line(row + 2), "-gone");
        t.keys("<Tab>");
        assert_eq!(t.s.ed.buf.line(row), "deleted    gone.txt…");
    }

    #[test]
    fn magit_refs_tab_shows_a_refs_cherries() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        repo.read(&["checkout", "-qb", "side"]).unwrap();
        repo.read(&["commit", "-q", "--allow-empty", "-m", "on side"])
            .unwrap();
        repo.read(&["checkout", "-q", "main"]).unwrap();
        t.keys(" myy");
        magit_settle(&mut t);
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l.contains("side "))
                .unwrap();
        assert!(
            t.s.ed.buf.line(row).ends_with('…'),
            "{}",
            t.s.ed.buf.line(row)
        );
        t.keys(&format!(":{}<Enter><Tab>", row + 1));
        magit_settle(&mut t);
        assert!(!t.s.ed.buf.line(row).ends_with('…'));
        let cherry = t.s.ed.buf.line(row + 1);
        assert!(
            cherry.starts_with("+ ") && cherry.ends_with(" on side"),
            "{cherry}"
        );
    }

    #[test]
    fn magit_refs_view_opens_from_dispatch_and_cycles_counts() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "first"]).unwrap();
        repo.read(&["branch", "side"]).unwrap();
        t.keys(" myy");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(text.contains("Branches") && text.contains("side"), "{text}");
        t.keys(" myr");
        magit_settle(&mut t);
        assert!(matches!(
            t.s.ed.magit.as_ref().unwrap().kind,
            crate::magit::Kind::Refs(_, _, crate::magit::refs::Count::Branches)
        ));
        assert!(
            t.s.ed.buf.text().contains("= side"),
            "{}",
            t.s.ed.buf.text()
        );
    }

    #[test]
    fn magit_commit_reuse_and_reedit_message() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "first message"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        // -C commits at once with the reused message.
        t.keys(" mc-CHEAD<Enter>c");
        magit_settle(&mut t);
        let inv = t.s.pending_git.take().unwrap();
        assert!(inv.args.iter().any(|a| a == "--reuse-message=HEAD"));
        repo.run(&inv.args, None).unwrap();
        let log = repo.read(&["log", "--format=%s"]).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&log),
            "first message\nfirst message\n"
        );
        // -c opens the draft with that message.
        fs::write(t.dir.path().join("f.txt"), "three\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        // -C applied to that commit only; it is no longer set.
        assert!(!t.s.ed.magit_values.contains_key(&('C', "--reuse-message=")));
        t.keys(" mc-cHEAD~1<Enter>c");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.text().starts_with("first message"),
            "{}",
            t.s.ed.buf.text()
        );
    }

    #[test]
    fn magit_diff_context_keys_and_refresh_menu() {
        let mut t = T::open(Some("f.txt"), Some("1\n2\n3\n4\n5\n6\n7\n8\n9\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "1\n2\n3\n4\nfive\n6\n7\n8\n9\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "two"]).unwrap();
        t.keys(" mdr");
        magit_settle(&mut t);
        t.keys("HEAD~1..HEAD<Enter>");
        magit_settle(&mut t);
        let args = |t: &T| match &t.s.ed.magit.as_ref().unwrap().kind {
            crate::magit::Kind::Diff(target, args) => (target.clone(), args.clone()),
            other => panic!("{other:?}"),
        };
        // = and + change the context; ~ restores the default.
        t.keys("=");
        magit_settle(&mut t);
        assert!(args(&t).1.contains(&"-U2".to_owned()));
        assert!(
            !t.s.ed.buf.text().contains("\n 2\n"),
            "{}",
            t.s.ed.buf.text()
        );
        for _ in 0..2 {
            t.keys("+");
            magit_settle(&mut t);
        }
        assert!(args(&t).1.contains(&"-U4".to_owned()));
        t.keys("~");
        magit_settle(&mut t);
        assert!(!args(&t).1.iter().any(|a| a.starts_with("-U")));
        // The refresh menu flips the range and switches its type.
        t.keys(" mDf");
        magit_settle(&mut t);
        assert_eq!(
            args(&t).0,
            crate::magit::diff::Target::Range("HEAD...HEAD~1".replace("...", ".."))
        );
        t.keys(" mDr");
        magit_settle(&mut t);
        assert_eq!(
            args(&t).0,
            crate::magit::diff::Target::Range("HEAD...HEAD~1".into())
        );
        // g applies the menu's arguments to the buffer.
        t.keys(" mD-U1<Enter>g");
        magit_settle(&mut t);
        assert!(args(&t).1.contains(&"-U1".to_owned()), "{:?}", args(&t).1);
        // Enter on a line visits that line of the revision's blob; C-j the file.
        t.keys(" mD-U<Enter>");
        t.keys("<Esc> mdr");
        magit_settle(&mut t);
        t.keys("HEAD~1..HEAD<Enter>");
        magit_settle(&mut t);
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l == "+five")
                .unwrap();
        // evil-collection: S-RET visits the blob, RET the worktree file.
        t.keys(&format!(":{}<Enter><S-Enter>", row + 1));
        magit_settle(&mut t);
        let blob = t.s.ed.blob.as_ref().expect("blob buffer");
        assert_eq!(blob.file, Path::new("f.txt"));
        assert_eq!(
            (t.s.ed.cur.line, t.s.ed.buf.line(t.s.ed.cur.line)),
            (4, "five".to_owned())
        );
        t.keys("q");
        magit_settle(&mut t);
        t.keys(&format!(":{}<Enter><Enter>", row + 1));
        magit_settle(&mut t);
        assert!(t.s.ed.blob.is_none() && t.s.ed.magit.is_none());
        assert_eq!(t.s.ed.cur.line, 4);
    }

    #[test]
    fn magit_diff_buffer_file_and_while_committing() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        fs::write(t.dir.path().join("g.txt"), "g\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.stage_file(Path::new("g.txt")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        fs::write(t.dir.path().join("g.txt"), "other\n").unwrap();
        // F d: this file's changes since HEAD only.
        t.keys(" mFd");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(text.contains("+two") && !text.contains("+other"), "{text}");
        // D F toggles the file filter off and back on.
        t.keys(" mDF");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.text().contains("+other"),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys(" mDF");
        magit_settle(&mut t);
        assert!(!t.s.ed.buf.text().contains("+other"));
        // C-c C-d in a draft shows what is staged.
        t.keys("q");
        repo.stage_file(Path::new("g.txt")).unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some());
        t.keys("<C-c><C-d>");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(text.contains("+other") && !text.contains("+two"), "{text}");
    }

    #[test]
    fn magit_discard_reverse_stage_and_unstage_all() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        fs::write(t.dir.path().join("g.txt"), "g\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.stage_file(Path::new("g.txt")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        fs::write(t.dir.path().join("g.txt"), "changed\n").unwrap();
        fs::write(t.dir.path().join("new.txt"), "new\n").unwrap();
        t.keys(" ms");
        magit_settle(&mut t);
        // Untracked files start hidden; show them.
        t.keys("gn<Tab>");
        let goto = |t: &mut T, f: &dyn Fn(&crate::magit::RowAction) -> bool| {
            let row =
                t.s.ed
                    .magit
                    .as_ref()
                    .unwrap()
                    .rows
                    .iter()
                    .position(|r| r.action.as_ref().is_some_and(f))
                    .expect("row");
            t.s.ed.set_cursor(row, 0);
        };
        use crate::magit::{RowAction, Section};
        // x on an untracked file asks, then deletes it; "n" keeps it.
        goto(
            &mut t,
            &|a| matches!(a, RowAction::File(p, Section::Untracked) if p == Path::new("new.txt")),
        );
        t.keys("xn<Enter>");
        magit_settle(&mut t);
        assert!(t.dir.path().join("new.txt").exists());
        goto(
            &mut t,
            &|a| matches!(a, RowAction::File(p, Section::Untracked) if p == Path::new("new.txt")),
        );
        t.keys("xy<Enter>");
        magit_settle(&mut t);
        assert!(!t.dir.path().join("new.txt").exists());
        // x on an unstaged file restores it from the index.
        goto(
            &mut t,
            &|a| matches!(a, RowAction::File(p, Section::Unstaged) if p == Path::new("g.txt")),
        );
        t.keys("xy<Enter>");
        magit_settle(&mut t);
        assert_eq!(
            fs::read_to_string(t.dir.path().join("g.txt")).unwrap(),
            "g\n"
        );
        // S stages all modified; - reverses the staged hunk in the worktree only.
        t.keys("S");
        magit_settle(&mut t);
        assert!(repo.read(&["diff", "--quiet"]).is_ok());
        goto(
            &mut t,
            &|a| matches!(a, RowAction::File(p, Section::Staged) if p == Path::new("f.txt")),
        );
        t.keys("<Tab>");
        magit_settle(&mut t);
        goto(
            &mut t,
            &|a| matches!(a, RowAction::Hunk(p, true, ..) if p == Path::new("f.txt")),
        );
        t.keys("-y<Enter>");
        magit_settle(&mut t);
        assert_eq!(
            fs::read_to_string(t.dir.path().join("f.txt")).unwrap(),
            "one\n"
        );
        assert!(repo.read(&["diff", "--cached", "--quiet"]).is_err());
        // U unstages everything.
        t.keys("Uy<Enter>");
        magit_settle(&mut t);
        assert!(repo.read(&["diff", "--cached", "--quiet"]).is_ok());
    }

    #[test]
    fn magit_section_movement_and_folding() {
        use crate::magit::{RowAction, Section};
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        for f in ["a", "b", "c"] {
            fs::write(t.dir.path().join(f), "1\n").unwrap();
            repo.stage_file(Path::new(f)).unwrap();
        }
        repo.read(&["commit", "-qm", "base"]).unwrap();
        for f in ["a", "b", "c"] {
            fs::write(t.dir.path().join(f), "2\n").unwrap();
        }
        repo.stage_file(Path::new("c")).unwrap();
        t.keys(" ms");
        magit_settle(&mut t);
        let at = |t: &T| t.s.ed.magit.as_ref().unwrap().action_at(t.s.ed.cur.line);
        let goto = |t: &mut T, want: RowAction| {
            let row =
                t.s.ed
                    .magit
                    .as_ref()
                    .unwrap()
                    .rows
                    .iter()
                    .position(|r| r.action.as_ref() == Some(&want))
                    .unwrap();
            t.s.ed.set_cursor(row, 0);
        };
        let file = |p: &str, s| RowAction::File(PathBuf::from(p), s);
        // gj / gk move between sibling files; gh goes up to the section.
        goto(&mut t, file("a", Section::Unstaged));
        t.keys("gj");
        assert_eq!(at(&t), Some(file("b", Section::Unstaged)));
        t.keys("gj");
        // b is the last unstaged file: stays (no sibling in the next section).
        assert_eq!(at(&t), Some(file("b", Section::Unstaged)));
        t.keys("gk");
        assert_eq!(at(&t), Some(file("a", Section::Unstaged)));
        t.keys("gh");
        assert_eq!(at(&t), Some(RowAction::Section(Section::Unstaged)));
        // ys copies the file at point; yb the buffer's revision (HEAD).
        goto(&mut t, file("a", Section::Unstaged));
        t.keys("ys");
        assert_eq!(t.s.ed.reg.text, "a");
        t.keys("yb");
        let head = repo.read(&["rev-parse", "HEAD"]).unwrap();
        assert_eq!(t.s.ed.reg.text, String::from_utf8_lossy(&head).trim());
        t.keys("gR");
        magit_settle(&mut t);
        goto(&mut t, RowAction::Section(Section::Unstaged));
        // ] moves to the next section heading.
        t.keys("]]");
        assert_eq!(at(&t), Some(RowAction::Section(Section::Staged)));
        // M-4 expands every file; 1 closes the section at point; M-1 closes all.
        t.keys("<M-4>");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(text.contains("+2"), "{text}");
        // C-k goes back to the previous section start; M-k / M-j siblings.
        goto(&mut t, file("c", Section::Staged));
        t.keys("<C-k>");
        assert_eq!(at(&t), Some(RowAction::Section(Section::Staged)));
        goto(&mut t, file("b", Section::Unstaged));
        t.keys("<M-k>");
        assert_eq!(at(&t), Some(file("a", Section::Unstaged)));
        t.keys("<M-j>");
        assert_eq!(at(&t), Some(file("b", Section::Unstaged)));
        t.keys("1");
        magit_settle(&mut t);
        assert!(
            t.s.ed
                .magit
                .as_ref()
                .unwrap()
                .closed
                .contains(&Section::Unstaged)
        );
        t.keys("<M-1>");
        magit_settle(&mut t);
        let view = t.s.ed.magit.as_ref().unwrap();
        assert!(view.closed.contains(&Section::Unstaged) && view.expanded.is_empty());
        assert!(!t.s.ed.buf.text().contains("\na\n") && !t.s.ed.buf.text().contains("+2"));
    }

    #[test]
    fn magit_draft_trailers_and_message_history() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "earlier message"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some());
        t.keys("inew message<Esc>");
        // C-c C-s signs off and C-c C-r reviews as you (git-commit-get-ident).
        t.keys("<C-c><C-s>");
        assert!(
            t.s.ed
                .buf
                .text()
                .contains("Signed-off-by: Fred <fred@example.test>"),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys("<C-c><C-r>");
        assert!(
            t.s.ed
                .buf
                .text()
                .contains("Reviewed-by: Fred <fred@example.test>"),
            "{}",
            t.s.ed.buf.text()
        );
        // C-c C-i: git-commit-insert-trailer; C-c there asks who to Cc.
        t.keys("<C-c><Tab>");
        assert_eq!(t.s.ed.magit_menu, Some('&'));
        t.keys("<C-c>Ann <ann@example.test><Enter>");
        assert!(
            t.s.ed.buf.text().contains("Cc: Ann <ann@example.test>"),
            "{}",
            t.s.ed.buf.text()
        );
        // M-k shows the previous commit message; M-j returns to the draft.
        t.keys("<M-k>");
        assert_eq!(t.s.ed.buf.text().trim(), "earlier message");
        t.keys("<M-j>");
        assert!(
            t.s.ed.buf.text().starts_with("new message"),
            "{}",
            t.s.ed.buf.text()
        );
        // Other keys after C-c keep their meaning.
        t.keys("<C-c>x");
        assert!(t.s.ed.buf.text().starts_with("ew message"));
    }

    #[test]
    fn magit_stash_push_limits_to_a_file() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        fs::write(t.dir.path().join("g.txt"), "g\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.stage_file(Path::new("g.txt")).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        fs::write(t.dir.path().join("g.txt"), "other\n").unwrap();
        // magit-stash-push is on level 5: C-x l shows it.
        t.keys(" mz<C-x>l5<Enter>");
        assert_eq!(t.s.ed.magit_menu, Some('z'));
        t.keys("P--g.txt<Enter>P");
        magit_settle(&mut t);
        assert_eq!(
            fs::read_to_string(t.dir.path().join("g.txt")).unwrap(),
            "g\n"
        );
        assert_eq!(
            fs::read_to_string(t.dir.path().join("f.txt")).unwrap(),
            "two\n"
        );
        assert!(!repo.read(&["stash", "list"]).unwrap().is_empty());
    }

    #[test]
    fn magit_ex_command_runs_upstream_names() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        t.keys(":Magit magit-status<Enter>");
        magit_settle(&mut t);
        assert!(
            t.s.ed
                .magit
                .as_ref()
                .is_some_and(|v| v.kind == crate::magit::Kind::Status)
        );
        // magit-status-jump: j in the dispatch menu, two-key fu/pu entries.
        fs::write(t.dir.path().join("new"), "n").unwrap();
        t.keys("gr");
        magit_settle(&mut t);
        t.keys(" mjn");
        let row = t.s.ed.buf.line(t.s.ed.cur.line);
        assert!(row.contains("Untracked files"), "{row}");
        t.keys(" mjfu");
        assert!(t.msg().contains("wasn't found"), "{}", t.msg());
        // ` shows the Git commands run in this repository.
        t.s.finish_git(
            crate::magit::repo::GitInvocation {
                expected_head: None,
                repo: crate::magit::repo::Repo::discover(t.dir.path()).unwrap(),
                args: vec!["fetch".into()],
                input: None,
                draft: None,
                draft_stamp: None,
                editor: false,
                env: vec![],
                after: None,
            },
            Err("fatal: no remote".into()),
        );
        magit_settle(&mut t);
        t.keys(" ms");
        magit_settle(&mut t);
        t.keys("`");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(
            text.contains("git fetch") && text.contains("fatal: no remote"),
            "{text}"
        );
        t.keys("q");
        magit_settle(&mut t);
        t.keys(":Magit magit-show-refs<Enter>");
        assert_eq!(t.s.ed.magit_menu, Some('y'));
        t.keys("<Esc>:Magit no-such-command<Enter>");
        assert!(t.msg().contains("unknown Magit command"), "{}", t.msg());
    }

    #[test]
    fn magit_file_dispatch_trace_and_edit_line_commit() {
        let mut t = T::open(Some("f.txt"), Some("foo() {\n  one\n}\nbar() {\n  x\n}\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "add foo"]).unwrap();
        fs::write(
            t.dir.path().join("f.txt"),
            "foo() {\n  two\n}\nbar() {\n  y\n}\n",
        )
        .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "change both"]).unwrap();
        fs::write(t.dir.path().join("g.txt"), "g\n").unwrap();
        repo.stage_file(Path::new("g.txt")).unwrap();
        repo.read(&["commit", "-qm", "unrelated"]).unwrap();
        t.keys(":e!<Enter>gg mFt");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(
            text.contains("change both") && text.contains("add foo"),
            "{text}"
        );
        assert!(!text.contains("unrelated"), "{text}");
        // Edit line: the commit that added line 1 (foo) is rebased with edit.
        t.keys("q");
        magit_settle(&mut t);
        t.keys("gg mFe");
        magit_settle(&mut t);
        let inv = t.s.pending_git.take().expect("rebase");
        assert!(inv.args.iter().any(|a| a == "rebase"), "{:?}", inv.args);
    }

    #[test]
    fn alt_keys_reach_magit_buffers_and_stay_esc_elsewhere() {
        // A plain buffer: Alt+j is Esc, j (insert mode left, cursor down).
        let mut t = T::open(Some("f.txt"), Some("one\ntwo\n"));
        t.keys("ix<M-j>");
        assert_eq!(t.s.ed.mode, crate::editor::Mode::Normal);
        assert_eq!(t.s.ed.cur.line, 1);
        // Word-jump in status: its labels never stage files.
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "changed\n").unwrap();
        t.keys(" ms");
        magit_settle(&mut t);
        let row =
            t.s.ed
                .magit
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .position(|r| {
                    matches!(
                        &r.action,
                        Some(crate::magit::RowAction::File(
                            _,
                            crate::magit::Section::Unstaged
                        ))
                    )
                })
                .unwrap();
        t.s.ed.set_cursor(row, 0);
        t.keys(" ss");
        magit_settle(&mut t);
        assert!(
            repo.read(&["diff", "--cached", "--quiet"]).is_ok(),
            "word-jump staged a file"
        );
        t.keys("<Esc>");
        // 1 on a commit row closes its section and lands on the heading.
        let row = t.s.ed.magit.as_ref().unwrap().rows.iter().position(|r| {
            matches!(
                &r.action,
                Some(crate::magit::RowAction::Section(
                    crate::magit::Section::UnpushedUpstream
                ))
            )
        });
        if let Some(h) = row {
            t.s.ed.set_cursor(h, 0);
            t.keys("<Tab>");
            magit_settle(&mut t);
            t.s.ed.set_cursor(h + 1, 0);
            t.keys("1");
            magit_settle(&mut t);
            assert_eq!(t.s.ed.cur.line, h);
        }
    }

    #[test]
    fn magit_buffers_take_upstream_menu_keys() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        t.keys(" ms");
        magit_settle(&mut t);
        // evil-collection's magit-mode-map: b l p ' " _ O and h for dispatch.
        for (key, menu) in [
            ("b", 'b'),
            ("l", 'l'),
            ("p", 'p'),
            ("'", 'o'),
            ("\"", 'u'),
            ("_", 'v'),
            ("O", 'X'),
            ("h", '?'),
            ("L", 'R'),
        ] {
            t.keys(key);
            assert_eq!(t.s.ed.magit_menu, Some(menu), "{key}");
            t.keys("<Esc>");
        }
        // Vim keeps j k for movement.
        t.keys("j");
        assert_eq!(t.s.ed.magit_menu, None);
    }

    #[test]
    fn magit_apply_and_reverse_hunks_from_a_commit_buffer() {
        let mut t = T::open(Some("f.txt"), Some("1\n2\n3\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "1\ntwo\n3\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "two"]).unwrap();
        // Reverse the commit's hunk in the worktree, then apply it again.
        t.keys(" mdc");
        magit_settle(&mut t);
        t.keys("HEAD<Enter>");
        magit_settle(&mut t);
        let row = t.s.ed.buf.text().lines().position(|l| l == "+two").unwrap();
        // Reversing asks first (magit-confirm 'reverse).
        t.keys(&format!(":{}<Enter>-y<Enter>", row + 1));
        magit_settle(&mut t);
        assert_eq!(
            fs::read_to_string(t.dir.path().join("f.txt")).unwrap(),
            "1\n2\n3\n"
        );
        t.keys(&format!(":{}<Enter>a", row + 1));
        magit_settle(&mut t);
        assert_eq!(
            fs::read_to_string(t.dir.path().join("f.txt")).unwrap(),
            "1\ntwo\n3\n"
        );
        // Applying it twice is refused by git apply --check.
        t.keys(&format!(":{}<Enter>a", row + 1));
        magit_settle(&mut t);
        assert!(
            t.msg().contains("patch") || t.msg().contains("error"),
            "{}",
            t.msg()
        );
    }

    #[test]
    fn magit_reverse_in_index_add_log_and_copy_diff() {
        let mut t = T::open(Some("f.txt"), Some("fn a() {\n    1\n}\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "fn a() {\n    2\n}\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "two"]).unwrap();
        t.keys(" mdc");
        magit_settle(&mut t);
        t.keys("HEAD<Enter>");
        magit_settle(&mut t);
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l == "+    2")
                .unwrap();
        // :Magit magit-copy-diff-as-kill copies the hunk (not to the
        // system clipboard in a test).
        t.s.ed.run_ex("set clipboard=");
        t.keys(&format!(
            "{}G:Magit magit-copy-diff-as-kill<Enter>",
            row + 1
        ));
        magit_settle(&mut t);
        assert!(t.s.ed.reg.text.contains("+    2"), "{:?}", t.s.ed.reg.text);
        assert!(t.s.ed.reg.text.starts_with("diff --git"));
        // u reverses the committed change in the index only (confirmed).
        t.keys(&format!(":{}<Enter>uy<Enter>", row + 1));
        magit_settle(&mut t);
        let staged = repo.read(&["diff", "--cached"]).unwrap();
        assert!(String::from_utf8_lossy(&staged).contains("+    1"));
        assert_eq!(
            fs::read_to_string(t.dir.path().join("f.txt")).unwrap(),
            "fn a() {\n    2\n}\n"
        );
        // C adds a changelog stub for the hunk to a new commit draft.
        t.keys(&format!(":{}<Enter>C", row + 1));
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some());
        assert!(
            t.s.ed.buf.text().contains("* f.txt (a): "),
            "{:?}",
            t.s.ed.buf.text()
        );
    }

    #[test]
    fn magit_log_select_navigation_and_history() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        for msg in ["first", "second"] {
            fs::write(t.dir.path().join("f.txt"), msg).unwrap();
            repo.stage_file(Path::new("f.txt")).unwrap();
            repo.read(&["commit", "-qm", msg]).unwrap();
        }
        repo.read(&["tag", "v1", "HEAD~1"]).unwrap();
        // c f without a commit at point opens magit-log-select; . picks.
        fs::write(t.dir.path().join("f.txt"), "fix").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys(":e!<Enter> mcf");
        magit_settle(&mut t);
        let sel = t.s.ed.magit.as_ref().unwrap();
        assert!(sel.select.is_some());
        assert!(sel.rows[0].text.contains("C-c C-c"), "{}", sel.rows[0].text);
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l.ends_with(" first"))
                .unwrap();
        // C-c C-r moves from ref to ref: HEAD's branch, then the tag.
        t.keys("gg<C-c><C-r>");
        assert_eq!(t.s.ed.cur.line, 1);
        t.keys("<C-c><C-r>");
        assert_eq!(t.s.ed.cur.line, row, "{}", t.s.ed.buf.text());
        t.keys(".");
        magit_settle(&mut t);
        // The terminal runs the commit (hooks, editor).
        let inv = t.s.pending_git.take().expect("fixup commit");
        inv.repo.run(&inv.args, None).unwrap();
        let subject = repo.read(&["log", "-1", "--format=%s"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&subject).trim(), "fixup! first");
        assert!(t.s.ed.magit.as_ref().is_none_or(|v| v.select.is_none()));
        // q aborts a selection without acting.
        t.keys(" mcf");
        magit_settle(&mut t);
        t.keys("q");
        assert_eq!(t.msg(), "Abort");
        // magit-log-move-to-revision in a log buffer.
        t.keys(" mll");
        magit_settle(&mut t);
        t.keys(":Magit magit-log-move-to-revision<Enter>");
        magit_settle(&mut t);
        t.keys("v1<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.line(t.s.ed.cur.line).ends_with(" first"));
        // A diff buffer remembers what it showed: C-c C-b and C-c C-f.
        t.keys(" mdu");
        magit_settle(&mut t);
        t.keys(" mds");
        magit_settle(&mut t);
        let shown = |t: &T| t.s.ed.magit.as_ref().map(|v| v.kind.clone());
        assert!(matches!(
            shown(&t),
            Some(crate::magit::Kind::Diff(
                crate::magit::diff::Target::Staged,
                _
            ))
        ));
        t.keys("<C-c><C-b>");
        magit_settle(&mut t);
        assert!(matches!(
            shown(&t),
            Some(crate::magit::Kind::Diff(
                crate::magit::diff::Target::Unstaged,
                _
            ))
        ));
        t.keys("<C-c><C-f>");
        magit_settle(&mut t);
        assert!(matches!(
            shown(&t),
            Some(crate::magit::Kind::Diff(
                crate::magit::diff::Target::Staged,
                _
            ))
        ));
        t.keys("<C-c><C-f>");
        assert!(t.msg().contains("No next entry"), "{}", t.msg());
        // M-Tab in status shows every file's diff, then hides them.
        fs::write(t.dir.path().join("f.txt"), "fix more").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        fs::write(t.dir.path().join("f.txt"), "fix more again").unwrap();
        t.keys(" ms");
        magit_settle(&mut t);
        t.keys("gr");
        magit_settle(&mut t);
        let m_tab = crate::key::Key {
            code: crate::key::KeyCode::Tab,
            ctrl: false,
            alt: true,
            shift: false,
        };
        t.s.handle_key(m_tab);
        magit_settle(&mut t);
        assert_eq!(
            t.s.ed.magit.as_ref().unwrap().expanded.len(),
            2,
            "{}",
            t.s.ed.buf.text()
        );
        t.s.handle_key(m_tab);
        magit_settle(&mut t);
        assert!(t.s.ed.magit.as_ref().unwrap().expanded.is_empty());
    }

    #[test]
    fn magit_abort_dwim_update_index_and_auto_revert() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        t.keys(":e!<Enter>");
        // Nothing to abort.
        t.keys(":Magit magit-abort-dwim<Enter>");
        assert_eq!(t.msg(), "Nothing to abort");
        // Space in a new branch name is a dash.
        t.keys(" mbn");
        magit_settle(&mut t);
        // magit-branch-read-upstream-first: start point, then the name.
        t.keys("<Enter>my topic<Enter>");
        magit_settle(&mut t);
        assert!(
            repo.read(&["rev-parse", "--verify", "my-topic"]).is_ok(),
            "{}",
            t.msg()
        );
        // Auto-revert: a Git command that changes a saved file reloads it.
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "two"]).unwrap();
        t.keys(":e!<Enter>");
        assert_eq!(t.s.ed.buf.text(), "two");
        t.s.finish_git(
            crate::magit::repo::GitInvocation {
                expected_head: None,
                repo: repo.clone(),
                args: vec![],
                input: None,
                draft: None,
                draft_stamp: None,
                editor: false,
                env: vec![],
                after: None,
            },
            {
                repo.read(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
                Ok(())
            },
        );
        assert_eq!(t.s.ed.buf.text(), "one");
        // magit-update-index: an edited index blob becomes the staged text.
        t.s.magit_action(crate::magit::Action::BlobVisit(
            crate::magit::blob::INDEX.into(),
            "f.txt".into(),
        ));
        magit_settle(&mut t);
        assert!(t.s.ed.blob.is_some(), "{}", t.msg());
        let (edit, _) =
            t.s.ed
                .buf
                .splice_edit(0, 1, &["staged".to_owned()])
                .unwrap();
        t.s.ed.buf.apply(edit);
        t.keys(":Magit magit-update-index<Enter>");
        magit_settle(&mut t);
        let staged = repo.read(&["show", ":f.txt"]).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&staged).trim(),
            "staged",
            "{}",
            t.msg()
        );
    }

    #[test]
    fn magit_git_commit_mode_for_messages_git_is_waiting_for() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        fs::write(t.dir.path().join(".git/COMMIT_EDITMSG"), "Subject\n").unwrap();
        t.keys(&format!(
            ":e {}<Enter>",
            t.dir.path().join(".git/COMMIT_EDITMSG").display()
        ));
        assert!(t.s.ed.commit_repo.is_some(), "{}", t.msg());
        // git-commit keys work; finishing saves and quits (with-editor-finish).
        t.keys("<C-c><C-s>");
        assert!(
            t.s.ed.buf.text().contains("Signed-off-by:"),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys(" mcc");
        assert!(t.s.quit, "{}", t.msg());
        let saved = fs::read_to_string(t.dir.path().join(".git/COMMIT_EDITMSG")).unwrap();
        assert!(saved.contains("Signed-off-by:"));
    }

    #[test]
    fn magit_diff_trace_definition_from_a_hunk() {
        let mut t = T::open(Some("f.rs"), Some("fn alpha() {\n    1\n}\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.rs")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        fs::write(t.dir.path().join("f.rs"), "fn alpha() {\n    2\n}\n").unwrap();
        repo.stage_file(Path::new("f.rs")).unwrap();
        repo.read(&["commit", "-qm", "two"]).unwrap();
        t.keys(" mdc");
        magit_settle(&mut t);
        t.keys("HEAD<Enter>");
        magit_settle(&mut t);
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l == "+    2")
                .unwrap();
        t.keys(&format!(":{}<Enter><C-c><C-t>", row + 1));
        magit_settle(&mut t);
        let kind = t.s.ed.magit.as_ref().map(|v| v.kind.clone());
        let Some(crate::magit::Kind::Log(_, args)) = kind else {
            panic!("{kind:?} {}", t.msg())
        };
        assert!(args.iter().any(|a| a == "-L:alpha:f.rs"), "{args:?}");
    }

    #[test]
    fn magit_save_repository_buffers_asks_before_refreshing() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        t.s.save_buffers = Some(true);
        t.keys("ccedited<Esc>");
        t.keys(" ms");
        assert!(t.msg().is_empty() || t.s.ed.mode != crate::editor::Mode::Normal);
        let crate::editor::Mode::Command(cl) = &t.s.ed.mode else {
            panic!("no save question: {}", t.msg())
        };
        assert!(cl.prompt.starts_with("Save file "), "{}", cl.prompt);
        t.keys("y<Enter>");
        magit_settle(&mut t);
        assert_eq!(t.file("f.txt"), "edited\n");
        assert!(t.s.ed.magit.is_some(), "status follows: {}", t.msg());
        // Answering n keeps the buffer unsaved and still runs the command.
        t.keys(":b1<Enter>ccagain<Esc> ms");
        t.keys("n<Enter>");
        magit_settle(&mut t);
        assert_eq!(t.file("f.txt"), "edited\n");
        assert!(t.s.ed.magit.is_some());
    }

    #[test]
    fn magit_dired_stage_and_log_marked_files() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        fs::write(t.dir.path().join("a"), "a\n").unwrap();
        fs::write(t.dir.path().join("b"), "b\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        t.keys(&format!(":e {}<Enter>", t.dir.path().display()));
        assert!(t.s.ed.dired.is_some());
        // Mark a, then F s stages the marked file only.
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l.ends_with(" a"))
                .unwrap();
        t.keys(&format!(":{}<Enter>m", row + 1));
        t.keys(" mFs");
        magit_settle(&mut t);
        let staged = repo.read(&["diff", "--cached", "--name-only"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&staged), "a\n");
        repo.read(&["commit", "-qm", "add a"]).unwrap();
        // F l logs the marked files.
        t.keys(" mFl");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(text.contains("add a") && !text.contains("base"), "{text}");
    }

    #[test]
    fn magit_file_log_reads_deleted_parent_and_tracked_symlink_name() {
        use std::os::unix::fs::symlink;
        let mut t = T::open(Some("target"), Some("target contents\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("target")).unwrap();
        repo.read(&["commit", "-qm", "target commit"]).unwrap();
        fs::create_dir(t.dir.path().join("nested")).unwrap();
        symlink("../target", t.dir.path().join("nested/link")).unwrap();
        repo.stage_file(Path::new("nested/link")).unwrap();
        repo.read(&["commit", "-qm", "symlink commit"]).unwrap();
        t.keys(&format!(
            ":e {}<Enter>",
            t.dir.path().join("nested/link").display()
        ));
        repo.read(&["rm", "nested/link"]).unwrap();
        repo.read(&["commit", "-qm", "remove symlink"]).unwrap();
        assert!(!t.dir.path().join("nested").exists());
        t.keys(" mL");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("symlink commit"), "{}", t.msg());
        assert!(t.s.ed.buf.text().contains("remove symlink"));
        assert!(!t.s.ed.buf.text().contains("target commit"));
        t.keys("q");
        assert!(t.s.ed.magit.is_none());
        assert_eq!(t.s.ed.buf.line(0), "target contents");
    }
    #[test]
    fn magit_push_menu_collects_arguments_and_prompts_before_git() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "initial"]).unwrap();
        let bare = tempfile::tempdir().unwrap();
        repo.read(&["init", "--bare", "-q", bare.path().to_str().unwrap()])
            .unwrap();
        repo.read(&["remote", "add", "origin", bare.path().to_str().unwrap()])
            .unwrap();
        t.keys(" mp");
        assert!(t.s.magit_job.is_none() && t.s.pending_git.is_none());
        t.keys("-n-hp");
        magit_settle(&mut t);
        assert!(t.s.pending_git.is_none());
        t.keys("origin<Enter>");
        magit_settle(&mut t);
        let inv = t.s.pending_git.take().expect("push invocation");
        let args: Vec<_> = inv.args.iter().map(|a| a.to_string_lossy()).collect();
        assert_eq!(
            args,
            [
                "push",
                "-v",
                "--dry-run",
                "--no-verify",
                "origin",
                "refs/heads/main:refs/heads/main"
            ]
        );
        t.keys(" mP-f-r");
        assert!(
            !t.s.ed
                .magit_options
                .contains(&crate::magit::MenuOption::PullFfOnly)
        );
        t.keys("-r-r");
        assert_eq!(
            crate::magit::menu_arguments(&t.s.ed, 'P'),
            ["--rebase=interactive"]
        );
        t.keys("-r-r");
        assert!(crate::magit::menu_arguments(&t.s.ed, 'P').is_empty());
    }
    #[test]
    fn magit_diff_menu_dwim_prompts_and_refreshes_with_buffer_arguments() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "initial"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "staged\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        fs::write(t.dir.path().join("f.txt"), "worktree\n").unwrap();
        t.keys(" md");
        assert!(t.s.magit_job.is_none());
        assert_eq!(
            crate::magit::menu_arguments(&t.s.ed, 'd'),
            ["--no-ext-diff", "--stat"]
        );
        t.keys("-s-AAs");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(
            text.contains("+staged") && !text.contains("+worktree"),
            "{text}"
        );
        assert!(matches!(
            &t.s.ed.magit.as_ref().unwrap().kind,
            crate::magit::Kind::Diff(crate::magit::diff::Target::Staged, args)
                if args == &["--diff-algorithm=default", "--no-ext-diff"]
        ));
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys("gr");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("+worktree"));
        t.keys(" md");
        assert_eq!(
            crate::magit::menu_arguments(&t.s.ed, 'd'),
            ["--diff-algorithm=default", "--no-ext-diff"]
        );
        t.keys("<Esc>q mdr");
        magit_settle(&mut t);
        t.keys("HEAD<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("+worktree"), "{}", t.msg());
        t.keys("q ms");
        magit_settle(&mut t);
        let staged =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l.starts_with("Staged changes"))
                .unwrap();
        t.keys(&format!(":{}<Enter> mdd", staged + 1));
        magit_settle(&mut t);
        assert!(matches!(
            &t.s.ed.magit.as_ref().unwrap().kind,
            crate::magit::Kind::Diff(crate::magit::diff::Target::Staged, _)
        ));
        t.keys("q q ml");
        t.keys("l");
        magit_settle(&mut t);
        t.keys("j mdc");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.text().contains("+original"),
            "{}",
            t.s.ed.buf.text()
        );
        let diff_buffers = |t: &T| {
            (0..t.s.bufs.len())
                .filter(|i| {
                    t.s.ed_at(*i)
                        .magit
                        .as_ref()
                        .is_some_and(|v| matches!(v.kind, crate::magit::Kind::Diff(..)))
                })
                .count()
        };
        let before = diff_buffers(&t);
        t.keys(" md-ws");
        magit_settle(&mut t);
        assert_eq!(diff_buffers(&t), before, "diff buffer reused in place");
    }
    #[test]
    fn magit_init_creates_repository_and_confirms_nesting() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        t.keys(" mi");
        assert!(t.s.magit_job.is_none());
        t.keys("<Enter>");
        magit_settle(&mut t);
        assert!(t.dir.path().join(".git").is_dir(), "{}", t.msg());
        assert!(t.s.ed.magit.is_some());
        t.keys("q mi");
        t.keys("nested/repo<Enter>");
        magit_settle(&mut t);
        assert!(!t.dir.path().join("nested").exists());
        t.keys("n<Enter>");
        assert!(t.msg().contains("Abort"));
        assert!(!t.dir.path().join("nested").exists());
        t.keys(" minested/repo<Enter>");
        magit_settle(&mut t);
        t.keys("y<Enter>");
        magit_settle(&mut t);
        assert!(
            t.dir.path().join("nested/repo/.git").is_dir(),
            "{}",
            t.msg()
        );
        let root = crate::magit::repo::Repo::discover(&t.dir.path().join("nested/repo"))
            .unwrap()
            .root;
        assert_eq!(t.s.ed.magit.as_ref().unwrap().repo.root, root);
        t.keys("q mi<Enter>");
        magit_settle(&mut t);
        assert!(matches!(
            &t.s.ed.mode,
            crate::editor::Mode::Command(cl) if cl.prompt.starts_with("Reinitialize existing repository")
        ));
        t.keys("<Esc> mi.git<Enter>");
        magit_settle(&mut t);
        assert!(
            matches!(
                &t.s.ed.mode,
                crate::editor::Mode::Command(cl) if cl.prompt.contains("inside a Git directory")
            ),
            "{}",
            t.msg()
        );
        t.keys("n<Enter>");
        assert!(!t.dir.path().join(".git/.git").exists());
        assert_eq!(t.s.ed.buf.line(0), "original");
    }
    #[test]
    fn magit_blame_navigates_chunks_shows_commits_and_quits() {
        let mut t = T::open(Some("f.txt"), Some("one\ntwo\nthree\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "first"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "one\nTWO\nthree\n").unwrap();
        repo.read(&["commit", "-qam", "second"]).unwrap();
        t.keys(":e!<Enter>");
        t.keys(" mB");
        assert!(t.s.magit_job.is_none());
        assert_eq!(crate::magit::menu_arguments(&t.s.ed, 'B'), ["-w"]);
        t.keys("b");
        magit_settle(&mut t);
        let blame = t.s.ed.blame.as_ref().expect("blaming");
        assert_eq!(blame.chunks.len(), 3);
        assert_eq!(blame.args, ["-w"]);
        assert!(t.s.ed.readonly);
        t.keys(" sq<Esc>");
        assert!(t.s.ed.blame.is_some(), "word-jump keys are not blame keys");
        t.keys("gg");
        t.keys("n");
        assert_eq!(t.s.ed.cur.line, 1);
        t.keys("N");
        assert!(t.msg().contains("No more chunks"));
        t.keys("P");
        assert_eq!(t.s.ed.cur.line, 1);
        t.keys("p");
        assert_eq!(t.s.ed.cur.line, 0);
        t.keys("c");
        assert_eq!(t.s.ed.blame.as_ref().unwrap().style(), "highlight");
        t.keys("j<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.text().contains("+TWO"), "{}", t.msg());
        t.keys("q");
        assert!(t.s.ed.blame.is_some());
        t.keys("q");
        assert!(t.s.ed.blame.is_none() && !t.s.ed.readonly);
        t.keys(" mBm");
        magit_settle(&mut t);
        assert!(t.s.ed.blame.as_ref().is_some_and(|b| b.echo()) && !t.s.ed.readonly);
        t.keys("gg");
        assert!(t.s.ed.blame.as_ref().unwrap().message(0).is_some());
        t.keys("ix<Esc>");
        t.keys("j");
        assert!(t.s.ed.blame.is_none());
        t.keys(" mBb");
        assert!(t.msg().contains("Save the buffer"), "{}", t.msg());
    }
    #[test]
    fn magit_blob_buffers_navigate_history_and_blame_revisions() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "first"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        repo.read(&["commit", "-qam", "second"]).unwrap();
        t.keys(":e!<Enter>");
        assert_eq!(t.s.ed.buf.line(0), "two");
        t.keys(" mFp");
        magit_settle(&mut t);
        let blob = t.s.ed.blob.clone().expect("blob buffer");
        assert_eq!(blob.file, Path::new("f.txt"));
        assert_eq!(t.s.ed.buf.line(0), "two");
        assert!(t.s.ed.readonly);
        t.keys(":w!<Enter>");
        assert!(t.msg().contains("read-only"), "{}", t.msg());
        t.keys("ix<Esc>");
        assert_eq!(t.s.ed.buf.line(0), "two");
        t.keys(" ms");
        magit_settle(&mut t);
        assert_eq!(
            t.s.ed.magit.as_ref().map(|v| v.repo.root.clone()),
            Some(repo.root.clone()),
            "{}",
            t.msg()
        );
        t.keys("q");
        assert!(t.s.ed.blob.is_some());
        t.keys("p");
        magit_settle(&mut t);
        assert_eq!(t.s.ed.buf.line(0), "one");
        assert!(t.msg().contains("first"), "{}", t.msg());
        t.keys("p");
        magit_settle(&mut t);
        assert!(t.msg().contains("beginning of time"));
        t.keys("b");
        magit_settle(&mut t);
        assert!(
            t.s.ed.blame.as_ref().is_some_and(|b| b.rev.is_some()),
            "{}",
            t.msg()
        );
        t.keys("qn");
        magit_settle(&mut t);
        assert_eq!(t.s.ed.buf.line(0), "two");
        t.keys("n");
        magit_settle(&mut t);
        assert!(t.s.ed.blob.is_none());
        assert!(t.s.ed.path.as_ref().is_some_and(|p| p.ends_with("f.txt")));
        assert_eq!(t.s.ed.buf.line(0), "two");
        t.keys(" mFv");
        magit_settle(&mut t);
        t.keys("HEAD~1<Enter><Enter>");
        magit_settle(&mut t);
        assert_eq!(t.s.ed.buf.line(0), "one", "{}", t.msg());
        t.keys("q");
        assert!(t.s.ed.blob.is_none());
        t.keys(" mBr");
        assert!(t.msg().contains("Only blob buffers"));
        t.keys(" mBb");
        magit_settle(&mut t);
        t.keys("b");
        magit_settle(&mut t);
        assert!(t.s.ed.blob.is_some(), "{}", t.msg());
        assert_eq!(t.s.ed.buf.line(0), "one");
        assert!(t.s.ed.blame.as_ref().is_some_and(|b| b.rev.is_some()));
        t.keys("b");
        assert!(t.msg().contains("no further history"), "{}", t.msg());
    }
    #[test]
    fn magit_file_dispatch_stages_renames_deletes_and_checks_out() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        let staged = |repo: &crate::magit::repo::Repo| {
            String::from_utf8(repo.read(&["diff", "--cached", "--name-only"]).unwrap()).unwrap()
        };
        t.keys(" mFs");
        magit_settle(&mut t);
        assert_eq!(staged(&repo), "f.txt\n", "{}", t.msg());
        t.keys(" mFu");
        magit_settle(&mut t);
        assert_eq!(staged(&repo), "");
        fs::write(t.dir.path().join(".gitignore"), "f.txt\n").unwrap();
        t.keys(" mFs");
        magit_settle(&mut t);
        t.keys("n<Enter>");
        assert_eq!(staged(&repo), "");
        t.keys(" mFs");
        magit_settle(&mut t);
        t.keys("y<Enter>");
        magit_settle(&mut t);
        assert_eq!(staged(&repo), "f.txt\n", "{} {:?}", t.msg(), t.s.ed.mode);
        fs::remove_file(t.dir.path().join(".gitignore")).unwrap();
        repo.read(&["commit", "-qm", "one"]).unwrap();
        t.keys(" mF,x");
        magit_settle(&mut t);
        t.keys("<Enter>");
        magit_settle(&mut t);
        assert!(!repo.tracked(Path::new("f.txt")), "{}", t.msg());
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys(" mF,r");
        magit_settle(&mut t);
        t.keys("<Enter>g.txt<Enter>");
        magit_settle(&mut t);
        assert!(t.dir.path().join("g.txt").exists() && !t.dir.path().join("f.txt").exists());
        assert!(
            t.s.ed.path.as_ref().unwrap().ends_with("g.txt"),
            "{:?}",
            t.s.ed.path
        );
        repo.read(&["commit", "-qm", "rename"]).unwrap();
        fs::write(t.dir.path().join("g.txt"), "changed\n").unwrap();
        t.keys(":e!<Enter> mF,c");
        magit_settle(&mut t);
        t.keys("<Enter><Enter>");
        magit_settle(&mut t);
        assert_eq!(
            fs::read_to_string(t.dir.path().join("g.txt")).unwrap(),
            "one\n"
        );
        t.keys(":e!<Enter>iedit<Esc> mF,k");
        magit_settle(&mut t);
        t.keys("<Enter>");
        assert!(t.msg().contains("Save"), "{}", t.msg());
        assert!(t.dir.path().join("g.txt").exists());
        fs::create_dir(t.dir.path().join("d")).unwrap();
        fs::write(t.dir.path().join("d/x"), "x").unwrap();
        t.keys("u mF,k");
        magit_settle(&mut t);
        t.keys("d<Enter>");
        t.keys("y<Enter>");
        assert!(
            t.dir.path().join("d").exists(),
            "only 'yes' confirms recursive delete"
        );
        t.keys(" mF,k");
        magit_settle(&mut t);
        t.keys("d<Enter>yes<Enter>");
        magit_settle(&mut t);
        assert!(!t.dir.path().join("d").exists(), "{}", t.msg());
        t.keys(" mF,r");
        magit_settle(&mut t);
        t.keys("../escape<Enter>x<Enter>");
        assert!(t.msg().contains("repository-relative"), "{}", t.msg());
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("x"), "precious").unwrap();
        std::os::unix::fs::symlink(outside.path(), t.dir.path().join("out")).unwrap();
        t.keys(" mF,k");
        magit_settle(&mut t);
        t.keys("out/x<Enter>");
        assert!(t.msg().contains("symbolic link"), "{}", t.msg());
        assert!(outside.path().join("x").exists());
        t.keys(" mF,r");
        magit_settle(&mut t);
        t.keys("g.txt<Enter>out/y<Enter>");
        assert!(t.msg().contains("symbolic link"), "{}", t.msg());
        t.keys(" mF,k");
        magit_settle(&mut t);
        t.keys("G.TXT<Enter>");
        assert!(t.msg().contains("does not exist"), "{}", t.msg());
        assert!(t.dir.path().join("g.txt").exists());
        fs::write(t.dir.path().join("scratch"), "x").unwrap();
        t.keys(" mF,k");
        magit_settle(&mut t);
        t.keys("scratch<Enter>");
        t.keys("<Enter>");
        assert!(
            t.dir.path().join("scratch").exists(),
            "untracked delete needs yes"
        );
        fs::write(t.dir.path().join(".gitignore"), "secret\n").unwrap();
        fs::write(t.dir.path().join("secret"), "x").unwrap();
        t.keys(" mF,k");
        magit_settle(&mut t);
        t.keys("secret<Enter>");
        assert!(t.msg().contains("ignored"), "{}", t.msg());
        assert!(t.dir.path().join("secret").exists());
        fs::create_dir(t.dir.path().join("dir")).unwrap();
        fs::write(t.dir.path().join("dir/in.txt"), "in\n").unwrap();
        t.keys(&format!(
            ":e {}<Enter>",
            t.dir.path().join("dir/in.txt").display()
        ));
        t.keys(" mF,r");
        magit_settle(&mut t);
        t.keys("dir<Enter>moved<Enter>");
        magit_settle(&mut t);
        assert!(t.dir.path().join("moved/in.txt").exists(), "{}", t.msg());
        fs::write(t.dir.path().join("a.txt"), "scratch").unwrap();
        fs::write(t.dir.path().join("moved/a.txt"), "precious").unwrap();
        t.keys(" mF,r");
        magit_settle(&mut t);
        t.keys("a.txt<Enter>moved<Enter>");
        assert!(t.msg().contains("already exists"), "{}", t.msg());
        assert_eq!(
            fs::read_to_string(t.dir.path().join("moved/a.txt")).unwrap(),
            "precious"
        );
        t.keys(" mF,k");
        magit_settle(&mut t);
        t.keys(".git<Enter>");
        assert!(t.msg().contains("metadata"), "{}", t.msg());
        assert!(t.dir.path().join(".git").is_dir());
        assert!(
            t.s.ed.path.as_ref().unwrap().ends_with("moved/in.txt"),
            "{:?}",
            t.s.ed.path
        );
    }
    #[test]
    fn magit_status_sections_jump_and_visit_commits_and_stashes() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "first"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "two\n").unwrap();
        repo.read(&["stash", "push", "-qm", "parked"]).unwrap();
        fs::write(t.dir.path().join("new"), "x").unwrap();
        t.keys(" ms");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(
            text.contains("Stashes (1)…") && text.contains("Recent commits…"),
            "{text}"
        );
        let line = |t: &T| t.s.ed.buf.line(t.s.ed.cur.line);
        t.keys("gz<Tab>");
        assert_eq!(line(&t), "Stashes (1)");
        t.keys("gn");
        assert!(line(&t).starts_with("Untracked files"));
        t.keys("gpu");
        assert_eq!(line(&t), "Recent commits…");
        t.keys("gfu");
        assert!(t.msg().contains("Unpulled from @{upstream}"), "{}", t.msg());
        t.keys("gpu<Tab>");
        assert_eq!(line(&t), "Recent commits");
        t.keys("j<Enter>");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.line(1).starts_with("Author:     "),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys("q");
        t.keys("gzj<Enter>");
        magit_settle(&mut t);
        assert!(matches!(
            t.s.ed.magit.as_ref().unwrap().kind,
            crate::magit::Kind::StashPatch(_)
        ));
    }
    #[test]
    fn magit_merge_edit_message_seeds_a_blank_draft_and_commits_two_parents() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        repo.read(&["checkout", "-qb", "topic"]).unwrap();
        fs::write(t.dir.path().join("other"), "x").unwrap();
        repo.stage_file(Path::new("other")).unwrap();
        repo.read(&["commit", "-qm", "topic work"]).unwrap();
        repo.read(&["checkout", "-q", "main"]).unwrap();
        t.keys(" mme");
        magit_settle(&mut t);
        t.keys("topic<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some(), "{}", t.msg());
        assert!(
            t.s.ed.buf.line(0).starts_with("Merge branch 'topic'"),
            "{}",
            t.s.ed.buf.line(0)
        );
        t.keys(":w<Enter> mcc");
        let inv = t.s.pending_git.take().expect("commit invocation");
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        t.s.finish_git(inv, result);
        let parents = repo.read(&["rev-list", "--parents", "-1", "HEAD"]).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&parents).split_whitespace().count(),
            3
        );
    }
    #[test]
    fn magit_rebase_todo_buffer_upstream_keys() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        for (file, msg) in [("f.txt", "first"), ("a", "second"), ("b", "third")] {
            fs::write(t.dir.path().join(file), msg).unwrap();
            repo.stage_file(Path::new(file)).unwrap();
            repo.read(&["commit", "-qm", msg]).unwrap();
        }
        t.keys(":e!<Enter> mRi");
        magit_settle(&mut t);
        // magit-log-select: pick HEAD~2 with `.`.
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l.ends_with(" first"))
                .unwrap();
        t.keys(&format!(":{}<Enter>.", row + 1));
        magit_settle(&mut t);
        assert!(t.s.ed.rebase_todo.is_some(), "{}", t.msg());
        let verb = |t: &T, l: usize| t.s.ed.buf.line(l);
        // Upstream's other action keys (c w m S F A).
        for (key, want) in [
            ("S", "fixup -c "),
            ("F", "fixup -C "),
            ("w", "reword "),
            ("m", "edit "),
            ("c", "pick "),
        ] {
            t.keys(&format!("gg{key}"));
            assert!(verb(&t, 0).starts_with(want), "{key}: {}", verb(&t, 0));
        }
        // b z insert lines; l t y read their argument.
        t.keys("ggb");
        assert_eq!(verb(&t, 1), "break");
        t.keys("z");
        assert_eq!(verb(&t, 2), "noop");
        t.keys("lmine<Enter>");
        assert_eq!(verb(&t, 3), "label mine");
        t.keys("tmine<Enter>");
        assert_eq!(verb(&t, 4), "reset mine");
        t.keys("yHEAD<Enter>");
        assert_eq!(verb(&t, 5), "pick HEAD");
        t.keys("MMmine<Enter>");
        assert_eq!(verb(&t, 6), "merge mine");
        // C-t switches to plain Vim keys: x deletes a character.
        t.keys("gg<C-t>x");
        assert!(t.s.ed.magit_prompt.is_none());
        assert!(!verb(&t, 0).starts_with("pick"), "{}", verb(&t, 0));
        t.keys("u<C-t>");
        assert!(verb(&t, 0).starts_with("pick"));
        // Enter shows the commit on the line.
        t.keys("gg<Enter>");
        magit_settle(&mut t);
        assert!(matches!(
            t.s.ed.magit.as_ref().map(|v| &v.kind),
            Some(crate::magit::Kind::Diff(
                crate::magit::diff::Target::Commit(_),
                _
            ))
        ));
        assert!(t.s.ed.buf.text().contains("first") || t.s.ed.buf.text().contains("second"));
        // Space stays the leader in the todo buffer.
        t.keys("q");
        magit_settle(&mut t);
        assert!(t.s.ed.rebase_todo.is_some());
        t.keys(" ms");
        magit_settle(&mut t);
        assert!(
            t.s.ed
                .magit
                .as_ref()
                .is_some_and(|v| v.kind == crate::magit::Kind::Status)
        );
    }

    #[test]
    fn magit_rebase_todo_buffer_keys_edit_and_run_the_list() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        for (file, msg) in [("f.txt", "first"), ("a", "second"), ("b", "third")] {
            fs::write(t.dir.path().join(file), msg).unwrap();
            repo.stage_file(Path::new(file)).unwrap();
            repo.read(&["commit", "-qm", msg]).unwrap();
        }
        t.keys(":e!<Enter> mRi");
        magit_settle(&mut t);
        // magit-log-select: pick HEAD~1 with `.`.
        let row =
            t.s.ed
                .buf
                .text()
                .lines()
                .position(|l| l.ends_with(" second"))
                .unwrap();
        t.keys(&format!(":{}<Enter>.", row + 1));
        magit_settle(&mut t);
        assert!(t.s.ed.rebase_todo.is_some(), "{}", t.msg());
        assert!(
            t.s.ed.buf.line(0).starts_with("pick "),
            "{}",
            t.s.ed.buf.line(0)
        );
        t.keys("gg<M-j>");
        assert!(
            t.s.ed.buf.line(1).contains("# second"),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys("ggd");
        assert!(t.s.ed.buf.line(0).starts_with("drop "));
        // fixup -C lines change action without keeping the option.
        let fixup = format!("fixup -C {}", t.s.ed.buf.line(1).split_once(' ').unwrap().1);
        let (edit, _) = t.s.ed.buf.splice_edit(1, 1, &[fixup]).unwrap();
        t.s.ed.buf.apply(edit);
        t.keys("ggjp");
        assert!(t.s.ed.buf.line(1).starts_with("pick ") && !t.s.ed.buf.line(1).contains("-C"));
        t.keys("ggx");
        t.keys("true<Enter>");
        assert_eq!(t.s.ed.buf.line(1), "exec true");
        t.keys("ZZ");
        let inv = t.s.pending_git.take().expect("rebase replay");
        assert!(inv.editor);
        let result = inv.repo.run(&inv.args, None).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        assert!(t.dir.path().join("a").exists());
        assert!(!t.dir.path().join("b").exists(), "third was dropped");
        t.keys(" mRi");
        magit_settle(&mut t);
        // The newest commit is the first row after the usage line.
        t.keys("2G.");
        magit_settle(&mut t);
        t.keys("ZQ");
        assert!(t.msg().contains("cancelled"), "{}", t.msg());
        assert!(t.s.pending_git.is_none());
    }
    #[test]
    fn magit_reflog_views_list_entries_and_visit_commits() {
        let mut t = T::open(Some("f.txt"), Some("one\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "first"]).unwrap();
        repo.read(&["commit", "-q", "--allow-empty", "-m", "second"])
            .unwrap();
        t.keys(" mlr");
        magit_settle(&mut t);
        let text = t.s.ed.buf.text();
        assert!(
            text.starts_with(&format!("{} 0  commit           second", &text[..7])),
            "{text}"
        );
        t.keys("j<Enter>");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.line(1).starts_with("Author:     "),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys("q q mlO");
        magit_settle(&mut t);
        t.keys("--bogus<Enter>");
        magit_settle(&mut t);
        assert!(t.msg().contains("invalid ref"), "{}", t.msg());
    }
    #[test]
    fn magit_file_log_rejects_buffers_without_a_source_file() {
        let mut t = T::open(None, None);
        t.keys(" mL");
        assert!(t.msg().contains("isn't visiting a file"), "{}", t.msg());
        assert!(t.s.magit_job.is_none());
        magit_repo(&t);
        t.s.ed.magit = Some(Box::new(crate::magit::View::status(
            crate::magit::repo::Repo::discover(t.dir.path()).unwrap(),
            Default::default(),
        )));
        t.keys(" mL");
        assert!(t.msg().contains("isn't visiting a file"), "{}", t.msg());
        assert!(t.s.magit_job.is_none());
    }
    #[test]
    fn magit_returns_from_commit_patch_to_history() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "initial"]).unwrap();
        t.keys(":Magit magit-log-head<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.magit.is_some());
        t.keys("j<Enter>");
        magit_settle(&mut t);
        assert!(
            t.s.ed.buf.line(1).starts_with("Author:     "),
            "{}",
            t.s.ed.buf.text()
        );
        t.keys("q");
        assert!(matches!(
            t.s.ed.magit.as_ref().unwrap().kind,
            crate::magit::Kind::Log(..)
        ));
    }
    #[test]
    fn magit_late_result_after_unnamed_buffer_replacement_is_ignored() {
        let mut t = T::open(None, None);
        magit_repo(&t);
        // A synthetic view supplies repository context without changing the slot.
        let view = crate::magit::View::status(
            crate::magit::repo::Repo::discover(t.dir.path()).unwrap(),
            Default::default(),
        );
        t.s.ed.magit = Some(Box::new(view));
        t.s.magit_action(crate::magit::Action::Status);
        t.s.ed.magit = None;
        t.keys(":e new-file<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.magit.is_none());
        assert!(t.s.ed.path.as_ref().unwrap().ends_with("new-file"));
    }
    #[test]
    fn magit_return_survives_buffer_deletion() {
        let mut t = T::open(Some("a.txt"), Some("a\n"));
        magit_repo(&t);
        t.keys(":e b.txt<Enter> ms");
        magit_wait(&mut t);
        t.keys(":bd1<Enter>q");
        assert!(t.s.ed.magit.is_none());
        assert!(t.s.ed.path.as_ref().unwrap().ends_with("b.txt"));
    }
    #[test]
    fn magit_commit_rejects_externally_changed_draft() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        // magit-commit-assert: a draft needs something to commit.
        crate::magit::repo::Repo::discover(t.dir.path())
            .unwrap()
            .stage_file(Path::new("f.txt"))
            .unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        t.keys("imy draft<Esc>:w<Enter>");
        let path = t.s.ed.path.clone().unwrap();
        fs::write(&path, "external draft\n").unwrap();
        t.keys(" mcc");
        assert!(t.s.pending_git.is_none());
        assert!(t.msg().contains("changed"));
        assert_eq!(fs::read_to_string(path).unwrap(), "external draft\n");
    }

    #[test]
    fn magit_recovered_commit_draft_keeps_submission_routing() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        // magit-commit-assert: a draft needs something to commit.
        crate::magit::repo::Repo::discover(t.dir.path())
            .unwrap()
            .stage_file(Path::new("f.txt"))
            .unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        t.keys("irecovered message<Esc>");
        t.s.write_swap();
        let info = swap::read(&t.s.swap_path).unwrap();
        t.s.recover(info);
        assert!(t.s.ed.commit_repo.is_some());
        t.keys(" mcc");
        assert!(t.s.pending_git.is_some());
        assert_eq!(t.s.ed.buf.line(0), "recovered message");
    }
    #[test]
    fn magit_warns_for_relative_unsaved_source_paths() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let absolute = t.s.ed.path.clone().unwrap();
        let cwd = std::env::current_dir().unwrap();
        let mut relative = PathBuf::new();
        for _ in cwd.components().skip(1) {
            relative.push("..");
        }
        relative.push(absolute.strip_prefix("/").unwrap());
        t.s.ed.path = Some(relative);
        t.keys("iunsaved <Esc> ms");
        magit_wait(&mut t);
        assert!(t.msg().contains("unsaved source"), "{}", t.msg());
    }
    #[test]
    fn magit_branch_picker_switches_and_preserves_unsaved_buffer() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "initial"]).unwrap();
        repo.read(&["branch", "other"]).unwrap();
        t.keys("iunsaved <Esc> mbb");
        magit_settle(&mut t);
        t.keys("other<Enter>");
        let inv = t.s.pending_git.take().unwrap();
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        t.s.finish_git(inv, result);
        assert_eq!(repo.status().unwrap().branch, "other");
        assert_eq!(t.s.ed.buf.line(0), "unsaved original");
    }

    #[test]
    fn magit_successful_commit_preserves_draft_changed_during_hook() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        t.keys("imy draft<Esc>:w<Enter> mcc");
        let path = t.s.ed.path.clone().unwrap();
        let inv = t.s.pending_git.take().unwrap();
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok());
        fs::write(&path, "external next draft\n").unwrap();
        t.s.finish_git(inv, result);
        assert_eq!(fs::read_to_string(path).unwrap(), "external next draft\n");
    }
    #[test]
    fn magit_visits_hunk_line() {
        let base = (0..30).map(|i| format!("line{i}\n")).collect::<String>();
        let mut t = T::open(Some("f.txt"), Some(&base));
        magit_repo(&t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "initial"]).unwrap();
        fs::write(
            t.dir.path().join("f.txt"),
            base.replace("line15\n", "changed\n"),
        )
        .unwrap();
        t.keys(" ms");
        magit_settle(&mut t);
        let row = t.s.ed.magit.as_ref().unwrap().rows.iter().position(|r| matches!(&r.action,Some(crate::magit::RowAction::File(p,crate::magit::Section::Unstaged)) if p == Path::new("f.txt"))).unwrap();
        t.s.ed.set_cursor(row, 0);
        t.keys("<Tab>");
        magit_settle(&mut t);
        let row =
            t.s.ed
                .magit
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .position(|r| r.text.contains("+changed"))
                .unwrap();
        t.s.ed.set_cursor(row, 0);
        t.keys("<Enter>");
        assert!(t.s.ed.magit.is_none());
        assert_eq!(t.s.ed.cur.line, 15);
    }
    #[test]
    fn magit_workflow_prompt_and_failed_operation_refresh() {
        let mut t = T::open(Some("f.txt"), Some("original\n"));
        magit_repo(&t);
        t.keys(" mbc");
        magit_settle(&mut t);
        assert!(t.s.ed.magit_prompt.is_some());
        t.keys("topic<Esc>");
        assert!(t.s.ed.magit_prompt.is_none());
        assert!(t.s.pending_git.is_none());
        t.keys(" ms");
        magit_settle(&mut t);
        fs::write(t.dir.path().join("later"), "new").unwrap();
        let repo = t.s.ed.magit.as_ref().unwrap().repo.clone();
        let inv = crate::magit::repo::GitInvocation {
            expected_head: None,
            repo,
            args: vec!["fetch".into()],
            input: None,
            draft: None,
            draft_stamp: None,
            editor: false,
            env: vec![],
            after: None,
        };
        t.s.finish_git(inv, Err("test operation failure".into()));
        magit_settle(&mut t);
        // The refreshed status lists the new file (in the hidden untracked section).
        assert!(
            t.s.ed
                .magit
                .as_ref()
                .unwrap()
                .snapshot
                .entries
                .iter()
                .any(|e| e.path == Path::new("later"))
        );
        assert!(format!("{:?}", t.s.ed.msg).contains("test operation failure"));
        t.keys(" mzl");
        magit_settle(&mut t);
        assert_eq!(
            t.s.ed.magit.as_ref().unwrap().kind,
            crate::magit::Kind::Stashes
        );
        assert!(
            t.s.ed
                .buf
                .to_bytes()
                .windows(11)
                .any(|b| b == b"Stashes (0)")
        );
    }
    #[test]
    fn magit_delayed_prompt_does_not_replace_newer_input() {
        for input in ["<Esc>", "i", ":"] {
            let mut t = T::open(Some("f.txt"), Some("source\n"));
            magit_repo(&t);
            t.keys(" mbc");
            t.keys(input);
            let mode = t.s.ed.mode.clone();
            magit_settle(&mut t);
            assert_eq!(t.s.ed.mode, mode);
            assert!(t.s.ed.magit_prompt.is_none());
        }
    }
    #[test]
    fn magit_delayed_prompt_cancelled_by_bracketed_paste() {
        let mut t = T::open(Some("f.txt"), Some("source\n"));
        magit_repo(&t);
        t.keys(" mbc");
        t.s.ed.paste("new input");
        magit_settle(&mut t);
        assert!(t.s.ed.magit_prompt.is_none());
        assert_eq!(t.s.ed.mode, Mode::Normal);
    }
    fn magit_stash_fixture(t: &mut T) -> crate::magit::repo::Repo {
        magit_repo(t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "base"]).unwrap();
        fs::write(t.dir.path().join("f.txt"), "stashed\n").unwrap();
        repo.read(&["stash", "push", "-qm", "saved"]).unwrap();
        t.keys(" mzl");
        magit_settle(t);
        t.s.ed.set_cursor(1, 0);
        repo
    }
    #[test]
    fn magit_stash_list_drop_requires_yes_and_preserves_source() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        t.keys("iunsaved <Esc>");
        let repo = magit_stash_fixture(&mut t);
        t.keys("x");
        assert!(matches!(t.s.ed.mode, Mode::Command(_)));
        t.keys("no<Enter>");
        magit_settle(&mut t);
        assert_eq!(repo.stashes().unwrap().len(), 1);
        t.keys("xyes<Enter>");
        magit_settle(&mut t);
        assert!(repo.stashes().unwrap().is_empty());
        assert!(
            t.s.ed
                .buf
                .to_bytes()
                .windows(11)
                .any(|b| b == b"Stashes (0)")
        );
        t.keys("q");
        assert_eq!(t.s.ed.buf.line(0), "unsaved base");
    }
    #[test]
    fn magit_stash_pop_and_inspect_use_saved_stash_identity() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_stash_fixture(&mut t);
        t.keys("<Enter>");
        magit_settle(&mut t);
        assert!(t.s.ed.buf.to_bytes().windows(8).any(|b| b == b"+stashed"));
        t.keys("q");
        t.s.ed.set_cursor(1, 0);
        t.keys("A");
        magit_settle(&mut t);
        assert!(repo.stashes().unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(t.dir.path().join("f.txt")).unwrap(),
            "stashed\n"
        );
    }
    #[test]
    fn magit_stash_drop_rejects_stale_confirmation() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_stash_fixture(&mut t);
        t.keys("x");
        fs::write(t.dir.path().join("f.txt"), "new stash\n").unwrap();
        repo.read(&["stash", "push", "-qm", "new"]).unwrap();
        t.keys("yes<Enter>");
        magit_settle(&mut t);
        assert_eq!(repo.stashes().unwrap().len(), 2);
        assert!(format!("{:?}", t.s.ed.msg).contains("stash list changed"));
    }
    fn magit_committed_fixture(t: &mut T) -> crate::magit::repo::Repo {
        magit_repo(t);
        let repo = crate::magit::repo::Repo::discover(t.dir.path()).unwrap();
        repo.read(&["config", "user.name", "Fred"]).unwrap();
        repo.read(&["config", "user.email", "fred@example.test"])
            .unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        repo.read(&["commit", "-qm", "original message"]).unwrap();
        repo
    }
    #[test]
    fn magit_amend_edits_message_and_staged_tree_without_clobbering_normal_draft() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        fs::write(t.dir.path().join("f.txt"), "staged\n").unwrap();
        // magit-commit-assert: a draft needs something to commit.
        crate::magit::repo::Repo::discover(t.dir.path())
            .unwrap()
            .stage_file(Path::new("f.txt"))
            .unwrap();
        t.keys(" mcc");
        magit_settle(&mut t);
        t.keys("inormal draft<Esc>:w<Enter>");
        let normal_path = t.s.ed.path.clone().unwrap();
        t.keys(" mca");
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some());
        assert_eq!(t.s.ed.buf.line(0), "original message");
        assert_ne!(t.s.ed.path.as_ref(), Some(&normal_path));
        assert_eq!(fs::read_to_string(&normal_path).unwrap(), "normal draft\n");
        fs::write(t.dir.path().join("f.txt"), "amended tree\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys("ccamended message<Esc> mcc");
        let inv = t.s.pending_git.take().expect("amend invocation");
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        t.s.finish_git(inv, result);
        magit_settle(&mut t);
        assert_eq!(
            String::from_utf8_lossy(&repo.read(&["log", "-1", "--format=%s"]).unwrap()).trim(),
            "amended message"
        );
        assert_eq!(
            repo.read(&["show", "HEAD:f.txt"]).unwrap(),
            b"amended tree\n"
        );
        assert_eq!(repo.history().unwrap().len(), 1);
        assert_eq!(fs::read_to_string(normal_path).unwrap(), "normal draft\n");
    }
    #[test]
    fn magit_reword_preserves_staged_changes_and_rejects_moved_head() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        fs::write(t.dir.path().join("f.txt"), "staged\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        t.keys(" mcw");
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some());
        t.keys("ccnew words<Esc> mcc");
        let inv = t.s.pending_git.take().expect("reword invocation");
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        t.s.finish_git(inv, result);
        magit_settle(&mut t);
        assert_eq!(repo.read(&["show", "HEAD:f.txt"]).unwrap(), b"base\n");
        assert_eq!(repo.read(&["show", ":f.txt"]).unwrap(), b"staged\n");
        t.keys(" mca");
        magit_settle(&mut t);
        repo.read(&["commit", "-qm", "intervening"]).unwrap();
        t.keys(" mcc");
        assert!(t.s.pending_git.is_none());
        assert!(format!("{:?}", t.s.ed.msg).contains("HEAD changed"));
    }
    #[test]
    fn magit_stash_menu_all_option_saves_ignored_files() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        fs::write(t.dir.path().join(".gitignore"), "ignored\n").unwrap();
        fs::write(t.dir.path().join("ignored"), "keep me\n").unwrap();
        t.keys(" mz-az");
        magit_settle(&mut t);
        assert!(t.s.ed.magit_prompt.is_some());
        t.keys("with ignored<Enter>");
        magit_settle(&mut t);
        let inv = t.s.pending_git.take().expect("stash --all invocation");
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        t.s.finish_git(inv, result);
        assert!(!t.dir.path().join("ignored").exists());
        assert_eq!(
            repo.read(&["show", "stash@{0}^3:ignored"]).unwrap(),
            b"keep me\n"
        );
    }
    #[test]
    fn magit_snapshot_menu_keeps_unsaved_source_and_both_saved_sides() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        fs::write(t.dir.path().join("f.txt"), "staged\n").unwrap();
        repo.stage_file(Path::new("f.txt")).unwrap();
        fs::write(t.dir.path().join("f.txt"), "worktree\n").unwrap();
        t.keys("iunsaved <Esc> mzZ");
        magit_settle(&mut t);
        assert!(t.s.pending_git.is_none());
        assert_eq!(
            repo.read(&["show", "stash@{0}:f.txt"]).unwrap(),
            b"worktree\n"
        );
        assert_eq!(
            repo.read(&["show", "stash@{0}^2:f.txt"]).unwrap(),
            b"staged\n"
        );
        assert_eq!(repo.read(&["show", ":f.txt"]).unwrap(), b"staged\n");
        assert_eq!(fs::read(t.dir.path().join("f.txt")).unwrap(), b"worktree\n");
        assert_eq!(t.s.ed.buf.to_bytes(), b"unsaved base\n");
        assert!(t.s.ed.buf.modified);
    }
    #[test]
    fn magit_snapshot_publication_failure_survives_switching_buffers() {
        use std::os::unix::fs::PermissionsExt;
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        fs::write(t.dir.path().join("f.txt"), "saved worktree\n").unwrap();
        let started = t.dir.path().join("hook-started");
        let gate = t.dir.path().join("hook-release");
        let hook = t.dir.path().join(".git/hooks/reference-transaction");
        fs::write(&hook, format!("#!/bin/sh\nif test \"$1\" = prepared; then\n touch '{}'\n while ! test -f '{}'; do sleep 0.01; done\n echo snapshot-publication-rejected >&2\n exit 1\nfi\n", started.display(), gate.display())).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        t.keys(" mzZ");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !started.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Release the hook even if an assertion below fails.
        let began = started.exists();
        let second = t.dir.path().join("second.txt");
        fs::write(&second, "second buffer\n").unwrap();
        t.keys(&format!(":e {}<Enter>", second.display()));
        fs::write(&gate, "release").unwrap();
        assert!(began, "reference transaction did not start");
        magit_settle(&mut t);
        assert_eq!(t.s.ed.path.as_ref(), Some(&second));
        assert_eq!(t.s.ed.buf.to_bytes(), b"second buffer\n");
        assert!(
            format!("{:?}", t.s.ed.msg).contains("snapshot-publication-rejected"),
            "{:?}",
            t.s.ed.msg
        );
        assert!(repo.stashes().unwrap().is_empty());
        assert_eq!(
            fs::read(t.dir.path().join("f.txt")).unwrap(),
            b"saved worktree\n"
        );
    }
    #[test]
    fn magit_saved_snapshot_refreshes_a_parked_stash_list_on_return() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        t.keys(" mzl");
        magit_settle(&mut t);
        let list = t.s.cur;
        fs::write(t.dir.path().join("f.txt"), "worktree\n").unwrap();
        t.keys(" mzZ");
        // Switch before consuming the finished mutation outcome.
        let second = t.dir.path().join("second.txt");
        fs::write(&second, "second buffer\n").unwrap();
        t.keys(&format!(":e {}<Enter>", second.display()));
        magit_settle(&mut t);
        assert_eq!(repo.stashes().unwrap().len(), 1);
        assert_eq!(t.s.ed.path.as_ref(), Some(&second));
        t.s.show(list);
        magit_settle(&mut t);
        assert!(String::from_utf8_lossy(&t.s.ed.buf.to_bytes()).contains("stash@{0}"));
    }
    #[test]
    fn magit_commit_menu_all_option_uses_saved_files() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        fs::write(t.dir.path().join("f.txt"), "saved tree\n").unwrap();
        t.keys("iunsaved <Esc> mc-ac");
        magit_settle(&mut t);
        assert!(t.s.ed.commit_repo.is_some());
        t.keys("iwith all<Esc> mcc");
        let inv = t.s.pending_git.take().expect("commit --all invocation");
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        t.s.finish_git(inv, result);
        magit_settle(&mut t);
        assert_eq!(repo.read(&["show", "HEAD:f.txt"]).unwrap(), b"saved tree\n");
    }
    #[test]
    fn magit_delayed_amend_draft_does_not_replace_newer_input() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        magit_committed_fixture(&mut t);
        t.keys(" mca");
        t.keys("i");
        magit_settle(&mut t);
        assert_eq!(t.s.ed.mode, Mode::Insert);
        assert!(t.s.ed.commit_repo.is_none());
    }
    #[test]
    fn magit_recovered_amend_keeps_mode_options_and_head_guard() {
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        t.keys(" mc+sa");
        magit_settle(&mut t);
        t.keys("ccrecovered amend<Esc>");
        t.s.write_swap();
        let info = swap::read(&t.s.swap_path).unwrap();
        t.s.recover(info);
        t.keys(" mcc");
        let inv = t.s.pending_git.take().expect("recovered amendment");
        assert!(inv.validate().is_ok());
        repo.read(&["commit", "--allow-empty", "-qm", "intervening"])
            .unwrap();
        assert!(inv.validate().unwrap_err().contains("HEAD changed"));
        // The recovered mode must still amend the original commit; options survive.
        assert!(inv.args.iter().any(|arg| arg == "--amend"));
        assert!(inv.args.iter().any(|arg| arg == "--signoff"));
        assert_eq!(t.s.ed.buf.line(0), "recovered amend");
    }
    #[test]
    fn magit_draft_menu_options_match_display_and_real_commit() {
        use std::os::unix::fs::PermissionsExt;
        let mut t = T::open(Some("f.txt"), Some("base\n"));
        let repo = magit_committed_fixture(&mut t);
        let hook = t.dir.path().join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        t.keys(" mc-n-ec");
        magit_settle(&mut t);
        t.keys("idraft message<Esc> mc");
        let Mode::Pick(p) = &t.s.ed.mode else {
            panic!("commit menu");
        };
        assert!(
            p.rows
                .iter()
                .any(|r| r.text.contains("Disable hooks") && r.text.contains("[on]"))
        );
        t.keys("+sc");
        let inv =
            t.s.pending_git
                .take()
                .expect("commit with new signoff option");
        let result = inv.repo.run(&inv.args, inv.input.as_deref()).map(|_| ());
        assert!(result.is_ok(), "{result:?}");
        t.s.finish_git(inv, result);
        magit_settle(&mut t);
        assert!(
            String::from_utf8_lossy(&repo.read(&["log", "-1", "--format=%B"]).unwrap())
                .contains("Signed-off-by: Fred <fred@example.test>")
        );
        t.keys(" mc-n-ec");
        magit_settle(&mut t);
        t.keys("isecond draft<Esc> mc-nc");
        let inv = t.s.pending_git.take().expect("commit with hooks restored");
        assert!(inv.repo.run(&inv.args, inv.input.as_deref()).is_err());
        assert_eq!(t.s.ed.buf.line(0), "second draft");
    }
}
