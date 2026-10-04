//! The terminal front end: fullscreen or inline viewport, event loop, signals.

use crate::args::{Args, LineArg};
use crate::complete::nearby;
use crate::config::Config;
use crate::editor::Mode;
use crate::ex::addr::Range;
use crate::fileio;
use crate::highlight::Highlighter;
use crate::key::{Key, KeyCode};
use crate::session::{Session, SwapChoice};
use crate::swap::{self, SwapInfo};
use crate::ui::{self, View, window_height};
use anyhow::{Result, anyhow};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{self, MoveTo, SetCursorStyle};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io::{self, IsTerminal, Stdout, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_millis(50);
const HIGHLIGHT_BUDGET: Duration = Duration::from_millis(20);
/// How long to wait for more pending input before drawing. Not zero: with
/// `use-dev-tty`, crossterm's poll reads nothing when given no time.
const BATCH: Duration = Duration::from_millis(1);

type Term = Terminal<CrosstermBackend<Stdout>>;

/// The whole terminal on the alternate screen (`full`), or an inline
/// window `height` rows tall under the prompt.
fn open_term(full: bool, height: u16) -> io::Result<Term> {
    if full {
        execute!(io::stdout(), EnterAlternateScreen)?;
    }
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: if full {
                Viewport::Fullscreen
            } else {
                Viewport::Inline(height)
            },
        },
    )
}

/// The editor's window: the whole screen, or inline under the prompt.
struct Ui {
    term: Term,
    full: bool,
    area: Option<Rect>,
    height: u16,
    view: View,
    bar_cursor: bool,
}

impl Ui {
    fn new(full: bool, height: u16) -> io::Result<Ui> {
        Ok(Ui {
            term: open_term(full, height)?,
            full,
            area: None,
            height,
            view: View::default(),
            bar_cursor: false,
        })
    }

    fn draw(&mut self, s: &mut Session, hl: &mut Highlighter, cfg: &Config) -> io::Result<()> {
        // Test hook: debug builds panic while drawing a line containing
        // PANIC when FRED_DEBUG_PANIC is set (see tests/e2e.rs).
        #[cfg(debug_assertions)]
        if std::env::var_os("FRED_DEBUG_PANIC").is_some()
            && s.ed.buf.line(s.ed.cur.line).contains("PANIC")
        {
            panic!("debug panic while drawing");
        }
        let view = &mut self.view;
        let done = self
            .term
            .draw(|f| ui::draw(f, &s.ed, view, hl, cfg, HIGHLIGHT_BUDGET))?;
        // `done.area` is the whole terminal; the buffer covers just the window.
        self.area = Some(done.buffer.area);
        let area = done.buffer.area;
        let gutter = ui::render::gutter_width(&s.ed, cfg).min(area.width as usize / 2);
        s.ed.viewport = Some(crate::zap::Viewport {
            view: self.view,
            rows: (area.height as usize).saturating_sub(2),
            cols: (area.width as usize).saturating_sub(gutter).max(1),
            wrap: cfg.wrap,
        });
        // Paging scrolls by the text rows actually on screen (minus status
        // and command lines), not the configured height, which may be "max".
        s.ed.win_height = (done.buffer.area.height as usize).saturating_sub(2).max(1);
        let bar = s.ed.mode == Mode::Insert;
        if bar != self.bar_cursor {
            self.bar_cursor = bar;
            let style = if bar {
                SetCursorStyle::SteadyBar
            } else {
                SetCursorStyle::SteadyBlock
            };
            execute!(io::stdout(), style)?;
        }
        Ok(())
    }

    /// Erase the window and leave the cursor where it started (fullscreen:
    /// back to the shell's screen, as it was).
    fn erase(&mut self) -> io::Result<()> {
        if self.full {
            return execute!(io::stdout(), LeaveAlternateScreen);
        }
        let y = self.area.map_or(0, |a| a.y);
        execute!(io::stdout(), MoveTo(0, y), Clear(ClearType::FromCursorDown))
    }

