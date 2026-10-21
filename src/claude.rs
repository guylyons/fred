//! `:claude` / `Space c`: Claude Code in a fred buffer, laid out like its
//! CLI (`> ` prompts, `⏺` replies and tools, `⎿` results). Enter on the
//! prompt sends it, Shift-Enter or Ctrl-J breaks the line, Ctrl-C interrupts. One
//! `claude_command` runs for the chat, each prompt a line on its stdin
//! (`--input-format stream-json`), with the reply streaming in while you edit,
//! here or in another buffer. It stays up between turns, so a command Claude
//! sent to the background still reports back; if it exits, the next prompt
//! `--resume`s the conversation.
//!
//! Around it, a porcelain as Magit is to git, on Claude Code's own SDK
//! control protocol (`control_request` lines on its stdin): `Space c` then
//! a key picks the model or permission mode, runs a slash command, resumes
//! a past conversation, interrupts (the conversation goes on) and so on.

use crate::buffer::Edit;
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Instant;

pub struct Chat {
    cmd: String,
    session: Option<String>,
    job: Option<Job>,
    /// From claude's `initialize`: its slash commands (name, what it does,
    /// argument) and models (name to ask for, name shown, what it is).
    commands: Vec<(String, String, String)>,
    models: Vec<(String, String, String)>,
    /// The model and permission mode in use, as claude last said.
    model: Option<String>,
    mode: Option<String>,
    /// A `Space c` key waiting for claude to start and say what it has.
    waiting: Option<Action>,
    /// Ctrl-C was sent: the turn ending is that.
    interrupted: bool,
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
    /// Where the next prompt goes, for `claude`; other commands get one
    /// prompt and their stdin closed.
    stdin: Option<ChildStdin>,
    /// The turn is over, though the process is still up: between prompts,
    /// or waiting on a background command, which starts a turn of its own.
    idle: bool,
    /// Control requests sent and not answered yet, by id.
    asked: Vec<(String, Req)>,
    next_id: u64,
}

/// What a control request asked, for when its answer comes.
#[derive(Debug, PartialEq)]
enum Req {
    Init,
    Model(String),
    Mode(String),
    Interrupt,
}

/// The `Space c` keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Chat,
    Model,
    Mode,
    Slash,
    Resume,
    New,
    Context,
    Interrupt,
}

/// `Space c` then: the key, what it does.
pub const LEADER: &[(char, Action, &str)] = &[
    ('c', Action::Chat, "chat"),
    ('/', Action::Slash, "slash command…"),
    ('m', Action::Model, "model…"),
    ('p', Action::Mode, "permission mode…"),
    ('r', Action::Resume, "resume a conversation…"),
    ('n', Action::New, "new conversation"),
    ('x', Action::Context, "context usage"),
    ('i', Action::Interrupt, "interrupt"),
];

/// Claude Code's permission modes (`--permission-mode`), as `p` offers them.
const MODES: &[(&str, &str)] = &[
    ("manual", "ask before edits and commands"),
    ("acceptEdits", "edit files without asking"),
    ("plan", "read and plan; change nothing"),
    ("auto", "a classifier decides what to ask about"),
    ("dontAsk", "never ask: anything that would is refused"),
    ("bypassPermissions", "never ask: no restrictions at all"),
];

impl Job {
    /// A line to claude's stdin. False if it can't take it.
    fn write(&mut self, line: &Value) -> bool {
        let Some(stdin) = &mut self.stdin else {
            return false;
        };
        writeln!(stdin, "{line}")
            .and_then(|_| stdin.flush())
            .is_ok()
    }

    /// Hand `prompt` to the running claude. False if it can't take it.
    fn prompt(&mut self, prompt: &str) -> bool {
        let line = json!({ "type": "user", "message": { "role": "user", "content": prompt } });
        if !self.write(&line) {
            return false;
        }
        self.idle = false;
        self.start = Instant::now();
        true
    }

    /// A control request (the Agent SDK's), answered by a
    /// `control_response` with its id.
    fn control(&mut self, req: Req, body: Value) -> bool {
        self.next_id += 1;
        let id = format!("fred-{}", self.next_id);
        let line = json!({ "type": "control_request", "request_id": id, "request": body });
        let ok = self.write(&line);
        if ok {
            self.asked.push((id, req));
        }
        ok
    }

