//! magit-branch.el suffixes.
use super::repo::{Repo, label};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// l: existing local, remote-tracking (create + pushRemote) or new name.
    CheckoutLocal,
    /// l with a new name: read the starting point.
    CheckoutNew(String),
    /// n
    Create,
    /// c
    CreateCheckout,
    /// s / S
    Spinoff,
    Spinout,
    /// m
    Rename,
    /// x, and its confirmation for a dirty current branch.
    Reset,
    /// x after the branch: read the target, defaulting to that branch's upstream.
    ResetTo(String),
    ResetConfirmed(String, String),
    /// k, and its follow-up questions.
    Delete,
    DeleteRemote(String),
    DeleteCurrent(String),
    /// Current branch, checkout target (None detaches): confirm unmerged deletion.
    DeleteCurrentUnmerged(String, Option<String>),
    DeleteUnmerged(String),
}

/// What the session should do after a branch step.
#[derive(Debug)]
pub enum Next {
    Done(Result<String, String>),
    Ask(super::Question, Vec<String>, Vec<String>),
    /// A network command for the terminal (delete on a remote).
    Git(Vec<String>),
    /// Open Fred's commit draft with this message (merge --edit).
    Draft(Vec<u8>),
    /// Show a diff (merge preview).
    Show(super::diff::Target),
    /// A terminal Git command that may open an editor (Fred, like with-editor).
    GitEditor(Vec<String>),
    /// An interactive rebase's captured todo list to edit.
    Todo(super::rebase::Plan),
    /// Run an interactive rebase with its prepared todo list.
    Replay(super::rebase::Plan),
    /// Show the status of another worktree.
    Status(std::path::PathBuf),
}

fn name(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid name {v:?}"));
    }
    Ok(v)
}

