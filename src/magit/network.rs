//! magit-push.el, magit-fetch.el and magit-pull.el suffixes.
use super::repo::{GitInvocation, Repo};
use std::ffi::OsString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    PushRemote,
    PushUpstream,
    PushElsewhere,
    PushOther,
    PushRefspecs,
    PushMatching,
    PushTag,
    PushTags,
    FetchRemote,
    FetchUpstream,
    FetchElsewhere,
    FetchAll,
    FetchBranch,
    FetchRefspec,
    FetchModules,
    PullRemote,
    PullUpstream,
    PullElsewhere,
    /// magit-pull-into-upstream: fast-forward the local upstream from its own.
    PullIntoUpstream,
    /// magit-push-to-remote: git push -v [ARGS] REMOTE, no refspec.
    PushToRemote,
}

impl Op {
    /// The prefix whose arguments this suffix reads.
    pub fn menu(self) -> char {
        use Op::*;
        match self {
            PushRemote | PushUpstream | PushElsewhere | PushOther | PushRefspecs | PushMatching
            | PushTag | PushTags | PushToRemote => 'p',
            PullRemote | PullUpstream | PullElsewhere | PullIntoUpstream => 'P',
            // magit-fetch-modules has its own transient.
            FetchModules => 'Z',
            _ => 'f',
        }
    }
}

fn checked(value: &str) -> Result<&str, String> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(format!("invalid value {value:?}"));
    }
    Ok(value)
}

impl Repo {
    pub(super) fn config(&self, key: &str) -> Option<String> {
        let out = self.read(&["config", "--get", key]).ok()?;
        Some(String::from_utf8_lossy(&out).trim_end().to_owned()).filter(|s| !s.is_empty())
    }
    /// Configured remotes. Names that look like options (possible via
    /// `git remote add -- -x`) are never offered, so no caller can pass one.
    pub fn remotes(&self) -> Result<Vec<String>, String> {
        Ok(String::from_utf8_lossy(&self.read(&["remote"])?)
            .lines()
            .filter(|r| !r.starts_with('-'))
            .map(str::to_owned)
            .collect())
    }
    pub fn current_branch(&self) -> Result<String, String> {
        self.read(&["symbolic-ref", "--short", "-q", "HEAD"])
            .ok()
            .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
            .filter(|b| !b.is_empty())
            .ok_or_else(|| "No branch is checked out".into())
    }
    /// magit-get-push-remote: branch.<b>.pushRemote, else remote.pushDefault,
    /// only when it names an existing remote.
    pub(super) fn push_remote(&self, branch: &str) -> Result<Option<String>, String> {
        let remotes = self.remotes()?;
        Ok(self
            .config(&format!("branch.{branch}.pushRemote"))
            .or_else(|| self.config("remote.pushDefault"))
            .filter(|r| remotes.contains(r)))
    }
    /// The configured upstream (remote, merge ref) if the remote is usable.
    fn upstream(&self, branch: &str) -> Result<Option<(String, String)>, String> {
        let remotes = self.remotes()?;
        Ok(self
            .config(&format!("branch.{branch}.remote"))
            .zip(self.config(&format!("branch.{branch}.merge")))
            .filter(|(r, _)| r == "." || remotes.contains(r)))
    }
    /// magit-get-current-remote: upstream remote, the only remote, or origin.
    pub(super) fn current_remote(&self) -> Result<Option<String>, String> {
        let remotes = self.remotes()?;
        let upstream = match self.current_branch() {
            Ok(b) => self.config(&format!("branch.{b}.remote")),
            Err(_) => None,
        };
        Ok(upstream.filter(|r| remotes.contains(r)).or_else(|| {
            if remotes.len() == 1 {
                remotes.first().cloned()
            } else {
                self.primary_remote(&remotes)
            }
        }))
    }
    /// magit-primary-remote: magit.primaryRemote, then "upstream", then "origin".
    pub(super) fn primary_remote(&self, remotes: &[String]) -> Option<String> {
        self.config("magit.primaryRemote")
            .into_iter()
            .chain(["upstream".into(), "origin".into()])
            .find(|r| remotes.contains(r))
    }
    fn only_remote(&self) -> Result<Option<String>, String> {
        let remotes = self.remotes()?;
        Ok(if remotes.len() == 1 {
            remotes.first().cloned()
        } else {
            None
        })
    }
    fn known_remote(&self, value: &str) -> Result<String, String> {
        let value = checked(value)?;
        if !self.remotes()?.iter().any(|r| r == value) {
            return Err(format!("no remote named {value:?}"));
        }
        Ok(value.to_owned())
    }
    /// magit-split-branch-name: "." for local branches, else the remote prefix.
    fn split_branch(&self, name: &str) -> Result<(String, String), String> {
        let name = checked(name)?;
        if self
            .read(&["show-ref", "--verify", "-q", &format!("refs/heads/{name}")])
            .is_ok()
        {
            return Ok((".".into(), name.into()));
        }
        let mut remotes = self.remotes()?;
        remotes.sort_by_key(|r| std::cmp::Reverse(r.len()));
        remotes
            .into_iter()
            .find_map(|r| {
                let rest = name.strip_prefix(&r)?.strip_prefix('/')?;
                (!rest.is_empty()).then(|| (r.clone(), rest.to_owned()))
            })
            .ok_or_else(|| format!("Invalid branch name {name}"))
            // The remainder becomes its own argv entry, so it must not read as an option.
            .and_then(|(r, rest)| Ok((r, checked(&rest)?.to_owned())))
    }