    /// Stop with it, reaping it in the background (claude may linger on its
    /// hooks or background commands).
    fn end(mut self, kill: bool) {
        if kill {
            let _ = self.child.kill();
        }
        drop(self.stdin.take());
        std::thread::spawn(move || self.child.wait());
    }
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
    Model(String),
    Mode(String),
    /// The answer to a control request: its id, whether it worked, and
    /// what came back (or the error).
    Control(String, Result<Value, String>),
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
            commands: vec![],
            models: vec![],
            model: None,
            mode: None,
            waiting: None,
            interrupted: false,
        }
    }

    /// `*claude* · opus · plan`, for the status line.
    pub fn title(&self) -> String {
        let mut t = "*claude*".to_string();
        for part in [&self.model, &self.mode].into_iter().flatten() {
            t += &format!(" · {part}");
        }
        t
    }
}

/// Claude, through `--permission-prompt-tool`, asks to use a tool: the
/// chat shows it, and the answer waits for `y` or `n` (see `answer`).
pub fn ask(ed: &mut Editor, args: &Value, reply: Sender<Result<String, String>>) {
    // All of it, on one line: the dialog shows everything being allowed.
    let name = args["tool_name"].as_str().unwrap_or("?");
    let what = format!("{name}({})", arg(&args["input"]).replace('\n', " ⏎ "));
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
    put(
        ed,
        if yes {
            "\n  ⎿  Allowed"
        } else {
            "\n  ⎿  Denied"
        },
    );
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
        return Some(vec![
            (s.fg(Color::DarkGray), 0..p),
            (bold.fg(c), p..line.len()),
        ]);
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

/// `✻ Claude is working… 12s`, its spinner turning, while a turn runs:
/// every buffer's status bar shows it.
pub fn working(ed: &Editor) -> Option<String> {
    const SPIN: [char; 6] = ['·', '✢', '✳', '✶', '✻', '✽'];
    let job = ed.claude.as_ref()?.job.as_ref().filter(|j| !j.idle)?;
    let t = job.start.elapsed();
    let c = SPIN[(t.as_millis() / 120) as usize % SPIN.len()];
    Some(format!("{c} Claude is working… {}s", t.as_secs()))
}

/// A turn is running.
pub fn busy(ed: &Editor) -> bool {
    ed.claude
        .as_ref()
        .and_then(|c| c.job.as_ref())
        .is_some_and(|j| !j.idle)
}

/// Its claude is running, between turns or not: tool approvals go here.
pub fn live(ed: &Editor) -> bool {
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
    if k == Key::ctrl('c') && chat.job.as_ref().is_some_and(|j| !j.idle) {
        interrupt(ed);
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
    if busy(ed) {
        return ed.set_msg("Claude is working · C-c to interrupt");
    }
    if prompt == "/clear" {
        return new_conversation(ed);
    }
    let end = ed.buf.len_chars();
    ed.apply(Edit {
        start: end,
        end,
        text: "\n\n> ".into(),
    });
    let last = ed.line_count() - 1;
    ed.set_cursor(last, 2);
    dispatch(ed, prompt);
}

/// Send `prompt` from a `Space c` key, as if typed: it shows above the
/// prompt being typed, which stays as it was.
fn submit(ed: &mut Editor, prompt: &str) {
    if busy(ed) {
        return ed.set_msg("Claude is working · C-c to interrupt");
    }
    put(ed, &format!("\n\n> {prompt}"));
    dispatch(ed, prompt.to_string());
}

/// Hand `prompt` to claude, starting it if it isn't running.
fn dispatch(ed: &mut Editor, prompt: String) {
    let chat = ed.claude.as_mut().unwrap();
    // A slash command must come first, as typed.
    let prompt = if prompt.starts_with('/') {
        prompt
    } else {
        with_files(&chat.files, prompt)
    };
    if let Some(mut job) = chat.job.take() {
        if job.prompt(&prompt) {
            chat.job = Some(job);
            ed.msg = working(ed).map(|s| (s, false));
            return;
        }
        job.end(false);
    }
    match spawn(&command(chat), Some(prompt)) {
        Ok(job) => {
            ed.claude.as_mut().unwrap().job = Some(job);
            ed.msg = None;
        }
        Err(e) => put(ed, &format!("\n\n⏺ {e}")),
    }
}

/// The command for a new claude: resuming the conversation, if there is one.
fn command(chat: &Chat) -> String {
    match &chat.session {
        Some(id) => format!("{} --resume {id}", chat.cmd),
        None => chat.cmd.clone(),
    }
}

/// `/clear`, `Space c n`: a new conversation, in a clean buffer.
fn new_conversation(ed: &mut Editor) {
    let chat = ed.claude.as_mut().unwrap();
    chat.session = None;
    if let Some(job) = chat.job.take() {
        job.end(true);
    }
    let end = ed.buf.len_chars();
    ed.apply(Edit {
        start: 0,
        end,
        text: "> ".into(),
    });
    ed.set_cursor(0, 2);
    ed.set_msg("new conversation");
}

/// Ctrl-C, `Space c i`: stop the turn, as Esc in the CLI. The conversation
/// and claude go on; a command that isn't claude is killed.
fn interrupt(ed: &mut Editor) {
    let Some(chat) = ed.claude.as_mut() else {
        return;
    };
    // A question left unanswered: dropping it tells Claude no.
    chat.ask = None;
    let Some(job) = chat.job.as_mut().filter(|j| !j.idle) else {
        return ed.set_msg("Claude isn't working");
    };
    if job.control(Req::Interrupt, json!({ "subtype": "interrupt" })) {
        chat.interrupted = true;
    } else {
        let mut job = chat.job.take().unwrap();
        let _ = job.child.kill();
        let _ = job.child.wait();
    }
    put(ed, "\n  ⎿  Interrupted");
    ed.set_msg("interrupted");
}

/// Start claude without a prompt, for what it says when it starts (its
/// models, commands); `then` runs when it has. False if it can't.
fn start(ed: &mut Editor, then: Action) -> bool {
    let chat = ed.claude.as_mut().unwrap();
    if chat.job.is_none() {
        match spawn(&command(chat), None) {
            Ok(job) => chat.job = Some(job),
            Err(e) => {
                ed.set_err(e);
                return false;
            }
        }
    }
    chat.waiting = Some(then);
    ed.set_msg("starting claude…");
    true
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

/// For the chat's Claude: it works as it would anywhere, beside the user's editor.
const IN_FRED: &str = "You are running inside fred, the user's Vim-like editor, \
which they keep using while you work. Read and change files with your own tools, \
as usual; fred reloads a file you change unless its buffer has unsaved changes. \
fred_state lists the open buffers and fred_read reads one, unsaved changes \
included. Never type into fred.";

/// How long Claude's Bash waits on a command before moving it to the
/// background and carrying on, as the CLI lets you with Ctrl-B. Your own
/// `$CLAUDE_CODE_AUTO_BACKGROUND_TIMEOUT_MS` wins.
const AUTO_BACKGROUND_MS: &str = "30000";

/// Run `cmd`; for claude, with `prompt` (if any) after `initialize`.
/// Another command takes the prompt on its stdin, once.
fn spawn(cmd: &str, prompt: Option<String>) -> Result<Job, String> {
    let mut cmd = cmd.to_string();
    let mut env = None;
    // `claude …`, or wrapped: `env X=1 claude …`, `/opt/bin/claude …`.
    let is_claude = cmd
        .split_whitespace()
        .any(|w| w == "claude" || w.ends_with("/claude"));
    if is_claude {
        cmd += " --input-format stream-json";
    }
    // The chat drives this fred, through its own `fred --mcp`, which also
    // asks you (`-p` can't) before a tool your settings don't allow.
    if let Some(p) = crate::remote::SOCKET.get() {
        env = Some(p);
        if let Some(exe) = std::env::current_exe().ok().filter(|_| is_claude) {
            let servers = json!({ "mcpServers": { "fred": {
                "command": exe, "args": ["--mcp"] } } });
            // It edits with its own tools; fred_keys is off so it never takes your editor.
            cmd += &format!(
                " --mcp-config {} --permission-prompt-tool mcp__fred__fred_approve \
                 --allowedTools mcp__fred__fred_state,mcp__fred__fred_read \
                 --disallowedTools mcp__fred__fred_keys \
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
    const BG: &str = "CLAUDE_CODE_AUTO_BACKGROUND_TIMEOUT_MS";
    if is_claude && std::env::var_os(BG).is_none() {
        command.env(BG, AUTO_BACKGROUND_MS);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("can't run claude: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut parser = Parser::default();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            for o in parser.line(&line) {
                if tx.send(o).is_err() {
                    return;
                }
            }
        }
    });
    let mut job = Job {
        child,
        rx,
        start: Instant::now(),
        stdin: None,
        idle: prompt.is_none(),
        asked: vec![],
        next_id: 0,
    };
    if is_claude {
        job.stdin = Some(stdin);
        let init = job.control(Req::Init, json!({ "subtype": "initialize" }));
        if !init || prompt.is_some_and(|p| !job.prompt(&p)) {
            job.end(true);
            return Err("can't send claude the prompt".into());
        }
    } else {
        let Some(prompt) = prompt else {
            job.end(true);
            return Err("claude_command isn't claude: it has no models or commands".into());
        };
        std::thread::spawn(move || stdin.write_all(prompt.as_bytes()));
    }
    Ok(job)
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
        let job = ed.claude.as_mut().unwrap().job.as_mut().unwrap();
        // A background command finished: Claude takes a turn of its own.
        let quiet = matches!(
            o,
            Out::Session(_) | Out::Done(_) | Out::Model(_) | Out::Mode(_) | Out::Control(..)
        );
        if job.idle && !quiet {
            job.idle = false;
            job.start = Instant::now();
        }
        match o {
            Out::Block => put(ed, "\n\n⏺ "),
            Out::Text(t) => put(ed, &t.replace('\n', "\n  ")),
            Out::Tool(t) => put(ed, &format!("\n\n⏺ {t}")),
            Out::Result(r) => put(ed, &format!("\n  ⎿  {r}")),
            Out::Session(id) => ed.claude.as_mut().unwrap().session = Some(id),
            Out::Model(m) => {
                let m = m.strip_prefix("claude-").unwrap_or(&m).to_string();
                ed.claude.as_mut().unwrap().model = Some(m);
            }
            Out::Mode(m) => ed.claude.as_mut().unwrap().mode = Some(m),
            Out::Control(id, r) => answered(ed, &id, r),
            // The turn is over; claude stays up for the next prompt.
            Out::Done(m) => {
                job.idle = true;
                let chat = ed.claude.as_mut().unwrap();
                let m = match std::mem::take(&mut chat.interrupted) {
                    true => "interrupted".into(),
                    false => m,
                };
                ed.set_msg(m);
            }
        }
    }
    if ended && let Some(job) = ed.claude.as_mut().unwrap().job.take() {
        let idle = job.idle;
        job.end(false);
        if idle {
            ed.buf.modified = false;
            return changed;
        }
        put(
            ed,
            "\n\n⏺ claude exited without answering (see claude_command)",
        );
        ed.msg = None;
    }
    ed.buf.modified = false;
    changed
}

/// `Space c` then a key, from any buffer: the session has shown the chat.
pub fn leader(ed: &mut Editor, k: Key) -> bool {
    if ed.mode != Mode::Normal || ed.zap.is_some() || ed.explain.is_some() {
        return false;
    }
    let space = Key::ch(' ');
    if ed.vim.pending == [space] && k == Key::ch('c') {
        ed.vim.pending.push(k);
        return true;
    }
    if ed.vim.pending != [space, Key::ch('c')] {
        return false;
    }
    ed.vim.pending.clear();
    match LEADER.iter().find(|l| Some(l.0) == k.char()) {
        Some(&(_, a, _)) => ed.pending_effect = Some(crate::ex::ExEffect::Claude(a)),
        None if k.is(KeyCode::Esc) || k == Key::ctrl('g') => {}
        None => ed.set_err(format!("SPC c {} is undefined", crate::org::describe(&[k]))),
    }
    true
}

/// Do a `Space c` key in the chat `ed`.
pub fn act(ed: &mut Editor, a: Action) {
    let Some(chat) = ed.claude.as_ref() else {
        return;
    };
    // These need claude up and what it said when it started.
    let ready = !chat.models.is_empty() && chat.job.is_some();
    match a {
        Action::Chat => {}
        Action::New => new_conversation(ed),
        Action::Interrupt => interrupt(ed),
        Action::Context => submit(ed, "/context"),
        Action::Resume => resume(ed),
        Action::Model | Action::Mode | Action::Slash if !ready => {
            start(ed, a);
        }
        Action::Model => {
            let chat = ed.claude.as_ref().unwrap();
            let w = chat
                .models
                .iter()
                .map(|m| m.1.chars().count())
                .max()
                .unwrap_or(0);
            let rows = chat
                .models
                .iter()
                .map(|m| format!("{:<w$}  {}", m.1, m.2))
                .collect();
            let values: Vec<String> = chat.models.iter().map(|m| m.0.clone()).collect();
            choose(ed, "model", rows, move |ed, i| {
                let v = values[i].clone();
                control(
                    ed,
                    Req::Model(v.clone()),
                    json!({ "subtype": "set_model", "model": v }),
                );
            });
        }
        Action::Mode => {
            let now = ed.claude.as_ref().unwrap().mode.clone().unwrap_or_default();
            let rows = MODES
                .iter()
                .map(|(m, what)| {
                    let mark = if *m == now { "  (now)" } else { "" };
                    format!("{m:<17}  {what}{mark}")
                })
                .collect();
            choose(ed, "permission mode", rows, |ed, i| {
                let m = MODES[i].0.to_string();
                control(
                    ed,
                    Req::Mode(m.clone()),
                    json!({ "subtype": "set_permission_mode", "mode": m }),
                );
            });
        }
        Action::Slash => {
            let chat = ed.claude.as_ref().unwrap();
            let mut cmds = chat.commands.clone();
            cmds.sort_by_key(|c| c.0.to_lowercase());
            let usage =
                |c: &(String, String, String)| format!("/{} {}", c.0, c.2).trim_end().to_string();
            let w = cmds
                .iter()
                .map(|c| usage(c).chars().count())
                .max()
                .unwrap_or(0)
                .min(36);
            let rows = cmds
                .iter()
                .map(|c| format!("{:<w$}  {}", usage(c), clip(first_sentence(&c.1))))
                .collect();
            choose(ed, "slash command", rows, move |ed, i| {
                let (name, _, hint) = &cmds[i];
                if hint.is_empty() {
                    submit(ed, &format!("/{name}"));
                } else {
                    // Its argument is yours to type.
                    let p = prompt_line(ed).unwrap_or(ed.line_count() - 1);
                    let start = ed.buf.line_to_char(p);
                    let end = ed.buf.len_chars();
                    ed.apply(Edit {
                        start,
                        end,
                        text: format!("> /{name} "),
                    });
                    let last = ed.line_count() - 1;
                    ed.set_cursor(last, ed.buf.line_len(last));
                    ed.mode = Mode::Insert;
                }
            });
        }
    }
}

/// Pick one of `rows`; `then` gets its index.
fn choose(
    ed: &mut Editor,
    title: &str,
    rows: Vec<String>,
    then: impl FnOnce(&mut Editor, usize) + Send + 'static,
) {
    if rows.is_empty() {
        return ed.set_err(format!("no {title} to pick"));
    }
    crate::pick::choose(ed, title, rows.clone(), "");
    ed.org_require_match = true;
    ed.org_then = Some(Box::new(move |ed, picked| {
        if let Some(i) = rows.iter().position(|r| *r == picked) {
            then(ed, i);
        }
    }));
}

/// Send a control request to the chat's claude.
fn control(ed: &mut Editor, req: Req, body: Value) {
    let Some(job) = ed.claude.as_mut().and_then(|c| c.job.as_mut()) else {
        return ed.set_err("claude isn't running");
    };
    if !job.control(req, body) {
        ed.set_err("claude isn't listening");
    }
}

fn first_sentence(s: &str) -> &str {
    s.find(". ").map_or(s, |i| &s[..=i])
}

/// Where Claude Code keeps this directory's conversations.
fn sessions_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".claude")))?;
    let cwd = std::env::current_dir().ok()?;
    let name: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Some(base.join("projects").join(name))
}

