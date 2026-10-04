//! magit-stash.el transforms: branch, branch here, format patch, clear.
use super::branch::Next;
use super::repo::Repo;
use super::workflows::{Stash, StashAction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Branch,
    BranchHere,
    FormatPatch,
    Clear,
}

impl Repo {
    fn find_stash(&self, name: &str) -> Result<Stash, String> {
        self.stashes()?
            .into_iter()
            .find(|s| s.selector == name || s.id == name)
            .ok_or_else(|| format!("No stash {name:?}"))
    }
    pub fn stash_prompts(&self, op: &Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        let d = at_point.unwrap_or_else(|| "stash@{0}".into());
        match op {
            Op::Branch | Op::BranchHere => (
                vec![
                    format!("Branch stash (default {d}): "),
                    "Branch name: ".into(),
                ],
                vec![d, String::new()],
            ),
            Op::FormatPatch => (
                vec![format!("Create patch from stash (default {d}): ")],
                vec![d],
            ),
            Op::Clear => (
                vec!["Drop all stashes in refs/stash? (y or n) ".into()],
                vec![String::new()],
            ),
        }
    }
    pub fn stash_step(&self, op: Op, a: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let done = |m: String| Ok(Next::Done(Ok(m)));
        if op == Op::Clear {
            if !matches!(at(0), "y" | "yes") {
                return Err("Abort".into());
            }
            self.read(&["update-ref", "-d", "refs/stash"])?;
            return done("Dropped all stashes".into());
        }
        let stash = self.find_stash(at(0))?;
        match op {
            Op::Branch | Op::BranchHere => {
                let branch = at(1);
                if branch.is_empty() || branch.starts_with('-') {
                    return Err(format!("invalid branch name {branch:?}"));
                }
                self.read(&["check-ref-format", "--branch", branch])
                    .map_err(|_| format!("{branch:?} is not a valid branch name"))?;
                if op == Op::Branch {
                    // magit-stash-branch: start where the stash was made.
                    self.read(&["stash", "branch", branch, &stash.selector])?;
                    return done(format!("Created {branch} from {}", stash.selector));
                }
                // magit-stash-branch-here: start here, then magit-stash-apply
                // (which keeps the stash, despite the docstring's "dropping").
                let start = self.current_branch().unwrap_or_else(|_| "HEAD".into());
                self.read(&["checkout", "-b", branch, &start, "--"])?;
                let applied = self.stash_action(&stash, StashAction::Apply)?;
                done(format!("Created {branch}; {applied}"))
            }
            Op::FormatPatch => {
                let name = String::from_utf8_lossy(&self.read(&[
                    "log",
                    "-1",
                    "--format=0001-%f.patch",
                    &stash.id,
                    "--",
                ])?)
                .trim()
                .to_owned();
                let patch = self.read(&["stash", "show", "-p", &stash.id])?;
                let path = self.root.join(&name);
                if path.exists() {
                    return Err(format!("{name} already exists"));
                }
                std::fs::write(&path, patch).map_err(|e| e.to_string())?;
                done(format!("Wrote {name}"))
            }
            Op::Clear => unreachable!(),
        }
    }
}
