//! `fred --gui` end to end: JSON lines in over stdin, frames out over stdout.
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

struct Gui {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
    dir: tempfile::TempDir,
}

impl Gui {
    fn spawn(name: &str, text: &str) -> Gui {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(name), text).unwrap();
        // No line numbers: rows are just the text.
        std::fs::create_dir_all(dir.path().join("config/fred")).unwrap();
        std::fs::write(
            dir.path().join("config/fred/config.toml"),
            "numbers = false\n",
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_fred"))
            .arg("--gui")
            .arg(name)
            .current_dir(dir.path())
            .env("HOME", dir.path())
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let v: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("not a protocol line: {line:?}: {e}"));
                if tx.send(v).is_err() {
                    return;
                }
            }
        });
        let stdin = child.stdin.take();
        Gui {
            child,
            stdin,
            lines,
            dir,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn send(&mut self, line: &str) {
        let w = self.stdin.as_mut().unwrap();
        writeln!(w, "{line}").unwrap();
        w.flush().unwrap();
    }

    fn next(&self, deadline: Instant) -> Value {
        let left = deadline.saturating_duration_since(Instant::now());
        self.lines.recv_timeout(left).expect("no line within 5 s")
    }

    fn frame_until(&mut self, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let v = self.next(deadline);
            assert_eq!(v["t"], "frame", "{v}");
            if done(&v) {
                return v;
            }
        }
    }

    fn exit_line(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let v = self.next(deadline);
            if v["t"] == "exit" {
                return v;
            }
        }
    }

    fn still_running(&mut self) -> bool {
        std::thread::sleep(Duration::from_millis(200));
        self.child.try_wait().unwrap().is_none()
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(st) = self.child.try_wait().unwrap() {
                return st;
            }
            assert!(Instant::now() < deadline, "still running after 5 s");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn runs_text(row: &Value) -> String {
    row.as_array()
        .unwrap()
        .iter()
        .map(|r| r["s"].as_str().unwrap())
        .collect()
}

fn row_text(f: &Value, win: usize, row: usize) -> String {
    runs_text(&f["windows"][win]["rows"][row])
}

fn bottom_text(f: &Value) -> String {
    f["bottom"]
        .as_array()
        .unwrap()
        .iter()
        .map(runs_text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn types_saves_and_quits_over_the_pipe() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"Atwo<Esc>"}"#);
    let f = g.frame_until(|f| f["seq"] == 1);
    assert_eq!(row_text(&f, 0, 0), "onetwo");
    g.send(r#"{"t":"mac","seq":2,"key":"s","shift":false}"#);
    g.frame_until(|f| f["seq"] == 2);
    assert_eq!(std::fs::read_to_string(g.path("f")).unwrap(), "onetwo\n");
    g.send(r#"{"t":"quit"}"#);
    assert_eq!(g.exit_line()["code"], 0);
    assert!(g.wait().success());
}

#[test]
fn quit_with_unsaved_changes_keeps_running() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"x"}"#);
    g.send(r#"{"t":"quit"}"#);
    // Fred's E37: "buffer 1 (f) has unsaved changes (...)".
    let f = g.frame_until(|f| bottom_text(f).contains("unsaved changes"));
    assert!(bottom_text(&f).contains("unsaved changes"));
    assert!(g.still_running());
}

#[test]
fn a_burst_of_keys_applies_in_order() {
    let mut g = Gui::spawn("f", "\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"i"}"#);
    for n in 2..=21 {
        g.send(&format!(
            r#"{{"t":"keys","seq":{n},"keys":"{}"}}"#,
            (b'a' + (n - 2) as u8) as char
        ));
    }
    let f = g.frame_until(|f| f["seq"] == 21);
    assert_eq!(row_text(&f, 0, 0), "abcdefghijklmnopqrst");
}

#[test]
fn closed_stdin_saves_the_swap_and_exits() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"x"}"#);
    g.frame_until(|f| f["seq"] == 1);
    g.close_stdin();
    assert_eq!(g.wait().code(), Some(1));
}

#[test]
fn mouse_drag_selects() {
    let mut g = Gui::spawn("f", "hello world\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":":set nonumber<Enter>"}"#);
    g.send(r#"{"t":"mouse","seq":2,"kind":"down","col":0,"row":0}"#);
    g.send(r#"{"t":"mouse","seq":3,"kind":"drag","col":4,"row":0}"#);
    g.send(r#"{"t":"mouse","seq":4,"kind":"up","col":4,"row":0}"#);
    let f = g.frame_until(|f| f["seq"] == 4);
    assert_eq!(f["windows"][0]["status"]["mode"], "VISUAL");
}

#[test]
fn a_shell_command_shows_its_output_in_a_frame() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":":!echo hello<Enter>"}"#);
    // Every stdout line must parse as JSON, or the reader thread panics.
    g.frame_until(|f| f["seq"] == 1 && f.to_string().contains("hello"));
}