/// A conversation's title: what Claude Code named it, else its first prompt.
fn session_title(text: &str) -> String {
    let mut title = None;
    let mut first = None;
    for line in text.lines() {
        if line.contains("\"custom-title\"") || line.contains("\"ai-title\"") {
            let v: Value = serde_json::from_str(line).unwrap_or_default();
            if let Some(t) = v["customTitle"].as_str().or(v["aiTitle"].as_str()) {
                title = Some(t.to_string());
            }
        } else if first.is_none()
            && line.contains("\"type\":\"user\"")
            && let Ok(v) = serde_json::from_str::<Value>(line)
            && let Some(t) = v["message"]["content"].as_str().and_then(typed)
        {
            first = Some(t.lines().next().unwrap_or_default().to_string());
        }
    }
    clip(&title.or(first).unwrap_or_else(|| "(empty)".into()))
}

/// What the user typed in a prompt: without the open files fred adds, and
/// None for Claude Code's own (`<command-name>`, `<local-command-stdout>`).
fn typed(prompt: &str) -> Option<&str> {
    let p = match prompt.split_once("</fred-buffers>") {
        Some((_, rest)) => rest.trim_start(),
        None => prompt,
    };
    (!p.starts_with('<') && !p.trim().is_empty()).then_some(p)
}

