//! magit-remote.el: add, rename, remove and prune remotes.
use super::Question;
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    /// remote.pushDefault is unset: name, url, then ask whether to set it.
    AddPushDefault(String, String),
    Rename,
    Remove,
    Prune,
    PruneRefspecs,
    /// Remote, stale (refspec, tracking refs): confirm pruning them.
    PruneStale(String, Vec<(String, Vec<String>)>),
    /// Every refspec is stale: [d]efault refspec, [r]emove remote or [a]bort.
    AllStale(String),
}

fn remote_name(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(format!("invalid remote name {v:?}"));
    }
    Ok(v)
}

impl Repo {
    fn existing_remote(&self, v: &str) -> Result<String, String> {
        let v = remote_name(v)?;
        if !self.remotes()?.iter().any(|r| r == v) {
            return Err(format!("no remote named {v:?}"));
        }
        Ok(v.to_owned())
    }
    /// magit-remote--cleanup-push-variables.
    fn cleanup_push_variables(&self, remote: &str, new: Option<&str>) {
        if self.config("remote.pushDefault").as_deref() == Some(remote) {
            let _ = match new {
                Some(n) => self.read(&["config", "remote.pushDefault", n]),
                None => self.read(&["config", "--unset", "remote.pushDefault"]),
            };
        }
        let Ok(out) = self.read(&[
            "config",
            "--name-only",
            "--get-regexp",
            r"^branch\..*\.pushremote$",
        ]) else {
            return;
        };
        for var in String::from_utf8_lossy(&out).lines() {
            if self.config(var).as_deref() == Some(remote) {
                let _ = match new {
                    Some(n) => self.read(&["config", var, n]),
                    None => self.read(&["config", "--unset", var]),
                };
            }
        }
    }

    pub fn remote_prompts(&self, op: &Op) -> Result<(Vec<String>, Vec<String>), String> {
        let current = self.current_remote().ok().flatten().unwrap_or_default();
        Ok(match op {
            Op::Add => (
                vec!["Remote name: ".into(), "Remote url: ".into()],
                vec![String::new(), String::new()],
            ),
            Op::Rename => (
                vec![
                    format!("Rename remote (default {current}): "),
                    "Rename to: ".into(),
                ],
                vec![current, String::new()],
            ),
            Op::Remove => (
                vec![format!("Delete remote (default {current}): ")],
                vec![current],
            ),
            Op::Prune => (
                vec![format!(
                    "Prune stale branches of remote (default {current}): "
                )],
                vec![current],
            ),
            Op::PruneRefspecs => (
                vec![format!("Prune refspecs of remote (default {current}): ")],
                vec![current],
            ),
            _ => return Err("not a menu suffix".into()),
        })
    }
    /// magit-remote-add's url default: origin's url with its owner replaced.
    pub fn suggested_url(&self, remote: &str) -> String {
        let Some(origin) = self.config("remote.origin.url") else {
            return String::new();
        };
        let re = regex::Regex::new(r"([^:/]+)/[^/]+(\.git)?$").expect("valid url regexp");
        match re.captures(&origin).and_then(|c| c.get(1)) {
            Some(m) => format!("{}{remote}{}", &origin[..m.start()], &origin[m.end()..]),
            None => String::new(),
        }
    }