impl Repo {
    fn ok(&self, args: &[&str]) -> bool {
        self.read(args).is_ok()
    }
    fn git(&self, args: &[&str]) -> Result<(), String> {
        self.read(args).map(|_| ())
    }
    fn local_branch(&self, b: &str) -> bool {
        self.ok(&["show-ref", "--verify", "-q", &format!("refs/heads/{b}")])
    }
    fn commitish(&self, rev: &str) -> Result<String, String> {
        let rev = name(rev)?;
        self.read(&[
            "rev-parse",
            "--verify",
            "-q",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ])
        .map(|_| rev.to_owned())
        .map_err(|_| format!("Not a valid starting-point: {rev}"))
    }
    /// magit-branch--read-name: valid and not taken.
    fn new_branch(&self, b: &str) -> Result<String, String> {
        let b = name(b)?;
        if self.local_branch(b) {
            return Err(format!("Cannot create {b}; it already exists"));
        }
        self.git(&["check-ref-format", "--branch", b])
            .map_err(|_| format!("{b:?} is not a valid branch name"))?;
        Ok(b.to_owned())
    }
    fn modified(&self) -> bool {
        !self.ok(&["diff", "--quiet"]) || !self.ok(&["diff", "--cached", "--quiet"])
    }
    fn upstream_of(&self, b: &str) -> Option<String> {
        let out = self
            .read(&["rev-parse", "--abbrev-ref", &format!("{b}@{{upstream}}")])
            .ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_owned()).filter(|s| !s.is_empty())
    }
    /// magit-split-branch-name for remote-tracking names: the longest known remote.
    fn remote_split(&self, name: &str) -> Option<(String, String)> {
        let mut remotes = self.remotes().ok()?;
        remotes.sort_by_key(|r| std::cmp::Reverse(r.len()));
        remotes.into_iter().find_map(|r| {
            let rest = name.strip_prefix(&r)?.strip_prefix('/')?;
            (!rest.is_empty()).then(|| (r.clone(), rest.to_owned()))
        })
    }
    /// magit-branch-merged-p: merged into its upstream (if any) and into TARGET;
    /// None means the current branch (false when detached), Some("") any other
    /// local branch.
    fn merged(&self, branch: &str, target: Option<&str>) -> bool {
        let reference = format!("refs/heads/{branch}");
        if let Some(u) = self.upstream_of(branch)
            && !self.ok(&["merge-base", "--is-ancestor", &reference, &u])
        {
            return false;
        }
        match target {
            Some("") => self
                .read(&[
                    "for-each-ref",
                    "--format=%(refname:short)",
                    "--contains",
                    &reference,
                    "refs/heads/",
                ])
                .is_ok_and(|o| String::from_utf8_lossy(&o).lines().any(|b| b != branch)),
            Some(t) => self.ok(&["merge-base", "--is-ancestor", &reference, t]),
            None => match self.current_branch() {
                Ok(c) => self.ok(&[
                    "merge-base",
                    "--is-ancestor",
                    &reference,
                    &format!("refs/heads/{c}"),
                ]),
                Err(_) => false,
            },
        }
    }
    /// magit-get-indirect-upstream-branch with FORCE, else magit-main-branch.
    fn delete_target(&self, branch: &str) -> Option<String> {
        let indirect = self.upstream_of(branch).filter(|u| {
            self.config(&format!("branch.{branch}.remote"))
                .is_some_and(|r| r != "." && u.strip_prefix(&format!("{r}/")) == Some(branch))
                && self.ok(&[
                    "merge-base",
                    "--is-ancestor",
                    u,
                    &format!("refs/heads/{branch}"),
                ])
        });
        let main = || {
            self.config("init.defaultBranch")
                .into_iter()
                .chain(["main", "master", "trunk", "development"].map(String::from))
                .find(|b| self.local_branch(b))
        };
        indirect.or_else(main).filter(|t| t != branch)
    }
    /// git reset --hard silently replaces untracked files the target tracks.
    pub(super) fn untracked_clobbered(&self, to: &str) -> Result<(), String> {
        use std::os::unix::ffi::OsStrExt;
        let index = self.read(&["ls-files", "-z"])?;
        let index: std::collections::HashSet<&[u8]> = index.split(|b| *b == 0).collect();
        let target = self.read(&["ls-tree", "-r", "-z", "--name-only", "--end-of-options", to])?;
        let refuse = |p: &[u8]| {
            Err(format!(
                "Untracked {} would be overwritten; move it first",
                label(Path::new(&String::from_utf8_lossy(p).into_owned()))
            ))
        };
        for t in target.split(|b| *b == 0).filter(|t| !t.is_empty()) {
            let path = self.root.join(std::ffi::OsStr::from_bytes(t));
            // The target's file replaces an untracked file, or a directory that
            // holds untracked files.
            if let Ok(meta) = path.symlink_metadata() {
                if meta.is_dir() {
                    let mut args: Vec<std::ffi::OsString> = vec![
                        "ls-files".into(),
                        "--others".into(),
                        "-z".into(),
                        "--".into(),
                    ];
                    args.push(std::ffi::OsStr::from_bytes(t).to_owned());
                    if !self.run(&args, None)?.is_empty() {
                        return refuse(t);
                    }
                } else if !index.contains(t) {
                    return refuse(t);
                }
            }
            // The target's directory replaces an untracked file.
            for (n, _) in t.iter().enumerate().filter(|(_, b)| **b == b'/') {
                let ancestor = &t[..n];
                let p = self.root.join(std::ffi::OsStr::from_bytes(ancestor));
                if p.symlink_metadata().is_ok_and(|m| !m.is_dir()) && !index.contains(ancestor) {
                    return refuse(ancestor);
                }
            }
        }
        Ok(())
    }
    /// Local and remote-tracking branch names for completion.
    pub fn branch_choices(&self) -> Vec<String> {
        self.read(&[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/",
            "refs/remotes/",
        ])
        .map(|o| {
            String::from_utf8_lossy(&o)
                .lines()
                .filter(|l| !l.ends_with("/HEAD"))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
    }

    /// Prompts and defaults for a menu suffix.
    pub fn branch_prompts(&self, op: &Op) -> Result<(Vec<String>, Vec<String>), String> {
        let current = self.current_branch().ok();
        let here = current.clone().unwrap_or_else(|| "HEAD".into());
        let previous = self
            .read(&["rev-parse", "--abbrev-ref", "@{-1}"])
            .ok()
            .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
            .filter(|b| !b.is_empty() && self.local_branch(b))
            .unwrap_or_default();
        let two = |a: &str, b: String| {
            (
                vec![a.to_owned(), format!("{b}: ")],
                vec![String::new(), here.clone()],
            )
        };
        Ok(match op {
            Op::CheckoutLocal => (vec!["Checkout branch: ".into()], vec![String::new()]),
            Op::Create => two(
                "Create branch named: ",
                format!("Create branch starting at (default {here})"),
            ),
            Op::CreateCheckout => two(
                "Create and checkout branch named: ",
                format!("Create and checkout branch starting at (default {here})"),
            ),
            Op::Spinoff => (vec!["Spin off branch: ".into()], vec![String::new()]),
            Op::Spinout => (vec!["Spin out branch: ".into()], vec![String::new()]),
            Op::Rename => (
                vec![
                    format!("Rename branch (default {here}): "),
                    "Rename branch to: ".into(),
                ],
                vec![here.clone(), String::new()],
            ),
            Op::Reset => (
                vec![format!("Reset branch (default {here}): ")],
                vec![here.clone()],
            ),
            Op::Delete => (
                vec![format!("Delete branch (default {previous}): ")],
                vec![previous],
            ),
            _ => return Err("not a menu suffix".into()),
        })
    }

    /// Run one answered step: `a` holds answers with empty ones defaulted,
    /// `defaults` the step's own defaults.
    pub fn branch_step(&self, op: Op, a: &[String], defaults: &[String]) -> Next {
        match self.branch_step_inner(op, a, defaults) {
            Ok(next) => next,
            Err(e) => Next::Done(Err(e)),
        }
    }
    fn branch_step_inner(&self, op: Op, a: &[String], defaults: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let done = |msg: String| Ok(Next::Done(Ok(msg)));
        // ponytail: -m/-r (levels 6/7) are not offered yet, so checkout takes no arguments.
        let checkout_args: Vec<&str> = vec![];
        match op {
            Op::CheckoutLocal => {
                let choice = name(at(0))?;
                if self.local_branch(choice) {
                    let mut argv = vec!["checkout"];
                    argv.extend(&checkout_args);
                    argv.extend([choice, "--"]);
                    self.git(&argv)?;
                    return done(format!("Checked out {choice}"));
                }
                if self.ok(&[
                    "show-ref",
                    "--verify",
                    "-q",
                    &format!("refs/remotes/{choice}"),
                ]) && let Some((remote, branch)) = self.remote_split(choice)
                    && !self.local_branch(&branch)
                {
                    if self.modified() {
                        return Err("Cannot checkout when there are uncommitted changes".into());
                    }
                    let (remote, branch) = (remote.as_str(), branch.as_str());
                    self.create_checkout(branch, choice, &checkout_args)?;
                    // magit-branch-checkout: a remote branch of the same name is the push target.
                    if self.config("remote.pushDefault").as_deref() != Some(remote) {
                        self.git(&["config", &format!("branch.{branch}.pushRemote"), remote])?;
                    }
                    return done(format!("Created and checked out {branch}"));
                }
                let new = self.new_branch(choice)?;
                let here = self.current_branch().unwrap_or_else(|_| "HEAD".into());
                Ok(Next::Ask(
                    super::Question::Branch(Op::CheckoutNew(new.clone())),
                    vec![format!("Create {new} starting at (default {here}): ")],
                    vec![here],
                ))
            }
            Op::CheckoutNew(new) => {
                let start = self.commitish(at(0))?;
                if self.modified() {
                    return Err("Cannot checkout when there are uncommitted changes".into());
                }
                self.create_checkout(&new, &start, &checkout_args)?;
                done(format!("Created and checked out {new}"))
            }
            Op::Create | Op::CreateCheckout => {
                let new = self.new_branch(at(0))?;
                let start = at(1);
                // magit-branch-and-checkout: a stash start point uses git stash branch.
                if op == Op::CreateCheckout
                    && start.starts_with("stash@{")
                    && start.ends_with('}')
                    && start[7..start.len() - 1]
                        .bytes()
                        .all(|b| b.is_ascii_digit())
                {
                    self.git(&["stash", "branch", &new, start])?;
                    return done(format!("Created {new} from {start}"));
                }
                let start = self.commitish(start)?;
                if op == Op::Create {
                    self.git(&["branch", &new, &start])?;
                    done(format!("Created {new}"))
                } else {
                    self.create_checkout(&new, &start, &checkout_args)?;
                    done(format!("Created and checked out {new}"))
                }
            }
            Op::Spinoff | Op::Spinout => {
                let new = self.new_branch(at(0))?;
                self.spinoff(&new, op == Op::Spinoff).and_then(done)
            }
            Op::Rename => {
                // Validate after stripping, so "heads/-M" cannot become an option.
                let old = at(0);
                let old = name(old.strip_prefix("heads/").unwrap_or(old))?.to_owned();
                let new = name(at(1))?;
                if old == new {
                    return Err("Old and new branch names are the same".into());
                }
                if self.local_branch(new) {
                    return Err(format!("Branch `{new}' already exists"));
                }
                let push = self.config(&format!("branch.{old}.pushRemote"));
                self.git(&["branch", "-m", &old, new])?;
                if let Some(remote) = push {
                    // Git moves branch.<old>.* to branch.<new>.*; keep the push target.
                    self.git(&["config", &format!("branch.{new}.pushRemote"), &remote])?;
                }
                done(format!("Renamed {old} to {new}"))
            }
            Op::Reset => {
                let branch = name(at(0))?.to_owned();
                if !self.local_branch(&branch) {
                    return Err(format!("{branch} is not a local branch"));
                }
                let upstream = self.upstream_of(&branch).unwrap_or_default();
                Ok(Next::Ask(
                    super::Question::Branch(Op::ResetTo(branch.clone())),
                    vec![format!("Reset {branch} to (default {upstream}): ")],
                    vec![upstream],
                ))
            }
            Op::ResetTo(branch) => {
                let to = self.commitish(at(0))?;
                if self.current_branch().ok().as_deref() == Some(branch.as_str()) && self.modified()
                {
                    return Ok(Next::Ask(
                        super::Question::Branch(Op::ResetConfirmed(branch, to)),
                        vec!["Uncommitted changes will be lost.  Proceed? (yes or no) ".into()],
                        vec![String::new()],
                    ));
                }
                self.reset_branch(&branch, &to).and_then(done)
            }
            Op::ResetConfirmed(branch, to) => {
                if at(0) != "yes" {
                    return Err("Abort".into());
                }
                self.reset_branch(&branch, &to).and_then(done)
            }
            Op::Delete => {
                let branch = name(at(0))?.to_owned();
                if !self.local_branch(&branch) {
                    if self.ok(&[
                        "show-ref",
                        "--verify",
                        "-q",
                        &format!("refs/remotes/{branch}"),
                    ]) && let Some((remote, _)) = self.remote_split(&branch)
                    {
                        return Ok(Next::Ask(
                            super::Question::Branch(Op::DeleteRemote(branch.clone())),
                            vec![format!(
                                "Deleting local refs/remotes/{branch}.  Also delete on {remote}? (y or n) "
                            )],
                            vec![String::new()],
                        ));
                    }
                    return Err(format!("No branch named {branch}"));
                }
                if self.current_branch().ok().as_deref() == Some(branch.as_str()) {
                    let target = self.delete_target(&branch);
                    let choices = match &target {
                        Some(t) => {
                            format!("[d]etach HEAD & delete, [c]heckout {t} & delete, [a]bort ")
                        }
                        None => "[d]etach HEAD & delete, [a]bort ".into(),
                    };
                    return Ok(Next::Ask(
                        super::Question::Branch(Op::DeleteCurrent(branch.clone())),
                        vec![format!("Branch {branch} is checked out.  {choices}")],
                        vec![target.unwrap_or_default()],
                    ));
                }
                if !self.merged(&branch, None) {
                    return Ok(Next::Ask(
                        super::Question::Branch(Op::DeleteUnmerged(branch.clone())),
                        vec![format!("Delete unmerged branch {branch}? (y or n) ")],
                        vec![String::new()],
                    ));
                }
                self.git(&["branch", "-d", &branch])?;
                self.unset_push_remote(&branch);
                done(format!("Deleted {branch}"))
            }
            Op::DeleteRemote(branch) => {
                let (remote, name) = self.remote_split(&branch).ok_or("not a remote branch")?;
                if matches!(at(0), "y" | "yes") {
                    return Ok(Next::Git(vec![
                        "push".into(),
                        "--delete".into(),
                        remote,
                        format!("refs/heads/{name}"),
                    ]));
                }
                self.git(&["update-ref", "-d", &format!("refs/remotes/{branch}")])?;
                done(format!("Deleted refs/remotes/{branch}"))
            }
            // Upstream confirms an unmerged branch before leaving it.
            Op::DeleteCurrent(branch) => {
                let target = match at(0) {
                    "d" => None,
                    "c" if defaults.first().is_some_and(|t| !t.is_empty()) => {
                        defaults.first().cloned()
                    }
                    _ => return Err("Abort".into()),
                };
                if !self.merged(&branch, Some(target.as_deref().unwrap_or(""))) {
                    return Ok(Next::Ask(
                        super::Question::Branch(Op::DeleteCurrentUnmerged(branch.clone(), target)),
                        vec![format!("Delete unmerged branch {branch}? (y or n) ")],
                        vec![String::new()],
                    ));
                }
                self.leave_and_delete(&branch, target.as_deref())
                    .and_then(done)
            }
            Op::DeleteCurrentUnmerged(branch, target) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                self.leave_and_delete(&branch, target.as_deref())
                    .and_then(done)
            }
            Op::DeleteUnmerged(branch) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                self.git(&["branch", "-D", &branch])?;
                self.unset_push_remote(&branch);
                done(format!("Deleted {branch}"))
            }
        }
    }
    fn create_checkout(&self, new: &str, start: &str, args: &[&str]) -> Result<(), String> {
        let mut argv = vec!["checkout"];
        argv.extend(args);
        argv.extend(["-b", new, start, "--"]);
        self.git(&argv)
    }
    fn unset_push_remote(&self, branch: &str) {
        let _ = self.git(&["config", "--unset", &format!("branch.{branch}.pushRemote")]);
    }
    fn leave_and_delete(&self, branch: &str, target: Option<&str>) -> Result<String, String> {
        match target {
            Some(t) => self.git(&["checkout", t, "--"])?,
            None => self.git(&["checkout", "--detach"])?,
        }
        self.git(&["branch", "-D", branch])?;
        self.unset_push_remote(branch);
        Ok(format!("Deleted {branch}"))
    }
    fn reset_branch(&self, branch: &str, to: &str) -> Result<String, String> {
        if self.current_branch().ok().as_deref() == Some(branch) {
            self.untracked_clobbered(to)?;
            self.git(&["reset", "--hard", to, "--"])?;
        } else {
            self.git(&[
                "update-ref",
                "-m",
                &format!("reset: moving to {to}"),
                &format!("refs/heads/{branch}"),
                to,
            ])?;
        }
        Ok(format!("Reset {branch} to {}", label(Path::new(to))))
    }
    /// magit--branch-spinoff.
    fn spinoff(&self, new: &str, checkout: bool) -> Result<String, String> {
        let checkout = checkout || self.modified();
        let Ok(current) = self.current_branch() else {
            if checkout {
                self.git(&["checkout", "-b", new])?;
            } else {
                self.git(&["branch", new])?;
            }
            return Ok(format!("Created {new}"));
        };
        let tracked = self.upstream_of(&current);
        // Refuse before changing anything if the hard reset would clobber files.
        if !checkout && let Some(t) = &tracked {
            let base = self
                .read(&["merge-base", &current, t])
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())?;
            self.untracked_clobbered(&base)?;
        }
        if checkout {
            self.git(&["checkout", "-b", new, &current, "--"])?;
        } else {
            self.git(&["branch", new, &current])?;
        }
        // ponytail: magit-branch-prefer-remote-upstream is nil by default, so the
        // indirect upstream is unset and tracking follows branch.autoSetupMerge.
        if let Some(tracked) = tracked {
            let base = self
                .read(&["merge-base", &current, &tracked])
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())?;
            let tip = self
                .read(&["rev-parse", &format!("refs/heads/{current}")])
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())?;
            if base != tip {
                if checkout {
                    self.git(&[
                        "update-ref",
                        "-m",
                        &format!("reset: moving to {base}"),
                        &format!("refs/heads/{current}"),
                        &base,
                    ])?;
                } else {
                    self.untracked_clobbered(&base)?;
                    self.git(&["reset", "--hard", &base, "--"])?;
                }
            }
        }
        Ok(format!("Spun off {new} from {current}"))
    }
}
