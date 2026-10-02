//! Exploratory end-to-end tests: run the real binary in a pseudo-terminal and
//! poke at the edges (tiny terminals, suspend/resume, batched input, pastes,
//! wrap mode, odd files, concurrent freds, performance).
//!
//! Tests whose names start with `bug_` demonstrate a problem and currently
//! FAIL with an explanation. The `perf_*` tests and `fuzz_*` are `#[ignore]`d;
//! the perf tests use the release binary and print their measurements:
//!
//! ```text
//! cargo build --release
//! cargo test --test explore perf_ -- --ignored --nocapture --test-threads=1
//! ```

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_fred");
const ROWS: u16 = 24;
const COLS: u16 = 80;

// ===================================================================== harness

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        for d in ["state", "config/fred", "home"] {
            fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        // These explorations are about the inline window (fullscreen is
        // covered in e2e.rs); `config` keeps this line too.
        fs::write(
            dir.path().join("config/fred/config.toml"),
            "fullscreen = false\nclipboard = false\n",
        )
        .unwrap();
        Env { dir }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        self.write_bytes(name, text.as_bytes())
    }
    fn write_bytes(&self, name: &str, data: &[u8]) -> PathBuf {
        let p = self.path(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, data).unwrap();
        p
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path(name)).unwrap()
    }
    fn read_bytes(&self, name: &str) -> Vec<u8> {
        fs::read(self.path(name)).unwrap()
    }
    fn config(&self, text: &str) {
        self.write(
            "config/fred/config.toml",
            &format!("fullscreen = false\n{text}"),
        );
    }
    fn swap_dir(&self) -> PathBuf {
        self.path("state/fred/swap")
    }
    /// Swap files (`*.swp`; not fred's `*.swp.tmpPID` files mid-write).
    fn swap_files(&self) -> Vec<PathBuf> {
        fs::read_dir(self.swap_dir()).map_or(vec![], |d| {
            d.map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|x| x == "swp"))
                .collect()
        })
    }
    /// Contents of all swap files.
    fn swap_texts(&self) -> Vec<String> {
        self.swap_files()
            .iter()
            .map(|s| fs::read_to_string(s).unwrap_or_default())
            .collect()
    }
    fn command(&self, program: &str) -> CommandBuilder {
        let mut c = CommandBuilder::new(program);
        c.env_clear();
        c.env("PATH", std::env::var("PATH").unwrap_or_default());
        c.env("TERM", "xterm-256color");
        c.env("HOME", self.path("home"));
        c.env("XDG_STATE_HOME", self.path("state"));
        c.env("XDG_CONFIG_HOME", self.path("config"));
        c.env("BASH_SILENCE_DEPRECATION_WARNING", "1");
        c.cwd(self.dir.path());
        c
    }
    fn fred(&self, args: &[&str]) -> Pty {
        self.fred_sized(args, ROWS, COLS)
    }
    fn fred_sized(&self, args: &[&str], rows: u16, cols: u16) -> Pty {
        self.spawn_bin(BIN, args, rows, cols)
    }
    fn spawn_bin(&self, bin: &str, args: &[&str], rows: u16, cols: u16) -> Pty {
        let mut c = self.command(bin);
        c.args(args);
        Pty::spawn_sized(c, rows, cols)
    }
    /// An interactive bash with job control and the prompt `PROMPT$ `;
    /// `$FRED` is the fred binary.
    fn bash(&self, rows: u16, cols: u16) -> Pty {
        let mut c = self.command("/bin/bash");
        c.args(["--norc", "--noprofile", "-i"]);
        c.env("PS1", "PROMPT$ ");
        c.env("FRED", BIN);
        Pty::spawn_sized(c, rows, cols)
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
    fn spawn_sized(cmd: CommandBuilder, rows: u16, cols: u16) -> Pty {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let raw = Arc::new(Mutex::new(vec![]));
        let (p, w, rw) = (Arc::clone(&parser), Arc::clone(&writer), Arc::clone(&raw));
        thread::spawn(move || {
            let mut buf = [0u8; 65536];
            let mut tail: Vec<u8> = vec![];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                rw.lock().unwrap().extend_from_slice(chunk);
                let mut parser = p.lock().unwrap();
                // vt100 itself has debug-mode overflow panics on 1-row screens;
                // don't let them take the harness down (start a fresh screen).
                let feed = |parser: &mut vt100::Parser, bytes: &[u8]| {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        parser.process(bytes)
                    }));
                    if r.is_err() {
                        let (rows, cols) = parser.screen().size();
                        *parser = vt100::Parser::new(rows, cols, 0);
                    }
                };
                // Answer cursor position queries like a real terminal: process
                // the output up to each query, then reply with the cursor
                // position (queries split across reads are handled too).
                let mut scan = tail.clone();
                scan.extend_from_slice(chunk);
                let offset = tail.len();
                let mut done = 0;
                let mut i = 0;
                while i + 4 <= scan.len() {
                    if &scan[i..i + 4] == b"\x1b[6n" {
                        let end = (i + 4).saturating_sub(offset).min(chunk.len());
                        feed(&mut parser, &chunk[done..end]);
                        done = end;
                        let (r, c) = parser.screen().cursor_position();
                        let _ = write!(w.lock().unwrap(), "\x1b[{};{}R", r + 1, c + 1);
                        i += 4;
                    } else {
                        i += 1;
                    }
                }
                feed(&mut parser, &chunk[done..]);
                let keep = scan.len().min(3);
                tail = scan[scan.len() - keep..].to_vec();
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

    /// Send keys; each segment is written separately (with a pause) so Esc
    /// isn't read as Alt.
    fn keys(&self, segments: &[&str]) {
        for s in segments {
            self.send(s);
            thread::sleep(Duration::from_millis(80));
        }
    }

    /// One write, no pause afterwards.
    fn send(&self, s: &str) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(s.as_bytes()).unwrap();
        w.flush().unwrap();
    }

    /// Everything the program wrote, with escapes made visible.
    fn raw(&self) -> String {
        String::from_utf8_lossy(&self.raw.lock().unwrap()).replace('\x1b', "<ESC>")
    }

    fn raw_len(&self) -> usize {
        self.raw.lock().unwrap().len()
    }

    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    /// Screen rows, trailing blanks trimmed.
    fn rows(&self) -> Vec<String> {
        let parser = self.parser.lock().unwrap();
        let s = parser.screen();
        let (_, cols) = s.size();
        s.rows(0, cols).map(|r| r.trim_end().to_string()).collect()
    }

    fn cursor(&self) -> (u16, u16) {
        self.parser.lock().unwrap().screen().cursor_position()
    }

    /// Contents of the screen cell under the terminal cursor.
    fn cell_at_cursor(&self) -> String {
        let parser = self.parser.lock().unwrap();
        let (r, c) = parser.screen().cursor_position();
        parser
            .screen()
            .cell(r, c)
            .map(|c| c.contents().to_string())
            .unwrap_or_default()
    }

    fn try_wait_for(&self, timeout: Duration, pred: impl Fn(&str) -> bool) -> Option<Duration> {
        let start = Instant::now();
        loop {
            if pred(&self.screen()) {
                return Some(start.elapsed());
            }
            if start.elapsed() > timeout {
                return None;
            }
            thread::sleep(Duration::from_micros(500));
        }
    }

    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) -> Duration {
        self.wait_for_within(what, Duration::from_secs(10), pred)
    }

    fn wait_for_within(
        &self,
        what: &str,
        timeout: Duration,
        pred: impl Fn(&str) -> bool,
    ) -> Duration {
        match self.try_wait_for(timeout, pred) {
            Some(d) => d,
            None => panic!("timed out waiting for {what}; screen:\n{}", self.screen()),
        }
    }

    fn wait_text(&self, text: &str) -> Duration {
        self.wait_for(text, |s| s.contains(text))
    }

    fn exit_status(&mut self, timeout: Duration) -> Option<portable_pty::ExitStatus> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(st) = self.child.try_wait().unwrap() {
                thread::sleep(Duration::from_millis(100));
                return Some(st);
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    fn wait_exit(&mut self) -> u32 {
        match self.exit_status(Duration::from_secs(10)) {
            Some(st) => st.exit_code(),
            None => {
                let _ = self.child.kill();
                panic!("fred did not exit; screen:\n{}", self.screen());
            }
        }
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn pid(&self) -> i32 {
        self.child.process_id().unwrap() as i32
    }

    fn signal(&self, sig: i32) {
        // SAFETY: sending a signal to our own child process.
        unsafe {
            libc::kill(self.pid(), sig);
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

impl Drop for Pty {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
    }
}

fn wait_until(what: &str, timeout: Duration, cond: impl Fn() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

/// The status row (` MODE  name ...  line:col`).
fn status_row(p: &Pty) -> String {
    p.rows()
        .into_iter()
        .rev()
        .find(|r| r.contains("NORMAL") || r.contains("INSERT") || r.contains("COMMAND"))
        .unwrap_or_default()
}

/// The row under the status row (messages, `:` input).
fn cmd_row(p: &Pty) -> String {
    let rows = p.rows();
    let i = rows
        .iter()
        .rposition(|r| r.contains("NORMAL") || r.contains("INSERT") || r.contains("COMMAND"))
        .unwrap_or(0);
    rows.get(i + 1).cloned().unwrap_or_default()
}

fn tail(s: &str, n: usize) -> String {
    let v: Vec<char> = s.chars().collect();
    v[v.len().saturating_sub(n)..].iter().collect()
}

/// The `pid` field of a swap file's JSON header.
fn swap_pid(path: &Path) -> Option<u32> {
    let text = fs::read_to_string(path).ok()?;
    let head = text.lines().next()?;
    let i = head.find("\"pid\":")? + 6;
    head[i..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// `a`..`z` repeated, `n` chars.
fn letters(n: usize) -> String {
    (0..n).map(|i| (b'a' + (i % 26) as u8) as char).collect()
}

/// The special line used for cursor placement checks: tab, CJK, emoji with
/// skin tone, ZWJ family, combining accent, control char.
const SPECIAL: &str = "a\tb漢字c👍🏽d👨‍👩‍👧ex\u{301}f\u{1}g";

// ============================================== 1. performance (release build)

fn release_bin() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/fred");
    assert!(
        p.exists(),
        "build the release binary first: cargo build --release"
    );
    p.display().to_string()
}

/// Resident set size of a process in KiB.
fn rss_kb(pid: i32) -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// CPU time (user+sys) of a process in seconds.
fn cpu_secs(pid: i32) -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // "M:SS.ss" or "H:MM:SS"
    s.split(':')
        .filter_map(|p| p.parse::<f64>().ok())
        .fold(0.0, |acc, v| acc * 60.0 + v)
}

/// Rust-like source: groups of 5 lines (~48 bytes/line), 5 `foo`s per group.
fn rust_like(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 50);
    let mut i = 0;
    let mut n = 0;
    while n < lines {
        s.push_str(&format!(
            "/// Documentation for function_{i}: adds foo to the length of bar and doubles it.\n\
             pub fn function_{i}(foo: u32, bar: &str) -> u32 {{\n\
             \x20   let value_{i} = foo.wrapping_add(bar.len() as u32); // foo\n\
             \x20   value_{i}.wrapping_mul(2) + foo\n\
             }}\n"
        ));
        i += 1;
        n += 5;
    }
    s
}

/// ~`bytes` of minified JSON on one line.
fn minified_json(bytes: usize) -> String {
    let mut s = String::from("{\"data\":[");
    let mut i = 0;
    while s.len() < bytes {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"id\":{i},\"name\":\"item-{i}\",\"tags\":[\"alpha\",\"beta\"],\"active\":true,\"score\":0.{i}}}"
        ));
        i += 1;
    }
    s.push_str("]}\n");
    s
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn p90(v: &[Duration]) -> Duration {
    let mut s = v.to_vec();
    s.sort();
    s[((s.len() - 1) as f64 * 0.9).round() as usize]
}

fn summary(v: &[Duration]) -> String {
    let mut s = v.to_vec();
    s.sort();
    let pick = |q: f64| s[((s.len() - 1) as f64 * q).round() as usize];
    format!(
        "n={} min {} / median {} / p90 {} / max {}",
        s.len(),
        ms(s[0]),
        ms(pick(0.5)),
        ms(pick(0.9)),
        ms(s[s.len() - 1])
    )
}

/// Type ASCII `text` one key at a time (the cursor must be on screen row
/// `row`), waiting for each char to be drawn and the terminal cursor to move
/// past it. Returns the per-key latency.
fn type_measured(p: &Pty, row: usize, text: &str) -> Vec<Duration> {
    let mut out = vec![];
    let mut typed = String::new();
    let start_col = p.cursor().1 as usize;
    for c in text.chars() {
        typed.push(c);
        let t = Instant::now();
        p.send(&c.to_string());
        let want = typed.trim_end().to_string();
        let col = (start_col + typed.len()) as u16;
        let ok = p.try_wait_for(Duration::from_secs(20), |_| {
            p.cursor() == (row as u16, col) && p.rows().get(row).is_some_and(|r| r.contains(&want))
        });
        assert!(
            ok.is_some(),
            "typed {typed:?} never showed up:\n{}",
            p.screen()
        );
        out.push(t.elapsed());
    }
    out
}

/// Time from writing `keys` until `pred` holds.
fn timed(p: &Pty, keys: &str, what: &str, pred: impl Fn(&str) -> bool) -> Duration {
    let t = Instant::now();
    p.send(keys);
    p.wait_for_within(what, Duration::from_secs(120), pred);
    t.elapsed()
}

/// Measurements, printed as they come; thresholds collected as problems.
#[derive(Default)]
struct Report {
    problems: Vec<String>,
}

impl Report {
    fn add(&mut self, k: &str, v: String) {
        println!("  {k:<55} {v}");
    }
    /// A single action; flag it over `limit_ms`.
    fn action(&mut self, k: &str, d: Duration, limit_ms: f64) {
        self.add(k, ms(d));
        if d.as_secs_f64() * 1000.0 > limit_ms {
            self.problems
                .push(format!("{k}: {} (limit {limit_ms} ms)", ms(d)));
        }
    }
    /// Per-keystroke latencies; flag a p90 over 100 ms.
    fn keys(&mut self, k: &str, v: &[Duration]) {
        self.add(k, summary(v));
        if p90(v) > Duration::from_millis(100) {
            self.problems
                .push(format!("{k}: p90 {} (limit 100 ms)", ms(p90(v))));
        }
    }
    /// CPU used while idle; flag more than 10% of a core.
    fn idle_cpu(&mut self, k: &str, cpu: f64, window: f64) {
        self.add(
            k,
            format!(
                "{cpu:.2} s CPU in {window:.0} s ({:.0}%)",
                cpu / window * 100.0
            ),
        );
        if cpu / window > 0.10 {
            self.problems.push(format!(
                "{k}: {:.0}% of a core while idle (limit 10%)",
                cpu / window * 100.0
            ));
        }
    }
    fn check(&mut self, k: &str, ok: bool, detail: String) {
        self.add(k, format!("{ok} {detail}"));
        if !ok {
            self.problems.push(format!("{k}: {detail}"));
        }
    }
    fn finish(self) {
        assert!(
            self.problems.is_empty(),
            "performance problems:\n  {}",
            self.problems.join("\n  ")
        );
    }
}

const TYPED: &str = "let sum = value_12 + function_3;";

/// `Space p` and `Space g` over a 20,000-file project.
#[test]
#[ignore]
fn perf_pickers() {
    let bin = release_bin();
    let env = Env::new();
    let body = rust_like(40);
    for d in 0..200 {
        fs::create_dir_all(env.path(&format!("d{d}"))).unwrap();
        for f in 0..100 {
            env.write(&format!("d{d}/f{f}.rs"), &body);
        }
    }
    env.write("d150/f50.rs", &format!("{body}// needle_xyz\n"));
    env.write("main.rs", "fn main() {}\n");
    let total = 20_000;
    let mut r = Report::default();
    println!(
        "== pickers: {total} files, {:.0} MB (release build, 40x120 pty)",
        (body.len() * 20_000) as f64 / 1e6
    );
    let t = Instant::now();
    let mut p = env.spawn_bin(&bin, &["--height", "max", "main.rs"], 40, 120);
    p.wait_for_within("first paint", Duration::from_secs(60), |s| {
        s.contains("NORMAL")
    });
    r.action("startup to first paint", t.elapsed(), 1000.0);

    let d = timed(&p, " p", "picker", |s| s.contains("find>"));
    r.action("Space p until the picker shows", d, 100.0);
    // The env's own state and config files are in the tree too.
    let listed = |s: &str| {
        s.lines().any(|l| {
            l.contains(" FIND ") && !l.contains('…') && {
                let w = l.split_whitespace().nth(1).unwrap_or("");
                w.split_once('/')
                    .is_some_and(|(a, b)| a == b && a.len() >= 5)
            }
        })
    };
    let d = d + timed(&p, "", "walk done", listed);
    r.action("Space p until all files are listed", d, 2000.0);
    let d = timed(&p, "d15f5", "ranked", |s| {
        s.contains("find> d15f5") && s.contains("> d15") && !listed(s)
    });
    r.action("typing 5 chars until ranked (all keys)", d, 500.0);
    p.keys(&["\x1b"]);
    p.wait_for("normal", |s| s.contains("NORMAL"));

    p.keys(&[" g"]);
    p.wait_for("grep", |s| s.contains("grep>"));
    let d = timed(&p, "needle_xyz", "rare match", |s| {
        s.contains("d150/f50.rs:") && s.contains(" 1 matches ")
    });
    r.action(
        "grep a rare word: typed until whole tree scanned",
        d,
        3000.0,
    );
    for _ in 0.."needle_xyz".len() {
        p.send("\x7f");
    }
    let d = timed(&p, "fn ", "common match", |s| s.contains("1000+ matches"));
    r.action("grep a common word: typed until 1000+ matches", d, 1000.0);
    p.keys(&["\x1b", ":q\r"]);
    p.wait_exit();
    r.finish();
}

/// ~4.8 MB / 100k lines of Rust.
#[test]
#[ignore]
fn perf_big_rust_file() {
    let bin = release_bin();
    let env = Env::new();
    let text = rust_like(100_000);
    let lines = text.lines().count();
    env.write("big.rs", &text);
    let first_line = text.lines().next().unwrap().to_string();
    let mut r = Report::default();
    println!(
        "== big.rs: {} lines, {:.2} MB (release build, 24x100 pty)",
        lines,
        text.len() as f64 / 1e6
    );

    let t = Instant::now();
    let mut p = env.spawn_bin(&bin, &["big.rs"], 24, 100);
    p.wait_for_within("first paint", Duration::from_secs(60), |s| {
        s.contains("NORMAL") && s.contains(&first_line[..30])
    });
    r.action("startup to first paint", t.elapsed(), 1000.0);
    let pid = p.pid();
    thread::sleep(Duration::from_millis(500));
    r.add("RSS after load", format!("{} MB", rss_kb(pid) / 1024));

    let d = timed(&p, "G", "last line", |s| {
        s.contains(&format!("{lines} }}")) && s.contains(&format!("{lines}:1"))
    });
    r.action("G until the last line is visible", d, 100.0);
    let c0 = cpu_secs(pid);
    thread::sleep(Duration::from_secs(3));
    r.add(
        "CPU during 3 s idle right after G (highlight catch-up)",
        format!("{:.2} s", cpu_secs(pid) - c0),
    );

    // Insert mode near the bottom.
    let d = timed(&p, "o", "INSERT", |s| s.contains("INSERT"));
    r.action(
        "o at the bottom until INSERT (word index rebuild)",
        d,
        100.0,
    );
    let row = p.cursor().0 as usize;
    let v = type_measured(&p, row, TYPED);
    r.keys("typing 32 chars near the bottom, per key", &v);
    p.keys(&["\x1b"]);

    // Insert mode near the top.
    p.keys(&["gg"]);
    p.wait_for("top", |s| s.contains(" 1:1"));
    let d = timed(&p, "O", "INSERT", |s| s.contains("INSERT"));
    r.action("O at the top until INSERT", d, 100.0);
    let row = p.cursor().0 as usize;
    let v = type_measured(&p, row, TYPED);
    r.keys("typing 32 chars near the top, per key", &v);
    p.keys(&["\x1b", "u", "u"]);
    p.wait_for("clean", |s| !s.contains("[+]"));

    // Search from the top for a line near the end.
    p.keys(&["gg", "/function_19990\\("]);
    let d = timed(&p, "\r", "search hit", |s| s.contains("99952:"));
    r.action("/function_19990\\( from the top (hit at 99952)", d, 1000.0);

    // Substitute over the whole file (80k lines change).
    p.keys(&[":,s/foo/bar/g"]);
    let d = timed(&p, "\r", "substitute", |s| s.contains("[+]"));
    r.action(":,s/foo/bar/g until [+]", d, 1000.0);
    thread::sleep(Duration::from_millis(300));
    r.add("RSS after substitute", format!("{} MB", rss_kb(pid) / 1024));
    let d = timed(&p, "u", "undo", |s| !s.contains("[+]"));
    r.action("u (undo the substitute) until [+] is gone", d, 1000.0);
    let d = timed(&p, "\x12", "redo", |s| s.contains("[+]"));
    r.action("Ctrl-R (redo) until [+]", d, 1000.0);

    p.keys(&[":w"]);
    let d = timed(&p, "\r", "written", |s| s.contains("written"));
    r.action(":w until 'written'", d, 1000.0);
    // Let highlighting catch up, then measure the steady state.
    thread::sleep(Duration::from_secs(15));
    let c0 = cpu_secs(pid);
    thread::sleep(Duration::from_secs(3));
    r.idle_cpu("steady-state idle CPU", cpu_secs(pid) - c0, 3.0);
    r.add("RSS at the end", format!("{} MB", rss_kb(pid) / 1024));
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(env.read("big.rs").contains("bar.wrapping_add"));
    r.finish();
}

/// ~850 KB: under the 1 MiB limit, so the completion word index is rebuilt
/// from scratch on every word-char keystroke.
#[test]
#[ignore]
fn perf_900k_file_completion_rebuild() {
    let bin = release_bin();
    let env = Env::new();
    let text = rust_like(18_500);
    assert!(text.len() < 1024 * 1024);
    env.write("mid.rs", &text);
    let mut r = Report::default();
    println!(
        "== mid.rs: {} lines, {} KB (release build)",
        text.lines().count(),
        text.len() / 1024
    );
    let mut p = env.spawn_bin(&bin, &["mid.rs"], 24, 100);
    p.wait_for_within("first paint", Duration::from_secs(60), |s| {
        s.contains("NORMAL")
    });
    let d = timed(&p, "o", "INSERT", |s| s.contains("INSERT"));
    r.action("o until INSERT", d, 100.0);
    let row = p.cursor().0 as usize;
    let v = type_measured(&p, row, TYPED);
    r.keys("typing near the top, per key", &v);
    p.keys(&["\x1b", "G"]);
    p.wait_for("bottom", |s| s.contains("18501:"));
    p.keys(&["o"]);
    p.wait_text("INSERT");
    let row = p.cursor().0 as usize;
    let v = type_measured(&p, row, TYPED);
    r.keys("typing near the bottom, per key", &v);
    // A fast typist: Esc, then `:w<CR>` 30 ms later.
    p.send("x");
    thread::sleep(Duration::from_millis(30));
    p.send("\x1b");
    thread::sleep(Duration::from_millis(30));
    p.send(":w\r");
    let wrote = p
        .try_wait_for(Duration::from_secs(3), |s| s.contains("written"))
        .is_some();
    r.check(
        "x, Esc, :w<CR> 30 ms apart -> written",
        wrote,
        status_row(&p),
    );
    p.keys(&["\x1b", ":q!\r"]);
    p.exit_status(Duration::from_secs(5));
    r.finish();
}

/// One 1 MiB line of minified JSON.
#[test]
#[ignore]
fn perf_minified_json() {
    perf_json(1024 * 1024, false);
}

/// The same with `wrap = true`.
#[test]
#[ignore]
fn perf_minified_json_wrap() {
    perf_json(1024 * 1024, true);
}

/// 900 KB of JSON on one line (under the 1 MiB completion-rebuild limit).
#[test]
#[ignore]
fn perf_minified_json_900k() {
    perf_json(900 * 1024, false);
}

fn perf_json(size: usize, wrap: bool) {
    let bin = release_bin();
    let env = Env::new();
    if wrap {
        env.config("wrap = true\n");
    }
    let text = minified_json(size);
    env.write("min.json", &text);
    let mut r = Report::default();
    println!(
        "== min.json: 1 line, {} bytes, wrap={wrap} (release build, 24x100 pty)",
        text.len()
    );
    let t = Instant::now();
    let mut p = env.spawn_bin(&bin, &["min.json"], 24, 100);
    p.wait_for_within("first paint", Duration::from_secs(60), |s| {
        s.contains("NORMAL") && s.contains("{\"data\"")
    });
    r.action("startup to first paint", t.elapsed(), 1000.0);
    let pid = p.pid();
    thread::sleep(Duration::from_millis(300));
    r.add("RSS after load", format!("{} MB", rss_kb(pid) / 1024));
    let c0 = cpu_secs(pid);
    thread::sleep(Duration::from_secs(2));
    r.idle_cpu("idle CPU (nothing typed)", cpu_secs(pid) - c0, 2.0);
    let last_col = format!("1:{}", text.len() - 1);
    let d = timed(&p, "$", "status at the last column", |s| {
        s.contains(&last_col)
    });
    r.action("$ until the status shows the last column", d, 100.0);
    thread::sleep(Duration::from_millis(200));
    r.check(
        "after $, the end of the line `]}` is on screen",
        p.screen().contains("]}"),
        format!("(cell under the cursor: {:?})", p.cell_at_cursor()),
    );
    let d = timed(&p, "0", "start of line", |s| {
        s.contains(" 1:1 ") || s.contains(" 1:1\n") || s.contains("  1:1")
    });
    r.action("0 back to the start", d, 100.0);
    let d = timed(&p, "w", "w", |s| s.contains("1:3"));
    r.action("w (one word right)", d, 100.0);
    let d = timed(&p, "i", "INSERT", |s| s.contains("INSERT"));
    r.action("i until INSERT", d, 100.0);
    let row = p.cursor().0 as usize;
    let v = type_measured(&p, row, "abcdefghij");
    r.keys("typing 10 chars at the start of the line, per key", &v);
    p.keys(&["\x1b"]);
    p.keys(&[":w"]);
    let d = timed(&p, "\r", "written", |s| s.contains("written"));
    r.action(":w until 'written'", d, 1000.0);
    r.add("RSS at the end", format!("{} MB", rss_kb(pid) / 1024));
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    r.finish();
}

/// RSS over time on the 1 MiB one-line JSON: idle, then after edits.
#[test]
#[ignore]
fn perf_json_memory_over_time() {
    let bin = release_bin();
    let env = Env::new();
    env.write("min.json", &minified_json(1024 * 1024));
    let mut p = env.spawn_bin(&bin, &["min.json"], 24, 100);
    p.wait_for_within("first paint", Duration::from_secs(60), |s| {
        s.contains("NORMAL")
    });
    let pid = p.pid();
    let mut samples = vec![];
    let mut sample = |label: String| {
        let mb = rss_kb(pid) / 1024;
        println!("  {label:<24} RSS {mb} MB");
        samples.push(mb);
    };
    println!("== min.json (1 MiB, one line): RSS over time (release build)");
    for s in 0..8 {
        thread::sleep(Duration::from_secs(1));
        sample(format!("idle {}s", s + 1));
    }
    p.keys(&["i"]);
    for (i, c) in "abcdefghij".chars().enumerate() {
        p.send(&c.to_string());
        thread::sleep(Duration::from_millis(150));
        sample(format!("after {} typed chars", i + 1));
    }
    p.keys(&["\x1b"]);
    for s in 0..4 {
        thread::sleep(Duration::from_secs(1));
        sample(format!("idle after edit {}s", s + 1));
    }
    let peak = samples.iter().max().copied().unwrap_or(0);
    p.keys(&[":q!\r"]);
    p.wait_exit();
    assert!(
        peak < 300,
        "a 1 MiB file peaked at {peak} MB RSS (samples {samples:?}); every 50 ms frame lays \
         out the whole line as one String per grapheme, even when nothing changed"
    );
}

/// Idle CPU and RSS as a function of the length of one long line.
#[test]
#[ignore]
fn perf_idle_cost_vs_line_length() {
    let bin = release_bin();
    println!("== idle cost of one long line (release build, 24x100 pty)");
    // Lines under 20,000 chars are also syntax highlighted, every frame.
    for kb in [4, 16, 64, 256, 1024] {
        let env = Env::new();
        env.write("min.json", &minified_json(kb * 1024));
        let mut p = env.spawn_bin(&bin, &["min.json"], 24, 100);
        p.wait_for_within("first paint", Duration::from_secs(60), |s| {
            s.contains("NORMAL")
        });
        thread::sleep(Duration::from_millis(500));
        let pid = p.pid();
        let c0 = cpu_secs(pid);
        thread::sleep(Duration::from_secs(3));
        let cpu = cpu_secs(pid) - c0;
        println!(
            "  {kb:>5} KB line: idle CPU {:>3.0}%, RSS {} MB",
            cpu / 3.0 * 100.0,
            rss_kb(pid) / 1024
        );
        p.keys(&[":q\r"]);
        p.wait_exit();
    }
}

/// Not a perf measurement as such: fred keeps writing to the terminal every
/// 50 ms even when nothing changes (constant traffic over SSH).
#[test]
fn idle_window_sends_nothing_to_the_terminal() {
    let env = Env::new();
    env.write("small.rs", "fn main() {\n    println!(\"hi\");\n}\n");
    let mut p = env.fred(&["small.rs"]);
    p.wait_text("NORMAL");
    thread::sleep(Duration::from_millis(500));
    let b0 = p.raw_len();
    thread::sleep(Duration::from_secs(2));
    let written = p.raw_len() - b0;
    let sample = tail(&p.raw(), 90);
    p.keys(&[":q\r"]);
    p.wait_exit();
    assert!(
        written < 64,
        "an idle fred wrote {written} bytes to the terminal in 2 s (~{} B/s): it redraws every \
         50 ms tick even when nothing changed, and ratatui re-sends style resets, show-cursor and \
         a cursor move each time. Sample: {sample}",
        written / 2
    );
}

// ================================================== 2. suspend / resume

#[test]
fn suspend_and_resume_under_bash() {
    let env = Env::new();
    env.write("f.txt", "alpha\nbeta\n");
    let p = env.bash(24, 80);
    p.wait_text("PROMPT$");
    p.keys(&["\"$FRED\" f.txt\r"]);
    p.wait_text("NORMAL");
    p.keys(&["x"]);
    p.wait_text("[+]");

    p.keys(&["\x1a"]);
    p.wait_text("Stopped");
    p.wait_for("a prompt after ^Z", |s| s.matches("PROMPT$").count() >= 2);
    let s = p.screen();
    assert!(
        !s.contains("lpha") && !s.contains("NORMAL"),
        "window not erased on ^Z:\n{s}"
    );
    assert_eq!(
        env.swap_texts().len(),
        1,
        "the unsaved edit goes to the swap on ^Z"
    );
    assert!(env.swap_texts()[0].contains("lpha\nbeta"));

    p.keys(&["fg\r"]);
    p.wait_text("NORMAL");
    let rows = p.rows();
    let fg = rows
        .iter()
        .rposition(|r| r == "PROMPT$ fg")
        .expect("`fg` line");
    let text = rows
        .iter()
        .position(|r| r.ends_with(" 1 lpha"))
        .unwrap_or_else(|| panic!("window not redrawn:\n{}", rows.join("\n")));
    assert_eq!(
        text,
        fg + 2,
        "the window should open right below the job line `fg` prints:\n{}",
        rows.join("\n")
    );
    assert_eq!(p.cursor(), (text as u16, 4), "cursor on line 1, column 1");

    p.keys(&["x"]);
    p.wait_text(" 1 pha");
    p.keys(&[":wq\r"]);
    p.wait_text("\"f.txt\" 2L, 9B written");
    p.wait_for("a prompt after :wq", |s| s.matches("PROMPT$").count() >= 3);
    assert_eq!(env.read("f.txt"), "pha\nbeta\n");
    assert!(env.swap_files().is_empty());
    assert!(!p.screen().contains("NORMAL"), "window erased on exit");
}

#[test]
fn resume_warns_when_the_file_changed_while_suspended() {
    let env = Env::new();
    env.write("f.txt", "alpha\n");
    let p = env.bash(24, 80);
    p.wait_text("PROMPT$");
    p.keys(&["\"$FRED\" f.txt\r"]);
    p.wait_text("NORMAL");
    p.keys(&["\x1a"]);
    p.wait_text("Stopped");
    env.write("f.txt", "changed by someone else\n");
    p.keys(&["fg\r"]);
    p.wait_text("? file changed on disk");
    p.keys(&[":q\r"]);
    p.wait_for("a prompt after :q", |s| s.matches("PROMPT$").count() >= 3);
}

#[test]
fn ctrl_z_works_when_fred_is_started_by_another_program() {
    // Like `git commit` running $EDITOR: fred's parent is not the shell.
    let env = Env::new();
    env.write("f.txt", "alpha\n");
    let p = env.bash(24, 80);
    p.wait_text("PROMPT$");
    p.keys(&["sh -c '\"$FRED\" f.txt; echo wrapper-done'\r"]);
    p.wait_text("NORMAL");
    p.keys(&["\x1a"]);
    let back = p.try_wait_for(Duration::from_secs(3), |s| s.contains("Stopped"));
    let screen = p.screen();
    // Get out: a second ^Z (the tty is in cooked mode now) stops the whole job.
    if back.is_none() {
        p.keys(&["\x1a"]);
        p.wait_text("Stopped");
    }
    p.keys(&["fg\r"]);
    p.wait_text("NORMAL");
    p.keys(&[":q\r"]);
    p.wait_text("wrapper-done");
    assert!(
        back.is_some(),
        "Ctrl-Z erased the window but the shell prompt never came back: fred stops only \
         itself (raise(SIGTSTP)) while its parent `sh` keeps waiting, so the terminal looks \
         hung until ^Z is pressed a second time. Screen 3 s after ^Z:\n{screen}"
    );
}

// ======================================== 3. tiny and changing terminal sizes

#[test]
fn three_row_terminal_is_usable() {
    let env = Env::new();
    env.write("f.txt", "hello\nworld\n");
    let mut p = env.fred_sized(&["f.txt"], 3, 20);
    p.wait_text("NORMAL");
    assert_eq!(p.rows()[0], "  1 hello");
    p.keys(&["x", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "ello\nworld\n");
}

#[test]
fn one_row_terminal_does_not_panic() {
    for cols in [20, 5] {
        let env = Env::new();
        env.write("f.txt", "hello\nworld\n");
        let mut p = env.fred_sized(&["f.txt"], 1, cols);
        thread::sleep(Duration::from_millis(1000));
        let alive = p.running();
        let raw = p.raw();
        if alive {
            p.keys(&[":q\r"]);
        }
        let code = p.exit_status(Duration::from_secs(3)).map(|s| s.exit_code());
        let panic_msg = raw
            .find("panicked")
            .map(|i| raw[i..].lines().take(2).collect::<Vec<_>>().join(" "));
        assert!(
            alive && panic_msg.is_none(),
            "fred in a 1x{cols} terminal: exit {code:?}, {panic_msg:?} \
             (debug build: `area.y + area.height - 2` underflows in render.rs draw_status; \
             release build: ratatui 'index outside of buffer ... (0, 65535)')"
        );
    }
}

#[test]
#[ignore = "deferred minor: a 2-row terminal has room only for the status and command rows"]
fn bug_two_row_terminal_shows_no_text() {
    let env = Env::new();
    env.write("f.txt", "hello\nworld\n");
    let mut p = env.fred_sized(&["f.txt"], 2, 10);
    p.wait_text("NORM");
    thread::sleep(Duration::from_millis(200));
    let rows = p.rows();
    let cursor = p.cursor();
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(
        rows.iter().any(|r| r.contains("hello")),
        "in a 2-row terminal the one text row is drawn over by the status row, so no text is \
         visible and the cursor {cursor:?} sits on the status bar: {rows:?}"
    );
}

#[test]
fn resize_to_one_row_keeps_the_unsaved_edit() {
    let env = Env::new();
    env.write("f.txt", "hello\nworld\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("hello");
    p.keys(&["x"]); // unsaved; the swap is only written after a 1 s pause
    p.wait_text("[+]");
    p.resize(1, 80);
    thread::sleep(Duration::from_millis(700));
    let alive = p.running();
    let raw = p.raw();
    if alive {
        p.resize(24, 80);
        thread::sleep(Duration::from_millis(300));
        p.keys(&["\x1b", ":q!\r"]);
    }
    let code = p.exit_status(Duration::from_secs(3)).map(|s| s.exit_code());
    let panic_msg = raw
        .find("panicked")
        .map(|i| raw[i..].lines().take(2).collect::<Vec<_>>().join(" "));
    assert!(
        alive && panic_msg.is_none(),
        "resizing the terminal to 1 row crashed fred (exit {code:?}): {panic_msg:?}. The panic \
         is in drawing, outside the catch_unwind around key handling, and the panic hook does \
         not write the swap (the spec says it does): swap files afterwards {:?}, file on disk \
         {:?} -> the unsaved edit is lost",
        env.swap_files(),
        env.read("f.txt")
    );
}

#[test]
fn resize_narrow_and_back_stays_usable() {
    let env = Env::new();
    env.write("f.txt", "hello\nworld\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("hello");
    p.keys(&["x"]);
    p.wait_text("[+]");
    for (r, c) in [(24, 5), (2, 80), (3, 5), (2, 3), (24, 1), (6, 12), (24, 80)] {
        p.resize(r, c);
        thread::sleep(Duration::from_millis(250));
        assert!(
            p.running() && !p.raw().contains("panicked"),
            "died after resizing to {r}x{c}:\n{}",
            tail(&p.raw(), 400)
        );
    }
    p.wait_text("  1 ello");
    p.keys(&["x", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "llo\nworld\n");
}

// =============================================== 4. batched / fast input

/// Write `batch` in ONE write (as a fast typist over SSH, or keys queued up
/// while fred is busy, would arrive), then finish with separate, slow keys.
/// Returns the file and the screen right after the batch.
fn run_batched(text: &str, setup: &[&str], batch: &str) -> (String, String) {
    let env = Env::new();
    env.write("f.txt", text);
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(setup);
    p.send(batch);
    thread::sleep(Duration::from_millis(400));
    let screen = p.screen();
    if p.running() {
        p.keys(&["\x1b", ":wq\r"]);
        p.wait_exit();
    }
    (env.read("f.txt"), screen)
}

#[test]
fn esc_batched_with_the_next_keys_is_not_lost() {
    // crossterm reads ESC + key in one read as Alt+key; fred turns that back
    // into Esc, key.
    let (got, _) = run_batched("hello\n", &[], "ifoo\x1b:wq\r");
    assert_eq!(got, "foohello\n");
    let (got, _) = run_batched("one\ntwo\nthree\n", &["A!"], "\x1bdd");
    assert_eq!(got, "two\nthree\n");
}

#[test]
#[ignore = "known limitation: crossterm parses ESC O <c> in one read as an SS3 key (Up for OA) or drops it"]
fn esc_o_batched_in_one_read() {
    let (got, _) = run_batched("one\ntwo\n", &[], "ifoo\x1bObar\x1b");
    assert_eq!(got, "bar\nfooone\ntwo\n");
    let (got, _) = run_batched("one\ntwo\n", &["j"], "ifoo\x1bOAbar\x1b");
    assert_eq!(got, "one\nAbar\nfootwo\n");
}

#[test]
fn esc_esc_then_a_command_in_one_write_works() {
    let (got, _) = run_batched("hello\n", &[], "ifoo\x1b\x1b:wq\r");
    assert_eq!(got, "foohello\n");
}

// ======================================================= 5. pasting

/// A ~2 KB block of indented code with a tab-indented line (no final newline).
fn paste_block() -> String {
    let mut s = String::new();
    let mut i = 0;
    while s.len() < 2000 {
        s.push_str(&format!(
            "fn compute_{i}(values: &[u32]) -> u32 {{\n    let mut total = 0;\n    for value in values {{\n        if value % 2 == 0 {{\n            total += value;\n        }} else {{\n\t\ttotal -= 1;\n        }}\n    }}\n    total\n}}\n"
        ));
        i += 1;
    }
    s.pop();
    s
}

#[test]
fn bracketed_paste_is_verbatim_and_one_undo_step() {
    let block = paste_block();
    let lines = block.lines().count();
    let env = Env::new();
    env.write("f.txt", "start\nend\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["o"]);
    p.wait_text("INSERT");
    p.send(&format!("\x1b[200~{}\x1b[201~", block.replace('\n', "\r")));
    p.wait_text(&format!("{}:2", lines + 1));
    p.keys(&["\x1b", ":w\r"]);
    p.wait_text("written");
    assert_eq!(
        env.read("f.txt"),
        format!("start\n{block}\nend\n"),
        "pasted text must go in exactly as pasted"
    );
    p.keys(&["u", ":w\r"]);
    p.wait_text("2L, 10B written");
    assert_eq!(
        env.read("f.txt"),
        "start\nend\n",
        "one `u` undoes the paste"
    );
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

/// Without bracketed paste the burst is indistinguishable from typing: each
/// Enter autoindents and the pasted indentation adds to it (staircase). Not a
/// bug as such; this records what happens.
#[test]
fn unbracketed_paste_burst_staircases_but_undoes_in_one_step() {
    let block = paste_block();
    let env = Env::new();
    env.write("f.txt", "start\nend\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["o"]);
    p.wait_text("INSERT");
    p.send(&block.replace('\n', "\r"));
    thread::sleep(Duration::from_millis(1500));
    p.keys(&["\x1b", ":w\r"]);
    p.wait_text("written");
    let got = env.read("f.txt");
    println!(
        "unbracketed: {} bytes pasted, {} bytes in the file; line 4 is {:?}",
        block.len(),
        got.len(),
        got.lines().nth(4).unwrap_or("")
    );
    assert!(got.starts_with(&format!("start\n{}\n", block.lines().next().unwrap())));
    p.keys(&["u", ":w\r"]);
    p.wait_text("2L, 10B written");
    assert_eq!(env.read("f.txt"), "start\nend\n");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
#[ignore = "deferred minor: '.' doesn't record bracketed pastes"]
fn bug_dot_repeat_after_a_paste_drops_the_pasted_text() {
    let env = Env::new();
    env.write("f.txt", "one\ntwo\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["o"]);
    p.send("\x1b[200~pasted\x1b[201~");
    p.wait_text(" 2 pasted");
    p.keys(&["\x1b", "j", ".", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(
        env.read("f.txt"),
        "one\npasted\ntwo\npasted\n",
        "`.` repeats the change by replaying recorded keys, and a paste is not a key, so \
         `o<paste><Esc>` repeats as an empty `o`"
    );
}

// =========================================== 6. wrap mode and display

#[test]
fn cursor_lands_on_special_characters() {
    for wrap in [false, true] {
        let env = Env::new();
        if wrap {
            env.config("wrap = true\n");
        }
        env.write("f.txt", &format!("{SPECIAL}\nend\n"));
        let mut p = env.fred_sized(&["f.txt"], 12, 40);
        p.wait_text("NORMAL");
        let mut bad = vec![];
        #[rustfmt::skip]
        let checks = [
            ("0", "a"), ("0fb", "b"), ("0fbh", " "), ("0f字", "字"), ("0f字h", "漢"),
            ("0fc", "c"), ("0fcl", "👍"), ("0fd", "d"), ("0fdl", "👨"), ("0fe", "e"),
            ("0fel", "x\u{301}"), ("0ff", "f"), ("0ffl", "^"), ("0fg", "g"), ("$", "g"),
        ];
        for (keys, want) in checks {
            p.keys(&[keys]);
            let got = p.cell_at_cursor();
            // A tab's cells are blanks; never-written blanks read as "".
            let blank_ok = want == " " && got.is_empty();
            if !got.starts_with(want) && !blank_ok {
                bad.push(format!(
                    "{keys}: cursor {:?} on {got:?}, want {want:?}",
                    p.cursor()
                ));
            }
        }
        let screen = p.screen();
        p.keys(&[":q\r"]);
        assert_eq!(p.wait_exit(), 0);
        assert!(bad.is_empty(), "wrap={wrap}: {bad:#?}\n{screen}");
    }
}

#[test]
fn cursor_cell_is_visible_when_scrolled_sideways() {
    let env = Env::new();
    env.write(
        "f.txt",
        &format!("{}\n{}{SPECIAL}\n", letters(100), "y".repeat(50)),
    );
    // 40 columns: a 4-column gutter and 36 text columns.
    let mut p = env.fred_sized(&["f.txt"], 12, 40);
    p.wait_text("NORMAL");
    let mut bad = vec![];
    for (keys, want) in [
        ("0", "a"),
        ("50l", "y"),
        ("l", "z"),
        ("j0fb", "b"),
        ("0f字", "字"),
        ("0fd", "d"),
        ("$", "g"),
    ] {
        p.keys(&[keys]);
        let got = p.cell_at_cursor();
        if got != want {
            bad.push(format!(
                "after {keys:>4}: the cursor {:?} shows {got:?}, the editor is on {want:?}",
                p.cursor()
            ));
        }
    }
    let screen = p.screen();
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(
        bad.is_empty(),
        "wrap off: once a line scrolls sideways the cursor sits in the last column, and that \
         column is drawn as the `›` cut marker (or blank, for a wide char) instead of the \
         character under the cursor:\n{}\n{screen}",
        bad.join("\n")
    );
}

#[test]
fn wrap_wide_char_at_row_end_keeps_the_cursor_on_it() {
    let env = Env::new();
    env.config("wrap = true\n");
    // 36 text columns: "a" + 17 漢 = 35 columns, so the 18th 漢 doesn't fit and
    // starts the second screen row.
    env.write("f.txt", &format!("a{}\nend\n", "漢".repeat(30)));
    let mut p = env.fred_sized(&["f.txt"], 12, 40);
    p.wait_text("NORMAL");
    assert_eq!(p.rows()[1], format!("    {}", "漢".repeat(13)));
    let mut bad = vec![];
    for (keys, want) in [
        ("0", (0, 4)),
        ("17l", (0, 37)),
        ("l", (1, 4)),
        ("l", (1, 6)),
        ("$", (1, 28)),
    ] {
        p.keys(&[keys]);
        let (cur, cell) = (p.cursor(), p.cell_at_cursor());
        if cur != want {
            bad.push(format!(
                "after {keys:>3}: cursor {cur:?} on {cell:?}, the 漢 is at {want:?}"
            ));
        }
    }
    let screen = p.screen();
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(
        bad.is_empty(),
        "wrap on: a wide char that doesn't fit at the end of a screen row moves to the next \
         row, but the cursor is placed as if every row held exactly 36 columns, so it is off \
         for everything after the break:\n{}\n{screen}",
        bad.join("\n")
    );
}

#[test]
fn wrap_scrolls_within_a_line_longer_than_the_window() {
    let env = Env::new();
    env.config("wrap = true\n");
    let long = letters(300) + "Z";
    env.write("f.txt", &format!("{long}\nb\nc\nd\n"));
    // 4 text rows x 36 columns = 144 visible chars of a 301-char line.
    let mut p = env.fred_sized(&["--height", "4", "f.txt"], 12, 40);
    p.wait_text("NORMAL");
    p.keys(&["$"]);
    p.wait_text("1:301");
    let (cur, cell, screen) = (p.cursor(), p.cell_at_cursor(), p.screen());
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(
        cell == "Z",
        "wrap on, 301-char line, 4-row window: after `$` the editor is on the final `Z` \
         (status 1:301), but the view can't scroll within a line, `Z` is not on screen \
         ({}), and the terminal cursor is clamped to the last row at {cur:?}, on {cell:?}:\n{screen}",
        if screen.contains('Z') {
            "visible"
        } else {
            "not visible"
        }
    );
}

#[test]
#[ignore = "deferred minor: terminals disagree on ZWJ/skin-tone emoji widths"]
fn bug_zwj_emoji_near_the_right_edge_spills_into_the_next_row() {
    let env = Env::new();
    env.write(
        "f.txt",
        &format!("{SPECIAL}\n{}{SPECIAL}\nend\n", "y".repeat(50)),
    );
    let mut p = env.fred_sized(&["f.txt"], 12, 40);
    p.wait_text("NORMAL");
    p.keys(&["j", "0fe"]);
    thread::sleep(Duration::from_millis(100));
    let rows = p.rows();
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(
        rows[2].starts_with("  3"),
        "fred treats 👨‍👩‍👧 as one 2-column grapheme (unicode-width), but terminals that draw ZWJ \
         sequences per code point (xterm, tmux, the vt100 crate) need 6 columns. Drawn at the \
         right edge it wraps into the next screen row and is never repaired. Row 3 should be \
         line 3's gutter, but is {:?}:\n{}",
        rows[2],
        rows.join("\n")
    );
}

#[test]
#[ignore = "deferred minor: wrap-mode window height counts logical lines"]
fn bug_wrap_window_is_sized_by_lines_and_hides_wrapped_text() {
    let env = Env::new();
    env.config("wrap = true\n");
    env.write("f.txt", &format!("short\n{}\nshort2\n", "x".repeat(80)));
    let mut p = env.fred_sized(&["f.txt"], 12, 40);
    p.wait_text("NORMAL");
    let rows = p.rows();
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(
        rows.iter().any(|r| r.contains("short2")),
        "wrap on, 3-line file whose line 2 wraps to 3 screen rows, 12-row terminal: the window \
         gets 3 text rows (one per line), so the end of line 2 and all of line 3 are hidden \
         although the window could grow to 5:\n{}",
        rows.join("\n")
    );
}

#[test]
fn wrap_mode_j_and_k_over_wrapped_lines() {
    let env = Env::new();
    env.config("wrap = true\n");
    // Enough lines for a full 12-row window (see the window-size bug above).
    env.write(
        "f.txt",
        &format!("short\n{}\nshort2\n{}", "x".repeat(80), "z\n".repeat(10)),
    );
    let mut p = env.fred_sized(&["f.txt"], 16, 40);
    p.wait_text("NORMAL");
    let rows = p.rows();
    assert_eq!(rows[1], format!("  2 {}", "x".repeat(36)));
    assert_eq!(rows[3], format!("    {}", "x".repeat(8)));
    assert_eq!(rows[4], "  3 short2");
    for (keys, pos, cell) in [
        ("j", (1, 4), "x"),
        ("j", (4, 4), "s"),
        ("k", (1, 4), "x"),
        ("$", (3, 11), "x"),
        ("j", (4, 9), "2"),
        ("k", (3, 11), "x"),
        ("gg", (0, 4), "s"),
    ] {
        p.keys(&[keys]);
        assert_eq!(
            (p.cursor(), p.cell_at_cursor().as_str()),
            (pos, cell),
            "after {keys}:\n{}",
            p.screen()
        );
    }
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn config_relative_numbers_unknown_key_and_truecolor_theme() {
    let src = "fn main() {\n\tlet x = 1;\n    println!(\"{x}\");\n}\n";
    // relative numbers
    let env = Env::new();
    env.config("relative_numbers = true\n");
    env.write("x.rs", src);
    let mut p = env.fred(&["x.rs"]);
    p.wait_text("NORMAL");
    p.keys(&["j"]);
    let rows = p.rows();
    assert!(rows[0].starts_with("  1 fn"), "{rows:?}");
    assert!(
        rows[1].starts_with("  2 "),
        "current line is absolute: {rows:?}"
    );
    assert!(rows[2].starts_with("  1 "), "{rows:?}");
    assert!(rows[3].starts_with("  2 }"), "{rows:?}");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);

    // an unknown key is reported, defaults are used
    let env = Env::new();
    env.config("colour = 1\n");
    env.write("x.rs", src);
    let mut p = env.fred(&["x.rs"]);
    p.wait_text("NORMAL");
    p.wait_text("? config: unknown field `colour`");
    assert!(p.rows()[0].starts_with("  1 fn main"));
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);

    // Nord: 24-bit colors with COLORTERM=truecolor, 256 colors without
    for truecolor in [true, false] {
        let env = Env::new();
        env.config("theme = \"Nord\"\n");
        env.write("x.rs", src);
        let mut c = env.command(BIN);
        c.arg("x.rs");
        if truecolor {
            c.env("COLORTERM", "truecolor");
        }
        let mut p = Pty::spawn_sized(c, 24, 80);
        p.wait_text("NORMAL");
        thread::sleep(Duration::from_millis(200));
        let raw = p.raw();
        assert!(!raw.contains("? "), "no error: {}", cmd_row(&p));
        assert_eq!(raw.contains("[38;2;"), truecolor, "24-bit fg colors");
        assert!(raw.contains("[38;5;") || truecolor, "256-color fallback");
        p.keys(&[":q\r"]);
        assert_eq!(p.wait_exit(), 0);
    }
}

// ================================================= 7. file situations

#[test]
fn read_only_file() {
    let env = Env::new();
    let ro = env.write("ro.txt", "locked\n");
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o444)).unwrap();
    let mut p = env.fred(&["ro.txt"]);
    p.wait_text("NORMAL");
    assert!(status_row(&p).contains("ro.txt [RO]"), "{}", status_row(&p));
    p.keys(&[":w\r"]);
    assert!(cmd_row(&p).starts_with("? "), "{}", cmd_row(&p));
    p.keys(&[":w!\r"]);
    assert_eq!(cmd_row(&p), "? permission denied");
    p.keys(&["x", ":wq!\r"]);
    assert_eq!(cmd_row(&p), "? permission denied");
    p.keys(&[":q\r"]);
    assert!(cmd_row(&p).contains("unsaved changes"));
    p.keys(&[":q!\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("ro.txt"), "locked\n");

    let mut p = env.fred(&["ro.txt"]);
    p.wait_text("NORMAL");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0, ":q on an unmodified read-only file");
}

#[test]
#[ignore = "deferred minor: read-only message wording"]
fn bug_read_only_w_message_suggests_w_bang_which_cannot_work() {
    let env = Env::new();
    let ro = env.write("ro.txt", "locked\n");
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o444)).unwrap();
    let mut p = env.fred(&["ro.txt"]);
    p.wait_text("NORMAL");
    p.keys(&[":w\r"]);
    let msg = cmd_row(&p);
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(
        msg, "? permission denied",
        "spec: files without write permission: `w` fails with `? permission denied` (and \
         `w!` does not change that). fred says `w!` will override, and `w!` then fails"
    );
}

#[test]
fn invalid_utf8_is_read_only_and_never_written() {
    let env = Env::new();
    let bytes = b"ok\xff\xfe bytes\nline2\n".to_vec();
    env.write_bytes("bin.txt", &bytes);
    let mut p = env.fred(&["bin.txt"]);
    p.wait_text("NORMAL");
    assert!(status_row(&p).contains("[RO]"));
    assert!(cmd_row(&p).contains("not valid UTF-8"), "{}", cmd_row(&p));
    p.keys(&["x"]);
    for cmd in [":w\r", ":w!\r", ":wq!\r", ":w copy.txt\r"] {
        p.keys(&[cmd]);
        assert_eq!(
            cmd_row(&p),
            "? file is not valid UTF-8; writing it would change it",
            "{cmd:?}"
        );
    }
    p.keys(&[":q!\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read_bytes("bin.txt"), bytes);
    assert!(!env.path("copy.txt").exists());
}

#[test]
fn crlf_bom_and_missing_final_newline_round_trip() {
    for (name, content, keys, want) in [
        ("crlf.txt", "one\r\ntwo\r\n", "A!", "one!\r\ntwo\r\n"),
        (
            "crlf2.txt",
            "one\r\ntwo\r\n",
            "onew",
            "one\r\nnew\r\ntwo\r\n",
        ),
        ("bom.txt", "\u{feff}one\ntwo", "A!", "\u{feff}one!\ntwo"),
        ("nofinal.txt", "one\ntwo", "jA!", "one\ntwo!"),
        ("bomcrlf.txt", "\u{feff}a\r\nb", "jA!", "\u{feff}a\r\nb!"),
    ] {
        let env = Env::new();
        env.write(name, content);
        let mut p = env.fred(&[name]);
        p.wait_text("NORMAL");
        p.keys(&[keys, "\x1b", ":wq\r"]);
        assert_eq!(p.wait_exit(), 0);
        assert_eq!(env.read(name), want, "{name}");
    }
}

#[test]
fn file_changed_on_disk_blocks_w_and_w_bang_overrides() {
    let env = Env::new();
    env.write("f.txt", "mine\n");
    let mut p = env.fred(&["f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["x"]);
    thread::sleep(Duration::from_millis(1100));
    env.write("f.txt", "theirs\n");
    p.keys(&[":w\r"]);
    assert_eq!(cmd_row(&p), "? file changed on disk (w! to overwrite)");
    assert_eq!(env.read("f.txt"), "theirs\n");
    p.keys(&[":w!\r"]);
    p.wait_text("written");
    assert_eq!(env.read("f.txt"), "ine\n");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
}

#[test]
fn unwritable_directory_falls_back_to_writing_in_place() {
    let env = Env::new();
    env.write("locked/f.txt", "abc\n");
    fs::set_permissions(env.path("locked"), fs::Permissions::from_mode(0o555)).unwrap();
    let mut p = env.fred(&["locked/f.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["x", ":w\r"]);
    let existing = cmd_row(&p);
    p.keys(&[":w locked/new.txt\r"]);
    let new = cmd_row(&p);
    p.keys(&[":q\r"]);
    let code = p.wait_exit();
    fs::set_permissions(env.path("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(existing, "\"locked/f.txt\" 1L, 3B written");
    assert_eq!(env.read("locked/f.txt"), "bc\n");
    assert_eq!(new, "? permission denied");
    assert_eq!(code, 0);
}

#[test]
fn symlink_is_followed_and_kept() {
    let env = Env::new();
    env.write("real.txt", "real\n");
    std::os::unix::fs::symlink("real.txt", env.path("link.txt")).unwrap();
    let mut p = env.fred(&["link.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["A!", "\x1b", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("real.txt"), "real!\n");
    assert!(
        fs::symlink_metadata(env.path("link.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
#[ignore = "deferred minor: dangling symlinks are replaced on write"]
fn bug_dangling_symlink_is_replaced_by_a_regular_file() {
    let env = Env::new();
    std::os::unix::fs::symlink("target.txt", env.path("link.txt")).unwrap();
    let mut p = env.fred(&["link.txt"]);
    p.wait_text("[new]");
    p.keys(&["inew", "\x1b", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    let still_link = fs::symlink_metadata(env.path("link.txt"))
        .unwrap()
        .file_type()
        .is_symlink();
    assert!(
        still_link && env.path("target.txt").exists(),
        "editing a symlink whose target doesn't exist yet: `:wq` renamed a temp file over the \
         link (now a symlink: {still_link}) instead of creating the target (exists: {}); spec: \
         'Resolve symlinks; write to the real path'",
        env.path("target.txt").exists()
    );
}

// ======================================= 8. two freds, stale swap files

#[test]
fn second_fred_opens_read_only_and_keeps_the_first_freds_swap() {
    let env = Env::new();
    env.write("f.txt", "shared\n");
    let mut a = env.fred(&["f.txt"]);
    a.wait_text("NORMAL");
    a.keys(&["Afrom-a", "\x1b"]);
    wait_until("A's swap", Duration::from_secs(3), || {
        env.swap_files().len() == 1
    });
    let swap = env.swap_files()[0].clone();
    assert_eq!(swap_pid(&swap), Some(a.pid() as u32));

    let mut b = env.fred(&["f.txt"]);
    b.wait_text(&format!(
        "swap: file is open in fred (pid {}): [o]pen read-only, [q]uit",
        a.pid()
    ));
    b.keys(&["o"]);
    b.wait_text("[RO]");
    b.keys(&["Afrom-b", "\x1b"]);
    thread::sleep(Duration::from_millis(1500));
    b.keys(&[":q\r"]);
    assert!(cmd_row(&b).contains("unsaved changes"));
    b.keys(&[":q!\r"]);
    assert_eq!(b.wait_exit(), 0);
    assert!(swap.exists(), "B must not delete A's swap");
    assert_eq!(swap_pid(&swap), Some(a.pid() as u32));
    assert!(fs::read_to_string(&swap).unwrap().contains("sharedfrom-a"));

    a.keys(&[":q!\r"]);
    assert_eq!(a.wait_exit(), 0);
    assert!(
        env.swap_files().is_empty(),
        "A's normal exit removes its swap"
    );
    assert_eq!(env.read("f.txt"), "shared\n");
}

#[test]
fn second_fred_is_warned_even_when_the_first_has_no_changes() {
    let env = Env::new();
    env.write("f.txt", "shared\n");
    let mut a = env.fred(&["f.txt"]);
    a.wait_text("NORMAL");
    let mut b = env.fred(&["f.txt"]);
    b.wait_text("NORMAL");
    thread::sleep(Duration::from_millis(300));
    let (status, cmd) = (status_row(&b), cmd_row(&b));
    b.keys(&[":q\r"]);
    a.keys(&[":q\r"]);
    assert_eq!(b.wait_exit(), 0);
    assert_eq!(a.wait_exit(), 0);
    assert!(
        cmd.contains("file is open in fred") || status.contains("[RO]"),
        "README: 'If the file is already open in another fred, it offers a read-only view \
         instead.' But the swap file is only created at the first unsaved change, so a second \
         fred on a file the first has open (unmodified) gets no warning and both can edit it. \
         The second fred shows {status:?} / {cmd:?}"
    );
}

#[test]
fn a_read_only_second_fred_never_touches_the_first_freds_swap() {
    let env = Env::new();
    env.write("f.txt", "shared\n");
    let mut a = env.fred(&["f.txt"]);
    a.wait_text("NORMAL");
    let mut b = env.fred(&["f.txt"]);
    b.wait_text(&format!("file is open in fred (pid {})", a.pid()));
    b.keys(&["o"]);
    b.wait_text("[RO]");
    a.keys(&["Afrom-a", "\x1b"]);
    wait_until("A's swap", Duration::from_secs(3), || {
        env.swap_texts().iter().any(|t| t.contains("sharedfrom-a"))
    });
    b.keys(&["Afrom-b", "\x1b"]);
    thread::sleep(Duration::from_millis(1500));
    let swaps = env.swap_files();
    assert_eq!(swaps.len(), 1);
    assert_eq!(
        swap_pid(&swaps[0]),
        Some(a.pid() as u32),
        "B overwrote A's swap"
    );
    assert!(
        fs::read_to_string(&swaps[0])
            .unwrap()
            .contains("sharedfrom-a")
    );
    b.keys(&[":q!\r"]);
    assert_eq!(b.wait_exit(), 0);
    assert!(
        fs::read_to_string(&swaps[0])
            .unwrap()
            .contains("sharedfrom-a"),
        "B removed A's swap"
    );
    a.keys(&[":wq\r"]);
    assert_eq!(a.wait_exit(), 0);
    assert!(env.swap_files().is_empty());
    assert_eq!(env.read("f.txt"), "sharedfrom-a\n");
}

#[test]
fn stale_swap_quit_leaves_file_and_swap_untouched() {
    let env = Env::new();
    env.write("f.txt", "original\n");
    let mut a = env.fred(&["f.txt"]);
    a.wait_text("NORMAL");
    a.keys(&["Aunsaved", "\x1b"]);
    // The swap file exists from the start (a lock); wait for the text.
    wait_until("swap with the unsaved text", Duration::from_secs(3), || {
        env.swap_texts()
            .iter()
            .any(|t| t.contains("originalunsaved"))
    });
    a.signal(libc::SIGKILL);
    a.exit_status(Duration::from_secs(3));
    let swap = env.swap_files()[0].clone();
    let before = fs::read(&swap).unwrap();

    let mut p = env.fred(&["f.txt"]);
    p.wait_text("swap found (saved");
    p.wait_text("[r]ecover, [d]elete, [q]uit");
    p.keys(&["q"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "original\n");
    assert_eq!(fs::read(&swap).unwrap(), before, "swap untouched");
    assert!(!p.screen().contains("swap found"), "prompt erased");

    let mut p = env.fred(&["f.txt"]);
    p.wait_text("swap found");
    p.keys(&["r"]);
    p.wait_text("recovered");
    p.keys(&[":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("f.txt"), "originalunsaved\n");
    assert!(env.swap_files().is_empty());
}

#[test]
fn e_into_a_file_with_a_stale_swap_offers_recovery_and_keeps_protecting() {
    let env = Env::new();
    env.write("f.txt", "orig\n");
    env.write("other.txt", "other\n");
    let mut a = env.fred(&["f.txt"]);
    a.wait_text("NORMAL");
    a.keys(&["Aold", "\x1b"]);
    wait_until("swap with the unsaved text", Duration::from_secs(3), || {
        env.swap_texts().iter().any(|t| t.contains("origold"))
    });
    a.signal(libc::SIGKILL);
    a.exit_status(Duration::from_secs(3));

    let mut b = env.fred(&["other.txt"]);
    b.wait_text("NORMAL");
    b.keys(&[":e f.txt\r"]);
    b.wait_text("f.txt: swap found");
    b.keys(&["r"]);
    b.wait_text("recovered");
    b.keys(&["Anew-edit", "\x1b"]);
    thread::sleep(Duration::from_millis(1500));
    b.signal(libc::SIGHUP); // the terminal went away
    assert_eq!(b.wait_exit(), 1);
    assert!(
        env.swap_texts()
            .iter()
            .any(|t| t.contains("origoldnew-edit")),
        "edits after recovering through :e must be in the swap: {:?}",
        env.swap_texts()
    );
    assert_eq!(env.read("f.txt"), "orig\n");
}

#[test]
fn e_into_a_file_with_a_stale_swap_can_be_cancelled() {
    let env = Env::new();
    env.write("f.txt", "orig\n");
    env.write("other.txt", "other\n");
    let mut a = env.fred(&["f.txt"]);
    a.wait_text("NORMAL");
    a.keys(&["Aold", "\x1b"]);
    wait_until("swap with the unsaved text", Duration::from_secs(3), || {
        env.swap_texts().iter().any(|t| t.contains("origold"))
    });
    a.signal(libc::SIGKILL);
    a.exit_status(Duration::from_secs(3));
    let mut b = env.fred(&["other.txt"]);
    b.wait_text("NORMAL");
    b.keys(&[":e f.txt\r"]);
    b.wait_text("f.txt: swap found");
    b.keys(&["q"]);
    b.wait_text("still editing other.txt");
    assert!(status_row(&b).contains("other.txt"));
    b.keys(&[":q\r"]);
    assert_eq!(b.wait_exit(), 0);
    assert!(
        env.swap_texts().iter().any(|t| t.contains("origold")),
        "the stale swap is kept"
    );
}

#[test]
fn e_into_a_file_open_in_another_fred_opens_it_read_only() {
    let env = Env::new();
    env.write("f.txt", "shared\n");
    env.write("other.txt", "other\n");
    let mut a = env.fred(&["f.txt"]);
    a.wait_text("NORMAL");
    let mut b = env.fred(&["other.txt"]);
    b.wait_text("NORMAL");
    b.keys(&[":e f.txt\r"]);
    b.wait_text(&format!("f.txt: file is open in fred (pid {})", a.pid()));
    b.keys(&["o"]);
    b.wait_text("[RO]");
    assert!(status_row(&b).contains("f.txt"));
    b.keys(&[":q!\r"]);
    assert_eq!(b.wait_exit(), 0);
    assert_eq!(env.swap_files().len(), 1, "A's lock is still there");
    a.keys(&[":q!\r"]);
    assert_eq!(a.wait_exit(), 0);
    assert!(env.swap_files().is_empty());
}

// ======================================= 9. :e, unnamed buffers, arguments

#[test]
fn edit_other_file_with_and_without_bang() {
    let env = Env::new();
    env.write("a.txt", "aaa\n");
    env.write("b.rs", "fn b() {}\n");
    let mut p = env.fred(&["a.txt"]);
    p.wait_text("NORMAL");
    p.keys(&["x"]);
    wait_until("swap", Duration::from_secs(3), || {
        env.swap_files().len() == 1
    });
    p.keys(&[":e! b.rs\r"]);
    p.wait_text("fn b() {}");
    assert!(status_row(&p).contains("b.rs"));
    assert!(status_row(&p).contains("rust"), "{}", status_row(&p));
    let swaps = env.swap_texts();
    assert!(
        swaps.len() == 1 && !swaps[0].contains("aa"),
        "e! discards a.txt's swap; only b.rs's lock is left: {swaps:?}"
    );
    p.keys(&[":e nosuch.txt\r"]);
    p.wait_text("\"nosuch.txt\" [new]");
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("a.txt"), "aaa\n");
    assert!(!env.path("nosuch.txt").exists());
}

#[test]
fn unnamed_buffer_write_name_and_partial_write() {
    let env = Env::new();
    let mut p = env.fred(&[]);
    p.wait_text("[No Name]");
    p.keys(&["ione", "\x1b", "otwo", "\x1b", "othree", "\x1b", ":w\r"]);
    assert_eq!(cmd_row(&p), "? no file name (use :w name)");
    p.keys(&[":2,3w part.txt\r"]);
    assert_eq!(cmd_row(&p), "\"part.txt\" 2L, 10B written");
    assert_eq!(env.read("part.txt"), "two\nthree\n");
    assert!(status_row(&p).contains("[No Name] [+]"));
    p.keys(&[":2,3w part.txt\r"]);
    assert_eq!(cmd_row(&p), "? file exists (w! to overwrite)");
    p.keys(&[":w new.txt\r"]);
    p.wait_text("\"new.txt\" 3L, 14B written");
    assert!(status_row(&p).contains("new.txt"));
    assert!(!status_row(&p).contains("[+]"));
    p.keys(&["ofour", "\x1b", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(env.read("new.txt"), "one\ntwo\nthree\nfour\n");
    assert!(p.screen().contains("\"new.txt\" 4L, 19B written"));
    assert!(env.swap_files().is_empty());
}

#[test]
#[ignore = "deferred minor: filetype is detected at open only"]
fn bug_naming_an_unnamed_buffer_does_not_detect_its_filetype() {
    let env = Env::new();
    let mut p = env.fred(&[]);
    p.wait_text("[No Name]");
    p.keys(&["ifn main() {}", "\x1b", ":w main.rs\r"]);
    p.wait_text("written");
    let status = status_row(&p);
    p.keys(&[":q\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert!(
        status.contains("rust"),
        "after `:w main.rs` names the buffer, highlighting stays Plain Text until restart: \
         {status:?}"
    );
}

#[test]
fn start_line_and_height_arguments() {
    let env = Env::new();
    let text: String = (1..=50).map(|i| format!("line{i}\n")).collect();
    env.write("n.txt", &text);
    for (args, first, cursor_line, text_rows) in [
        (&["+30", "n.txt"][..], 21, 30, 12),
        (&["+", "n.txt"][..], 39, 50, 12),
        (&["+0", "n.txt"][..], 1, 1, 12),
        (&["+999", "n.txt"][..], 39, 50, 12),
        (&["n.txt", "+10"][..], 1, 10, 12),
        (&["--height", "3", "n.txt"][..], 1, 1, 3),
        (&["--height", "3", "+25", "n.txt"][..], 24, 25, 3),
    ] {
        // Each case starts fresh: fred remembers where the last one left off.
        let _ = fs::remove_file(env.path("state/fred/recent"));
        let mut p = env.fred(args);
        p.wait_text("NORMAL");
        let rows: Vec<String> = p.rows().into_iter().filter(|r| !r.is_empty()).collect();
        assert_eq!(rows.len(), text_rows + 2, "{args:?}: {rows:?}");
        assert_eq!(rows[0], format!("{first:>3} line{first}"), "{args:?}");
        assert!(
            rows[text_rows].ends_with(&format!(" {cursor_line}:1")),
            "{args:?}: {}",
            rows[text_rows]
        );
        p.keys(&[":q\r"]);
        assert_eq!(p.wait_exit(), 0);
    }
}

// ============================================================ extras

#[test]
fn tab_inserts_a_tab_in_a_new_makefile() {
    let env = Env::new();
    let mut p = env.fred(&["Makefile"]);
    p.wait_text("[new]");
    p.keys(&["iall:", "\r", "\t", "cc -o x x.c", "\x1b", ":wq\r"]);
    assert_eq!(p.wait_exit(), 0);
    assert_eq!(
        env.read("Makefile"),
        "all:\n\tcc -o x x.c\n",
        "spec: Tab inserts `\\t`, or spaces only if the file is detected as space-indented; a \
         new (or unindented) file defaults to 4 spaces, which breaks Makefiles"
    );
}

/// Tiny deterministic PRNG for the fuzz test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[(self.next() % v.len() as u64) as usize]
    }
}

/// Random vim keys and ex commands; fred must never crash (took ~75 s).
#[test]
#[ignore]
fn fuzz_random_keys_never_crash() {
    #[rustfmt::skip]
    let keys: &[&str] = &[
        "h", "j", "k", "l", "w", "b", "e", "W", "B", "E", "x", "X", "dd", "dw", "db", "de", "D",
        "p", "P", "u", "\x12", "J", "3J", "o", "O", "i", "a", "A", "I", "r", "rx", ".", "V", "Vjd",
        "Vkc", "yy", "3j", "5k", "G", "gg", "$", "0", "^", "{", "}", "fa", "tb", ";", ",", "/a\r",
        "?b\r", "n", "N", "s", "S", "C", "cw", "c$", "\x1b", "\r", "\t", "\x7f", "漢", "é",
        "👍🏽", " ", "abc", "\x04", "\x15", "\x06", "\x02", "\x0e", "\x10", "\x17", "ma", "'a",
        "mb", "\x1b[A", "\x1b[B", "\x1b[C", "\x1b[D", "\x1b[H", "\x1b[F", "\x1b[3~",
        ":1,$s/a/b/g\r", ":g/x/d\r", ":v/./d\r", ":m0\r", ":t$\r", ":2,1d\r", ":'a,'bd\r",
        ":0\r", ":$\r", ":.-5\r", ":/a/\r", ":?b?\r", ":j\r", ":1,$j\r", ":s/x/\\n/g\r",
        ":g/^/m0\r", ":.,+3m$\r", ":'a,'bt0\r", ":g/a/s/a/\\n/g\r", ":$-2,$j\r", ":;+1d\r",
        ":s/$/漢/\r", ":s/^/\\t/\r", ":g/b/j\r", ":v/a/t.\r", ":3\r", ":s//x/\r", ":s/(/x/\r",
        "\x1b[200~pa\rste\x1b[201~", "\x1b\x1b",
    ];
    let mut failures = vec![];
    for seed in 1..=12u64 {
        let env = Env::new();
        let small = seed > 6;
        if small {
            env.config("wrap = true\nrelative_numbers = true\ntabstop = 3\n");
        }
        env.write(
            "f.txt",
            "abc def\n\tx漢字 b\n\nlast a b c\ne\u{301}👍🏽 a\n  indented b\nend\n",
        );
        let (rows, cols) = if small { (6, 15) } else { (12, 40) };
        let mut p = env.fred_sized(&["f.txt"], rows, cols);
        p.wait_text("NORMAL");
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log = vec![];
        for step in 0..400 {
            let k = rng.pick(keys);
            log.push(k);
            p.send(k);
            thread::sleep(Duration::from_millis(12));
            if step % 20 == 0 && !p.running() {
                break;
            }
        }
        thread::sleep(Duration::from_millis(200));
        if !p.running() || p.raw().contains("panicked") {
            let raw = p.raw();
            let i = raw.find("panicked").unwrap_or(0);
            failures.push(format!(
                "seed {seed}: died; last keys {:?}\n{}",
                &log[log.len().saturating_sub(15)..],
                &raw[i..raw.len().min(i + 300)]
            ));
        } else {
            p.keys(&["\x1b", "\x1b", ":q!\r"]);
            assert_eq!(p.wait_exit(), 0);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
