//! magit-tag.el: create, release, delete and prune tags.
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create,
    Release,
    /// The release tag is known; read its message.
    ReleaseMessage(String),
    Delete,
    Prune,
    /// Remote, tags only local, tags only remote: confirm local deletion.
    PruneLocal(String, Vec<String>, Vec<String>),
    /// Remote and tags only remote: confirm deletion from the remote.
    PruneRemote(String, Vec<String>),
}

/// magit-release-tag-regexp.
fn release_re() -> regex::Regex {
    regex::Regex::new(
        r"^((?:v(?:ersion)?|r(?:elease)?)[-_/]?)?([0-9]+(?:\.[0-9]+)*(?:-[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*)?)$",
    )
    .expect("valid release regexp")
}

/// version-to-list with magit-tag-version-regexp-alist; None if unparsable.
pub fn version_list(v: &str) -> Option<Vec<i64>> {
    let mut out = vec![];
    let mut chars = v.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut n = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                n.push(d);
                chars.next();
            }
            out.push(n.parse().ok()?);
        } else if "-._+ ".contains(c) {
            chars.next();
        } else {
            let mut w = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_alphabetic()) {
                w.push(d.to_ascii_lowercase());
                chars.next();
            }
            out.push(match w.as_str() {
                "snapshot" | "cvs" | "git" | "bzr" | "svn" | "hg" | "darcs" | "unknown" => -4,
                "alpha" => -3,
                "beta" => -2,
                "pre" | "rc" => -1,
                _ => return None,
            });
        }
    }
    Some(out)
}
fn version_cmp(a: &[i64], b: &[i64]) -> std::cmp::Ordering {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn annotating(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--annotate" || a == "--sign" || a.starts_with("--local-user"))
}

impl Repo {
    /// magit--list-releases: (version, tag, message), highest first.
    pub fn releases(&self) -> Vec<(String, String, String)> {
        let re = release_re();
        let Ok(out) = self.read(&["tag", "-n"]) else {
            return vec![];
        };
        let mut list: Vec<_> = String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|line| {
                let (tag, msg) = line.split_once(' ')?;
                let ver = re.captures(tag)?.get(2)?.as_str().to_owned();
                Some((
                    version_list(&ver)?,
                    ver,
                    tag.to_owned(),
                    msg.trim().to_owned(),
                ))
            })
            .collect();
        list.sort_by(|a, b| version_cmp(&b.0, &a.0));
        list.into_iter().map(|(_, v, t, m)| (v, t, m)).collect()
    }
    fn tags(&self) -> Vec<String> {
        self.read(&["tag", "--list"])
            .map(|o| {
                String::from_utf8_lossy(&o)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    fn tag_name(&self, t: &str) -> Result<String, String> {
        if t.is_empty() || t.starts_with('-') || t.chars().any(char::is_control) {
            return Err(format!("invalid tag name {t:?}"));
        }
        self.read(&["check-ref-format", &format!("refs/tags/{t}")])
            .map_err(|_| format!("{t:?} is not a valid tag name"))?;
        Ok(t.to_owned())
    }

    pub fn tag_prompts(
        &self,
        op: &Op,
        args: &[String],
        at_point: Option<String>,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let here = self.current_branch().unwrap_or_else(|_| "HEAD".into());
        Ok(match op {
            Op::Create => {
                let mut p = vec![
                    "Create tag: ".to_owned(),
                    format!("Place tag on (default {here}): "),
                ];
                if annotating(args) || args.iter().any(|a| a == "--edit") {
                    p.push("Tag message: ".into());
                }
                let d = vec![String::new(), here, String::new()];
                (p, d)
            }
            Op::Release => {
                let releases = self.releases();
                let subject = self
                    .read(&["log", "-1", "--format=%s"])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .unwrap_or_default();
                let ver = subject.strip_prefix("Release version ").map(str::to_owned);
                match (releases.first(), ver) {
                    (None, ver) => {
                        let default = ver
                            .map(|v| {
                                if v.starts_with(|c: char| c.is_ascii_digit()) {
                                    format!("v{v}")
                                } else {
                                    v
                                }
                            })
                            .unwrap_or_default();
                        (
                            vec![format!("Create first release tag (default {default}): ")],
                            vec![default],
                        )
                    }
                    // A tag named after the release commit needs no question.
                    (Some((_, ptag, _)), Some(v)) => {
                        let prefix = release_re()
                            .captures(ptag)
                            .and_then(|c| c.get(1))
                            .map_or("", |m| m.as_str())
                            .to_owned();
                        (vec![], vec![format!("{prefix}{v}")])
                    }
                    (Some((_, ptag, _)), None) => (
                        vec![format!("Create release tag (previous was {ptag}): ")],
                        vec![ptag.clone()],
                    ),
                }
            }
            Op::Delete => {
                let d = at_point.unwrap_or_default();
                (vec![format!("Delete tag (default {d}): ")], vec![d])
            }
            Op::Prune => {
                let remotes = self.remotes()?;
                let d = if remotes.len() == 1 {
                    remotes[0].clone()
                } else {
                    String::new()
                };
                (
                    vec![format!("Prune tags using remote (default {d}): ")],
                    vec![d],
                )
            }
            _ => return Err("not a menu suffix".into()),
        })
    }

    /// Run one answered step; `a` holds answers with defaults applied.
    pub fn tag_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let invocation = |name: &str, rev: Option<&str>, msg: Option<&str>| {
            // Fred has no editor for git; a message read here replaces --edit.
            let mut argv: Vec<String> = vec!["tag".into()];
            argv.extend(args.iter().filter(|x| *x != "--edit").cloned());
            if let Some(m) = msg {
                argv.extend(["-m".into(), m.to_owned()]);
            }
            argv.extend(["--".into(), name.to_owned()]);
            argv.extend(rev.map(str::to_owned));
            Next::Git(argv)
        };
        match op {
            Op::Create => {
                let name = self.tag_name(at(0))?;
                let rev = at(1);
                if rev.starts_with('-') || rev.is_empty() {
                    return Err(format!("invalid revision {rev:?}"));
                }
                self.read(&["rev-parse", "--verify", "-q", "--end-of-options", rev])
                    .map_err(|_| format!("unknown revision {rev:?}"))?;
                let wants_message = annotating(args) || args.iter().any(|x| x == "--edit");
                let msg = Some(at(2)).filter(|m| wants_message && !m.is_empty());
                if wants_message && msg.is_none() {
                    return Err("A tag message is required".into());
                }
                Ok(invocation(&name, Some(rev), msg))
            }
            Op::Release => {
                let tag = self.tag_name(at(0))?;
                let first = self.releases().is_empty();
                if !first && !annotating(args) {
                    return Ok(invocation(&tag, None, None));
                }
                // magit-tag-release: derive the message from the previous release.
                let ver = release_re()
                    .captures(&tag)
                    .and_then(|c| c.get(2))
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_else(|| tag.clone());
                let message = match self.releases().first() {
                    Some((pver, _, pmsg)) if pmsg.contains(pver.as_str()) => {
                        pmsg.replacen(pver.as_str(), &ver, 1)
                    }
                    Some((_, ptag, pmsg)) if pmsg.contains(ptag.as_str()) => {
                        pmsg.replacen(ptag.as_str(), &tag, 1)
                    }
                    _ => {
                        let name = self
                            .root
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let mut c = name.chars();
                        let cap = c
                            .next()
                            .map(|f| f.to_uppercase().chain(c).collect::<String>())
                            .unwrap_or_default();
                        format!("{cap} {ver}")
                    }
                };
                Ok(Next::Ask(
                    super::Question::Tag(Op::ReleaseMessage(tag)),
                    vec![format!("Tag message (default {message}): ")],
                    vec![message],
                ))
            }
            Op::ReleaseMessage(tag) => Ok(invocation(&tag, None, Some(at(0)))),
            Op::Delete => {
                let name = self.tag_name(at(0))?;
                self.read(&["tag", "-d", "--", &name]).map(|_| ())?;
                Ok(Next::Done(Ok(format!("Deleted tag {name}"))))
            }
            Op::Prune => {
                let remote = at(0);
                if !self.remotes()?.iter().any(|r| r == remote) {
                    return Err(format!("no remote named {remote:?}"));
                }
                let out = self.read(&["ls-remote", "--tags", "--refs", remote])?;
                let rtags: Vec<String> = String::from_utf8_lossy(&out)
                    .lines()
                    .filter_map(|l| l.split_once("refs/tags/").map(|(_, t)| t.to_owned()))
                    .collect();
                let tags = self.tags();
                let local: Vec<_> = tags
                    .iter()
                    .filter(|t| !rtags.contains(t))
                    .cloned()
                    .collect();
                let remote_only: Vec<_> = rtags
                    .iter()
                    .filter(|t| !tags.contains(t))
                    .cloned()
                    .collect();
                if local.is_empty() && remote_only.is_empty() {
                    return Ok(Next::Done(
                        Ok("Same tags exist locally and remotely".into()),
                    ));
                }
                self.prune_next(remote.to_owned(), local, remote_only)
            }
            Op::PruneLocal(remote, local, remote_only) => {
                if matches!(at(0), "y" | "yes") {
                    let mut argv = vec!["tag", "-d", "--"];
                    argv.extend(local.iter().map(String::as_str));
                    self.read(&argv)?;
                }
                self.prune_next(remote, vec![], remote_only)
            }
            Op::PruneRemote(remote, remote_only) => {
                if !matches!(at(0), "y" | "yes") {
                    return Ok(Next::Done(Ok("Kept remote tags".into())));
                }
                let mut argv = vec!["push".to_owned(), remote];
                argv.extend(remote_only.iter().map(|t| format!(":refs/tags/{t}")));
                Ok(Next::Git(argv))
            }
        }
    }
    fn prune_next(
        &self,
        remote: String,
        local: Vec<String>,
        remote_only: Vec<String>,
    ) -> Result<Next, String> {
        let what = |v: &[String], place: &str| match v {
            [one] => format!("Delete {one} {place}? (y or n) "),
            many => format!("Delete {} tags {place}? (y or n) ", many.len()),
        };
        if !local.is_empty() {
            return Ok(Next::Ask(
                super::Question::Tag(Op::PruneLocal(remote, local.clone(), remote_only)),
                vec![what(&local, "locally")],
                vec![String::new()],
            ));
        }
        if !remote_only.is_empty() {
            return Ok(Next::Ask(
                super::Question::Tag(Op::PruneRemote(remote, remote_only.clone())),
                vec![what(&remote_only, "from remote")],
                vec![String::new()],
            ));
        }
        Ok(Next::Done(Ok("Pruned tags".into())))
    }
}