/// `Space c r`: this directory's conversations, newest first; the one
/// picked is shown and the next prompt goes on with it.
fn resume(ed: &mut Editor) {
    if busy(ed) {
        return ed.set_msg("Claude is working · C-c to interrupt");
    }
    let Some(dir) = sessions_dir() else {
        return ed.set_err("no Claude Code conversations here");
    };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    // ponytail: reads the 40 newest whole; an index if that gets slow.
    files.truncate(40);
    let now = std::time::SystemTime::now();
    let found: Vec<(String, std::path::PathBuf)> = files
        .into_iter()
        .map(|(when, p)| {
            let ago = now.duration_since(when).unwrap_or_default().as_secs();
            let ago = match ago {
                s if s < 3600 => format!("{}m ago", s / 60),
                s if s < 86400 => format!("{}h ago", s / 3600),
                s => format!("{}d ago", s / 86400),
            };
            let title = session_title(&std::fs::read_to_string(&p).unwrap_or_default());
            (format!("{ago:>8}  {title}"), p)
        })
        .collect();
    let rows = found.iter().map(|f| f.0.clone()).collect();
    choose(ed, "conversation", rows, move |ed, i| {
        let path = &found[i].1;
        let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            return;
        };
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let chat = ed.claude.as_mut().unwrap();
        if let Some(job) = chat.job.take() {
            job.end(true);
        }
        chat.session = Some(id);
        let shown = history(&text) + "\n\n> ";
        let end = ed.buf.len_chars();
        ed.apply(Edit {
            start: 0,
            end,
            text: shown.trim_start().into(),
        });
        let last = ed.line_count() - 1;
        ed.set_cursor(last, 2);
        ed.set_msg(format!("resumed: {}", session_title(&text)));
    });
}

