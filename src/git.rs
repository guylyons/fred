//! Lines changed against git's staging area, marked in the gutter as you
//! type (like vim-gitgutter).

use crate::buffer::Buffer;
use similar::{Algorithm, DiffOp};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Diffing stops refining after this long (a huge file stays responsive).
const DEADLINE: Duration = Duration::from_millis(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Added,
    Changed,
    /// Lines were removed just above this one.
    RemovedAbove,
    /// Lines were removed just below this one (at the end of the file).
    RemovedBelow,
}

/// The staged text: loading, absent (no repo, untracked), or its lines.
type Base = Arc<Mutex<Option<Option<Vec<String>>>>>;

#[derive(Debug, Default)]
pub struct Gutter {
    base: Base,
    /// One per buffer line, for buffer version `seen`.
    marks: Vec<Option<Mark>>,
    seen: Option<u64>,
}

impl Gutter {
    /// Fetch `path`'s staged version in the background.
    pub fn load(path: &Path) -> Gutter {
        let g = Gutter::default();
        let base = Arc::clone(&g.base);
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        std::thread::spawn(move || {
            let staged = staged(&path);
            if let Ok(mut b) = base.lock() {
                *b = Some(staged);
            }
        });
        g
    }

    /// The file is in git: the gutter has a column for marks.
    pub fn active(&self) -> bool {
        self.base.lock().is_ok_and(|b| matches!(*b, Some(Some(_))))
    }

    pub fn mark(&self, line: usize) -> Option<Mark> {
        self.marks.get(line).copied().flatten()
    }

    /// Recompute after the buffer changed; true if the marks may have.
    pub fn refresh(&mut self, buf: &Buffer) -> bool {
        if self.seen == Some(buf.version) {
            return false;
        }
        let Ok(base) = self.base.lock() else {
            return false;
        };
        let Some(base) = &*base else {
            return false; // still loading: try again next tick
        };
        self.marks = match base {
            Some(lines) => {
                // Borrow lines from the rope where they're contiguous (most
                // are): copying 100k lines per keystroke took ~25 ms.
                let cur: Vec<Cow<str>> = if buf.len_bytes() == 0 {
                    vec![]
                } else {
                    buf.rope()
                        .lines()
                        .take(buf.len_lines())
                        .map(|l| match l.as_str() {
                            Some(s) => Cow::Borrowed(s.trim_end_matches(['\n', '\r'])),
                            None => {
                                Cow::Owned(l.to_string().trim_end_matches(['\n', '\r']).to_string())
                            }
                        })
                        .collect()
                };
                let base: Vec<&str> = lines.iter().map(String::as_str).collect();
                let cur: Vec<&str> = cur.iter().map(AsRef::as_ref).collect();
                marks(&base, &cur)
            }
            None => vec![],
        };
        self.seen = Some(buf.version);
        true
    }
}

/// `git show :./NAME`, run in the file's directory; None if it fails.
fn staged(path: &Path) -> Option<Vec<String>> {
    let dir = path.parent()?;
    let name = path.file_name()?.to_str()?;
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("show")
        .arg(format!(":./{name}"))
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
    )
}

/// A file's state in `git status`, for the dots in the file pickers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    /// Changed, staged or not.
    Modified,
    /// Untracked, or newly added.
    New,
}

/// `git status` for the repository at `root`: paths relative to the
/// repository's top level → state. None if git fails.
pub fn status(root: &Path) -> Option<HashMap<String, FileState>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| parse_status(&out.stdout))
}

/// `XY path\0` entries; a rename or copy is followed by its old path.
fn parse_status(out: &[u8]) -> HashMap<String, FileState> {
    let mut map = HashMap::new();
    let mut fields = out.split(|&b| b == 0).filter(|f| f.len() > 3);
    while let Some(f) = fields.next() {
        let (x, y) = (f[0], f[1]);
        let Ok(path) = std::str::from_utf8(&f[3..]) else {
            continue;
        };
        if matches!(x, b'R' | b'C') {
            fields.next();
        }
        let state = if x == b'?' || x == b'A' {
            FileState::New
        } else if x == b'D' || y == b'D' {
            continue; // gone from the working tree: never listed
        } else {
            FileState::Modified
        };
        map.insert(path.to_string(), state);
    }
    map
}

