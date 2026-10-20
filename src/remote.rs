//! Claude drives a running fred in front of you. Each fred listens on a
//! socket only you can reach; `fred --mcp` is the MCP server Claude Code
//! runs, passing its tool calls to that fred: `$FRED_SOCKET` (set for the
//! `:claude` chat, which so drives its own fred), else the newest one.

use serde_json::{Value, json};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

/// A tool call for the event loop: name, arguments, where the answer goes.
pub type Request = (String, Value, Reply);
pub type Reply = Sender<Result<String, String>>;

/// This fred's socket, for the `:claude` chat's `$FRED_SOCKET`.
pub static SOCKET: OnceLock<PathBuf> = OnceLock::new();

/// How long `fred --mcp` waits for an answer (but not for an approval).
const WAIT: Duration = Duration::from_secs(60);
/// A call that waited this long is refused: fred_keys takes a few seconds to
/// type, and must answer within `WAIT`, or Claude retries keys that landed.
const LATE: Duration = Duration::from_secs(50);

pub struct Remote {
    rx: Receiver<(Request, Instant)>,
    path: PathBuf,
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// The sockets' directory, made if missing; refused unless it is a real
/// directory of ours nobody else can enter: a socket there types keys, and
/// keys run shell commands.
fn private_dir() -> io::Result<PathBuf> {
    let d = std::env::temp_dir().join(format!("fred-{}", uid()));
    let _ = fs::DirBuilder::new().mode(0o700).create(&d);
    let m = fs::symlink_metadata(&d)?;
    if !m.is_dir() || m.uid() != uid() || m.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::other(format!("{} is not private", d.display())));
    }
    Ok(d)
}

impl Remote {
    pub fn listen() -> io::Result<Remote> {
        let path = private_dir()?.join(format!("{}.sock", std::process::id()));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || answer(conn, tx));
            }
        });
        let _ = SOCKET.set(path.clone());
        Ok(Remote { rx, path })
    }
}

