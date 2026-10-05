//! Persistent sessions (ob-comint.el adapted): an interpreter per
//! `(language, :session name)` kept as a child process. Fred has no
//! comint buffers, so each evaluation sends the code followed by an
//! end-of-evaluation marker command (upstream's `*-eoe-indicator`) and
//! reads the process output up to the marker. Interpreters without a
//! usable plain-stdin REPL (Python, Ruby, Node, Julia) run a small loop
//! that reads a code block up to a terminator line, evaluates it in a
//! persistent global environment, then prints the marker. The session's
//! input and output are kept as a transcript for
//! org-babel-switch-to-session.

use super::core::{R, err};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

/// How a session's interpreter is run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A shell by name (sh, bash, zsh, fish, ...).
    Shell(String),
    Python(String),
    Ruby(String),
    Node(String),
    R(String),
    Julia(String),
    Haskell,
    Ocaml(String),
    /// A command reading stdin, the marker command and its output.
    Plain(Vec<String>, String, String),
}

const END: &str = "__fred_ob_end__";

impl Kind {
    fn label(&self) -> String {
        match self {
            Kind::Shell(s) => format!("shell:{s}"),
            Kind::Python(_) => "python".into(),
            Kind::Ruby(_) => "ruby".into(),
            Kind::Node(_) => "js".into(),
            Kind::R(_) => "R".into(),
            Kind::Julia(_) => "julia".into(),
            Kind::Haskell => "haskell".into(),
            Kind::Ocaml(_) => "ocaml".into(),
            Kind::Plain(c, ..) => format!("plain:{}", c.join(" ")),
        }
    }

    fn command(&self) -> Vec<String> {
        let split = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        match self {
            Kind::Shell(s) => vec![s.clone()],
            Kind::Python(c) => {
                let mut v = split(c);
                v.extend(["-u".into(), "-c".into(), PY_LOOP.into()]);
                v
            }
            Kind::Ruby(c) => {
                let mut v = split(c);
                v.extend(["-e".into(), RUBY_LOOP.into()]);
                v
            }
            Kind::Node(c) => {
                let mut v = split(c);
                v.extend(["-e".into(), NODE_LOOP.into()]);
                v
            }
            Kind::R(c) => split(c),
            Kind::Julia(c) => {
                let mut v = split(c);
                v.extend(["-e".into(), JULIA_LOOP.into()]);
                v
            }
            Kind::Haskell => vec!["ghci".into(), "-v0".into()],
            Kind::Ocaml(c) => {
                let mut v = split(c);
                v.push("-noprompt".into());
                v
            }
            Kind::Plain(c, ..) => c.clone(),
        }
    }

    /// Text sent after the code, and the marker read back.
    fn ending(&self) -> (String, &str) {
        match self {
            Kind::Shell(_) => ("\necho 'org_babel_sh_eoe'\n".into(), "org_babel_sh_eoe"),
            Kind::Python(_) => (format!("\n{END}\n"), "org_babel_python_eoe"),
            Kind::Ruby(_) => (format!("\n{END}\n"), "org_babel_ruby_eoe"),
            Kind::Node(_) => (format!("\n{END}\n"), "org-babel-js-eoe"),
            Kind::Julia(_) => (format!("\n{END}\n"), "org_babel_julia_eoe"),
            Kind::R(_) => ("\ncat('org_babel_R_eoe\\n')\n".into(), "org_babel_R_eoe"),
            Kind::Haskell => ("\nputStrLn \"org-babel-haskell-eoe\"\n".into(), "org-babel-haskell-eoe"),
            Kind::Ocaml(_) => ("\n\"org-babel-ocaml-eoe\";;\n".into(), "org-babel-ocaml-eoe"),
            Kind::Plain(_, cmd, marker) => (format!("\n{cmd}\n"), marker.as_str()),
        }
    }

    fn setup(&self) -> &str {
        match self {
            Kind::Haskell => ":set prompt \"\"\n:set prompt-cont \"\"\n",
            _ => "",
        }
    }
}

const PY_LOOP: &str = r#"import sys, traceback
__ob_g = {'__name__': '__main__'}
while True:
    __ob_lines = []
    while True:
        __ob_l = sys.stdin.readline()
        if not __ob_l:
            sys.exit(0)
        if __ob_l.rstrip('\n') == '__fred_ob_end__':
            break
        __ob_lines.append(__ob_l)
    try:
        exec(compile(''.join(__ob_lines), '<org-babel>', 'exec'), __ob_g)
    except SystemExit:
        raise
    except BaseException:
        traceback.print_exc(file=sys.stdout)
    sys.stdout.write('org_babel_python_eoe\n')
    sys.stdout.flush()
"#;

const RUBY_LOOP: &str = r##"$stdout.sync = true
$ob_last = nil
loop do
  code = +""
  l = nil
  while (l = $stdin.gets)
    break if l.chomp == "__fred_ob_end__"
    code << l
  end
  exit if l.nil?
  begin
    $ob_last = TOPLEVEL_BINDING.eval(code)
  rescue Exception => e
    puts "#{e.class}: #{e.message}"
  end
  puts "org_babel_ruby_eoe"
end
"##;

