//! magit-sequence.el: cherry-pick and revert.
use super::Question;
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// A: magit-cherry-copy.
    Pick,
    /// a: magit-cherry-apply (no commit).
    Apply,
    /// h: magit-cherry-harvest, then the branch to remove the cherries from.
    Harvest,
    HarvestFrom(String),
    /// d: magit-cherry-donate, then the destination branch.
    Donate,
    DonateTo(String),
    /// n / s: move cherries to a new branch (stay / checkout), then its name.
    Spinout,
    Spinoff,
    NewBranch(String, bool),
    /// V / v: revert with or without committing.
    Revert,
    RevertNoCommit,
    /// Merge commits need a mainline: the pending op and its commits.
    Mainline(Box<Op>, String),
}

fn rev(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid revision {v:?}"));
    }
    Ok(v)
}

impl Repo {
    /// magit-sequencer-in-progress-p: (kind, verb) of a running cherry-pick/revert.
    pub fn sequencer(&self) -> Option<&'static str> {
        let exists = |name: &str| {
            self.read(&["rev-parse", "--git-path", name])
                .ok()
                .map(|p| self.root.join(String::from_utf8_lossy(&p).trim()))
                .is_some_and(|p| p.exists())
        };
        if exists("CHERRY_PICK_HEAD") {
            return Some("cherry-pick");
        }
        if exists("REVERT_HEAD") {
            return Some("revert");
        }
        let todo = self
            .read(&["rev-parse", "--git-path", "sequencer/todo"])
            .ok()
            .and_then(|p| {
                std::fs::read_to_string(self.root.join(String::from_utf8_lossy(&p).trim())).ok()
            })?;
        match todo.split_whitespace().next()? {
            "pick" | "p" => Some("cherry-pick"),
            "revert" => Some("revert"),
            _ => None,
        }
    }
    /// The commits named by an answer: one commit, or A..B in order.
    fn cherries(&self, answer: &str) -> Result<Vec<String>, String> {
        let answer = rev(answer.trim())?;
        let out = if answer.contains("..") {
            self.read(&["rev-list", "--reverse", "--end-of-options", answer])
        } else {
            self.read(&[
                "rev-parse",
                "--verify",
                "-q",
                "--end-of-options",
                &format!("{answer}^{{commit}}"),
            ])
        }
        .map_err(|_| format!("unknown revision {answer:?}"))?;
        let list: Vec<String> = String::from_utf8_lossy(&out)
            .lines()
            .map(str::to_owned)
            .collect();
        if list.is_empty() {
            return Err(format!("{answer} names no commits"));
        }
        Ok(list)
    }
    fn is_merge(&self, c: &str) -> bool {
        self.read(&["rev-parse", "--verify", "-q", &format!("{c}^2")])
            .is_ok()
    }
    fn ancestor(&self, a: &str, b: &str) -> bool {
        self.read(&["merge-base", "--is-ancestor", a, b]).is_ok()
    }

    pub fn sequence_prompts(
        &self,
        op: &Op,
        at_point: Option<String>,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let d = at_point.unwrap_or_default();
        let what = match op {
            Op::Pick => "Cherry-pick",
            Op::Apply => "Apply changes from commit",
            Op::Harvest => "Harvest cherry",
            Op::Donate => "Donate cherry",
            Op::Spinout => "Spinout cherry",
            Op::Spinoff => "Spinoff cherry",
            Op::Revert => "Revert commit",
            Op::RevertNoCommit => "Revert changes",
            _ => return Err("not a menu suffix".into()),
        };
        Ok((vec![format!("{what} (default {d}): ")], vec![d]))
    }

    /// Run one answered step. `args` are the menu's arguments.
    pub fn sequence_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let current = self.current_branch().ok();
        match op {
            Op::Pick | Op::Apply | Op::Revert | Op::RevertNoCommit => {
                let commits = self.cherries(at(0))?;
                self.pick(op, at(0), &commits, args, None)
            }
            Op::Mainline(op, answer) => {
                let n = at(0);
                if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || n == "0" {
                    return Err("Mainline must be a positive number".into());
                }
                let commits = self.cherries(&answer)?;
                self.pick(*op, &answer, &commits, args, Some(n))
            }
            Op::Harvest | Op::Donate | Op::Spinout | Op::Spinoff => {
                let away = op != Op::Harvest;
                let commits = self.cherries(at(0))?;
                let src = match &current {
                    Some(b) => b.clone(),
                    None if op == Op::Donate => self.short_head()?,
                    None => {
                        return Err(format!(
                            "Cannot {} cherries while HEAD is detached",
                            verb(&op)
                        ));
                    }
                };
                // magit--cherry-move-read-args: reachability decides direction.
                let reachable = self.ancestor(&commits[0], "HEAD");
                if !away && reachable {
                    return Err("Cannot harvest cherries that are reachable from HEAD".into());
                }
                if away && !reachable {
                    return Err(format!(
                        "Cannot {} cherries that are not reachable from HEAD",
                        verb(&op)
                    ));
                }
                let range = at(0).to_owned();
                Ok(match op {
                    Op::Harvest => {
                        let branches = self.containing(&commits[0]);
                        match branches.as_slice() {
                            [] => self.cherry_move(&commits, None, &src, args, None, true)?,
                            [one] => {
                                return self.sequence_step(
                                    Op::HarvestFrom(range),
                                    std::slice::from_ref(one),
                                    args,
                                );
                            }
                            _ => Next::Ask(
                                Question::Sequence(Op::HarvestFrom(range)),
                                vec![format!("Remove {} from branch: ", plural(commits.len()))],
                                vec![String::new()],
                            ),
                        }
                    }
                    Op::Donate => Next::Ask(
                        Question::Sequence(Op::DonateTo(range)),
                        vec![format!("Move {} to branch: ", plural(commits.len()))],
                        vec![String::new()],
                    ),
                    _ => {
                        let upstream = current
                            .as_deref()
                            .and_then(|b| {
                                self.read(&[
                                    "rev-parse",
                                    "--abbrev-ref",
                                    &format!("{b}@{{upstream}}"),
                                ])
                                .ok()
                            })
                            .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                            .unwrap_or_default();
                        Next::Ask(
                            Question::Sequence(Op::NewBranch(range, op == Op::Spinoff)),
                            vec![
                                format!("Create branch from {} cherries named: ", commits.len()),
                                format!("Starting at (default {upstream}): "),
                            ],
                            vec![String::new(), upstream],
                        )
                    }
                })
            }
            Op::HarvestFrom(range) => {
                let commits = self.cherries(&range)?;
                let from = rev(at(0))?.to_owned();
                let dst = current.ok_or("Cannot harvest cherries while HEAD is detached")?;
                // magit-cherry-harvest stays on the current branch.
                self.cherry_move(&commits, Some(&from), &dst, args, None, true)
            }
            Op::DonateTo(range) => {
                let commits = self.cherries(&range)?;
                let dst = rev(at(0))?.to_owned();
                let src = current.map_or_else(|| self.short_head(), Ok)?;
                if dst == src
                    || self
                        .read(&["show-ref", "--verify", "-q", &format!("refs/heads/{dst}")])
                        .is_err()
                {
                    return Err(format!("{dst} is not another local branch"));
                }
                self.cherry_move(&commits, Some(&src), &dst, args, None, false)
            }
            Op::NewBranch(range, checkout) => {
                let commits = self.cherries(&range)?;
                let name = rev(at(0))?.to_owned();
                self.read(&["check-ref-format", "--branch", &name])
                    .map_err(|_| format!("{name:?} is not a valid branch name"))?;
                if self
                    .read(&["show-ref", "--verify", "-q", &format!("refs/heads/{name}")])
                    .is_ok()
                {
                    return Err(format!("Branch {name} already exists"));
                }
                let start = rev(at(1))?.to_owned();
                let src = current.ok_or("Cannot move cherries while HEAD is detached")?;
                self.cherry_move(&commits, Some(&src), &name, args, Some(&start), checkout)
            }
        }
    }
    fn short_head(&self) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.read(&["rev-parse", "HEAD"])?)
            .trim()
            .to_owned())
    }
    fn containing(&self, commit: &str) -> Vec<String> {
        self.read(&["branch", "--format=%(refname:short)", "--contains", commit])
            .map(|o| {
                String::from_utf8_lossy(&o)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    /// magit--cherry-pick: merges need --mainline; mixed merges are refused.
    fn pick(
        &self,
        op: Op,
        answer: &str,
        commits: &[String],
        args: &[String],
        mainline: Option<&str>,
    ) -> Result<Next, String> {
        let revert = matches!(op, Op::Revert | Op::RevertNoCommit);
        let verb = if revert { "revert" } else { "cherry-pick" };
        let merges = commits.iter().filter(|c| self.is_merge(c)).count();
        let mut argv: Vec<String> = vec![verb.into()];
        let mut args: Vec<String> = args
            .iter()
            .filter(|a| !a.starts_with("--mainline="))
            .cloned()
            .collect();
        if matches!(op, Op::Apply | Op::RevertNoCommit) {
            args.retain(|a| a != "--ff");
            args.insert(0, "--no-commit".into());
        }
        if merges > 0 && merges < commits.len() {
            return Err(format!("Cannot {verb} merge and non-merge commits at once"));
        }
        if merges > 0 {
            match mainline {
                Some(n) => args.push(format!("--mainline={n}")),
                None => {
                    return Ok(Next::Ask(
                        Question::Sequence(Op::Mainline(Box::new(op), answer.to_owned())),
                        vec!["Replay merges relative to parent: ".into()],
                        vec!["1".into()],
                    ));
                }
            }
        }
        // Fred has no editor for git: a single --edit pick/revert stops before
        // committing and the commit draft edits MERGE_MSG.
        let edit = args.iter().any(|a| a == "--edit");
        args.retain(|a| a != "--edit");
        if edit && commits.len() == 1 && !args.iter().any(|a| a == "--no-commit") {
            args.retain(|a| a != "--ff");
            argv.push("--no-commit".into());
            argv.extend(args);
            argv.push("--end-of-options".into());
            argv.extend(commits.iter().cloned());
            let full: Vec<std::ffi::OsString> = argv.into_iter().map(Into::into).collect();
            self.run(&full, None)?;
            return Ok(Next::Draft(self.merge_message()));
        }
        argv.extend(args);
        argv.push("--end-of-options".into());
        argv.extend(commits.iter().cloned());
        Ok(Next::Git(argv))
    }
    /// magit--cherry-move: pick onto DST, then drop the cherries from SRC.
    // ponytail: commits are a contiguous range, so `rebase --onto` replaces
    // upstream's perl GIT_SEQUENCE_EDITOR that drops arbitrary picks.
    fn cherry_move(
        &self,
        commits: &[String],
        src: Option<&str>,
        dst: &str,
        args: &[String],
        start: Option<&str>,
        checkout_dst: bool,
    ) -> Result<Next, String> {
        let current = self.current_branch().ok();
        if self
            .read(&["show-ref", "--verify", "-q", &format!("refs/heads/{dst}")])
            .is_err()
        {
            let mut argv = vec!["branch", "--", dst];
            argv.extend(start);
            self.read(&argv)?;
        }
        if current.as_deref() != Some(dst) {
            self.read(&["checkout", dst, "--"])?;
        }
        let mut argv: Vec<std::ffi::OsString> = vec!["cherry-pick".into()];
        argv.extend(args.iter().filter(|a| *a != "--edit").map(Into::into));
        argv.push("--end-of-options".into());
        argv.extend(commits.iter().map(Into::into));
        self.run(&argv, None)?;
        let Some(src) = src else {
            return Ok(Next::Done(Ok(format!(
                "Harvested {}",
                plural(commits.len())
            ))));
        };
        let tip = commits.last().expect("non-empty cherries");
        let keep = format!("{}^", commits[0]);
        let src_tip = String::from_utf8_lossy(&self.read(&["rev-parse", src])?)
            .trim()
            .to_owned();
        if *tip == src_tip {
            if self
                .read(&["show-ref", "--verify", "-q", &format!("refs/heads/{src}")])
                .is_ok()
            {
                self.read(&[
                    "update-ref",
                    "-m",
                    &format!("reset: moving to {keep}"),
                    &format!("refs/heads/{src}"),
                    &keep,
                    tip,
                ])?;
            }
        } else {
            self.read(&["checkout", src, "--"])?;
            self.read(&["rebase", "--onto", &keep, tip, src])?;
        }
        let back = if checkout_dst { dst } else { src };
        self.read(&["checkout", back, "--"])?;
        Ok(Next::Done(Ok(format!(
            "Moved {} to {dst}",
            plural(commits.len())
        ))))
    }
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 cherry".into()
    } else {
        format!("{n} cherries")
    }
}
fn verb(op: &Op) -> &'static str {
    match op {
        Op::Harvest | Op::HarvestFrom(_) => "harvest",
        Op::Donate | Op::DonateTo(_) => "donate",
        Op::Spinout => "spinout",
        _ => "spinoff",
    }
}