impl Remote {
    /// The tool calls waiting. One that waited past `LATE` (fred was stuck in
    /// a prompt) is refused: Claude is about to be told it failed, and may
    /// try again, so its keys must not land late.
    pub fn take(&self) -> impl Iterator<Item = Request> + '_ {
        self.rx.try_iter().filter_map(|((tool, args, reply), at)| {
            if tool != "fred_approve" && at.elapsed() >= LATE {
                let _ = reply.send(Err("fred was busy; nothing was done".into()));
                return None;
            }
            Some((tool, args, reply))
        })
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// One request a connection: a line `{"tool": .., "args": {..}}` in, a line
/// `{"ok": text}` or `{"err": text}` out.
fn answer(conn: UnixStream, tx: Sender<(Request, Instant)>) {
    let mut line = String::new();
    // Nothing sent: `find` checking that this fred is alive.
    if BufReader::new(&conn).read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let req: Value = serde_json::from_str(&line).unwrap_or_default();
    let tool = req["tool"].as_str().unwrap_or_default().to_string();
    let (rtx, rrx) = mpsc::channel();
    if tx.send(((tool, req["args"].clone(), rtx), Instant::now())).is_err() {
        return;
    }
    let reply = match rrx.recv() {
        Ok(Ok(t)) => json!({ "ok": t }),
        Ok(Err(e)) => json!({ "err": e }),
        Err(_) => return,
    };
    let _ = writeln!(&conn, "{reply}");
}

/// `$FRED_SOCKET`, else the newest fred still listening.
fn find() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("FRED_SOCKET") {
        return Ok(p.into());
    }
    let dir = private_dir().map_err(|e| e.to_string())?;
    let mut socks: Vec<(SystemTime, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    socks.sort();
    socks
        .into_iter()
        .rev()
        .map(|(_, p)| p)
        // A socket nobody answers is a fred that died without cleaning up.
        .find(|p| {
            let alive = UnixStream::connect(p).is_ok();
            if !alive {
                let _ = fs::remove_file(p);
            }
            alive
        })
        .ok_or_else(|| "fred isn't running: start fred, then try again".into())
}

fn call(path: &Path, tool: &str, args: &Value) -> Result<String, String> {
    let conn = UnixStream::connect(path).map_err(|e| format!("can't reach fred: {e}"))?;
    // fred answers between frames; a minute means it is stuck in a prompt.
    // An approval waits on you, however long you take.
    let wait = (tool != "fred_approve").then_some(WAIT);
    let _ = conn.set_read_timeout(wait);
    writeln!(&conn, "{}", json!({ "tool": tool, "args": args }))
        .map_err(|e| format!("can't reach fred: {e}"))?;
    let mut line = String::new();
    let _ = BufReader::new(&conn).read_line(&mut line);
    let v: Value = serde_json::from_str(&line)
        .map_err(|_| "fred didn't answer (it quit, or is waiting on a prompt)".to_string())?;
    match v["ok"].as_str() {
        Some(t) => Ok(t.into()),
        None => Err(v["err"].as_str().unwrap_or("fred gave no answer").into()),
    }
}

const INSTRUCTIONS: &str = "fred is the user's Vim-like editor, open in front of \
them. fred_state shows what they have open, fred_read the text of a buffer \
(unsaved changes included), and fred_keys works in fred as they would, so \
they watch every step. Save (`:w`) before a build or test needs the file; \
otherwise leave edits unsaved for the user to review, unless they asked.";

fn tools() -> Value {
    json!([
        {
            "name": "fred_state",
            "title": "What fred shows",
            "description": "The mode, the current buffer (number, path, cursor line and column, line count, unsaved), the message line, and every open buffer, most recently used first. Buffer numbers work with fred_read and with `:b N<CR>` in fred_keys.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "fred_read",
            "title": "Read a buffer",
            "description": "Lines of a buffer as fred has them, unsaved changes included, each as `N<TAB>text`. With `path`, opens that file in fred first (the user sees it, at `start`) and reads it. At most 500 lines a call; the header says how many there are.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "A file to open in fred and read (relative to where fred runs, or absolute)" },
                    "buffer": { "type": "integer", "minimum": 1, "description": "Buffer number from fred_state (default: the current one, or the one `path` opens)" },
                    "start": { "type": "integer", "minimum": 1, "description": "First line (default 1)" },
                    "end": { "type": "integer", "minimum": 1, "description": "Last line (default: start + 499)" }
                }
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "fred_keys",
            "title": "Type into fred",
            "description": "Type keys into fred as the user would; they watch them typed one by one (a few seconds at most), and their own keys wait. Text typed in Insert mode goes in exactly as sent: no autoindent, no completion. Vim notation: `<Esc>` `<CR>` `<BS>` `<Tab>` `<C-w>` `<M-x>` `<Up>`, and `<lt>` for `<`. fred is Vim-like: `:e src/main.rs<CR>`, `:b 2<CR>`, `42G`, `/foo<CR>`, `ciwbar<Esc>`, `:%s/a/b/g<CR>`, `u` to undo, `:w<CR>` to save. End in Normal mode (`<Esc>`) unless leaving the user mid-edit. Shell commands typed here (`:!`, `:r !`, filters) are refused: run them with your own tool. Returns what fred_state would after, and the lines around the cursor.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": { "type": "string", "description": "Keys in Vim notation" }
                },
                "required": ["keys"]
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "fred_approve",
            "title": "Ask the user in fred",
            "description": "Claude Code's --permission-prompt-tool for fred's :claude chat: asks the user, in fred, to allow a tool call (y/n). Not for calling directly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool_name": { "type": "string" },
                    "input": { "type": "object" },
                    "tool_use_id": { "type": "string" }
                },
                "required": ["tool_name", "input"]
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

/// A JSON-RPC message from Claude Code; `None` for notifications.
fn handle(msg: &Value, call: impl Fn(&str, &Value) -> Result<String, String>) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let mut reply = match msg["method"].as_str().unwrap_or_default() {
        "initialize" => json!({ "result": {
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "fred", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS
        }}),
        "ping" => json!({ "result": {} }),
        "tools/list" => json!({ "result": { "tools": tools() } }),
        "tools/call" => {
            let p = &msg["params"];
            let (text, err) = match call(p["name"].as_str().unwrap_or_default(), &p["arguments"]) {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            json!({ "result": { "content": [{ "type": "text", "text": text }], "isError": err } })
        }
        m => json!({ "error": { "code": -32601, "message": format!("unknown method: {m}") } }),
    };
    reply["jsonrpc"] = "2.0".into();
    reply["id"] = id;
    Some(reply)
}

/// `fred --mcp`: the MCP server, on stdin and stdout.
pub fn mcp() -> i32 {
    let mut out = io::stdout();
    for line in io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let reply = handle(&msg, |tool, args| call(&find()?, tool, args));
        if let Some(r) = reply
            && writeln!(out, "{r}").and_then(|_| out.flush()).is_err()
        {
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaks_mcp() {
        let echo = |tool: &str, args: &Value| -> Result<String, String> {
            match tool {
                "fred_keys" => Ok(format!("typed {}", args["keys"])),
                _ => Err("no".into()),
            }
        };
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}});
        let r = handle(&init, echo).unwrap();
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["serverInfo"]["name"], "fred");
        assert!(handle(&json!({"method": "notifications/initialized"}), echo).is_none());
        let list = handle(&json!({"id": 2, "method": "tools/list"}), echo).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 4);
        let call = json!({"id": 3, "method": "tools/call",
            "params": {"name": "fred_keys", "arguments": {"keys": "jj"}}});
        let r = handle(&call, echo).unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "typed \"jj\"");
        assert_eq!(r["result"]["isError"], false);
        let bad = json!({"id": 4, "method": "tools/call", "params": {"name": "x"}});
        assert_eq!(handle(&bad, echo).unwrap()["result"]["isError"], true);
        assert_eq!(handle(&json!({"id": 5, "method": "nope"}), echo).unwrap()["error"]["code"], -32601);
    }

    #[test]
    fn a_call_that_waited_too_long_is_refused() {
        let (tx, rx) = mpsc::channel();
        let r = Remote { rx, path: PathBuf::from("/nonexistent/fred.sock") };
        let old = Instant::now() - WAIT;
        let (keys, late) = mpsc::channel();
        let (ok, approve) = mpsc::channel();
        tx.send((("fred_keys".into(), json!({}), keys), old)).unwrap();
        tx.send((("fred_approve".into(), json!({}), ok), old)).unwrap();
        let got: Vec<String> = r.take().map(|(tool, ..)| tool).collect();
        assert_eq!(got, ["fred_approve"]);
        assert!(late.recv().unwrap().is_err());
        assert!(approve.try_recv().is_err());
    }

    #[test]
    fn a_call_goes_through_the_socket() {
        let r = Remote::listen().unwrap();
        let path = r.path.clone();
        let fred = std::thread::spawn(move || {
            let ((tool, args, reply), _) = r.rx.recv().unwrap();
            reply.send(Ok(format!("{tool} {}", args["keys"]))).unwrap();
            let ((_, _, reply), _) = r.rx.recv().unwrap();
            reply.send(Err("bad".into())).unwrap();
        });
        assert_eq!(call(&path, "fred_keys", &json!({"keys": "x"})).unwrap(), "fred_keys \"x\"");
        assert_eq!(call(&path, "y", &json!({})).unwrap_err(), "bad");
        fred.join().unwrap();
        let m = fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o700);
    }
}
