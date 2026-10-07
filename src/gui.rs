//! The C interface the native macOS front end (`macos/`) drives. The GUI
//! draws the same cell grid the terminal shows, with its own font and colors.
//!
//! One `Gui` per window, used from one thread. Colors are terminal-style
//! (default, palette index, or RGB) so the GUI's theme decides the palette.

use crate::app::{self, AiJob, Input};
use crate::config::Config;
use crate::editor::Mode;
use crate::ex::addr::Range;
use crate::ex::cmd::ExEffect;
use crate::highlight::Highlighter;
use crate::key::{Key, KeyCode};
use crate::magit::repo::GitInvocation;
use crate::session::{Session, SwapChoice};
use crate::swap::{self, SwapInfo};
use crate::ui::{self, View};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::{Color, Modifier};
use std::ffi::{CStr, CString, c_char};
use std::panic::{self, AssertUnwindSafe};
use std::thread::JoinHandle;
use std::time::Instant;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FredCell {
    /// `color()`: 0 default, 0x01_0000NN palette index NN, 0x02_RRGGBB.
    pub fg: u32,
    pub bg: u32,
    /// Byte range of the cell's text in `FredFrame::text`; empty when the
    /// cell is covered by the wide character before it.
    pub text_start: u32,
    pub text_len: u32,
    /// 1 bold, 2 italic, 4 underline, 8 reversed, 16 dim, 32 crossed out.
    pub flags: u32,
}

#[repr(C)]
pub struct FredFrame {
    pub cols: u16,
    pub rows: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub cursor_bar: bool,
    pub cells: *const FredCell,
    pub text: *const u8,
    pub text_len: usize,
}

/// Work that holds the editor until it finishes; keys are dropped meanwhile,
/// as the terminal drops them while it is handed over.
enum Job {
    Ai(AiJob, Range, bool, Instant),
    Git(JoinHandle<Result<(), String>>, GitInvocation),
    Shell(JoinHandle<Result<String, String>>, String),
}

/// A swap file question waiting for its answer. `name` is set for `:e`.
struct Ask {
    info: SwapInfo,
    alive: bool,
    name: Option<String>,
    saved_msg: Option<(String, bool)>,
}

pub struct Gui {
    s: Session,
    hl: Highlighter,
    cfg: Config,
    view: View,
    term: Terminal<TestBackend>,
    ask: Option<Ask>,
    job: Option<Job>,
    crashed: bool,
    cells: Vec<FredCell>,
    text: Vec<u8>,
    title: CString,
}

thread_local! {
    static ERROR: std::cell::RefCell<CString> = std::cell::RefCell::new(CString::default());
}

fn set_error(e: impl ToString) {
    let e = CString::new(e.to_string().replace('\0', "")).unwrap_or_default();
    ERROR.with(|cell| *cell.borrow_mut() = e);
}

/// Why the last `fred_new` returned null.
#[unsafe(no_mangle)]
pub extern "C" fn fred_error() -> *const c_char {
    ERROR.with(|cell| cell.borrow().as_ptr())
}

/// Open fred with command-line `argv` (without the program name): the same
/// file and `+N` arguments the terminal takes. Null on failure (`fred_error`).
///
/// # Safety
/// `argv` points to `argc` NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fred_new(argv: *const *const c_char, argc: usize) -> *mut Gui {
    let argv: Vec<String> = (0..argc)
        // SAFETY: the caller passes `argc` valid C strings.
        .map(|i| unsafe { CStr::from_ptr(*argv.add(i)) })
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let args = match crate::args::parse(argv) {
        Ok(crate::args::ArgsOrInfo::Run(a)) => a,
        Ok(_) => crate::args::Args::default(),
        Err(e) => {
            set_error(e);
            return std::ptr::null_mut();
        }
    };
    let (cfg, cfg_err) = Config::load();
    let opened = panic::catch_unwind(AssertUnwindSafe(|| app::open(&args, &cfg, cfg_err, true)));
    let (s, leftover, hl) = match opened {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => {
            set_error(e);
            return std::ptr::null_mut();
        }
        Err(_) => {
            set_error("crashed while opening");
            return std::ptr::null_mut();
        }
    };
    let mut g = Gui {
        s,
        hl,
        cfg,
        view: View::default(),
        term: Terminal::new(TestBackend::new(80, 24)).expect("in-memory terminal"),
        ask: None,
        job: None,
        crashed: false,
        cells: vec![],
        text: vec![],
        title: CString::default(),
    };
    match leftover {
        Some(info) => g.ask(info, None),
        // The swap file marks the file as open for the whole session.
        None => g.s.lock(),
    }
    Box::into_raw(Box::new(g))
}

