//! magit-commit.el fixup/squash family ("Edit" and "Edit and rebase").
use super::Question;
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Fixup,
    Squash,
    Alter,
    Augment,
    Revise,
    InstantFixup,
    InstantSquash,
    /// Nothing staged: confirm committing everything (--all).
    StageAll(Box<Op>, String),
    /// The target is published: confirm rewriting it.
    Published(Box<Op>, String, bool),
}

impl Op {
    /// (git option, edit message, no patch needed, rebase immediately).
    fn shape(&self) -> (&'static str, bool, bool, bool) {
        match self {
            Op::Fixup => ("--fixup=", false, false, false),
            Op::Squash => ("--squash=", false, false, false),
            Op::Alter => ("--fixup=amend:", true, false, false),
            Op::Augment => ("--squash=", true, false, false),
            Op::Revise => ("--fixup=reword:", true, true, false),
            Op::InstantFixup => ("--fixup=", false, false, true),
            Op::InstantSquash => ("--squash=", false, false, true),
            Op::StageAll(op, _) | Op::Published(op, ..) => op.shape(),
        }
    }
    fn verb(&self) -> &'static str {
        match self {
            Op::Fixup | Op::InstantFixup => "Fixup",
            Op::Squash | Op::InstantSquash => "Squash",
            Op::Alter => "Alter",
            Op::Augment => "Augment",
            Op::Revise => "Revise",
            Op::StageAll(op, _) | Op::Published(op, ..) => op.verb(),
        }
    }
}

impl Repo {
    pub fn commit_prompts(&self, op: &Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        let d = at_point.unwrap_or_default();
        (
            vec![format!("{} commit (default {d}): ", op.verb())],
            vec![d],
        )
    }

    pub fn commit_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let (op, target, mut args, confirmed) = match op {
            Op::StageAll(op, target) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                let mut args = args.to_vec();
                args.push("--all".into());
                (*op, target, args, false)
            }
            Op::Published(op, target, all) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                let mut args = args.to_vec();
                if all {
                    args.push("--all".into());
                }
                (*op, target, args, true)
            }
            op => (op, at(0).to_owned(), args.to_vec(), false),
        };
        if target.is_empty() || target.starts_with('-') || target.chars().any(char::is_control) {
            return Err(format!("invalid commit {target:?}"));
        }
        let id = String::from_utf8_lossy(
            &self
                .read(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    "--end-of-options",
                    &format!("{target}^{{commit}}"),
                ])
                .map_err(|_| format!("unknown commit {target:?}"))?,
        )
        .trim()
        .to_owned();
        let (option, edit, nopatch, rebase) = op.shape();
        // magit-commit-assert.
        if !nopatch && !self.commit_ready(&args, !edit)? {
            return Ok(Next::Ask(
                Question::Commit(Op::StageAll(Box::new(op), id)),
                vec!["Nothing staged.  Commit all uncommitted changes? (y or n) ".into()],
                vec![String::new()],
            ));
        }
        if rebase
            && self
                .read(&["merge-base", "--is-ancestor", &id, "HEAD"])
                .is_err()
        {
            return Err(format!("{target} isn't an ancestor of HEAD"));
        }
        // magit-rebase-interactive-assert: rewriting published history asks.
        if !confirmed {
            let published = self
                .read(&[
                    "branch",
                    "-r",
                    "--format=%(refname:short)",
                    "--contains",
                    &id,
                ])
                .map(|o| {
                    String::from_utf8_lossy(&o)
                        .lines()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !published.is_empty() {
                let all = args.iter().any(|x| x == "--all");
                return Ok(Next::Ask(
                    Question::Commit(Op::Published(Box::new(op), id, all)),
                    vec![format!(
                        "Some of these commits have already been published to {}.  Do you really want to modify them? (y or n) ",
                        if published.len() == 1 {
                            published[0].clone()
                        } else {
                            format!("{} public branches", published.len())
                        }
                    )],
                    vec![String::new()],
                ));
            }
        }
        args.retain(|x| x != "--");
        let mut commit = vec!["commit".to_owned()];
        if rebase {
            args.retain(|x| !x.starts_with("--gpg-sign"));
            commit.push("--no-gpg-sign".into());
        }
        commit.extend(args);
        commit.push(format!("{option}{id}"));
        commit.push(if edit { "--edit" } else { "--no-edit" }.into());
        if !rebase {
            return Ok(Next::GitEditor(commit));
        }
        // Edit and rebase: commit now, then autosquash it into its target.
        let argv: Vec<std::ffi::OsString> = commit.into_iter().map(Into::into).collect();
        self.run(&argv, None)?;
        let parent = self
            .read(&["rev-parse", "--verify", "-q", &format!("{id}^")])
            .ok();
        let mut rebase = vec![
            "-c".to_owned(),
            "sequence.editor=true".into(),
            "rebase".into(),
            "-i".into(),
            "--autosquash".into(),
            "--autostash".into(),
            "--keep-empty".into(),
        ];
        match parent {
            Some(p) => rebase.extend([
                "--end-of-options".into(),
                String::from_utf8_lossy(&p).trim().to_owned(),
            ]),
            None => rebase.push("--root".into()),
        }
        Ok(Next::GitEditor(rebase))
    }
    /// Something to commit: staged changes, or unstaged ones with --all, or
    /// (for non-strict variants) an argument that makes an empty commit useful.
    fn commit_ready(&self, args: &[String], strict: bool) -> Result<bool, String> {
        let staged = self.read(&["diff", "--cached", "--quiet"]).is_err();
        let unstaged = self.read(&["diff", "--quiet"]).is_err();
        if staged || (unstaged && args.iter().any(|a| a == "--all")) {
            return Ok(true);
        }
        if !strict
            && args.iter().any(|a| {
                a == "--allow-empty"
                    || a == "--reset-author"
                    || a == "--signoff"
                    || a.starts_with("--author=")
            })
        {
            return Ok(true);
        }
        if self.merge_in_progress() {
            let unmerged = self
                .read(&["ls-files", "--unmerged"])
                .is_ok_and(|o| !o.is_empty());
            if unmerged {
                return Err("Unresolved conflicts".into());
            }
            return Ok(true);
        }
        if !unstaged {
            return Err("Nothing staged (or unstaged)".into());
        }
        Ok(false)
    }
}
