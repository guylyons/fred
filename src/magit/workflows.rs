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
            expected_head: None,
            repo: self.clone(),
            args: args.into_iter().map(OsString::from).collect(),
            input: None,
            draft: None,
            draft_stamp: None,
        })
    }
}
use std::os::unix::ffi::OsStringExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub id: String,
    pub selector: String,
    pub subject: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashAction {
    Apply,
    Pop,
    Drop,
}
impl Repo {
    pub fn stashes(&self) -> Result<Vec<Stash>, String> {
        let bytes = self.read(&["stash", "list", "--format=%H%x00%gd%x00%gs"])?;
        bytes
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|line| {
                let mut fields = line.splitn(3, |b| *b == 0);
                let id = String::from_utf8_lossy(fields.next().ok_or("invalid stash listing")?)
                    .into_owned();
                let selector =
                    String::from_utf8_lossy(fields.next().ok_or("invalid stash listing")?)
                        .into_owned();
                let subject =
                    String::from_utf8_lossy(fields.next().ok_or("invalid stash listing")?)
                        .into_owned();
                Ok(Stash {
                    id,
                    selector,
                    subject,
                })
            })
            .collect()
    }
    fn check_stash(&self, stash: &Stash) -> Result<(), String> {
        if self
            .stashes()?
            .iter()
            .any(|s| s.selector == stash.selector && s.id == stash.id)
        {
            Ok(())
        } else {
            Err("stash list changed; refresh and select again".into())
        }
    }
    pub fn stash_patch(&self, stash: &Stash) -> Result<Vec<u8>, String> {
        let mut patch = format!("{} {}\n", stash.selector, stash.subject).into_bytes();
        if let Ok(notes) = self.read(&["notes", "show", &stash.id]) {
            patch.extend_from_slice(b"\nNotes\n");
            patch.extend(notes);
        }
        for (heading, from, to) in [
            ("Unstaged", format!("{}^2", stash.id), stash.id.clone()),
            (
                "Staged",
                format!("{}^1", stash.id),
                format!("{}^2", stash.id),
            ),
        ] {
            patch.extend_from_slice(format!("\n{heading}\n").as_bytes());
            patch.extend(self.read(&[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                &from,
                &to,
                "--",
            ])?);
        }
        let untracked = format!("{}^3", stash.id);
        if self.read(&["rev-parse", "--verify", &untracked]).is_ok() {
            patch.extend_from_slice(b"\nUntracked files\n");
            patch.extend(self.read(&[
                "show",
                "--format=",
                "--root",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                &untracked,
                "--",
            ])?);
        }
        Ok(patch)
    }
    pub fn stash_action(&self, stash: &Stash, action: StashAction) -> Result<&'static str, String> {
        if action != StashAction::Apply {
            self.check_stash(stash)?;
        }
        if action != StashAction::Drop {
            // Restore the saved index as Magit's normal apply/pop path does.
            // Failed application retains the stash, including installed conflicts.
            if let Err(index_error) = self.read(&["stash", "apply", "--index", &stash.id]) {
                // An installed conflict is already a useful result. Do not apply again.
                if self.status()?.entries.iter().any(|e| e.conflict) {
                    return Err(index_error);
                }
                match self.read(&["stash", "apply", &stash.id]) {
                    Ok(_) => return Ok("Stash applied without its saved index; stash retained"),
                    Err(error) => return Err(format!("{index_error}\n{error}\nStash retained")),
                }
            }
        }
        if action != StashAction::Apply {
            // Applying may take time; verify the reflog position again before removing it.
            self.check_stash(stash)?;
            self.read(&["stash", "drop", &stash.selector])?;
        }
        Ok(match action {
            StashAction::Apply => "Stash applied; saved index restored",
            StashAction::Pop => "Stash popped; saved index restored",
            StashAction::Drop => "Stash dropped",
        })
    }
}
