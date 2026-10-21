//! The native front end: frames of styled runs for a helper-process protocol.
//! `fred --gui` reads [`proto::In`] lines on stdin and writes frames on
//! stdout; the Mac app draws them.
pub mod frame;
pub mod mac;
pub mod proto;

use crate::app::{self, AiJob};
use crate::args::{Args, LineArg};
use crate::complete::nearby;
use crate::config::Config;
use crate::editor::Mode;
use crate::ex::addr::Range;
use crate::highlight::Highlighter;
use crate::key::{Key, KeyCode};
use crate::magit::repo::GitInvocation;
use crate::session::{Session, SwapChoice};
use crate::swap;
use anyhow::{Result, anyhow};
use proto::In;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode as C, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::fd::FromRawFd;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Instant;

/// Work that holds the editor until it finishes; keys are dropped meanwhile,
/// as the terminal drops them while it is handed over.
enum Job {
    Ai(AiJob, Range, bool, Instant),
    Git(JoinHandle<Result<(), String>>, Box<GitInvocation>),
    Shell(JoinHandle<Result<String, String>>, String),
}

impl Job {
    fn is_finished(&self) -> bool {
        match self {
            Job::Ai(h, ..) => h.is_finished(),
            Job::Git(h, _) => h.is_finished(),
            Job::Shell(h, _) => h.is_finished(),
        }
    }
}

/// The protocol's own stdin and stdout. Fd 1 becomes stderr and fd 0
/// `/dev/null`, so no stray print and no child process (shell, Git, Claude)
/// can write into the frames or read the app's input.
fn take_stdio() -> Result<(File, File)> {
    // SAFETY: plain fd syscalls; the new fds are owned by the returned Files.
    unsafe {
        let input = libc::fcntl(0, libc::F_DUPFD_CLOEXEC, 3);
        let output = libc::fcntl(1, libc::F_DUPFD_CLOEXEC, 3);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
        if input < 0 || output < 0 || null < 0 {
            return Err(anyhow!("--gui: {}", std::io::Error::last_os_error()));
        }
        libc::dup2(2, 1);
        libc::dup2(null, 0);
        libc::close(null);
        Ok((File::from_raw_fd(input), File::from_raw_fd(output)))
    }
}

fn send(out: &mut impl Write, v: &impl serde::Serialize) -> std::io::Result<()> {
    serde_json::to_writer(&mut *out, v)?;
    out.write_all(b"\n")?;
    out.flush()
}

pub fn run(args: Args, cfg: Config, cfg_err: Option<String>) -> Result<i32> {
    let (input, output) = take_stdio()?;
    let mut out = BufWriter::new(output);
    let unknown = crate::magit::options::set(cfg.magit.0.clone());
    let cfg_err = cfg_err.or_else(|| unknown.into_iter().next());
    let (mut s, leftover) = match Session::open(args.file.clone(), &cfg, &swap::swap_dir()) {
        Ok(x) => x,
        Err(e) => {
            let name = args
                .file
                .as_ref()
                .map_or(String::new(), |p| format!("{}: ", p.display()));
            eprintln!("fred: {name}{e}");
            let _ = send(&mut out, &serde_json::json!({"t": "exit", "code": 1}));
            return Ok(1);
        }
    };
    match args.line {
        Some(LineArg::N(n)) => s.goto_line(Some(n)),
        Some(LineArg::Last) => s.goto_line(None),
        None => {}
    }
    // The terminal's "ansi" default follows the terminal's colors; the GUI has none, so it gets Material Dark.
    let theme = if cfg.theme == "ansi" { "material-dark" } else { &cfg.theme };
    s.hl = match Highlighter::new(theme, true) {
        Ok(h) => h,
        Err(e) => {
            s.ed.set_err(e);
            Highlighter::new("ansi", true).map_err(|e| anyhow!(e))?
        }
    };
    s.hl.set_file(
        s.ed.path.as_deref().or(s.ed.syntax_path.as_deref()),
        &s.ed.buf,
    );
    if let Some(r) = &s.hl.disabled {
        s.ed.set_msg(r.clone());
    }
    if let Some(e) = cfg_err {
        s.ed.set_err(e);
    }
    s.ed.nearby = nearby::Nearby::new(s.ed.path.clone());
    // ponytail: the swap question goes native in Phase 6; until then, read-only.
    if leftover.is_some() {
        s.ed.readonly = true;
        s.no_swap = true;
        s.ed.set_msg("swap file found: opened read-only");
    }
    s.lock();
    let remote = crate::remote::Remote::listen().ok();
    let stop = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(sig, Arc::clone(&stop))?;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(input).lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<In>(&line) {
                Ok(m) => {
                    if tx.send(m).is_err() {
                        break;
                    }
                }
                Err(e) => eprintln!("fred --gui: ignored {line:?}: {e}"),
            }
        }
        // Dropping `tx` tells the loop the app has gone.
    });

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        event_loop(&mut s, &cfg, &rx, &stop, remote.as_ref(), &mut out)
    }));
    let code = match result {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            s.write_swap();
            let _ = send(&mut out, &serde_json::json!({"t": "exit", "code": 1}));
            return Err(e);
        }
        Err(_) => {
            // The panic hook has already put the message on stderr.
            s.write_swap();
            if s.ed.buf.modified {
                eprintln!(
                    "fred: crashed; unsaved changes are in {} (open the file again to recover them)",
                    s.swap_path.display()
                );
            }
            let _ = send(&mut out, &serde_json::json!({"t": "exit", "code": 101}));
            return Ok(101);
        }
    };
    s.remember_place();
    if code == 0 {
        s.cleanup();
    }
    let _ = send(&mut out, &serde_json::json!({"t": "exit", "code": code}));
    Ok(code)
}

