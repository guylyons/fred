//! `:claude` / `Space c`: Claude Code in a fred buffer, laid out like its
//! CLI (`> ` prompts, `⏺` replies and tools, `⎿` results). Enter on the
//! prompt sends it, Shift-Enter or Ctrl-J breaks the line, Ctrl-C interrupts. Each prompt
//! runs `claude_command` (`--resume`d, so it is one conversation) with the
//! reply streaming in while you edit, here or in another buffer.

use crate::buffer::Edit;
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Instant;

pub struct Chat {
    cmd: String,
    session: Option<String>,
    job: Option<Job>,
    /// The files open in fred, `path:line` a line each, the one used last
    /// first; the session keeps it current, and each prompt carries it.
    pub files: String,
    /// Claude asking to use a tool (`fred_approve`): what, its input, and
    /// where y or n goes.
    ask: Option<(String, Value, crate::remote::Reply)>,
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
            files: String::new(),
            ask: None,
        }
    }
}

/// Claude, through `--permission-prompt-tool`, asks to use a tool: the
/// chat shows it, and the answer waits for `y` or `n` (see `answer`).
pub fn ask(ed: &mut Editor, args: &Value, reply: Sender<Result<String, String>>) {
    let what = tool(args["tool_name"].as_str().unwrap_or("?"), &args["input"]);
    put(ed, &format!("\n\n⏺ Allow {what}? y/n"));
    ed.buf.modified = false;
    ed.claude.as_mut().unwrap().ask = Some((what, args["input"].clone(), reply));
}

/// The question waiting for `y` or `n`, if any.
pub fn question(ed: &Editor) -> Option<String> {
    let (what, ..) = ed.claude.as_ref()?.ask.as_ref()?;
    Some(format!("Allow {what}? y/n"))
}

/// `y` or `n` to the question: the reply Claude Code's permission prompt
/// tool expects. True if `k` answered it.
pub fn answer(ed: &mut Editor, k: Key) -> bool {
    let yes = k == Key::ch('y');
    if !(yes || k == Key::ch('n')) {
        return false;
    }
    let Some((_, input, reply)) = ed.claude.as_mut().and_then(|c| c.ask.take()) else {
        return false;
    };
    let v = match yes {
        true => json!({ "behavior": "allow", "updatedInput": input }),
        false => json!({ "behavior": "deny", "message": "The user said no, in fred." }),
    };
    let _ = reply.send(Ok(v.to_string()));
    put(ed, if yes { "\n  ⎿  Allowed" } else { "\n  ⎿  Denied" });
    ed.buf.modified = false;
    true
}

/// Colors for a question (`⏺ Allow Bash(ls)? y/n`, in the chat or on the
/// message line) and its answer; None for other lines, which are Markdown.
pub fn styles(line: &str) -> Option<crate::highlight::LineStyles> {
    use ratatui::style::{Color, Modifier, Style};
    let s = Style::default();
    let bold = s.add_modifier(Modifier::BOLD);
    if let Some(rest) = line.strip_prefix("  ⎿  ") {
        let c = match rest {
            "Allowed" => Color::Green,
            "Denied" | "Interrupted" => Color::Red,
            _ => return None,
        };
        let p = line.len() - rest.len();
        return Some(vec![(s.fg(Color::DarkGray), 0..p), (bold.fg(c), p..line.len())]);
    }
    let at = line.find("Allow ")?;
    if !matches!(&line[..at], "" | "⏺ ") || !line.ends_with("? y/n") {
        return None;
    }
    let (what, q) = (at + "Allow ".len(), line.len() - "? y/n".len());
    let end = line.len();
    Some(vec![
        (s.fg(Color::Yellow), 0..at),
        (bold.fg(Color::Yellow), at..what),
        (bold.fg(Color::Cyan), what..q),
        (bold.fg(Color::Yellow), q..q + 2),
        (bold.fg(Color::Green), end - 3..end - 2),
        (s.fg(Color::DarkGray), end - 2..end - 1),
        (bold.fg(Color::Red), end - 1..end),
    ])
}

