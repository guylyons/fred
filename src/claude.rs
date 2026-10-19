//! `:claude` / `Space c`: Claude Code in a fred buffer, laid out like its
//! CLI (`> ` prompts, `⏺` replies and tools, `⎿` results). Enter on the
//! prompt sends it, Ctrl-J breaks the line, Ctrl-C interrupts. Each prompt
//! runs `claude_command` (`--resume`d, so it is one conversation) with the
//! reply streaming in while you edit, here or in another buffer.

use crate::buffer::Edit;
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

pub struct Chat {
    cmd: String,
    session: Option<String>,
    job: Option<Job>,
}

struct Job {
    child: Child,
    rx: Receiver<Out>,
    start: Instant,
}

/// What a line of `--output-format stream-json` adds to the chat.
#[derive(Debug, PartialEq)]
pub enum Out {
    /// A reply starts: a new `⏺`.
    Block,
    Text(String),
    /// `⏺ Bash(cargo test)`.
    Tool(String),
    /// `⎿  first line of the tool's output`.
    Result(String),
    Session(String),
    /// The turn is over; the status line says how it went.
    Done(String),
}

impl Chat {
    pub fn new(cmd: &str) -> Chat {
        Chat {
            cmd: cmd.into(),
            session: None,
            job: None,
        }
    }
}

/// The CLI's spinner on the status line, while Claude works.
pub fn status(ed: &Editor) -> Option<String> {
    const SPIN: [char; 6] = ['·', '✢', '✳', '✶', '✻', '✽'];
    let t = ed.claude.as_ref()?.job.as_ref()?.start.elapsed();
    let c = SPIN[(t.as_millis() / 120) as usize % SPIN.len()];
    Some(format!("{c} Working… {}s · C-c to interrupt", t.as_secs()))
}

pub fn busy(ed: &Editor) -> bool {
    ed.claude.as_ref().is_some_and(|c| c.job.is_some())
}

/// Enter (on the prompt), Ctrl-J and Ctrl-C in a chat buffer.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    let Some(chat) = &mut ed.claude else {
        return false;
    };
    let insert = ed.mode == Mode::Insert;
    let normal = ed.mode == Mode::Normal && ed.vim.pending.is_empty();
    if !(insert || normal) {
        return false;
    }
    if k == Key::ctrl('c') && chat.job.is_some() {
        let mut job = chat.job.take().unwrap();
        let _ = job.child.kill();
        let _ = job.child.wait();
        put(ed, "\n  ⎿  Interrupted");
        ed.set_msg("interrupted");
        return true;
    }
    if insert && k == Key::ctrl('j') {
        crate::vim::ops::insert_text(ed, "\n");
        return true;
    }
    let plain_enter = k.code == KeyCode::Enter && !k.ctrl && !k.alt && !k.shift;
    if plain_enter && ed.popup.is_none() && prompt_line(ed).is_some_and(|p| ed.cur.line >= p) {
        send(ed);
        return true;
    }
    false
}

/// The last line starting with `>`: the prompt being typed.
fn prompt_line(ed: &Editor) -> Option<usize> {
    (0..ed.line_count())
        .rev()
        .find(|&l| ed.buf.line(l).starts_with('>'))
}

fn send(ed: &mut Editor) {
    let Some(p) = prompt_line(ed) else { return };
    let start = ed.buf.line_to_char(p);
    let text = ed.buf.slice(start, ed.buf.len_chars());
    let prompt = text.trim_start_matches('>').trim().to_string();
    if prompt.is_empty() {
        return;
    }
    let chat = ed.claude.as_mut().unwrap();
    if chat.job.is_some() {
        return ed.set_msg("Claude is working · C-c to interrupt");
    }
    if prompt == "/clear" {
        chat.session = None;
        let end = ed.buf.len_chars();
        ed.apply(Edit {
            start: 0,
            end,
            text: "> ".into(),
        });
        ed.set_cursor(0, 2);
        return ed.set_msg("new conversation");
    }
    let cmd = match &chat.session {
        Some(id) => format!("{} --resume {id}", chat.cmd),
        None => chat.cmd.clone(),
    };
    let end = ed.buf.len_chars();
    ed.apply(Edit {
        start: end,
        end,
        text: "\n\n> ".into(),
    });
    let last = ed.line_count() - 1;
    ed.set_cursor(last, 2);
    match spawn(&cmd, prompt) {
        Ok(job) => {
            ed.claude.as_mut().unwrap().job = Some(job);
            ed.msg = status(ed).map(|s| (s, false));
        }
        Err(e) => put(ed, &format!("\n\n⏺ {e}")),
    }
}

