//! magit-status.el headers and the log/stash sections of magit-status-sections-hook.
use super::Section;
use super::repo::{Commit, Repo, label};
use super::workflows::Stash;
use std::path::Path;

/// Everything in a status buffer besides file sections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extra {
    pub headers: Vec<String>,
    pub stashes: Vec<Stash>,
    /// Log sections in hook order: section, heading, commits.
    pub logs: Vec<(Section, String, Vec<Commit>)>,
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
        };
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
