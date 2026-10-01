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
        for d in ["state", "config", "home"] {
            fs::create_dir(dir.path().join(d)).unwrap();
        }
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
fn window_is_inline_and_erased_on_exit() {
    let env = Env::new();
    env.write("f.txt", "secret contents\n");
    let mut c = env.command("sh");
    c.args(["-c", "echo before-marker; exec \"$0\" \"$@\"", BIN, "f.txt"]);
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
fn startup_errors() {
    let env = Env::new();
    fs::create_dir(env.path("adir")).unwrap();
    let mut p = env.fred(&["adir"]);
    assert_eq!(p.wait_exit(), 1);
    assert!(p.screen().contains("is a directory"), "{}", p.screen());

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
    c.args(["-c", "\"$0\" f.txt; echo \"exit=$?\"; stty -a", BIN]);
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
