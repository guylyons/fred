//! magit-sequence.el rebase suffixes and git-rebase.el's todo editing.
//!
//! Interactive rebases run in two phases, since Git cannot wait on Fred as an
//! editor process: a capture run copies Git's own todo list and fails its
//! sequence editor (Git aborts cleanly, reapplying any autostash), Fred edits the
//! copy, and a replay run installs it with `cp` as the sequence editor.
use super::Question;
use super::branch::Next;
use super::repo::{GitInvocation, Repo};
use crate::editor::{Editor, Mode};
use crate::ex::ExEffect;
use crate::key::{Key, KeyCode};
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    OntoPushRemote,
    OntoUpstream,
    /// Upstream unset: the answer sets it, then rebase onto it.
    SetUpstream,
    Elsewhere,
    /// s: new base, then the first commit to move.
    Subset,
    Interactive,
    /// m / w / k: rewrite one commit's todo line non-interactively.
    EditCommit,
    RewordCommit,
    RemoveCommit,
    Autosquash,
    /// Confirmations before an interactive rebase: action, commit, args-free.
    Published(Box<Op>, String),
    MergesInRange(Box<Op>, String),
    Continue,
    Skip,
    EditTodo,
    Abort,
}

/// An interactive rebase waiting for its edited todo list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub repo: Repo,
    /// `--root`, or the base commit.
    pub base: Vec<String>,
    pub args: Vec<String>,
    /// HEAD (commit and branch) when the todo was captured; replay refuses if
    /// either changed.
    pub head: String,
    pub branch: Option<String>,
    pub todo: PathBuf,
}

