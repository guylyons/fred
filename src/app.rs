//! The terminal front end: inline viewport, event loop, signals.

use crate::args::{Args, LineArg};
use crate::complete::nearby;
use crate::config::Config;
use crate::editor::Mode;
use crate::fileio;
use crate::highlight::Highlighter;
use crate::key::{Key, KeyCode};
use crate::session::Session;
use crate::swap::{self, SwapInfo};
use crate::ui::{self, View, window_height};
use anyhow::{Result, anyhow};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{self, MoveTo, SetCursorStyle};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, Clear, ClearType};
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io::{self, IsTerminal, Stdout, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_millis(50);
const HIGHLIGHT_BUDGET: Duration = Duration::from_millis(20);

type Term = Terminal<CrosstermBackend<Stdout>>;

fn open_term(height: u16) -> io::Result<Term> {
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )
}

/// The inline window.
struct Ui {
    term: Term,
    area: Option<Rect>,
    height: u16,
    view: View,
    bar_cursor: bool,
}

impl Ui {
    fn new(height: u16) -> io::Result<Ui> {
        Ok(Ui {
            term: open_term(height)?,
            area: None,
            height,
            view: View::default(),
            bar_cursor: false,
        })
    }

    fn draw(&mut self, s: &Session, hl: &mut Highlighter, cfg: &Config) -> io::Result<()> {
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

    /// Erase the window and leave the cursor where it started.
    fn erase(&mut self) -> io::Result<()> {
        let y = self.area.map_or(0, |a| a.y);
        execute!(io::stdout(), MoveTo(0, y), Clear(ClearType::FromCursorDown))
    }

    /// Recreate the window (terminal resized, or the window needs to grow).
    fn rebuild(&mut self, height: u16) -> io::Result<()> {
        self.erase()?;
        self.term = open_term(height)?;
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
            DisableBracketedPaste
        )?;
        terminal::disable_raw_mode()?;
        io::stdout().flush()
    }

    fn reopen(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnableBracketedPaste)?;
        self.term = open_term(self.height)?;
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

/// Put the terminal back the way the shell expects it.
fn restore_terminal() {
    let _ = execute!(
        io::stdout(),
        cursor::Show,
        SetCursorStyle::DefaultUserShape,
        DisableBracketedPaste
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
        println!();
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

/// Ask what to do with a leftover swap file. Returns false to quit.
fn swap_prompt(
    ui: &mut Ui,
    s: &mut Session,
    hl: &mut Highlighter,
    cfg: &Config,
    info: SwapInfo,
) -> Result<bool> {
    let alive = swap::owner_alive(&info);
    let prompt = if alive {
        format!(
            "swap: file is open in fred (pid {}): [o]pen read-only, [q]uit",
            info.pid
        )
    } else {
        format!(
            "swap found (saved {}): [r]ecover, [d]elete, [q]uit",
            ago(info.saved_at)
        )
    };
    loop {
        s.ed.msg = Some((prompt.clone(), false));
        ui.draw(s, hl, cfg)?;
        let Event::Key(k) = event::read()? else {
            continue;
        };
        let Some(key) = map_keys(k).pop() else {
            continue;
        };
        match (alive, key.char(), key.code) {
            (true, Some('o'), _) => {
                s.ed.readonly = true;
                s.no_swap = true;
                s.ed.set_msg("opened read-only");
                return Ok(true);
            }
            (false, Some('r'), _) => {
                s.recover(info);
                hl.set_file(s.ed.path.as_deref(), &s.ed.buf);
                return Ok(true);
            }
            (false, Some('d'), _) => {
                s.discard_swap();
                s.ed.msg = None;
                return Ok(true);
            }
            (_, Some('q'), _) | (_, _, KeyCode::Esc) => return Ok(false),
            _ => {}
        }
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
        Event::Resize(..) => *resized = true,
        _ => {}
    }
    if let Some(d) = s.ed.buf.take_dirty_from() {
        hl.invalidate(d);
    }
}

fn suspend(ui: &mut Ui, s: &mut Session) -> Result<()> {
    s.write_swap();
    ui.close()?;
    // SAFETY: raising a signal has no memory-safety preconditions.
    unsafe {
        libc::raise(libc::SIGTSTP);
    }
    ui.reopen()?;
    if let Some(p) = &s.ed.path
        && fileio::changed_on_disk(p, s.stamp.as_ref())
    {
        s.ed.set_err("file changed on disk since it was read");
    }
    Ok(())
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
        execute!(io::stdout(), EnableBracketedPaste)?;
        let rows = terminal::size()?.1;
        Ok(Ui::new(window_height(cfg.height, s.ed.line_count(), rows))?)
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
            eprintln!("fred: {}", note.as_deref().unwrap_or("panicked"));
            if s.ed.buf.modified {
                eprintln!(
                    "fred: crashed; unsaved changes are in {} (open the file again to recover them)",
                    s.swap_path.display()
                );
            }
            return Ok(101);
        }
    };
    ui.close()?;
    if code == 0 {
        s.cleanup();
        if let Some(w) = &s.written {
            println!("{w}");
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
    if let Some(info) = leftover
        && !swap_prompt(ui, s, hl, cfg, info)?
    {
        s.no_swap = true;
        return Ok(0);
    }
    loop {
        ui.draw(s, hl, cfg)?;
        if stop.load(Ordering::Relaxed) {
            s.write_swap();
            return Ok(1);
        }
        let mut resized = false;
        if event::poll(TICK)? {
            loop {
                let ev = event::read()?;
                if let Event::Key(k) = ev
                    && map_key(k) == Some(Key::ctrl('z'))
                {
                    suspend(ui, s)?;
                    break;
                }
                step(s, hl, ev, &mut resized);
                if s.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        if s.quit {
            return Ok(0);
        }
        if s.reloaded {
            s.reloaded = false;
            hl.set_file(s.ed.path.as_deref(), &s.ed.buf);
            s.ed.nearby = nearby::spawn(s.ed.path.clone());
            ui.view = View::default();
        }
        let rows = terminal::size()?.1;
        let shown = (ui.height as usize).saturating_sub(2);
        let want = window_height(cfg.height, s.ed.line_count().max(shown), rows);
        if resized || want > ui.height {
            ui.rebuild(want)?;
        }
        s.maybe_swap(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use event::KeyCode as C;

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
