//! magit-files.el: read-only buffers visiting REV:FILE, and blob navigation.
use super::repo::Repo;
use crate::editor::{Editor, Mode};
use crate::ex::ExEffect;
use crate::key::{Key, KeyCode};
use std::path::{Path, PathBuf};

pub const WORKTREE: &str = "{worktree}";
pub const INDEX: &str = "{index}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub repo: Repo,
    /// An abbreviated commit, or "{index}".
    pub rev: String,
    /// Repository-relative path.
    pub file: PathBuf,
}

fn value(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid value {v:?}"));
    }
    Ok(v)
}

impl Repo {
    /// Resolve a user revision to the abbreviated commit a blob buffer shows.
    pub fn blob_rev(&self, rev: &str) -> Result<String, String> {
        if rev == INDEX || rev == WORKTREE {
            return Ok(rev.into());
        }
        let out = self
            .read(&[
                "rev-parse",
                "--verify",
                "-q",
                "--short",
                "--end-of-options",
                &format!("{}^{{commit}}", value(rev)?),
            ])
            .map_err(|_| format!("unknown revision {rev:?}"))?;
        Ok(String::from_utf8_lossy(&out).trim().into())
    }
    pub fn blob_bytes(&self, rev: &str, file: &Path) -> Result<Vec<u8>, String> {
        let file = file.to_str().ok_or("non-UTF-8 path")?;
        let object = if rev == INDEX {
            // Stage 0 explicitly: ":1:f" would otherwise read as a merge stage.
            format!(":0:{file}")
        } else {
            format!("{}:{file}", value(rev)?)
        };
        self.read(&["cat-file", "blob", &object])
            .map_err(|_| format!("{file} does not exist in {rev}"))
    }
    /// More than one index stage: the file is unmerged.
    pub fn conflicted(&self, file: &Path) -> bool {
        let args = self.path_args(&["ls-files", "--stage"], file);
        self.run(&args, None)
            .is_ok_and(|out| out.split(|b| *b == b'\n').filter(|l| !l.is_empty()).count() > 1)
    }
    fn anything_staged(&self, file: &Path) -> bool {
        let mut args = vec!["diff".into(), "--cached".into(), "--quiet".into()];
        args.extend(self.path_args(&[], file));
        self.run(&args, None).is_err()
    }
    /// `git log --format=%h --name-only --follow REV -- FILE` as (rev, file) pairs.
    fn file_revisions(
        &self,
        rev: &str,
        file: &Path,
        limit: Option<usize>,
    ) -> Vec<(String, PathBuf)> {
        let limit = limit.map(|n| format!("-{n}"));
        let mut args: Vec<&str> = vec!["log"];
        args.extend(limit.as_deref());
        args.extend(["-z", "--format=%h", "--name-only", "--follow", rev]);
        let args = self.path_args(&args, file);
        let Ok(out) = self.run(&args, None) else {
            return vec![];
        };
        // -z keeps names unquoted: "hash\0\nname\0" per commit.
        let fields: Vec<_> = out
            .split(|b| *b == 0)
            .map(|f| f.strip_prefix(b"\n").unwrap_or(f))
            .filter(|f| !f.is_empty())
            .collect();
        fields
            .chunks(2)
            .filter(|p| p.len() == 2)
            .map(|p| {
                use std::os::unix::ffi::OsStrExt;
                (
                    String::from_utf8_lossy(p[0]).into_owned(),
                    PathBuf::from(std::ffi::OsStr::from_bytes(p[1])),
                )
            })
            .collect()
    }
    /// magit-blob-ancestor.
    pub fn blob_ancestor(&self, rev: &str, file: &Path) -> Option<(String, PathBuf)> {
        match rev {
            WORKTREE if self.anything_staged(file) => Some((INDEX.into(), file.into())),
            WORKTREE | INDEX => self
                .blob_rev("HEAD")
                .ok()
                .map(|head| (head, file.to_path_buf())),
            _ => self
                .file_revisions(value(rev).ok()?, file, Some(2))
                .into_iter()
                .nth(1),
        }
    }
    /// magit-blob-successor.
    pub fn blob_successor(&self, rev: &str, file: &Path) -> Option<(String, PathBuf)> {
        match rev {
            WORKTREE => None,
            INDEX => Some((WORKTREE.into(), file.into())),
            _ => {
                let revs = self.file_revisions("HEAD", file, None);
                let at = revs
                    .iter()
                    .position(|(r, _)| rev.starts_with(r.as_str()) || r.starts_with(rev));
                match at {
                    Some(0) | None => Some((
                        if self.anything_staged(file) {
                            INDEX
                        } else {
                            WORKTREE
                        }
                        .into(),
                        file.into(),
                    )),
                    Some(i) => Some(revs[i - 1].clone()),
                }
            }
        }
    }
}

