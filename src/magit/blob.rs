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
