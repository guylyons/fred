//! End-to-end tests: run the real binary in a pseudo-terminal.

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_fred");
const ROWS: u16 = 24;
const COLS: u16 = 80;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        for d in ["state", "config/fred", "home"] {
            fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        // Tests mustn't touch the real clipboard.
        fs::write(
            dir.path().join("config/fred/config.toml"),
            "clipboard = false\n",
        )
        .unwrap();
        Env { dir }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.path(name);
        fs::write(&p, text).unwrap();
        p
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path(name)).unwrap()
    }
    fn swap_dir(&self) -> PathBuf {
        self.path("state/fred/swap")
    }
    fn swap_files(&self) -> Vec<PathBuf> {
        fs::read_dir(self.swap_dir()).map_or(vec![], |d| d.map(|e| e.unwrap().path()).collect())
    }
    fn command(&self, program: &str) -> CommandBuilder {
        let mut c = CommandBuilder::new(program);
        c.env_clear();
        c.env("PATH", std::env::var("PATH").unwrap_or_default());
        c.env("TERM", "xterm-256color");
        c.env("HOME", self.path("home"));
        c.env("XDG_STATE_HOME", self.path("state"));
        c.env("XDG_CONFIG_HOME", self.path("config"));
        c.cwd(self.dir.path());
        c
    }
    fn fred(&self, args: &[&str]) -> Pty {
        let mut c = self.command(BIN);
        c.args(args);
        Pty::spawn(c)
    }
}

struct Pty {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    parser: Arc<Mutex<vt100::Parser>>,
    raw: Arc<Mutex<Vec<u8>>>,
    master: Box<dyn MasterPty + Send>,
}

impl Pty {
    fn spawn(cmd: CommandBuilder) -> Pty {
        Pty::spawn_with(cmd, true)
    }

    /// `answer_dsr: false` plays a terminal that never reports the cursor.
    fn spawn_with(cmd: CommandBuilder, answer_dsr: bool) -> Pty {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
        let raw = Arc::new(Mutex::new(vec![]));
        let (p, w, rw) = (Arc::clone(&parser), Arc::clone(&writer), Arc::clone(&raw));
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                rw.lock().unwrap().extend_from_slice(chunk);
                let mut parser = p.lock().unwrap();
                parser.process(chunk);
                // Answer cursor position queries like a real terminal.
                if answer_dsr && chunk.windows(4).any(|x| x == b"\x1b[6n") {
                    let (r, c) = parser.screen().cursor_position();
                    let _ = write!(w.lock().unwrap(), "\x1b[{};{}R", r + 1, c + 1);
                }
            }
        });
        Pty {
            child,
            writer,
            parser,
            raw,
            master: pair.master,
        }
    }

    /// Send keys; each segment is written separately so Esc isn't read as Alt.
    fn keys(&self, segments: &[&str]) {
        for s in segments {
            {
                let mut w = self.writer.lock().unwrap();
                w.write_all(s.as_bytes()).unwrap();
                w.flush().unwrap();
            }
            thread::sleep(Duration::from_millis(80));
        }
    }

    /// Everything the program wrote, with escapes made visible.
    #[allow(dead_code)]
    fn raw(&self) -> String {
        String::from_utf8_lossy(&self.raw.lock().unwrap()).replace('\x1b', "<ESC>")
    }

    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if pred(&self.screen()) {
                return;
            }
            thread::sleep(Duration::from_millis(30));
        }
        panic!("timed out waiting for {what}; screen:\n{}", self.screen());
    }

    fn wait_text(&self, text: &str) {
        self.wait_for(text, |s| s.contains(text));
    }

    fn wait_exit(&mut self) -> u32 {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if let Some(st) = self.child.try_wait().unwrap() {
                thread::sleep(Duration::from_millis(100));
                return st.exit_code();
            }
            thread::sleep(Duration::from_millis(30));
        }
        let _ = self.child.kill();
        panic!("fred did not exit; screen:\n{}", self.screen());
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn signal(&self, sig: i32) {
        let pid = self.child.process_id().unwrap() as i32;
        // SAFETY: sending a signal to our own child process.
        unsafe {
            libc::kill(pid, sig);
        }
    }

    fn resize(&self, rows: u16, cols: u16) {
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }
}

