//! magit-apply.el: discard, reverse, stage all modified and unstage all.
use super::Section;
use super::branch::Next;
use super::repo::{Diff, Entry, Repo, label};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Discard,
    Reverse,
    StageModified,
    UnstageAll,
}

/// The section (with the files it listed), file (with its status) or hunk
/// at point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Thing {
    Section(Section, Vec<PathBuf>),
    File(PathBuf, Section, String),
    Hunk(Diff, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    pub kind: Kind,
    pub thing: Option<Thing>,
}

/// Discarding this staged/untracked file deletes it from disk.
fn deletes(section: Section, xy: &str) -> bool {
    let b = xy.as_bytes();
    let (x, y) = (
        b.first().copied().unwrap_or(b' '),
        b.get(1).copied().unwrap_or(b' '),
    );
    section == Section::Untracked
        || (section == Section::Staged && matches!(x, b'A' | b'C') && y != b'M')
}

impl Op {
    /// magit-confirm's question, if any.
    pub fn question(&self) -> Result<Option<String>, String> {
        let name = |p: &PathBuf| label(p);
        Ok(Some(match (self.kind, &self.thing) {
            (Kind::StageModified, _) => return Ok(None),
            (Kind::UnstageAll, _) => "Unstage all changes? (y or n) ".into(),
            (_, None) => return Err("Nothing at point".into()),
            // magit-discard-files--resolve: magit-checkout-stage.
            (Kind::Discard, Some(Thing::File(p, Section::Conflicts, _))) => format!(
                "Resolve {}: checkout [o]urs, [t]heirs or restore the [c]onflict? ",
                name(p)
            ),
            (
                _,
                Some(Thing::File(_, Section::Conflicts, _) | Thing::Section(Section::Conflicts, _)),
            ) => {
                return Err("Resolve conflicts one file at a time".into());
            }
            (Kind::Discard, Some(Thing::Hunk(..))) => "Discard hunk? (y or n) ".into(),
            (Kind::Reverse, Some(Thing::Hunk(..))) => "Reverse hunk? (y or n) ".into(),
            (Kind::Discard, Some(Thing::File(p, s, xy))) if deletes(*s, xy) => {
                format!("Delete {}? (y or n) ", name(p))
            }
            (Kind::Discard, Some(Thing::File(p, s, _))) => {
                let side = if *s == Section::Staged {
                    "staged"
                } else {
                    "unstaged"
                };
                format!("Discard {side} changes in {}? (y or n) ", name(p))
            }
            (Kind::Reverse, Some(Thing::File(p, Section::Staged, _))) => {
                format!("Reverse changes in {}? (y or n) ", name(p))
            }
            (Kind::Reverse, Some(Thing::File(..))) => {
                return Err("Cannot reverse unstaged changes".into());
            }
            (Kind::Discard, Some(Thing::Section(s, files)))
                if matches!(s, Section::Untracked | Section::Unstaged | Section::Staged) =>
            {
                let what = match s {
                    Section::Untracked => "Delete",
                    Section::Staged => "Discard staged changes in",
                    _ => "Discard unstaged changes in",
                };
                let shown: Vec<String> = files.iter().take(5).map(name).collect();
                let more = if files.len() > 5 { ", ..." } else { "" };
                format!(
                    "{what} {} files ({}{more})? (y or n) ",
                    files.len(),
                    shown.join(", ")
                )
            }
            (Kind::Reverse, Some(Thing::Section(Section::Staged, files))) => {
                format!("Reverse staged changes in {} files? (y or n) ", files.len())
            }
            (_, Some(Thing::Section(..))) => return Err("Nothing to do for this section".into()),
        }))
    }
}