/// magit-blob-mode-map; blame keys take precedence while blaming.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    let Some(blob) = &ed.blob else {
        return false;
    };
    if ed.mode != Mode::Normal || ed.zap.is_some() || ed.explain.is_some() || k.ctrl || k.alt {
        return false;
    }
    if ed.vim.pending == [Key::ch('g')] && k.char() == Some('r') {
        ed.vim.pending.clear();
        let (rev, file) = (blob.rev.clone(), blob.file.clone());
        ed.pending_effect = Some(ExEffect::Magit(super::Action::BlobVisit(rev, file)));
        return true;
    }
    if !ed.vim.pending.is_empty() {
        return false;
    }
    use super::blame::Kind as B;
    let action = match k.code {
        KeyCode::Char('p') => super::Action::BlobPrevious,
        KeyCode::Char('n') => super::Action::BlobNext,
        KeyCode::Char('b') => super::Action::Blame(B::Addition),
        KeyCode::Char('r') => super::Action::Blame(B::Removal),
        KeyCode::Char('f') => super::Action::Blame(B::Reverse),
        KeyCode::Char('q') => super::Action::BlobQuit,
        KeyCode::Enter => return false,
        _ => return false,
    };
    ed.pending_effect = Some(ExEffect::Magit(action));
    true
}

/// magit-files.el file commands from the file dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileOp {
    Stage,
    /// magit-file-stage on an ignored file, after confirmation.
    StageIgnored,
    Unstage,
    Untrack,
    Rename,
    Delete,
    /// Delete after confirming a recursive directory removal.
    DeleteDir,
    Checkout,
}

/// A repository-relative path from an answer (or its default).
pub fn relative(answer: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(answer.trim_end_matches('/'));
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!("{answer:?} is not a repository-relative path"));
    }
    // Like git, never treat repository metadata as a worktree file.
    if path
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
    {
        return Err(format!("{answer:?} is inside Git's metadata"));
    }
    Ok(path)
}

/// `root/rel` only if every component exists with exactly this on-disk name and
/// no intermediate component is a symlink, so filesystem fallbacks cannot leave
/// the repository or alias another file through case-insensitivity. Like
/// upstream's require-match readers. With `must_exist` false, the last
/// component must not exist under any spelling.
pub fn exact(root: &Path, rel: &Path, must_exist: bool) -> Result<PathBuf, String> {
    let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_owned()).collect();
    let mut cur = root.to_path_buf();
    for (i, name) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let present = std::fs::read_dir(&cur)
            .map_err(|e| e.to_string())?
            .flatten()
            .any(|e| e.file_name() == *name);
        cur.push(name);
        if !present {
            if last && !must_exist {
                if cur.symlink_metadata().is_ok() {
                    return Err(format!("{} already exists", super::repo::label(rel)));
                }
                return Ok(cur);
            }
            return Err(format!("{} does not exist", super::repo::label(rel)));
        }
        if last && !must_exist {
            return Err(format!("{} already exists", super::repo::label(rel)));
        }
        if !last
            && cur
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(format!(
                "{} is beyond a symbolic link",
                super::repo::label(rel)
            ));
        }
    }
    Ok(cur)
}

impl Repo {
    pub fn tracked(&self, file: &Path) -> bool {
        let args = self.path_args(&["ls-files", "--error-unmatch"], file);
        self.run(&args, None).is_ok()
    }
    pub fn ignored(&self, file: &Path) -> bool {
        // check-ignore takes plain paths, not pathspec magic.
        let args = [
            "check-ignore".into(),
            "-q".into(),
            "--".into(),
            file.as_os_str().to_owned(),
        ];
        self.run(&args, None).is_ok()
    }
    /// Run a file command; `answers` are already defaulted and validated.
    pub fn file_op(&self, op: FileOp, answers: &[PathBuf], rev: &str) -> Result<(), String> {
        let first = answers.first().ok_or("missing file")?;
        let run =
            |args: &[&str], file: &Path| self.run(&self.path_args(args, file), None).map(|_| ());
        match op {
            FileOp::Stage => run(&["add"], first),
            FileOp::StageIgnored => run(&["add", "--force"], first),
            FileOp::Unstage => self.unstage_file(first),
            FileOp::Untrack => run(&["rm", "--cached"], first),
            FileOp::Checkout => {
                let rev = self.blob_rev(rev)?;
                run(&["checkout", &rev], first)
            }
            // Both paths were resolved with `exact`; `to` is the final name.
            FileOp::Rename => {
                let to = answers.get(1).ok_or("missing destination")?;
                let (from_abs, to_abs) = (self.root.join(first), self.root.join(to));
                if self.tracked(first) {
                    let mut args: Vec<std::ffi::OsString> = vec!["mv".into(), "--".into()];
                    args.push(from_abs.into());
                    args.push(to_abs.into());
                    self.run(&args, None).map(|_| ())
                } else {
                    std::fs::rename(from_abs, to_abs).map_err(|e| e.to_string())
                }
            }
            FileOp::Delete | FileOp::DeleteDir => {
                let abs = self.root.join(first);
                let dir = abs.is_dir();
                if self.tracked(first) {
                    let mut args = vec!["rm"];
                    if dir {
                        args.push("-r");
                    }
                    run(&args, first)
                } else if dir {
                    std::fs::remove_dir_all(abs).map_err(|e| e.to_string())
                } else {
                    std::fs::remove_file(abs).map_err(|e| e.to_string())
                }
            }
        }
    }
}
