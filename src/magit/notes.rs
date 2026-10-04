//! magit-notes.el: notes config, edit, remove, merge and prune.
use super::branch::Next;
use super::repo::Repo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// c / d / C / D: core.notesRef and notes.displayRef, local or global.
    NotesRef(bool),
    DisplayRef(bool),
    Edit,
    Remove,
    Merge,
    Prune,
    MergeCommit,
    MergeAbort,
}

/// magit-notes-read-ref: short names live under refs/notes/.
fn notes_ref(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('-') || name.chars().any(char::is_control) {
        return Err(format!("invalid notes ref {name:?}"));
    }
    Ok(if name.starts_with("refs/") {
        name.to_owned()
    } else {
        format!("refs/notes/{name}")
    })
}

impl Repo {
    /// magit-notes-merging-p: NOTES_MERGE_WORKTREE has entries.
    pub fn notes_merging(&self) -> bool {
        self.read(&["rev-parse", "--git-path", "NOTES_MERGE_WORKTREE"])
            .ok()
            .and_then(|p| {
                std::fs::read_dir(self.root.join(String::from_utf8_lossy(&p).trim())).ok()
            })
            .is_some_and(|mut d| {
                d.any(|e| e.is_ok_and(|e| !e.file_name().to_string_lossy().starts_with('.')))
            })
    }
    fn scoped_config(&self, global: bool, key: &str) -> Vec<String> {
        let mut args = vec!["config"];
        if global {
            args.push("--global");
        }
        args.extend(["--get-all", key]);
        self.read(&args)
            .map(|o| {
                String::from_utf8_lossy(&o)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn notes_prompts(&self, op: Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        let scope = |g: bool| if g { "global" } else { "local" };
        match op {
            Op::NotesRef(g) => {
                let cur = self.scoped_config(g, "core.notesRef").join(",");
                (
                    vec![format!(
                        "Set {} core.notesRef (empty unsets; now {cur}): ",
                        scope(g)
                    )],
                    vec![String::new()],
                )
            }
            Op::DisplayRef(g) => {
                let cur = self.scoped_config(g, "notes.displayRef").join(",");
                (
                    vec![format!(
                        "Set {} notes.displayRef, comma separated (empty unsets; now {cur}): ",
                        scope(g)
                    )],
                    vec![String::new()],
                )
            }
            Op::Edit | Op::Remove => {
                let d = at_point.unwrap_or_else(|| "HEAD".into());
                let verb = if op == Op::Edit {
                    "Edit notes"
                } else {
                    "Remove notes"
                };
                (vec![format!("{verb} (default {d}): ")], vec![d])
            }
            Op::Merge => (vec!["Merge reference: ".into()], vec![String::new()]),
            _ => (vec![], vec![]),
        }
    }
    pub fn notes_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let done = |m: String| Ok(Next::Done(Ok(m)));
        let set = |global: bool, key: &str, values: Vec<String>| -> Result<(), String> {
            let mut base = vec!["config"];
            if global {
                base.push("--global");
            }
            let mut unset = base.clone();
            unset.extend(["--unset-all", key]);
            let _ = self.read(&unset);
            for v in values {
                let mut add = base.clone();
                add.extend(["--add", key, &v]);
                self.read(&add)?;
            }
            Ok(())
        };
        match op {
            Op::NotesRef(global) => {
                let v = at(0).trim();
                let values = if v.is_empty() {
                    vec![]
                } else {
                    vec![notes_ref(v)?]
                };
                set(global, "core.notesRef", values)?;
                done("Set core.notesRef".into())
            }
            Op::DisplayRef(global) => {
                let values = at(0)
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(notes_ref)
                    .collect::<Result<Vec<_>, _>>()?;
                set(global, "notes.displayRef", values)?;
                done("Set notes.displayRef".into())
            }
            Op::Edit | Op::Remove => {
                let commit = at(0);
                if commit.is_empty()
                    || commit.starts_with('-')
                    || commit.chars().any(char::is_control)
                {
                    return Err(format!("invalid revision {commit:?}"));
                }
                let verb = if op == Op::Edit { "edit" } else { "remove" };
                let argv = vec![
                    "notes".into(),
                    verb.into(),
                    "--end-of-options".into(),
                    commit.into(),
                ];
                Ok(if op == Op::Edit {
                    Next::GitEditor(argv)
                } else {
                    Next::Git(argv)
                })
            }
            Op::Merge => {
                let r = notes_ref(at(0))?;
                let mut argv = vec!["notes".to_owned(), "merge".into()];
                argv.extend(
                    args.iter()
                        .filter(|x| x.starts_with("--strategy="))
                        .cloned(),
                );
                argv.push(r);
                Ok(Next::GitEditor(argv))
            }
            Op::Prune => {
                let mut argv = vec!["notes".to_owned(), "prune".into()];
                if args.iter().any(|x| x == "--dry-run") {
                    argv.push("--dry-run".into());
                }
                Ok(Next::Git(argv))
            }
            Op::MergeCommit => Ok(Next::GitEditor(vec![
                "notes".into(),
                "merge".into(),
                "--commit".into(),
            ])),
            Op::MergeAbort => Ok(Next::Git(vec![
                "notes".into(),
                "merge".into(),
                "--abort".into(),
            ])),
        }
    }
}
