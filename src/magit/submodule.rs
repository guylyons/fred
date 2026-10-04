//! magit-submodule.el: add, register, populate, update, sync, unpopulate,
//! remove and list modules.
use super::Question;
use super::branch::Next;
use super::repo::Repo;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Register,
    Populate,
    Update,
    Synchronize,
    Unpopulate,
    Remove,
    /// --force removal of these modules, some dirty: confirm stashing them first.
    RemoveDirty(Vec<String>, Vec<String>),
    List,
}

impl Op {
    fn verb(&self) -> &'static str {
        match self {
            Op::Register => "Register",
            Op::Populate => "Populate",
            Op::Update => "Update",
            Op::Synchronize => "Synchronize",
            Op::Unpopulate => "Unpopulate",
            _ => "Remove",
        }
    }
    /// The menu arguments each suffix accepts (magit-submodule-arguments).
    fn accepts(&self, arg: &str) -> bool {
        match self {
            Op::Add | Op::Unpopulate | Op::Remove => arg == "--force",
            Op::Populate | Op::Synchronize => arg == "--recursive",
            Op::Update => matches!(
                arg,
                "--force"
                    | "--remote"
                    | "--recursive"
                    | "--checkout"
                    | "--rebase"
                    | "--merge"
                    | "--no-fetch"
            ),
            _ => false,
        }
    }
}