/// Close the editor: remember the place and remove the swap file, unless
/// it quit with an error (or crashed), when the swap keeps the changes.
///
/// # Safety
/// `g` came from `fred_new` and is not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fred_free(g: *mut Gui) {
    // SAFETY: the caller gives back what `fred_new` made.
    let mut g = unsafe { Box::from_raw(g) };
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        if g.crashed || !g.s.quit {
            g.s.write_swap();
            return;
        }
        g.s.remember_place();
        g.s.cleanup();
    }));
}

/// Run `f` on the editor. A panic saves the swap and marks the editor
/// crashed (`fred_tick` reports it) instead of unwinding into Swift.
fn with<T: Default>(g: *mut Gui, f: impl FnOnce(&mut Gui) -> T) -> T {
    // SAFETY: the GUI passes the pointer `fred_new` returned, on one thread.
    let g = unsafe { &mut *g };
    if g.crashed {
        return T::default();
    }
    panic::catch_unwind(AssertUnwindSafe(|| f(g))).unwrap_or_else(|_| {
        g.crashed = true;
        let _ = panic::catch_unwind(AssertUnwindSafe(|| g.s.write_swap()));
        T::default()
    })
}

/// A key: `code` is 0 for the character `ch`, else Esc, Enter, Backspace,
/// Tab, BackTab, Up, Down, Left, Right, Home, End, PageUp, PageDown, Delete
/// (1-14). `mods`: 1 Ctrl, 2 Alt (Option), 4 Shift.
#[unsafe(no_mangle)]
pub extern "C" fn fred_key(g: *mut Gui, code: u32, ch: u32, mods: u32) {
    let Some(key) = make_key(code, ch, mods) else {
        return;
    };
    with(g, |g| {
        if g.job.is_some() {
            return;
        }
        if let Some(ask) = &g.ask {
            if let Some(choice) = app::swap_choice(ask.alive, key) {
                g.answer(choice);
            }
            return;
        }
        app::input(&mut g.s, &mut g.hl, &mut g.view, Input::Key(key));
    })
}

fn make_key(code: u32, ch: u32, mods: u32) -> Option<Key> {
    let (ctrl, alt, shift) = (mods & 1 != 0, mods & 2 != 0, mods & 4 != 0);
    let code = match code {
        // Ctrl-G cancels, as in Emacs; Ctrl-[ is Esc, as in a terminal.
        0 if ctrl && matches!(char::from_u32(ch)?, 'g' | 'G' | '[') => KeyCode::Esc,
        0 => KeyCode::Char(match char::from_u32(ch)? {
            c if ctrl => c.to_ascii_lowercase(),
            c => c,
        }),
        1 => KeyCode::Esc,
        2 => KeyCode::Enter,
        3 => KeyCode::Backspace,
        4 if shift => KeyCode::BackTab,
        4 => KeyCode::Tab,
        5 => KeyCode::BackTab,
        6 => KeyCode::Up,
        7 => KeyCode::Down,
        8 => KeyCode::Left,
        9 => KeyCode::Right,
        10 => KeyCode::Home,
        11 => KeyCode::End,
        12 => KeyCode::PageUp,
        13 => KeyCode::PageDown,
        14 => KeyCode::Delete,
        _ => return None,
    };
    let esc = code == KeyCode::Esc;
    Some(Key {
        code,
        ctrl: ctrl && !esc,
        alt,
        shift: shift && !matches!(code, KeyCode::Char(_) | KeyCode::BackTab),
    })
}

/// Pasted text (Cmd-V).
///
/// # Safety
/// `text` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fred_paste(g: *mut Gui, text: *const c_char) {
    // SAFETY: the caller passes a valid C string.
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    with(g, |g| {
        if g.job.is_none() && g.ask.is_none() {
            app::input(&mut g.s, &mut g.hl, &mut g.view, Input::Paste(&text));
        }
    })
}

