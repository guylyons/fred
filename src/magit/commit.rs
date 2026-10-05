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
    /// magit-commit-absorb (needs git-absorb).
    Absorb,
    /// magit-commit-create with nothing staged: commit everything (--all)?
    DraftAll,
    /// magit-reshelve-since: the first commit (picked in a log), then the
    /// date for it.
    ReshelveSince,
    ReshelveSinceDate(String),
    /// Nothing staged: confirm absorbing all unstaged changes.
    AbsorbAll(String),
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
            Op::Reshelve
            | Op::AbsorbModules
            | Op::Autofixup
            | Op::Absorb
            | Op::AbsorbAll(_)
            | Op::ReshelveSince
            | Op::ReshelveSinceDate(_)
            | Op::DraftAll => ("", false, true, false),
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
            Op::Reshelve | Op::ReshelveSince | Op::ReshelveSinceDate(_) => "Reshelve",
            Op::DraftAll => "Commit",
            Op::AbsorbModules | Op::Autofixup | Op::Absorb | Op::AbsorbAll(_) => "Absorb into",
        }
    }
    /// The transient whose arguments this suffix reads instead of magit-commit's.
    pub fn arg_menu(&self) -> Option<char> {
        match self {
            Op::Autofixup => Some('H'),
            Op::Absorb | Op::AbsorbAll(_) => Some('A'),
            // magit-reshelve-since signs with magit-rebase-arguments' key.
            Op::ReshelveSince | Op::ReshelveSinceDate(_) => Some('r'),
            _ => None,
        }
    }
}