    /// The questions a suffix still needs answered, given current configuration.
    pub fn network_prompts(&self, op: Op) -> Result<Vec<String>, String> {
        use Op::*;
        let one = |s: &str| Ok(vec![s.to_owned()]);
        let remote_unless_only = |prompt: &str| -> Result<Vec<String>, String> {
            Ok(if self.only_remote()?.is_some() {
                vec![]
            } else {
                vec![prompt.to_owned()]
            })
        };
        match op {
            PushRemote | FetchRemote | PullRemote => {
                let branch = self.current_branch()?;
                if self.push_remote(&branch)?.is_some() {
                    return Ok(vec![]);
                }
                let verb = match op {
                    PushRemote => "push there",
                    FetchRemote => "fetch from there",
                    _ => "pull from there",
                };
                // magit-prefer-push-default: offer remote.pushDefault instead.
                let var = if super::options::flag("magit-prefer-push-default", false) {
                    "remote.pushDefault".to_owned()
                } else {
                    format!("branch.{branch}.pushRemote")
                };
                one(&format!("Set {var} and {verb}: "))
            }
            PushUpstream | PullUpstream => {
                let branch = self.current_branch()?;
                if self.upstream(&branch)?.is_some() {
                    return Ok(vec![]);
                }
                let verb = if op == PushUpstream {
                    "push"
                } else {
                    "pull from"
                };
                one(&format!("Set upstream of {branch} and {verb} there: "))
            }
            PushElsewhere => one(&format!("Push {} to: ", self.current_branch()?)),
            PushOther => Ok(vec!["Push: ".into(), "Push to: ".into()]),
            PushRefspecs => Ok(vec!["Push to remote: ".into(), "Push refspec,s: ".into()]),
            PushMatching => remote_unless_only("Push matching branches to: "),
            PushTag => {
                let mut prompts = vec!["Push tag: ".to_owned()];
                prompts.extend(remote_unless_only("Push tag to remote: ")?);
                Ok(prompts)
            }
            PushTags => remote_unless_only("Push tags to remote: "),
            FetchUpstream | FetchAll | FetchModules => Ok(vec![]),
            FetchElsewhere => one("Fetch remote: "),
            FetchBranch => Ok(vec![
                "Fetch from remote or url: ".into(),
                "Fetch branch: ".into(),
            ]),
            FetchRefspec => Ok(vec![
                "Fetch from remote or url: ".into(),
                "Fetch using refspec: ".into(),
            ]),
            PullElsewhere => one("Pull: "),
            PullIntoUpstream => Ok(vec![]),
            PushToRemote => one("Push to remote: "),
        }
    }