/// A mouse event at cell `col`, `row`: `kind` 0 press, 1 drag, 2 release
/// (left button), 3 wheel up, 4 wheel down.
#[unsafe(no_mangle)]
pub extern "C" fn fred_mouse(g: *mut Gui, kind: u32, col: u16, row: u16) {
    let kind = match kind {
        0 => MouseEventKind::Down(MouseButton::Left),
        1 => MouseEventKind::Drag(MouseButton::Left),
        2 => MouseEventKind::Up(MouseButton::Left),
        3 => MouseEventKind::ScrollUp,
        4 => MouseEventKind::ScrollDown,
        _ => return,
    };
    with(g, |g| {
        if g.job.is_some() || g.ask.is_some() {
            return;
        }
        let area = g.term.backend().buffer().area;
        let m = MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };
        ui::render::mouse(&mut g.s.ed, &mut g.view, area, m);
    })
}

/// The window now holds `cols` x `rows` cells.
#[unsafe(no_mangle)]
pub extern "C" fn fred_resize(g: *mut Gui, cols: u16, rows: u16) {
    with(g, |g| {
        g.term.backend_mut().resize(cols.max(1), rows.max(1));
        app::input(&mut g.s, &mut g.hl, &mut g.view, Input::Resize);
    })
}

/// Cmd-S: `:w`.
#[unsafe(no_mangle)]
pub extern "C" fn fred_save(g: *mut Gui) {
    with(g, |g| {
        if g.job.is_none() && g.ask.is_none() {
            g.s.perform(ExEffect::Write {
                path: None,
                force: false,
                range: None,
                then_quit: false,
            });
        }
    })
}

/// The window's close button: `:q`, which refuses with unsaved changes.
#[unsafe(no_mangle)]
pub extern "C" fn fred_close(g: *mut Gui) {
    with(g, |g| {
        if g.job.is_none() && g.ask.is_none() {
            g.s.perform(ExEffect::Quit { force: false });
        }
    })
}

/// Background work, every 50ms or so. Bits: 1 redraw, 2 quit, 4 crashed.
#[unsafe(no_mangle)]
pub extern "C" fn fred_tick(g: *mut Gui) -> u32 {
    // SAFETY: as in `with`.
    let crashed = |g: *mut Gui| unsafe { (*g).crashed };
    let bits = with(g, |g| g.tick());
    if crashed(g) { 4 } else { bits }
}

/// Draw. The frame stays valid until the next call with `g`.
#[unsafe(no_mangle)]
pub extern "C" fn fred_render(g: *mut Gui) -> FredFrame {
    with(g, |g| Some(g.render())).unwrap_or(FredFrame {
        cols: 0,
        rows: 0,
        cursor_x: 0,
        cursor_y: 0,
        cursor_visible: false,
        cursor_bar: false,
        cells: std::ptr::null(),
        text: std::ptr::null(),
        text_len: 0,
    })
}

/// The file's name for the window title. Valid until the next call.
#[unsafe(no_mangle)]
pub extern "C" fn fred_title(g: *mut Gui) -> *const c_char {
    with(g, |g| {
        let name =
            g.s.ed
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or("[No Name]".into(), |n| n.to_string_lossy().into_owned());
        let dot = if g.s.ed.buf.modified { " •" } else { "" };
        g.title = CString::new(format!("{name}{dot}").replace('\0', "")).unwrap_or_default();
        Some(g.title.as_ptr())
    })
    .unwrap_or(c"".as_ptr())
}

impl Gui {
    fn ask(&mut self, info: SwapInfo, name: Option<String>) {
        self.ask = Some(Ask {
            alive: swap::owner_alive(&info),
            info,
            name,
            saved_msg: self.s.ed.msg.take(),
        });
    }

    fn answer(&mut self, choice: SwapChoice) {
        let ask = self.ask.take().expect("a question");
        self.s.ed.msg = ask.saved_msg;
        if ask.name.is_some() {
            self.s.resolve_edit(choice);
            return;
        }
        // The file's own swap, found when opening it.
        match choice {
            SwapChoice::Cancel => {
                self.s.no_swap = true;
                self.s.quit = true;
                return;
            }
            SwapChoice::ReadOnly => {
                self.s.ed.readonly = true;
                self.s.no_swap = true;
                self.s.ed.set_msg("opened read-only");
            }
            SwapChoice::Recover => {
                self.s.recover(ask.info);
                self.hl.set_file(
                    self.s
                        .ed
                        .path
                        .as_deref()
                        .or(self.s.ed.syntax_path.as_deref()),
                    &self.s.ed.buf,
                );
            }
            SwapChoice::Delete => {
                self.s.discard_swap();
                self.s.ed.msg = None;
            }
        }
        self.s.lock();
    }

