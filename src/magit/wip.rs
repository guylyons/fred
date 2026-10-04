//! magit-wip.el: work-in-progress refs (refs/wip/index/..., refs/wip/wtree/...).
use super::branch::Next;
use super::repo::Repo;
use super::workflows::StashIndex;
use std::ffi::OsString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// magit-wip-commit: index and worktree of tracked files.
    Commit,
    LogIndex,
    LogWorktree,
    LogCurrent,
    Purge,
    PurgeConfirmed(Vec<String>),
}

const NAMESPACE: &str = "refs/wip/";

impl Repo {
    /// magit-wip-get-ref: the full name of HEAD's branch, or HEAD.
    fn wip_ref(&self) -> Option<String> {
        let r = self
            .read(&["symbolic-ref", "HEAD"])
            .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
            .unwrap_or_else(|_| "HEAD".into());
        self.read(&["rev-parse", "--verify", "-q", &r])
            .ok()
            .map(|_| r)
    }
    fn wip_name(kind: &str, r: &str) -> String {
        format!("{NAMESPACE}{kind}/{r}")
    }
    fn rev(&self, r: &str) -> Option<String> {
        self.read(&["rev-parse", "--verify", "-q", r])
            .ok()
            .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
    }
    /// magit-wip-update-wipref (magit-wip-merge-branch nil): a wip ref that
    /// no longer descends from the branch restarts from it.
    fn wip_update(
        &self,
        r: &str,
        wipref: &str,
        tree: &str,
        msg: &str,
        start: &str,
    ) -> Result<bool, String> {
        let head = self.rev(r).ok_or("no ref")?;
        let mut parent = match self.rev(wipref) {
            Some(w)
                if self.rev(&format!("{w}^0")).is_some()
                    && self
                        .read(&["merge-base", &w, &head])
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                        == Some(head.clone()) =>
            {
                w
            }
            _ => head.clone(),
        };
        let commit = |tree: &str, parent: &str, m: &str| -> Result<String, String> {
            let out = self.read(&["commit-tree", "--no-gpg-sign", "-p", parent, "-m", m, tree])?;
            Ok(String::from_utf8_lossy(&out).trim().to_owned())
        };
        if parent == head {
            let m = format!("start autosaving {start}");
            let c = commit(&format!("{head}^{{tree}}"), &head, &m)?;
            self.read(&["update-ref", "--create-reflog", "-m", &m, wipref, &c])?;
            parent = c;
        }
        if self
            .read(&["diff-tree", "--quiet", &parent, tree, "--"])
            .is_ok()
        {
            return Ok(false);
        }
        let c = commit(tree, &parent, msg)?;
        self.read(&["update-ref", "--create-reflog", "-m", msg, wipref, &c])?;
        Ok(true)
    }
    /// magit-wip-commit: commit the index, then the worktree's tracked files.
    pub fn wip_commit(&self, msg: &str) -> Result<String, String> {
        let r = self
            .wip_ref()
            .ok_or("No commit to base work-in-progress refs on")?;
        let tree = String::from_utf8_lossy(&self.read(&["write-tree"])?)
            .trim()
            .to_owned();
        let index = self.wip_update(&r, &Self::wip_name("index", &r), &tree, msg, "index")?;
        // The worktree tree: a temporary index from the wip parent plus
        // every tracked change (add -u).
        let tmp = StashIndex::new()?;
        let idx = tmp.0.join("index");
        let run = |args: &[&str]| {
            self.run_index(
                &args.iter().map(OsString::from).collect::<Vec<_>>(),
                None,
                Some(&idx),
            )
        };
        run(&["read-tree", "HEAD"])?;
        run(&["add", "-u", "."])?;
        let wtree = String::from_utf8_lossy(&run(&["write-tree"])?)
            .trim()
            .to_owned();
        let worktree =
            self.wip_update(&r, &Self::wip_name("wtree", &r), &wtree, msg, "worktree")?;
        Ok(match (index, worktree) {
            (false, false) => "No changes since the last wip commit".into(),
            _ => format!("Saved work in progress to {NAMESPACE}{{index,wtree}}/{r}"),
        })
    }
    pub fn wip_step(&self, op: Op, a: &[String]) -> Result<Next, String> {
        let r = self.wip_ref().unwrap_or_else(|| "HEAD".into());
        let log = |revs: Vec<String>| {
            Ok(Next::View(super::Kind::Log(
                revs,
                vec!["-n256".into(), "--graph".into(), "--decorate".into()],
            )))
        };
        match op {
            Op::Commit => Ok(Next::Done(Ok(self.wip_commit("wip-save tracked files")?))),
            Op::LogIndex => log(vec![Self::wip_name("index", &r)]),
            Op::LogWorktree => log(vec![Self::wip_name("wtree", &r)]),
            Op::LogCurrent => {
                let mut revs = vec![self.current_branch().unwrap_or_else(|_| "HEAD".into())];
                for w in [Self::wip_name("wtree", &r), Self::wip_name("index", &r)] {
                    if self.rev(&w).is_some() {
                        revs.push(w);
                    }
                }
                log(revs)
            }
            Op::Purge => {
                // Wip refs whose ref no longer exists (HEAD's are kept).
                let out = self.read(&["for-each-ref", "--format=%(refname)", NAMESPACE])?;
                let dangling: Vec<String> = String::from_utf8_lossy(&out)
                    .lines()
                    .filter(|w| {
                        // refs/wip/<kind>/<ref>
                        let target = w
                            .strip_prefix(NAMESPACE)
                            .and_then(|r| r.split_once('/'))
                            .map_or("", |(_, t)| t);
                        target != "HEAD"
                            && self.read(&["show-ref", "--verify", "-q", target]).is_err()
                    })
                    .map(str::to_owned)
                    .collect();
                if dangling.is_empty() {
                    return Ok(Next::Done(Ok(
                        "All wip-refs have a corresponding ref".into()
                    )));
                }
                Ok(Next::Ask(
                    super::Question::Wip(Op::PurgeConfirmed(dangling.clone())),
                    vec![format!(
                        "Delete {} wip-refs without corresponding ref? (y or n) ",
                        dangling.len()
                    )],
                    vec![String::new()],
                ))
            }
            Op::PurgeConfirmed(refs) => {
                if !matches!(a.first().map(|s| s.trim()), Some("y" | "yes")) {
                    return Err("Abort".into());
                }
                for w in &refs {
                    self.read(&["update-ref", "-d", w])?;
                }
                Ok(Next::Done(Ok(format!("Deleted {} wip-refs", refs.len()))))
            }
        }
    }
}
