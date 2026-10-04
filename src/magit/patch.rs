//! magit-patch.el (create, apply, save, request pull) and magit-am from
//! magit-sequence.el.
use super::branch::Next;
use super::diff::Target;
use super::repo::Repo;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create,
    Apply,
    /// magit-patch-save from the menu; resolved to SaveDiff in a diff buffer.
    Save,
    SaveDiff(Target, Vec<String>),
    RequestPull,
    AmPatches,
    AmMaildir,
    /// The am menu's "a": plain patch, or abort while applying.
    AmApply,
    AmContinue,
    AmSkip,
    AmAbort,
}

impl Op {
    /// The menu whose arguments apply.
    pub fn menu(&self) -> char {
        match self {
            Op::Create => 'K',
            Op::Apply => 'a',
            _ => 'w',
        }
    }
}

fn rev(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid revision {v:?}"));
    }
    Ok(v)
}

impl Repo {
    /// magit-am-in-progress-p.
    pub fn am_in_progress(&self) -> bool {
        self.git_path("rebase-apply/applying")
            .is_ok_and(|p| p.exists())
    }
    /// A file answer: ~/ expands, relative names are relative to the toplevel.
    fn answer_path(&self, answer: &str) -> Result<PathBuf, String> {
        let answer = answer.trim();
        if answer.is_empty() || answer.chars().any(char::is_control) {
            return Err("A file name is required".into());
        }
        Ok(match answer.strip_prefix("~/") {
            Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
            None => self.root.join(answer),
        })
    }
    /// Resolve in-progress keys (magit-am's Apply versus Actions groups).
    pub fn patch_resolve(&self, op: Op) -> Result<Op, String> {
        let applying = self.am_in_progress();
        Ok(match op {
            Op::AmPatches if applying => Op::AmContinue,
            Op::AmApply if applying => Op::AmAbort,
            Op::AmApply => Op::Apply,
            Op::AmSkip | Op::AmContinue | Op::AmAbort if !applying => {
                return Err("Not applying any patches".into());
            }
            Op::AmMaildir if applying => return Err("Already applying patches".into()),
            op => op,
        })
    }
    pub fn patch_prompts(&self, op: &Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        let d = at_point.unwrap_or_default();
        let ask = |p: &str, d: &str| {
            if d.is_empty() {
                format!("{p}: ")
            } else {
                format!("{p} (default {d}): ")
            }
        };
        match op {
            Op::Create => (vec![ask("Create patches for range or commit", &d)], vec![d]),
            Op::Apply | Op::AmPatches => (vec![ask("Apply patch", &d)], vec![d]),
            Op::AmMaildir => (vec!["Apply mbox or Maildir: ".into()], vec![String::new()]),
            Op::SaveDiff(..) => (vec!["Write patch file: ".into()], vec![String::new()]),
            Op::RequestPull => {
                let remote = self
                    .remotes()
                    .ok()
                    .and_then(|r| r.into_iter().next())
                    .unwrap_or_default();
                let start = self
                    .current_branch()
                    .ok()
                    .and_then(|b| self.upstream_of(&b))
                    .unwrap_or_default();
                (
                    vec![ask("Remote", &remote), ask("Start", &start), "End: ".into()],
                    vec![remote, start, String::new()],
                )
            }
            _ => (vec![], vec![]),
        }
    }
    pub fn patch_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        match op {
            Op::Create => {
                let answer = rev(at(0))?;
                let range = if answer.contains("..") {
                    answer.to_owned()
                } else {
                    format!("{answer}^..{answer}")
                };
                let mut argv: Vec<std::ffi::OsString> = vec!["format-patch".into()];
                argv.extend(args.iter().map(Into::into));
                argv.extend([range.into(), "--".into()]);
                let out = self.run(&argv, None)?;
                let written = String::from_utf8_lossy(&out).lines().count();
                // magit-patch-create visits the cover letter.
                if args.iter().any(|x| x == "--cover-letter") {
                    let value = |p: &str| args.iter().find_map(|x| x.strip_prefix(p));
                    let name = match value("--reroll-count=") {
                        Some(v) => format!("v{v}-0000-cover-letter.patch"),
                        None => "0000-cover-letter.patch".into(),
                    };
                    let dir = value("--output-directory=")
                        .map(|d| self.answer_path(d))
                        .transpose()?
                        .unwrap_or(self.root.clone());
                    return Ok(Next::Visit(dir.join(name)));
                }
                Ok(Next::Done(Ok(format!("Wrote {written} patches"))))
            }
            Op::Apply => {
                let file = self.answer_path(at(0))?;
                let mut argv: Vec<std::ffi::OsString> = vec!["apply".into()];
                argv.extend(args.iter().map(Into::into));
                argv.extend(["--".into(), file.into()]);
                self.run(&argv, None)?;
                Ok(Next::Done(Ok("Applied patch".into())))
            }
            Op::SaveDiff(target, diff_args) => {
                let file = self.answer_path(at(0))?;
                let patch = self.diff_output(&target, &diff_args)?;
                // create_new: never follow a symlink or replace an existing file.
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&file)
                    .and_then(|mut f| f.write_all(&patch))
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                Ok(Next::Done(Ok(format!("Wrote {}", file.display()))))
            }
            Op::Save => Err("Only diff buffers can be saved as patches".into()),
            Op::RequestPull => {
                let remote = at(0);
                if !self.remotes()?.iter().any(|r| r == remote) {
                    return Err(format!("No remote {remote:?}"));
                }
                let url = self
                    .config(&format!("remote.{remote}.url"))
                    .ok_or_else(|| format!("Remote {remote} has no url"))?;
                Ok(Next::View(super::Kind::Output(
                    format!("request-pull {remote}"),
                    vec![
                        "request-pull".into(),
                        rev(at(1))?.into(),
                        url,
                        rev(at(2))?.into(),
                    ],
                )))
            }
            Op::AmPatches | Op::AmMaildir => {
                let file = self.answer_path(at(0))?;
                let mut argv = vec!["am".to_owned()];
                argv.extend(args.iter().cloned());
                if op == Op::AmPatches {
                    argv.push("--".into());
                }
                argv.push(file.to_string_lossy().into_owned());
                Ok(Next::GitEditor(argv))
            }
            Op::AmContinue => {
                if self.read(&["diff", "--quiet"]).is_err() {
                    return Err("Cannot continue due to unstaged changes".into());
                }
                Ok(Next::GitEditor(vec!["am".into(), "--continue".into()]))
            }
            Op::AmSkip => Ok(Next::GitEditor(vec!["am".into(), "--skip".into()])),
            Op::AmAbort => Ok(Next::Git(vec!["am".into(), "--abort".into()])),
            Op::AmApply => unreachable!("resolved by patch_resolve"),
        }
    }
}