/// magit-submodule-read-path's default: the URL's last component without .git.
fn url_name(url: &str) -> String {
    let last = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("");
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

impl Repo {
    /// magit-list-module-paths: gitlinks at stage 0.
    pub fn module_paths(&self) -> Result<Vec<String>, String> {
        let out = self.read(&["ls-files", "-z", "--stage"])?;
        Ok(out
            .split(|b| *b == 0)
            .filter_map(|e| {
                let e = String::from_utf8_lossy(e);
                let (meta, path) = e.split_once('\t')?;
                let mut f = meta.split(' ');
                (f.next()? == "160000" && f.nth(1)? == "0").then(|| path.to_owned())
            })
            .collect())
    }
    /// magit-module-worktree-p.
    fn populated(&self, module: &str) -> bool {
        self.root.join(module).join(".git").exists()
    }
    fn suitable(&self, op: &Op) -> Vec<String> {
        self.module_paths()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| match op {
                Op::Register | Op::Populate => !self.populated(m),
                Op::Update | Op::Synchronize => self.populated(m),
                _ => true,
            })
            .collect()
    }
    pub fn submodule_prompts(
        &self,
        op: &Op,
        at_point: Option<String>,
    ) -> (Vec<String>, Vec<String>) {
        match op {
            Op::Add => (
                vec![
                    "Add submodule (remote url): ".into(),
                    "Add submodules at path (default from url): ".into(),
                    "Submodule name (default path): ".into(),
                ],
                vec![String::new(), String::new(), String::new()],
            ),
            Op::List | Op::RemoveDirty(..) => (vec![], vec![]),
            _ => {
                // magit-module-confirm: the module at point, else a suitable one.
                let d = at_point
                    .or_else(|| self.suitable(op).into_iter().next())
                    .unwrap_or_default();
                (
                    vec![format!(
                        "{} module(s), comma separated (default {d}): ",
                        op.verb()
                    )],
                    vec![d],
                )
            }
        }
    }
    fn modules(&self, op: &Op, answer: &str) -> Result<Vec<String>, String> {
        let known = self.module_paths()?;
        let suitable = self.suitable(op);
        let modules: Vec<String> = answer
            .split(',')
            .map(|m| m.trim().trim_end_matches('/').to_owned())
            .filter(|m| !m.is_empty())
            .collect();
        if modules.is_empty() {
            return Err("No module selected".into());
        }
        for m in &modules {
            if !known.contains(m) {
                return Err(format!("{m} is not a module"));
            }
            if !suitable.contains(m) {
                return Err(format!("{} does not apply to module {m}", op.verb()));
            }
        }
        Ok(modules)
    }
    pub fn submodule_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        let mut args: Vec<String> = args.iter().filter(|x| op.accepts(x)).cloned().collect();
        let git = |words: &[&str], args: &[String], modules: &[String]| {
            let mut argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            argv.extend(args.iter().cloned());
            argv.push("--".into());
            argv.extend(modules.iter().cloned());
            argv
        };
        match op {
            Op::List => Ok(Next::View(super::Kind::Modules)),
            Op::Add => {
                let url = at(0);
                if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
                    return Err(format!("invalid url {url:?}"));
                }
                let path = match at(1) {
                    "" => url_name(url),
                    p => p.trim_end_matches('/').to_owned(),
                };
                let name = match at(2) {
                    "" => path.clone(),
                    n => n.to_owned(),
                };
                for v in [&path, &name] {
                    if v.is_empty()
                        || v.starts_with('-')
                        || v.chars().any(char::is_control)
                        || Path::new(v)
                            .components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                        || v.split('/').any(|c| c == ".git")
                    {
                        return Err(format!("invalid module path or name {v:?}"));
                    }
                }
                let mut argv = vec!["submodule".into(), "add".into(), "--name".into(), name];
                argv.extend(args);
                argv.extend(["--".into(), url.into(), path]);
                Ok(Next::Git(argv))
            }
            Op::Register => Ok(Next::Git(git(
                &["submodule", "init"],
                &[],
                &self.modules(&op, at(0))?,
            ))),
            Op::Populate => Ok(Next::Git(git(
                &["submodule", "update", "--init"],
                &args,
                &self.modules(&op, at(0))?,
            ))),
            Op::Update => Ok(Next::Git(git(
                &["submodule", "update"],
                &args,
                &self.modules(&op, at(0))?,
            ))),
            Op::Synchronize => Ok(Next::Git(git(
                &["submodule", "sync"],
                &args,
                &self.modules(&op, at(0))?,
            ))),
            Op::Unpopulate => Ok(Next::Git(git(
                &["submodule", "deinit"],
                &args,
                &self.modules(&op, at(0))?,
            ))),
            Op::Remove => {
                let modules = self.modules(&op, at(0))?;
                // Never remove uncommitted work silently (magit-submodule-remove).
                let dirty: Vec<String> = modules
                    .iter()
                    .filter(|m| {
                        self.populated(m)
                            && Repo {
                                root: self.root.join(m),
                            }
                            .read(&["status", "--porcelain"])
                            .map_or(true, |o| !o.is_empty())
                    })
                    .cloned()
                    .collect();
                if dirty.is_empty() {
                    return self.remove_modules(&modules, &args, &[]);
                }
                if args.iter().any(|x| x == "--force") {
                    let question = if dirty.len() == 1 {
                        format!("Remove dirty module {}? (y or n) ", dirty[0])
                    } else {
                        format!("Remove {} dirty modules? (y or n) ", dirty.len())
                    };
                    return Ok(Next::Ask(
                        Question::Submodule(Op::RemoveDirty(modules, dirty)),
                        vec![question],
                        vec![String::new()],
                    ));
                }
                let clean: Vec<String> =
                    modules.into_iter().filter(|m| !dirty.contains(m)).collect();
                let note = format!(
                    "Omitting {} with uncommitted changes: {}",
                    if dirty.len() == 1 {
                        "module"
                    } else {
                        "modules"
                    },
                    dirty.join(", ")
                );
                if clean.is_empty() {
                    return Err(note);
                }
                self.remove_modules(&clean, &args, &[])?;
                Ok(Next::Done(Ok(note)))
            }
            Op::RemoveDirty(modules, dirty) => {
                let modules = if matches!(at(0), "y" | "yes") {
                    modules
                } else {
                    args.retain(|x| x != "--force");
                    modules.into_iter().filter(|m| !dirty.contains(m)).collect()
                };
                if modules.is_empty() {
                    return Err("Abort".into());
                }
                let backup: Vec<String> =
                    dirty.into_iter().filter(|m| modules.contains(m)).collect();
                self.remove_modules(&modules, &args, &backup)
            }
        }
    }
    /// absorbgitdirs, deinit and rm; dirty modules are stashed first.
    fn remove_modules(
        &self,
        modules: &[String],
        args: &[String],
        backup: &[String],
    ) -> Result<Next, String> {
        for m in backup {
            Repo {
                root: self.root.join(m),
            }
            .read(&[
                "stash",
                "push",
                "-m",
                "backup before removal of this module",
            ])?;
        }
        let run = |words: &[&str], args: &[String]| -> Result<Vec<u8>, String> {
            let mut argv: Vec<std::ffi::OsString> = words.iter().map(Into::into).collect();
            argv.extend(args.iter().map(Into::into));
            argv.push("--".into());
            argv.extend(modules.iter().map(Into::into));
            self.run(&argv, None)
        };
        run(&["submodule", "absorbgitdirs"], &[])?;
        run(&["submodule", "deinit"], args)?;
        run(&["rm"], args)?;
        Ok(Next::Done(Ok(format!("Removed {}", modules.join(", ")))))
    }
    /// magit-list-submodules rows: path, branch or (detached)/(unpopulated), describe.
    pub fn module_rows(&self) -> Result<Vec<(String, String)>, String> {
        let modules = self.module_paths()?;
        let width = modules.iter().map(String::len).max().unwrap_or(0).min(40);
        Ok(modules
            .into_iter()
            .map(|m| {
                let text = if !self.populated(&m) {
                    "(unpopulated)".to_owned()
                } else {
                    let sub = Repo {
                        root: self.root.join(&m),
                    };
                    let branch = sub.current_branch().unwrap_or_else(|_| "(detached)".into());
                    let desc = sub
                        .read(&["describe", "--tags"])
                        .or_else(|_| sub.read(&["rev-parse", "--short", "HEAD"]))
                        .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                        .unwrap_or_default();
                    format!("{branch:<25} {desc}")
                };
                (format!("{m:<width$} {text}"), m)
            })
            .collect())
    }
    /// magit-submodule-visit: the module's status.
    pub fn module_dir(&self, module: &str) -> Result<PathBuf, String> {
        if !self.module_paths()?.iter().any(|m| m == module) {
            return Err(format!("{module} is not a module"));
        }
        if !self.populated(module) {
            return Err(format!("Module {module} is not populated"));
        }
        Ok(self.root.join(module))
    }
}