fn event_loop(
    s: &mut Session,
    cfg: &Config,
    rx: &Receiver<In>,
    stop: &AtomicBool,
    remote: Option<&crate::remote::Remote>,
    out: &mut impl Write,
) -> Result<i32> {
    let mut term = Terminal::new(TestBackend::new(80, 24))?;
    let mut seq = 0;
    let mut dirty = true;
    let mut checked = Instant::now();
    let mut active = Instant::now();
    let mut job: Option<Job> = None;
    // The close button during a job: quit once it is done.
    let mut quit_held = false;
    // Your input while Claude types, for after, and where it was typed.
    let mut held: Vec<In> = vec![];
    let mut held_at = None;
    loop {
        if !s.which_key
            && active.elapsed() >= app::WHICH_KEY
            && crate::vim::which_key(&s.ed).is_some()
        {
            s.which_key = true;
            dirty = true;
        }
        if dirty {
            send(out, &frame::build(s, cfg, &mut term, seq))?;
            // Keep drawing while visible lines are still being highlighted.
            dirty = s.hl_incomplete;
        }
        if stop.load(Ordering::Relaxed) {
            s.write_swap();
            return Ok(1);
        }
        let busy = dirty
            || active.elapsed() < app::IDLE_AFTER
            || matches!(s.ed.mode, Mode::Pick(_))
            || s.working()
            || job.is_some();
        let tick = match () {
            _ if s.in_terminal() && active.elapsed() < app::IDLE_AFTER => app::TERM_TICK,
            _ if s.typing.is_some() => app::TYPE_TICK,
            _ if busy => app::TICK,
            _ => app::IDLE_TICK,
        };
        let mut next = match rx.recv_timeout(tick) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            // EOF: the app died. As SIGHUP.
            Err(RecvTimeoutError::Disconnected) => {
                s.write_swap();
                return Ok(1);
            }
        };
        // Everything queued is handled before the next frame.
        while let Some(m) = next.take() {
            active = Instant::now();
            dirty = true;
            seq = seq_of(&m).unwrap_or(seq);
            let typed = !matches!(m, In::Resize { .. } | In::Quit);
            if s.typing.is_some() && typed {
                if matches!(&m, In::Keys { keys, .. } if keys == "<C-c>") {
                    s.stop_typing();
                } else if !matches!(m, In::Mouse { .. }) {
                    held_at.get_or_insert((s.cur, std::mem::discriminant(&s.ed.mode)));
                    held.push(m);
                }
            } else if job.is_some() && matches!(m, In::Quit) {
                quit_held = true;
            } else if job.is_none() || !typed {
                input(s, &mut term, m);
            }
            if s.quit {
                return Ok(0);
            }
            next = rx.try_recv().ok();
        }
        // Claude's requests wait while a job holds the editor (and are
        // refused as late), as the terminal blocks during one.
        let remote = if job.is_some() { None } else { remote };
        let (changed, acted) = app::tick_session(s, remote, &mut checked);
        if acted {
            active = Instant::now();
        }
        dirty |= changed;
        if s.typing.is_none() && !held.is_empty() {
            let ms = std::mem::take(&mut held);
            // Claude moved you: keys meant for where you were would run as
            // commands where it left you.
            if held_at.take() == Some((s.cur, std::mem::discriminant(&s.ed.mode))) {
                for m in ms {
                    input(s, &mut term, m);
                }
            } else {
                s.ed.set_err(format!(
                    "Claude moved you: {} keys you typed meanwhile were dropped",
                    ms.len()
                ));
            }
            dirty = true;
        }
        dirty |= tick_jobs(s, cfg, &mut job);
        if quit_held && job.is_none() {
            quit_held = false;
            input(s, &mut term, In::Quit);
            if s.quit {
                return Ok(0);
            }
        }
    }
}

