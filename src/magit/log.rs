//! magit-log.el: log buffers over revisions with git-log arguments.
use super::Kind;
use super::branch::Next;
use super::repo::{Repo, label};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Current,
    Head,
    Related,
    Other,
    LocalBranches,
    AllBranches,
    All,
    Reflog,
    MatchingBranches,
    MatchingTags,
    Merged,
    /// magit-shortlog-since / -range.
    ShortlogSince,
    ShortlogRange,
    /// magit-cherry: head, then upstream.
    Cherry,
}

/// One washed log line: graph prefix, then a commit or a continuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub commit: Option<String>,
}

fn rev(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid revision {v:?}"));
    }
    Ok(v)
}

/// The arguments git log receives: Fred draws the graph colors and refs itself.
pub fn git_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut out = vec![];
    // magit-log-refresh-buffer drops --graph when --reverse is used.
    let reverse = args.iter().any(|a| a == "--reverse");
    for a in args {
        if let Some(n) = a.strip_prefix("-n") {
            n.parse::<usize>()
                .map_err(|_| format!("invalid commit limit {n:?}"))?;
        }
        if !a.starts_with("-- ")
            && !matches!(
                a.as_str(),
                "--color" | "--decorate" | "++header" | "--follow"
            )
            && !(reverse && a == "--graph")
        {
            out.push(a.clone());
        }
    }
    Ok(out)
}

/// magit-log-get-commit-limit.
pub fn limit(args: &[String]) -> Option<usize> {
    args.iter()
        .find_map(|a| a.strip_prefix("-n").and_then(|n| n.parse().ok()))
}

/// Replace (or with None, drop) the commit limit.
pub fn with_limit(args: &[String], n: Option<usize>) -> Vec<String> {
    let mut out: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("-n"))
        .cloned()
        .collect();
    if let Some(n) = n {
        out.push(format!("-n{n}"));
    }
    out
}

