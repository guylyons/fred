//! magit-merge.el suffixes.
use super::Question;
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Plain,
    /// e: merge without committing, then edit MERGE_MSG in Fred's commit draft.
    EditMsg,
    NoCommit,
    Absorb,
    /// Absorbing the main branch needs a typed "yes".
    AbsorbMain(String),
    /// magit-merge-assert confirmed for a dirty worktree: the op and its answer.
    Dirty(Box<Op>, String),
    Preview,
    Squash,
    Dissolve,
    Abort,
}

fn rev(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid revision {v:?}"));
    }
    Ok(v)
}

impl Repo {
    pub fn merge_in_progress(&self) -> bool {
        self.read(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .is_ok()
    }
    fn previous_branch(&self) -> String {
        self.read(&["rev-parse", "--abbrev-ref", "@{-1}"])
            .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
            .unwrap_or_default()
    }
    /// magit-read-other-branches-or-commits: comma-separated for an octopus.
    fn merge_revs(&self, answer: &str) -> Result<Vec<String>, String> {
        answer
            .split(',')
            .map(|r| {
                let r = rev(r.trim())?;
                self.read(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    "--end-of-options",
                    &format!("{r}^{{commit}}"),
                ])
                .map_err(|_| format!("unknown revision {r:?}"))?;
                Ok(r.to_owned())
            })
            .collect()
    }
    /// The commit message of an in-progress merge (or squash), for the draft.
    pub fn merge_message(&self) -> Vec<u8> {
        ["MERGE_MSG", "SQUASH_MSG"]
            .iter()
            .find_map(|name| {
                let path = self.read(&["rev-parse", "--git-path", name]).ok()?;
                let path = self.root.join(String::from_utf8_lossy(&path).trim());
                std::fs::read(path).ok()
            })
            .unwrap_or_default()
    }

    pub fn merge_prompts(&self, op: &Op) -> Result<(Vec<String>, Vec<String>), String> {
        let other = self.previous_branch();
        let current = self.current_branch().ok();
        Ok(match op {
            Op::Plain | Op::EditMsg | Op::NoCommit => {
                (vec![format!("Merge (default {other}): ")], vec![other])
            }
            Op::Squash => (vec![format!("Squash (default {other}): ")], vec![other]),
            Op::Preview => (
                vec![format!("Preview merge (default {other}): ")],
                vec![other],
            ),
            Op::Absorb => (
                vec![format!("Absorb branch (default {other}): ")],
                vec![other],
            ),
            Op::Dissolve => {
                let into = current
                    .as_deref()
                    .and_then(|b| self.upstream_local(b))
                    .unwrap_or_default();
                let from = current.unwrap_or_else(|| "HEAD".into());
                (
                    vec![format!("Merge `{from}' into (default {into}): ")],
                    vec![into],
                )
            }
            Op::Abort => (vec!["Abort merge? (y or n) ".into()], vec![String::new()]),
            Op::AbsorbMain(_) | Op::Dirty(..) => return Err("not a menu suffix".into()),
        })
    }
    /// magit-get-local-upstream-branch.
    fn upstream_local(&self, b: &str) -> Option<String> {
        (self.config(&format!("branch.{b}.remote")).as_deref() == Some("."))
            .then(|| self.config(&format!("branch.{b}.merge")))
            .flatten()
            .map(|m| m.trim_start_matches("refs/heads/").to_owned())
    }