    fn tick(&mut self) -> u32 {
        let s = &mut self.s;
        if s.quit {
            return 2;
        }
        let mut dirty = s.tick_magit();
        if self.job.as_ref().is_some_and(|j| match j {
            Job::Ai(h, ..) => h.is_finished(),
            Job::Git(h, _) => h.is_finished(),
            Job::Shell(h, _) => h.is_finished(),
        }) {
            match self.job.take().expect("a job") {
                Job::Ai(h, r, explain, _) => app::finish_ai(s, r, explain, h.join()),
                Job::Git(h, inv) => {
                    let result = h.join().unwrap_or_else(|_| Err("Git crashed".into()));
                    s.finish_git(inv, result);
                }
                Job::Shell(h, cmd) => {
                    match h.join().unwrap_or_else(|_| Err("crashed".into())) {
                        // ponytail: output shown in the :explain box; a
                        // scrolling output buffer when long output bites.
                        Ok(out) => {
                            let line = s.ed.cur.line;
                            let r = Range {
                                start: line,
                                end: line,
                            };
                            s.ed.explain = Some((r, format!(":!{cmd}\n{}", out.trim_end())));
                        }
                        Err(e) => s.ed.set_err(e),
                    }
                    // Dired's `!` may have changed the directory.
                    crate::dired::refresh(&mut s.ed);
                }
            }
            dirty = true;
        }
        if let Some(Job::Ai(_, r, explain, start)) = &self.job {
            app::ai_spin(s, *r, *explain, *start);
            dirty = true;
        }
        if self.job.is_none() {
            if let Some(inv) = s.pending_git.take() {
                self.job = Some(git_job(s, inv));
                dirty = true;
            } else if let Some(cmd) = s.pending_shell.take() {
                s.ed.set_msg(format!(":!{cmd}"));
                let run = cmd.clone();
                let h = std::thread::spawn(move || crate::shell::capture(&run, None));
                self.job = Some(Job::Shell(h, cmd));
                dirty = true;
            } else if let Some((r, prompt, explain)) = s.pending_ai.take() {
                let h = app::start_ai(&self.cfg, prompt, explain);
                self.job = Some(Job::Ai(h, r, explain, Instant::now()));
                dirty = true;
            }
        }
        if self.ask.is_none()
            && let Some(pe) = s.pending_edit.clone()
        {
            let name = pe.path.display().to_string();
            self.ask(pe.info, Some(name));
        }
        if let Some(ask) = &self.ask {
            let prompt = app::swap_prompt(&ask.info, ask.alive, ask.name.as_deref());
            if self.s.ed.msg.as_ref().is_none_or(|(m, _)| *m != prompt) {
                self.s.ed.msg = Some((prompt, false));
                dirty = true;
            }
        }
        dirty |= app::tick(&mut self.s, &mut self.hl, &mut self.view);
        // Keep drawing while visible lines are still being highlighted.
        dirty |= self.s.ed.magit.is_none() && self.hl.incomplete();
        if self.s.quit { 2 } else { dirty as u32 }
    }

