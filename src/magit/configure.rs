//! magit-branch-configure and magit-remote-configure variables, plus
//! orphan branches, shelving and unshallowing.
use super::Question;
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// branch.<b>.description (edited in Fred like upstream's editor).
    Description,
    /// branch.<b>.merge/remote via --set-upstream-to.
    Upstream,
    BranchRebase,
    BranchPushRemote,
    PullRebase,
    PushDefault,
    AutoSetupMerge,
    AutoSetupRebase,
    RemoteUrl,
    RemoteFetch,
    RemotePushurl,
    RemotePush,
    RemoteTagopt,
    RemoteFollowHead,
    Orphan,
    Shelve,
    Unshelve,
    /// magit-delete-shelved-branch.
    DeleteShelved,
    /// magit-push-notes-ref: notes ref, then remote.
    PushNotesRef,
    Unshallow,
    /// Unshallow: also replace the single refspec? (remote)
    UnshallowRefspec(String),
    /// magit--git-variable:boolean with :global t (mergetool.*): cycles
    /// true, false, unset; the default is what Git assumes when unset.
    GlobalBool(&'static str, &'static str),
    /// A global variable naming a mergetool (merge.tool, merge.guitool).
    GlobalTool(&'static str),
}

/// A variable whose key press cycles through fixed choices, then unsets.
fn choices(op: &Op) -> Option<&'static [&'static str]> {
    Some(match op {
        Op::BranchRebase | Op::PullRebase => &["true", "merges", "interactive", "false"],
        Op::AutoSetupMerge => &["always", "true", "false"],
        Op::AutoSetupRebase => &["always", "local", "remote", "never"],
        Op::RemoteTagopt => &["--no-tags", "--tags"],
        Op::RemoteFollowHead => &["create", "always", "warn"],
        _ => return None,
    })
}

fn value(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid value {v:?}"));
    }
    Ok(v)
}