    /// Recreate the inline window (terminal resized, or the window needs to
    /// grow). Fullscreen follows the terminal's size by itself.
    fn rebuild(&mut self, height: u16) -> io::Result<()> {
        if self.full {
            return Ok(());
        }
        self.erase()?;
        self.term = open_term(false, height)?;
        self.height = height;
        self.area = None;
        Ok(())
    }

    /// Give the terminal back to the shell.
    fn close(&mut self) -> io::Result<()> {
        self.erase()?;
        execute!(
            io::stdout(),
            cursor::Show,
            SetCursorStyle::DefaultUserShape,
            DisableBracketedPaste,
            DisableMouseCapture
        )?;
        terminal::disable_raw_mode()?;
        io::stdout().flush()
    }

    fn reopen(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
        self.term = open_term(self.full, self.height)?;
        self.area = None;
        self.bar_cursor = false;
        Ok(())
    }
}

/// crossterm key → fred keys. Alt+X becomes Esc, X: fred binds no Alt keys,
/// and an Esc that arrives together with the next key (SSH, tmux) is
/// reported as Alt+key.
pub fn map_keys(k: KeyEvent) -> Vec<Key> {
    match map_key(k) {
        Some(key) if key.alt => vec![Key::new(KeyCode::Esc), Key { alt: false, ..key }],
        Some(key) => vec![key],
        None => vec![],
    }
}

/// crossterm key → fred key.
pub fn map_key(k: KeyEvent) -> Option<Key> {
    use event::KeyCode as C;
    if k.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let code = match k.code {
        // Ctrl-G cancels, as in Emacs: it is Esc everywhere.
        C::Char('g' | 'G') if ctrl && !alt => return Some(Key::new(KeyCode::Esc)),
        C::Char(c) => KeyCode::Char(if ctrl { c.to_ascii_lowercase() } else { c }),
        C::Esc => KeyCode::Esc,
        C::Enter => KeyCode::Enter,
        C::Backspace => KeyCode::Backspace,
        C::Tab if k.modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        C::Tab => KeyCode::Tab,
        C::BackTab => KeyCode::BackTab,
        C::Up => KeyCode::Up,
        C::Down => KeyCode::Down,
        C::Left => KeyCode::Left,
        C::Right => KeyCode::Right,
        C::Home => KeyCode::Home,
        C::End => KeyCode::End,
        C::PageUp => KeyCode::PageUp,
        C::PageDown => KeyCode::PageDown,
        C::Delete => KeyCode::Delete,
        _ => return None,
    };
    Some(Key { code, ctrl, alt })
}

/// `-i`/`--height` mean inline and `-f` fullscreen; otherwise the config
/// decides (fullscreen by default). "auto" goes fullscreen for a file with
/// more lines than the inline window shows, or to browse a directory.
fn use_fullscreen(args: &Args, cfg: &Config, ed: &crate::editor::Editor, rows: u16) -> bool {
    if args.inline || args.height.is_some() {
        return false;
    }
    if args.fullscreen {
        return true;
    }
    cfg.fullscreen.unwrap_or_else(|| {
        let inline_rows = (window_height(cfg.height, usize::MAX, rows) as usize).saturating_sub(2);
        matches!(ed.mode, Mode::Pick(_)) || ed.line_count() > inline_rows
    })
}

/// The terminal is gone: its window closed, or the other end of the pty.
fn tty_hung_up() -> bool {
    let mut p = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd and a zero timeout.
    unsafe { libc::poll(&mut p, 1, 0) == 1 && p.revents & libc::POLLHUP != 0 }
}

/// Put the terminal back the way the shell expects it.
fn restore_terminal() {
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        cursor::Show,
        SetCursorStyle::DefaultUserShape,
        DisableBracketedPaste,
        DisableMouseCapture
    );
    let _ = terminal::disable_raw_mode();
}

/// Set while the event loop runs under `catch_unwind`: the hook then only
/// records the message, and `run` prints it after saving the swap and
/// erasing the window (which would otherwise erase the message too).
static CATCHING: AtomicBool = AtomicBool::new(false);
static PANIC_NOTE: Mutex<Option<String>> = Mutex::new(None);