impl Repo {
    /// magit-get-previous-branch.
    fn previous(&self) -> Option<String> {
        let current = self.current_branch().ok();
        (1..=20).find_map(|i| {
            let out = self
                .read(&["rev-parse", "--abbrev-ref", &format!("@{{-{i}}}")])
                .ok()?;
            let b = String::from_utf8_lossy(&out).trim().to_owned();
            (!b.is_empty() && Some(&b) != current.as_ref()).then_some(b)
        })
    }
    fn abbrev(&self, rev: &str) -> Option<String> {
        let out = self.read(&["rev-parse", "--abbrev-ref", rev]).ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_owned()).filter(|s| !s.is_empty())
    }
    /// magit-log-related's revisions.
    fn related(&self) -> Vec<String> {
        let mut head = None;
        let current = self.current_branch().ok().or_else(|| {
            let name = self
                .git_path("rebase-merge/head-name")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().trim_start_matches("refs/heads/").to_owned())
                .filter(|s| !s.is_empty());
            if name.is_some() {
                head = Some("HEAD".to_owned());
                name
            } else {
                self.previous()
            }
        });
        let Some(current) = current else {
            return vec!["HEAD".into()];
        };
        let target = self.abbrev(&format!("{current}@{{push}}"));
        let upstream = self.abbrev(&format!("{current}@{{upstream}}"));
        let upup = upstream
            .as_ref()
            .filter(|u| self.ok(&["show-ref", "--verify", "-q", &format!("refs/heads/{u}")]))
            .and_then(|u| self.abbrev(&format!("{u}@{{upstream}}")));
        let mut revs = vec![];
        for r in [Some(current), head, target, upstream, upup]
            .into_iter()
            .flatten()
        {
            if !revs.contains(&r) {
                revs.push(r);
            }
        }
        revs
    }
    pub fn log_prompts(&self, op: &Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        match op {
            Op::Other => {
                let d = at_point.or_else(|| self.previous()).unwrap_or_default();
                (vec![format!("Log rev,s (default {d}): ")], vec![d])
            }
            Op::MatchingBranches => (
                vec!["Type a pattern to pass to --branches: ".into()],
                vec![String::new()],
            ),
            Op::MatchingTags => (
                vec!["Type a pattern to pass to --tags: ".into()],
                vec![String::new()],
            ),
            Op::Merged => {
                let d = at_point.unwrap_or_default();
                let here = self.current_branch().unwrap_or_default();
                (
                    vec![
                        format!("Log merge of commit (default {d}): "),
                        format!("Merged into (default {here}): "),
                    ],
                    vec![d, here],
                )
            }
            Op::ShortlogSince => {
                // magit-get-current-tag.
                let d = self
                    .read(&["describe", "--tags", "--abbrev=0"])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .unwrap_or_default();
                (vec![format!("Shortlog since (default {d}): ")], vec![d])
            }
            Op::Cherry => {
                let head = self.current_branch().unwrap_or_default();
                let upstream = self.upstream_of(&head).unwrap_or_default();
                (
                    vec![
                        format!("Cherry head (default {head}): "),
                        format!("Cherry upstream (default {upstream}): "),
                    ],
                    vec![head, upstream],
                )
            }
            Op::ShortlogRange => {
                let d = at_point.unwrap_or_default();
                (
                    vec![format!("Shortlog for revision or range (default {d}): ")],
                    vec![d],
                )
            }
            _ => (vec![], vec![]),
        }
    }
    /// The revisions to log, or a question to ask first.
    pub fn log_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let branch = self.current_branch().ok();
        let with_head = |mut v: Vec<String>| {
            if branch.is_none() {
                v.insert(0, "HEAD".into());
            }
            v
        };
        let revs = match op {
            Op::Current => vec![branch.clone().unwrap_or_else(|| "HEAD".into())],
            Op::Head => vec!["HEAD".into()],
            Op::Related => self.related(),
            Op::Other => at(0)
                .split([',', ' '])
                .filter(|r| !r.is_empty())
                .map(|r| rev(r).map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|v| {
                    if v.is_empty() {
                        Err("No revision given".into())
                    } else {
                        Ok(v)
                    }
                })?,
            Op::LocalBranches => with_head(vec!["--branches".into()]),
            Op::AllBranches => with_head(vec!["--branches".into(), "--remotes".into()]),
            Op::All => vec!["--all".into()],
            Op::Reflog => vec!["--reflog".into()],
            Op::MatchingBranches | Op::MatchingTags => {
                let p = at(0);
                if p.is_empty() || p.chars().any(char::is_control) {
                    return Err("A pattern is required".into());
                }
                let kind = if op == Op::MatchingTags {
                    "tags"
                } else {
                    "branches"
                };
                vec!["HEAD".into(), format!("--{kind}={p}")]
            }
            Op::Merged => return self.log_merged(rev(at(0))?, rev(at(1))?, args),
            Op::ShortlogSince => {
                return Ok(Next::View(Kind::Shortlog(
                    format!("{}..", rev(at(0))?),
                    args.to_vec(),
                )));
            }
            Op::Cherry => {
                let (head, upstream) = (rev(at(0))?, rev(at(1))?);
                for r in [head, upstream] {
                    self.read(&["rev-parse", "--verify", "-q", "--end-of-options", r])
                        .map_err(|_| format!("unknown revision {r:?}"))?;
                }
                return Ok(Next::View(Kind::Cherry(head.into(), upstream.into())));
            }
            Op::ShortlogRange => {
                return Ok(Next::View(Kind::Shortlog(
                    rev(at(0))?.to_owned(),
                    args.to_vec(),
                )));
            }
        };
        Ok(Next::View(Kind::Log(revs, args.to_vec())))
    }
    /// magit-log-merged without git-when-merged: the oldest first-parent
    /// commit of BRANCH that descends from COMMIT is the merge.
    fn log_merged(&self, commit: &str, branch: &str, args: &[String]) -> Result<Next, String> {
        let lines = |argv: &[&str]| -> Result<Vec<String>, String> {
            Ok(String::from_utf8_lossy(&self.read(argv)?)
                .lines()
                .map(str::to_owned)
                .collect())
        };
        let id = |r: &str| -> Result<String, String> {
            Ok(String::from_utf8_lossy(
                &self
                    .read(&[
                        "rev-parse",
                        "--verify",
                        "-q",
                        "--end-of-options",
                        &format!("{r}^{{commit}}"),
                    ])
                    .map_err(|_| format!("unknown revision {r:?}"))?,
            )
            .trim()
            .to_owned())
        };
        let (c, b) = (id(commit)?, id(branch)?);
        let first_parent = lines(&["rev-list", "--first-parent", &b])?;
        if first_parent.contains(&c) {
            // Commit is directly on this branch: show surrounding history.
            // magit-log-merged-commit-count.
            let half = super::options::int("magit-log-merged-commit-count", 20).max(2) as usize / 2;
            let from = id(&format!("{c}~{half}")).unwrap_or_else(|_| {
                lines(&["rev-list", "--max-parents=0", &c])
                    .ok()
                    .and_then(|v| v.into_iter().next())
                    .unwrap_or(c.clone())
            });
            let ahead = lines(&["rev-list", "--first-parent", &format!("{c}..{b}")])?.len();
            let to = if ahead <= half {
                b.clone()
            } else {
                format!("{b}~{}", ahead - half)
            };
            let mut args = args.to_vec();
            if !args.iter().any(|a| a == "--first-parent") {
                args.insert(0, "--first-parent".into());
            }
            return Ok(Next::View(Kind::Log(vec![format!("{from}..{to}")], args)));
        }
        let ancestry = lines(&["rev-list", "--ancestry-path", &format!("{c}..{b}")])?;
        let m = ancestry
            .iter()
            .rev()
            .find(|x| first_parent.contains(x))
            .ok_or_else(|| format!("Could not find when {commit} was merged into {branch}"))?;
        Ok(Next::View(Kind::Log(
            vec![format!("{m}^1..{m}")],
            args.to_vec(),
        )))
    }
    /// magit-insert-cherry-commits: (+ or -, commit, subject), newest first.
    pub fn cherry(
        &self,
        head: &str,
        upstream: &str,
    ) -> Result<Vec<(char, String, String)>, String> {
        let out = self.read(&["cherry", "-v", upstream, head])?;
        let mut v: Vec<_> = String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| {
                let mut f = l.splitn(3, ' ');
                let sign = f.next()?.chars().next()?;
                Some((
                    sign,
                    f.next()?.to_owned(),
                    f.next().unwrap_or("").to_owned(),
                ))
            })
            .collect();
        v.reverse();
        Ok(v)
    }
    /// magit-git-shortlog.
    pub fn shortlog(&self, rev: &str, args: &[String]) -> Result<Vec<u8>, String> {
        let mut argv = vec!["shortlog"];
        argv.extend(args.iter().map(String::as_str));
        argv.extend([rev, "--"]);
        self.read(&argv)
    }
    /// magit-log-refresh-buffer: git log with the buffer's revisions and arguments.
    pub fn log_lines(
        &self,
        revs: &[String],
        args: &[String],
        files: &[std::path::PathBuf],
    ) -> Result<Vec<Line>, String> {
        if self.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_err()
            && revs.iter().all(|r| r == "HEAD")
        {
            return Ok(vec![]);
        }
        let decorate = args.iter().any(|a| a == "--decorate");
        let mut argv: Vec<std::ffi::OsString> = vec![
            "log".into(),
            "--no-color".into(),
            "--format=%x1e%H%x1f%D%x1f%s".into(),
        ];
        argv.extend(git_args(args)?.into_iter().map(Into::into));
        argv.extend(revs.iter().map(Into::into));
        argv.push("--".into());
        argv.extend(files.iter().map(Into::into));
        // "-- path" arguments (magit-dired-log's files) limit the log.
        for f in args.iter().filter_map(|a| a.strip_prefix("-- ")) {
            argv.push(format!(":(literal){f}").into());
        }
        let out = self.run(&argv, None)?;
        let limit = out.len().min(4 * 1024 * 1024);
        let text = String::from_utf8_lossy(&out[..limit]);
        Ok(text
            .lines()
            .map(|line| {
                match line.split_once('\x1e').filter(|(_, rest)| {
                    // Only our format line: a full object id before the first field
                    // separator (patch text may contain \x1e too).
                    rest.split('\x1f').next().is_some_and(|id| {
                        matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                }) {
                    Some((graph, rest)) => {
                        let f: Vec<&str> = rest.split('\x1f').collect();
                        let get = |i: usize| f.get(i).copied().unwrap_or("");
                        let refs = if decorate && !get(1).is_empty() {
                            format!("({}) ", get(1))
                        } else {
                            String::new()
                        };
                        let id = get(0);
                        // magit-log-wash-rev: hash, refs, then the summary;
                        // author and date go in the margin.
                        Line {
                            text: label(std::path::Path::new(&format!(
                                "{graph}{} {refs}{}",
                                &id[..8],
                                get(2)
                            ))),
                            commit: Some(id.to_owned()).filter(|i| !i.is_empty()),
                        }
                    }
                    None => Line {
                        text: label(std::path::Path::new(
                            &line.chars().take(20_000).collect::<String>(),
                        )),
                        commit: None,
                    },
                }
            })
            .collect())
    }
}
