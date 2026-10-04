//! Structured, repository-anchored workflow commands.
use super::repo::{GitInvocation, Repo};
use std::ffi::OsString;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Stash,
    StashUntracked,
    StashStaged,
    StashKeepIndex,
    StashApply,
    CreateBranch,
    CreateSwitch,
    RenameBranch,
    DeleteBranch,
    Tag,
    Amend,
    Fixup,
    Merge,
    Squash,
    MergeContinue,
    MergeAbort,
    Rebase,
    RebaseContinue,
    RebaseSkip,
    RebaseAbort,
    CherryPick,
    CherryContinue,
    CherrySkip,
    CherryAbort,
    Revert,
    RevertContinue,
    RevertSkip,
    RevertAbort,
}
impl Operation {
    pub fn prompt(self) -> Option<&'static str> {
        use Operation::*;
        match self {
            Stash | StashUntracked | StashStaged | StashKeepIndex => {
                Some("Stash message (optional): ")
            }
            StashApply => Some("Stash reference: "),
            CreateBranch | CreateSwitch | RenameBranch | DeleteBranch => Some("Branch name: "),
            Tag => Some("Tag name: "),
            Fixup | Merge | Squash | Rebase | CherryPick | Revert => Some("Commit or revision: "),
            _ => None,
        }
    }
}
impl Repo {
    pub fn active_workflow(&self) -> Result<Option<&'static str>, String> {
        for (file, name) in [
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
        ] {
            let bytes = self.read(&["rev-parse", "--path-format=absolute", "--git-path", file])?;
            let path = std::path::PathBuf::from(OsString::from_vec(
                bytes.strip_suffix(b"\n").unwrap_or(&bytes).to_vec(),
            ));
            if path.exists() {
                return Ok(Some(name));
            }
        }
        let bytes = self.read(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "sequencer/todo",
        ])?;
        let path = std::path::PathBuf::from(OsString::from_vec(
            bytes.strip_suffix(b"\n").unwrap_or(&bytes).to_vec(),
        ));
        match std::fs::read(path) {
            Ok(todo) => {
                for line in todo.split(|b| *b == b'\n') {
                    if line.starts_with(b"pick ") {
                        return Ok(Some("cherry-pick"));
                    }
                    if line.starts_with(b"revert ") {
                        return Ok(Some("revert"));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
        Ok(None)
    }
    pub fn operation(&self, op: Operation, value: &str) -> Result<GitInvocation, String> {
        use Operation::*;
        let mut args: Vec<String> = vec!["-c".into(), "core.editor=true".into()];
        let mut add = |parts: &[&str]| args.extend(parts.iter().map(|s| s.to_string()));
        let revision = || -> Result<String, String> {
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err("invalid revision".into());
            }
            let bytes = self.read(&[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{value}^{{commit}}"),
            ])?;
            Ok(String::from_utf8_lossy(&bytes).trim().to_string())
        };
        match op {
            Stash | StashUntracked | StashStaged | StashKeepIndex => {
                add(&["stash", "push"]);
                match op {
                    StashUntracked => add(&["--include-untracked"]),
                    StashStaged => add(&["--staged"]),
                    StashKeepIndex => add(&["--keep-index"]),
                    _ => (),
                }
                if !value.is_empty() {
                    add(&["--message", value]);
                }
            }
            StashApply => add(&["stash", "apply", &revision()?]),
            CreateBranch | CreateSwitch | RenameBranch | DeleteBranch | Tag => {
                if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control)
                {
                    return Err("invalid reference name".into());
                }
                self.read(&[
                    "check-ref-format",
                    &format!(
                        "refs/{}/{}",
                        if op == Tag { "tags" } else { "heads" },
                        value
                    ),
                ])?;
                match op {
                    CreateBranch => add(&["branch", value]),
                    CreateSwitch => add(&["switch", "-c", value]),
                    RenameBranch => add(&["branch", "-m", value]),
                    DeleteBranch => add(&["branch", "-d", value]),
                    _ => add(&["tag", value]),
                }
            }
            Amend => add(&["commit", "--amend", "--no-edit"]),
            Fixup => add(&["commit", &format!("--fixup={}", revision()?)]),
            Merge => add(&["merge", "--no-edit", &revision()?]),
            Squash => add(&["merge", "--squash", &revision()?]),
            Rebase => add(&["rebase", &revision()?]),
            CherryPick => add(&["cherry-pick", &revision()?]),
            Revert => add(&["revert", "--no-edit", &revision()?]),
            _ => {
                let (command, flag) = match op {
                    MergeContinue => ("merge", "--continue"),
                    MergeAbort => ("merge", "--abort"),
                    RebaseContinue => ("rebase", "--continue"),
                    RebaseSkip => ("rebase", "--skip"),
                    RebaseAbort => ("rebase", "--abort"),
                    CherryContinue => ("cherry-pick", "--continue"),
                    CherrySkip => ("cherry-pick", "--skip"),
                    CherryAbort => ("cherry-pick", "--abort"),
                    RevertContinue => ("revert", "--continue"),
                    RevertSkip => ("revert", "--skip"),
                    RevertAbort => ("revert", "--abort"),
                    _ => unreachable!(),
                };
                if self.active_workflow()? != Some(command) {
                    return Err(format!("no {command} operation in progress"));
                }
                add(&[command, flag]);
            }
        }
        Ok(GitInvocation {
            repo: self.clone(),
            args: args.into_iter().map(OsString::from).collect(),
            input: None,
            draft: None,
            draft_stamp: None,
        })
    }
}
use std::os::unix::ffi::OsStringExt;
