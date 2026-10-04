//! magit-gitignore.el (ignore rules, skip-worktree, assume-unchanged) and
//! magit-sparse-checkout.el.
use super::branch::Next;
use super::repo::Repo;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Topdir,
    Subdir,
    Gitdir,
    System,
    SkipWorktree,
    NoSkipWorktree,
    AssumeUnchanged,
    NoAssumeUnchanged,
    SparseEnable,
    SparseDisable,
    SparseReapply,
    SparseSet,
    SparseAdd,
}

/// A repository-relative path without `..`, `.git` or a leading `-`.
fn relative(p: &str) -> Result<&str, String> {
    let p = p.trim().trim_matches('/');
    if p.starts_with('-')
        || p.chars().any(char::is_control)
        || Path::new(p).components().any(|c| {
            !matches!(c, Component::Normal(_)) || c.as_os_str().eq_ignore_ascii_case(".git")
        })
    {
        return Err(format!("invalid path {p:?}"));
    }
    Ok(p)
}

impl Repo {
    fn bool_config(&self, key: &str) -> bool {
        self.read(&["config", "--type=bool", key])
            .is_ok_and(|o| String::from_utf8_lossy(&o).trim() == "true")
    }
    /// magit-sparse-checkout-enabled-p.
    pub fn sparse_enabled(&self) -> bool {
        self.bool_config("core.sparseCheckout")
    }
    fn untracked(&self) -> Vec<String> {
        self.read(&["ls-files", "-z", "--others", "--exclude-standard"])
            .map(|o| {
                o.split(|b| *b == 0)
                    .filter(|f| !f.is_empty())
                    .map(|f| String::from_utf8_lossy(f).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
    /// magit-gitignore-read-pattern's default for the file at point.
    fn ignore_default(&self, file: Option<&str>, dir: &str) -> String {
        let Some(file) = file else {
            return String::new();
        };
        let untracked = self.untracked();
        let in_dir = |f: &str| match dir {
            "" => Some(f.to_owned()),
            d => f.strip_prefix(&format!("{d}/")).map(str::to_owned),
        };
        let Some(rel) = in_dir(file) else {
            return String::new();
        };
        let choices: Vec<String> = untracked.iter().filter_map(|f| in_dir(f)).collect();
        if choices.contains(&rel) {
            return format!("/{rel}");
        }
        match Path::new(&rel).extension() {
            Some(ext) => {
                let glob = format!("*.{}", ext.to_string_lossy());
                let has = choices.iter().any(|c| c.ends_with(&glob[1..]));
                if has { glob } else { String::new() }
            }
            None => String::new(),
        }
    }
    pub fn ignore_prompts(&self, op: &Op, file: Option<String>) -> (Vec<String>, Vec<String>) {
        let ask = |p: &str, d: &str| {
            if d.is_empty() {
                format!("{p}: ")
            } else {
                format!("{p} (default {d}): ")
            }
        };
        let file = file.as_deref();
        match op {
            Op::Topdir | Op::Gitdir | Op::System => {
                let d = self.ignore_default(file, "");
                (vec![ask("File or pattern to ignore", &d)], vec![d])
            }
            Op::Subdir => {
                let dir = file
                    .and_then(|f| Path::new(f).parent())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let d = self.ignore_default(file, &dir);
                (
                    vec![
                        ask("Limit rule to files in", &dir),
                        ask("File or pattern to ignore", &d),
                    ],
                    vec![dir, d],
                )
            }
            Op::SkipWorktree | Op::NoSkipWorktree | Op::AssumeUnchanged | Op::NoAssumeUnchanged => {
                let verb = match op {
                    Op::SkipWorktree => "Skip worktree for",
                    Op::NoSkipWorktree => "Do not skip worktree for",
                    Op::AssumeUnchanged => "Assume file to be unchanged",
                    _ => "Do not assume file to be unchanged",
                };
                let d = file.unwrap_or("").to_owned();
                (vec![ask(verb, &d)], vec![d])
            }
            Op::SparseSet => (
                vec!["Include these directories (space separated): ".into()],
                vec![String::new()],
            ),
            Op::SparseAdd => (
                vec!["Add these directories (space separated): ".into()],
                vec![String::new()],
            ),
            _ => (vec![], vec![]),
        }
    }
    /// magit--gitignore: append RULE (backslashes doubled) on its own line.
    fn append_rule(&self, rule: &str, file: &Path, stage: bool) -> Result<Next, String> {
        let rule = rule.trim();
        if rule.is_empty() || rule.contains('\n') {
            return Err("A pattern is required".into());
        }
        if file
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(format!("{} is a symbolic link", file.display()));
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut text = std::fs::read(file).unwrap_or_default();
        if !text.is_empty() && !text.ends_with(b"\n") {
            text.push(b'\n');
        }
        text.extend(rule.replace('\\', "\\\\").bytes());
        text.push(b'\n');
        std::fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))?;
        if stage {
            let argv: Vec<std::ffi::OsString> = vec!["add".into(), "--".into(), file.into()];
            self.run(&argv, None)?;
        }
        Ok(Next::Done(Ok(format!(
            "Ignoring {rule} in {}",
            file.display()
        ))))
    }
    pub fn ignore_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        // Git's warnings (e.g. paths left despite sparse patterns) are shown.
        let git = |words: Vec<String>| -> Result<Next, String> {
            let out = self
                .command()
                .args(&words)
                .output()
                .map_err(|e| e.to_string())?;
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            if !out.status.success() {
                return Err(stderr);
            }
            Ok(Next::Done(Ok(if stderr.is_empty() {
                format!("git {}", words.join(" "))
            } else {
                stderr
            })))
        };
        let dirs = |answer: &str| -> Result<Vec<String>, String> {
            let v: Vec<String> = answer
                .split_whitespace()
                .map(|d| relative(d).map(str::to_owned))
                .collect::<Result<_, _>>()?;
            if v.iter().any(String::is_empty) || v.is_empty() {
                return Err("No directories given".into());
            }
            Ok(v)
        };
        // magit-sparse-checkout--auto-enable.
        let auto_enable = || -> Result<(), String> {
            if self.sparse_enabled() {
                if !self.bool_config("core.sparseCheckoutCone") {
                    return Err("Magit's sparse checkout functionality requires cone mode".into());
                }
                return Ok(());
            }
            self.read(&["sparse-checkout", "init", "--cone"])
                .map(|_| ())
        };
        match op {
            Op::Topdir => self.append_rule(at(0), &self.root.join(".gitignore"), true),
            Op::Subdir => {
                // An existing directory inside the worktree, through no symlink
                // that leaves it (upstream reads an existing directory).
                let answer = at(0);
                let dir = if Path::new(answer).is_absolute() {
                    PathBuf::from(answer)
                } else {
                    self.root.join(relative(answer)?)
                };
                let canon = std::fs::canonicalize(&dir)
                    .map_err(|_| format!("{} is not a directory", dir.display()))?;
                let root = std::fs::canonicalize(&self.root).map_err(|e| e.to_string())?;
                let inside = canon
                    .strip_prefix(&root)
                    .map_err(|_| format!("{} isn't inside the repository", dir.display()))?;
                if !canon.is_dir()
                    || inside
                        .components()
                        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
                {
                    return Err(format!("invalid directory {}", dir.display()));
                }
                self.append_rule(at(1), &canon.join(".gitignore"), true)
            }
            Op::Gitdir => {
                let common = self.read(&["rev-parse", "--git-common-dir"])?;
                let common = self.root.join(String::from_utf8_lossy(&common).trim());
                self.append_rule(at(0), &common.join("info/exclude"), false)
            }
            Op::System => {
                let file = self
                    .config("core.excludesFile")
                    .ok_or("Variable `core.excludesFile' isn't set")?;
                let file = match file.strip_prefix("~/") {
                    Some(rest) => {
                        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest)
                    }
                    None => PathBuf::from(file),
                };
                self.append_rule(at(0), &file, false)
            }
            Op::SkipWorktree | Op::NoSkipWorktree | Op::AssumeUnchanged | Op::NoAssumeUnchanged => {
                let file = relative(at(0))?;
                if file.is_empty() {
                    return Err("A file is required".into());
                }
                self.read(&["ls-files", "--error-unmatch", "--", file])
                    .map_err(|_| format!("{file} is not tracked"))?;
                let flag = match op {
                    Op::SkipWorktree => "--skip-worktree",
                    Op::NoSkipWorktree => "--no-skip-worktree",
                    Op::AssumeUnchanged => "--assume-unchanged",
                    _ => "--no-assume-unchanged",
                };
                git(vec![
                    "update-index".into(),
                    flag.into(),
                    "--".into(),
                    file.into(),
                ])
            }
            // Upstream always runs these; Git reports what doesn't apply.
            Op::SparseEnable => {
                let mut w = vec!["sparse-checkout".to_owned(), "init".into(), "--cone".into()];
                w.extend(args.iter().filter(|x| *x == "--sparse-index").cloned());
                git(w)
            }
            Op::SparseDisable | Op::SparseReapply => {
                let verb = if op == Op::SparseDisable {
                    "disable"
                } else {
                    "reapply"
                };
                git(vec!["sparse-checkout".into(), verb.into()])
            }
            Op::SparseSet | Op::SparseAdd => {
                let dirs = dirs(at(0))?;
                auto_enable()?;
                let verb = if op == Op::SparseSet { "set" } else { "add" };
                let mut w = vec!["sparse-checkout".to_owned(), verb.into()];
                w.extend(dirs);
                git(w)
            }
        }
    }
}