/// The question on the message line as (tool, argument): `Allow Bash(ls)? y/n`
/// is `("Bash", "ls")`. The approval box shows it.
pub fn asked(msg: &str) -> Option<(&str, &str)> {
    let call = msg.strip_prefix("Allow ")?.strip_suffix("? y/n")?;
    let (name, arg) = call.split_once('(')?;
    Some((name, arg.strip_suffix(')')?))
}

/// The CLI's spinner on the status line, while Claude works.
pub fn status(ed: &Editor) -> Option<String> {
    if let Some(q) = question(ed) {
        return Some(q);
    }
    working(ed).map(|w| format!("{w} · C-c to interrupt"))
}

/// `✻ Claude is working… 12s`, its spinner turning, while a turn runs:
/// every buffer's status bar shows it.
pub fn working(ed: &Editor) -> Option<String> {
    const SPIN: [char; 6] = ['·', '✢', '✳', '✶', '✻', '✽'];
    let t = ed.claude.as_ref()?.job.as_ref()?.start.elapsed();
    let c = SPIN[(t.as_millis() / 120) as usize % SPIN.len()];
    Some(format!("{c} Claude is working… {}s", t.as_secs()))
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
        // A question left unanswered: dropping it tells Claude no.
        chat.ask = None;
        let mut job = chat.job.take().unwrap();
        let _ = job.child.kill();
        let _ = job.child.wait();
        put(ed, "\n  ⎿  Interrupted");
        ed.set_msg("interrupted");
        return true;
    }
    let shift_enter = k.code == KeyCode::Enter && k.shift;
    if insert && (k == Key::ctrl('j') || shift_enter) {
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
    let prompt = with_files(&ed.claude.as_ref().unwrap().files, prompt);
    match spawn(&cmd, prompt) {
        Ok(job) => {
            ed.claude.as_mut().unwrap().job = Some(job);
            ed.msg = status(ed).map(|s| (s, false));
        }
        Err(e) => put(ed, &format!("\n\n⏺ {e}")),
    }
}

/// The prompt as sent: what is open in fred first, so "this file" means
/// something. The chat shows only what was typed.
fn with_files(files: &str, prompt: String) -> String {
    if files.is_empty() {
        return prompt;
    }
    format!(
        "<fred-buffers>\nFiles open in the fred editor, as path:cursor-line, \
         the one used last first (\"unsaved\": the buffer differs from disk):\n\
         {files}</fred-buffers>\n\n{prompt}"
    )
}

/// For the chat's Claude: fred is its editor, and the user watches it work.
const IN_FRED: &str = "You are running inside fred, the user's Vim-like editor, \
and they watch you work in it. fred is your only way to read and change files: \
fred_read with a path opens a file in fred and reads it; fred_keys types Vim keys \
into fred, one by one where the user sees them (`:e path<CR>` to open or create a \
file, `/text<CR>` or `42G` to move, `ciw`, `dd`, `o`, `O`, `A`, `:s/a/b/<CR>`, `u`). \
Start each fred_keys call with <Esc>, keep each one to one small edit, and end it \
in Normal mode. Text typed in Insert mode goes in exactly as sent, with no \
autoindent, so include the indentation. After editing, fred_read the lines to \
check them. Save with `:w<CR>` before running anything that reads the file, such \
as a build or tests. Use Bash, Grep and Glob to search and run commands, never to \
change files.";