fn rev(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid revision {v:?}"));
    }
    Ok(v)
}
/// `--root` is an option; a base commit follows --end-of-options.
fn base_args(base: &[String]) -> Vec<OsString> {
    match base {
        [root] if root == "--root" => vec!["--root".into()],
        [onto, new, upstream] if onto == "--onto" => vec![
            "--onto".into(),
            new.into(),
            "--end-of-options".into(),
            upstream.into(),
        ],
        _ => std::iter::once("--end-of-options".into())
            .chain(base.iter().map(Into::into))
            .collect(),
    }
}
/// A path for a shell command line (sequence.editor is run by a shell).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl Repo {
    pub(super) fn git_path(&self, name: &str) -> Result<PathBuf, String> {
        let p = self.read(&["rev-parse", "--git-path", name])?;
        Ok(self.root.join(String::from_utf8_lossy(&p).trim()))
    }
    /// magit-rebase-in-progress-p.
    pub fn rebase_in_progress(&self) -> bool {
        self.git_path("rebase-merge").is_ok_and(|p| p.exists())
            || self.git_path("rebase-apply/onto").is_ok_and(|p| p.exists())
    }
    fn head(&self) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.read(&["rev-parse", "HEAD"])?)
            .trim()
            .to_owned())
    }

    pub fn rebase_prompts(
        &self,
        op: &Op,
        at_point: Option<String>,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let d = at_point.unwrap_or_default();
        let upstream = self
            .read(&["rev-parse", "--abbrev-ref", "@{upstream}"])
            .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
            .unwrap_or_default();
        Ok(match op {
            Op::Elsewhere => (vec![format!("Rebase onto (default {d}): ")], vec![d]),
            Op::Subset => (
                vec![
                    format!("Rebase subset onto (default {upstream}): "),
                    format!("Starting with commit (default {d}): "),
                ],
                vec![upstream, d],
            ),
            Op::Interactive => (
                vec![format!("Rebase interactively from commit (default {d}): ")],
                vec![d],
            ),
            Op::EditCommit => (vec![format!("Edit commit (default {d}): ")], vec![d]),
            Op::RewordCommit => (vec![format!("Reword commit (default {d}): ")], vec![d]),
            Op::RemoveCommit => (vec![format!("Remove commit (default {d}): ")], vec![d]),
            Op::Abort => (
                vec!["Abort this rebase? (y or n) ".into()],
                vec![String::new()],
            ),
            _ => (vec![], vec![]),
        })
    }

    pub fn rebase_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        let interactive = args.iter().any(|x| x == "--interactive");
        let plain: Vec<String> = args
            .iter()
            .filter(|x| *x != "--interactive")
            .cloned()
            .collect();
        let rebase_onto = |target: &str| -> Result<Next, String> {
            if interactive {
                return self.capture(vec![target.to_owned()], plain.clone());
            }
            let mut argv = vec!["rebase".to_owned()];
            argv.extend(plain.clone());
            argv.extend(["--end-of-options".into(), target.into()]);
            Ok(Next::GitEditor(argv))
        };
        match op {
            Op::OntoPushRemote => {
                let branch = self.current_branch()?;
                let remote = self
                    .push_remote(&branch)?
                    .ok_or("No push-remote is configured; set one with the push menu first")?;
                rebase_onto(&format!("{remote}/{branch}"))
            }
            Op::OntoUpstream => {
                let branch = self.current_branch()?;
                if self
                    .read(&["rev-parse", "--verify", "-q", "@{upstream}"])
                    .is_ok()
                {
                    return rebase_onto("@{upstream}");
                }
                Ok(Next::Ask(
                    Question::Rebase(Op::SetUpstream),
                    vec![format!("Set upstream of {branch} and rebase onto that: ")],
                    vec![String::new()],
                ))
            }
            Op::SetUpstream => {
                let branch = self.current_branch()?;
                let target = rev(at(0))?;
                self.read(&["branch", "--set-upstream-to", target, "--", &branch])?;
                rebase_onto(target)
            }
            Op::Elsewhere => rebase_onto(rev(at(0))?),
            Op::Subset => {
                let onto = rev(at(0))?;
                let start = rev(at(1))?;
                if interactive {
                    return self.capture(
                        vec!["--onto".into(), onto.into(), format!("{start}^")],
                        plain.clone(),
                    );
                }
                let mut argv = vec!["rebase".to_owned()];
                argv.extend(plain);
                argv.extend(["--onto".into(), onto.into(), format!("{start}^")]);
                Ok(Next::GitEditor(argv))
            }
            Op::Interactive | Op::EditCommit | Op::RewordCommit | Op::RemoveCommit => {
                self.interactive_from(op, rev(at(0))?, &plain, true)
            }
            Op::Published(op, commit) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                self.interactive_from(*op, &commit, &plain, false)
            }
            Op::MergesInRange(op, commit) => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                let base = self.rebase_base(&commit)?;
                self.rewrite(*op, &commit, base, &plain)
            }
            Op::Autosquash => {
                // magit-rebase-autosquash: squash into commits not on the upstream.
                let base = self
                    .read(&["merge-base", "@{upstream}", "HEAD"])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .map_err(|_| "No upstream: use interactive rebase to choose a base")?;
                let mut argv = vec![
                    "-c".to_owned(),
                    "sequence.editor=true".into(),
                    "rebase".into(),
                    "-i".into(),
                    "--autosquash".into(),
                    "--keep-empty".into(),
                ];
                argv.extend(plain);
                argv.extend(["--end-of-options".into(), base]);
                Ok(Next::GitEditor(argv))
            }
            Op::Continue => {
                if !self.rebase_in_progress() {
                    return Err("No rebase in progress".into());
                }
                if self
                    .read(&["diff", "--quiet", "--ignore-submodules"])
                    .is_err()
                {
                    return Err("Cannot continue rebase with unstaged changes".into());
                }
                Ok(Next::GitEditor(vec!["rebase".into(), "--continue".into()]))
            }
            Op::Skip => Ok(Next::GitEditor(vec!["rebase".into(), "--skip".into()])),
            Op::EditTodo => Ok(Next::GitEditor(vec!["rebase".into(), "--edit-todo".into()])),
            Op::Abort => {
                if !matches!(at(0), "y" | "yes") {
                    return Err("Abort".into());
                }
                Ok(Next::Git(vec!["rebase".into(), "--abort".into()]))
            }
        }
    }
    /// magit-rebase-interactive-1 and magit-rebase-interactive-assert.
    fn interactive_from(
        &self,
        op: Op,
        commit: &str,
        args: &[String],
        confirm: bool,
    ) -> Result<Next, String> {
        let id = String::from_utf8_lossy(
            &self
                .read(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    "--end-of-options",
                    &format!("{commit}^{{commit}}"),
                ])
                .map_err(|_| format!("unknown commit {commit:?}"))?,
        )
        .trim()
        .to_owned();
        if self
            .read(&["merge-base", "--is-ancestor", &id, "HEAD"])
            .is_err()
        {
            return Err(format!("{commit} isn't an ancestor of HEAD"));
        }
        if confirm {
            let published = self
                .read(&[
                    "branch",
                    "-r",
                    "--format=%(refname:short)",
                    "--contains",
                    &id,
                ])
                .map(|o| {
                    String::from_utf8_lossy(&o)
                        .lines()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !published.is_empty() {
                let to = match published.as_slice() {
                    [one] => one.clone(),
                    many => format!("{} public branches", many.len()),
                };
                return Ok(Next::Ask(
                    Question::Rebase(Op::Published(Box::new(op), id)),
                    vec![format!(
                        "Some of these commits have already been published to {to}.  Do you really want to modify them? (y or n) "
                    )],
                    vec![String::new()],
                ));
            }
        }
        let base = self.rebase_base(&id)?;
        let merges = match &base {
            Some(b) => self.read(&["rev-list", "--merges", &format!("{b}..HEAD")]),
            None => self.read(&["rev-list", "--merges", "HEAD"]),
        }
        .is_ok_and(|o| !o.is_empty());
        if merges && !args.iter().any(|a| a.starts_with("--rebase-merges")) {
            return Ok(Next::Ask(
                Question::Rebase(Op::MergesInRange(Box::new(op), id)),
                vec!["Proceed despite merge in rebase range? (y or n) ".into()],
                vec![String::new()],
            ));
        }
        self.rewrite(op, &id, base, args)
    }
    /// The commit's parent, or None for a root commit (--root).
    fn rebase_base(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self
            .read(&["rev-parse", "--verify", "-q", &format!("{id}^")])
            .ok()
            .map(|o| String::from_utf8_lossy(&o).trim().to_owned()))
    }
    fn rewrite(
        &self,
        op: Op,
        id: &str,
        base: Option<String>,
        args: &[String],
    ) -> Result<Next, String> {
        let base = match base {
            Some(b) => vec![b],
            None => vec!["--root".into()],
        };
        let next = self.capture(base, args.to_vec())?;
        let action = match op {
            Op::EditCommit => "edit",
            Op::RewordCommit => "reword",
            Op::RemoveCommit => "drop",
            _ => return Ok(next),
        };
        // magit-rebase--perl-editor: change the first pick of the commit.
        let Next::Todo(plan) = next else {
            return Ok(next);
        };
        let text = std::fs::read_to_string(&plan.todo).map_err(|e| e.to_string())?;
        let mut done = false;
        let edited: Vec<String> = text
            .lines()
            .map(|line| {
                let mut words = line.split_whitespace();
                if !done
                    && matches!(words.next(), Some("pick" | "p"))
                    && words.next().is_some_and(|h| id.starts_with(h))
                {
                    done = true;
                    let rest = line.split_once(' ').map_or("", |(_, r)| r);
                    return format!("{action} {rest}");
                }
                line.to_owned()
            })
            .collect();
        if !done {
            return Err(format!(
                "{} is not in the rebase todo",
                &id[..id.len().min(8)]
            ));
        }
        std::fs::write(&plan.todo, edited.join("\n") + "\n").map_err(|e| e.to_string())?;
        Ok(Next::Replay(plan))
    }
    /// Phase one: let Git write its todo list into a private file, then abort.
    fn capture(&self, base: Vec<String>, args: Vec<String>) -> Result<Next, String> {
        // Resolve symbolic bases now, so a fetch or upstream change while the
        // list is open cannot retarget the replay.
        let base = base
            .into_iter()
            .map(|b| {
                if b.starts_with("--") {
                    return Ok(b);
                }
                self.read(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    "--end-of-options",
                    &format!("{b}^{{commit}}"),
                ])
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                .map_err(|_| format!("unknown revision {b:?}"))
            })
            .collect::<Result<Vec<_>, String>>()?;
        // A fresh file per capture: an open buffer of an older list is never reused.
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "fred-rebase-todo-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let todo = self.git_path(&name)?;
        let _ = std::fs::remove_file(&todo);
        let save = shell_quote(&todo.to_string_lossy());
        // Git's autostash reapplies worktree changes but not the index: keep it.
        let index = self.read(&["write-tree"]).ok();
        let mut argv: Vec<OsString> = vec![
            "-c".into(),
            format!("sequence.editor=f() {{ cp \"$1\" {save}; exit 1; }}; f").into(),
            "rebase".into(),
            "-i".into(),
        ];
        argv.extend(args.iter().map(Into::into));
        argv.extend(base_args(&base));
        let out = self
            .command()
            .args(&argv)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("GIT_SEQUENCE_EDITOR")
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if let Some(tree) = index {
            let tree = String::from_utf8_lossy(&tree).trim().to_owned();
            let _ = self.read(&["read-tree", &tree]);
        }
        if !todo.exists() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        Ok(Next::Todo(Plan {
            repo: self.clone(),
            base,
            args,
            head: self.head()?,
            branch: self.current_branch().ok(),
            todo,
        }))
    }
}

