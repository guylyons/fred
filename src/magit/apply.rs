//! magit-apply.el: discard, reverse, stage all modified and unstage all.
use super::Section;
use super::branch::Next;
use super::repo::{Diff, Repo, label};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Discard,
    Reverse,
    StageModified,
    UnstageAll,
}

/// The section, file or hunk at point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Thing {
    Section(Section),
    File(PathBuf, Section),
    Hunk(Diff, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    pub kind: Kind,
    pub thing: Option<Thing>,
}

impl Op {
    /// magit-confirm's question, if any.
    pub fn question(&self) -> Result<Option<String>, String> {
        let name = |p: &PathBuf| label(p);
        Ok(Some(match (self.kind, &self.thing) {
            (Kind::StageModified, _) => return Ok(None),
            (Kind::UnstageAll, _) => "Unstage all changes? (y or n) ".into(),
            (_, None) => return Err("Nothing at point".into()),
            (Kind::Discard, Some(Thing::Hunk(..))) => "Discard hunk? (y or n) ".into(),
            (Kind::Reverse, Some(Thing::Hunk(..))) => "Reverse hunk? (y or n) ".into(),
            (Kind::Discard, Some(Thing::File(p, Section::Untracked))) => {
                format!("Delete untracked {}? (y or n) ", name(p))
            }
            (Kind::Discard, Some(Thing::File(p, Section::Staged))) => {
                format!("Discard staged changes in {}? (y or n) ", name(p))
            }
            (Kind::Discard, Some(Thing::File(p, _))) => {
                format!("Discard changes in {}? (y or n) ", name(p))
            }
            (Kind::Reverse, Some(Thing::File(p, _))) => {
                format!("Reverse changes in {}? (y or n) ", name(p))
            }
            (Kind::Discard, Some(Thing::Section(Section::Untracked))) => {
                "Delete all untracked files? (y or n) ".into()
            }
            (Kind::Discard, Some(Thing::Section(Section::Staged))) => {
                "Discard all staged changes? (y or n) ".into()
            }
            (Kind::Discard, Some(Thing::Section(_))) => {
                "Discard all unstaged changes? (y or n) ".into()
            }
            (Kind::Reverse, Some(Thing::Section(_))) => {
                "Reverse all staged changes? (y or n) ".into()
            }
        }))
    }
}

impl Repo {
    fn in_head(&self, path: &Path) -> bool {
        self.read(&[
            "cat-file",
            "-e",
            &format!("HEAD:{}", path.to_string_lossy()),
        ])
        .is_ok()
    }
    /// magit-discard-files--discard for a staged file: back to HEAD in the
    /// index and worktree, or gone if HEAD does not have it.
    fn discard_staged_file(&self, path: &Path) -> Result<(), String> {
        if self.in_head(path) {
            self.run(&self.path_args(&["checkout", "HEAD"], path), None)?;
        } else {
            self.run(&self.path_args(&["rm", "-f", "-q"], path), None)?;
        }
        Ok(())
    }
    /// Reverse staged changes in the worktree only.
    fn reverse_staged(&self, path: Option<&Path>) -> Result<(), String> {
        let mut args: Vec<std::ffi::OsString> = [
            "diff",
            "--cached",
            "--no-ext-diff",
            "--no-color",
            "--binary",
        ]
        .iter()
        .map(Into::into)
        .collect();
        if let Some(p) = path {
            args = self.path_args(
                &[
                    "diff",
                    "--cached",
                    "--no-ext-diff",
                    "--no-color",
                    "--binary",
                ],
                p,
            );
        }
        let patch = self.run(&args, None)?;
        if patch.is_empty() {
            return Err("Nothing to reverse".into());
        }
        self.run(&["apply".into(), "--reverse".into()], Some(&patch))
            .map(|_| ())
    }
    pub fn apply_step(&self, op: Op, answer: &str) -> Result<Next, String> {
        if op.question()?.is_some() && !matches!(answer.trim(), "y" | "yes") {
            return Err("Abort".into());
        }
        let done = |m: &str| Ok(Next::Done(Ok(m.to_owned())));
        match (op.kind, op.thing) {
            (Kind::StageModified, _) => {
                self.read(&["add", "-u", "--", "."])?;
                done("Staged all modified files")
            }
            (Kind::UnstageAll, _) => {
                if self.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
                    self.read(&["reset", "-q", "--", "."])?;
                } else {
                    self.read(&["rm", "--cached", "-r", "-q", "--", "."])?;
                }
                done("Unstaged all changes")
            }
            (_, None) => Err("Nothing at point".into()),
            (_, Some(Thing::File(_, Section::Conflicts) | Thing::Section(Section::Conflicts))) => {
                Err("Resolve conflicts with the file's own commands".into())
            }
            (Kind::Discard, Some(Thing::Hunk(diff, i))) => {
                let mode: &[&str] = if diff.staged {
                    &["--index", "--reverse"]
                } else {
                    &["--reverse"]
                };
                self.apply_hunk_with(&diff, i, mode).map_err(|e| {
                    if diff.staged {
                        format!("{e} (discard the file's unstaged changes first)")
                    } else {
                        e
                    }
                })?;
                done("Discarded hunk")
            }
            (Kind::Reverse, Some(Thing::Hunk(diff, i))) => {
                if !diff.staged {
                    return Err("Cannot reverse unstaged changes".into());
                }
                self.apply_hunk_with(&diff, i, &["--reverse"])?;
                done("Reversed hunk")
            }
            (Kind::Discard, Some(Thing::File(p, Section::Untracked))) => {
                self.run(&self.path_args(&["clean", "-f", "-d", "-q"], &p), None)?;
                done("Deleted untracked file")
            }
            (Kind::Discard, Some(Thing::File(p, Section::Staged))) => {
                self.discard_staged_file(&p)?;
                done("Discarded staged changes")
            }
            (Kind::Discard, Some(Thing::File(p, _))) => {
                self.run(&self.path_args(&["checkout"], &p), None)?;
                done("Discarded changes")
            }
            (Kind::Reverse, Some(Thing::File(p, Section::Staged))) => {
                self.reverse_staged(Some(&p))?;
                done("Reversed changes")
            }
            (Kind::Reverse, Some(Thing::File(..))) => Err("Cannot reverse unstaged changes".into()),
            (Kind::Discard, Some(Thing::Section(Section::Untracked))) => {
                self.read(&["clean", "-f", "-d", "-q", "--", "."])?;
                done("Deleted untracked files")
            }
            (Kind::Discard, Some(Thing::Section(Section::Unstaged))) => {
                self.read(&["checkout", "--", "."])?;
                done("Discarded unstaged changes")
            }
            (Kind::Discard, Some(Thing::Section(Section::Staged))) => {
                let names = self.read(&["diff", "--cached", "--name-only", "-z"])?;
                for name in names.split(|b| *b == 0).filter(|n| !n.is_empty()) {
                    let p = PathBuf::from(String::from_utf8_lossy(name).into_owned());
                    self.discard_staged_file(&p)?;
                }
                done("Discarded staged changes")
            }
            (Kind::Reverse, Some(Thing::Section(Section::Staged))) => {
                self.reverse_staged(None)?;
                done("Reversed staged changes")
            }
            _ => Err("Nothing to do for this section".into()),
        }
    }
}