    pub fn merge_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        self.merge_inner(op, a, args, true)
    }
    fn merge_inner(
        &self,
        op: Op,
        a: &[String],
        args: &[String],
        check: bool,
    ) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let git = |mut argv: Vec<String>| {
            argv.insert(0, "merge".into());
            Ok(Next::Git(argv))
        };
        // magit-merge-assert: ask before merging over uncommitted changes
        // (magit-anything-modified-p t ignores submodules).
        let dirty = || {
            self.read(&["diff", "--quiet", "--ignore-submodules"])
                .is_err()
                || self
                    .read(&["diff", "--cached", "--quiet", "--ignore-submodules"])
                    .is_err()
        };
        if check && matches!(op, Op::Plain | Op::EditMsg | Op::NoCommit | Op::Squash) && dirty() {
            return Ok(Next::Ask(
                Question::Merge(Op::Dirty(Box::new(op), at(0).to_owned())),
                vec!["Merging with dirty worktree is risky.  Continue? (y or n) ".into()],
                vec![String::new()],
            ));
        }
        let with_args = |extra: &[&str], drop_ff_only: bool, revs: Vec<String>| {
            let mut argv: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
            argv.extend(
                args.iter()
                    .filter(|x| !(drop_ff_only && *x == "--ff-only"))
                    .cloned(),
            );
            if extra.contains(&"--no-ff") {
                argv.dedup();
            }
            argv.push("--".into());
            argv.extend(revs);
            argv
        };
        let other_local = |b: &str| {
            self.current_branch().ok().as_deref() != Some(b)
                && self
                    .read(&["show-ref", "--verify", "-q", &format!("refs/heads/{b}")])
                    .is_ok()
        };
        match op {
            Op::Dirty(op, answer) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                self.merge_inner(*op, &[answer], args, false)
            }
            Op::Plain => git(with_args(&["--no-edit"], false, self.merge_revs(at(0))?)),
            Op::NoCommit | Op::EditMsg => {
                let mut extra = vec!["--no-commit"];
                if !args.iter().any(|x| x == "--no-ff") {
                    extra.push("--no-ff");
                }
                let argv = with_args(&extra, true, self.merge_revs(at(0))?);
                if op == Op::NoCommit {
                    return git(argv);
                }
                // magit-merge-editmsg: Fred edits MERGE_MSG in its own commit draft.
                let mut full = vec!["merge".to_owned()];
                full.extend(argv);
                let full: Vec<std::ffi::OsString> = full.into_iter().map(Into::into).collect();
                self.run(&full, None)?;
                if !self.merge_in_progress() {
                    return Ok(Next::Done(Ok("Already up to date".into())));
                }
                Ok(Next::Draft(self.merge_message()))
            }
            Op::Squash => git(vec!["--squash".into(), "--".into(), rev(at(0))?.into()]),
            Op::Preview => {
                let mut revs = self.merge_revs(at(0))?;
                if revs.len() != 1 {
                    return Err("Preview takes a single revision".into());
                }
                let other = revs.remove(0);
                // git merge-tree exits 1 for conflicts but still prints the tree.
                let out = self
                    .command()
                    .args(["merge-tree", "--write-tree", "HEAD", &other])
                    .stdin(std::process::Stdio::null())
                    .output()
                    .map_err(|e| e.to_string())?;
                let tree = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                if tree.len() < 40 || !tree.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
                }
                Ok(Next::Show(super::diff::Target::Range(format!(
                    "HEAD..{tree}"
                ))))
            }
            // magit-read-other-local-branch for absorb and dissolve.
            Op::Absorb => {
                let branch = rev(at(0))?.to_owned();
                if !other_local(&branch) {
                    return Err(format!("{branch} is not another local branch"));
                }
                self.absorb(branch, args)
            }
            Op::AbsorbMain(branch) => {
                if at(0) != "yes" {
                    return Err("Abort".into());
                }
                self.absorb_1(&branch, args)
            }
            Op::Dissolve => {
                let into = rev(at(0))?.to_owned();
                if !other_local(&into) {
                    return Err(format!("{into} is not another local branch"));
                }
                let current = self.current_branch().ok();
                let head = String::from_utf8_lossy(&self.read(&["rev-parse", "HEAD"])?)
                    .trim()
                    .to_owned();
                self.read(&["checkout", &into, "--"])?;
                match current {
                    Some(branch) => self.absorb(branch, args),
                    None => git(with_args(&["--no-edit"], false, vec![head])),
                }
            }
            Op::Abort => {
                if !self.merge_in_progress() {
                    return Err("No merge in progress".into());
                }
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                git(vec!["--abort".into()])
            }
        }
    }
    /// magit--merge-absorb: absorbing the main branch must be confirmed.
    fn absorb(&self, branch: String, args: &[String]) -> Result<Next, String> {
        let main = self
            .config("init.defaultBranch")
            .into_iter()
            .chain(["main", "master", "trunk", "development"].map(String::from))
            .find(|b| {
                self.read(&["show-ref", "--verify", "-q", &format!("refs/heads/{b}")])
                    .is_ok()
            });
        if main.as_deref() == Some(branch.as_str()) {
            return Ok(Next::Ask(
                Question::Merge(Op::AbsorbMain(branch.clone())),
                vec![format!(
                    "Do you really want to merge `{branch}' into another branch? (yes or no) "
                )],
                vec![String::new()],
            ));
        }
        self.absorb_1(&branch, args)
    }
    /// magit--merge-absorb-1: merge, then delete the merged branch.
    // ponytail: the pre-merge force push to the push target and PR remotes are not
    // ported; deletion runs only after a successful in-process merge.
    fn absorb_1(&self, branch: &str, args: &[String]) -> Result<Next, String> {
        let mut argv: Vec<&str> = vec!["merge"];
        argv.extend(args.iter().map(String::as_str));
        argv.extend(["--no-edit", "--", branch]);
        self.read(&argv)?;
        self.read(&["branch", "-D", "--", branch])?;
        let _ = self.read(&["config", "--unset", &format!("branch.{branch}.pushRemote")]);
        Ok(Next::Done(Ok(format!("Merged and removed {branch}"))))
    }
}