fn install_panic_hook() {
    let prev = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if CATCHING.load(Ordering::SeqCst) {
            if let Ok(mut note) = PANIC_NOTE.lock() {
                *note = Some(info.to_string());
            }
            return;
        }
        restore_terminal();
        let _ = writeln!(io::stdout());
        prev(info);
    }));
}

fn ago(saved_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let s = now.saturating_sub(saved_at);
    match s {
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

/// Ask what to do with a file's existing swap file. `name` is set for `:e`
/// (where `q` cancels the `:e` instead of quitting).
fn ask_swap(
    ui: &mut Ui,
    s: &mut Session,
    hl: &mut Highlighter,
    cfg: &Config,
    info: &SwapInfo,
    name: Option<&str>,
) -> Result<SwapChoice> {
    let alive = swap::owner_alive(info);
    let what = match name {
        Some(n) => format!("{n}: "),
        None => "swap: ".into(),
    };
    let prompt = if alive {
        format!(
            "{what}file is open in fred (pid {}): [o]pen read-only, [q]uit",
            info.pid
        )
    } else {
        let what = name.map_or(String::new(), |n| format!("{n}: "));
        format!(
            "{what}swap found (saved {}): [r]ecover, [d]elete, [q]uit",
            ago(info.saved_at)
        )
    };
    let saved_msg = s.ed.msg.take();
    let mut redraw = true;
    loop {
        if redraw {
            s.ed.msg = Some((prompt.clone(), false));
            ui.draw(s, hl, cfg)?;
        }
        // Poll rather than block, to notice a terminal that went away.
        redraw = event::poll(TICK)?;
        if !redraw {
            if tty_hung_up() {
                return Ok(SwapChoice::Cancel);
            }
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        let Some(key) = map_keys(k).pop() else {
            continue;
        };
        let choice = match (alive, key.char(), key.code) {
            (true, Some('o'), _) => SwapChoice::ReadOnly,
            (false, Some('r'), _) => SwapChoice::Recover,
            (false, Some('d'), _) => SwapChoice::Delete,
            (_, Some('q'), _) | (_, _, KeyCode::Esc) => SwapChoice::Cancel,
            _ => continue,
        };
        s.ed.msg = saved_msg;
        return Ok(choice);
    }
}

fn step(s: &mut Session, hl: &mut Highlighter, ev: Event, resized: &mut bool) {
    match ev {
        Event::Key(k) => {
            for key in map_keys(k) {
                s.handle_key(key);
            }
        }
        Event::Paste(text) => s.ed.paste(&text),
        Event::Resize(..) => {
            s.ed.zap = None;
            *resized = true;
        }
        _ => {}
    }
    if let Some(d) = s.ed.buf.take_dirty_from() {
        hl.invalidate(d);
    }
}

/// Give the terminal to something else (`away`), then take it back. The
/// swap is saved first, and the file checked for changes after.
fn hand_over(ui: &mut Ui, s: &mut Session, away: impl FnOnce()) -> Result<()> {
    s.write_swap();
    ui.close()?;
    away();
    ui.reopen()?;
    if let Some(p) = &s.ed.path
        && fileio::changed_on_disk(p, s.stamp.as_ref())
    {
        s.ed.set_err("file changed on disk since it was read");
    }
    Ok(())
}

fn suspend(ui: &mut Ui, s: &mut Session) -> Result<()> {
    hand_over(ui, s, || {
        // Stop the whole process group, as vim does: when fred runs under
        // another program (git's $EDITOR), stopping only fred would leave
        // that parent waiting and the terminal looking hung.
        // SAFETY: sending a signal has no memory-safety preconditions.
        unsafe {
            libc::kill(0, libc::SIGTSTP);
        }
    })
}

/// `:!cmd`: run it on the real terminal, then wait for Enter, as vim does.
/// `:ai`: Claude (`cfg.ai_command`, plus `cfg.ai_rules`) answers in the
/// background while a spinner holds the editor; keys typed meanwhile are
/// dropped, so the lines can't change under the reply. `:explain`
/// (`cfg.explain_command`) shows the reply in a box over the lines instead.
// ponytail: no cancel; kill the child on Esc if a stuck claude bites.
fn ask_claude(
    ui: &mut Ui,
    s: &mut Session,
    hl: &mut Highlighter,
    cfg: &Config,
    r: Range,
    prompt: String,
    explain: bool,
) -> Result<()> {
    const SPIN: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let cmd = if explain {
        cfg.explain_command.clone()
    } else {
        cfg.ai_command.clone()
    };
    let verb = if explain { "explaining" } else { "rewriting" };
    let prompt = match cfg.ai_rules.trim() {
        "" => prompt,
        rules => format!("{prompt}\nRules: {rules}\n"),
    };
    let job = std::thread::spawn(move || crate::shell::capture(&cmd, Some(prompt)));
    let what = match r.end - r.start {
        0 => format!("line {}", r.start + 1),
        _ => format!("lines {}-{}", r.start + 1, r.end + 1),
    };
    let start = Instant::now();
    while !job.is_finished() {
        let t = start.elapsed();
        let spin = SPIN[(t.as_millis() / 100) as usize % SPIN.len()];
        s.ed.set_msg(format!("{spin} Claude is {verb} {what}… {}s", t.as_secs()));
        ui.draw(s, hl, cfg)?;
        if event::poll(TICK)? {
            event::read()?;
        }
        if tty_hung_up() {
            return Ok(());
        }
    }
    match job.join() {
        Ok(Ok(out)) if explain => {
            s.ed.explain = Some((r, out.trim().to_string()));
            s.ed.msg = None;
        }
        Ok(Ok(out)) => {
            s.ed.ai_reply(r, &out);
            s.ed.set_msg(format!("Claude rewrote {what}"));
        }
        Ok(Err(e)) => s.ed.set_err(e),
        Err(_) => s.ed.set_err("claude: crashed"),
    }
    Ok(())
}

fn run_shell(ui: &mut Ui, s: &mut Session, cmd: &str) -> Result<()> {
    hand_over(ui, s, || {
        let mut out = io::stdout();
        let _ = writeln!(out, ":!{cmd}");
        let note = match crate::shell::command(cmd).status() {
            Ok(st) if st.success() => String::new(),
            Ok(st) => st
                .code()
                .map_or("[killed] ".into(), |c| format!("[exit {c}] ")),
            Err(e) => format!("[{e}] "),
        };
        let _ = write!(out, "\n{note}Press Enter to continue");
        let _ = out.flush();
        let _ = io::stdin().read_line(&mut String::new());
    })
}

fn run_git(ui: &mut Ui, s: &mut Session, inv: crate::magit::repo::GitInvocation) -> Result<()> {
    // Catch SIGINT in Fred while exec resets the child's handler to the default.
    // Ctrl-C interrupts Git/hooks without killing the editor and its draft.
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupt_handler =
        signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))?;
    s.git_busy = true;
    let mut result = Err("Git command did not run".into());
    let handoff = hand_over(ui, s, || {
        let mut out = io::stdout();
        let _ = writeln!(
            out,
            "Git: {}",
            inv.args
                .iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut cmd = inv.repo.command();
        if inv.editor
            && let Ok(exe) = std::env::current_exe()
        {
            // GIT_EDITOR is run by a shell: quote Fred's own path.
            let quoted = format!("'{}'", exe.to_string_lossy().replace('\'', "'\\''"));
            cmd.env("GIT_EDITOR", quoted);
            // Fred's -c sequence.editor must win over an inherited override.
            cmd.env_remove("GIT_SEQUENCE_EDITOR");
        }
        cmd.args(&inv.args);
        cmd.stdin(if inv.input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::inherit()
        });
        result = (|| {
            inv.validate()?;
            let mut child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
            let writer = inv.input.as_ref().map(|bytes| {
                let data = bytes.clone();
                let mut input = child.stdin.take().expect("piped Git input");
                std::thread::spawn(move || input.write_all(&data))
            });
            let status = child.wait().map_err(|e| e.to_string())?;
            if let Some(writer) = writer {
                let _ = writer.join();
            }
            if status.success() {
                Ok(())
            } else {
                Err(format!("Git failed ({status}); see command output"))
            }
        })();
        let note = result
            .as_ref()
            .err()
            .map_or("Git completed", String::as_str);
        let _ = write!(out, "\n{note}\nPress Enter to continue");
        let _ = out.flush();
        if !interrupted.load(Ordering::Relaxed) {
            let _ = io::stdin().read_line(&mut String::new());
        }
    });
    signal_hook::low_level::unregister(interrupt_handler);
    s.finish_git(inv, result);
    handoff
}

pub fn run(args: Args, mut cfg: Config, cfg_err: Option<String>) -> Result<i32> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("fred: not a terminal");
        return Ok(1);
    }
    if let Some(h) = args.height {
        cfg.height = h;
    }
    let (mut s, leftover) = match Session::open(args.file.clone(), &cfg, &swap::swap_dir()) {
        Ok(x) => x,
        Err(e) => {
            let name = args
                .file
                .as_ref()
                .map_or(String::new(), |p| format!("{}: ", p.display()));
            eprintln!("fred: {name}{e}");
            return Ok(1);
        }
    };
    match args.line {
        Some(LineArg::N(n)) => s.goto_line(Some(n)),
        Some(LineArg::Last) => s.goto_line(None),
        None => {}
    }
    let truecolor = matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor" | "24bit")
    );
    let mut hl = match Highlighter::new(&cfg.theme, truecolor) {
        Ok(h) => h,
        Err(e) => {
            s.ed.set_err(e);
            Highlighter::new("ansi", truecolor).map_err(|e| anyhow!(e))?
        }
    };
    hl.set_file(s.ed.path.as_deref(), &s.ed.buf);
    if let Some(r) = &hl.disabled {
        s.ed.set_msg(r.clone());
    }
    if let Some(e) = cfg_err {
        s.ed.set_err(e);
    }
    s.ed.nearby = nearby::spawn(s.ed.path.clone());

    let stop = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(sig, Arc::clone(&stop))?;
    }
    terminal::enable_raw_mode()?;
    let setup = (|| -> Result<Ui> {
        execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
        let rows = terminal::size()?.1;
        let full = use_fullscreen(&args, &cfg, &s.ed, rows);
        Ok(Ui::new(
            full,
            window_height(cfg.height, s.ed.line_count(), rows),
        )?)
    })();
    let mut ui = match setup {
        Ok(ui) => ui,
        Err(e) => {
            restore_terminal();
            return Err(e.context("could not set up the terminal"));
        }
    };
    install_panic_hook();

    CATCHING.store(true, Ordering::SeqCst);
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        event_loop(&mut ui, &mut s, &mut hl, &cfg, leftover, &stop)
    }));
    CATCHING.store(false, Ordering::SeqCst);
    let code = match result {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            s.write_swap();
            let _ = ui.close();
            return Err(e);
        }
        Err(_) => {
            s.write_swap();
            let _ = ui.close();
            let note = PANIC_NOTE.lock().ok().and_then(|mut n| n.take());
            // The terminal may be gone: a failed print must not panic again.
            let mut err = io::stderr();
            let _ = writeln!(err, "fred: {}", note.as_deref().unwrap_or("panicked"));
            if s.ed.buf.modified {
                let _ = writeln!(
                    err,
                    "fred: crashed; unsaved changes are in {} (open the file again to recover them)",
                    s.swap_path.display()
                );
            }
            return Ok(101);
        }
    };
    s.remember_place();
    ui.close()?;
    if code == 0 {
        s.cleanup();
        if let Some(w) = &s.written {
            let _ = writeln!(io::stdout(), "{w}");
        }
    }
    Ok(code)
}

