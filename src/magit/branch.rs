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
    ResetConfirmed(String, String),
    /// k, and its follow-up questions.
    Delete,
    DeleteRemote(String),
    DeleteCurrent(String),
    DeleteUnmerged(String),
}

/// What the session should do after a branch step.
pub enum Next {
    Done(Result<String, String>),
    Ask(Op, Vec<String>, Vec<String>),
    /// A network command for the terminal (delete on a remote).
    Git(Vec<String>),
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
            Op::Reset => {
                let upstream = current
                    .as_deref()
                    .and_then(|b| self.upstream_of(b))
                    .unwrap_or_default();
                (
                    vec![
                        format!("Reset branch (default {here}): "),
                        format!("Reset to (default {upstream}): "),
                    ],
                    vec![here.clone(), upstream],
                )
            }
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
                ]) && let Some((remote, branch)) = choice.split_once('/')
                    && !self.local_branch(branch)
                {
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
                    Op::CheckoutNew(new.clone()),
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
                let old = name(at(0))?;
                let old = old.strip_prefix("heads/").unwrap_or(old).to_owned();
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
                let to = self.commitish(at(1))?;
                if self.current_branch().ok().as_deref() == Some(branch.as_str()) && self.modified()
                {
                    return Ok(Next::Ask(
                        Op::ResetConfirmed(branch, to),
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
                    ]) && let Some((remote, _)) = branch.split_once('/')
                    {
                        return Ok(Next::Ask(
                            Op::DeleteRemote(branch.clone()),
                            vec![format!(
                                "Deleting local refs/remotes/{branch}.  Also delete on {remote}? (y or n) "
                            )],
                            vec![String::new()],
                        ));
                    }
                    return Err(format!("No branch named {branch}"));
                }
                if self.current_branch().ok().as_deref() == Some(branch.as_str()) {
                    let target = self.upstream_of(&branch).filter(|u| self.local_branch(u));
                    let choices = match &target {
                        Some(t) => {
                            format!("[d]etach HEAD & delete, [c]heckout {t} & delete, [a]bort ")
                        }
                        None => "[d]etach HEAD & delete, [a]bort ".into(),
                    };
                    return Ok(Next::Ask(
                        Op::DeleteCurrent(branch.clone()),
                        vec![format!("Branch {branch} is checked out.  {choices}")],
                        vec![target.unwrap_or_default()],
                    ));
                }
                self.delete_local(&branch, false)
            }
            Op::DeleteRemote(branch) => {
                let (remote, name) = branch.split_once('/').ok_or("not a remote branch")?;
                if matches!(at(0), "y" | "yes") {
                    return Ok(Next::Git(vec![
                        "push".into(),
                        "--delete".into(),
                        remote.into(),
                        format!("refs/heads/{name}"),
                    ]));
                }
                self.git(&["update-ref", "-d", &format!("refs/remotes/{branch}")])?;
                done(format!("Deleted refs/remotes/{branch}"))
            }
            Op::DeleteCurrent(branch) => {
                let target = defaults.first().filter(|t| !t.is_empty()).cloned();
                match at(0) {
                    "d" => self.git(&["checkout", "--detach"])?,
                    "c" if target.is_some() => {
                        self.git(&["checkout", target.as_deref().unwrap_or_default(), "--"])?
                    }
                    _ => return Err("Abort".into()),
                }
                self.delete_local(&branch, true)
            }
            Op::DeleteUnmerged(branch) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                self.unset_push_remote(&branch);
                self.git(&["branch", "-D", &branch])?;
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
    /// Unmerged branches (not in their upstream, nor HEAD) need confirmation.
    fn delete_local(&self, branch: &str, checked_out: bool) -> Result<Next, String> {
        let into = self.upstream_of(branch).unwrap_or_else(|| "HEAD".into());
        let merged = self.ok(&[
            "merge-base",
            "--is-ancestor",
            &format!("refs/heads/{branch}"),
            &into,
        ]);
        if !merged {
            return Ok(Next::Ask(
                Op::DeleteUnmerged(branch.to_owned()),
                vec![format!("Delete unmerged branch {branch}? (y or n) ")],
                vec![String::new()],
            ));
        }
        self.unset_push_remote(branch);
        self.git(&["branch", if checked_out { "-D" } else { "-d" }, branch])?;
        Ok(Next::Done(Ok(format!("Deleted {branch}"))))
    }
    fn reset_branch(&self, branch: &str, to: &str) -> Result<String, String> {
        if self.current_branch().ok().as_deref() == Some(branch) {
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
        if checkout {
            self.git(&["checkout", "-b", new, &current, "--"])?;
        } else {
            self.git(&["branch", new, &current])?;
        }
        // The new branch tracks the old one, as with magit-get-indirect-upstream-branch.
        self.git(&["branch", &format!("--set-upstream-to={current}"), new])?;
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
                    self.git(&["reset", "--hard", &base, "--"])?;
                }
            }
        }
        Ok(format!("Spun off {new} from {current}"))
    }
}