impl Plan {
    /// Phase two: run the rebase with the edited todo installed by `cp`.
    pub fn replay(&self) -> Result<GitInvocation, String> {
        if self.repo.head()? != self.head || self.repo.current_branch().ok() != self.branch {
            return Err(
                "HEAD moved since the todo list was created; start the rebase again".into(),
            );
        }
        let mut args: Vec<OsString> = vec![
            "-c".into(),
            format!(
                "sequence.editor=cp {}",
                shell_quote(&self.todo.to_string_lossy())
            )
            .into(),
            "rebase".into(),
            "-i".into(),
        ];
        args.extend(self.args.iter().map(Into::into));
        args.extend(base_args(&self.base));
        Ok(GitInvocation {
            expected_head: Some(self.head.clone()),
            repo: self.repo.clone(),
            args,
            input: None,
            draft: None,
            draft_stamp: None,
            editor: true,
            after: None,
        })
    }
}

/// git-rebase-mode keys (evil-collection): p r e s f d change the action,
/// x adds an exec line, M-k/M-j move a line, ZZ runs the rebase, ZQ cancels.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    if ed.rebase_todo.is_none() || ed.mode != Mode::Normal || ed.zap.is_some() {
        return false;
    }
    if ed.vim.pending == [Key::ch('Z')] {
        ed.vim.pending.clear();
        let action = match k.char() {
            Some('Z') => super::Action::RebaseFinish,
            Some('Q') => super::Action::RebaseCancel,
            _ => return true,
        };
        ed.pending_effect = Some(ExEffect::Magit(action));
        return true;
    }
    if !ed.vim.pending.is_empty() || k.ctrl {
        return false;
    }
    if k.alt {
        let delta: isize = match k.code {
            KeyCode::Char('k') | KeyCode::Char('p') => -1,
            KeyCode::Char('j') | KeyCode::Char('n') => 1,
            _ => return false,
        };
        move_line(ed, delta);
        return true;
    }
    let action = match k.code {
        KeyCode::Char('p') => "pick",
        KeyCode::Char('r') => "reword",
        KeyCode::Char('e') => "edit",
        KeyCode::Char('s') => "squash",
        KeyCode::Char('f') => "fixup",
        KeyCode::Char('d') => "drop",
        KeyCode::Char('x') => {
            ed.magit_prompt = Some(super::Prompt::RebaseExec);
            ed.open_cmdline('=', "");
            if let Mode::Command(cl) = &mut ed.mode {
                cl.prompt = "Execute: ".into();
            }
            return true;
        }
        KeyCode::Char('Z') => {
            ed.vim.pending = vec![k];
            return true;
        }
        _ => return false,
    };
    set_action(ed, action);
    true
}
fn splice(ed: &mut Editor, at: usize, remove: usize, insert: &[String]) {
    let pos = (ed.cur.line, ed.cur.byte);
    ed.undo.begin(pos);
    if let Some((edit, _)) = ed.buf.splice_edit(at, remove, insert) {
        let inverse = ed.buf.apply(edit);
        ed.undo.record(inverse);
    }
    ed.undo.end(pos);
}
/// git-rebase-set-action: only commit lines ("pick <hash> ...") change.
fn set_action(ed: &mut Editor, action: &str) {
    let line = ed.cur.line;
    let text = ed.buf.line(line);
    let mut words = text.splitn(2, ' ');
    let (Some(verb), Some(mut rest)) = (words.next(), words.next()) else {
        return;
    };
    // `fixup -C <commit>` carries an option: other actions take just the commit.
    if let Some(r) = rest
        .strip_prefix("-C ")
        .or_else(|| rest.strip_prefix("-c "))
    {
        rest = r;
    }
    let commit_verbs = [
        "pick", "p", "reword", "r", "edit", "e", "squash", "s", "fixup", "f", "drop", "d",
    ];
    if !commit_verbs.contains(&verb) {
        return;
    }
    splice(ed, line, 1, &[format!("{action} {rest}")]);
    if line + 1 < ed.buf.len_lines() {
        ed.set_cursor(line + 1, 0);
    }
}
fn move_line(ed: &mut Editor, delta: isize) {
    let line = ed.cur.line;
    let target = line as isize + delta;
    // The todo region ends where Git's help comment starts.
    let end = (0..ed.buf.len_lines())
        .find(|&i| ed.buf.line(i).starts_with("# Rebase "))
        .unwrap_or(ed.buf.len_lines());
    let text = ed.buf.line(line);
    if target < 0
        || target as usize >= end
        || line >= end
        || text.starts_with('#')
        || text.trim().is_empty()
    {
        return;
    }
    let (a, b) = (line.min(target as usize), line.max(target as usize));
    let (first, second) = (ed.buf.line(a), ed.buf.line(b));
    splice(ed, a, 2, &[second, first]);
    ed.set_cursor(target as usize, 0);
}
/// git-rebase-exec: insert "exec COMMAND" below the current line.
pub fn insert_exec(ed: &mut Editor, command: &str) {
    if command.trim().is_empty() {
        return;
    }
    let at = ed.cur.line + 1;
    let line = format!("exec {}", command.trim());
    if at < ed.buf.len_lines() {
        splice(ed, at, 0, &[line]);
    } else {
        let last = ed.buf.line(at - 1);
        splice(ed, at - 1, 1, &[last, line]);
    }
    ed.set_cursor(at, 0);
}