fn seq_of(m: &In) -> Option<u64> {
    match m {
        In::Keys { seq, .. }
        | In::Paste { seq, .. }
        | In::Mouse { seq, .. }
        | In::Mac { seq, .. } => Some(*seq),
        In::Resize { .. } | In::Quit => None,
    }
}

/// Finish a job that is done, and start one the editor asked for.
fn tick_jobs(s: &mut Session, cfg: &Config, job: &mut Option<Job>) -> bool {
    if job.as_ref().is_some_and(Job::is_finished) {
        match job.take().expect("a job") {
            Job::Ai(h, r, explain, _) => app::finish_ai(s, r, explain, h.join()),
            Job::Git(h, inv) => {
                let result = h.join().unwrap_or_else(|_| Err("Git crashed".into()));
                s.finish_git(*inv, result);
            }
            Job::Shell(h, cmd) => {
                match h.join().unwrap_or_else(|_| Err("crashed".into())) {
                    // ponytail: output shown in the :explain box; a scrolling
                    // output buffer when long output bites.
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
        return true;
    }
    if let Some(Job::Ai(_, r, explain, start)) = job {
        app::ai_spin(s, *r, *explain, *start);
        return true;
    }
    if job.is_some() {
        return false;
    }
    if let Some(inv) = s.pending_git.take() {
        *job = Some(git_job(s, inv));
    } else if let Some(cmd) = s.pending_shell.take() {
        s.ed.set_msg(format!(":!{cmd}"));
        let run = cmd.clone();
        let h = std::thread::spawn(move || crate::shell::capture(&run, None));
        *job = Some(Job::Shell(h, cmd));
    } else if let Some((r, prompt, explain)) = s.pending_ai.take() {
        *job = Some(Job::Ai(
            app::start_ai(cfg, prompt, explain),
            r,
            explain,
            Instant::now(),
        ));
    } else if s.pending_edit.is_some() {
        // ponytail: the swap question goes native in Phase 6.
        s.resolve_edit(SwapChoice::ReadOnly);
    } else {
        return false;
    }
    true
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
            std::thread::spawn(move || stdin.write_all(&data));
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
    Job::Git(h, Box::new(inv))
}

/// One message from the app, as `step` handles a terminal event.
fn input(s: &mut Session, term: &mut Terminal<TestBackend>, m: In) {
    let keyboard = matches!(m, In::Keys { .. } | In::Paste { .. } | In::Mac { .. });
    let keep_view = s.ed.zap.is_some();
    if keyboard {
        s.win_mut().view.last_click = None;
    }
    let terminal = s.in_terminal() && s.ed.menu.is_none();
    match m {
        In::Resize { cols, rows } => {
            term.backend_mut().resize(cols, rows);
            s.ed.zap = None;
        }
        In::Keys { keys, .. } if terminal => {
            for k in crate::key::parse_keys(&keys) {
                s.term_key(crossterm_key(k));
            }
        }
        In::Keys { keys, .. } => handle_keys(s, &keys),
        In::Paste { text, .. } if terminal => s.term_paste(&text),
        In::Paste { text, .. } => s.ed.paste(&text),
        In::Mouse { kind, col, row, .. } => {
            let kind = match kind.as_str() {
                "down" => MouseEventKind::Down(MouseButton::Left),
                "up" => MouseEventKind::Up(MouseButton::Left),
                "drag" => MouseEventKind::Drag(MouseButton::Left),
                "right" => MouseEventKind::Down(MouseButton::Right),
                "scroll_up" => MouseEventKind::ScrollUp,
                "scroll_down" => MouseEventKind::ScrollDown,
                _ => return,
            };
            let area = term.backend().buffer().area;
            s.mouse(
                area,
                MouseEvent {
                    kind,
                    column: col,
                    row,
                    modifiers: KeyModifiers::NONE,
                },
            );
        }
        In::Mac { key, shift, .. } => mac::mac_key(s, key, shift),
        In::Quit => handle_keys(s, "<Esc>:qa<Enter>"),
    }
    if keyboard && !keep_view && s.ed.zap.is_none() && s.ed.vim.pending != [Key::ch(' ')] {
        s.win_mut().view.detached = false;
    }
    if let Some(d) = s.ed.buf.take_dirty_from() {
        s.hl.invalidate(d);
    }
}

/// Keys in Vim notation. Alt keys go whole to the buffers that bind them;
/// elsewhere Alt+X is Esc, X, as in the terminal.
fn handle_keys(s: &mut Session, keys: &str) {
    for k in crate::key::parse_keys(keys) {
        let whole = crate::magit::wants_alt(&s.ed)
            || crate::org::wants_alt(&s.ed)
            || crate::occur::wants_alt(&s.ed, k);
        if k.alt && !whole {
            s.handle_key(Key::new(KeyCode::Esc));
            s.handle_key(Key { alt: false, ..k });
        } else {
            s.handle_key(k);
        }
    }
}

/// A fred key as the crossterm key a terminal buffer takes.
fn crossterm_key(k: Key) -> KeyEvent {
    let code = match k.code {
        KeyCode::Char(c) => C::Char(c),
        KeyCode::Esc => C::Esc,
        KeyCode::Enter => C::Enter,
        KeyCode::Backspace => C::Backspace,
        KeyCode::Tab => C::Tab,
        KeyCode::BackTab => C::BackTab,
        KeyCode::Up => C::Up,
        KeyCode::Down => C::Down,
        KeyCode::Left => C::Left,
        KeyCode::Right => C::Right,
        KeyCode::Home => C::Home,
        KeyCode::End => C::End,
        KeyCode::PageUp => C::PageUp,
        KeyCode::PageDown => C::PageDown,
        KeyCode::Delete => C::Delete,
    };
    let mut m = KeyModifiers::NONE;
    m.set(KeyModifiers::CONTROL, k.ctrl);
    m.set(KeyModifiers::ALT, k.alt);
    m.set(KeyModifiers::SHIFT, k.shift || k.code == KeyCode::BackTab);
    KeyEvent::new(code, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_lines_parse() {
        let p = |l: &str| serde_json::from_str::<In>(l).unwrap();
        assert_eq!(
            p(r#"{"t":"keys","seq":7,"keys":"<C-w>j"}"#),
            In::Keys {
                seq: 7,
                keys: "<C-w>j".into()
            }
        );
        assert_eq!(
            p(r#"{"t":"mac","seq":10,"key":"s","shift":false}"#),
            In::Mac {
                seq: 10,
                key: 's',
                shift: false
            }
        );
        assert_eq!(p(r#"{"t":"quit"}"#), In::Quit);
        assert!(serde_json::from_str::<In>(r#"{"t":"bogus"}"#).is_err());
    }

    #[test]
    fn terminal_keys_keep_their_modifiers() {
        let k = crossterm_key(Key::ctrl('c'));
        assert_eq!((k.code, k.modifiers), (C::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(
            crossterm_key(Key::new(KeyCode::BackTab)).modifiers,
            KeyModifiers::SHIFT
        );
    }
}