    pub fn remote_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let done = |m: String| Ok(Next::Done(Ok(m)));
        match op {
            Op::Add => {
                let name = remote_name(at(0))?.to_owned();
                if self.remotes()?.contains(&name) {
                    return Err(format!("remote {name} already exists"));
                }
                let mut url = at(1).to_owned();
                if url.is_empty() {
                    url = self.suggested_url(&name);
                }
                if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
                    return Err("A remote url is required".into());
                }
                if let Some(rest) = url.strip_prefix("~/") {
                    url = std::env::var("HOME")
                        .map(|h| format!("{h}/{rest}"))
                        .unwrap_or(url);
                }
                // magit-remote-add-set-remote.pushDefault is ask-if-unset.
                if self.config("remote.pushDefault").is_none() {
                    return Ok(Next::Ask(
                        Question::Remote(Op::AddPushDefault(name.clone(), url)),
                        vec![format!("Set `remote.pushDefault' to \"{name}\"? (y or n) ")],
                        vec![String::new()],
                    ));
                }
                self.add_remote(&name, &url, args, false)
            }
            Op::AddPushDefault(name, url) => {
                let set = matches!(at(0), "y" | "yes");
                self.add_remote(&name, &url, args, set)
            }
            Op::Rename => {
                let old = self.existing_remote(at(0))?;
                let new = remote_name(at(1))?;
                if old == new {
                    return done("Nothing to rename".into());
                }
                self.read(&["remote", "rename", &old, new])?;
                self.cleanup_push_variables(&old, Some(new));
                done(format!("Renamed {old} to {new}"))
            }
            Op::Remove => {
                let remote = self.existing_remote(at(0))?;
                self.read(&["remote", "rm", &remote])?;
                self.cleanup_push_variables(&remote, None);
                done(format!("Removed remote {remote}"))
            }
            Op::Prune => {
                let remote = self.existing_remote(at(0))?;
                Ok(Next::Git(vec!["remote".into(), "prune".into(), remote]))
            }
            Op::PruneRefspecs => {
                let remote = self.existing_remote(at(0))?;
                self.prune_refspecs(remote)
            }
            Op::PruneStale(remote, stale) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                let var = format!("remote.{remote}.fetch");
                for (refspec, refs) in &stale {
                    self.read(&["config", "--fixed-value", "--unset", &var, refspec])?;
                    for r in refs {
                        self.read(&["update-ref", "-d", r])?;
                    }
                }
                done(format!("Pruned {} stale refspecs", stale.len()))
            }
            Op::AllStale(remote) => match at(0) {
                "d" => {
                    let var = format!("remote.{remote}.fetch");
                    self.read(&["config", "--unset-all", &var])?;
                    self.read(&[
                        "config",
                        &var,
                        &format!("+refs/heads/*:refs/remotes/{remote}/*"),
                    ])?;
                    done("Replaced with the default refspec".into())
                }
                "r" => {
                    self.read(&["remote", "rm", &remote])?;
                    self.cleanup_push_variables(&remote, None);
                    done(format!("Removed remote {remote}"))
                }
                _ => Err("Abort".into()),
            },
        }
    }
    fn add_remote(
        &self,
        name: &str,
        url: &str,
        args: &[String],
        push_default: bool,
    ) -> Result<Next, String> {
        // Fetching after add is network work for the terminal.
        let fetch = args.iter().any(|a| a == "-f");
        self.read(&["remote", "add", "--", name, url])?;
        if push_default {
            self.read(&["config", "remote.pushDefault", name])?;
        }
        if fetch {
            return Ok(Next::Git(vec!["fetch".into(), name.into()]));
        }
        Ok(Next::Done(Ok(format!("Added remote {name}"))))
    }
    /// magit-remote-prune-refspecs.
    fn prune_refspecs(&self, remote: String) -> Result<Next, String> {
        let remote_refs: Vec<String> =
            String::from_utf8_lossy(&self.read(&["ls-remote", "--", &remote])?)
                .lines()
                .filter_map(|l| l.split_once('\t').map(|(_, r)| r.to_owned()))
                .collect();
        let tracking: Vec<String> = String::from_utf8_lossy(&self.read(&[
            "for-each-ref",
            "--format=%(refname)",
            &format!("refs/remotes/{remote}/"),
        ])?)
        .lines()
        .map(str::to_owned)
        .collect();
        let refspecs: Vec<String> = self
            .read(&["config", "--get-all", &format!("remote.{remote}.fetch")])
            .map(|o| {
                String::from_utf8_lossy(&o)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let glob = |pattern: &str, s: &str| match pattern.split_once('*') {
            Some((pre, post)) => {
                s.starts_with(pre) && s.ends_with(post) && s.len() >= pre.len() + post.len()
            }
            None => pattern == s,
        };
        let mut stale = vec![];
        for refspec in &refspecs {
            let Some((theirs, ours)) = refspec.trim_start_matches('+').split_once(':') else {
                continue;
            };
            if !remote_refs.iter().any(|r| glob(theirs, r)) {
                let refs: Vec<String> =
                    tracking.iter().filter(|t| glob(ours, t)).cloned().collect();
                stale.push((refspec.clone(), refs));
            }
        }
        if stale.is_empty() {
            return Ok(Next::Done(Ok(format!(
                "No stale refspecs for remote {remote:?}"
            ))));
        }
        if stale.len() == refspecs.len() {
            return Ok(Next::Ask(
                Question::Remote(Op::AllStale(remote.clone())),
                vec![format!(
                    "All of {remote}'s refspecs are stale.  replace with [d]efault refspec, [r]emove remote, [a]bort "
                )],
                vec![String::new()],
            ));
        }
        let branches: usize = stale.iter().map(|(_, r)| r.len()).sum();
        let question = match stale.as_slice() {
            [(refspec, refs)] => format!(
                "Prune stale refspec {refspec} and {} branches? (y or n) ",
                refs.len()
            ),
            many => format!(
                "Prune {} stale refspecs and {branches} branches? (y or n) ",
                many.len()
            ),
        };
        Ok(Next::Ask(
            Question::Remote(Op::PruneStale(remote, stale)),
            vec![question],
            vec![String::new()],
        ))
    }
}
