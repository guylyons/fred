//! magit-clone.el: regular, shallow, bare, mirror and sparse clones.
//! The clone runs in the background (no credential prompts), then the new
//! repository's status opens.
use super::branch::Next;
use super::repo::Repo;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Regular,
    Shallow,
    ShallowSince,
    ShallowExclude,
    Bare,
    Mirror,
    Sparse,
}

/// magit-clone--url-to-name.
pub fn url_to_name(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_owned())
}

/// magit-clone--name-to-url: a name per magit-clone-name-alist and
/// magit-clone-url-format; `user` is a Git variable when it has a dot.
pub fn name_to_url(name: &str, config: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    use super::options;
    let alist: Vec<(String, String, String)> = match options::value("magit-clone-name-alist") {
        Some(toml::Value::Array(a)) => a
            .iter()
            .filter_map(|e| {
                let e = e.as_array()?;
                Some((
                    e.first()?.as_str()?.to_owned(),
                    e.get(1)?.as_str()?.to_owned(),
                    e.get(2)?.as_str()?.to_owned(),
                ))
            })
            .collect(),
        _ => [
            (
                r"\`\(?:github:\|gh:\)?\([^:]+\)\'",
                "github.com",
                "github.user",
            ),
            (
                r"\`\(?:gitlab:\|gl:\)\([^:]+\)\'",
                "gitlab.com",
                "gitlab.user",
            ),
            (
                r"\`\(?:sourcehut:\|sh:\)\([^:]+\)\'",
                "git.sr.ht",
                "sourcehut.user",
            ),
        ]
        .iter()
        .map(|(r, h, u)| (r.to_string(), h.to_string(), u.to_string()))
        .collect(),
    };
    for (re, host, user) in alist {
        let Ok(re) = regex::Regex::new(&options::emacs_regex(&re)) else {
            continue;
        };
        let Some(repo) = re
            .captures(name)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_owned())
        else {
            continue;
        };
        let format = match options::value("magit-clone-url-format") {
            Some(toml::Value::String(f)) => f,
            Some(toml::Value::Table(t)) => t
                .get(&host)
                .or_else(|| t.get("t"))
                .and_then(|v| v.as_str())
                .ok_or("Bogus `magit-clone-url-format' (bad type or missing default)")?
                .to_owned(),
            _ if host == "git.sr.ht" => "git@%h:%n".into(),
            _ => "git@%h:%n.git".into(),
        };
        let full = if repo.contains('/') {
            repo
        } else if user.contains('.') {
            let u = config(&user).ok_or(format!("Set {user:?} or specify owner explicitly"))?;
            format!("{u}/{repo}")
        } else {
            format!("{user}/{repo}")
        };
        return Ok(format.replace("%h", &host).replace("%n", &full));
    }
    Err("Not an url and no matching entry in `magit-clone-name-alist'".into())
}

fn value(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid value {v:?}"));
    }
    Ok(v)
}