/// A saved conversation, laid out as the chat shows one.
fn history(jsonl: &str) -> String {
    let mut out = String::new();
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["isMeta"] == true || v["isSidechain"] == true {
            continue;
        }
        let blocks = v["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        match v["type"].as_str() {
            Some("user") => {
                if let Some(t) = v["message"]["content"].as_str().and_then(typed) {
                    out += &format!("\n\n> {t}");
                }
                for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                    out += &format!("\n  ⎿  {}", result(&b["content"], b["is_error"] == true));
                }
            }
            Some("assistant") => {
                for b in &blocks {
                    match b["type"].as_str() {
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or_default();
                            out += &format!("\n\n⏺ {}", t.replace('\n', "\n  "));
                        }
                        Some("tool_use") => {
                            let name = b["name"].as_str().unwrap_or_default();
                            out += &format!("\n\n⏺ {}", tool(name, &b["input"]));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Claude answered control request `id`.
fn answered(ed: &mut Editor, id: &str, r: Result<Value, String>) {
    let chat = ed.claude.as_mut().unwrap();
    let Some(job) = chat.job.as_mut() else { return };
    let Some(i) = job.asked.iter().position(|a| a.0 == id) else {
        return;
    };
    let (_, req) = job.asked.remove(i);
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    match (req, r) {
        (Req::Init, Ok(v)) => {
            chat.commands = v["commands"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| (s(&c["name"]), s(&c["description"]), s(&c["argumentHint"])))
                .collect();
            chat.models = v["models"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| (s(&m["value"]), s(&m["displayName"]), s(&m["description"])))
                .collect();
            if let Some(mode) = v["current_permission_mode"].as_str() {
                chat.mode = Some(mode.into());
            }
            if let Some(a) = chat.waiting.take() {
                ed.msg = None;
                act(ed, a);
            }
        }
        (Req::Model(m), Ok(_)) => {
            ed.set_msg(format!("model: {m}"));
            ed.claude.as_mut().unwrap().model = Some(m);
        }
        (Req::Mode(m), Ok(_)) => {
            ed.set_msg(format!("permission mode: {m}"));
            ed.claude.as_mut().unwrap().mode = Some(m);
        }
        (Req::Interrupt, _) => {}
        (_, Err(e)) => {
            chat.waiting = None;
            ed.set_err(format!("claude: {e}"));
        }
    }
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
    // Nothing above the prompt yet: the text goes first, then the blank line.
    let (at, text) = match p {
        0 => (0, format!("{}\n\n", text.trim_start_matches('\n'))),
        _ => (ed.buf.line_to_char(p).saturating_sub(2), text.to_string()),
    };
    let text = text.as_str();
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

/// Reads claude's `--output-format stream-json`, a line at a time.
#[derive(Default)]
pub struct Parser {
    /// Text came as it was written (`--include-partial-messages`): the
    /// whole message after it isn't shown again.
    streamed: bool,
}

/// A line, on its own (tests).
pub fn parse(line: &str) -> Vec<Out> {
    Parser::default().line(line)
}

impl Parser {
    pub fn line(&mut self, line: &str) -> Vec<Out> {
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
            Some("system") if v["subtype"] == "init" => {
                let mut out = vec![Out::Session(s(&v["session_id"]))];
                if let Some(m) = v["model"].as_str() {
                    out.push(Out::Model(m.into()));
                }
                if let Some(m) = v["permissionMode"].as_str() {
                    out.push(Out::Mode(m.into()));
                }
                out
            }
            Some("system") if v["permissionMode"].is_string() => {
                vec![Out::Mode(s(&v["permissionMode"]))]
            }
            Some("control_response") => {
                let r = &v["response"];
                let id = s(&r["request_id"]);
                let answer = match r["subtype"].as_str() {
                    Some("success") => Ok(r["response"].clone()),
                    _ => Err(r["error"].as_str().unwrap_or("failed").to_string()),
                };
                vec![Out::Control(id, answer)]
            }
            Some("stream_event") => {
                let e = &v["event"];
                match e["type"].as_str() {
                    Some("content_block_start") if e["content_block"]["type"] == "text" => {
                        vec![Out::Block]
                    }
                    Some("content_block_delta") if e["delta"]["type"] == "text_delta" => {
                        self.streamed = true;
                        vec![Out::Text(s(&e["delta"]["text"]))]
                    }
                    _ => vec![],
                }
            }
            Some("assistant") => {
                // Text not streamed: a slash command's output (`/context`), or
                // a claude_command without --include-partial-messages.
                let streamed = std::mem::take(&mut self.streamed);
                let mut out = vec![];
                for b in content() {
                    match b["type"].as_str() {
                        Some("text") if !streamed && !s(&b["text"]).is_empty() => {
                            out.push(Out::Block);
                            out.push(Out::Text(s(&b["text"])));
                        }
                        Some("tool_use") => out.push(Out::Tool(tool(&s(&b["name"]), &b["input"]))),
                        _ => {}
                    }
                }
                out
            }
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
}

/// `Bash(cargo test)`, as the CLI shows a tool call.
fn tool(name: &str, input: &Value) -> String {
    format!(
        "{name}({})",
        clip(arg(input).lines().next().unwrap_or_default())
    )
}

/// What a call works on: its command, path, pattern…, else its whole input,
/// so an approval never hides what it allows.
fn arg(input: &Value) -> String {
    let keys = [
        "file_path",
        "command",
        "pattern",
        "path",
        "url",
        "query",
        "description",
    ];
    match keys.iter().find_map(|k| input[k].as_str()) {
        Some(a) => a.into(),
        None if input.as_object().is_none_or(|o| o.is_empty()) => String::new(),
        None => input.to_string(),
    }
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
        assert_eq!(
            v,
            json!({ "behavior": "allow", "updatedInput": { "command": "ls" } })
        );
        assert_eq!(
            ed.buf.text(),
            "> go\n\n⏺ Allow Bash(ls)? y/n\n  ⎿  Allowed\n\n> "
        );
        assert!(question(&ed).is_none() && !answer(&mut ed, Key::ch('n')));
    }

    #[test]
    fn an_approval_shows_all_it_allows() {
        let mut ed = Editor::new(Buffer::from_text("> go\n\n> "));
        ed.claude = Some(Box::new(Chat::new("true")));
        let a = "a".repeat(90);
        let cmd = json!({ "tool_name": "Bash", "input": { "command": format!("ls\n{a} | sh") } });
        ask(&mut ed, &cmd, mpsc::channel().0);
        assert_eq!(
            question(&ed).unwrap(),
            format!("Allow Bash(ls ⏎ {a} | sh)? y/n")
        );
        let other = json!({ "tool_name": "mcp__db__run", "input": { "sql": "DROP t" } });
        ask(&mut ed, &other, mpsc::channel().0);
        assert_eq!(
            question(&ed).unwrap(),
            r#"Allow mcp__db__run({"sql":"DROP t"})? y/n"#
        );
    }

    #[test]
    fn questions_and_answers_have_colors() {
        use ratatui::style::Color;
        let fg = |l: &str| -> Vec<(Option<Color>, String)> {
            let st = styles(l).unwrap();
            st.into_iter()
                .map(|(s, r)| (s.fg, l[r].to_string()))
                .collect()
        };
        let q = fg("⏺ Allow Bash(ls)? y/n");
        assert_eq!(q[2], (Some(Color::Cyan), "Bash(ls)".into()));
        assert_eq!(q[4], (Some(Color::Green), "y".into()));
        assert_eq!(q[6], (Some(Color::Red), "n".into()));
        assert_eq!(fg("Allow Read(x)? y/n")[0].1, "");
        assert_eq!(
            fg("  ⎿  Allowed")[1],
            (Some(Color::Green), "Allowed".into())
        );
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

    /// A stand-in `claude` answering the control protocol as Claude Code
    /// 2.1 does: `initialize`, the `set_*` requests, a turn per prompt
    /// (a slash command's synthetic reply).
    fn fake_claude(dir: &std::path::Path) -> String {
        let path = dir.join("claude");
        let script = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"subtype":"initialize"'*) echo '{"type":"control_response","response":{"subtype":"success","request_id":"'$id'","response":{"commands":[{"name":"model","description":"Set the model","argumentHint":"[model]"},{"name":"context","description":"Show context. And more.","argumentHint":""}],"models":[{"value":"sonnet","displayName":"Sonnet","description":"fast"}],"current_permission_mode":"auto"}}}' ;;
    *'"subtype":"set_'*|*'"subtype":"interrupt"'*) echo '{"type":"control_response","response":{"subtype":"success","request_id":"'$id'","response":{}}}' ;;
    *'"type":"user"'*) echo '{"type":"system","subtype":"init","session_id":"S","model":"claude-opus-5-5","permissionMode":"auto"}'
      echo '{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Context usage"}]}}'
      echo '{"type":"result","is_error":false,"session_id":"S","duration_ms":0,"total_cost_usd":0}' ;;
  esac
done
"#;
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        format!("{} -p", path.display())
    }

    /// Tick until `done`, for up to 5 s.
    fn until(ed: &mut Editor, done: impl Fn(&Editor) -> bool) {
        let t = Instant::now();
        while !done(ed) && t.elapsed().as_secs() < 5 {
            tick(ed);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn space_c_keys_drive_claude_through_its_control_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let mut ed = Editor::new(Buffer::from_text("> "));
        ed.claude = Some(Box::new(Chat::new(&fake_claude(dir.path()))));
        // The model: claude starts, says what it has, and the picker shows it.
        act(&mut ed, Action::Model);
        until(&mut ed, |e| matches!(e.mode, Mode::Pick(_)));
        ed.handle_key(Key::new(KeyCode::Enter));
        until(&mut ed, |e| e.claude.as_ref().unwrap().model.is_some());
        assert_eq!(
            ed.claude.as_ref().unwrap().title(),
            "*claude* · sonnet · auto"
        );
        // A slash command without an argument runs, and its output shows.
        act(&mut ed, Action::Slash);
        let Mode::Pick(p) = &ed.mode else { panic!() };
        assert!(
            p.rows[0].text.starts_with("/context  ") && p.rows[0].text.ends_with("Show context.")
        );
        assert!(p.rows[1].text.starts_with("/model [model]  Set the model"));
        ed.handle_key(Key::new(KeyCode::Enter));
        until(&mut ed, |e| {
            e.buf.text().contains("Context usage") && !busy(e)
        });
        assert_eq!(ed.buf.text(), "> /context\n\n⏺ Context usage\n\n> ");
        // The permission mode, picked by searching for it.
        act(&mut ed, Action::Mode);
        for k in crate::key::parse_keys("plan<Enter>") {
            ed.handle_key(k);
        }
        until(&mut ed, |e| {
            e.claude.as_ref().unwrap().mode.as_deref() == Some("plan")
        });
        assert_eq!(ed.msg.as_ref().unwrap().0, "permission mode: plan");
        // One with an argument goes on the prompt, for you to finish.
        act(&mut ed, Action::Slash);
        for k in crate::key::parse_keys("model<Enter>") {
            ed.handle_key(k);
        }
        assert!(ed.buf.text().ends_with("> /model "), "{}", ed.buf.text());
        assert_eq!(ed.mode, Mode::Insert);
    }

    #[test]
    fn space_c_is_a_prefix() {
        let mut ed = Editor::new(Buffer::from_text("x"));
        for k in crate::key::parse_keys(" cm") {
            ed.handle_key(k);
        }
        assert_eq!(
            ed.pending_effect,
            Some(crate::ex::ExEffect::Claude(Action::Model))
        );
        assert!(ed.vim.pending.is_empty());
    }

    #[test]
    fn a_saved_conversation_reads_back_as_the_chat() {
        let jsonl = [
            r#"{"type":"user","message":{"content":"<fred-buffers>\na.rs:1\n</fred-buffers>\n\nfix it"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"caveat"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/context</command-name>"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"On it.\nNow."},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"a\nb"}]}}"#,
            r#"{"type":"ai-title","aiTitle":"Fixing it"}"#,
        ]
        .join("\n");
        assert_eq!(
            history(&jsonl),
            "\n\n> fix it\n\n⏺ On it.\n  Now.\n\n⏺ Bash(ls)\n  ⎿  a … +1 lines"
        );
        assert_eq!(session_title(&jsonl), "Fixing it");
        assert_eq!(
            session_title(&jsonl[..jsonl.find("\n{\"type\":\"ai-title").unwrap()]),
            "fix it"
        );
    }

    #[test]
    fn only_unstreamed_text_is_shown_whole() {
        let mut p = Parser::default();
        let whole = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;
        assert_eq!(p.line(whole), [Out::Block, Out::Text("hi".into())]);
        p.line(r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#);
        assert!(p.line(whole).is_empty());
        assert_eq!(
            p.line(r#"{"type":"control_response","response":{"subtype":"error","request_id":"x","error":"no"}}"#),
            [Out::Control("x".into(), Err("no".into()))]
        );
    }
}
