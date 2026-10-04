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
    /// Merges in the instant rebase range would be flattened: confirm.
    Merges(Box<Op>, String, bool),
    /// magit-commit-reshelve.
    Reshelve,
    /// magit-commit-absorb-modules.
    AbsorbModules,
    /// magit-commit-autofixup (needs git-autofixup).
    Autofixup,
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
            Op::StageAll(op, _) | Op::Published(op, ..) | Op::Merges(op, ..) => op.shape(),
            Op::Reshelve | Op::AbsorbModules | Op::Autofixup => ("", false, true, false),
        }
    }
    fn verb(&self) -> &'static str {
        match self {
            Op::Fixup | Op::InstantFixup => "Fixup",
            Op::Squash | Op::InstantSquash => "Squash",
            Op::Alter => "Alter",
            Op::Augment => "Augment",
            Op::Revise => "Revise",
            Op::StageAll(op, _) | Op::Published(op, ..) | Op::Merges(op, ..) => op.verb(),
            Op::Reshelve => "Reshelve",
            Op::AbsorbModules | Op::Autofixup => "Absorb into",
        }
    }
}

impl Repo {
    fn git_path_exists(&self, name: &str) -> bool {
        self.read(&["rev-parse", "--git-path", name])
            .map(|p| self.root.join(String::from_utf8_lossy(&p).trim()).exists())
            .unwrap_or(false)
    }
    pub fn commit_prompts(&self, op: &Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        if *op == Op::Reshelve {
            return self.reshelve_prompt();
        }
        if matches!(op, Op::AbsorbModules | Op::Autofixup) {
            // Commits since the upstream (its merge base for autofixup).
            let d = self
                .current_branch()
                .ok()
                .and_then(|b| self.upstream_of(&b))
                .unwrap_or_default();
            return (
                vec![format!("Absorb into commits since (default {d}): ")],
                vec![d],
            );
        }
        let d = at_point.unwrap_or_default();
        (
            vec![format!("{} commit (default {d}): ", op.verb())],
            vec![d],
        )
    }