/// A mark for each line of `cur` that differs from `base`.
pub fn marks<S: AsRef<str>>(base: &[S], cur: &[S]) -> Vec<Option<Mark>> {
    let old: Vec<&str> = base.iter().map(AsRef::as_ref).collect();
    let new: Vec<&str> = cur.iter().map(AsRef::as_ref).collect();
    let ops = similar::capture_diff_slices_deadline(
        Algorithm::Myers,
        &old,
        &new,
        Some(Instant::now() + DEADLINE),
    );
    let mut out = vec![None; new.len()];
    for op in ops {
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert {
                new_index, new_len, ..
            } => out[new_index..new_index + new_len].fill(Some(Mark::Added)),
            // Replacing 1 line with 3: 1 changed, 2 added.
            DiffOp::Replace {
                old_len,
                new_index,
                new_len,
                ..
            } => {
                let changed = old_len.min(new_len);
                out[new_index..new_index + changed].fill(Some(Mark::Changed));
                out[new_index + changed..new_index + new_len].fill(Some(Mark::Added));
            }
            DiffOp::Delete { new_index, .. } => {
                let (at, mark) = if new_index < out.len() {
                    (new_index, Mark::RemovedAbove)
                } else if new_index > 0 {
                    (new_index - 1, Mark::RemovedBelow)
                } else {
                    continue; // everything deleted: no line to mark
                };
                if out[at].is_none() {
                    out[at] = Some(mark);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Mark::*;

    fn m(base: &str, cur: &str) -> Vec<Option<Mark>> {
        let l = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
        marks(&l(base), &l(cur))
    }

    #[test]
    fn marks_added_changed_removed() {
        assert_eq!(m("a\nb\nc", "a\nb\nc"), [None, None, None]);
        assert_eq!(m("a\nc", "a\nb\nc"), [None, Some(Added), None]);
        assert_eq!(m("a\nb\nc", "a\nB\nc"), [None, Some(Changed), None]);
        assert_eq!(
            m("a\nb", "a\nB\nC\nD"),
            [None, Some(Changed), Some(Added), Some(Added)]
        );
        assert_eq!(m("a\nb\nc", "a\nc"), [None, Some(RemovedAbove)]);
        assert_eq!(m("a\nb\nc", "a\nb"), [None, Some(RemovedBelow)]);
        assert_eq!(m("", "x\ny"), [Some(Added), Some(Added)]);
        assert_eq!(m("a", ""), Vec::<Option<Mark>>::new());
    }

    #[test]
    fn parses_status() {
        let out =
            b" M src/app.rs\0?? new.txt\0A  added.rs\0R  to.rs\0from.rs\0 D gone.rs\0MM both.rs\0";
        let m = parse_status(out);
        assert_eq!(m.get("src/app.rs"), Some(&FileState::Modified));
        assert_eq!(m.get("new.txt"), Some(&FileState::New));
        assert_eq!(m.get("added.rs"), Some(&FileState::New));
        assert_eq!(m.get("to.rs"), Some(&FileState::Modified));
        assert_eq!(m.get("from.rs"), None);
        assert_eq!(m.get("gone.rs"), None);
        assert_eq!(m.get("both.rs"), Some(&FileState::Modified));
    }

    #[test]
    fn staged_text_from_a_real_repo() {
        let d = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(d.path())
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap()
                .status
                .success()
        };
        if !git(&["init", "-q"]) {
            return; // no git here: nothing to test
        }
        std::fs::write(d.path().join("f"), "one\ntwo\n").unwrap();
        assert_eq!(staged(&d.path().join("f")), None, "untracked");
        assert!(git(&["add", "f"]));
        std::fs::write(d.path().join("f"), "one\nTWO\nthree\n").unwrap();
        assert_eq!(staged(&d.path().join("f")).unwrap(), ["one", "two"]);
        let mut g = Gutter::load(&d.path().join("f"));
        let buf = Buffer::from_text("one\nTWO\nthree");
        let t = Instant::now();
        while !g.refresh(&buf) {
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(g.active());
        assert_eq!(
            (g.mark(0), g.mark(1), g.mark(2)),
            (None, Some(Changed), Some(Added))
        );
        assert!(!g.refresh(&buf), "same version: nothing to do");
    }
}