    /// Build the Git command for a suffix; configuration it sets is written first.
    pub fn network(
        &self,
        op: Op,
        answers: &[String],
        args: &[String],
    ) -> Result<GitInvocation, String> {
        use Op::*;
        let answer = |i: usize| -> Result<&str, String> {
            answers
                .get(i)
                .map(String::as_str)
                .ok_or_else(|| "missing answer".into())
        };
        let mut cmd: Vec<String> = vec![];
        let mut add = |parts: &[&str]| cmd.extend(parts.iter().map(|s| s.to_string()));
        match op {
            PushRemote | FetchRemote | PullRemote => {
                let branch = self.current_branch()?;
                let remote = match self.push_remote(&branch)? {
                    Some(remote) if answers.is_empty() => remote,
                    _ => {
                        let remote = self.known_remote(answer(0)?)?;
                        let var = if super::options::flag("magit-prefer-push-default", false) {
                            "remote.pushDefault".to_owned()
                        } else {
                            format!("branch.{branch}.pushRemote")
                        };
                        self.read(&["config", &var, &remote])?;
                        remote
                    }
                };
                match op {
                    PushRemote => {
                        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
                        add(&["push", "-v"]);
                        add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                        add(&[&remote, &refspec]);
                    }
                    FetchRemote => {
                        add(&["fetch", &remote]);
                        add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                    }
                    _ => {
                        add(&["pull"]);
                        add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                        add(&[&remote, &branch]);
                    }
                }
            }
            PushUpstream | PullUpstream => {
                let branch = self.current_branch()?;
                let (remote, merge, set) = match self.upstream(&branch)? {
                    Some((remote, merge)) if answers.is_empty() => (remote, merge, false),
                    _ => {
                        let target = answer(0)?;
                        let (remote, merge) = self.split_branch(target)?;
                        if op == PullUpstream {
                            self.read(&["branch", "--set-upstream-to", target, &branch])?;
                        }
                        let merge = if merge.starts_with("refs/") {
                            merge
                        } else {
                            format!("refs/heads/{merge}")
                        };
                        (remote, merge, true)
                    }
                };
                if op == PushUpstream {
                    add(&["push", "-v"]);
                    add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                    if set && !args.iter().any(|a| a == "--set-upstream") {
                        add(&["--set-upstream"]);
                    }
                    add(&[&remote, &format!("{branch}:{merge}")]);
                } else {
                    add(&["pull"]);
                    add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                    add(&[&remote, &merge]);
                }
            }
            PushElsewhere | PushOther => {
                let (source, target) = if op == PushElsewhere {
                    (self.current_branch()?, answer(0)?)
                } else {
                    (checked(answer(0)?)?.to_owned(), answer(1)?)
                };
                // magit-git-push: qualify the target only when it is not yet tracked.
                let (remote, name) = self.split_branch(target)?;
                let tracked = self
                    .read(&[
                        "show-ref",
                        "--verify",
                        "-q",
                        &format!("refs/remotes/{remote}/{name}"),
                    ])
                    .is_ok();
                let namespace = if tracked { "" } else { "refs/heads/" };
                add(&["push", "-v"]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                add(&[&remote, &format!("{source}:{namespace}{name}")]);
            }
            PushRefspecs => {
                let remote = self.known_remote(answer(0)?)?;
                add(&["push", "-v"]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                add(&[&remote]);
                for refspec in answer(1)?.split(',').map(str::trim) {
                    add(&[checked(refspec)?]);
                }
            }
            PushMatching | PushTags => {
                let remote = match self.only_remote()? {
                    Some(remote) if answers.is_empty() => remote,
                    _ => self.known_remote(answer(0)?)?,
                };
                if op == PushMatching {
                    add(&["push", "-v"]);
                    add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                    add(&[&remote, ":"]);
                } else {
                    add(&["push", &remote, "--tags"]);
                    add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                }
            }
            PushTag => {
                let tag = checked(answer(0)?)?;
                self.read(&["show-ref", "--verify", "-q", &format!("refs/tags/{tag}")])
                    .map_err(|_| format!("no tag named {tag:?}"))?;
                let remote = match self.only_remote()? {
                    Some(remote) if answers.len() == 1 => remote,
                    _ => self.known_remote(answer(1)?)?,
                };
                add(&["push", &remote, tag]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
            }
            FetchUpstream => {
                let remote = self
                    .current_remote()?
                    .ok_or("The \"current\" remote could not be determined")?;
                add(&["fetch", &remote]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
            }
            FetchElsewhere => {
                let remote = self.known_remote(answer(0)?)?;
                add(&["fetch", &remote]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
            }
            FetchAll => {
                add(&["fetch", "--all"]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
            }
            FetchBranch | FetchRefspec => {
                add(&["fetch", checked(answer(0)?)?, checked(answer(1)?)?]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
            }
            // magit-fetch-modules: its own transient's arguments.
            FetchModules => {
                add(&["fetch", "--recurse-submodules"]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
            }
            PullIntoUpstream => {
                // magit-pull--upstreams: a local upstream whose own upstream is remote.
                let up1 = self
                    .upstream_of(&self.current_branch()?)
                    .filter(|u| self.local_branch(u));
                let up2 = up1.as_deref().and_then(|u| self.upstream_of(u));
                let (Some(up1), Some(up2)) = (up1, up2) else {
                    return Err("Cannot perform background update of upstream branch".into());
                };
                let (remote, branch) = self.split_branch(&up2)?;
                if remote == "." {
                    return Err("Cannot perform background update of upstream branch".into());
                }
                // Upstream fetches, then update-ref's the branch to FETCH_HEAD
                // unconditionally; a forced refspec does both in one command.
                add(&["fetch", &remote, &format!("+{branch}:refs/heads/{up1}")]);
            }
            PushToRemote => {
                let remote = self.known_remote(answer(0)?)?;
                add(&["push", "-v"]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                add(&[&remote]);
            }
            PullElsewhere => {
                let (remote, branch) = self.split_branch(answer(0)?)?;
                add(&["pull"]);
                add(&args.iter().map(String::as_str).collect::<Vec<_>>());
                add(&[&remote, &branch]);
            }
        }
        Ok(GitInvocation {
            expected_head: None,
            repo: self.clone(),
            args: cmd.into_iter().map(OsString::from).collect(),
            input: None,
            draft: None,
            draft_stamp: None,
            editor: false,
            env: vec![],
            after: None,
        })
    }
}
