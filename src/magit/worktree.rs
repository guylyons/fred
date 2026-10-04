//! magit-worktree.el: create, move, delete and visit worktrees.
use super::Question;
use super::branch::Next;
use super::repo::{Repo, label};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// b: checkout a commit in a new worktree.
    Checkout,
    /// c: new branch in a new worktree.
    Branch,
    Move,
    Delete,
    /// Deletion of this worktree path, and whether it holds uncommitted work.
    DeleteConfirmed(PathBuf, bool),
    Visit,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub locked: Option<String>,
}

impl Repo {
    /// magit-list-worktrees (git worktree list --porcelain -z).
    pub fn worktrees(&self) -> Result<Vec<Worktree>, String> {
        use std::os::unix::ffi::OsStrExt;
        let out = self.read(&["worktree", "list", "--porcelain", "-z"])?;
        let mut list: Vec<Worktree> = vec![];
        for field in out.split(|b| *b == 0) {
            if let Some(p) = field.strip_prefix(b"worktree ") {
                list.push(Worktree {
                    path: PathBuf::from(std::ffi::OsStr::from_bytes(p)),
                    ..Default::default()
                });
            } else if let Some(b) = field.strip_prefix(b"branch ") {
                if let Some(w) = list.last_mut() {
                    let b = String::from_utf8_lossy(b);
                    w.branch = Some(b.trim_start_matches("refs/heads/").to_owned());
                }
            } else if field.starts_with(b"locked")
                && let Some(w) = list.last_mut()
            {
                let reason = String::from_utf8_lossy(field.strip_prefix(b"locked").unwrap_or(b""));
                w.locked = Some(reason.trim().to_owned());
            }
        }
        Ok(list)
    }
    /// magit-read-worktree-directory-sibling's initial input.
    fn sibling(&self, commit: &str) -> String {
        let name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let prefix = name.split('_').next().unwrap_or(&name);
        let parent = self.root.parent().unwrap_or(Path::new("/"));
        parent
            .join(format!("{prefix}_{}", commit.replace('/', "-")))
            .to_string_lossy()
            .into_owned()
    }
    fn existing_worktree(&self, answer: &str) -> Result<PathBuf, String> {
        let path = PathBuf::from(answer);
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        self.worktrees()?
            .into_iter()
            .find(|w| std::fs::canonicalize(&w.path).unwrap_or(w.path.clone()) == path)
            .map(|w| w.path)
            .ok_or_else(|| format!("{} is not a worktree of this repository", label(&path)))
    }
    pub fn worktree_prompts(
        &self,
        op: &Op,
        at_point: Option<String>,
    ) -> (Vec<String>, Vec<String>) {
        let here = self.current_branch().unwrap_or_else(|_| "HEAD".into());
        let other = self
            .worktrees()
            .unwrap_or_default()
            .into_iter()
            .skip(1)
            .map(|w| w.path.to_string_lossy().into_owned())
            .next()
            .unwrap_or_default();
        let d = at_point.unwrap_or(other);
        match op {
            Op::Checkout => (
                vec![
                    "In new worktree; checkout: ".into(),
                    "Directory (default sibling): ".into(),
                ],
                vec![String::new(), String::new()],
            ),
            Op::Branch => (
                vec![
                    "In new worktree; checkout new branch named: ".into(),
                    format!("Starting at (default {here}): "),
                    "Directory (default sibling): ".into(),
                ],
                vec![String::new(), here, String::new()],
            ),
            Op::Move => (
                vec![
                    format!("Move worktree (default {d}): "),
                    "Move worktree to: ".into(),
                ],
                vec![d, String::new()],
            ),
            Op::Delete => (vec![format!("Delete worktree (default {d}): ")], vec![d]),
            Op::Visit => (
                vec![format!("Show status for worktree (default {d}): ")],
                vec![d],
            ),
            Op::DeleteConfirmed(..) => (vec![], vec![]),
        }
    }
    pub fn worktree_step(&self, op: Op, a: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let checked = |v: &str| -> Result<String, String> {
            if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
                return Err(format!("invalid value {v:?}"));
            }
            Ok(v.to_owned())
        };
        let directory = |answer: &str, commit: &str| -> Result<PathBuf, String> {
            let d = if answer.is_empty() {
                self.sibling(commit)
            } else {
                answer.to_owned()
            };
            let d = match d.strip_prefix("~/") {
                Some(rest) => std::env::var("HOME")
                    .map(|h| format!("{h}/{rest}"))
                    .unwrap_or(d.clone()),
                None => d,
            };
            let path = std::path::absolute(self.root.join(d)).map_err(|e| e.to_string())?;
            if path.exists() {
                return Err(format!("{} already exists", label(&path)));
            }
            Ok(path)
        };
        match op {
            Op::Checkout => {
                let commit = checked(at(0))?;
                self.read(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    "--end-of-options",
                    &format!("{commit}^{{commit}}"),
                ])
                .map_err(|_| format!("unknown revision {commit:?}"))?;
                let dir = directory(at(1), &commit)?;
                let mut args: Vec<std::ffi::OsString> =
                    vec!["worktree".into(), "add".into(), dir.clone().into()];
                args.push(commit.into());
                self.run(&args, None)?;
                Ok(Next::Status(dir))
            }
            Op::Branch => {
                let branch = checked(at(0))?;
                self.read(&["check-ref-format", "--branch", &branch])
                    .map_err(|_| format!("{branch:?} is not a valid branch name"))?;
                let start = checked(at(1))?;
                let dir = directory(at(2), &branch)?;
                let mut args: Vec<std::ffi::OsString> = vec![
                    "worktree".into(),
                    "add".into(),
                    "-b".into(),
                    branch.into(),
                    dir.clone().into(),
                ];
                args.push(start.into());
                self.run(&args, None)?;
                Ok(Next::Status(dir))
            }
            Op::Move => {
                let worktree = self.existing_worktree(at(0))?;
                if worktree.join(".git").is_dir() {
                    return Err("You may not move the main working tree".into());
                }
                let to = checked(at(1))?;
                let to = std::path::absolute(self.root.join(to)).map_err(|e| e.to_string())?;
                let args: Vec<std::ffi::OsString> =
                    vec!["worktree".into(), "move".into(), worktree.into(), to.into()];
                self.run(&args, None)?;
                Ok(Next::Done(Ok("Moved worktree".into())))
            }
            Op::Delete => {
                let worktree = self.existing_worktree(at(0))?;
                if worktree.join(".git").is_dir() {
                    return Err(format!(
                        "Deleting {} would delete the shared .git directory",
                        label(&worktree)
                    ));
                }
                let info = self
                    .worktrees()?
                    .into_iter()
                    .find(|w| w.path == worktree)
                    .unwrap_or_default();
                if let Some(reason) = info.locked {
                    let reason = if reason.is_empty() {
                        String::new()
                    } else {
                        format!(" [reason: {reason}]")
                    };
                    return Err(format!("Cannot delete locked {}{reason}", label(&worktree)));
                }
                let name = worktree
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // magit-worktree-delete: uncommitted work makes it a typed "yes".
                let dirty = !Repo {
                    root: worktree.clone(),
                }
                .read(&["status", "--porcelain"])
                .unwrap_or_default()
                .is_empty();
                let question = if dirty {
                    format!("Delete worktree \"{name}\" despite uncommitted changes? (yes or no) ")
                } else {
                    format!("Delete worktree \"{name}\"? (y or n) ")
                };
                Ok(Next::Ask(
                    Question::Worktree(Op::DeleteConfirmed(worktree, dirty)),
                    vec![question],
                    vec![String::new()],
                ))
            }
            Op::DeleteConfirmed(worktree, dirty) => {
                let yes = if dirty {
                    at(0) == "yes"
                } else {
                    matches!(at(0), "y" | "yes")
                };
                if !yes {
                    return Err("Abort".into());
                }
                let args: Vec<std::ffi::OsString> = vec![
                    "worktree".into(),
                    "remove".into(),
                    "--force".into(),
                    worktree.clone().into(),
                ];
                self.run(&args, None)?;
                self.read(&["worktree", "prune"])?;
                Ok(Next::Done(Ok(format!(
                    "Deleted worktree {}",
                    label(&worktree)
                ))))
            }
            Op::Visit => Ok(Next::Status(self.existing_worktree(at(0))?)),
        }
    }
}