/// The base directory clones are relative to (Fred's current directory).
impl Repo {
    pub fn clone_prompts(op: &Op) -> (Vec<String>, Vec<String>) {
        let mut prompts = vec![
            "Clone from url or name: ".to_owned(),
            "Clone to (default name from url): ".into(),
        ];
        match op {
            Op::ShallowSince => prompts.push("Exclude commits before: ".into()),
            Op::ShallowExclude => prompts.push("Exclude commits reachable from: ".into()),
            _ => {}
        }
        // magit-clone-set-remote.pushDefault ask.
        if !matches!(op, Op::Bare | Op::Mirror) && push_default() == Some(true) {
            prompts.push("Set `remote.pushDefault' to the remote? (y or n) ".into());
        }
        let defaults = vec![String::new(); prompts.len()];
        (prompts, defaults)
    }
    /// `self.root` is the directory relative answers resolve against.
    pub fn clone_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        // magit-clone-read-repository: urls and paths as given, names through
        // magit-clone-name-alist.
        let given = value(at(0))?;
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let base = super::options::string("magit-clone-default-directory", None)
            .map(|d| match d.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(d),
            })
            .unwrap_or_else(|| self.root.clone());
        let url_owned = if given.contains("://")
            || given.contains('@')
            || self.root.join(given).exists()
            || given.starts_with(['/', '.', '~'])
        {
            given.to_owned()
        } else {
            name_to_url(given, |k| self.config(k))?
        };
        let url = url_owned.as_str();
        let name = url_to_name(url);
        let mut dir = match at(1) {
            "" => base.join(name.clone().ok_or("Cannot derive a directory name")?),
            d => match d.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => base.join(d),
            },
        };
        let extra = usize::from(matches!(op, Op::ShallowSince | Op::ShallowExclude));
        let set_push = !matches!(op, Op::Bare | Op::Mirror)
            && match push_default() {
                Some(true) => matches!(at(2 + extra), "y" | "yes"),
                Some(false) => true,
                None => false,
            };
        // magit-clone-internal: an existing non-empty directory gets the
        // repository's name inside it.
        if dir.exists() {
            if !dir.is_dir() {
                return Err(format!(
                    "{} already exists and is not a directory",
                    dir.display()
                ));
            }
            if std::fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
            {
                let inner = name.map(|n| dir.join(n)).filter(|d| !d.exists());
                dir = inner.ok_or_else(|| format!("{} already exists", dir.display()))?;
            }
        }
        let mut argv: Vec<String> = vec!["clone".into()];
        match op {
            Op::Shallow => argv.push("--depth=1".into()),
            Op::ShallowSince => argv.push(format!("--shallow-since={}", value(at(2))?)),
            Op::ShallowExclude => argv.push(format!("--shallow-exclude={}", value(at(2))?)),
            Op::Bare => argv.push("--bare".into()),
            Op::Mirror => argv.push("--mirror".into()),
            Op::Sparse => argv.push("--no-checkout".into()),
            Op::Regular => {}
        }
        argv.extend(args.iter().cloned());
        argv.extend(["--".into(), url.into(), dir.to_string_lossy().into_owned()]);
        // The terminal runs the clone (credentials, progress, Ctrl-C);
        // finish() completes it afterwards.
        Ok(Next::Invoke(super::repo::GitInvocation {
            expected_head: None,
            repo: self.clone(),
            args: argv.into_iter().map(Into::into).collect(),
            input: None,
            draft: None,
            draft_stamp: None,
            editor: false,
            env: vec![],
            after: Some(super::repo::After::Clone(After {
                dir,
                op,
                args: args.to_vec(),
                set_push,
            })),
        }))
    }
}

/// What magit-clone-internal's sentinel does after a successful clone.
#[derive(Clone, Debug)]
pub struct After {
    pub dir: PathBuf,
    pub op: Op,
    pub args: Vec<String>,
    /// magit-clone-set-remote.pushDefault: set it to the clone's remote.
    pub set_push: bool,
}

/// magit-clone-set-remote.pushDefault: Some(true) ask, Some(false) set, None don't.
fn push_default() -> Option<bool> {
    match super::options::value("magit-clone-set-remote.pushDefault") {
        Some(toml::Value::Boolean(true)) => Some(false),
        Some(toml::Value::Boolean(false)) => None,
        _ => Some(true),
    }
}

impl After {
    pub fn finish(self) -> Result<Next, String> {
        let new = Repo {
            root: self.dir.clone(),
        };
        if matches!(self.op, Op::Bare | Op::Mirror) {
            // Status needs a worktree.
            return Ok(Next::Done(Ok(format!(
                "Cloned into {}",
                self.dir.display()
            ))));
        }
        // magit-clone-set-remote-head (nil): drop the remote's HEAD.
        let remote = self
            .args
            .iter()
            .find_map(|x| x.strip_prefix("--origin="))
            .map(str::to_owned)
            .or_else(|| {
                new.read(&["config", "clone.defaultRemote"])
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
            })
            .unwrap_or_else(|| "origin".into());
        if !super::options::flag("magit-clone-set-remote-head", false) {
            let _ = new.read(&["remote", "set-head", &remote, "-d"]);
        }
        if self.set_push {
            new.read(&["config", "remote.pushDefault", &remote])?;
        }
        if self.op == Op::Sparse {
            new.read(&["sparse-checkout", "init", "--cone"])?;
            // An empty remote has nothing to check out yet.
            if let Ok(branch) = new.current_branch()
                && new.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok()
            {
                new.read(&["checkout", &branch, "--"])?;
            }
        }
        // magit-post-clone-hook, in the new repository.
        super::options::run_hook("magit-post-clone-hook", &self.dir);
        Ok(Next::Status(self.dir))
    }
}
