//! magit-ediff.el, adapted: Fred has no Ediff, so comparisons run the user's
//! `git difftool` and conflicts their `git mergetool`, in the terminal.
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// magit-ediff-resolve-rest / -all / magit-git-mergetool: a conflicted file.
    Resolve,
    ShowUnstaged,
    ShowStaged,
    ShowWorktree,
    ShowCommit,
    /// magit-ediff-compare: two revisions.
    Compare,
    ShowStash,
}

fn value(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid value {v:?}"));
    }
    Ok(v)
}

impl Repo {
    fn unmerged(&self) -> Vec<String> {
        self.read(&["diff", "--name-only", "--diff-filter=U", "-z"])
            .map(|o| {
                o.split(|b| *b == 0)
                    .filter(|f| !f.is_empty())
                    .map(|f| String::from_utf8_lossy(f).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn ediff_prompts(
        &self,
        op: &Op,
        file: Option<String>,
        commit: Option<String>,
    ) -> (Vec<String>, Vec<String>) {
        let ask = |p: &str, d: &str| {
            if d.is_empty() {
                format!("{p}: ")
            } else {
                format!("{p} (default {d}): ")
            }
        };
        let file = file.unwrap_or_default();
        match op {
            Op::Resolve => {
                let d = if self.unmerged().contains(&file) {
                    file
                } else {
                    self.unmerged().into_iter().next().unwrap_or_default()
                };
                (vec![ask("Resolve", &d)], vec![d])
            }
            Op::ShowUnstaged | Op::ShowStaged | Op::ShowWorktree => {
                (vec![ask("Show changes in file", &file)], vec![file])
            }
            Op::ShowCommit => {
                let d = commit.unwrap_or_else(|| "HEAD".into());
                (vec![ask("Show commit", &d)], vec![d])
            }
            Op::Compare => (
                vec![
                    "Compare revision (A): ".into(),
                    "With revision (B, default worktree): ".into(),
                ],
                vec![String::new(), String::new()],
            ),
            Op::ShowStash => (
                vec![ask("Show stash", "stash@{0}")],
                vec!["stash@{0}".into()],
            ),
        }
    }
    pub fn ediff_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        // magit-git-mergetool:--tool / a difftool's --tool.
        let tool: Vec<String> = args
            .iter()
            .filter(|x| x.starts_with("--tool="))
            .cloned()
            .collect();
        let difftool = |mut words: Vec<String>, file: Option<&str>| -> Result<Next, String> {
            let mut argv = vec!["difftool".to_owned(), "-y".into()];
            argv.extend(tool.iter().cloned());
            argv.append(&mut words);
            argv.push("--".into());
            if let Some(f) = file {
                argv.push(value(f)?.into());
            }
            Ok(Next::Git(argv))
        };
        match op {
            Op::Resolve => {
                let f = value(at(0))?;
                if !self.unmerged().iter().any(|u| u == f) {
                    return Err(format!("{f} has no conflicts"));
                }
                let mut argv = vec!["mergetool".to_owned()];
                argv.extend(tool.iter().cloned());
                argv.extend(["--".into(), f.into()]);
                Ok(Next::Git(argv))
            }
            Op::ShowUnstaged => difftool(vec![], Some(at(0))),
            Op::ShowStaged => difftool(vec!["--cached".into()], Some(at(0))),
            Op::ShowWorktree => difftool(vec!["HEAD".into()], Some(at(0))),
            Op::ShowCommit => {
                let c = value(at(0))?;
                difftool(vec![format!("{c}^"), c.into()], None)
            }
            Op::Compare => {
                let a = value(at(0))?;
                let mut words = vec![a.to_owned()];
                if !at(1).is_empty() {
                    words.push(value(at(1))?.into());
                }
                difftool(words, None)
            }
            Op::ShowStash => {
                let s = value(at(0))?;
                difftool(vec![format!("{s}^"), s.into()], None)
            }
        }
    }
}