fn spawn(cmd: &str, prompt: String) -> Result<Job, String> {
    let mut cmd = cmd.to_string();
    let mut env = None;
    // The chat drives this fred, through its own `fred --mcp`, which also
    // asks you (`-p` can't) before a tool your settings don't allow.
    if let Some(p) = crate::remote::SOCKET.get() {
        env = Some(p);
        let is_claude = cmd.split_whitespace().next().is_some_and(|w| w.ends_with("claude"));
        if let Some(exe) = std::env::current_exe().ok().filter(|_| is_claude) {
            let servers = json!({ "mcpServers": { "fred": {
                "command": exe, "args": ["--mcp"] } } });
            // Its own file tools are off: it reads and edits in fred, in front of you.
            cmd += &format!(
                " --mcp-config {} --permission-prompt-tool mcp__fred__fred_approve \
                 --allowedTools mcp__fred__fred_state,mcp__fred__fred_read,mcp__fred__fred_keys \
                 --disallowedTools Read,Edit,Write,MultiEdit,NotebookEdit \
                 --append-system-prompt {}",
                crate::shell::quote(&servers.to_string()),
                crate::shell::quote(IN_FRED)
            );
        }
    }
    let mut command = crate::shell::command(&cmd);
    if let Some(p) = env {
        command.env("FRED_SOCKET", p);
    }
    let mut child = command
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

    #[test]
    fn asks_before_a_tool_and_answers_claude() {
        let mut ed = Editor::new(Buffer::from_text("> go\n\n> "));
        ed.claude = Some(Box::new(Chat::new("true")));
        let (tx, rx) = mpsc::channel();
        let args = json!({ "tool_name": "Bash", "input": { "command": "ls" } });
        ask(&mut ed, &args, tx);
        assert_eq!(question(&ed).as_deref(), Some("Allow Bash(ls)? y/n"));
        assert!(!answer(&mut ed, Key::ch('x')));
        assert!(answer(&mut ed, Key::ch('y')));
        let v: Value = serde_json::from_str(&rx.recv().unwrap().unwrap()).unwrap();
        assert_eq!(v, json!({ "behavior": "allow", "updatedInput": { "command": "ls" } }));
        assert_eq!(ed.buf.text(), "> go\n\n⏺ Allow Bash(ls)? y/n\n  ⎿  Allowed\n\n> ");
        assert!(question(&ed).is_none() && !answer(&mut ed, Key::ch('n')));
    }

    #[test]
    fn questions_and_answers_have_colors() {
        use ratatui::style::Color;
        let fg = |l: &str| -> Vec<(Option<Color>, String)> {
            let st = styles(l).unwrap();
            st.into_iter().map(|(s, r)| (s.fg, l[r].to_string())).collect()
        };
        let q = fg("⏺ Allow Bash(ls)? y/n");
        assert_eq!(q[2], (Some(Color::Cyan), "Bash(ls)".into()));
        assert_eq!(q[4], (Some(Color::Green), "y".into()));
        assert_eq!(q[6], (Some(Color::Red), "n".into()));
        assert_eq!(fg("Allow Read(x)? y/n")[0].1, "");
        assert_eq!(fg("  ⎿  Allowed")[1], (Some(Color::Green), "Allowed".into()));
        assert_eq!(fg("  ⎿  Denied")[1], (Some(Color::Red), "Denied".into()));
        assert_eq!(asked("Allow Bash(ls -l)? y/n"), Some(("Bash", "ls -l")));
        assert_eq!(asked("⏺ Allow Bash(ls)? y/n"), None);
        assert!(styles("  ⎿  ok").is_none() && styles("say Allow x? y/n").is_none());
    }

    #[test]
    fn shift_enter_breaks_the_prompt() {
        let mut ed = Editor::new(Buffer::from_text("> hi"));
        ed.claude = Some(Box::new(Chat::new("true")));
        ed.mode = Mode::Insert;
        ed.set_cursor(0, 4);
        assert!(key(&mut ed, Key::with(KeyCode::Enter, "S")));
        assert_eq!(ed.buf.text(), "> hi\n");
        assert!(!busy(&ed));
    }

    #[test]
    fn prompts_carry_the_open_files() {
        assert_eq!(with_files("", "hi".into()), "hi");
        let p = with_files("/x/a.rs:3\n", "fix this".into());
        assert!(p.starts_with("<fred-buffers>\n"), "{p}");
        assert!(p.ends_with("/x/a.rs:3\n</fred-buffers>\n\nfix this"), "{p}");
    }
}