fn spawn(cmd: &str, prompt: String) -> Result<Job, String> {
    let mut child = crate::shell::command(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("can't run claude: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    std::thread::spawn(move || stdin.write_all(prompt.as_bytes()));
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            for o in parse(&line) {
                if tx.send(o).is_err() {
                    return;
                }
            }
        }
    });
    Ok(Job {
        child,
        rx,
        start: Instant::now(),
    })
}

/// Put what arrived into the chat. True if anything changed.
pub fn tick(ed: &mut Editor) -> bool {
    let Some(job) = ed.claude.as_mut().and_then(|c| c.job.as_mut()) else {
        return false;
    };
    let mut outs = vec![];
    let ended = loop {
        match job.rx.try_recv() {
            Ok(o) => outs.push(o),
            Err(TryRecvError::Empty) => break false,
            Err(TryRecvError::Disconnected) => break true,
        }
    };
    let changed = !outs.is_empty() || ended;
    for o in outs {
        match o {
            Out::Block => put(ed, "\n\n⏺ "),
            Out::Text(t) => put(ed, &t.replace('\n', "\n  ")),
            Out::Tool(t) => put(ed, &format!("\n\n⏺ {t}")),
            Out::Result(r) => put(ed, &format!("\n  ⎿  {r}")),
            Out::Session(id) => ed.claude.as_mut().unwrap().session = Some(id),
            // The turn is over, though claude may still be exiting (its
            // hooks): reap it in the background so the next prompt can go.
            Out::Done(m) => {
                if let Some(mut job) = ed.claude.as_mut().unwrap().job.take() {
                    std::thread::spawn(move || job.child.wait());
                }
                ed.set_msg(m);
                break;
            }
        }
    }
    if ended && let Some(mut job) = ed.claude.as_mut().unwrap().job.take() {
        let _ = job.child.wait();
        put(
            ed,
            "\n\n⏺ claude exited without answering (see claude_command)",
        );
        ed.msg = None;
    }
    ed.buf.modified = false;
    changed
}

/// Insert `text` at the end of the transcript, just before the prompt's
/// blank line, moving the cursor along if it is after that point.
fn put(ed: &mut Editor, text: &str) {
    if prompt_line(ed).is_none() {
        let end = ed.buf.len_chars();
        ed.apply(Edit {
            start: end,
            end,
            text: "\n\n> ".into(),
        });
    }
    let p = prompt_line(ed).unwrap();
    let at = ed.buf.line_to_char(p).saturating_sub(2);
    let cur = ed.buf.pos_to_char(ed.cur.line, ed.cur.byte);
    ed.apply(Edit {
        start: at,
        end: at,
        text: text.into(),
    });
    if cur >= at {
        let (l, b) = ed.buf.char_to_pos(cur + text.chars().count());
        ed.cur.line = l;
        ed.cur.byte = b;
    }
}