    fn render(&mut self) -> FredFrame {
        let (s, view, hl, cfg) = (&mut self.s, &mut self.view, &mut self.hl, &self.cfg);
        let done = self
            .term
            .draw(|f| ui::draw(f, &s.ed, view, hl, cfg, app::HIGHLIGHT_BUDGET))
            .expect("in-memory terminal");
        let area = done.buffer.area;
        app::drawn(s, *view, area);
        self.cells.clear();
        self.text.clear();
        let mut covered = 0;
        for cell in &self.term.backend().buffer().content {
            let start = self.text.len() as u32;
            if covered == 0 {
                self.text.extend_from_slice(cell.symbol().as_bytes());
                covered = unicode_width::UnicodeWidthStr::width(cell.symbol()).max(1);
            }
            covered -= 1;
            let m = cell.modifier;
            let flags = [
                Modifier::BOLD,
                Modifier::ITALIC,
                Modifier::UNDERLINED,
                Modifier::REVERSED,
                Modifier::DIM,
                Modifier::CROSSED_OUT,
            ]
            .iter()
            .enumerate()
            .fold(0, |f, (i, b)| if m.contains(*b) { f | 1 << i } else { f });
            self.cells.push(FredCell {
                fg: color(cell.fg),
                bg: color(cell.bg),
                text_start: start,
                text_len: self.text.len() as u32 - start,
                flags,
            });
        }
        let backend = self.term.backend();
        let cursor = backend.cursor_position();
        FredFrame {
            cols: area.width,
            rows: area.height,
            cursor_x: cursor.x,
            cursor_y: cursor.y,
            cursor_visible: backend.cursor_visible(),
            cursor_bar: s.ed.mode == Mode::Insert,
            cells: self.cells.as_ptr(),
            text: self.text.as_ptr(),
            text_len: self.text.len(),
        }
    }
}

/// Run a Git command that the terminal would hand the screen to, with its
/// output captured instead: no terminal for prompts or an editor.
fn git_job(s: &mut Session, inv: GitInvocation) -> Job {
    s.git_busy = true;
    let what = inv
        .args
        .iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    s.ed.set_msg(format!("Git: {what}…"));
    let mut cmd = inv.repo.command();
    cmd.envs(inv.env.iter().map(|(k, v)| (k, v)))
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(&inv.args)
        .stdin(if inv.input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
    let (editor, validated, input) = (inv.editor, inv.validate(), inv.input.clone());
    let h = std::thread::spawn(move || {
        // ponytail: Git that wants an editor (rebase -i) needs a terminal;
        // run Fred as GIT_EDITOR in a new window when the GUI grows windows.
        if editor {
            return Err("this Git command needs Fred in a terminal".into());
        }
        validated?;
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("git: {e}"))?;
        if let Some(data) = input
            && let Some(mut stdin) = child.stdin.take()
        {
            std::thread::spawn(move || std::io::Write::write_all(&mut stdin, &data));
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty());
        Err(last.map_or(format!("Git failed ({})", out.status), |l| {
            l.trim().to_string()
        }))
    });
    Job::Git(h, inv)
}

/// A cell color for the GUI's theme to resolve: default, palette index
/// (the 16 named colors are 0-15), or RGB.
fn color(c: Color) -> u32 {
    let idx = |i: u8| 0x0100_0000 | i as u32;
    match c {
        Color::Reset => 0,
        Color::Black => idx(0),
        Color::Red => idx(1),
        Color::Green => idx(2),
        Color::Yellow => idx(3),
        Color::Blue => idx(4),
        Color::Magenta => idx(5),
        Color::Cyan => idx(6),
        Color::Gray => idx(7),
        Color::DarkGray => idx(8),
        Color::LightRed => idx(9),
        Color::LightGreen => idx(10),
        Color::LightYellow => idx(11),
        Color::LightBlue => idx(12),
        Color::LightMagenta => idx(13),
        Color::LightCyan => idx(14),
        Color::White => idx(15),
        Color::Indexed(i) => idx(i),
        Color::Rgb(r, g, b) => 0x0200_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_colors() {
        assert_eq!(make_key(0, 'x' as u32, 0), Some(Key::ch('x')));
        assert_eq!(make_key(0, 'R' as u32, 1), Some(Key::ctrl('r')));
        assert_eq!(make_key(0, '[' as u32, 1), Some(Key::new(KeyCode::Esc)));
        assert_eq!(make_key(0, 'g' as u32, 1), Some(Key::new(KeyCode::Esc)));
        assert_eq!(make_key(4, 0, 4), Some(Key::new(KeyCode::BackTab)));
        let alt_j = make_key(0, 'j' as u32, 2).unwrap();
        assert!(alt_j.alt && alt_j.char().is_none());
        assert_eq!(make_key(99, 0, 0), None);
        assert_eq!(color(Color::Reset), 0);
        assert_eq!(color(Color::DarkGray), 0x0100_0008);
        assert_eq!(color(Color::Rgb(1, 2, 3)), 0x0201_0203);
    }
}
