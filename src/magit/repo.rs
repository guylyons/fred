//! Git operations use original path bytes and never shell interpolation.
use std::ffi::OsString;
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub root: PathBuf,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead_behind: Option<String>,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub conflict: bool,
    pub xy: String,
}
fn path(bytes: &[u8]) -> PathBuf {
    OsString::from_vec(bytes.to_vec()).into()
}
fn trim_nl(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    bytes
}
pub fn label(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
impl Repo {
    pub fn discover(from: &Path) -> Result<Self, String> {
        let abs = std::path::absolute(from).map_err(|e| e.to_string())?;
        let dir = if abs.is_dir() {
            abs.as_path()
        } else {
            abs.parent().unwrap_or(Path::new("."))
        };
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--show-toplevel"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("git: {e}"))?;
        if !out.status.success() {
            return Err("not in a Git repository".into());
        }
        let root = path(&trim_nl(out.stdout))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        Ok(Self { root })
    }
    pub fn command(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("--literal-pathspecs").arg("-C").arg(&self.root);
        c
    }
    pub fn run(&self, args: &[OsString], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let mut cmd = self.command();
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
        // Feed stdin concurrently: a hook may emit output before consuming input.
        let writer = input.map(|data| {
            let mut stdin = child.stdin.take().expect("piped stdin");
            let bytes = data.to_vec();
            std::thread::spawn(move || stdin.write_all(&bytes))
        });
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if let Some(w) = writer {
            let _ = w.join();
        }
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr[..out.stderr.len().min(16384)]);
            return Err(format!("git ({}): {}", out.status, stderr.trim()));
        }
        Ok(out.stdout)
    }
    pub fn read(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        self.run(&args.iter().map(OsString::from).collect::<Vec<_>>(), None)
    }
    pub fn status(&self) -> Result<Snapshot, String> {
        let data = self.read(&["status", "--porcelain=v2", "--branch", "-z"])?;
        Self::parse_status(&data)
    }
    pub fn parse_status(data: &[u8]) -> Result<Snapshot, String> {
        let mut records = data.split(|b| *b == 0);
        let mut s = Snapshot::default();
        while let Some(rec) = records.next() {
            if let Some(h) = rec.strip_prefix(b"# branch.head ") {
                s.branch = String::from_utf8_lossy(h).into();
            } else if let Some(h) = rec.strip_prefix(b"# branch.upstream ") {
                s.upstream = Some(String::from_utf8_lossy(h).into());
            } else if let Some(h) = rec.strip_prefix(b"# branch.ab ") {
                s.ahead_behind = Some(String::from_utf8_lossy(h).into());
            } else if matches!(rec.first(), Some(b'1' | b'2' | b'u' | b'?')) {
                let n = match rec[0] {
                    b'1' => 9,
                    b'2' => 10,
                    b'u' => 11,
                    _ => 2,
                };
                let fields: Vec<_> = rec.splitn(n, |b| *b == b' ').collect();
                if fields.len() != n {
                    return Err("invalid Git status output".into());
                }
                let untracked = rec[0] == b'?';
                let conflict = rec[0] == b'u';
                let xy = if untracked {
                    b"??".as_slice()
                } else {
                    fields[1]
                };
                if xy.len() != 2 {
                    return Err("invalid Git status flags".into());
                }
                let old_path = if rec[0] == b'2' {
                    Some(path(records.next().ok_or("missing rename source")?))
                } else {
                    None
                };
                s.entries.push(Entry {
                    path: path(fields[n - 1]),
                    old_path,
                    staged: !untracked && !conflict && xy[0] != b'.',
                    unstaged: !untracked && !conflict && xy[1] != b'.',
                    untracked,
                    conflict,
                    xy: String::from_utf8_lossy(xy).into(),
                });
            }
        }
        if s.branch == "(detached)" {
            s.branch = "detached HEAD".into();
        }
        Ok(s)
    }
    pub fn path_args(&self, args: &[&str], path: &Path) -> Vec<OsString> {
        let mut a: Vec<_> = args.iter().map(OsString::from).collect();
        a.push("--".into());
        a.push(path.as_os_str().to_owned());
        a
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub start: usize,
    pub end: usize,
    pub line: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    pub path: PathBuf,
    pub staged: bool,
    pub bytes: Vec<u8>,
    pub hunks: Vec<Hunk>,
}
impl Repo {
    pub fn stage_file(&self, path: &Path) -> Result<(), String> {
        self.run(&self.path_args(&["add", "-A"], path), None)
            .map(|_| ())
    }
    pub fn unstage_file(&self, path: &Path) -> Result<(), String> {
        if self.read(&["rev-parse", "--verify", "HEAD"]).is_ok() {
            let mut args = self.path_args(&["reset", "-q", "HEAD"], path);
            if let Some(old) = self
                .status()?
                .entries
                .iter()
                .find(|e| e.path == path)
                .and_then(|e| e.old_path.clone())
            {
                args.push(old.into_os_string());
            }
            self.run(&args, None).map(|_| ())
        } else {
            self.run(
                &self.path_args(&["rm", "--cached", "-f", "--ignore-unmatch"], path),
                None,
            )
            .map(|_| ())
        }
    }
    pub fn diff(&self, path: &Path, staged: bool) -> Result<Diff, String> {
        let mut a = vec![
            "-c",
            "diff.noprefix=false",
            "diff",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--no-relative",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--unified=3",
        ];
        if staged {
            a.push("--cached");
        }
        let bytes = self.run(&self.path_args(&a, path), None)?;
        let mut hunks: Vec<Hunk> = vec![];
        let mut offset = 0;
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            if line.starts_with(b"@@ ") {
                if let Some(h) = hunks.last_mut() {
                    h.end = offset;
                }
                let header = String::from_utf8_lossy(line);
                let range = header.split_whitespace().nth(2).unwrap_or("+1");
                let target = range
                    .trim_start_matches('+')
                    .split(',')
                    .next()
                    .unwrap_or("1")
                    .parse::<usize>()
                    .unwrap_or(1);
                hunks.push(Hunk {
                    start: offset,
                    end: bytes.len(),
                    line: target.saturating_sub(1),
                });
            }
            offset += line.len();
        }
        Ok(Diff {
            path: path.to_path_buf(),
            staged,
            bytes,
            hunks,
        })
    }
    pub fn apply_hunk(&self, diff: &Diff, hunk: usize) -> Result<(), String> {
        let status = self.status()?;
        let entry = status
            .entries
            .iter()
            .find(|e| e.path == diff.path)
            .ok_or("file changed; refresh and select again")?;
        if entry.conflict || entry.old_path.is_some() {
            return Err("use whole-file staging for conflicts or renames".into());
        }
        let current = self.diff(&diff.path, diff.staged)?;
        if current != *diff {
            return Err("repository changed; refresh and select the hunk again".into());
        }
        let h = diff
            .hunks
            .get(hunk)
            .ok_or("no textual hunk; use the whole-file operation")?;
        let header_end = diff.hunks.first().ok_or("no hunk")?.start;
        let mut patch = diff.bytes[..header_end].to_vec();
        patch.extend_from_slice(&diff.bytes[h.start..h.end]);
        let mut args: Vec<OsString> = ["apply", "--cached", "--recount", "--whitespace=nowarn"]
            .iter()
            .map(OsString::from)
            .collect();
        if diff.staged {
            args.push("--reverse".into());
        }
        let mut check = args.clone();
        check.push("--check".into());
        self.run(&check, Some(&patch))?;
        self.run(&args, Some(&patch)).map(|_| ())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}