impl Repo {
    fn apply_patch(&self, patch: &[u8], mode: &[&str]) -> Result<(), String> {
        let mut args: Vec<std::ffi::OsString> = vec!["apply".into(), "--whitespace=nowarn".into()];
        args.extend(mode.iter().map(Into::into));
        let mut check = args.clone();
        check.push("--check".into());
        self.run(&check, Some(patch))?;
        self.run(&args, Some(patch)).map(|_| ())
    }
    /// The staged diff of one file, with Repo::diff's hardened arguments.
    fn staged_patch(&self, path: &Path) -> Result<Vec<u8>, String> {
        let d = self.diff(path, true)?;
        if d.bytes.is_empty() {
            return Err(format!("{} has no staged changes", label(path)));
        }
        if d.hunks.is_empty() {
            return Err(format!(
                "Cannot discard staged changes to binary {}; unstage instead",
                label(path)
            ));
        }
        Ok(d.bytes)
    }
    /// magit-discard-files for one entry, by its status (upstream's table).
    fn discard_entry(&self, e: &Entry, section: Section) -> Result<(), String> {
        let b = e.xy.as_bytes();
        let (x, y) = (
            b.first().copied().unwrap_or(b' '),
            b.get(1).copied().unwrap_or(b' '),
        );
        let p = &e.path;
        let run = |args: &[&str]| self.run(&self.path_args(args, p), None).map(|_| ());
        match section {
            Section::Untracked => run(&["clean", "-f", "-d", "-q"]),
            Section::Unstaged => match y {
                b'M' | b'T' | b'D' => run(&["checkout"]),
                b'A' => Err(format!("{} is intent-to-add; unstage it instead", label(p))),
                _ => Err(format!("Nothing to discard in {}", label(p))),
            },
            Section::Staged => match x {
                b'M' | b'T' => {
                    let patch = self.staged_patch(p)?;
                    if y == b' ' {
                        // Index and worktree both back to HEAD.
                        self.apply_patch(&patch, &["--reverse", "--index"])
                    } else {
                        // Keep the unstaged work (upstream: --cached, then
                        // --reject): reverse the index, and the worktree only
                        // where the reverse applies cleanly; overlapping
                        // edits stay as they are.
                        self.apply_patch(&patch, &["--reverse", "--cached"])?;
                        let _ = self.apply_patch(&patch, &["--reverse"]);
                        Ok(())
                    }
                }
                // A new file with unstaged edits becomes untracked, content kept.
                b'A' | b'C' if y == b'M' => {
                    run(&["add"])?;
                    run(&["reset", "-q"])
                }
                b'A' | b'C' => run(&["rm", "-f", "-q"]),
                // magit-discard-files--resurrect (staged): back into the index.
                b'D' => run(&["reset", "-q"]),
                b'R' => {
                    let orig = e.old_path.clone().ok_or("rename without its source")?;
                    if self.root.join(p).exists() {
                        if let Some(dir) = self.root.join(&orig).parent() {
                            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                        }
                        let args: Vec<std::ffi::OsString> =
                            vec!["mv".into(), "--".into(), p.into(), orig.into()];
                        self.run(&args, None).map(|_| ())
                    } else {
                        run(&["rm", "--cached", "-q"])?;
                        self.run(&self.path_args(&["reset", "-q"], &orig), None)
                            .map(|_| ())
                    }
                }
                _ => Err(format!("Nothing to discard in {}", label(p))),
            },
            _ => Err("Resolve conflicts with the file's own commands".into()),
        }
    }
    /// magit-checkout-stage: take one side of a conflicted file (and stage
    /// it), or restore the conflict.
    pub fn checkout_stage(&self, p: &Path, xy: &str, side: &str) -> Result<Next, String> {
        let arg = match side {
            "o" | "ours" => "--ours",
            "t" | "theirs" => "--theirs",
            "c" | "conflict" => "--merge",
            _ => return Err("Answer o, t or c".into()),
        };
        let b = xy.as_bytes();
        let (x, y) = (
            b.first().copied().unwrap_or(b' '),
            b.get(1).copied().unwrap_or(b' '),
        );
        let run = |args: &[&str]| self.run(&self.path_args(args, p), None).map(|_| ());
        // A side that deleted the file resolves by removing it.
        let deleted = matches!(
            (arg, x, y),
            ("--ours", b'D', _)
                | ("--ours", b'U', b'A')
                | ("--theirs", _, b'D')
                | ("--theirs", b'A', b'U')
        );
        if deleted {
            run(&["rm", "-q"])?;
        } else if arg == "--merge" {
            run(&["checkout", "--merge"])?;
        } else {
            run(&["checkout", arg])?;
            run(&["add", "-u"])?;
        }
        Ok(Next::Done(Ok(format!(
            "Checked out {} of {}",
            &arg[2..],
            label(p)
        ))))
    }
    pub fn apply_step(&self, op: Op, answer: &str) -> Result<Next, String> {
        if let (Kind::Discard, Some(Thing::File(p, Section::Conflicts, xy))) = (op.kind, &op.thing)
        {
            return self.checkout_stage(p, xy, answer.trim());
        }
        if op.question()?.is_some() && !matches!(answer.trim(), "y" | "yes") {
            return Err("Abort".into());
        }
        let done = |m: String| Ok(Next::Done(Ok(m)));
        // The repository now: entries that left the section are skipped.
        let fresh = |path: &PathBuf, s: Section| -> Result<Option<Entry>, String> {
            Ok(self
                .status()?
                .entries
                .into_iter()
                .find(|e| &e.path == path && s.contains(e)))
        };
        match (op.kind, op.thing) {
            (Kind::StageModified, _) => {
                self.read(&["add", "-u", "--", "."])?;
                done("Staged all modified files".into())
            }
            (Kind::UnstageAll, _) => {
                if self.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
                    self.read(&["reset", "-q", "--", "."])?;
                } else {
                    self.read(&["rm", "--cached", "-r", "-q", "--", "."])?;
                }
                done("Unstaged all changes".into())
            }
            (_, None) => Err("Nothing at point".into()),
            (Kind::Discard, Some(Thing::Hunk(diff, i))) => {
                let mode: &[&str] = if diff.staged {
                    &["--index", "--reverse"]
                } else {
                    &["--reverse"]
                };
                self.apply_hunk_with(&diff, i, mode).map_err(|e| {
                    if diff.staged {
                        format!("{e} (discard the file's unstaged changes first)")
                    } else {
                        e
                    }
                })?;
                done("Discarded hunk".into())
            }
            (Kind::Reverse, Some(Thing::Hunk(diff, i))) => {
                if !diff.staged {
                    return Err("Cannot reverse unstaged changes".into());
                }
                self.apply_hunk_with(&diff, i, &["--reverse"])?;
                done("Reversed hunk".into())
            }
            (kind, Some(Thing::File(p, s, xy))) => {
                let e = fresh(&p, s)?.ok_or("File changed; refresh and select again")?;
                if e.xy != xy {
                    return Err("File changed; refresh and select again".into());
                }
                if kind == Kind::Reverse {
                    let patch = self.staged_patch(&p)?;
                    self.apply_patch(&patch, &["--reverse"])?;
                    return done(format!("Reversed changes in {}", label(&p)));
                }
                self.discard_entry(&e, s)?;
                done(format!("Discarded {}", label(&p)))
            }
            (kind, Some(Thing::Section(s, files))) => {
                let mut failed = vec![];
                let mut count = 0;
                for p in &files {
                    let Some(e) = fresh(p, s)? else { continue };
                    let r = if kind == Kind::Reverse {
                        self.staged_patch(p)
                            .and_then(|patch| self.apply_patch(&patch, &["--reverse"]))
                    } else {
                        self.discard_entry(&e, s)
                    };
                    match r {
                        Ok(()) => count += 1,
                        Err(err) => failed.push(err),
                    }
                }
                if !failed.is_empty() {
                    return Err(format!("Done for {count} files; {}", failed.join("; ")));
                }
                done(format!("Done for {count} files"))
            }
        }
    }
}