const NODE_LOOP: &str = r#"const vm = require('vm');
const ctx = vm.createContext({require, console, process, Buffer, setTimeout, setInterval, clearTimeout, clearInterval, __ob_last: undefined});
let buf = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', d => {
  buf += d;
  let i;
  while ((i = buf.indexOf('\n__fred_ob_end__\n')) >= 0) {
    const code = buf.slice(0, i);
    buf = buf.slice(i + 17);
    try { ctx.__ob_last = vm.runInContext(code, ctx); }
    catch (e) { console.log(String(e && e.stack || e)); }
    process.stdout.write('org-babel-js-eoe\n');
  }
});
"#;

const JULIA_LOOP: &str = r#"while true
    code = IOBuffer()
    while true
        l = readline(stdin; keep=true)
        if l == ""
            exit(0)
        end
        if chomp(l) == "__fred_ob_end__"
            break
        end
        write(code, l)
    end
    try
        global ans = include_string(Main, String(take!(code)))
    catch e
        showerror(stdout, e)
        println()
    end
    println("org_babel_julia_eoe")
    flush(stdout)
end
"#;

struct Session {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    transcript: String,
    kind: Kind,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

thread_local! {
    static SESSIONS: std::cell::RefCell<HashMap<(String, String), Session>> = std::cell::RefCell::new(HashMap::new());
}

fn start(kind: &Kind, dir: &Path) -> R<Session> {
    let cmd = kind.command();
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("exec \"$@\" 2>&1")
        .arg("sh")
        .args(&cmd)
        .current_dir(if dir.is_dir() { dir } else { Path::new("/") })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot start session {}: {e}", cmd.join(" ")))?;
    let mut out = child.stdout.take().ok_or("no session output")?;
    let stdin = child.stdin.take().ok_or("no session input")?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut s = Session { child, stdin, rx, pending: vec![], transcript: String::new(), kind: kind.clone() };
    let setup = kind.setup();
    if !setup.is_empty() {
        s.stdin.write_all(setup.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(s)
}

impl Session {
    fn eval(&mut self, code: &str) -> R<String> {
        let (ending, marker) = self.kind.ending();
        let marker = marker.to_owned();
        let input = format!("{code}{ending}");
        self.stdin.write_all(input.as_bytes()).and_then(|_| self.stdin.flush()).map_err(|e| format!("Session process is gone: {e}"))?;
        self.transcript.push_str(&format!("> {}\n", code.replace('\n', "\n> ")));
        loop {
            let text = String::from_utf8_lossy(&self.pending).into_owned();
            if let Some(i) = text.find(&marker) {
                let mut out = text[..i].to_owned();
                // OCaml prints the marker as `- : string = "marker"`.
                if matches!(self.kind, Kind::Ocaml(_)) {
                    out.truncate(out.rfind('\n').map_or(0, |l| l + 1));
                }
                let rest = &text[i + marker.len()..];
                let rest = rest.split_once('\n').map_or("", |(_, r)| r);
                self.pending = rest.as_bytes().to_vec();
                self.transcript.push_str(&out);
                return Ok(out);
            }
            match self.rx.recv() {
                Ok(chunk) => self.pending.extend(chunk),
                Err(_) => {
                    let out = String::from_utf8_lossy(&self.pending).into_owned();
                    self.transcript.push_str(&out);
                    self.transcript.push_str("\n[process exited]\n");
                    return err(format!("Session process exited: {}", out.trim()));
                }
            }
        }
    }
}

fn key(kind: &Kind, name: &str) -> (String, String) {
    (kind.label(), name.to_owned())
}

/// Evaluate CODE in session NAME (started in DIR when needed); the output
/// before the end marker.
pub fn eval(kind: &Kind, name: &str, dir: &Path, code: &str) -> R<String> {
    SESSIONS.with(|s| {
        let mut map = s.borrow_mut();
        let k = key(kind, name);
        let alive = map.get_mut(&k).is_some_and(|x| matches!(x.child.try_wait(), Ok(None)));
        if !alive {
            map.insert(k.clone(), start(kind, dir)?);
        }
        let out = map.get_mut(&k).unwrap().eval(code);
        if out.is_err() {
            map.remove(&k);
        }
        out
    })
}

/// org-babel-comint-buffer-livep.
pub fn live(kind: &Kind, name: &str) -> bool {
    SESSIONS.with(|s| s.borrow_mut().get_mut(&key(kind, name)).is_some_and(|x| matches!(x.child.try_wait(), Ok(None))))
}

/// Start session NAME if it is not running.
pub fn initiate(kind: &Kind, name: &str, dir: &Path) -> R<()> {
    if !live(kind, name) {
        let s = start(kind, dir)?;
        SESSIONS.with(|m| m.borrow_mut().insert(key(kind, name), s));
    }
    Ok(())
}

/// The session's input and output so far.
pub fn transcript(kind: &Kind, name: &str) -> Option<String> {
    SESSIONS.with(|s| s.borrow().get(&key(kind, name)).map(|x| x.transcript.clone()))
}

pub fn kill(kind: &Kind, name: &str) {
    SESSIONS.with(|s| s.borrow_mut().remove(&key(kind, name)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_session_keeps_state() {
        let k = Kind::Shell("sh".into());
        let dir = std::env::temp_dir();
        assert_eq!(eval(&k, "t", &dir, "X=5").unwrap(), "");
        assert_eq!(eval(&k, "t", &dir, "echo $X").unwrap(), "5\n");
        assert_eq!(eval(&k, "t", &dir, "printf a").unwrap(), "a");
        assert!(transcript(&k, "t").unwrap().contains("> echo $X\n5\n"));
        kill(&k, "t");
        assert!(!live(&k, "t"));
    }
}
