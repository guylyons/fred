//! magit-status.el headers and the log/stash sections of magit-status-sections-hook.
use super::Section;
use super::repo::{Commit, Repo, label};
use super::workflows::Stash;
use std::path::Path;

/// An in-progress section: section, heading, rows (text, commit to visit).
pub type SequenceSection = (Section, String, Vec<(String, Option<String>)>);
/// Everything in a status buffer besides file sections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extra {
    pub headers: Vec<String>,
    pub stashes: Vec<Stash>,
    /// Log sections in hook order: section, heading, commits.
    pub logs: Vec<(Section, String, Vec<Commit>)>,
    /// In-progress sections (merge, rebase, am, sequencer, bisect): section,
    /// heading and rows (text, commit to visit).
    pub sequences: Vec<SequenceSection>,
}

/// magit-log-section-commit-count.
fn recent() -> usize {
    super::options::int("magit-log-section-commit-count", 10).max(0) as usize
}
/// magit-status buffer log arguments default to -n256.
pub const LIMIT: usize = 256;

fn header(keyword: &str, text: &str) -> String {
    format!("{keyword:<10}{text}")
}

impl Repo {
    fn summary(&self, rev: &str) -> Option<String> {
        let out = self
            .read(&["log", "-1", "--format=%s", "--end-of-options", rev, "--"])
            .ok()?;
        let s = String::from_utf8_lossy(&out).trim().to_owned();
        Some(if s.is_empty() {
            "(no commit message)".into()
        } else {
            label(Path::new(&s))
        })
    }
    fn verify(&self, rev: &str) -> bool {
        self.read(&["rev-parse", "--verify", "-q", "--end-of-options", rev])
            .is_ok()
    }
    fn short_ref(&self, rev: &str) -> Option<String> {
        let out = self
            // Internal revisions only ("HEAD", "BRANCH@{upstream}"); rev-parse
            // would echo --end-of-options here.
            .read(&["rev-parse", "--abbrev-ref", rev])
            .ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_owned()).filter(|s| !s.is_empty())
    }
    fn git_dir_file(&self, name: &str) -> Option<String> {
        let p = self
            .read(&["rev-parse", "--path-format=absolute", "--git-path", name])
            .ok()?;
        std::fs::read_to_string(String::from_utf8_lossy(&p).trim()).ok()
    }
    /// "%h %s" of a revision, or None.
    fn summary_line(&self, rev: &str) -> Option<(String, String)> {
        let out = self
            .read(&[
                "log",
                "-1",
                "--no-walk",
                "--format=%H%x00%h %s",
                "--end-of-options",
                rev,
                "--",
            ])
            .ok()?;
        let s = String::from_utf8_lossy(&out);
        let (full, text) = s.trim_end().split_once('\0')?;
        Some((full.to_owned(), text.to_owned()))
    }
    /// magit-sequence-insert-sequence: the stopped commit, the done commits
    /// (newest first) and onto.
    fn sequence_rows(&self, stop: Option<&str>, onto: &str) -> Vec<(String, Option<String>)> {
        let mut rows = vec![];
        let head = self
            .read(&["rev-parse", "HEAD"])
            .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
            .unwrap_or_default();
        let done: Vec<(String, String)> = self
            .read(&[
                "log",
                "--format=%H%x00%h %s",
                &format!("{onto}..HEAD"),
                "--",
            ])
            .map(|o| {
                String::from_utf8_lossy(&o)
                    .lines()
                    .filter_map(|l| {
                        l.split_once('\0')
                            .map(|(a, b)| (a.to_owned(), b.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let stop = stop.and_then(|s| self.summary_line(s));
        if let Some((full, text)) = &stop
            && !done.iter().any(|(d, _)| d == full)
        {
            let unmerged = self
                .read(&["ls-files", "--unmerged"])
                .is_ok_and(|o| !o.is_empty());
            let modified = self.read(&["diff", "--quiet", "HEAD"]).is_err();
            let kind = if unmerged {
                "join"
            } else if modified {
                "work"
            } else {
                "gone"
            };
            rows.push((format!("{kind} {text}"), Some(full.clone())));
        }
        for (full, text) in &done {
            let kind = if stop.as_ref().is_some_and(|(s, _)| s == full) {
                if *full == head { "stop" } else { "like" }
            } else {
                "done"
            };
            rows.push((format!("{kind} {text}"), Some(full.clone())));
        }
        if let Some((full, text)) = self.summary_line(onto) {
            rows.push((format!("onto {text}"), Some(full)));
        }
        rows
    }
    /// magit-insert-merge-log, -rebase-sequence, -am-sequence,
    /// -sequencer-sequence and the bisect output, rest and log sections.
    fn sequence_sections(&self) -> Vec<SequenceSection> {
        let mut out = vec![];
        let file = |n: &str| self.git_dir_file(n);
        // Merge in progress: the commits being merged.
        if let Some(heads) = file("MERGE_HEAD") {
            let heads: Vec<&str> = heads.split_whitespace().collect();
            let names: Vec<String> = heads
                .iter()
                .map(|h| {
                    self.read(&[
                        "name-rev",
                        "--name-only",
                        "--no-undefined",
                        "--refs=refs/heads/*",
                        h,
                    ])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .unwrap_or_else(|_| h[..h.len().min(7)].to_owned())
                })
                .collect();
            let rows = heads
                .first()
                .and_then(|h| {
                    self.read(&["log", "--format=%H%x00%h %s", &format!("HEAD..{h}"), "--"])
                        .ok()
                })
                .map(|o| {
                    String::from_utf8_lossy(&o)
                        .lines()
                        .filter_map(|l| l.split_once('\0'))
                        .map(|(full, text)| (text.to_owned(), Some(full.to_owned())))
                        .collect()
                })
                .unwrap_or_default();
            out.push((
                Section::Merging,
                format!("Merging {}:", names.join(", ")),
                rows,
            ));
        }
        // Rebase (merge backend) in progress.
        if let Some(onto) = file("rebase-merge/onto") {
            let onto = onto.trim().to_owned();
            let name = file("rebase-merge/head-name")
                .map(|n| n.trim().trim_start_matches("refs/heads/").to_owned())
                .unwrap_or_default();
            let onto_name = self
                .read(&[
                    "name-rev",
                    "--name-only",
                    "--no-undefined",
                    "--refs=refs/heads/*",
                    &onto,
                ])
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                .unwrap_or_else(|_| onto[..onto.len().min(7)].to_owned());
            let mut rows = vec![];
            // The remaining todo, last to be applied first.
            if let Some(todo) = file("rebase-merge/git-rebase-todo") {
                for line in todo
                    .lines()
                    .rev()
                    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                {
                    let mut w = line.splitn(3, ' ');
                    let (cmd, target) = (w.next().unwrap_or(""), w.next().unwrap_or(""));
                    let commit = self.summary_line(target).map(|(f, _)| f);
                    rows.push((
                        line.to_owned(),
                        commit.filter(|_| {
                            !matches!(cmd, "exec" | "x" | "label" | "l" | "reset" | "t")
                        }),
                    ));
                }
            }
            let stop = file("rebase-merge/stopped-sha").map(|s| s.trim().to_owned());
            rows.extend(self.sequence_rows(stop.as_deref(), &onto));
            out.push((
                Section::Sequence,
                format!("Rebasing {name} onto {onto_name}"),
                rows,
            ));
        } else if file("rebase-apply/applying").is_some() {
            // magit-insert-am-sequence.
            let next: usize = file("rebase-apply/next")
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or(1);
            let last: usize = file("rebase-apply/last")
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or(0);
            let mut rows = vec![];
            for i in (next..=last).rev() {
                let patch = file(&format!("rebase-apply/{i:04}")).unwrap_or_default();
                let subject = patch
                    .lines()
                    .find_map(|l| l.strip_prefix("Subject: "))
                    .unwrap_or("")
                    .trim_start_matches("[PATCH] ")
                    .to_owned();
                let kind = if i == next { "stop" } else { "pick" };
                rows.push((format!("{kind} {i:04} {subject}"), None));
            }
            rows.extend(self.sequence_rows(None, "ORIG_HEAD"));
            out.push((Section::Sequence, "Applying patches".into(), rows));
        } else if let Some(onto) = file("rebase-apply/onto") {
            let name = file("rebase-apply/head-name")
                .map(|n| n.trim().trim_start_matches("refs/heads/").to_owned())
                .unwrap_or_default();
            let stop = file("rebase-apply/original-commit").map(|s| s.trim().to_owned());
            let rows = self.sequence_rows(stop.as_deref(), onto.trim());
            out.push((
                Section::Sequence,
                format!("Rebasing {name} onto {}", onto.trim()),
                rows,
            ));
        }
        // Cherry-pick or revert sequence.
        let picking = file("CHERRY_PICK_HEAD").is_some();
        if picking || file("REVERT_HEAD").is_some() {
            let mut rows = vec![];
            if let Some(todo) = file("sequencer/todo") {
                for line in todo.lines().skip(1).collect::<Vec<_>>().into_iter().rev() {
                    let mut w = line.splitn(3, ' ');
                    let (cmd, hash) = (w.next().unwrap_or(""), w.next().unwrap_or(""));
                    if matches!(cmd, "pick" | "revert") {
                        rows.push((line.to_owned(), self.summary_line(hash).map(|(f, _)| f)));
                    }
                }
            }
            let stop = file(if picking {
                "CHERRY_PICK_HEAD"
            } else {
                "REVERT_HEAD"
            })
            .map(|s| s.trim().to_owned());
            let onto = file("sequencer/head")
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|| "HEAD".into());
            rows.extend(self.sequence_rows(stop.as_deref(), &onto));
            out.push((
                Section::Sequence,
                if picking {
                    "Cherry Picking"
                } else {
                    "Reverting"
                }
                .into(),
                rows,
            ));
        }
        // Bisecting: output, the rest to test, and the log.
        if file("BISECT_LOG").is_some() {
            let output = file("BISECT_CMD_OUTPUT");
            let mut lines: Vec<String> = match &output {
                Some(o) => o.lines().map(str::to_owned).collect(),
                None => vec![
                    "Bisecting: (no saved bisect output)".into(),
                    "It appears you have invoked \"git bisect\" from a shell.".into(),
                ],
            };
            let heading = if lines.is_empty() {
                String::new()
            } else {
                lines.remove(0)
            };
            out.push((
                Section::BisectOutput,
                heading,
                lines.into_iter().map(|l| (l, None)).collect(),
            ));
            let mut argv = vec![
                "bisect",
                "visualize",
                "git",
                "log",
                "--format=%H%x00%h %D %s",
            ];
            if super::options::flag("magit-bisect-show-graph", true) {
                argv.push("--graph");
            }
            let rest = self
                .read(&argv)
                .map(|o| {
                    String::from_utf8_lossy(&o)
                        .lines()
                        .map(|l| match l.split_once('\0') {
                            Some((pre, text)) => {
                                // The graph, then the full hash.
                                let split = pre.len().saturating_sub(40);
                                let (graph, full) = pre.split_at(split);
                                (format!("{graph}{text}"), Some(full.to_owned()))
                            }
                            None => (l.to_owned(), None),
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push((Section::BisectRest, "Bisect Rest".into(), rest));
            let log = self
                .read(&["bisect", "log"])
                .map(|o| {
                    String::from_utf8_lossy(&o)
                        .lines()
                        .filter(|l| !l.starts_with("# status:"))
                        .map(|l| (l.trim_start_matches("# ").to_owned(), None))
                        .collect()
                })
                .unwrap_or_default();
            out.push((Section::BisectLog, "Bisect Log".into(), log));
        }
        out
    }
    /// Commits of a range for a status section (`git log RANGE`).
    pub fn log_range(&self, range: &str, limit: Option<usize>) -> Vec<Commit> {
        let limit = limit.map(|n| format!("-{n}"));
        let mut args = vec!["log", "--format=%H%x00%s%x00%an%x00%ad%x00", "--date=short"];
        args.extend(limit.as_deref());
        args.extend(["--end-of-options", range, "--"]);
        let Ok(bytes) = self.read(&args) else {
            return vec![];
        };
        let fields: Vec<_> = bytes.split(|b| *b == 0).collect();
        fields
            .chunks(4)
            .filter(|c| c.len() == 4)
            .map(|c| Commit {
                id: String::from_utf8_lossy(c[0]).trim().to_owned(),
                subject: String::from_utf8_lossy(c[1]).into(),
                author: String::from_utf8_lossy(c[2]).into(),
                date: String::from_utf8_lossy(c[3]).into(),
            })
            .filter(|c| !c.id.is_empty())
            .collect()
    }

    /// magit-status-headers-hook: head, upstream, push and tags.
    fn status_headers(&self, branch: Option<&str>) -> Vec<String> {
        let mut out = vec![];
        let born = self.verify("HEAD");
        out.push(match (branch, born) {
            (Some(b), true) => header(
                "Head:",
                &format!(
                    "{} {}",
                    label(Path::new(b)),
                    self.summary("HEAD").unwrap_or_default()
                ),
            ),
            (Some(b), false) => header(
                "Head:",
                &format!("{} (no commits yet)", label(Path::new(b))),
            ),
            (None, _) => {
                let hash = self.short_ref("HEAD").and_then(|_| {
                    self.read(&["rev-parse", "--short", "HEAD"])
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                });
                header(
                    "Head:",
                    &format!(
                        "{} {}",
                        hash.unwrap_or_default(),
                        self.summary("HEAD").unwrap_or_default()
                    ),
                )
            }
        });
        let Some(branch) = branch else {
            return out;
        };
        // magit-insert-upstream-branch-header.
        let remote = self.config(&format!("branch.{branch}.remote"));
        let merge = self.config(&format!("branch.{branch}.merge"));
        if remote.is_some() || merge.is_some() {
            // Upstream's pcase: only "true"/"false" decide; anything else falls back to
            // pull.rebase read as a boolean (magit-get-boolean).
            let rebase = match self.config(&format!("branch.{branch}.rebase")).as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => self
                    .read(&["config", "--bool", "--get", "pull.rebase"])
                    .is_ok_and(|o| o.starts_with(b"true")),
            };
            let keyword = if rebase { "Rebase:" } else { "Merge:" };
            let remotes = self.remotes().unwrap_or_default();
            let upstream = self
                .verify(&format!("{branch}@{{upstream}}"))
                .then(|| self.short_ref(&format!("{branch}@{{upstream}}")))
                .flatten();
            // magit--unnamed-upstream-p and magit--valid-upstream-p; config values are
            // labelled so an embedded newline cannot split the row.
            let unnamed = |r: &str| {
                r.starts_with('/')
                    || r.starts_with("./")
                    || r.starts_with("../")
                    || r.contains([':', '@'])
            };
            let text = match (&upstream, &remote, &merge) {
                (Some(u), _, _) => format!(
                    "{} {}",
                    label(Path::new(u)),
                    self.summary(u).unwrap_or_default()
                ),
                (None, Some(r), Some(m)) if unnamed(r) && m.starts_with("refs/") => {
                    format!("{} from {}", label(Path::new(m)), label(Path::new(r)))
                }
                (None, Some(r), Some(m))
                    if (r == "." || remotes.contains(r)) && m.starts_with("refs/") =>
                {
                    if r == "." {
                        format!("{} does not exist", label(Path::new(m)))
                    } else {
                        format!(
                            "{} does not exist on {}",
                            label(Path::new(m)),
                            label(Path::new(r))
                        )
                    }
                }
                _ => "invalid upstream configuration".into(),
            };
            out.push(header(keyword, &text));
        }
        // magit-insert-push-branch-header.
        let push = self
            .config(&format!("branch.{branch}.pushRemote"))
            .or_else(|| self.config("remote.pushDefault"));
        if let Some(remote) = push {
            let target = format!("{remote}/{branch}");
            let text = if self.verify(&format!("refs/remotes/{target}")) {
                format!(
                    "{} {}",
                    label(Path::new(&target)),
                    self.summary(&format!("refs/remotes/{target}"))
                        .unwrap_or_default()
                )
            } else if self.remotes().unwrap_or_default().contains(&remote) {
                format!("{} does not exist", label(Path::new(&target)))
            } else {
                format!("{} remote does not exist", label(Path::new(&remote)))
            };
            out.push(header("Push:", &text));
        }
        // magit-insert-tags-header.
        if born {
            let this = self
                .read(&["describe", "--long", "--tags", "HEAD"])
                .ok()
                .and_then(|o| {
                    let s = String::from_utf8_lossy(&o).trim().to_owned();
                    let (rest, _hash) = s.rsplit_once('-')?;
                    let (tag, count) = rest.rsplit_once('-')?;
                    Some((tag.to_owned(), count.parse::<usize>().unwrap_or(0)))
                });
            let next = self
                .read(&["describe", "--contains", "HEAD"])
                .ok()
                .and_then(|o| {
                    let s = String::from_utf8_lossy(&o).trim().to_owned();
                    let tag = s.split(['~', '^']).next()?.to_owned();
                    if this.as_ref().is_some_and(|(t, _)| *t == tag) {
                        return None;
                    }
                    let count = self
                        .read(&["rev-list", "--count", &format!("HEAD..{tag}"), "--"])
                        .ok()
                        .and_then(|o| String::from_utf8_lossy(&o).trim().parse().ok())
                        .unwrap_or(0usize);
                    Some((tag, count))
                });
            let fmt = |(t, n): &(String, usize)| {
                if *n > 0 {
                    format!("{} ({n})", label(Path::new(t)))
                } else {
                    label(Path::new(t))
                }
            };
            match (&this, &next) {
                (Some(a), Some(b)) => out.push(header("Tags:", &format!("{}, {}", fmt(a), fmt(b)))),
                (Some(a), None) | (None, Some(a)) => out.push(header("Tag:", &fmt(a))),
                (None, None) => (),
            }
        }
        out
    }

    /// magit-status-sections-hook after the file sections.
    pub fn status_extra(&self) -> Extra {
        let branch = self.current_branch().ok();
        let mut extra = Extra {
            headers: self.status_headers(branch.as_deref()),
            stashes: self.stashes().unwrap_or_default(),
            logs: vec![],
            sequences: vec![],
        };
        extra.sequences = self.sequence_sections();
        if !self.verify("HEAD") {
            return extra;
        }
        let upstream = branch
            .as_ref()
            .filter(|b| self.verify(&format!("{b}@{{upstream}}")))
            .and_then(|b| self.short_ref(&format!("{b}@{{upstream}}")));
        let push = branch.as_ref().and_then(|b| {
            let remote = self.push_remote(b).ok()??;
            let target = format!("{remote}/{b}");
            self.verify(&format!("refs/remotes/{target}"))
                .then_some(target)
        });
        // magit--insert-pushremote-log-p: skip push sections that duplicate upstream.
        let push = push.filter(|p| Some(p) != upstream.as_ref());
        let mut add = |section, heading: String, commits: Vec<Commit>| {
            if !commits.is_empty() {
                extra.logs.push((section, heading, commits));
            }
        };
        if let Some(p) = &push {
            add(
                Section::UnpushedPush,
                format!("Unpushed to {p}"),
                self.log_range(&format!("refs/remotes/{p}..HEAD"), Some(LIMIT)),
            );
        }
        // magit-insert-unpushed-to-upstream-or-recent.
        match &upstream {
            Some(u)
                if !self
                    .read(&["merge-base", "--is-ancestor", "HEAD", "@{upstream}"])
                    .is_ok() =>
            {
                add(
                    Section::UnpushedUpstream,
                    format!("Unmerged into {u}"),
                    self.log_range("@{upstream}..HEAD", Some(LIMIT)),
                );
            }
            _ => add(
                Section::UnpushedUpstream,
                "Recent commits".into(),
                self.log_range("HEAD", Some(recent())),
            ),
        }
        if let Some(p) = &push {
            add(
                Section::UnpulledPush,
                format!("Unpulled from {p}"),
                self.log_range(&format!("HEAD..refs/remotes/{p}"), Some(LIMIT)),
            );
        }
        if let Some(u) = &upstream {
            add(
                Section::UnpulledUpstream,
                format!("Unpulled from {u}"),
                self.log_range("HEAD..@{upstream}", Some(LIMIT)),
            );
        }
        extra
    }
}