fn event_loop(
    ui: &mut Ui,
    s: &mut Session,
    hl: &mut Highlighter,
    cfg: &Config,
    leftover: Option<SwapInfo>,
    stop: &AtomicBool,
) -> Result<i32> {
    if let Some(info) = leftover {
        match ask_swap(ui, s, hl, cfg, &info, None)? {
            SwapChoice::Cancel => {
                s.no_swap = true;
                return Ok(0);
            }
            SwapChoice::ReadOnly => {
                s.ed.readonly = true;
                s.no_swap = true;
                s.ed.set_msg("opened read-only");
            }
            SwapChoice::Recover => {
                s.recover(info);
                hl.set_file(s.ed.path.as_deref(), &s.ed.buf);
            }
            SwapChoice::Delete => {
                s.discard_swap();
                s.ed.msg = None;
            }
        }
    }
    // The swap file marks the file as open for the whole session.
    s.lock();
    // Draw only when something changed: an idle fred costs no CPU and sends
    // nothing to the terminal (which matters over SSH).
    let mut dirty = true;
    loop {
        if dirty {
            ui.draw(s, hl, cfg)?;
            // Keep drawing while visible lines are still being highlighted.
            dirty = s.ed.magit.is_none() && hl.incomplete();
        }
        // SIGTERM/SIGHUP, or a terminal that went away without a SIGHUP
        // (crossterm then spins reading end-of-file at 100% CPU).
        if stop.load(Ordering::Relaxed) || tty_hung_up() {
            s.write_swap();
            return Ok(1);
        }
        let mut resized = false;
        if event::poll(TICK)? {
            dirty = true;
            loop {
                let ev = event::read()?;
                let redraw_view = matches!(ev, Event::Mouse(_) | Event::Resize(..));
                if let Event::Key(k) = ev
                    && map_key(k) == Some(Key::ctrl('z'))
                {
                    suspend(ui, s)?;
                    break;
                }
                match ev {
                    Event::Mouse(m) => {
                        if let Some(area) = ui.area {
                            ui::render::mouse(&mut s.ed, &mut ui.view, cfg, area, m);
                        }
                    }
                    ev => {
                        let keep_view = s.ed.zap.is_some();
                        let keyboard =
                            matches!(ev, Event::Key(_) | Event::Paste(_) | Event::Resize(..));
                        if keyboard {
                            ui.view.last_click = None;
                        }
                        step(s, hl, ev, &mut resized);
                        if keyboard
                            && !keep_view
                            && s.ed.zap.is_none()
                            && s.ed.vim.pending != [Key::ch(' ')]
                        {
                            ui.view.detached = false;
                        }
                    }
                }
                // Synchronize the viewport snapshot before handling keys that
                // follow mouse scrolling or a terminal resize in the same batch.
                if s.quit || redraw_view || !event::poll(BATCH)? {
                    break;
                }
            }
        }
        if s.quit {
            return Ok(0);
        }
        if s.tick_magit() {
            dirty = true;
        }
        if let Some(inv) = s.pending_git.take() {
            run_git(ui, s, inv)?;
            dirty = true;
        }
        if let Some(cmd) = s.pending_shell.take() {
            run_shell(ui, s, &cmd)?;
            // Dired's `!` may have changed the directory.
            crate::dired::refresh(&mut s.ed);
            dirty = true;
        }
        if let Some((r, prompt, explain)) = s.pending_ai.take() {
            ask_claude(ui, s, hl, cfg, r, prompt, explain)?;
            dirty = true;
        }
        if let Some(pe) = s.pending_edit.clone() {
            let name = pe.path.display().to_string();
            let choice = ask_swap(ui, s, hl, cfg, &pe.info, Some(&name))?;
            s.resolve_edit(choice);
            dirty = true;
        }
        if s.reloaded {
            s.reloaded = false;
            hl.set_file(s.ed.path.as_deref(), &s.ed.buf);
            s.ed.nearby = nearby::spawn(s.ed.path.clone());
            ui.view = View::default();
            dirty = true;
        }
        let rows = terminal::size()?.1;
        let shown = (ui.height as usize).saturating_sub(2);
        // A picker gets the full configured height, whatever the file's size.
        let lines = match s.ed.mode {
            Mode::Pick(_) => usize::MAX,
            _ => s.ed.line_count().max(shown),
        };
        let want = window_height(cfg.height, lines, rows);
        // Fullscreen never grows (its height isn't tracked): without the
        // `!ui.full`, an open picker redrew every tick.
        if resized || (!ui.full && want > ui.height) {
            ui.rebuild(want)?;
            dirty = true;
        }
        // Picker results from background threads, or a grep due to start.
        if crate::pick::tick(&mut s.ed) {
            ui.view.last_click = None;
            dirty = true;
        }
        // A definition search may have found where to jump.
        if let Some(eff) = s.ed.pending_effect.take() {
            s.perform(eff);
            dirty = true;
        }
        // Git marks after an edit (or once the staged text has loaded).
        dirty |= s.ed.git.refresh(&s.ed.buf);
        let msg = s.ed.msg.clone();
        s.maybe_swap(Instant::now());
        dirty |= s.ed.msg != msg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use event::KeyCode as C;

    #[test]
    fn fullscreen_when_the_file_outgrows_the_inline_window() {
        use crate::buffer::Buffer;
        use crate::editor::Editor;
        let lines = |n: usize| Editor::new(Buffer::from_text(&"x\n".repeat(n)));
        let args = Args::default();
        // Fullscreen by default, whatever the size.
        assert!(use_fullscreen(&args, &Config::default(), &lines(1), 40));
        let cfg = Config {
            fullscreen: None,
            ..Config::default()
        };
        // "auto", default height 12: 12 lines fit inline, 13 don't.
        assert!(!use_fullscreen(&args, &cfg, &lines(12), 40));
        assert!(use_fullscreen(&args, &cfg, &lines(13), 40));
        // A short terminal shows fewer lines inline.
        assert!(use_fullscreen(&args, &cfg, &lines(8), 10));
        let inline = Args {
            inline: true,
            ..Args::default()
        };
        assert!(!use_fullscreen(&inline, &cfg, &lines(500), 40));
        let full = Args {
            fullscreen: true,
            ..Args::default()
        };
        assert!(use_fullscreen(&full, &cfg, &lines(1), 40));
        let never = Config {
            fullscreen: Some(false),
            ..Config::default()
        };
        assert!(!use_fullscreen(&args, &never, &lines(500), 40));
        let always = Config {
            fullscreen: Some(true),
            ..Config::default()
        };
        assert!(use_fullscreen(&args, &always, &lines(1), 40));
        let max = Config {
            height: usize::MAX,
            fullscreen: None,
            ..Config::default()
        };
        assert!(!use_fullscreen(&args, &max, &lines(30), 40));
        assert!(use_fullscreen(&args, &max, &lines(37), 40));
    }

    fn ev(code: C, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn maps_keys() {
        assert_eq!(
            map_key(ev(C::Char('r'), KeyModifiers::CONTROL)),
            Some(Key::ctrl('r'))
        );
        assert_eq!(
            map_key(ev(C::Char('R'), KeyModifiers::SHIFT)),
            Some(Key::ch('R'))
        );
        assert_eq!(
            map_key(ev(C::Tab, KeyModifiers::SHIFT)),
            Some(Key::new(KeyCode::BackTab))
        );
        assert_eq!(
            map_key(ev(C::Esc, KeyModifiers::NONE)),
            Some(Key::new(KeyCode::Esc))
        );
        assert_eq!(
            map_key(ev(C::Char('g'), KeyModifiers::CONTROL)),
            Some(Key::new(KeyCode::Esc))
        );
        let mut release = ev(C::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(map_key(release), None);
        assert_eq!(map_key(ev(C::F(1), KeyModifiers::NONE)), None);
    }

    #[test]
    fn alt_key_is_esc_then_key() {
        // Esc batched with the next key (SSH, tmux) arrives as Alt+key.
        assert_eq!(
            map_keys(ev(C::Char(':'), KeyModifiers::ALT)),
            vec![Key::new(KeyCode::Esc), Key::ch(':')]
        );
        assert_eq!(
            map_keys(ev(C::Char('d'), KeyModifiers::ALT | KeyModifiers::CONTROL)),
            vec![Key::new(KeyCode::Esc), Key::ctrl('d')]
        );
        assert_eq!(
            map_keys(ev(C::Char('x'), KeyModifiers::NONE)),
            vec![Key::ch('x')]
        );
    }

    #[test]
    fn ages() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(ago(now - 5), "5s ago");
        assert_eq!(ago(now - 180), "3m ago");
        assert_eq!(ago(now - 7200), "2h ago");
    }
}