#[test]
fn edit_and_save() {
    let env = Env::new();
    env.write("f.txt", "hello\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("hello");
    p.wait_text("NORMAL");
    p.keys(&["A world", "\x1b", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "hello world\n");
    assert!(
        p.screen().contains("\"f.txt\" 1L, 12B written"),
        "{}",
        p.screen()
    );
    assert!(env.swap_files().is_empty());
}

#[test]
fn fullscreen_for_a_big_file_and_screen_restored_on_exit() {
    let env = Env::new();
    let filler: String = (1..=30).map(|i| format!("filler {i}\n")).collect();
    env.write("f.txt", &format!("secret contents\n{filler}"));
    let mut c = env.command("sh");
    c.args(["-c", "echo before-marker; exec \"$0\" \"$@\"", BIN, "f.txt"]);
    let mut p = Pty::spawn(c);
    p.wait_text("secret contents");
    let s = p.screen();
    assert!(!s.contains("before-marker"), "not fullscreen:\n{s}");
    // The status line is on the terminal's last-but-one row.
    let rows: Vec<&str> = s.lines().collect();
    assert!(rows.len() >= ROWS as usize - 1, "{s}");
    assert!(rows[ROWS as usize - 2].contains("NORMAL"), "{s}");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    let s = p.screen();
    assert!(s.contains("before-marker"), "{s}");
    assert!(!s.contains("secret contents"), "{s}");
    assert!(p.raw().contains("<ESC>[?1049h") && p.raw().contains("<ESC>[?1049l"));
}

#[test]
fn fullscreen_unless_inline_is_asked_for() {
    let env = Env::new();
    env.write("small.txt", "one\ntwo\nthree\n");
    let run = |env: &Env, args: &[&str]| -> bool {
        let mut c = env.command("sh");
        c.args(["-c", "echo before-marker; exec \"$0\" \"$@\""]);
        c.arg(BIN);
        c.args(args);
        let mut p = Pty::spawn(c);
        p.wait_text("three");
        p.wait_text("NORMAL");
        let inline = p.screen().contains("before-marker");
        p.keys(&[":q\r"]);
        assert_eq!(p.wait_exit(), 0);
        inline
    };
    // Even a 3-line file: fullscreen is the default.
    assert!(!run(&env, &["small.txt"]));
    assert!(run(&env, &["-i", "small.txt"]));
    fs::create_dir_all(env.path("config/fred")).unwrap();
    env.write(
        "config/fred/config.toml",
        "fullscreen = false\nclipboard = false\n",
    );
    assert!(run(&env, &["small.txt"]));
    assert!(!run(&env, &["-f", "small.txt"]));
    // "auto" is still there: small files inline.
    env.write(
        "config/fred/config.toml",
        "fullscreen = \"auto\"\nclipboard = false\n",
    );
    assert!(run(&env, &["small.txt"]));
}

#[test]
fn reopening_a_file_returns_to_the_last_position() {
    let env = Env::new();
    let text: String = (1..=60).map(|i| format!("line {i}\n")).collect();
    env.write("f.txt", &text);
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["41G", "w"]);
    p.wait_text("41:6");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("41:6");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    // +LINE still wins.
    let mut p = env.fred(&["+5", "f.txt"]);
    p.wait_text("5:1");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn window_is_inline_and_erased_on_exit() {
    let env = Env::new();
    env.write("f.txt", "secret contents\n");
    let mut c = env.command("sh");
    c.args([
        "-c",
        "echo before-marker; exec \"$0\" \"$@\"",
        BIN,
        "--inline",
        "f.txt",
    ]);
    let mut p = Pty::spawn(c);
    p.wait_text("secret contents");
    assert!(p.screen().contains("before-marker"));
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    let s = p.screen();
    assert!(s.contains("before-marker"), "{s}\nraw: {:?}", p.raw());
    assert!(!s.contains("secret contents"), "window not erased:\n{s}");
    assert!(!s.contains("NORMAL"), "{s}");
}

#[test]
fn quit_refuses_unsaved_changes() {
    let env = Env::new();
    env.write("f.txt", "abc\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("abc");
    p.keys(&["x", ":q\r"]);
    p.wait_text("unsaved changes");
    assert!(p.running());
    p.keys(&[":q!\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "abc\n");
    assert!(env.swap_files().is_empty(), "discarded edits leave no swap");
}

#[test]
fn window_grows_with_the_file() {
    let env = Env::new();
    let mut c = env.command("sh");
    c.args([
        "-c",
        "echo before-marker; exec \"$0\" \"$@\"",
        BIN,
        "--inline",
        "new.txt",
    ]);
    let mut p = Pty::spawn(c);
    p.wait_text("[new]");
    p.keys(&[
        "ione", "\x1b", "otwo", "\x1b", "othree", "\x1b", "ofour", "\x1b",
    ]);
    p.wait_for("all four lines visible", |s| {
        ["one", "two", "three", "four"]
            .iter()
            .all(|w| s.contains(w))
    });
    assert!(
        p.screen().contains("before-marker"),
        "growing must not erase what is above:\n{}",
        p.screen()
    );
    p.keys(&[":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("new.txt"), "one\ntwo\nthree\nfour\n");
    assert!(p.screen().contains("before-marker"));
}

#[test]
fn completion_popup_in_real_terminal() {
    let env = Env::new();
    env.write("f.txt", "elephant\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("elephant");
    p.keys(&["oel", "\t", "\x1b", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "elephant\nelephant\n");
}

#[test]
fn syntax_highlighting_colors_code() {
    let env = Env::new();
    env.write("x.rs", "fn main() {}\n");
    let mut p = env.fred(&["x.rs"]);
    p.wait_text("fn main");
    let colored = {
        let parser = p.parser.lock().unwrap();
        let screen = parser.screen();
        let (r, _) = (0..ROWS)
            .flat_map(|r| (0..COLS).map(move |c| (r, c)))
            .find(|&(r, c)| {
                screen.cell(r, c).is_some_and(|x| x.contents() == "f")
                    && screen.cell(r, c + 1).is_some_and(|x| x.contents() == "n")
            })
            .expect("fn on screen");
        let c = (0..COLS)
            .find(|&c| screen.cell(r, c).is_some_and(|x| x.contents() == "f"))
            .unwrap();
        screen.cell(r, c).unwrap().fgcolor() != vt100::Color::Default
    };
    assert!(colored, "`fn` should be colored");
    assert!(p.screen().contains("rust"));
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn sigterm_leaves_swap_and_recovery_restores_it() {
    let env = Env::new();
    env.write("f.txt", "hello\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("hello");
    p.keys(&["ichanged ", "\x1b"]);
    thread::sleep(Duration::from_millis(1600));
    assert_eq!(env.swap_files().len(), 1, "swap written after a pause");
    p.signal(libc::SIGTERM);
    assert_eq!(p.wait_exit(), 1);
    assert_eq!(env.read("f.txt"), "hello\n", "the file itself is untouched");
    let swaps = env.swap_files();
    assert_eq!(swaps.len(), 1);
    assert!(
        fs::read_to_string(&swaps[0])
            .unwrap()
            .contains("changed hello")
    );

    let mut p = env.fred(&["f.txt"]);
    p.wait_text("swap found");
    p.keys(&["r"]);
    p.wait_text("recovered");
    p.keys(&[":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "changed hello\n");
    assert!(env.swap_files().is_empty());
}

#[test]
fn deleting_a_swap() {
    let env = Env::new();
    env.write("f.txt", "hello\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("hello");
    p.keys(&["x", "\x1b"]);
    thread::sleep(Duration::from_millis(1600));
    p.signal(libc::SIGHUP);
    assert_eq!(p.wait_exit(), 1);
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("swap found");
    p.keys(&["d"]);
    p.wait_text("NORMAL");
    // The old swap is gone; the only swap now is this fred's clean lock.
    let swaps = env.swap_files();
    assert_eq!(swaps.len(), 1);
    let text = fs::read_to_string(&swaps[0]).unwrap();
    assert!(
        text.contains("\"clean\":true") && !text.contains("ello"),
        "{text}"
    );
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "hello\n");
    assert!(env.swap_files().is_empty());
}

#[test]
fn survives_resize() {
    let env = Env::new();
    env.write("f.txt", "one\ntwo\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("two");
    p.resize(30, 50);
    thread::sleep(Duration::from_millis(200));
    p.keys(&["x"]);
    p.wait_text("[+]");
    p.resize(12, 100);
    thread::sleep(Duration::from_millis(200));
    p.keys(&[":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "ne\ntwo\n");
}

#[test]
fn fred_dir_opens_dired() {
    let env = Env::new();
    fs::create_dir_all(env.path("adir/sub")).unwrap();
    env.write("adir/sub/deep.txt", "found it\n");
    env.write("adir/top.txt", "top\n");
    let mut p = env.fred(&["adir"]);
    p.wait_text(" DIRED ");
    p.wait_text("sub/");
    p.wait_text("top.txt");
    // Vim search moves to an entry; Enter goes into a directory...
    p.keys(&["/sub\r", "\r"]);
    p.wait_text("deep.txt");
    // ...and opens a file.
    p.keys(&["/deep\r", "\r"]);
    p.wait_text("found it");
    // Space - lists the file's directory again, `-` goes up.
    p.keys(&[" -"]);
    p.wait_text("deep.txt");
    p.keys(&["-"]);
    p.wait_text("top.txt");
    // Space j still browses, vertico style.
    p.keys(&[" j"]);
    p.wait_text(" FILES ");
    p.keys(&["\x1b", ":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn startup_errors() {
    let out = std::process::Command::new(BIN)
        .arg("f")
        .stdin(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a terminal"));

    let out = std::process::Command::new(BIN)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("fred "));

    let out = std::process::Command::new(BIN)
        .args(["a", "b"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn start_line_argument() {
    let env = Env::new();
    env.write("f.txt", "a\nb\nc\n");
    let mut p = env.fred(&["+2", "f.txt"]);
    p.wait_text("2:1");
    p.keys(&["dd", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "a\nc\n");
}

#[test]
fn paging_with_height_max_moves_one_screen() {
    let env = Env::new();
    let text: String = (1..=64000).map(|i| format!("{i}\n")).collect();
    env.write("f.txt", &text);
    let mut p = env.fred(&["--height", "max", "f.txt"]);
    p.wait_text("NORMAL");
    // The window is ROWS - 2 tall: ROWS - 4 text rows plus status and command.
    let page = ROWS as usize - 4;
    p.keys(&["\x06"]);
    p.wait_text(&format!("{}:1", 1 + page));
    p.keys(&["\x06"]);
    p.wait_text(&format!("{}:1", 1 + 2 * page));
    p.keys(&["\x02"]);
    p.wait_text(&format!("{}:1", 1 + page));
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn space_p_finds_and_opens_a_file() {
    let env = Env::new();
    env.write("a.txt", "first\n");
    fs::create_dir_all(env.path("src/deep")).unwrap();
    env.write("src/deep/target.rs", "fn found() {}\n");
    let mut p = env.fred(&["a.txt"]);
    p.wait_text("NORMAL");
    p.keys(&[" p", "trgrs"]);
    p.wait_text("src/deep/target.rs");
    p.wait_text("find> trgrs");
    // Unsaved changes stay in a.txt's buffer; Ctrl-^ goes back to them.
    p.keys(&["\x1b", "x", " p", "trgrs", "\r"]);
    p.wait_text("fn found() {}");
    p.wait_text("\"src/deep/target.rs\" 1L");
    p.keys(&["\x1e"]);
    p.wait_text("irst");
    p.keys(&[":q\r"]);
    p.wait_text("unsaved changes (q! to discard");
    p.keys(&[":q!\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("a.txt"), "first\n");
}

#[test]
fn space_g_greps_and_lands_on_the_line() {
    let env = Env::new();
    env.write("a.txt", "first\n");
    let body: String = (1..=50).map(|i| format!("line {i}\n")).collect();
    env.write("b.txt", &format!("{body}    let needle = 1;\n"));
    let mut p = env.fred(&["a.txt"]);
    p.wait_text("NORMAL");
    p.keys(&[" g", "need.e"]);
    p.wait_text("b.txt:51: let needle = 1;");
    p.wait_text("1 matches");
    p.keys(&["\r"]);
    p.wait_text("51:9");
    // The pattern is the last search: n finds it again (wrapping around).
    p.keys(&["gg"]);
    p.wait_text(" 1:1 ");
    p.keys(&["n"]);
    p.wait_text("51:9");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn space_k_finds_a_line_in_the_file() {
    let env = Env::new();
    let body: String = (1..=80).map(|i| format!("row {i}\n")).collect();
    env.write("f.txt", &format!("{body}the target line\n"));
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&[" k", "targ line"]);
    p.wait_text("81: the target line");
    p.wait_text("1/81 lines");
    p.keys(&["\r"]);
    p.wait_text("81:5");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

/// Closing the terminal (a killed parent, a crashed test harness) without
/// a SIGHUP used to leave fred spinning at 100% CPU forever.
#[test]
fn exits_when_the_terminal_goes_away() {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    let env = Env::new();
    env.write("f.txt", "keep me\n");
    let (mut m, mut s) = (0, 0);
    let ws = libc::winsize {
        ws_row: ROWS,
        ws_col: COLS,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty fills in two new descriptors, owned below.
    let r = unsafe {
        libc::openpty(
            &mut m,
            &mut s,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &ws as *const _ as *mut _,
        )
    };
    assert_eq!(r, 0);
    // fred must not inherit our end, or it would keep its own terminal open.
    // SAFETY: setting a flag on a descriptor we own.
    unsafe { libc::fcntl(m, libc::F_SETFD, libc::FD_CLOEXEC) };
    let (master, slave) = unsafe { (fs::File::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };
    let mut cmd = std::process::Command::new(BIN);
    // Fullscreen: inline would wait on a cursor report this pty never sends.
    cmd.args(["-f", "f.txt"])
        .current_dir(env.dir.path())
        .env_clear()
        .env("TERM", "xterm-256color")
        .env("HOME", env.path("home"))
        .env("XDG_STATE_HOME", env.path("state"))
        .env("XDG_CONFIG_HOME", env.path("config"))
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    // Wait for the first frame, then close our end of the terminal.
    let mut seen = Vec::new();
    let mut reader = master.try_clone().unwrap();
    let start = Instant::now();
    while !String::from_utf8_lossy(&seen).contains("NORMAL") {
        assert!(start.elapsed() < Duration::from_secs(10), "no first frame");
        let mut b = [0u8; 4096];
        let n = reader.read(&mut b).unwrap();
        seen.extend_from_slice(&b[..n]);
    }
    drop(reader);
    drop(master);
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            panic!("fred kept running after its terminal closed");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(1), "{status:?}");
}

/// Idle means idle: no redraws (each would write at least the cursor).
#[test]
fn idle_picker_draws_nothing() {
    let env = Env::new();
    env.write("f.txt", "x\n");
    for args in [&["-f", "f.txt"][..], &["-i", "f.txt"][..]] {
        let mut p = env.fred(args);
        p.wait_text("NORMAL");
        p.keys(&[" k"]);
        p.wait_text("lines>");
        thread::sleep(Duration::from_millis(300));
        let before = p.raw().len();
        thread::sleep(Duration::from_millis(1000));
        assert_eq!(p.raw().len(), before, "{args:?}: redrawing while idle");
        p.keys(&["\x1b", ":q\r"]);
        assert_eq!(p.wait_exit(), 0);
    }
}

#[test]
fn git_marks_show_as_you_type() {
    let env = Env::new();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(env.dir.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !git(&["init", "-q"]) {
        return; // no git on this machine
    }
    env.write("f.txt", "one\ntwo\nthree\n");
    assert!(git(&["add", "f.txt"]));
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("three");
    // Staged and unchanged: a column for marks, but none shown.
    let marked = |s: &str, text: &str| s.lines().any(|l| l.starts_with('▎') && l.contains(text));
    assert!(!p.screen().contains('▎'), "{}", p.screen());
    p.keys(&["o", "added line", "\x1b"]);
    p.wait_for("added mark", |s| marked(s, "added line"));
    p.keys(&["ggcw", "ONE", "\x1b"]);
    p.wait_for("changed mark", |s| marked(s, "ONE"));
    assert!(!marked(&p.screen(), "three"));
    p.keys(&[":q!\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn file_pickers_dot_changed_files() {
    let env = Env::new();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(env.dir.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !git(&["init", "-q"]) {
        return; // no git on this machine
    }
    // Keep the env's own files out of `git status`.
    env.write(".gitignore", "state/\nconfig/\nhome/\n");
    env.write("changed.txt", "one\n");
    env.write("same.txt", "one\n");
    assert!(git(&["add", "."]) && git(&["commit", "-qm", "x"]));
    env.write("changed.txt", "two\n");
    env.write("fresh.txt", "new\n");
    let mut p = env.fred(&["same.txt"]);
    p.wait_text("NORMAL");
    p.keys(&[" p"]);
    // The dot sits in the column before the name: green new, yellow changed.
    let dot = |p: &Pty, name: &str| -> Option<vt100::Color> {
        let parser = p.parser.lock().unwrap();
        let screen = parser.screen();
        let (rows, _) = screen.size();
        (0..rows).find_map(|y| {
            let row = screen.contents_between(y, 0, y, 60);
            // A column, not a byte offset: `●` is 3 bytes wide.
            let x = row[..row.find(name)?].chars().count() as u16;
            let cell = screen.cell(y, x.checked_sub(2)?)?;
            (cell.contents() == "●").then(|| cell.fgcolor())
        })
    };
    p.wait_for("dots", |_| {
        dot(&p, "changed.txt").is_some() && dot(&p, "fresh.txt").is_some()
    });
    assert_eq!(dot(&p, "changed.txt"), Some(vt100::Color::Idx(3)));
    assert_eq!(dot(&p, "fresh.txt"), Some(vt100::Color::Idx(2)));
    assert_eq!(dot(&p, "same.txt"), None);
    p.keys(&["\x1b", ":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn shell_pwd_and_cd() {
    let env = Env::new();
    env.write("f.txt", "abc\n");
    fs::create_dir(env.path("sub")).unwrap();
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    // :! hands over the terminal, then waits for Enter.
    p.keys(&[":!echo hello-from-shell\r"]);
    p.wait_text("hello-from-shell");
    p.wait_text("Press Enter to continue");
    p.keys(&["\r"]);
    p.wait_text("NORMAL");
    // :cd and :pwd; the file keeps its own path.
    p.keys(&[":cd sub\r"]);
    p.wait_for("cd", |s| s.lines().any(|l| l.trim_end().ends_with("/sub")));
    p.keys(&[":pwd\r"]);
    p.wait_for("pwd", |s| s.lines().any(|l| l.trim_end().ends_with("/sub")));
    p.keys(&["x", ":w\r"]);
    // (The file's path is absolute now: `written` is past the edge.)
    p.wait_text("f.txt\" 1L");
    assert_eq!(env.read("f.txt"), "bc\n");
    assert!(
        !env.path("sub/f.txt").exists(),
        "wrote into the new directory"
    );
    // A filter, from the editor.
    p.keys(&["ofoo bar", "\x1b", ":.!tr a-z A-Z\r"]);
    p.wait_text("FOO BAR");
    p.keys(&[":q!\r"]);
    assert_eq!(p.wait_exit(), 0);
}

/// Prints what the window looks like (run with `--ignored --nocapture`).
#[test]
#[ignore]
fn show_screen() {
    let env = Env::new();
    env.write("main.rs", "use std::io;\n\nfn main() {\n    let greeting = \"hello\";\n    let greeting_len = greeting.len();\n    println!(\"{greeting} {greeting_len}\");\n}\n");
    let mut c = env.command("sh");
    c.args([
        "-c",
        "echo '$ ls'; echo 'main.rs'; echo '$ fred main.rs'; exec \"$0\" \"$@\"",
        BIN,
        "main.rs",
    ]);
    let mut p = Pty::spawn(c);
    p.wait_text("NORMAL");
    println!("--- normal mode ---\n{}", p.screen().trim_end());
    p.keys(&["5G", "o", "let x = gre"]);
    p.wait_text("INSERT");
    thread::sleep(Duration::from_millis(200));
    println!(
        "--- insert with completion popup ---\n{}",
        p.screen().trim_end()
    );
    p.keys(&["\x1b", ":s/x/y/"]);
    thread::sleep(Duration::from_millis(200));
    println!("--- typing an ex command ---\n{}", p.screen().trim_end());
    p.keys(&["\r", ":wq\r"]);
    p.wait_exit();
    println!("--- after :wq ---\n{}", p.screen().trim_end());
}

#[test]
fn esc_batched_with_next_keys() {
    // Over SSH or in tmux, Esc and the keys after it arrive in one read.
    let env = Env::new();
    env.write("f.txt", "x\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["ihello \x1b:wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "hello x\n");
}

#[test]
fn terminal_restored_when_startup_fails() {
    let env = Env::new();
    env.write("f.txt", "x\n");
    let mut c = env.command("sh");
    // Inline: a terminal that never reports the cursor fails at startup.
    c.args([
        "-c",
        "\"$0\" --inline f.txt; echo \"exit=$?\"; stty -a",
        BIN,
    ]);
    let mut p = Pty::spawn_with(c, false);
    p.wait_text("exit=1");
    p.wait_text("icanon");
    let s = p.screen();
    assert!(
        !s.contains("-icanon") && !s.contains("-echo "),
        "terminal left in raw mode:\n{s}"
    );
    assert!(p.raw().contains("<ESC>[?2004l"), "bracketed paste left on");
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn panic_keeps_unsaved_work_and_reports() {
    let env = Env::new();
    env.write("f.txt", "hello\n");
    let mut c = env.command(BIN);
    c.arg("f.txt");
    c.env("FRED_DEBUG_PANIC", "1");
    let mut p = Pty::spawn(c);
    p.wait_text("hello");
    // No pause: the regular swap timer hasn't fired when the panic hits.
    p.keys(&["iPANIC "]);
    assert_eq!(p.wait_exit(), 101);
    assert_eq!(env.read("f.txt"), "hello\n");
    let swaps = env.swap_files();
    assert_eq!(swaps.len(), 1, "a panic must leave a swap file");
    assert!(
        fs::read_to_string(&swaps[0])
            .unwrap()
            .contains("PANIC hello")
    );
    let s = p.screen();
    assert!(
        s.contains("debug panic while drawing"),
        "panic message lost:\n{s}"
    );
    assert!(s.contains("unsaved changes are in"), "{s}");
    assert!(p.raw().contains("<ESC>[?2004l"), "terminal not restored");
}

#[test]
fn ai_spins_while_waiting_then_replaces_the_lines() {
    let env = Env::new();
    env.write("f.txt", "a\nb\nc\n");
    // The fake Claude fails unless ai_rules reached the prompt.
    env.write(
        "config/fred/config.toml",
        r#"clipboard = false
ai_rules = "be terse"
ai_command = '''p=$(cat); printf %s "$p" | grep -q 'Rules: be terse' || exit 1; sleep 1; echo BEE'''
"#,
    );
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["j", ":ai shout\r"]);
    p.wait_text("Claude is rewriting line 2");
    // Held: keys typed while waiting are dropped.
    p.keys(&["dd"]);
    p.wait_text("Claude rewrote line 2");
    p.keys(&[":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "a\nBEE\nc\n");
}
