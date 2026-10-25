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
            if v["t"] == "minimap" {
                continue;
            }
            assert_eq!(v["t"], "frame", "{v}");
            if done(&v) {
                return v;
            }
        }
    }

    /// Every line for `d`.
    fn lines_for(&mut self, d: Duration) -> Vec<Value> {
        let deadline = Instant::now() + d;
        std::iter::from_fn(|| {
            let left = deadline.saturating_duration_since(Instant::now());
            self.lines.recv_timeout(left).ok()
        })
        .collect()
    }

    /// Every line up to and including the next minimap.
    fn until_minimap(&mut self) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut lines = vec![];
        loop {
            let v = self.next(deadline);
            let done = v["t"] == "minimap";
            lines.push(v);
            if done {
                return lines;
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
    g.send(r#"{"t":"mouse","seq":2,"kind":"down","col":0,"row":1}"#);
    g.send(r#"{"t":"mouse","seq":3,"kind":"drag","col":4,"row":1}"#);
    g.send(r#"{"t":"mouse","seq":4,"kind":"up","col":4,"row":1}"#);
    let f = g.frame_until(|f| f["seq"] == 4);
    // `fred --gui` sets selectmode=mouse: the drag starts Select mode.
    assert_eq!(f["windows"][0]["status"]["mode"], "SELECT");
    // The selection is a span, not reversed cells.
    assert_eq!(
        f["windows"][0]["sel"],
        serde_json::json!([{"y":0,"x":0,"w":5}])
    );
    let row = f["windows"][0]["rows"][0].as_array().unwrap();
    assert!(
        row.iter().all(|r| r["f"].as_u64().unwrap_or(0) & 8 == 0),
        "{row:?}"
    );
    // Typing replaces it.
    g.send(r#"{"t":"keys","seq":5,"keys":"x"}"#);
    let f = g.frame_until(|f| f["seq"] == 5);
    assert_eq!(row_text(&f, 0, 0), "x world");
    assert_eq!(f["windows"][0]["status"]["mode"], "INSERT");
    assert!(f["windows"][0].get("sel").is_none(), "no selection, no sel");
}

#[test]
fn a_shell_command_shows_its_output_in_a_frame() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":":!echo hello<Enter>"}"#);
    // Every stdout line must parse as JSON, or the reader thread panics.
    g.frame_until(|f| f["seq"] == 1 && f.to_string().contains("hello"));
}

#[test]
fn quit_during_a_job_waits_for_it() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":":!sleep 1<Enter>"}"#);
    g.frame_until(|f| f["seq"] == 1);
    let start = Instant::now();
    g.send(r#"{"t":"quit"}"#);
    assert_eq!(g.exit_line()["code"], 0);
    assert!(
        start.elapsed() >= Duration::from_millis(700),
        "quit before the job ended"
    );
}

#[test]
fn open_shows_the_file_in_the_active_window() {
    let mut g = Gui::spawn("a", "one\n");
    std::fs::write(g.path("b"), "two\n").unwrap();
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    let b = g.path("b").display().to_string();
    g.send(&format!(
        r#"{{"t":"open","seq":1,"path":{}}}"#,
        Value::from(b.clone())
    ));
    let f = g.frame_until(|f| f["seq"] == 1);
    let w = &f["windows"][0];
    assert_eq!(w["status"]["name"], "b");
    assert_eq!(row_text(&f, 0, 0), "two");
    assert!(w["path"].as_str().unwrap().ends_with("/b"), "{w}");
}

#[test]
fn open_while_modified_keeps_the_change_in_a_hidden_buffer() {
    // As the `Space p` picker: the modified buffer stays open, hidden.
    let mut g = Gui::spawn("a", "one\n");
    std::fs::write(g.path("b"), "two\n").unwrap();
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"x"}"#);
    let (a, b) = (g.path("a"), g.path("b"));
    let open = |seq: u32, p: &PathBuf| {
        format!(
            r#"{{"t":"open","seq":{seq},"path":{}}}"#,
            Value::from(p.display().to_string())
        )
    };
    g.send(&open(2, &b));
    let f = g.frame_until(|f| f["seq"] == 2);
    assert_eq!(f["windows"][0]["status"]["name"], "b");
    g.send(&open(3, &a));
    let f = g.frame_until(|f| f["seq"] == 3);
    assert_eq!(f["windows"][0]["status"]["name"], "a");
    assert_eq!(f["windows"][0]["status"]["modified"], true);
    assert_eq!(row_text(&f, 0, 0), "ne");
}

fn open_line(seq: u32, p: &std::path::Path, preview: bool) -> String {
    format!(
        r#"{{"t":"open","seq":{seq},"path":{},"preview":{preview}}}"#,
        Value::from(p.display().to_string())
    )
}

/// The active window's tabs: (name, preview, current).
fn tabs(f: &Value) -> Vec<(String, bool, bool)> {
    let w = f["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["active"] == true)
        .unwrap();
    w["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["preview"].as_bool().unwrap(),
                t["current"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn names(f: &Value) -> Vec<String> {
    tabs(f).into_iter().map(|t| t.0).collect()
}

#[test]
fn a_click_on_the_first_text_row_lands_on_line_one() {
    let mut g = Gui::spawn("f", "one\ntwo\nthree\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10,"margin":8}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"jj"}"#);
    g.send(r#"{"t":"mouse","seq":2,"kind":"down","col":1,"row":1}"#);
    g.send(r#"{"t":"mouse","seq":3,"kind":"up","col":1,"row":1}"#);
    let f = g.frame_until(|f| f["seq"] == 3);
    let w = &f["windows"][0];
    assert_eq!(
        (w["header"].as_u64(), w["margin"].as_u64()),
        (Some(1), Some(8))
    );
    assert_eq!(w["status"]["pos"], "1:2");
    assert_eq!(row_text(&f, 0, 0), "one");
    // The tab row itself is not text.
    g.send(r#"{"t":"keys","seq":4,"keys":"G"}"#);
    g.send(r#"{"t":"mouse","seq":5,"kind":"down","col":1,"row":0}"#);
    let f = g.frame_until(|f| f["seq"] == 5);
    // G keeps the column ('nostartofline').
    assert_eq!(f["windows"][0]["status"]["pos"], "3:2");
}

#[test]
fn tab_ops_select_close_move_keep_over_the_pipe() {
    let mut g = Gui::spawn("a", "a\n");
    std::fs::write(g.path("b"), "b\n").unwrap();
    std::fs::write(g.path("c"), "c\n").unwrap();
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    let (b, c) = (g.path("b"), g.path("c"));
    g.send(&open_line(1, &b, false));
    g.send(&open_line(2, &c, false));
    let f = g.frame_until(|f| f["seq"] == 2);
    assert_eq!(names(&f), ["a", "b", "c"]);
    let id = f["windows"][0]["id"].as_u64().unwrap();
    g.send(&format!(
        r#"{{"t":"tab","seq":3,"win":{id},"op":"select","index":0}}"#
    ));
    let f = g.frame_until(|f| f["seq"] == 3);
    assert_eq!(f["windows"][0]["status"]["name"], "a");
    g.send(&format!(
        r#"{{"t":"tab","seq":4,"win":{id},"op":"move","index":0,"to":2}}"#
    ));
    let f = g.frame_until(|f| f["seq"] == 4);
    assert_eq!(names(&f), ["b", "c", "a"]);
    g.send(&format!(
        r#"{{"t":"tab","seq":5,"win":{id},"op":"close","index":0}}"#
    ));
    let f = g.frame_until(|f| f["seq"] == 5);
    assert_eq!(names(&f), ["c", "a"]);
    g.send(&open_line(6, &b, true));
    let f = g.frame_until(|f| f["seq"] == 6);
    assert_eq!(
        tabs(&f),
        [
            ("c".into(), false, false),
            ("a".into(), false, false),
            ("b".into(), true, true)
        ]
    );
    g.send(&format!(
        r#"{{"t":"tab","seq":7,"win":{id},"op":"keep","index":2}}"#
    ));
    let f = g.frame_until(|f| f["seq"] == 7);
    assert_eq!(tabs(&f)[2], ("b".into(), false, true));
    // A stale index (a fast double close) is ignored: no crash, no error.
    g.send(&format!(
        r#"{{"t":"tab","seq":8,"win":{id},"op":"close","index":9}}"#
    ));
    let f = g.frame_until(|f| f["seq"] == 8);
    assert!(
        !bottom_text(&f).contains("no such tab"),
        "{}",
        bottom_text(&f)
    );
    assert_eq!(names(&f), ["c", "a", "b"]);
}

#[test]
fn open_preview_then_open_preview_replaces_the_tab() {
    let mut g = Gui::spawn("a", "a\n");
    std::fs::write(g.path("b"), "b\n").unwrap();
    std::fs::write(g.path("c"), "c\n").unwrap();
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    let (b, c) = (g.path("b"), g.path("c"));
    g.send(&open_line(1, &b, true));
    let f = g.frame_until(|f| f["seq"] == 1);
    assert_eq!(tabs(&f)[1], ("b".into(), true, true));
    g.send(&open_line(2, &c, true));
    let f = g.frame_until(|f| f["seq"] == 2);
    assert_eq!(
        tabs(&f),
        [("a".into(), false, false), ("c".into(), true, true)]
    );
    // A paste into the preview keeps it.
    g.send(r#"{"t":"paste","seq":3,"text":"x"}"#);
    let f = g.frame_until(|f| f["seq"] == 3);
    assert_eq!(tabs(&f)[1], ("c".into(), false, true));
}

#[test]
fn scroll_sets_the_windows_top() {
    let text: String = (1..=100).map(|n| format!("{n}\n")).collect();
    let mut g = Gui::spawn("f", &text);
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    let f = g.frame_until(|_| true);
    let id = f["windows"][0]["id"].as_u64().unwrap();
    g.send(&format!(r#"{{"t":"scroll","seq":1,"win":{id},"top":50}}"#));
    let f = g.frame_until(|f| f["seq"] == 1);
    let w = &f["windows"][0];
    assert_eq!(
        (w["top"].as_u64(), w["lines"].as_u64()),
        (Some(50), Some(100))
    );
    assert_eq!(row_text(&f, 0, 0), "51");
    // The cursor stays where it was, as with the wheel.
    assert_eq!(w["status"]["pos"], "1:1");
    // Past the end: clamped to the last screenful.
    g.send(&format!(
        r#"{{"t":"scroll","seq":2,"win":{id},"top":5000}}"#
    ));
    let f = g.frame_until(|f| f["seq"] == 2);
    let rows = f["windows"][0]["rows"].as_array().unwrap().len();
    assert_eq!(row_text(&f, 0, rows - 1), "100");
}

#[test]
fn cmd_w_closes_the_tab_not_the_window() {
    let mut g = Gui::spawn("a", "a\n");
    std::fs::write(g.path("b"), "b\n").unwrap();
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(&open_line(1, &g.path("b"), false));
    g.send(r#"{"t":"keys","seq":2,"keys":"i"}"#);
    g.send(r#"{"t":"mac","seq":3,"key":"w","shift":false}"#);
    let f = g.frame_until(|f| f["seq"] == 3);
    assert_eq!(tabs(&f), [("a".into(), false, true)]);
    assert_eq!(f["windows"][0]["status"]["mode"], "NORMAL");
    g.send(r#"{"t":"mac","seq":4,"key":"w","shift":false}"#);
    let f = g.frame_until(|f| f["seq"] == 4);
    assert_eq!(names(&f), ["[No Name]"]);
    assert!(g.still_running());
}

fn minimaps(lines: &[Value]) -> Vec<&Value> {
    lines.iter().filter(|v| v["t"] == "minimap").collect()
}

/// Line 1's first run: its column and length.
fn first_run(m: &Value) -> (u64, u64) {
    let r = &m["lines"][0][0];
    (r[0].as_u64().unwrap(), r[1].as_u64().unwrap())
}

#[test]
fn a_minimap_follows_the_first_frame() {
    let mut g = Gui::spawn("f.rs", "fn main() {\n\tx\n}\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10,"margin":8}"#);
    let mut lines = g.until_minimap();
    lines.extend(g.lines_for(Duration::from_millis(400)));
    assert_eq!(lines[0]["t"], "frame");
    let id = &lines[0]["windows"][0]["id"];
    let m = minimaps(&lines);
    assert_eq!(m.len(), 1, "{lines:?}");
    assert_eq!(&m[0]["win"], id);
    let starts: Vec<Value> = m[0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l[0].clone())
        .collect();
    // `fn` is purple; the tab is expanded (8 columns by default).
    assert_eq!(starts[0], serde_json::json!([0, 2, 0x100_0000 | 0xC792EA]));
    assert_eq!(
        (&starts[1][0], &starts[1][1]),
        (&serde_json::json!(8), &serde_json::json!(1))
    );
    assert_eq!(starts.len(), 3);
}

#[test]
fn no_margin_no_minimap() {
    let mut g = Gui::spawn("f", "x\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    assert!(minimaps(&g.lines_for(Duration::from_millis(400))).is_empty());
}

#[test]
fn fifty_keys_in_one_batch_send_at_most_one_minimap() {
    let mut g = Gui::spawn("f", "x\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10,"margin":8}"#);
    g.until_minimap();
    let batch: Vec<String> = (1..=50)
        .map(|n| {
            format!(
                r#"{{"t":"keys","seq":{n},"keys":"{}"}}"#,
                if n == 1 { "A" } else { "y" }
            )
        })
        .collect();
    g.send(&batch.join("\n"));
    let mut lines = g.until_minimap();
    lines.extend(g.lines_for(Duration::from_millis(600)));
    let m = minimaps(&lines);
    assert_eq!(m.len(), 1, "{m:?}");
    // It shows the text after the whole batch: `x` and 49 `y`s.
    assert_eq!(first_run(m[0]), (0, 50));
    // And it comes after the batch's last frame.
    let last = lines.iter().rposition(|v| v["seq"] == 50).unwrap();
    assert!(lines.iter().position(|v| v["t"] == "minimap").unwrap() > last);
}

#[test]
fn switching_tabs_between_unedited_files_sends_a_minimap_for_each() {
    let mut g = Gui::spawn("a", "a\n");
    std::fs::write(g.path("b"), "bb\n").unwrap();
    g.send(r#"{"t":"resize","cols":40,"rows":10,"margin":8}"#);
    g.until_minimap();
    g.send(&open_line(1, &g.path("b"), false));
    assert_eq!(first_run(g.until_minimap().last().unwrap()), (0, 2));
    g.send(r#"{"t":"keys","seq":2,"keys":"<C-^>"}"#);
    assert_eq!(first_run(g.until_minimap().last().unwrap()), (0, 1));
}

#[test]
fn the_wheel_scrolls_over_the_minimap_and_tab_row() {
    let text: String = (1..=100).map(|n| format!("{n}\n")).collect();
    let mut g = Gui::spawn("f", &text);
    g.send(r#"{"t":"resize","cols":80,"rows":20,"margin":12}"#);
    let top = |f: &Value| f["windows"][0]["top"].as_u64().unwrap();
    g.send(r#"{"t":"mouse","seq":1,"kind":"scroll_down","col":75,"row":5}"#);
    let t1 = top(&g.frame_until(|f| f["seq"] == 1));
    assert!(t1 > 0, "the minimap's wheel scrolls");
    g.send(r#"{"t":"mouse","seq":2,"kind":"scroll_down","col":10,"row":5}"#);
    let t2 = top(&g.frame_until(|f| f["seq"] == 2));
    assert!(t2 > t1, "the text's wheel scrolls");
    g.send(r#"{"t":"mouse","seq":3,"kind":"scroll_down","col":10,"row":0}"#);
    assert!(
        top(&g.frame_until(|f| f["seq"] == 3)) > t2,
        "the tab row's too"
    );
    // A click on the minimap is still not a click on the text.
    g.send(r#"{"t":"mouse","seq":4,"kind":"down","col":75,"row":5}"#);
    let f = g.frame_until(|f| f["seq"] == 4);
    assert_eq!(f["windows"][0]["status"]["pos"], "1:1");
}

#[test]
fn slow_typing_sends_no_minimap_until_a_pause() {
    let mut g = Gui::spawn("f", "x\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10,"margin":8}"#);
    g.until_minimap();
    let mut lines = vec![];
    for n in 1..=8 {
        let k = if n == 1 { "A" } else { "y" };
        g.send(&format!(r#"{{"t":"keys","seq":{n},"keys":"{k}"}}"#));
        lines.extend(g.lines_for(Duration::from_millis(120)));
    }
    assert!(minimaps(&lines).is_empty(), "{lines:?}");
    assert_eq!(first_run(g.until_minimap().last().unwrap()), (0, 8));
}

#[test]
fn narrow_windows_get_no_minimap_margin() {
    let mut g = Gui::spawn("f", "x\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10,"margin":8}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"<C-w>v"}"#);
    let f = g.frame_until(|f| f["seq"] == 1);
    let ws = f["windows"].as_array().unwrap();
    assert_eq!(ws.len(), 2);
    assert!(ws.iter().all(|w| w["margin"] == 0), "{ws:?}");
    g.send(r#"{"t":"keys","seq":2,"keys":"<C-w>o"}"#);
    let f = g.frame_until(|f| f["seq"] == 2);
    assert_eq!(f["windows"][0]["margin"], 8);
}
