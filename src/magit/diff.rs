//! magit-diff.el prefix suffixes and their generated diff buffers.
use super::repo::Repo;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Dwim,
    Range,
    Paths,
    Unstaged,
    Staged,
    Worktree,
    ShowCommit,
    ShowStash,
}

/// What a generated diff buffer shows; kept so gr can recompute it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Unstaged,
    Staged,
    /// A range, or a single revision compared with the working tree.
    Range(String),
    Paths(PathBuf, PathBuf),
    Commit(String),
}
impl Target {
    pub fn title(&self) -> String {
        match self {
            Self::Unstaged => "Changes between index and working tree".into(),
            Self::Staged => "Changes between HEAD and index".into(),
            Self::Range(range) => format!("Changes in {}", super::repo::label(range.as_ref())),
            Self::Paths(a, b) => format!(
                "Changes between {} and {}",
                super::repo::label(a),
                super::repo::label(b)
            ),
            Self::Commit(id) => format!("Commit {}", &id[..id.len().min(12)]),
        }
    }
}

fn revision(value: &str) -> Result<&str, String> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(format!("invalid revision {value:?}"));
    }
    Ok(value)
}

impl Repo {
    /// Prompts a suffix needs when nothing at point supplies its value.
    pub fn diff_prompts(op: Op) -> Vec<String> {
        match op {
            Op::Dwim | Op::Range => vec!["Diff for range: ".into()],
            Op::Paths => vec!["First file: ".into(), "Second file: ".into()],
            Op::ShowCommit => vec!["Show commit: ".into()],
            Op::ShowStash => vec!["Show stash: ".into()],
            Op::Unstaged | Op::Staged | Op::Worktree => vec![],
        }
    }
    /// Turn prompt answers into a target, validating revisions and files.
    pub fn diff_target(&self, op: Op, answers: &[String]) -> Result<Target, String> {
        let answer = |i: usize| answers.get(i).map(String::as_str).ok_or("missing answer");
        Ok(match op {
            // Passed through like magit-diff-range (HEAD^!, A^@ ...); Git reports bad ones.
            Op::Dwim | Op::Range => Target::Range(revision(answer(0)?)?.into()),
            Op::ShowCommit => {
                let rev = format!("{}^{{commit}}", revision(answer(0)?)?);
                let id = self
                    .read(&["rev-parse", "--verify", "-q", "--end-of-options", &rev])
                    .map_err(|_| format!("unknown commit {:?}", answer(0).unwrap_or("")))?;
                Target::Commit(String::from_utf8_lossy(&id).trim().into())
            }
            Op::Paths => {
                let file = |i: usize| -> Result<PathBuf, String> {
                    let path = self.root.join(answer(i)?);
                    if !path.is_file() {
                        return Err(format!("no such file {:?}", answer(i)?));
                    }
                    Ok(path)
                };
                Target::Paths(file(0)?, file(1)?)
            }
            Op::Unstaged | Op::Staged | Op::Worktree | Op::ShowStash => {
                return Err("this diff does not read a value".into());
            }
        })
    }
    pub fn diff_output(&self, target: &Target, args: &[String]) -> Result<Vec<u8>, String> {
        let mut argv: Vec<std::ffi::OsString> = vec![];
        let show = matches!(target, Target::Commit(_));
        argv.push(if show { "show" } else { "diff" }.into());
        // magit-insert-diff/revision always ask for the patch, even with --stat.
        argv.push("-p".into());
        argv.push("--no-color".into());
        if *target == Target::Staged {
            argv.push("--cached".into());
        }
        if matches!(target, Target::Paths(..)) {
            argv.push("--no-index".into());
        }
        // --show-signature only applies to commits.
        argv.extend(
            args.iter()
                .filter(|a| show || *a != "--show-signature")
                // magit-diff-paths passes no transient arguments.
                .filter(|_| !matches!(target, Target::Paths(..)))
                .map(Into::into),
        );
        match target {
            Target::Range(range) => argv.push(revision(range)?.into()),
            Target::Commit(id) => argv.push(revision(id)?.into()),
            _ => (),
        }
        argv.push("--".into());
        if let Target::Paths(a, b) = target {
            argv.push(a.into());
            argv.push(b.into());
        }
        let out = self
            .command()
            .args(&argv)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("git: {e}"))?;
        // diff --no-index exits 1 when the files differ.
        if out.status.success()
            || (matches!(target, Target::Paths(..)) && out.status.code() == Some(1))
        {
            Ok(out.stdout)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }
}
