//! Running shell commands for `:!`, `:r !cmd` and `:[range]!` filters.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// `$SHELL -c CMD` (or `/bin/sh`).
pub fn command(cmd: &str) -> Command {
    let sh = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut c = Command::new(sh);
    c.arg("-c").arg(cmd);
    c
}

/// Run `cmd` with `input` on stdin (or none); its output, or why it failed.
pub fn capture(cmd: &str, input: Option<String>) -> Result<String, String> {
    let mut child = command(cmd)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't run the shell: {e}"))?;
    // Write from another thread: a filter that answers as it reads would
    // otherwise block on a full pipe.
    let writer = input.and_then(|text| {
        let mut stdin = child.stdin.take()?;
        Some(std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        }))
    });
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if let Some(w) = writer {
        let _ = w.join();
    }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().find(|l| !l.trim().is_empty());
        return Err(match (first, out.status.code()) {
            (Some(l), _) => l.trim().to_string(),
            (None, Some(c)) => format!("command failed (exit {c})"),
            (None, None) => "command was killed".into(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `%` → the current file's name (quoted for the shell); `\%` → `%`.
pub fn expand(cmd: &str, file: Option<&Path>) -> Result<String, String> {
    let mut out = String::new();
    let mut it = cmd.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' if it.peek() == Some(&'%') => out.push(it.next().unwrap()),
            '%' => {
                let f = file.ok_or("no file name for %")?;
                out.push_str(&quote(&f.to_string_lossy()));
            }
            _ => out.push(c),
        }
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_the_file() {
        let f = Path::new("src/main.rs");
        assert_eq!(expand("wc -l %", Some(f)).unwrap(), "wc -l src/main.rs");
        assert_eq!(expand("echo 100\\%", Some(f)).unwrap(), "echo 100%");
        assert_eq!(
            expand("cat %", Some(Path::new("my file's.txt"))).unwrap(),
            "cat 'my file'\\''s.txt'"
        );
        assert!(expand("cat %", None).is_err());
        assert_eq!(expand("ls", None).unwrap(), "ls");
    }

    #[test]
    fn captures_output_and_errors() {
        assert_eq!(capture("echo hi", None).unwrap(), "hi\n");
        assert_eq!(capture("sort", Some("b\na\n".into())).unwrap(), "a\nb\n");
        assert_eq!(capture("echo oops >&2; exit 3", None).unwrap_err(), "oops");
        assert_eq!(
            capture("exit 4", None).unwrap_err(),
            "command failed (exit 4)"
        );
        // More than a pipe holds, both ways.
        let big = "x\n".repeat(200_000);
        assert_eq!(capture("cat", Some(big.clone())).unwrap(), big);
    }
}