    pub fn commit_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        match op {
            Op::Reshelve => return self.reshelve(at(0), args),
            Op::AbsorbModules => return self.absorb_modules(at(0).trim()),
            Op::Autofixup => {
                let since = at(0).trim();
                if since.is_empty() || since.starts_with('-') {
                    return Err(format!("invalid commit {since:?}"));
                }
                if self.read(&["autofixup", "--help"]).is_err() {
                    return Err("This command requires git-autofixup".into());
                }
                let base = self
                    .read(&["merge-base", "--end-of-options", since, "HEAD"])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .map_err(|_| format!("unknown commit {since:?}"))?;
                return Ok(Next::Git(vec!["autofixup".into(), "-vv".into(), base]));
            }
            _ => {}
        }
        let (op, target, args, confirmed) = match op {
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
            Op::Merges(op, target, all) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                let mut args = args.to_vec();
                if all {
                    args.push("--all".into());
                }
                return self.commit_now(*op, target, args);
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
        let (_, edit, nopatch, rebase) = op.shape();
        // magit-commit-assert.
        if !nopatch && !self.commit_ready(&args, !edit)? {
            return Ok(Next::Ask(
                Question::Commit(Op::StageAll(Box::new(op), id)),
                vec!["Nothing staged.  Commit all uncommitted changes? (y or n) ".into()],
                vec![String::new()],
            ));
        }
        if rebase {
            // An instant rebase must not run inside another operation: the
            // in-process commit would become a merge commit that autosquash drops.
            if self.merge_in_progress() || self.sequencer().is_some() || self.rebase_in_progress() {
                return Err("Finish the merge, sequence or rebase in progress first".into());
            }
            if self
                .read(&["merge-base", "--is-ancestor", &id, "HEAD"])
                .is_err()
            {
                return Err(format!("{target} isn't an ancestor of HEAD"));
            }
        }
        let all = args.iter().any(|x| x == "--all");
        // magit-rebase-interactive-assert: rewriting published history asks.
        if !confirmed {
            let published = self
                .read(&[
                    "branch",
                    "-r",
                    "--format=%(refname:short) %(objectname)",
                    "--contains",
                    &id,
                ])
                .map(|o| {
                    // delay-edit-confirm: branches exactly at the target are not
                    // modified by adding a fixup after it.
                    String::from_utf8_lossy(&o)
                        .lines()
                        .filter_map(|l| l.split_once(' '))
                        .filter(|(name, oid)| !name.ends_with("/HEAD") && *oid != id)
                        .map(|(name, _)| name.to_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !published.is_empty() {
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
        // Instant rebases would flatten merges in their range: ask first.
        if rebase
            && self
                .read(&["rev-list", "--merges", &format!("{id}..HEAD")])
                .is_ok_and(|o| !o.is_empty())
        {
            return Ok(Next::Ask(
                Question::Commit(Op::Merges(Box::new(op), id, all)),
                vec!["Proceed despite merge in rebase range? (y or n) ".into()],
                vec![String::new()],
            ));
        }
        self.commit_now(op, id, args)
    }
    /// Create the fixup/squash commit; instant variants then autosquash it.
    fn commit_now(&self, op: Op, id: String, mut args: Vec<String>) -> Result<Next, String> {
        let (option, edit, _, rebase) = op.shape();
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
        // magit-commit-assert checks MERGE_MSG (merges, cherry-picks, reverts).
        if self.git_path_exists("MERGE_MSG") {
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

impl Repo {
    /// magit-commit-reshelve: (prompt, default) for the new date.
    pub fn reshelve_prompt(&self) -> (Vec<String>, Vec<String>) {
        let verb = if self.author_is_me() {
            "Change author and committer dates to"
        } else {
            "Change committer date to"
        };
        (vec![format!("{verb} (default now): ")], vec!["now".into()])
    }
    /// magit-rev-author-p HEAD.
    fn author_is_me(&self) -> bool {
        let get = |a: &[&str]| {
            self.read(a)
                .ok()
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
        };
        let me = get(&["config", "user.email"]);
        me.is_some() && me == get(&["log", "-1", "--format=%ae", "HEAD"])
    }
    /// Change the committer (and, for your own commit, author) date of HEAD.
    pub fn reshelve(&self, date: &str, args: &[String]) -> Result<Next, String> {
        let date = date.trim();
        if date.is_empty() || date.starts_with('-') || date.chars().any(char::is_control) {
            return Err(format!("invalid date {date:?}"));
        }
        // GIT_COMMITTER_DATE has no "now": use Git's raw "<seconds> <zone>".
        let date = if date == "now" {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
            format!("{secs} +0000")
        } else {
            date.to_owned()
        };
        let mut argv = vec!["commit".to_owned(), "--amend".into(), "--no-edit".into()];
        if self.author_is_me() {
            argv.push(format!("--date={date}"));
        }
        // Only arguments that keep the commit's content and message.
        argv.extend(
            args.iter()
                .filter(|a| a.starts_with("--gpg-sign") || *a == "--no-verify")
                .cloned(),
        );
        let out = self
            .command()
            .env("GIT_COMMITTER_DATE", &date)
            .args(&argv)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        Ok(Next::Done(Ok("Reshelved HEAD".into())))
    }
    /// magit-commit-absorb-modules: a fixup commit per modified module,
    /// targeting the last commit since COMMIT that touched it.
    pub fn absorb_modules(&self, since: &str) -> Result<Next, String> {
        if since.is_empty() || since.starts_with('-') || since.chars().any(char::is_control) {
            return Err(format!("invalid commit {since:?}"));
        }
        self.read(&[
            "merge-base",
            "--is-ancestor",
            "--end-of-options",
            since,
            "HEAD",
        ])
        .map_err(|_| format!("{since} isn't an ancestor of HEAD"))?;
        let modules = self.module_paths()?;
        let modified: Vec<&String> = modules
            .iter()
            .filter(|m| self.read(&["diff", "--quiet", "HEAD", "--", m]).is_err())
            .collect();
        if modified.is_empty() {
            return Err("There are no modified modules that could be absorbed".into());
        }
        let mut made = 0;
        for m in modified {
            let subject =
                self.read(&["log", "-1", "--format=%s", &format!("{since}.."), "--", m])?;
            let subject = String::from_utf8_lossy(&subject).trim().to_owned();
            if subject.is_empty() {
                continue;
            }
            self.read(&[
                "commit",
                "-m",
                &format!("fixup! {subject}"),
                "--only",
                "--",
                m,
            ])?;
            made += 1;
        }
        Ok(Next::Done(Ok(format!("Created {made} fixup commits"))))
    }
}
