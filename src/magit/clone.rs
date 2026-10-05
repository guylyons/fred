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
            "Clone repository: ".to_owned(),
            "Clone to (default name from url): ".into(),
        ];
        match op {
            Op::ShallowSince => prompts.push("Exclude commits before: ".into()),
            Op::ShallowExclude => prompts.push("Exclude commits reachable from: ".into()),
            _ => {}
        }
        let defaults = vec![String::new(); prompts.len()];
        (prompts, defaults)
    }
    /// `self.root` is the directory relative answers resolve against.
    pub fn clone_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        let url = value(at(0))?;
        let name = url_to_name(url);
        let mut dir = match at(1) {
            "" => self
                .root
                .join(name.clone().ok_or("Cannot derive a directory name")?),
            d => match d.strip_prefix("~/") {
                Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
                None => self.root.join(d),
            },
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
            after: Some(super::repo::After::Clone(After {
                dir,
                op,
                args: args.to_vec(),
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
        // magit-clone-set-remote-head is nil: drop the remote's HEAD.
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
        let _ = new.read(&["remote", "set-head", &remote, "-d"]);
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
