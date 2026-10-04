//! magit-subtree.el: import (add, add commit, merge, pull) and export
//! (push, split) subtrees.
use super::branch::Next;
use super::repo::Repo;
use std::path::{Component, Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    AddCommit,
    Merge,
    Pull,
    Push,
    Split,
}

impl Op {
    /// The import or export menu whose arguments apply.
    pub fn menu(&self) -> char {
        match self {
            Op::Push | Op::Split => 'E',
            _ => 'I',
        }
    }
    fn prompts(&self) -> (&'static str, &'static [&'static str]) {
        match self {
            Op::Add => ("Add subtree", &["From repository: ", "Ref: "]),
            Op::AddCommit => ("Add subtree", &["Commit: "]),
            Op::Merge => ("Merge into subtree", &["Commit: "]),
            Op::Pull => ("Pull into subtree", &["From repository: ", "Ref: "]),
            Op::Push => ("Push subtree", &["To repository: ", "To reference: "]),
            Op::Split => ("Split subtree", &["Commit: "]),
        }
    }
    fn subcommand(&self) -> &'static str {
        match self {
            Op::Add | Op::AddCommit => "add",
            Op::Merge => "merge",
            Op::Pull => "pull",
            Op::Push => "push",
            Op::Split => "split",
        }
    }
}

fn value(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid value {v:?}"));
    }
    Ok(v)
}

/// magit-subtree-prefix: from --prefix= if set, else read.
fn prefix_arg(args: &[String]) -> Option<&str> {
    args.iter().find_map(|a| a.strip_prefix("--prefix="))
}

impl Repo {
    pub fn subtree_prompts(&self, op: &Op, args: &[String]) -> (Vec<String>, Vec<String>) {
        let (verb, rest) = op.prompts();
        let mut prompts: Vec<String> = vec![];
        if prefix_arg(args).is_none() {
            prompts.push(format!("{verb}: "));
        }
        prompts.extend(rest.iter().map(|p| p.to_string()));
        let defaults = vec![String::new(); prompts.len()];
        (prompts, defaults)
    }
    /// magit-subtree-read-prefix: a directory inside the repository.
    fn subtree_prefix(&self, p: &str) -> Result<String, String> {
        let p = p.trim().trim_end_matches('/');
        let path = Path::new(p);
        let relative = if path.is_absolute() {
            // Compare canonical forms: the root may be reached through symlinks.
            let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or(p.to_path_buf());
            let abs = match (path.parent(), path.file_name()) {
                (Some(parent), Some(name)) => canon(parent).join(name),
                _ => path.to_path_buf(),
            };
            abs.strip_prefix(canon(&self.root))
                .map_err(|_| format!("{p} isn't inside the repository at {}", self.root.display()))?
                .to_path_buf()
        } else {
            path.to_path_buf()
        };
        let s = relative.to_string_lossy().into_owned();
        if s.is_empty()
            || s.starts_with('-')
            || s.chars().any(char::is_control)
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || relative.components().any(|c| c.as_os_str() == ".git")
        {
            return Err(format!("invalid subtree prefix {p:?}"));
        }
        Ok(s)
    }
    pub fn subtree_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let (prefix, rest) = match prefix_arg(args) {
            Some(p) => (p.to_owned(), a),
            None => (
                a.first().cloned().unwrap_or_default(),
                a.get(1..).unwrap_or(&[]),
            ),
        };
        let prefix = self.subtree_prefix(&prefix)?;
        let mut argv = vec![
            "subtree".to_owned(),
            op.subcommand().into(),
            format!("--prefix={prefix}"),
        ];
        // magit-subtree-arguments: everything but --prefix=.
        argv.extend(args.iter().filter(|x| !x.starts_with("--prefix=")).cloned());
        for v in rest.iter().take(op.prompts().1.len()) {
            argv.push(value(v.trim())?.to_owned());
        }
        if rest.len() < op.prompts().1.len() {
            return Err("missing answer".into());
        }
        Ok(Next::Git(argv))
    }
}