impl Repo {
    /// magit-get-current-remote, required.
    fn this_remote(&self) -> Result<String, String> {
        self.current_remote()?.ok_or_else(|| "No remote".into())
    }
    /// The config key an op edits.
    fn var_key(&self, op: &Op) -> Result<String, String> {
        let branch = || self.current_branch();
        Ok(match op {
            Op::BranchRebase => format!("branch.{}.rebase", branch()?),
            Op::BranchPushRemote => format!("branch.{}.pushRemote", branch()?),
            Op::PullRebase => "pull.rebase".into(),
            Op::PushDefault => "remote.pushDefault".into(),
            Op::AutoSetupMerge => "branch.autoSetupMerge".into(),
            Op::AutoSetupRebase => "branch.autoSetupRebase".into(),
            Op::RemoteUrl => format!("remote.{}.url", self.this_remote()?),
            Op::RemoteFetch => format!("remote.{}.fetch", self.this_remote()?),
            Op::RemotePushurl => format!("remote.{}.pushurl", self.this_remote()?),
            Op::RemotePush => format!("remote.{}.push", self.this_remote()?),
            Op::RemoteTagopt => format!("remote.{}.tagOpt", self.this_remote()?),
            Op::RemoteFollowHead => format!("remote.{}.followRemoteHEAD", self.this_remote()?),
            _ => return Err("not a variable".into()),
        })
    }
    /// Prompts, or none for variables that cycle on the key press.
    pub fn configure_prompts(&self, op: &Op) -> Result<(Vec<String>, Vec<String>), String> {
        let one = |p: String| Ok((vec![p], vec![String::new()]));
        match op {
            Op::Upstream => {
                let b = self.current_branch()?;
                let now = self.upstream_of(&b).unwrap_or_default();
                one(format!("Upstream for {b} (empty unsets; now {now}): "))
            }
            Op::RemoteUrl | Op::RemoteFetch | Op::RemotePushurl | Op::RemotePush => {
                let key = self.var_key(op)?;
                let now = self.config(&key).unwrap_or_default();
                one(format!("{key} (empty unsets; now {now}): "))
            }
            Op::Orphan => Ok((
                vec![
                    "Create and checkout orphan branch: ".into(),
                    "Starting at (default HEAD): ".into(),
                ],
                vec![String::new(), "HEAD".into()],
            )),
            Op::Shelve => one("Shelve branch: ".into()),
            Op::Unshelve => one("Unshelve branch: ".into()),
            Op::DeleteShelved => one("Delete shelved branch: ".into()),
            Op::PushNotesRef => {
                let d = self.current_remote()?.unwrap_or_default();
                Ok((
                    vec![
                        "Push notes (default commits): ".into(),
                        format!("Push to remote (default {d}): "),
                    ],
                    vec!["commits".into(), d],
                ))
            }
            Op::UnshallowRefspec(r) => {
                let refspec = self
                    .config(&format!("remote.{r}.fetch"))
                    .unwrap_or_default();
                one(format!(
                    "Also replace refspec {refspec} with +refs/heads/*:refs/remotes/{r}/*? (yes or no) "
                ))
            }
            Op::GlobalTool(key) => {
                // magit--read-mergetool's choices: git mergetool --tool-help.
                let tools = self
                    .read(&["mergetool", "--tool-help"])
                    .map(|o| {
                        String::from_utf8_lossy(&o)
                            .lines()
                            .filter_map(|l| l.strip_prefix("\t\t"))
                            .filter_map(|l| l.split_whitespace().next())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                let now = self.global_config(key).unwrap_or_default();
                one(format!("{key} (empty unsets; now {now}; tools: {tools}): "))
            }
            _ => Ok((vec![], vec![])),
        }
    }
    fn global_config(&self, key: &str) -> Option<String> {
        let out = self.read(&["config", "--global", "--get", key]).ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_owned()).filter(|s| !s.is_empty())
    }
    fn set_global(&self, key: &str, v: Option<&str>) -> Result<(), String> {
        match v {
            Some(v) => self.read(&["config", "--global", key, v]).map(|_| ()),
            None => {
                let _ = self.read(&["config", "--global", "--unset-all", key]);
                Ok(())
            }
        }
    }
    fn set_config(&self, key: &str, v: Option<&str>) -> Result<(), String> {
        match v {
            Some(v) => self.read(&["config", "--replace-all", key, v]).map(|_| ()),
            None => {
                let _ = self.read(&["config", "--unset-all", key]);
                Ok(())
            }
        }
    }
    /// git's reflog of a ref, moved with it (magit--rename-reflog-file).
    fn rename_reflog(&self, old: &str, new: &str) -> Result<(), String> {
        let from = self.git_path(&format!("logs/{old}"))?;
        if from.exists() {
            let to = self.git_path(&format!("logs/{new}"))?;
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::rename(from, to).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn configure_step(&self, op: Op, a: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        let done = |m: String| Ok(Next::Done(Ok(m)));
        if let Some(list) = choices(&op) {
            // magit--git-variable:choices: the next choice, then unset.
            let key = self.var_key(&op)?;
            let now = self.config(&key);
            let next = match now.as_deref() {
                None => list.first().copied(),
                Some(v) => list.iter().skip_while(|c| **c != v).nth(1).copied(),
            };
            self.set_config(&key, next)?;
            return done(format!("{key} = {}", next.unwrap_or("(unset)")));
        }
        match op {
            Op::GlobalBool(key, default) => {
                let next = match self.global_config(key).as_deref() {
                    None => Some("true"),
                    Some("true") => Some("false"),
                    _ => None,
                };
                self.set_global(key, next)?;
                done(format!(
                    "{key} = {}",
                    next.unwrap_or(&format!("(unset, {default})"))
                ))
            }
            Op::GlobalTool(key) => {
                let v = match at(0) {
                    "" => None,
                    v => Some(value(v)?),
                };
                self.set_global(key, v)?;
                done(format!("{key} = {}", v.unwrap_or("(unset)")))
            }
            Op::BranchPushRemote | Op::PushDefault => {
                // Choices are the remotes.
                let key = self.var_key(&op)?;
                let remotes = self.remotes()?;
                let now = self.config(&key);
                let next = match now.as_deref() {
                    None => remotes.first().cloned(),
                    Some(v) => remotes.iter().skip_while(|r| *r != v).nth(1).cloned(),
                };
                self.set_config(&key, next.as_deref())?;
                done(format!("{key} = {}", next.as_deref().unwrap_or("(unset)")))
            }
            Op::Description => {
                let b = self.current_branch()?;
                Ok(Next::GitEditor(vec![
                    "branch".into(),
                    "--edit-description".into(),
                    b,
                ]))
            }
            Op::Upstream => {
                let b = self.current_branch()?;
                match at(0) {
                    "" => {
                        let _ = self.read(&["branch", "--unset-upstream", &b]);
                        done(format!("Unset upstream of {b}"))
                    }
                    u => {
                        self.read(&["branch", &format!("--set-upstream-to={}", value(u)?), &b])?;
                        done(format!("Upstream of {b} is {u}"))
                    }
                }
            }
            Op::RemoteUrl | Op::RemoteFetch | Op::RemotePushurl | Op::RemotePush => {
                let key = self.var_key(&op)?;
                let v = match at(0) {
                    "" => None,
                    v if v.chars().any(char::is_control) => {
                        return Err(format!("invalid value {v:?}"));
                    }
                    v => Some(v),
                };
                self.set_config(&key, v)?;
                done(format!("{key} = {}", v.unwrap_or("(unset)")))
            }
            Op::Orphan => {
                let b = value(at(0))?;
                self.read(&["check-ref-format", "--branch", b])
                    .map_err(|_| format!("{b:?} is not a valid branch name"))?;
                let start = value(at(1))?;
                self.read(&["checkout", "--orphan", b, start, "--"])?;
                done(format!("Created orphan branch {b}"))
            }
            Op::Shelve => {
                let b = value(at(0))?;
                if self.current_branch().ok().as_deref() == Some(b) {
                    return Err("Cannot shelve the current branch".into());
                }
                let old = format!("refs/heads/{b}");
                self.read(&["show-ref", "--verify", "-q", &old])
                    .map_err(|_| format!("No branch {b}"))?;
                let date = String::from_utf8_lossy(&self.read(&[
                    "log",
                    "-1",
                    "--format=%cs",
                    &old,
                    "--",
                ])?)
                .trim()
                .to_owned();
                let new = format!("refs/shelved/{date}-{b}");
                self.read(&["update-ref", &new, &old, ""])?;
                self.rename_reflog(&old, &new)?;
                self.set_config(&format!("branch.{b}.pushRemote"), None)?;
                self.read(&["branch", "-D", "--", b])?;
                done(format!("Shelved {b} as {}", &new[13..]))
            }
            Op::DeleteShelved => {
                let s = value(at(0))?;
                let r = format!("refs/shelved/{s}");
                self.read(&["show-ref", "--verify", "-q", &r])
                    .map_err(|_| format!("No shelved branch {s}"))?;
                self.read(&["update-ref", "-d", &r])?;
                done(format!("Deleted shelved {s}"))
            }
            Op::PushNotesRef => {
                let n = value(at(0))?;
                let r = value(at(1))?;
                if !self.remotes()?.iter().any(|x| x == r) {
                    return Err(format!("No remote {r:?}"));
                }
                let note = if n.starts_with("refs/") {
                    n.to_owned()
                } else {
                    format!("refs/notes/{n}")
                };
                Ok(Next::Git(vec!["push".into(), r.into(), note]))
            }
            Op::Unshelve => {
                let s = value(at(0))?;
                let old = format!("refs/shelved/{s}");
                self.read(&["show-ref", "--verify", "-q", &old])
                    .map_err(|_| format!("No shelved branch {s}"))?;
                // A YYYY-MM-DD- prefix is dropped.
                let dated = s.len() > 11
                    && s.as_bytes()[..11].iter().enumerate().all(|(i, c)| match i {
                        4 | 7 | 10 => *c == b'-',
                        _ => c.is_ascii_digit(),
                    });
                let name = if dated { &s[11..] } else { s };
                let new = format!("refs/heads/{name}");
                self.read(&["update-ref", &new, &old, ""])
                    .map_err(|_| format!("Branch {name} already exists"))?;
                self.rename_reflog(&old, &new)?;
                self.read(&["update-ref", "-d", &old])?;
                done(format!("Unshelved {name}"))
            }
            Op::Unshallow => {
                let r = self.this_remote()?;
                let fetch = self
                    .read(&["config", "--get-all", &format!("remote.{r}.fetch")])
                    .unwrap_or_default();
                let fetch = String::from_utf8_lossy(&fetch);
                let lines: Vec<&str> = fetch.lines().collect();
                if lines.len() == 1 && !lines[0].contains('*') {
                    let (prompts, defaults) =
                        self.configure_prompts(&Op::UnshallowRefspec(r.clone()))?;
                    return Ok(Next::Ask(
                        Question::Configure(Op::UnshallowRefspec(r)),
                        prompts,
                        defaults,
                    ));
                }
                Ok(Next::Git(vec!["fetch".into(), "--unshallow".into(), r]))
            }
            Op::UnshallowRefspec(r) => {
                if at(0) == "yes" {
                    self.set_config(
                        &format!("remote.{r}.fetch"),
                        Some(&format!("+refs/heads/*:refs/remotes/{r}/*")),
                    )?;
                }
                Ok(Next::Git(vec!["fetch".into(), "--unshallow".into(), r]))
            }
            _ => Err("unexpected configure step".into()),
        }
    }
}