/// magit-commit-ask-to-stage: None (nil) refuses, Some(false) (stage)
/// commits everything without asking, Some(true) (t, verbose) asks.
pub fn ask_to_stage() -> Option<bool> {
    match super::options::value("magit-commit-ask-to-stage") {
        Some(toml::Value::Boolean(false)) => None,
        Some(toml::Value::String(s)) if s == "stage" => Some(false),
        _ => Some(true),
    }
}
/// magit-git-executable-find: NAME on PATH or in git's exec path
/// ("git NAME --help" would open a man page instead).
fn git_exec_exists(repo: &Repo, name: &str) -> bool {
    let exec_path = repo
        .read(&["--exec-path"])
        .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
        .unwrap_or_default();
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([std::path::PathBuf::from(exec_path)])
        .any(|d| d.join(name).is_file())
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
        if matches!(op, Op::AbsorbModules | Op::Autofixup | Op::Absorb) {
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
            Op::ReshelveSince => {
                let commit = at(0).trim();
                if commit.is_empty() || commit.starts_with('-') {
                    return Err(format!("invalid commit {commit:?}"));
                }
                let current = self
                    .current_branch()
                    .map_err(|_| "Refusing to reshelve detached head".to_owned())?;
                let range = format!("{commit}^..{current}");
                let count: i64 = String::from_utf8_lossy(&self.read(&[
                    "rev-list",
                    "--count",
                    "--end-of-options",
                    &range,
                ])?)
                .trim()
                .parse()
                .unwrap_or(0);
                // The default: one minute per commit, ending now.
                let now = super::margin::now() - count * 60;
                let default = super::margin::strftime("%F %T %z", now);
                return Ok(Next::Ask(
                    Question::Commit(Op::ReshelveSinceDate(commit.to_owned())),
                    vec![format!("Date for first commit (default {default}): ")],
                    vec![default],
                ));
            }
            Op::ReshelveSinceDate(commit) => return self.reshelve_since(&commit, at(0), args),
            Op::AbsorbModules => return self.absorb_modules(at(0).trim()),
            Op::Autofixup | Op::Absorb => {
                let since = at(0).trim();
                if since.is_empty() || since.starts_with('-') {
                    return Err(format!("invalid commit {since:?}"));
                }
                let tool = if op == Op::Absorb {
                    "git-absorb"
                } else {
                    "git-autofixup"
                };
                if !git_exec_exists(self, tool) {
                    return Err(format!("This command requires {tool}"));
                }
                let base = self
                    .read(&["merge-base", "--end-of-options", since, "HEAD"])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .map_err(|_| format!("unknown commit {since:?}"))?;
                let staged = !self.ok(&["diff", "--cached", "--quiet"]);
                let unstaged = !self.ok(&["diff", "--quiet"]);
                if op == Op::Autofixup {
                    if !staged && !unstaged {
                        return Err("There are no changes that could be absorbed".into());
                    }
                    let mut argv = vec!["autofixup".to_owned()];
                    argv.extend(args.iter().cloned());
                    argv.push(base);
                    return Ok(Next::Git(argv));
                }
                if !staged {
                    if !unstaged {
                        return Err("There are no changes that could be absorbed".into());
                    }
                    return Ok(Next::Ask(
                        Question::Commit(Op::AbsorbAll(base)),
                        vec!["Nothing staged.  Absorb all unstaged changes? (y or n) ".into()],
                        vec![String::new()],
                    ));
                }
                let mut argv = vec!["absorb".to_owned()];
                argv.extend(args.iter().cloned());
                argv.extend(["-b".into(), base]);
                return Ok(Next::Git(argv));
            }
            Op::AbsorbAll(base) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                self.read(&["add", "-u", "--", ":/"])?;
                let mut argv = vec!["absorb".to_owned()];
                argv.extend(args.iter().cloned());
                argv.extend(["-b".into(), base]);
                return Ok(Next::Git(argv));
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
        // magit-commit-assert, with magit-commit-ask-to-stage.
        let mut args = args;
        if !nopatch && !self.commit_ready(&args, !edit)? {
            match ask_to_stage() {
                None => return Err("Nothing staged".into()),
                Some(false) => args.push("--all".into()),
                Some(true) => {}
            }
        }
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
                        // magit-list-publishing-branches: magit-published-branches only.
                        .filter(|(name, _)| {
                            super::options::strings("magit-published-branches", &["origin/master"])
                                .iter()
                                .any(|l| l == name)
                        })
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
    pub(crate) fn commit_ready(&self, args: &[String], strict: bool) -> Result<bool, String> {
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
        // magit-rev-author-p: same name or same email.
        let same = |me: &[&str], fmt: &str| {
            let me = get(me);
            me.is_some() && me == get(&["log", "-1", fmt, "HEAD"])
        };
        same(&["config", "user.name"], "--format=%an")
            || same(&["config", "user.email"], "--format=%ae")
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
            // The local zone, as upstream's %F %T %z default.
            let zone = self
                .read(&["var", "GIT_COMMITTER_IDENT"])
                .ok()
                .and_then(|o| {
                    String::from_utf8_lossy(&o)
                        .split_whitespace()
                        .last()
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "+0000".into());
            format!("{secs} {zone}")
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
    /// magit-reshelve-since: give the commits since COMMIT (inclusive) on the
    /// current branch consecutive dates a minute apart, from DATE. Upstream
    /// runs git filter-branch; Fred rewrites them with commit-tree.
    pub fn reshelve_since(
        &self,
        commit: &str,
        date: &str,
        args: &[String],
    ) -> Result<Next, String> {
        let date = date.trim();
        if date.is_empty() || date.starts_with('-') || date.chars().any(char::is_control) {
            return Err(format!("invalid date {date:?}"));
        }
        let current = self
            .current_branch()
            .map_err(|_| "Refusing to reshelve detached head".to_owned())?;
        // git's own date parser, through --since.
        let parsed =
            String::from_utf8_lossy(&self.read(&["rev-parse", &format!("--since={date}")])?)
                .trim()
                .to_owned();
        let mut secs: i64 = parsed
            .strip_prefix("--max-age=")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("invalid date {date:?}"))?;
        let zone = date
            .split_whitespace()
            .last()
            .filter(|z| (z.starts_with('+') || z.starts_with('-')) && z.len() == 5)
            .unwrap_or("+0000")
            .to_owned();
        let range = format!("{commit}^..{current}");
        let old_tip = String::from_utf8_lossy(&self.read(&["rev-parse", &current])?)
            .trim()
            .to_owned();
        let revs = String::from_utf8_lossy(&self.read(&[
            "rev-list",
            "--reverse",
            "--topo-order",
            "--end-of-options",
            &range,
        ])?)
        .into_owned();
        let sign = args.iter().find(|a| a.starts_with("--gpg-sign"));
        let committer_only = super::options::flag("magit-reshelve-since-committer-only", false);
        let mut map: std::collections::HashMap<String, String> = Default::default();
        let mut tip = old_tip.clone();
        for rev in revs.lines() {
            let info = String::from_utf8_lossy(&self.read(&[
                "log",
                "-1",
                "--format=%T%x00%P%x00%an%x00%ae%x00%ad%x00%cn%x00%ce",
                "--date=raw",
                rev,
            ])?)
            .into_owned();
            let f: Vec<&str> = info.trim_end().split('\0').collect();
            let [tree, parents, an, ae, ad, cn, ce] = f[..] else {
                return Err(format!("cannot read {rev}"));
            };
            let message = self.read(&["log", "-1", "--format=%B", rev])?;
            let mut argv: Vec<String> = vec!["commit-tree".into()];
            for p in parents.split_whitespace() {
                argv.extend([
                    "-p".into(),
                    map.get(p).cloned().unwrap_or_else(|| p.to_owned()),
                ]);
            }
            if let Some(s) = sign {
                argv.push(s.replacen("--gpg-sign", "-S", 1).replacen("-S=", "-S", 1));
            }
            argv.push(tree.into());
            let stamp = format!("{secs} {zone}");
            let author_date = if committer_only {
                ad.to_owned()
            } else {
                stamp.clone()
            };
            let out = self
                .command()
                .env("GIT_AUTHOR_NAME", an)
                .env("GIT_AUTHOR_EMAIL", ae)
                .env("GIT_AUTHOR_DATE", &author_date)
                .env("GIT_COMMITTER_NAME", cn)
                .env("GIT_COMMITTER_EMAIL", ce)
                .env("GIT_COMMITTER_DATE", &stamp)
                .args(&argv)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    child
                        .stdin
                        .take()
                        .expect("piped stdin")
                        .write_all(&message)?;
                    child.wait_with_output()
                })
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
            }
            let new = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            map.insert(rev.to_owned(), new.clone());
            tip = new;
            secs += 60;
        }
        self.read(&[
            "update-ref",
            "-m",
            "reshelve",
            &format!("refs/heads/{current}"),
            &tip,
            &old_tip,
        ])?;
        Ok(Next::Done(Ok(format!("Reshelved {} commits", map.len()))))
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
            // Only a moved gitlink (submodule status "+"), not dirty contents.
            .filter(|m| {
                self.read(&[
                    "diff",
                    "--quiet",
                    "--ignore-submodules=dirty",
                    "HEAD",
                    "--",
                    m,
                ])
                .is_err()
            })
            .collect();
        if modified.is_empty() {
            return Err("There are no modified modules that could be absorbed".into());
        }
        let (mut made, mut failed) = (0, vec![]);
        for m in modified {
            let subject =
                self.read(&["log", "-1", "--format=%s", &format!("{since}.."), "--", m])?;
            let subject = String::from_utf8_lossy(&subject).trim().to_owned();
            if subject.is_empty() {
                continue;
            }
            if let Err(e) = self.read(&[
                "commit",
                "-m",
                &format!("fixup! {subject}"),
                "--only",
                "--",
                m,
            ]) {
                failed.push(format!("{m}: {e}"));
                continue;
            }
            made += 1;
        }
        if !failed.is_empty() {
            return Err(format!(
                "Created {made} fixup commits; failed: {}",
                failed.join("; ")
            ));
        }
        Ok(Next::Done(Ok(format!("Created {made} fixup commits"))))
    }
}
