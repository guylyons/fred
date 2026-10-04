//! magit-reset.el: reset HEAD, index and/or worktree.
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Mixed,
    Soft,
    Hard,
    Keep,
    Index,
    Worktree,
}

impl Repo {
    pub fn reset_prompt(&self, op: Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        let here = self
            .current_branch()
            .unwrap_or_else(|_| "detached head".into());
        let d = at_point.unwrap_or_default();
        let prompt = match op {
            Op::Mixed | Op::Keep => format!("Reset {here} to"),
            Op::Soft => format!("Soft reset {here} to"),
            Op::Hard => format!("Hard reset {here} to"),
            Op::Index => "Reset index to".into(),
            Op::Worktree => "Reset worktree to".into(),
        };
        (vec![format!("{prompt} (default {d}): ")], vec![d])
    }

    pub fn reset_step(&self, op: Op, a: &[String]) -> Result<Next, String> {
        let commit = a.first().map(String::as_str).unwrap_or("");
        if commit.is_empty() || commit.starts_with('-') || commit.chars().any(char::is_control) {
            return Err(format!("invalid revision {commit:?}"));
        }
        self.read(&[
            "rev-parse",
            "--verify",
            "-q",
            "--end-of-options",
            &format!("{commit}^{{commit}}"),
        ])
        .map_err(|_| format!("unknown revision {commit:?}"))?;
        let done = |m: String| Ok(Next::Done(Ok(m)));
        match op {
            Op::Worktree => {
                // magit-reset-worktree: check out every file of COMMIT through a
                // temporary index, leaving HEAD and the real index alone.
                let dir = std::env::temp_dir().join(format!("fred-reset-{}", std::process::id()));
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let index = dir.join("index");
                let read_tree: Vec<std::ffi::OsString> = ["read-tree", "--end-of-options", commit]
                    .map(Into::into)
                    .into();
                let result = self
                    .run_index(&read_tree, None, Some(&index))
                    .and_then(|_| {
                        let checkout: Vec<std::ffi::OsString> =
                            ["checkout-index", "--all", "--force"]
                                .map(Into::into)
                                .into();
                        self.run_index(&checkout, None, Some(&index))
                    });
                let _ = std::fs::remove_dir_all(&dir);
                result?;
                done(format!("Reset worktree to {commit}"))
            }
            Op::Index => {
                // magit-reset-internal with a path: git reset COMMIT -- .
                self.read(&["reset", "-q", commit, "--", "."])?;
                done(format!("Reset index to {commit}"))
            }
            _ => {
                let mode = match op {
                    Op::Mixed => "--mixed",
                    Op::Soft => "--soft",
                    Op::Hard => "--hard",
                    _ => "--keep",
                };
                if op == Op::Hard {
                    self.untracked_clobbered(commit)?;
                }
                self.read(&["reset", "-q", mode, commit, "--"])?;
                done(format!("Reset to {commit}"))
            }
        }
    }
}
