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
    /// The same, run in this directory relative to the toplevel (git -C).
    GitCommandIn(std::path::PathBuf),
    ResetQuickly,
    /// magit-clean: untracked (0), untracked and ignored (1), ignored only (2).
    Clean(u8),
    /// magit-find-git-config-file.
    GitConfigFile,
    RemoteSetHead,
    RemoteUnsetHead,
    /// magit-stage-files (ignored files too when true) / magit-unstage-files.
    StageFiles(bool),
    UnstageFiles,
}

/// magit-completing-read-multiple: comma-separated repository-relative files.
fn files(answer: &str) -> Result<Vec<std::ffi::OsString>, String> {
    let mut out = vec![];
    for f in answer.split(',').map(str::trim).filter(|f| !f.is_empty()) {
        out.push(super::repo::literal_pathspec(&super::blob::relative(f)?));
    }
    if out.is_empty() {
        return Err("No file selected".into());
    }
    Ok(out)
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
            Op::GitCommand { .. } | Op::GitCommandIn(_) => {
                (vec!["git ".into()], vec![String::new()])
            }
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
            Op::Clean(n) => {
                let what = ["untracked", "untracked and ignored", "ignored"][*n as usize % 3];
                (
                    vec![format!("Remove {what} files? (yes or no) ")],
                    vec![String::new()],
                )
            }
            Op::GitConfigFile => (vec![], vec![]),
            Op::StageFiles(_) | Op::UnstageFiles => {
                let d = at_point.unwrap_or_default();
                let verb = match op {
                    Op::StageFiles(true) => "Stage ignored file,s",
                    Op::StageFiles(false) => "Stage file,s",
                    _ => "Unstage file,s",
                };
                let suffix = if d.is_empty() {
                    String::new()
                } else {
                    format!(" (default {d})")
                };
                (vec![format!("{verb}{suffix}: ")], vec![d])
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
            Op::GitCommand { .. } | Op::GitCommandIn(_) => {
                // A leading "git" is optional, as upstream's prompt shows it.
                let mut words = split_words(at(0))?;
                if words.first().map(String::as_str) == Some("git") {
                    words.remove(0);
                }
                if words.is_empty() {
                    return Err("No git subcommand".into());
                }
                // The repository stays the toplevel (so its buffers refresh);
                // a working directory is git -C relative to it.
                if let Op::GitCommandIn(dir) = &op
                    && !dir.as_os_str().is_empty()
                {
                    words.splice(0..0, ["-C".to_owned(), dir.to_string_lossy().into_owned()]);
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
            Op::Clean(n) => {
                if at(0) != "yes" {
                    return Err("Abort".into());
                }
                let mut argv = vec!["clean", "-f", "-d"];
                match n {
                    1 => argv.push("-x"),
                    2 => argv.push("-X"),
                    _ => {}
                }
                self.read(&argv)?;
                Ok(Next::Done(Ok("Cleaned".into())))
            }
            Op::GitConfigFile => {
                let p = self.read(&["rev-parse", "--git-path", "config"])?;
                Ok(Next::Visit(
                    self.root.join(String::from_utf8_lossy(&p).trim()),
                ))
            }
            Op::StageFiles(force) => {
                let mut argv: Vec<std::ffi::OsString> = vec!["add".into()];
                if force {
                    argv.push("--force".into());
                }
                argv.push("--".into());
                argv.extend(files(at(0))?);
                self.run(&argv, None)?;
                Ok(Next::Done(Ok("Staged".into())))
            }
            Op::UnstageFiles => {
                // magit-unstage-1: git rm --cached before the first commit.
                let born = self.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
                let mut argv: Vec<std::ffi::OsString> = if born {
                    vec!["reset".into(), "-q".into(), "HEAD".into()]
                } else {
                    vec!["rm".into(), "--cached".into(), "-q".into()]
                };
                argv.push("--".into());
                argv.extend(files(at(0))?);
                self.run(&argv, None)?;
                Ok(Next::Done(Ok("Unstaged".into())))
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
