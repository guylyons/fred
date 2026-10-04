//! magit-refs.el: list and compare branches, remote branches and tags.
use super::repo::Repo;

/// magit-refs-show-commit-count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Count {
    #[default]
    Nothing,
    Branches,
    All,
}

impl Count {
    pub fn next(self) -> Self {
        match self {
            Count::Nothing => Count::Branches,
            Count::Branches => Count::All,
            Count::All => Count::Nothing,
        }
    }
}

/// magit-refs-primary-column-width's minimum and maximum.
const PRIMARY: (usize, usize) = (16, 32);

/// The arguments for-each-ref accepts from the menu (git tag takes the same).
fn ref_args(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| {
            ["--contains=", "--merged", "--no-merged", "--sort="]
                .iter()
                .any(|p| a.starts_with(p))
        })
        .cloned()
        .collect()
}

fn pad(s: &str, width: usize) -> String {
    format!(
        "{s}{}",
        " ".repeat(width.saturating_sub(s.chars().count()).max(1))
    )
}

impl Repo {
    /// magit-rev-diff-count FOCUS REF: (behind, ahead) of REF relative to FOCUS.
    fn diff_count(&self, focus: &str, r: &str) -> Option<(usize, usize)> {
        let out = self
            .read(&[
                "rev-list",
                "--count",
                "--left-right",
                &format!("{focus}...{r}"),
            ])
            .ok()?;
        let s = String::from_utf8_lossy(&out);
        let mut f = s.split_whitespace().map(|n| n.parse().ok());
        Some((f.next()??, f.next()??))
    }
    /// magit-refs--format-focus-column.
    fn focus_column(&self, focus: &str, r: &str, head: bool, count: bool) -> String {
        let width = if count { 5 } else { 1 };
        let text = if r == focus || (head && focus == "HEAD") {
            if focus == "HEAD" { "@" } else { "*" }.to_owned()
        } else if count {
            match self.diff_count(focus, r) {
                Some((_, ahead)) if ahead > 0 => format!("<{ahead}"),
                Some((behind, _)) if behind > 0 => format!("{behind}>"),
                Some(_) => "=".into(),
                None => String::new(),
            }
        } else {
            String::new()
        };
        format!("{text:>width$} ")
    }
    /// magit-refs-sections-hook: (row text, commit to visit). `focus` is the
    /// ref others are compared with.
    pub fn refs_rows(
        &self,
        focus: &str,
        args: &[String],
        count: Count,
    ) -> Result<Vec<(String, Option<String>)>, String> {
        let args = ref_args(args);
        let mut rows: Vec<(String, Option<String>)> = vec![];
        let lines = |extra: &[&str]| -> Result<Vec<Vec<String>>, String> {
            let mut argv: Vec<&str> = vec!["for-each-ref"];
            argv.extend(extra.iter().copied());
            argv.extend(args.iter().map(String::as_str));
            Ok(String::from_utf8_lossy(&self.read(&argv)?)
                .lines()
                .map(|l| l.split('\0').map(str::to_owned).collect())
                .collect())
        };
        let current = self.current_branch().ok();
        // magit-insert-branch-description.
        if let Some(b) = &current
            && let Some(desc) = self.config(&format!("branch.{b}.description"))
        {
            let mut d = desc.lines();
            rows.push((format!("{b}: {}", d.next().unwrap_or("")), None));
            for l in d {
                rows.push((l.to_owned(), None));
            }
            rows.push((String::new(), None));
        }
        // magit-insert-local-branches.
        let local = lines(&[
            "--format=%(HEAD)%00%(refname:short)%00%(objectname)%00%(upstream:short)%00%(upstream:track)%00%(subject)",
            "refs/heads",
        ])?;
        let width = local
            .iter()
            .map(|f| f.get(1).map_or(0, |b| b.chars().count() + 1))
            .max()
            .unwrap_or(0)
            .clamp(PRIMARY.0, PRIMARY.1);
        rows.push(("Branches".into(), None));
        if current.is_none()
            && let Ok(head) = self.read(&["rev-parse", "HEAD"])
        {
            let id = String::from_utf8_lossy(&head).trim().to_owned();
            let subject = self
                .read(&["log", "-1", "--format=%s", "HEAD"])
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                .unwrap_or_default();
            rows.push((
                format!(
                    "{}{}{subject}",
                    self.focus_column(focus, "HEAD", true, count != Count::Nothing),
                    pad("(detached)", width)
                ),
                Some(id),
            ));
        }
        for f in &local {
            let get = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
            let (head, branch, id, upstream, track) =
                (get(0) == "*", get(1), get(2), get(3), get(4));
            let num = |key: &str| {
                track
                    .split(['[', ']', ','])
                    .find_map(|p| p.trim().strip_prefix(key).map(|n| n.trim().to_owned()))
            };
            let ahead = num("ahead").map(|n| format!(" {n}>")).unwrap_or_default();
            let behind = num("behind").map(|n| format!("<{n} ")).unwrap_or_default();
            let name = format!("{branch}{ahead}");
            let upstream = match upstream {
                "" => String::new(),
                u if track == "[gone]" => format!("{u} (gone) "),
                u => format!("{u} "),
            };
            rows.push((
                format!(
                    "{}{}{behind}{upstream}{}",
                    self.focus_column(focus, branch, head, count != Count::Nothing),
                    pad(&name, width),
                    get(5)
                ),
                Some(id.to_owned()),
            ));
        }
        rows.push((String::new(), None));
        // magit-insert-remote-branches.
        for remote in self.remotes()? {
            let url = self.config(&format!("remote.{remote}.url"));
            let push = self.config(&format!("remote.{remote}.pushurl"));
            let urls = [url, push]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
            rows.push((format!("Remote {remote} ({urls}):"), None));
            let refs = lines(&[
                "--format=%(symref:short)%00%(refname:short)%00%(refname)%00%(objectname)%00%(subject)",
                &format!("refs/remotes/{remote}"),
            ])?;
            let head_ref = format!("refs/remotes/{remote}/HEAD");
            let head = refs
                .iter()
                .find(|f| f.get(2) == Some(&head_ref) && !f[0].is_empty())
                .map(|f| f[0].clone());
            for f in refs.iter().filter(|f| f.get(2) != Some(&head_ref)) {
                let get = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
                let short = get(1).strip_prefix(&format!("{remote}/")).unwrap_or(get(1));
                let marker = if head.as_deref() == Some(get(1)) {
                    " (HEAD)"
                } else {
                    ""
                };
                rows.push((
                    format!(
                        "{}{}{}",
                        self.focus_column(focus, get(1), false, count != Count::Nothing),
                        pad(&format!("{short}{marker}"), width),
                        get(4)
                    ),
                    Some(get(3).to_owned()),
                ));
            }
            rows.push((String::new(), None));
        }
        // magit-insert-tags: git tag --list -n.
        let mut argv: Vec<&str> = vec!["tag", "--list", "-n"];
        argv.extend(args.iter().map(String::as_str));
        let tags = String::from_utf8_lossy(&self.read(&argv)?).into_owned();
        let tags: Vec<&str> = tags
            .lines()
            .filter(|l| !l.starts_with([' ', '\t']))
            .collect();
        if !tags.is_empty() {
            rows.push((format!("Tags ({})", tags.len()), None));
            for line in tags {
                let (tag, msg) = line
                    .split_once([' ', '\t'])
                    .map_or((line, ""), |(t, m)| (t, m.trim()));
                let id = self
                    .read(&[
                        "rev-parse",
                        "--verify",
                        "-q",
                        &format!("refs/tags/{tag}^{{commit}}"),
                    ])
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned());
                rows.push((
                    format!(
                        "{}{}{msg}",
                        self.focus_column(focus, tag, false, count == Count::All),
                        pad(tag, width)
                    ),
                    id,
                ));
            }
        }
        Ok(rows)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// magit-show-refs-head.
    Head,
    /// magit-show-refs-current.
    Current,
    /// magit-show-refs-other.
    Other,
    /// magit-refs-set-show-commit-count (cycles in a refs buffer).
    Count,
}

impl Repo {
    /// The ref to compare with, or a question for magit-show-refs-other.
    pub fn refs_focus(&self, op: &Op, answer: Option<&str>) -> Result<String, String> {
        match op {
            Op::Head => Ok("HEAD".into()),
            Op::Current => Ok(self.current_branch().unwrap_or_else(|_| "HEAD".into())),
            _ => {
                let r = answer.unwrap_or("").trim();
                if r.is_empty() || r.starts_with('-') || r.chars().any(char::is_control) {
                    return Err(format!("invalid ref {r:?}"));
                }
                self.read(&["rev-parse", "--verify", "-q", "--end-of-options", r])
                    .map_err(|_| format!("unknown ref {r:?}"))?;
                Ok(r.to_owned())
            }
        }
    }
}
