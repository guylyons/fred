//! Structured, repository-anchored workflow commands.
use super::repo::{GitInvocation, Repo};
use std::ffi::OsString;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Stash,
    StashUntracked,
    StashStaged,
    StashKeepIndex,
    StashWorktree,
    SnapshotBoth,
    SnapshotIndex,
    SnapshotWorktree,
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
            Stash | StashUntracked | StashStaged | StashKeepIndex | StashWorktree => {
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
    /// Magit's stash-create plumbing: construct saved trees in a private index,
    /// publish the recoverable object before changing any worktree files.
    pub fn save_stash(
        &self,
        operation: Operation,
        message: &str,
        args: &[String],
    ) -> Result<(), String> {
        use Operation::*;
        if !matches!(
            operation,
            StashWorktree | SnapshotBoth | SnapshotIndex | SnapshotWorktree
        ) {
            return Err("invalid source stash operation".into());
        }
        if args
            .iter()
            .any(|a| a != "--all" && a != "--include-untracked")
        {
            return Err("invalid stash argument".into());
        }
        let snapshot = self.status()?;
        if snapshot.entries.iter().any(|e| e.conflict) {
            return Err("resolve index conflicts before saving a snapshot".into());
        }
        let worktree = operation != SnapshotIndex;
        let index = !matches!(operation, StashWorktree | SnapshotWorktree);
        let all = args.iter().any(|a| a == "--all");
        let untracked = worktree && !args.is_empty();
        let files = if untracked {
            if all {
                self.read(&["ls-files", "--others", "-z"])?
            } else {
                self.read(&["ls-files", "--others", "--exclude-standard", "-z"])?
            }
        } else {
            vec![]
        };
        if !snapshot.entries.iter().any(|e| {
            if worktree {
                e.unstaged || (index && e.staged)
            } else {
                e.staged
            }
        }) && files.is_empty()
        {
            return Err("No changes to save".into());
        }
        let head = String::from_utf8_lossy(&self.read(&["rev-parse", "--verify", "HEAD"])?)
            .trim()
            .to_owned();
        let summary = String::from_utf8_lossy(&self.read(&["log", "-1", "--format=%h %s"])?)
            .trim()
            .to_owned();
        let summary = format!("{}: {summary}", snapshot.branch);
        let message = if operation != StashWorktree {
            format!("WIP on {summary}")
        } else if message.is_empty() {
            format!("On {summary}")
        } else {
            message.to_owned()
        };
        let staged_tree = String::from_utf8_lossy(&self.read(&["write-tree"])?)
            .trim()
            .to_owned();
        let base = if index {
            head.clone()
        } else {
            self.stash_commit(&staged_tree, &[&head], "pre-stash index")?
        };
        let saved_index =
            self.stash_commit(&staged_tree, &[&base], &format!("index on {summary}"))?;
        let temporary = StashIndex::new()?;
        let run = |args: &[&str], input: Option<&[u8]>| {
            self.run_index(
                &args.iter().map(OsString::from).collect::<Vec<_>>(),
                input,
                Some(&temporary.0.join("index")),
            )
        };
        let mut parents = vec![base.as_str(), saved_index.as_str()];
        let untracked_commit;
        if !files.is_empty() {
            run(&["read-tree", "--empty"], None)?;
            run(
                &["update-index", "--add", "--remove", "-z", "--stdin"],
                Some(&files),
            )?;
            let tree = String::from_utf8_lossy(&run(&["write-tree"], None)?)
                .trim()
                .to_owned();
            untracked_commit =
                self.stash_commit(&tree, &[], &format!("untracked files on {summary}"))?;
            parents.push(&untracked_commit);
        }
        run(&["read-tree", &saved_index], None)?;
        if worktree {
            // Compare against the index we copied, including staged changes
            // reversed in the worktree (which a HEAD diff would omit).
            let files = self.read(&["diff", "--no-ext-diff", "--name-only", "-z", "--"])?;
            run(
                &["update-index", "--add", "--remove", "-z", "--stdin"],
                Some(&files),
            )?;
        }
        let tree = String::from_utf8_lossy(&run(&["write-tree"], None)?)
            .trim()
            .to_owned();
        let saved = self.stash_commit(&tree, &parents, &message)?;
        self.read(&[
            "update-ref",
            "--create-reflog",
            "-m",
            &message,
            "refs/stash",
            &saved,
        ])?;
        if operation == StashWorktree {
            // Upstream restores tracked files from the index, leaving it intact.
            self.read(&["checkout", "--", "."])?;
            if untracked {
                if all {
                    self.read(&["clean", "--force", "-d", "-x"])?;
                } else {
                    self.read(&["clean", "--force", "-d"])?;
                }
            }
        }
        Ok(())
    }
    fn stash_commit(&self, tree: &str, parents: &[&str], message: &str) -> Result<String, String> {
        let mut args: Vec<OsString> = ["-c", "commit.gpgsign=false", "commit-tree", tree]
            .into_iter()
            .map(OsString::from)
            .collect();
        for parent in parents {
            args.extend(["-p".into(), (*parent).into()]);
        }
        let id = self.run(&args, Some(message.as_bytes()))?;
        Ok(String::from_utf8_lossy(&id).trim().to_owned())
    }
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
            StashWorktree | SnapshotBoth | SnapshotIndex | SnapshotWorktree => {
                return Err("use source stash creation for this operation".into());
            }
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
            editor: false,
            after: None,
        })
    }
}
use std::os::unix::ffi::OsStringExt;

struct StashIndex(std::path::PathBuf);
impl StashIndex {
    fn new() -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "fred-stash-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
impl Drop for StashIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
