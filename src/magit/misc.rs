//! magit-git-command (magit.el), magit-reset-quickly, magit-remote-set-head
//! and -unset-head.
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Run a git subcommand: in the repository root, or this directory.
    GitCommand {
        topdir: bool,
    },
    ResetQuickly,
    RemoteSetHead,
    RemoteUnsetHead,
}

/// split-string-shell-command: words, with '...' and "..." quoting and
/// backslash escapes.
pub fn split_words(s: &str) -> Result<Vec<String>, String> {
    let mut words = vec![];
    let mut word = String::new();
    let mut started = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("unterminated '".into()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => word.push(chars.next().ok_or("trailing \\")?),
                        Some(c) => word.push(c),
                        None => return Err("unterminated \"".into()),
                    }
                }
            }
            '\\' => {
                started = true;
                word.push(chars.next().ok_or("trailing \\")?);
            }
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

impl Repo {
    pub fn misc_prompts(&self, op: &Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        match op {
            Op::GitCommand { .. } => (vec!["git ".into()], vec![String::new()]),
            Op::ResetQuickly => {
                let here = self
                    .current_branch()
                    .unwrap_or_else(|_| "detached head".into());
                let d = at_point.unwrap_or_default();
                let suffix = if d.is_empty() {
                    String::new()
                } else {
                    format!(" (default {d})")
                };
                (vec![format!("Reset {here} to{suffix}: ")], vec![d])
            }
            Op::RemoteSetHead | Op::RemoteUnsetHead => {
                let d = self.current_remote().ok().flatten().unwrap_or_default();
                let verb = if *op == Op::RemoteSetHead {
                    "Set"
                } else {
                    "Unset"
                };
                (
                    vec![format!("{verb} HEAD for remote (default {d}): ")],
                    vec![d],
                )
            }
        }
    }
    pub fn misc_step(&self, op: Op, a: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        match op {
            Op::GitCommand { .. } => {
                // A leading "git" is optional, as upstream's prompt shows it.
                let mut words = split_words(at(0))?;
                if words.first().map(String::as_str) == Some("git") {
                    words.remove(0);
                }
                if words.is_empty() {
                    return Err("No git subcommand".into());
                }
                Ok(Next::GitEditor(words))
            }
            Op::ResetQuickly => {
                let c = at(0);
                if c.is_empty() || c.starts_with('-') || c.chars().any(char::is_control) {
                    return Err(format!("invalid revision {c:?}"));
                }
                self.read(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    "--end-of-options",
                    &format!("{c}^{{commit}}"),
                ])
                .map_err(|_| format!("unknown revision {c:?}"))?;
                self.read(&["reset", "--mixed", "-q", c, "--"])?;
                Ok(Next::Done(Ok(format!("Reset HEAD to {c}"))))
            }
            Op::RemoteSetHead | Op::RemoteUnsetHead => {
                let r = at(0);
                if !self.remotes()?.iter().any(|x| x == r) {
                    return Err(format!("No remote {r:?}"));
                }
                let how = if op == Op::RemoteSetHead {
                    "--auto"
                } else {
                    "--delete"
                };
                Ok(Next::Git(vec![
                    "remote".into(),
                    "set-head".into(),
                    r.into(),
                    how.into(),
                ]))
            }
        }
    }
}