pub fn parse(line: &str) -> Vec<Out> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let content = || {
        v["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    match v["type"].as_str() {
        Some("system") if v["subtype"] == "init" => vec![Out::Session(s(&v["session_id"]))],
        Some("stream_event") => {
            let e = &v["event"];
            match e["type"].as_str() {
                Some("content_block_start") if e["content_block"]["type"] == "text" => {
                    vec![Out::Block]
                }
                Some("content_block_delta") if e["delta"]["type"] == "text_delta" => {
                    vec![Out::Text(s(&e["delta"]["text"]))]
                }
                _ => vec![],
            }
        }
        Some("assistant") => content()
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .map(|b| Out::Tool(tool(&s(&b["name"]), &b["input"])))
            .collect(),
        Some("user") => content()
            .iter()
            .filter(|b| b["type"] == "tool_result")
            .map(|b| Out::Result(result(&b["content"], b["is_error"] == true)))
            .collect(),
        Some("result") => {
            let mut m = if v["is_error"] == true {
                format!("? {}", s(&v["result"]))
            } else {
                format!(
                    "done in {:.0}s · ${:.2}",
                    v["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0,
                    v["total_cost_usd"].as_f64().unwrap_or(0.0)
                )
            };
            let denied = v["permission_denials"].as_array().map_or(0, Vec::len);
            if denied > 0 {
                m += &format!(" · {denied} tool use(s) denied (see claude_command)");
            }
            vec![Out::Session(s(&v["session_id"])), Out::Done(m)]
        }
        _ => vec![],
    }
}

/// `Bash(cargo test)`, as the CLI shows a tool call.
fn tool(name: &str, input: &Value) -> String {
    let arg = [
        "file_path",
        "command",
        "pattern",
        "path",
        "url",
        "query",
        "description",
    ]
    .iter()
    .find_map(|k| input[k].as_str())
    .unwrap_or_default();
    format!("{name}({})", clip(arg.lines().next().unwrap_or_default()))
}

/// A tool's output, in one line: its first, and how many more.
fn result(content: &Value, err: bool) -> String {
    let text = match content {
        Value::String(t) => t.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let first = clip(lines.next().unwrap_or("(no output)"));
    let more = lines.count();
    let err = if err { "Error: " } else { "" };
    match more {
        0 => format!("{err}{first}"),
        n => format!("{err}{first} … +{n} lines"),
    }
}

fn clip(s: &str) -> String {
    match s.char_indices().nth(80) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    #[test]
    fn parses_the_stream() {
        let p = |l: &str| parse(l);
        assert_eq!(
            p(r#"{"type":"system","subtype":"init","session_id":"S"}"#),
            [Out::Session("S".into())]
        );
        assert_eq!(
            p(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#
            ),
            [Out::Text("hi".into())]
        );
        assert_eq!(
            p(
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"echo ok","description":"x"}}]}}"#
            ),
            [Out::Tool("Bash(echo ok)".into())]
        );
        assert_eq!(
            p(
                r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"a\nb\nc"}]}}"#
            ),
            [Out::Result("a … +2 lines".into())]
        );
        assert!(matches!(
            &p(r#"{"type":"result","is_error":false,"session_id":"S","duration_ms":2000,"total_cost_usd":0.1,"permission_denials":[]}"#)[..],
            [Out::Session(_), Out::Done(m)] if m == "done in 2s · $0.10"
        ));
        assert!(p("not json").is_empty());
    }

    #[test]
    fn replies_go_above_the_prompt_being_typed() {
        let mut ed = Editor::new(Buffer::from_text("> hi\n\n> next"));
        ed.claude = Some(Box::new(Chat::new("true")));
        ed.set_cursor(2, 6);
        put(&mut ed, "\n\n⏺ ");
        put(&mut ed, "hello\nthere");
        assert_eq!(ed.buf.text(), "> hi\n\n⏺ hello\nthere\n\n> next");
        assert_eq!((ed.cur.line, ed.cur.byte), (5, 6));
    }

    #[test]
    fn a_turn_runs_the_command() {
        // A stand-in claude: echoes one reply in stream-json.
        let reply = r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"text"}}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"pong"}}}
{"type":"result","is_error":false,"session_id":"S","duration_ms":0,"total_cost_usd":0}"#;
        let mut ed = Editor::new(Buffer::from_text("> ping"));
        ed.claude = Some(Box::new(Chat::new(&format!(
            "cat >/dev/null; echo '{reply}'"
        ))));
        ed.set_cursor(0, 3);
        assert!(key(&mut ed, Key::new(KeyCode::Enter)));
        let t = Instant::now();
        while busy(&ed) && t.elapsed().as_secs() < 5 {
            tick(&mut ed);
        }
        assert_eq!(ed.buf.text(), "> ping\n\n⏺ pong\n\n> ");
        assert_eq!(ed.claude.as_ref().unwrap().session.as_deref(), Some("S"));
    }
}