#[derive(Clone, Debug)]
pub struct GitInvocation {
    pub repo: Repo,
    pub args: Vec<OsString>,
    pub input: Option<Vec<u8>>,
    pub draft: Option<PathBuf>,
    pub draft_stamp: Option<crate::fileio::FileStamp>,
}
impl Repo {
    pub fn history(&self) -> Result<Vec<Commit>, String> {
        if self.read(&["rev-parse", "--verify", "HEAD"]).is_err() {
            return Ok(vec![]);
        }
        let bytes = self.read(&[
            "log",
            "-100",
            "--format=%H%x00%s%x00%an%x00%ad%x00",
            "--date=short",
        ])?;
        let mut fields = bytes.split(|b| *b == 0);
        let mut commits = vec![];
        while let Some(id) = fields.next() {
            let id = String::from_utf8_lossy(id).trim().to_owned();
            if id.is_empty() {
                continue;
            }
            let subject = String::from_utf8_lossy(fields.next().ok_or("invalid log")?).into();
            let author = String::from_utf8_lossy(fields.next().ok_or("invalid log")?).into();
            let date = String::from_utf8_lossy(fields.next().ok_or("invalid log")?).into();
            commits.push(Commit {
                id,
                subject,
                author,
                date,
            });
        }
        Ok(commits)
    }
    pub fn branches(&self) -> Result<Vec<String>, String> {
        let bytes = self.read(&["for-each-ref", "--format=%(refname:short)", "refs/heads/"])?;
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_owned)
            .collect())
    }
    pub fn commit_patch(&self, id: &str) -> Result<Vec<u8>, String> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid commit hash".into());
        }
        self.read(&[
            "show",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            id,
            "--",
        ])
    }
    pub fn commit_invocation(
        &self,
        message: Vec<u8>,
        draft: PathBuf,
    ) -> Result<GitInvocation, String> {
        if message.iter().all(|b| b.is_ascii_whitespace()) {
            return Err("empty commit message".into());
        }
        let draft_stamp = crate::fileio::load(&draft)?.stamp;
        Ok(GitInvocation {
            draft_stamp,
            repo: self.clone(),
            args: ["commit", "-F", "-"].iter().map(OsString::from).collect(),
            input: Some(message),
            draft: Some(draft),
        })
    }
}
