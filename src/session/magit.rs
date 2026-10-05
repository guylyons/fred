use super::*;
use crate::magit::diff::{Op as DiffOp, Target};
use crate::magit::repo::{GitInvocation, Repo, label};
use crate::magit::{Action, Kind, Question, Row, RowAction, Section, View};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::thread::JoinHandle;

pub(super) enum Outcome {
    NoticeView(Box<View>, String, Option<RowAction>, usize),
    ErrorView(Box<View>, String, Option<RowAction>, usize),
    View(Box<View>, Option<RowAction>, usize),
    Prompt(Repo, crate::magit::workflows::Operation, Vec<String>),
    Ask(Repo, crate::magit::Question, Vec<String>, Vec<String>),
    InitConfirm(PathBuf, String),
    /// A completed operation and the message to show.
    Done(Repo, String),
    /// An interactive rebase todo list to open for editing.
    Todo(crate::magit::rebase::Plan),
    Blame(Box<crate::magit::blame::Blame>),
    /// A blob to show at a line, a message, then optionally blame it.
    Blob(
        crate::magit::blob::Blob,
        Vec<u8>,
        usize,
        Option<String>,
        Option<crate::magit::blame::Kind>,
    ),
    /// A rename finished: result, old and new absolute paths.
    Moved(Repo, Result<(), String>, PathBuf, PathBuf),
    /// {worktree}: visit the file itself at a line.
    VisitFile(PathBuf, usize),
    Draft(Repo, crate::magit::CommitMode, Vec<u8>, Vec<String>),
    Branches(Repo, Vec<String>),
    Git(GitInvocation),
    /// A network command whose configuration was already written.
    ConfiguredGit(GitInvocation),
    Saved(Repo, Result<(), String>),
    /// Text for the unnamed register (the kill ring) and a message.
    Copy(String, String),
    /// Answer a question without asking (the commit at point).
    Answer(Repo, Question, Vec<String>, Vec<String>),
    /// A shell command for the terminal.
    Shell(String),
}
/// magit-log-select: the current branch's log, in which picking a commit
/// answers QUESTION (after ANSWERS). INITIAL is the commit to start on;
/// otherwise the first that is not a fixup!, squash! or amend! commit.
fn log_select(
    repo: Repo,
    question: Question,
    answers: Vec<String>,
    defaults: Vec<String>,
    message: String,
    initial: Option<String>,
    origin: usize,
) -> Result<Outcome, String> {
    let head = repo.current_branch().unwrap_or_else(|_| "HEAD".into());
    let mut view = View::status(repo, Default::default());
    // magit-log-select-mode's magit-log-default-arguments.
    view.kind = Kind::Log(
        vec![head],
        vec!["--graph".into(), "-n256".into(), "--decorate".into()],
    );
    view.rows.clear();
    view.return_to = origin;
    view.select = Some(Box::new(crate::magit::Select {
        question,
        answers,
        defaults,
        message,
    }));
    refresh_view(&mut view)?;
    let fallback = view
        .rows
        .iter()
        .position(|r| {
            matches!(r.action, Some(RowAction::Commit(_)))
                && !["fixup! ", "squash! ", "amend! "]
                    .iter()
                    .any(|p| r.text.contains(p))
        })
        .unwrap_or(0);
    Ok(Outcome::View(
        Box::new(view),
        initial.map(RowAction::Commit),
        fallback,
    ))
}
/// What a hunk patch at point is used for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchUse {
    Apply,
    Reverse,
    ReverseIndex,
}
/// The patch for the hunk (or file) at LINE of a diff, commit or stash buffer.
fn diff_patch_at(view: &View, line: usize) -> Result<Vec<u8>, String> {
    let repo = &view.repo;
    // The raw output and the rows before it (a diff buffer's title).
    let (bytes, offset) = match &view.kind {
        Kind::Diff(target, args) => (repo.diff_output(target, args)?, 1),
        Kind::Patch(id) => (repo.commit_patch(id)?, 0),
        Kind::StashPatch(stash) => (repo.stash_patch(stash)?, 0),
        _ => return Err("Not in a diff buffer".into()),
    };
    if bytes.len() > 1024 * 1024 {
        return Err("Diff too large to apply from here".into());
    }
    let lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    let at = line.checked_sub(offset).ok_or("No hunk or file at point")?;
    crate::magit::diff::hunk_patch(&lines, at)
}
pub(super) struct Job {
    input_generation: u64,
    slot: usize,
    clock: u64,
    work: JoinHandle<Result<Outcome, String>>,
}
impl Session {
    fn magit_from(&self) -> PathBuf {
        self.ed
            .magit
            .as_ref()
            .map(|v| v.repo.root.clone())
            .or_else(|| self.ed.blob.as_ref().map(|b| b.repo.root.clone()))
            .or_else(|| self.ed.commit_repo.as_ref().map(|r| r.root.clone()))
            .or_else(|| self.ed.rebase_todo.as_ref().map(|p| p.repo.root.clone()))
            .or_else(|| self.ed.dired.as_ref().map(|d| d.dir.clone()))
            .or_else(|| self.ed.path.clone())
            .unwrap_or_else(|| PathBuf::from("."))
    }
    fn start_magit(&mut self, work: impl FnOnce() -> Result<Outcome, String> + Send + 'static) {
        self.ed.set_msg("Git: working…");
        self.magit_job = Some(Job {
            input_generation: self.ed.magit_input_generation,
            slot: self.cur,
            clock: self.clock,
            work: std::thread::spawn(work),
        });
    }
    pub fn magit_action(&mut self, action: Action) {
        if action == Action::Return {
            if let Some(v) = &self.ed.magit {
                let slot = v.return_to;
                if slot < self.bufs.len() && slot != self.cur {
                    self.show(slot);
                } else if let Ok(previous) = self.buffer_arg("#") {
                    self.show(previous);
                } else {
                    self.buffer(BufCmd::Delete, "", false);
                }
            }
            return;
        }
        if let Action::Saved(inner) = action {
            self.saving_done = true;
            self.magit_action(*inner);
            self.saving_done = false;
            return;
        }
        if let Action::SaveAnswered(path, save, rest, inner) = action {
            if save
                && let Some(i) =
                    (0..self.bufs.len()).find(|&i| self.ed_at(i).path.as_ref() == Some(&path))
            {
                let origin = self.cur;
                self.show(i);
                self.write(None, false, None);
                if origin != self.cur && origin < self.bufs.len() {
                    self.show(origin);
                }
            }
            return self.ask_save(rest, inner);
        }
        // magit-save-repository-buffers (magit-maybe-save-repository-buffers
        // before refreshes and Git commands).
        if !self.saving_done
            && matches!(
                action,
                Action::Status
                    | Action::Refresh
                    | Action::RefreshAll
                    | Action::Commit
                    | Action::Answered(..)
                    | Action::Submit(..)
                    | Action::Net(_)
                    | Action::GitRun(_)
                    | Action::Workflow(_)
            )
        {
            let how = self.save_buffers;
            let root = Repo::discover(&self.magit_from()).ok().map(|r| r.root);
            let modified: Vec<PathBuf> = match (how, &root) {
                (Some(_), Some(root)) => (0..self.bufs.len())
                    .filter_map(|i| {
                        let ed = self.ed_at(i);
                        (ed.buf.modified
                            && ed.magit.is_none()
                            && !ed.generated()
                            && ed.commit_repo.is_none())
                        .then(|| ed.path.clone())
                        .flatten()
                        .filter(|p| swap::canonical(p).starts_with(root))
                    })
                    .collect(),
                _ => vec![],
            };
            if !modified.is_empty() {
                if how == Some(true) {
                    return self.ask_save(modified, Box::new(action));
                }
                let origin = self.cur;
                for p in &modified {
                    if let Some(i) =
                        (0..self.bufs.len()).find(|&i| self.ed_at(i).path.as_ref() == Some(p))
                    {
                        self.show(i);
                        self.write(None, false, None);
                    }
                }
                if origin != self.cur && origin < self.bufs.len() {
                    self.show(origin);
                }
            }
        }
        // magit-process-kill works while a Git process runs.
        if action == Action::ListRepositories {
            let origin = self.cur;
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            self.start_magit(move || {
                let mut view = View::status(Repo { root: home }, Default::default());
                view.kind = Kind::Repos;
                view.rows.clear();
                view.return_to = origin;
                refresh_view(&mut view)?;
                Ok(Outcome::View(Box::new(view), None, 1))
            });
            return;
        }
        if let Action::RepolistMark(mark) = action {
            let line = self.ed.cur.line;
            let Some(view) = self.ed.magit.as_mut().filter(|v| v.kind == Kind::Repos) else {
                return self.ed.set_err("Not in a repository list");
            };
            let Some(RowAction::Repo(p)) = view.action_at(line) else {
                return;
            };
            if mark {
                view.marked.insert(p);
            } else {
                view.marked.remove(&p);
            }
            if let Some(row) = view.rows.get_mut(line) {
                row.text.replace_range(..1, if mark { "*" } else { " " });
            }
            let view = view.as_ref().clone();
            // tabulated-list-put-tag ... t: and move to the next line.
            self.install_magit(
                view,
                None,
                (line + 1).min(self.ed.line_count().saturating_sub(1)),
            );
            return;
        }
        if matches!(action, Action::RepolistFetch | Action::RepolistFindFile) {
            let Some(view) = self.ed.magit.as_deref().filter(|v| v.kind == Kind::Repos) else {
                return self.ed.set_err("Not in a repository list");
            };
            // magit-repolist--get-repos: the marked ones, else all (confirmed).
            let mut repos: Vec<PathBuf> = view
                .rows
                .iter()
                .filter_map(|r| match &r.action {
                    Some(RowAction::Repo(p)) if view.marked.contains(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            if repos.is_empty() {
                if crate::magit::options::confirm("repolist-all") {
                    let repo = view.repo.clone();
                    let q = if action == Action::RepolistFetch {
                        crate::magit::misc::Op::RepolistAll(true)
                    } else {
                        crate::magit::misc::Op::RepolistAll(false)
                    };
                    crate::magit::prompt(
                        &mut self.ed,
                        crate::magit::Prompt::Ask(
                            repo,
                            Question::Misc(q),
                            vec![],
                            vec![
                                "Nothing selected.  Act on ALL displayed repositories? (y or n) "
                                    .into(),
                            ],
                            vec![],
                        ),
                    );
                    return;
                }
                repos = view
                    .rows
                    .iter()
                    .filter_map(|r| match &r.action {
                        Some(RowAction::Repo(p)) => Some(p.clone()),
                        _ => None,
                    })
                    .collect();
            }
            return self.repolist_act(action == Action::RepolistFetch, repos);
        }
        if action == Action::TodoHelp {
            return self.ed.set_msg(
                "Rebase todo: p r e s f d set action  x exec  b break  l label  t reset  M merge  \
                 M-j/M-k move  RET show  ZZ run  ZQ cancel",
            );
        }
        if action == Action::ProcessKill {
            let n = Repo::discover(&self.magit_from())
                .map_or(0, |r| crate::magit::repo::kill_running(&r.root));
            return if n == 0 {
                self.ed.set_err("No process at point")
            } else {
                self.ed.set_msg(format!("Killed {n} Git process(es)"))
            };
        }
        if self.magit_job.is_some() || self.pending_git.is_some() || self.git_busy {
            self.ed.set_err("Git operation in progress");
            return;
        }
        // magit-pre-refresh-hook.
        if action == Action::Refresh
            && let Some(view) = self.ed.magit.as_deref()
        {
            crate::magit::options::run_hook("magit-pre-refresh-hook", &view.repo.root);
        }
        // magit-refresh-verbose times every refresh.
        if self.refresh_verbose && action == Action::Refresh && self.profile_once.is_none() {
            let calls = crate::magit::repo::CALLS_COUNT.load(std::sync::atomic::Ordering::Relaxed);
            self.profile_once = Some((std::time::Instant::now(), calls));
        }
        if let Action::Stash(operation) = action {
            let Some(mut view) = self.ed.magit.as_deref().cloned() else {
                self.magit_action(Action::Stashes);
                return;
            };
            if !matches!(view.kind, Kind::Stashes | Kind::StashPatch(_)) {
                self.magit_action(Action::Stashes);
                return;
            }
            let stash = match &view.kind {
                Kind::StashPatch(stash) => Some(stash.clone()),
                _ => match view.action_at(self.ed.cur.line) {
                    Some(RowAction::Stash(stash)) => Some(stash),
                    _ => None,
                },
            };
            let Some(stash) = stash else {
                self.ed.set_err("select a stash first");
                return;
            };
            if operation == crate::magit::workflows::StashAction::Drop {
                crate::magit::prompt(
                    &mut self.ed,
                    crate::magit::Prompt::DropStash(view.repo, stash),
                );
                return;
            }
            let selected = view.action_at(self.ed.cur.line);
            let fallback = self.ed.cur.line;
            self.start_magit(move || {
                let result = view.repo.stash_action(&stash, operation);
                refresh_view(&mut view)?;
                match result {
                    Ok(message) => Ok(Outcome::NoticeView(
                        Box::new(view),
                        message.into(),
                        selected,
                        fallback,
                    )),
                    Err(error) => Ok(Outcome::ErrorView(
                        Box::new(view),
                        error,
                        selected,
                        fallback,
                    )),
                }
            });
            return;
        }
        if let Action::DropStash(repo, stash) = action {
            let Some(mut view) = self.ed.magit.as_deref().cloned().filter(|v| v.repo == repo)
            else {
                self.ed.set_err("stash view changed; select again");
                return;
            };
            let selected = view.action_at(self.ed.cur.line);
            let fallback = self.ed.cur.line;
            self.start_magit(move || {
                let result = repo.stash_action(&stash, crate::magit::workflows::StashAction::Drop);
                refresh_view(&mut view)?;
                match result {
                    Ok(message) => Ok(Outcome::NoticeView(
                        Box::new(view),
                        message.into(),
                        selected,
                        fallback,
                    )),
                    Err(error) => Ok(Outcome::ErrorView(
                        Box::new(view),
                        error,
                        selected,
                        fallback,
                    )),
                }
            });
            return;
        }
        if let Action::Answered(repo, Question::File(op), answers, args) = action {
            return self.file_answered(repo, op, answers, args);
        }
        if let Action::Answered(repo, Question::Branch(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'b'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo.branch_step_args(op, &merged, &defaults, &args);
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Answered(repo, Question::Tag(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 't'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .tag_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Answered(repo, Question::Merge(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'M'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .merge_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Answered(repo, Question::Reset(op), answers, defaults) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .reset_step(op, &merged)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Answered(repo, Question::Log(op), answers, defaults) = action {
            let menu = if matches!(
                op,
                crate::magit::log::Op::ShortlogSince | crate::magit::log::Op::ShortlogRange
            ) {
                'S'
            } else {
                'l'
            };
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, menu));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .log_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Answered(repo, Question::Submodule(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'o'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .submodule_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if matches!(action, Action::DiffBufferFile | Action::DiffWhileCommitting) {
            use crate::magit::diff::Target;
            let origin = self.cur;
            let mut args = crate::magit::menu_arguments(&self.ed, 'd');
            args.retain(|a| !a.starts_with("-- "));
            if action == Action::DiffWhileCommitting {
                // Staged changes; while amending, everything since HEAD's parent.
                let Some(repo) = self.ed.commit_repo.clone() else {
                    return self.ed.set_err("No commit in progress");
                };
                // magit-commit-diff--args: reword shows HEAD^..HEAD, amend
                // everything since HEAD^, --all the worktree.
                let mode = self.ed.commit_mode.clone();
                let all = self.ed.commit_args.iter().any(|a| a == "--all");
                self.start_magit(move || {
                    let parent = repo.read(&["rev-parse", "--verify", "-q", "HEAD^"]).is_ok();
                    let target = match mode {
                        crate::magit::CommitMode::Reword(_) if parent => {
                            Target::Range("HEAD^..HEAD".into())
                        }
                        crate::magit::CommitMode::Amend(_) if parent && all => {
                            Target::Range("HEAD^".into())
                        }
                        crate::magit::CommitMode::Amend(_) if parent => {
                            args.push("--cached".into());
                            Target::Range("HEAD^".into())
                        }
                        _ if all => Target::Range("HEAD".into()),
                        _ => Target::Staged,
                    };
                    diff_view(repo, target, args, origin)
                });
                return;
            }
            // A blob shows its commit; a file its changes since HEAD.
            let blob = self
                .ed
                .blob
                .as_ref()
                .map(|b| (b.repo.clone(), b.rev.clone(), b.file.clone()));
            let (from, path) = (self.magit_from(), self.ed.path.clone());
            self.start_magit(move || {
                let (repo, target, file) = match blob {
                    Some((repo, rev, file)) => {
                        let id = repo
                            .read(&[
                                "rev-parse",
                                "--verify",
                                "-q",
                                "--end-of-options",
                                &format!("{rev}^{{commit}}"),
                            ])
                            .map_err(|_| format!("{rev} is not a commit"))?;
                        (
                            repo,
                            Target::Commit(String::from_utf8_lossy(&id).trim().into()),
                            file,
                        )
                    }
                    None => {
                        let repo = Repo::discover(&from)?;
                        let file = path
                            .as_deref()
                            .ok_or("Buffer isn't visiting a file")
                            .and_then(|p| {
                                repo_relative(&repo, p).map_err(|_| "Buffer isn't visiting a file")
                            })?;
                        (repo, Target::Range("HEAD".into()), file)
                    }
                };
                args.push(format!("-- {}", file.to_string_lossy()));
                diff_view(repo, target, args, origin)
            });
            return;
        }
        if let Action::Menu(menu) = action {
            crate::magit::open_menu(&mut self.ed, menu);
            return;
        }
        if let Action::GitRun(words) = action {
            let (origin, from) = (self.cur, self.magit_from());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let argv = words.iter().map(|w| w.to_string()).collect();
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::Git(argv),
                    origin,
                ))
            });
            return;
        }
        if let Action::Answered(
            repo,
            Question::Misc(crate::magit::misc::Op::LogJump),
            answers,
            defaults,
        ) = action
        {
            return self.log_jump(repo, merge_answers(&answers, &defaults));
        }
        if let Action::Answered(
            _,
            Question::Misc(op @ crate::magit::misc::Op::RepolistAll(_)),
            answers,
            _,
        ) = action
        {
            if !matches!(answers.first().map(|a| a.trim()), Some("y" | "yes")) {
                return self.ed.set_msg("Abort");
            }
            let repos = self
                .ed
                .magit
                .as_deref()
                .map(|v| {
                    v.rows
                        .iter()
                        .filter_map(|r| match &r.action {
                            Some(RowAction::Repo(p)) => Some(p.clone()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            return self.repolist_act(op == crate::magit::misc::Op::RepolistAll(true), repos);
        }
        if let Action::Answered(
            _,
            Question::Misc(crate::magit::misc::Op::RepolistFile),
            answers,
            _,
        ) = action
        {
            let file = answers
                .first()
                .map(|a| a.trim().to_owned())
                .unwrap_or_default();
            let repos = self.repolist_files.take().unwrap_or_default();
            if file.is_empty() {
                return self.ed.set_err("No file");
            }
            let mut opened = 0;
            for r in repos {
                let p = r.join(&file);
                if p.is_file() {
                    self.open_pick(p, None);
                    opened += 1;
                }
            }
            return self
                .ed
                .set_msg(format!("Opened {file} in {opened} repositories"));
        }
        if let Action::Answered(repo, Question::Misc(op), answers, defaults) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .misc_step(op, &merged)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Misc(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            // magit-git-command runs in the current file's directory.
            let here = self
                .ed
                .path
                .as_ref()
                .filter(|_| self.ed.magit.is_none())
                .and_then(|p| p.parent().map(Path::to_path_buf));
            let files = matches!(
                op,
                crate::magit::misc::Op::StageFiles(_)
                    | crate::magit::misc::Op::UnstageFiles
                    | crate::magit::misc::Op::RemovingFile
            );
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Commit(id)) if !files => Some(id),
                        Some(RowAction::File(p, _) | RowAction::Hunk(p, ..)) if files => {
                            Some(p.to_string_lossy().into_owned())
                        }
                        _ => None,
                    });
            // The visited file is the default file (magit-file-relative-name).
            let visited = self
                .ed
                .path
                .clone()
                .filter(|_| files && !self.ed.generated());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let at_point = at_point.or_else(|| {
                    visited
                        .and_then(|p| repo_relative(&repo, &p).ok())
                        .map(|p| p.to_string_lossy().into_owned())
                });
                let op = match (op, here) {
                    (crate::magit::misc::Op::GitCommand { topdir: false }, Some(dir)) => {
                        crate::magit::misc::Op::GitCommandIn(
                            repo_relative(&repo, &dir).unwrap_or_default(),
                        )
                    }
                    (crate::magit::misc::Op::ShellCommand { topdir: false }, Some(dir)) => {
                        crate::magit::misc::Op::ShellCommandIn(
                            repo_relative(&repo, &dir).unwrap_or_default(),
                        )
                    }
                    (op, _) => op,
                };
                let (prompts, defaults) = repo.misc_prompts(&op, at_point);
                if prompts.is_empty() {
                    let next = repo
                        .misc_step(op, &[])
                        .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Misc(op), defaults, prompts))
            });
            return;
        }
        if matches!(
            action,
            Action::TraceDefinition
                | Action::EditLineCommit
                | Action::DiffTrace
                | Action::DiffEditHunk
        ) {
            let origin = self.cur;
            // The visited file (or blob) and the line / name at point.
            let mut blob = self
                .ed
                .blob
                .as_ref()
                .map(|b| (b.repo.clone(), b.rev.clone(), b.file.clone()));
            let (from, path) = (self.magit_from(), self.ed.path.clone());
            let mut line = self.ed.cur.line + 1;
            let text = self.ed.buf.line(self.ed.cur.line);
            let col = self.ed.cur.byte.min(text.len());
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            let start = text[..col]
                .rfind(|c: char| !is_word(c))
                .map_or(0, |i| i + 1);
            let end = text[col..]
                .find(|c: char| !is_word(c))
                .map_or(text.len(), |i| col + i);
            let mut name = text.get(start..end).unwrap_or("").to_owned();
            // From a hunk: the side of the file it shows (magit-diff-visit-file)
            // and the function the hunk changes.
            let action = match action {
                Action::DiffTrace | Action::DiffEditHunk => {
                    let Some(view) = self.ed.magit.as_deref() else {
                        return;
                    };
                    let Some((rev, file, l)) = diff_visit(view, self.ed.cur.line, false) else {
                        return self.ed.set_err("No hunk at point");
                    };
                    name = self.hunk_defun().and_then(|(_, d)| d).unwrap_or_default();
                    blob = Some((view.repo.clone(), rev, file));
                    line = l + 1;
                    if action == Action::DiffTrace {
                        Action::TraceDefinition
                    } else {
                        Action::EditLineCommit
                    }
                }
                a => a,
            };
            let log_args = crate::magit::menu_arguments(&self.ed, 'l');
            let rebase_args = crate::magit::menu_arguments(&self.ed, 'r');
            self.start_magit(move || {
                let (repo, rev, file) = match blob {
                    // The worktree (or index) side: blame the file itself.
                    Some((repo, rev, file))
                        if rev == crate::magit::blob::WORKTREE
                            || rev == crate::magit::blob::INDEX =>
                    {
                        (repo, None, file)
                    }
                    Some((repo, rev, file)) => (repo, Some(rev), file),
                    None => {
                        let repo = Repo::discover(&from)?;
                        let file = path
                            .as_deref()
                            .ok_or("Buffer isn't visiting a file")
                            .and_then(|p| {
                                repo_relative(&repo, p).map_err(|_| "Buffer isn't visiting a file")
                            })?;
                        (repo, None, file)
                    }
                };
                let file_s = file.to_string_lossy().into_owned();
                if action == Action::TraceDefinition {
                    if name.is_empty() {
                        return Err("No function at point found".into());
                    }
                    // regexp-quote, and ":" escaped for -L.
                    let quoted: String = name
                        .chars()
                        .flat_map(|c| {
                            if "\\.*+?[](){}^$|:".contains(c) {
                                vec!['\\', c]
                            } else {
                                vec![c]
                            }
                        })
                        .collect();
                    let commit = rev
                        .clone()
                        .unwrap_or_else(|| repo.current_branch().unwrap_or_else(|_| "HEAD".into()));
                    let mut args: Vec<String> = log_args
                        .into_iter()
                        .filter(|a| !a.starts_with("-L") && a != "--graph")
                        .collect();
                    args.insert(0, format!("-L:{quoted}:{file_s}"));
                    let next = crate::magit::branch::Next::View(Kind::Log(vec![commit], args));
                    return Ok(branch_outcome(repo, next, origin));
                }
                // magit-edit-line-commit: the commit that added this line.
                let mut argv = vec![
                    "blame".to_owned(),
                    "--porcelain".into(),
                    "-L".into(),
                    format!("{line},{line}"),
                ];
                if let Some(r) = &rev {
                    argv.push(r.clone());
                }
                argv.extend(["--".into(), file_s]);
                let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
                let out = repo.read(&refs)?;
                let commit = String::from_utf8_lossy(&out)
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                if commit.is_empty() || commit.bytes().all(|b| b == b'0') {
                    return Err("This line has not been committed yet".into());
                }
                let next = if repo
                    .read(&["merge-base", "--is-ancestor", &commit, "HEAD"])
                    .is_ok()
                {
                    repo.rebase_step(
                        crate::magit::rebase::Op::EditCommit,
                        &[commit],
                        &rebase_args,
                    )
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)))
                } else {
                    repo.read(&["checkout", "-q", &commit, "--"])?;
                    crate::magit::branch::Next::Done(Ok(format!(
                        "Checked out {}",
                        &commit[..8.min(commit.len())]
                    )))
                };
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Jump(name) = action {
            // magit-jump-to-*: the section's heading row, by its title.
            let row = self.ed.magit.as_ref().and_then(|v| {
                v.rows.iter().position(|r| {
                    matches!(r.action, Some(RowAction::Section(_)))
                        && r.text.get(2..).is_some_and(|t| t.starts_with(name))
                })
            });
            match row {
                Some(line) => self.ed.set_cursor(line, 0),
                None => self.ed.set_msg(format!("Section \"{name}\" wasn't found")),
            }
            return;
        }
        if action == Action::ParentStatus {
            let origin = self.cur;
            let from = self.magit_from();
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                // A submodule's superproject, else the repository around it.
                let sup = repo
                    .read(&["rev-parse", "--show-superproject-working-tree"])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .unwrap_or_default();
                let parent = if sup.is_empty() {
                    let up = repo.root.parent().ok_or("No parent repository")?;
                    Repo::discover(up)
                        .map_err(|_| "No parent repository".to_owned())?
                        .root
                } else {
                    PathBuf::from(sup)
                };
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::Status(parent),
                    origin,
                ))
            });
            return;
        }
        if let Action::Answered(repo, Question::Wip(op), answers, _) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let next = repo
                    .wip_step(op, &answers)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if action == Action::DiredAm {
            let Some(d) = self.ed.dired.as_ref() else {
                return self.ed.set_err("Not in a dired buffer");
            };
            let dir = d.dir.clone();
            let files = crate::dired::selection(&self.ed);
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'w'));
            self.start_magit(move || {
                let repo = Repo::discover(&dir)?;
                if files.is_empty() {
                    return Err("No patch files selected".into());
                }
                let mut argv = vec!["am".to_owned()];
                argv.extend(args);
                argv.push("--".into());
                argv.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::GitEditor(argv),
                    origin,
                ))
            });
            return;
        }
        if matches!(action, Action::AsyncShell | Action::EdiffStage) {
            let file = match self
                .ed
                .magit
                .as_deref()
                .and_then(|v| v.action_at(self.ed.cur.line))
            {
                Some(RowAction::File(p, _) | RowAction::Hunk(p, ..)) => Some(p),
                _ => self.hunk_defun().map(|(f, _)| PathBuf::from(f)),
            };
            let Some(file) = file else {
                return self.ed.set_err("No file at point");
            };
            if action == Action::AsyncShell {
                return self.magit_action(Action::Misc(crate::magit::misc::Op::AsyncShell(file)));
            }
            let (origin, from) = (self.cur, self.magit_from());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let argv = vec![
                    "add".into(),
                    "--patch".into(),
                    "--".into(),
                    file.to_string_lossy().into_owned(),
                ];
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::GitEditor(argv),
                    origin,
                ))
            });
            return;
        }
        if action == Action::UpdateIndex {
            let Some(b) = self
                .ed
                .blob
                .as_ref()
                .filter(|b| b.rev == crate::magit::blob::INDEX)
            else {
                return self.ed.set_err("Not visiting the index blob of a file");
            };
            let (repo, file) = (b.repo.clone(), b.file.clone());
            let text = self.ed.buf.to_bytes();
            let origin = self.cur;
            self.start_magit(move || {
                let path = crate::magit::repo::literal_pathspec(&file);
                let staged =
                    repo.run(&["ls-files".into(), "-s".into(), "--".into(), path], None)?;
                let mode = String::from_utf8_lossy(&staged)
                    .split_whitespace()
                    .next()
                    .map(str::to_owned)
                    .ok_or("File is not in the index")?;
                let id = repo.run(
                    &["hash-object".into(), "-w".into(), "--stdin".into()],
                    Some(&text),
                )?;
                let id = String::from_utf8_lossy(&id).trim().to_owned();
                repo.run(
                    &[
                        "update-index".into(),
                        "--cacheinfo".into(),
                        format!("{mode},{id},{}", file.to_string_lossy()).into(),
                    ],
                    None,
                )?;
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::Done(Ok(format!(
                        "Updated index entry of {}",
                        file.display()
                    ))),
                    origin,
                ))
            });
            return;
        }
        if let Action::Thing(what) = action {
            // Placeholders upstream: only a URL at point can be browsed.
            let line = self.ed.buf.line(self.ed.cur.line);
            let col = self.ed.cur.byte.min(line.len());
            let url = line
                .match_indices("http")
                .map(|(i, _)| i)
                .filter(|&i| i <= col)
                .filter_map(|i| {
                    let end = line[i..]
                        .find(|c: char| c.is_whitespace() || "<>\"')".contains(c))
                        .map_or(line.len(), |e| i + e);
                    let u = &line[i..end];
                    (end >= col && (u.starts_with("http://") || u.starts_with("https://")))
                        .then(|| u.to_owned())
                })
                .last();
            return match (what, url) {
                ('o', Some(u)) => self.browse(&u),
                ('o', None) => self
                    .ed
                    .set_err("There is no thing at point that could be browsed"),
                ('e', _) => self
                    .ed
                    .set_err("There is no thing at point that could be edited"),
                _ => self
                    .ed
                    .set_err("There is no thing at point that we know how to copy"),
            };
        }
        if action == Action::Info {
            return self.browse("https://magit.vc/manual/magit/");
        }
        if action == Action::AutoRevertMode {
            self.auto_revert = !self.auto_revert;
            let on = if self.auto_revert {
                "enabled"
            } else {
                "disabled"
            };
            return self.ed.set_msg(format!("Magit-Auto-Revert mode {on}"));
        }
        if action == Action::WipMode {
            self.wip_mode = !self.wip_mode;
            let on = if self.wip_mode { "enabled" } else { "disabled" };
            return self.ed.set_msg(format!("Magit-Wip mode {on}"));
        }
        if action == Action::WipCommitFile {
            let Some(path) = self.ed.path.clone().filter(|_| !self.ed.generated()) else {
                return self.ed.set_err("Not visiting a file");
            };
            let file = match Repo::discover(&path).and_then(|r| repo_relative(&r, &path)) {
                Ok(f) => f,
                Err(e) => return self.ed.set_err(e),
            };
            return self.magit_action(Action::Wip(crate::magit::wip::Op::CommitFile(file)));
        }
        if let Action::Wip(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let next = repo
                    .wip_step(op, &[])
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Answered(_, Question::ReverseDiff(line, index), answers, _) = action {
            if !matches!(answers.first().map(|a| a.trim()), Some("y" | "yes")) {
                return self.ed.set_msg("Abort");
            }
            let how = if index {
                PatchUse::ReverseIndex
            } else {
                PatchUse::Reverse
            };
            return self.apply_diff(how, line);
        }
        if action == Action::CopyDiff {
            return self.copy_diff();
        }
        if let Action::Margin(what) = action {
            let Some(view) = self.ed.magit.as_mut() else {
                return self
                    .ed
                    .set_err("Magit margin isn't supported in this buffer");
            };
            if view.margin.is_none() {
                view.margin = crate::magit::margin::Margin::for_kind(&view.kind);
            }
            let Some(m) = view.margin.as_mut() else {
                return self
                    .ed
                    .set_err("Magit margin isn't supported in this buffer");
            };
            match what {
                'L' => m.shown = !m.shown,
                'l' => m.cycle_style(),
                'd' => m.details = !m.details,
                _ => m.shortstat = !m.shortstat,
            }
            // Load stamps (and shortstats) the buffer does not have yet.
            return self.magit_action(Action::Refresh);
        }
        if let Action::DiffToggle(what) = action {
            let Some(view) = self.ed.magit.as_mut() else {
                return;
            };
            let on = if what == 't' {
                view.refine = !view.refine;
                view.refine
            } else {
                view.fontify = !view.fontify;
                view.fontify
            };
            let name = if what == 't' {
                "Hunk refinement"
            } else {
                "Hunk fontification"
            };
            return self
                .ed
                .set_msg(format!("{name} {}", if on { "on" } else { "off" }));
        }
        if action == Action::GitGuiBlame {
            // The visited file at this line, else (upstream reads them) HEAD's.
            let Some(path) = self.ed.path.clone().filter(|_| !self.ed.generated()) else {
                return self.ed.set_err("Not visiting a file");
            };
            let repo = match Repo::discover(&path) {
                Ok(r) => r,
                Err(e) => return self.ed.set_err(e),
            };
            let file = match repo_relative(&repo, &path) {
                Ok(f) => f,
                Err(e) => return self.ed.set_err(e),
            };
            let line = self.ed.cur.line + 1;
            return self.magit_action(Action::Misc(crate::magit::misc::Op::GitGuiBlame(
                file, line,
            )));
        }
        if action == Action::AbortDwim {
            use crate::magit::{merge, patch, rebase, sequence};
            let repo = match Repo::discover(&self.magit_from()) {
                Ok(r) => r,
                Err(e) => return self.ed.set_err(e),
            };
            // magit-abort-dwim's order: merge, rebase, am, sequencer, bisect.
            let workflow = repo.active_workflow().ok().flatten();
            let next = if workflow == Some("merge") {
                Action::Merge(merge::Op::Abort)
            } else if repo.am_in_progress() {
                Action::Patch(patch::Op::AmAbort)
            } else if workflow == Some("rebase") {
                Action::Rebase(rebase::Op::Abort)
            } else if matches!(workflow, Some("cherry-pick" | "revert")) {
                Action::Sequence(sequence::Op::Abort)
            } else if repo.bisecting() {
                Action::Bisect(crate::magit::bisect::Op::Reset)
            } else {
                return self.ed.set_msg("Nothing to abort");
            };
            return self.magit_action(next);
        }
        if let Action::DebugToggle(which) = action {
            use crate::magit::repo::{DEBUG, PROFILE, RECORD};
            use std::sync::atomic::Ordering;
            let (flag, name) = match which {
                'd' => (&DEBUG, "Git debug"),
                'r' => (&RECORD, "Subprocess recording"),
                'p' => (&PROFILE, "Profiling"),
                _ => {
                    self.refresh_verbose = !self.refresh_verbose;
                    let on = if self.refresh_verbose { "on" } else { "off" };
                    return self.ed.set_msg(format!("Verbose refresh {on}"));
                }
            };
            let on = !flag.fetch_xor(true, Ordering::Relaxed);
            return self.ed.set_msg(format!(
                "{name} {} (see the process buffer)",
                if on { "on" } else { "off" }
            ));
        }
        if action == Action::ProfileRefresh {
            if self.ed.magit.is_none() {
                return self.ed.set_err("Not in a Magit buffer");
            }
            let calls = crate::magit::repo::CALLS_COUNT.load(std::sync::atomic::Ordering::Relaxed);
            self.profile_once = Some((std::time::Instant::now(), calls));
            return self.magit_action(Action::Refresh);
        }
        if action == Action::ZapCaches {
            for ed in self.editors_mut() {
                if let Some(v) = &mut ed.magit {
                    v.stamps.clear();
                    v.dirty = true;
                }
            }
            if self.ed.magit.is_some() {
                return self.magit_action(Action::Refresh);
            }
            return self.ed.set_msg("Zapped caches");
        }
        if action == Action::SaveRepositoryBuffers {
            let Ok(repo) = Repo::discover(&self.magit_from()) else {
                return self.ed.set_err("not in a Git repository");
            };
            let origin = self.cur;
            let slots: Vec<usize> = (0..self.bufs.len())
                .filter(|&i| {
                    let ed = self.ed_at(i);
                    ed.buf.modified
                        && ed.magit.is_none()
                        && !ed.generated()
                        && ed
                            .path
                            .as_ref()
                            .is_some_and(|p| swap::canonical(p).starts_with(&repo.root))
                })
                .collect();
            let mut saved = 0;
            for i in &slots {
                self.show(*i);
                if self.write(None, false, None) {
                    saved += 1;
                }
            }
            if self.cur != origin && origin < self.bufs.len() {
                self.show(origin);
            }
            return self.ed.set_msg(format!(
                "Saved {saved} buffer(s) in {}",
                repo.root.display()
            ));
        }
        if action == Action::LogMoveTo {
            return self.magit_action(Action::Misc(crate::magit::misc::Op::LogJump));
        }
        if let Action::NextReference(previous) = action {
            return self.next_reference(previous);
        }
        if action == Action::CycleDiffs {
            return self.cycle_diffs();
        }
        if let Action::Go(backward) = action {
            let Some(mut view) = self.ed.magit.as_deref().cloned() else {
                return;
            };
            let (from, to) = if backward {
                (&mut view.back, &mut view.forward)
            } else {
                (&mut view.forward, &mut view.back)
            };
            let Some(kind) = from.pop() else {
                return self.ed.set_err(if backward {
                    "No previous entry in buffer's history"
                } else {
                    "No next entry in buffer's history"
                });
            };
            to.push(std::mem::replace(&mut view.kind, kind));
            view.rows.clear();
            self.start_magit(move || {
                refresh_view(&mut view)?;
                Ok(Outcome::View(Box::new(view), None, 0))
            });
            return;
        }
        if let Action::DescribeSection(full) = action {
            let Some(view) = self.ed.magit.as_deref() else {
                return self.ed.set_err("Not in a Magit buffer");
            };
            let line = self.ed.cur.line;
            let text = match view.action_at(line) {
                Some(a) if full => format!("{a:?} at line {} in {:?}", line + 1, view.kind),
                Some(a) => format!("{a:?}"),
                None => format!("No section at line {}", line + 1),
            };
            return self.ed.set_msg(text);
        }
        if matches!(action, Action::SelectPick | Action::SelectQuit) {
            let Some(view) = self.ed.magit.as_deref() else {
                return;
            };
            let Some(select) = view.select.clone() else {
                return self.ed.set_err("Not selecting a commit");
            };
            let repo = view.repo.clone();
            let picked = match view.action_at(self.ed.cur.line) {
                Some(RowAction::Commit(id)) => Some(id),
                _ => None,
            };
            if action == Action::SelectPick && picked.is_none() {
                return self.ed.set_err("No commit at point");
            }
            // magit-mode-bury-buffer 'kill: back to where the selection
            // started, then delete the selection buffer.
            let (back, me) = (view.return_to, self.cur);
            if back < self.bufs.len() && back != me {
                self.show(back);
            }
            self.buffer(BufCmd::Delete, &(me + 1).to_string(), true);
            let Some(id) = picked.filter(|_| action == Action::SelectPick) else {
                return self.ed.set_msg("Abort");
            };
            let mut answers = select.answers;
            answers.push(id);
            return self.magit_action(Action::Answered(
                repo,
                select.question,
                answers,
                select.defaults,
            ));
        }
        if action == Action::BufferLock {
            let Some(view) = self.ed.magit.as_mut() else {
                return;
            };
            // Upstream cannot lock a status buffer either.
            if matches!(view.kind, Kind::Status) {
                return self.ed.set_err("Buffer locking is not supported here");
            }
            view.locked = !view.locked;
            let msg = if view.locked {
                "Locked buffer to its value"
            } else {
                "Unlocked buffer"
            };
            return self.ed.set_msg(msg);
        }
        if action == Action::SaveMessage {
            return crate::magit::message::save_message(&mut self.ed);
        }
        if let Action::Changelog(gnu) = action {
            return crate::magit::message::insert_changelog(&mut self.ed, gnu);
        }
        if matches!(action, Action::CommitAddLog | Action::AddChangeLogEntry) {
            let Some((file, defun)) = self.hunk_defun() else {
                return self.ed.set_err("No file or hunk at point");
            };
            if action == Action::AddChangeLogEntry {
                return self.add_change_log_entry(file, defun);
            }
            self.pending_add_log = Some((file, defun));
            return self.magit_action(Action::Commit);
        }
        if action == Action::ReverseInIndex {
            let Some(view) = self.ed.magit.as_deref() else {
                return;
            };
            // magit-unstage-committed: only committed changes are reversed here.
            if !crate::magit::options::flag("magit-unstage-committed", true) {
                return self.ed.set_err("Cannot unstage committed changes");
            }
            if !crate::magit::committed_diff(&view.kind) {
                return self.ed.set_err("Cannot reverse this change in the index");
            }
            let (repo, line) = (view.repo.clone(), self.ed.cur.line);
            crate::magit::prompt(
                &mut self.ed,
                crate::magit::Prompt::Ask(
                    repo,
                    Question::ReverseDiff(line, true),
                    vec![],
                    vec!["Reverse this change in the index? (y or n) ".into()],
                    vec![],
                ),
            );
            return;
        }
        if let Action::ApplyDiff(reverse) = action {
            use crate::magit::diff::Target;
            let Some(view) = self.ed.magit.as_deref() else {
                return;
            };
            // magit-apply / magit-reverse refusals.
            let refuse = match (&view.kind, reverse) {
                (Kind::Diff(Target::Unstaged, _), true) => Some("Cannot reverse unstaged changes"),
                (Kind::Diff(Target::Unstaged | Target::Staged, _), false) => {
                    Some("Change is already in the working tree")
                }
                (Kind::Diff(Target::Paths(..), _), _) => Some("Cannot apply a diff between files"),
                _ => None,
            };
            if let Some(e) = refuse {
                return self.ed.set_err(e);
            }
            let line = self.ed.cur.line;
            if reverse {
                // magit-confirm 'reverse.
                let repo = view.repo.clone();
                crate::magit::prompt(
                    &mut self.ed,
                    crate::magit::Prompt::Ask(
                        repo,
                        Question::ReverseDiff(line, false),
                        vec![],
                        vec!["Reverse this change in the worktree? (y or n) ".into()],
                        vec![],
                    ),
                );
                return;
            }
            return self.apply_diff(PatchUse::Apply, line);
        }
        if let Action::Trailer(key) = action {
            if self.ed.commit_repo.is_none() {
                return self.ed.set_err("Not in a commit message draft");
            }
            crate::magit::prompt(&mut self.ed, crate::magit::Prompt::Trailer(Some(key)));
            return;
        }
        if let Action::Answered(repo, Question::Ediff(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'V'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .ediff_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if matches!(action, Action::Ediff(_) | Action::EdiffDwim) {
            use crate::magit::ediff::Op as E;
            let from = self.magit_from();
            let at = self
                .ed
                .magit
                .as_ref()
                .and_then(|v| v.action_at(self.ed.cur.line));
            let (file, commit) = match &at {
                Some(RowAction::File(p, _) | RowAction::Hunk(p, ..)) => {
                    (Some(p.to_string_lossy().into_owned()), None)
                }
                Some(RowAction::Commit(id)) => (None, Some(id.clone())),
                _ => (None, None),
            };
            // magit-ediff-dwim: by the section at point, else the menu.
            let op = match (action, &at) {
                (Action::Ediff(op), _) => op,
                (_, Some(RowAction::File(_, Section::Conflicts))) => E::Resolve,
                (_, Some(RowAction::File(_, Section::Staged) | RowAction::Hunk(_, true, ..))) => {
                    E::ShowStaged
                }
                (_, Some(RowAction::File(..) | RowAction::Hunk(..))) => E::ShowUnstaged,
                (_, Some(RowAction::Commit(_))) => E::ShowCommit,
                (_, Some(RowAction::Stash(_))) => E::ShowStash,
                _ => return crate::magit::open_menu(&mut self.ed, 'U'),
            };
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.ediff_prompts(&op, file, commit);
                Ok(Outcome::Ask(repo, Question::Ediff(op), defaults, prompts))
            });
            return;
        }
        // magit-dired-log: the marked files, or the directory.
        if action == Action::FileLog && self.ed.dired.is_some() {
            let marked = crate::dired::marked(&self.ed);
            let dir = self
                .ed
                .dired
                .as_ref()
                .map(|d| d.dir.clone())
                .unwrap_or_default();
            let files = if marked.is_empty() { vec![dir] } else { marked };
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'l');
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let mut args = args;
                for f in &files {
                    let rel = repo_relative(&repo, f).unwrap_or_default();
                    let rel = rel.to_string_lossy();
                    args.push(format!("-- {}", if rel.is_empty() { "." } else { &rel }));
                }
                let rev = repo.current_branch().unwrap_or_else(|_| "HEAD".into());
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::View(Kind::Log(vec![rev], args)),
                    origin,
                ))
            });
            return;
        }
        if action == Action::DiredJump {
            // magit-dired-jump: dired at the file at point, else the toplevel.
            let Some(view) = self.ed.magit.as_ref() else {
                return self.ed.set_err("Not in a Magit buffer");
            };
            let file = match view.action_at(self.ed.cur.line) {
                Some(RowAction::File(p, _) | RowAction::Hunk(p, ..)) => {
                    Some(view.repo.root.join(p))
                }
                _ => None,
            };
            let dir = file
                .as_ref()
                .and_then(|f| f.parent().map(Path::to_path_buf))
                .unwrap_or(view.repo.root.clone());
            // ponytail: opens the directory; point is not moved to the file.
            self.open_pick(dir, None);
            return;
        }
        if action == Action::DiffUnmerged {
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'd');
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                if !repo.merge_in_progress() {
                    return Err("No merge is in progress".into());
                }
                // magit--merge-range: merge base .. MERGE_HEAD.
                diff_view(
                    repo,
                    crate::magit::diff::Target::Range("HEAD...MERGE_HEAD".into()),
                    args,
                    origin,
                )
            });
            return;
        }
        if let Action::RevisionJump(part) = action {
            // A commit buffer: headers, then a blank line, the message, then
            // notes, the diffstat and the diff.
            let Some(view) = self.ed.magit.as_ref() else {
                return;
            };
            let rows: Vec<&str> = view.rows.iter().map(|r| r.text.as_str()).collect();
            let blank = rows.iter().position(|l| l.trim().is_empty());
            let target = match part {
                "headers" => Some(0),
                "message" => blank.map(|b| b + 1),
                "notes" => rows.iter().position(|l| l.starts_with("Notes")),
                "diffstat" => crate::magit::diff::stat_or_diff(&rows, 0)
                    .filter(|i| !rows[*i].starts_with("diff ")),
                _ => rows.iter().position(|l| l.starts_with("diff ")),
            };
            match target {
                Some(i) => self.ed.set_cursor(i, 0),
                None => self.ed.set_msg(format!("No {part} section")),
            }
            return;
        }
        if action == Action::BlameVisitFile {
            let Some(b) = self.ed.blame.as_ref() else {
                return self.ed.set_err("Not in a blame buffer");
            };
            let Some(c) = b.chunk_at(self.ed.cur.line) else {
                return self.ed.set_err("No blame chunk here");
            };
            let (repo, rev, file, line) = (
                b.repo.clone(),
                c.rev.clone(),
                c.orig_file.clone(),
                c.orig_line,
            );
            self.start_magit(move || blob_outcome(repo, rev, file, line, None, None));
            return;
        }
        if action == Action::LogHalfLimit {
            match self.ed.magit.as_mut().map(|v| &mut v.kind) {
                Some(Kind::Log(_, args)) => {
                    let n = crate::magit::log::limit(args)
                        .map(|n| n / 2)
                        .filter(|n| *n > 0);
                    *args = crate::magit::log::with_limit(args, n);
                }
                _ => return self.ed.set_err("Not in a log buffer"),
            }
            self.magit_action(Action::Refresh);
            return;
        }
        if action == Action::Version {
            return self
                .ed
                .set_msg(concat!("Magit port in fred ", env!("CARGO_PKG_VERSION")));
        }
        if action == Action::LogRefresh {
            let args = crate::magit::menu_arguments(&self.ed, 'l');
            match self.ed.magit.as_mut().map(|v| &mut v.kind) {
                Some(Kind::Log(_, buffer_args)) => *buffer_args = args,
                Some(Kind::Reflog(_)) => {
                    return self
                        .ed
                        .set_err("Cannot change log arguments in reflog buffers");
                }
                Some(Kind::Cherry(..)) => {
                    return self
                        .ed
                        .set_err("Cannot change log arguments in cherry buffers");
                }
                _ => return self.ed.set_err("Not in a log buffer"),
            }
            self.magit_action(Action::Refresh);
            return;
        }
        if action == Action::ProcessBuffer {
            self.git_log.extend(crate::magit::repo::take_calls());
            let excess = self.git_log.len().saturating_sub(1000);
            self.git_log.drain(..excess);
            // magit-process-buffer: this repository's Git commands, newest last.
            let Ok(repo) = Repo::discover(&self.magit_from()) else {
                return self.ed.set_err("not in a Git repository");
            };
            let mut view = View::status(repo.clone(), Default::default());
            view.kind = Kind::Process;
            view.return_to = self.cur;
            view.rows = vec![Row {
                text: format!("Git commands in {} (q return)", repo.root.display()),
                action: None,
            }];
            for (root, line, result) in &self.git_log {
                if *root != repo.root {
                    continue;
                }
                view.rows.push(Row {
                    text: format!("{} git {line}", if result.is_ok() { "  0" } else { "  1" }),
                    action: None,
                });
                if let Err(e) = result {
                    for l in e.lines() {
                        view.rows.push(Row {
                            text: format!("    {}", label(Path::new(l))),
                            action: None,
                        });
                    }
                }
            }
            if view.rows.len() == 1 {
                view.rows.push(Row {
                    text: "No Git commands run yet".into(),
                    action: None,
                });
            }
            let last = view.rows.len() - 1;
            self.install_magit(view, None, last);
            return;
        }
        if action == Action::StashPush {
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'Q');
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                // Options, then the file limit after "--" as a literal pathspec.
                let mut argv: Vec<OsString> = vec!["stash".into(), "push".into()];
                argv.extend(
                    args.iter()
                        .filter(|a| !a.starts_with("-- "))
                        .map(OsString::from),
                );
                argv.push("--".into());
                for f in args.iter().filter_map(|a| a.strip_prefix("-- ")) {
                    argv.push(format!(":(literal){f}").into());
                }
                let next = match repo.run(&argv, None) {
                    Ok(_) => crate::magit::branch::Next::Done(Ok("Stashed".into())),
                    Err(e) => crate::magit::branch::Next::Done(Err(e)),
                };
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if action == Action::RefreshAll {
            let Some(repo) = self.ed.magit.as_ref().map(|v| v.repo.clone()) else {
                return;
            };
            for ed in self.editors_mut() {
                if let Some(view) = &mut ed.magit
                    && view.repo == repo
                {
                    view.dirty = true;
                }
            }
            self.magit_action(Action::Refresh);
            return;
        }
        if let Action::Fold(how) = action {
            use crate::magit::Fold;
            let Some(mut view) = self.ed.magit.as_deref().cloned() else {
                return;
            };
            if view.kind != Kind::Status {
                return self.ed.set_err("Folding needs a status buffer");
            }
            let selected = view.action_at(self.ed.cur.line);
            let fallback = self.ed.cur.line;
            // Files whose diffs to load and show.
            let files_of = |view: &crate::magit::View, s: Section| -> Vec<(PathBuf, bool)> {
                if !matches!(s, Section::Unstaged | Section::Staged) {
                    return vec![];
                }
                view.snapshot
                    .entries
                    .iter()
                    .filter(|e| s.contains(e))
                    .map(|e| (e.path.clone(), s == Section::Staged))
                    .collect()
            };
            let mut load = vec![];
            let mut heading = None;
            match (how, &selected) {
                (Fold::Level(n), _) => {
                    view.expanded.clear();
                    view.closed.clear();
                    if n == 1 {
                        for r in &view.rows {
                            if let Some(RowAction::Section(s)) = r.action {
                                view.closed.insert(s);
                            }
                        }
                    }
                    if n >= 3 {
                        load.extend(files_of(&view, Section::Unstaged));
                        load.extend(files_of(&view, Section::Staged));
                    }
                }
                (Fold::LevelHere(n), Some(_)) => {
                    // The top-level section around point: the nearest heading
                    // above (commits and stashes included); point moves to it.
                    let s = view.rows[..=fallback.min(view.rows.len().saturating_sub(1))]
                        .iter()
                        .rev()
                        .find_map(|r| match r.action {
                            Some(RowAction::Section(s)) => Some(s),
                            _ => None,
                        });
                    if let Some(s) = s {
                        heading = Some(RowAction::Section(s));
                        let staged = s == Section::Staged;
                        if n == 1 {
                            view.closed.insert(s);
                        } else {
                            view.closed.remove(&s);
                            if matches!(s, Section::Unstaged | Section::Staged) {
                                view.expanded.retain(|(_, st)| *st != staged);
                            }
                        }
                        if n >= 3 {
                            load.extend(files_of(&view, s));
                        }
                    }
                }
                (Fold::Show, Some(RowAction::Section(s))) => {
                    view.closed.remove(s);
                }
                (Fold::Hide, Some(RowAction::Section(s))) => {
                    view.closed.insert(*s);
                }
                (Fold::ShowChildren, Some(RowAction::Section(s))) => {
                    view.closed.remove(s);
                    load.extend(files_of(&view, *s));
                }
                (Fold::HideChildren, Some(RowAction::Section(s)))
                    if matches!(s, Section::Unstaged | Section::Staged) =>
                {
                    let staged = *s == Section::Staged;
                    view.expanded.retain(|(_, st)| *st != staged);
                }
                (Fold::Show | Fold::ShowChildren, Some(RowAction::File(p, s)))
                    if matches!(s, Section::Unstaged | Section::Staged) =>
                {
                    load.push((p.clone(), *s == Section::Staged));
                }
                (Fold::Hide | Fold::HideChildren, Some(RowAction::File(p, s))) => {
                    view.expanded.remove(&(p.clone(), *s == Section::Staged));
                }
                (Fold::Hide | Fold::HideChildren, Some(RowAction::Hunk(p, staged, ..))) => {
                    view.expanded.remove(&(p.clone(), *staged));
                }
                _ => {}
            }
            let selected = heading.or(selected);
            if load.is_empty() {
                view.rebuild();
                self.install_magit(view, selected, fallback);
                return;
            }
            self.start_magit(move || {
                for key in load {
                    let diff = view.repo.diff(&key.0, key.1)?;
                    view.diffs.insert(key.clone(), diff);
                    view.expanded.insert(key);
                }
                view.rebuild();
                Ok(Outcome::View(Box::new(view), selected, fallback))
            });
            return;
        }
        if action == Action::RebaseShowCommit {
            let Some(plan) = self.ed.rebase_todo.clone() else {
                return;
            };
            let text = self.ed.buf.line(self.ed.cur.line);
            let Some(id) = crate::magit::rebase::line_commit(&text).map(str::to_owned) else {
                return self.ed.set_err("No commit on this line");
            };
            let origin = self.cur;
            self.start_magit(move || {
                let full = plan
                    .repo
                    .read(&[
                        "rev-parse",
                        "--verify",
                        "-q",
                        "--end-of-options",
                        &format!("{id}^{{commit}}"),
                    ])
                    .map_err(|_| format!("unknown commit {id}"))?;
                let full = String::from_utf8_lossy(&full).trim().to_owned();
                diff_view(
                    plan.repo,
                    crate::magit::diff::Target::Commit(full),
                    vec!["--stat".into(), "--no-ext-diff".into()],
                    origin,
                )
            });
            return;
        }
        if let Action::DiffRefresh(how) = action {
            use crate::magit::diff::Refresh;
            // magit-diff-toggle-file-filter: swap the "-- file" limit with
            // the suspended one.
            if how == Refresh::FileFilter {
                let toggled = match self.ed.magit.as_mut() {
                    Some(view) => match &mut view.kind {
                        Kind::Diff(_, args) => {
                            let current: Vec<String> = args
                                .iter()
                                .filter(|a| a.starts_with("-- "))
                                .cloned()
                                .collect();
                            if current.is_empty() && view.suspended.is_empty() {
                                Err("No file filter to toggle; set one with -- in the diff menu")
                            } else {
                                args.retain(|a| !a.starts_with("-- "));
                                args.extend(std::mem::take(&mut view.suspended));
                                view.suspended = current;
                                Ok(())
                            }
                        }
                        _ => Err("Not in a diff buffer"),
                    },
                    None => Err("Not in a diff buffer"),
                };
                match toggled {
                    Ok(()) => self.magit_action(Action::Refresh),
                    Err(e) => self.ed.set_err(e),
                }
                return;
            }
            let args = crate::magit::menu_arguments(&self.ed, 'd');
            let changed = match self.ed.magit.as_mut().map(|v| &mut v.kind) {
                Some(Kind::Diff(target, buffer_args)) => match how {
                    Refresh::Buffer => {
                        // --cached is part of what the buffer shows, not a menu option.
                        let cached = buffer_args.iter().any(|a| a == "--cached");
                        *buffer_args = args;
                        if cached {
                            buffer_args.push("--cached".into());
                        }
                        Ok(())
                    }
                    how => target.refreshed(how).map(|t| *target = t),
                },
                _ => Err("Not in a diff buffer".into()),
            };
            match changed {
                Ok(()) => self.magit_action(Action::Refresh),
                Err(e) => self.ed.set_err(e),
            }
            return;
        }
        if let Action::Answered(repo, Question::Apply(op), answers, _) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let next = repo
                    .apply_step(op, answers.first().map(String::as_str).unwrap_or(""))
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::ApplyOp(kind) = action {
            use crate::magit::apply::{Op, Thing};
            let Some(view) = self.ed.magit.as_deref() else {
                return;
            };
            // The files and statuses this buffer showed.
            let shown = |s: Section| -> Vec<PathBuf> {
                view.snapshot
                    .entries
                    .iter()
                    .filter(|e| s.contains(e))
                    .map(|e| e.path.clone())
                    .collect()
            };
            let thing = match view.action_at(self.ed.cur.line) {
                Some(RowAction::Section(s)) => Some(Thing::Section(s, shown(s))),
                Some(RowAction::File(p, s)) => {
                    let xy = view
                        .snapshot
                        .entries
                        .iter()
                        .find(|e| e.path == p)
                        .map(|e| e.xy.clone())
                        .unwrap_or_default();
                    Some(Thing::File(p, s, xy))
                }
                Some(RowAction::Hunk(p, staged, i, _)) => view
                    .diffs
                    .get(&(p, staged))
                    .map(|d| Thing::Hunk(d.clone(), i)),
                _ => None,
            };
            let (repo, origin) = (view.repo.clone(), self.cur);
            let op = Op { kind, thing };
            match op.question() {
                Err(e) => self.ed.set_err(e),
                Ok(Some(q)) => crate::magit::prompt(
                    &mut self.ed,
                    crate::magit::Prompt::Ask(repo, Question::Apply(op), vec![], vec![q], vec![]),
                ),
                Ok(None) => self.start_magit(move || {
                    let next = repo
                        .apply_step(op, "")
                        .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                    Ok(branch_outcome(repo, next, origin))
                }),
            }
            return;
        }
        if let Action::Answered(repo, Question::Configure(op), answers, defaults) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .configure_step(op, &merged)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Configure(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.configure_prompts(&op)?;
                if prompts.is_empty() {
                    let next = repo.configure_step(op, &[])?;
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(
                    repo,
                    Question::Configure(op),
                    defaults,
                    prompts,
                ))
            });
            return;
        }
        if let Action::Answered(repo, Question::Ignore(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, '>'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .ignore_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Ignore(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, '>');
            // magit-current-file: the file at point, or the visited file.
            let file = self
                .ed
                .magit
                .as_ref()
                .and_then(|v| match v.action_at(self.ed.cur.line) {
                    Some(RowAction::File(p, _)) | Some(RowAction::Hunk(p, ..)) => {
                        Some(p.to_string_lossy().into_owned())
                    }
                    _ => None,
                });
            let visited = self.ed.path.clone();
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let file = file.or_else(|| {
                    visited
                        .and_then(|p| repo_relative(&repo, &p).ok())
                        .map(|p| p.to_string_lossy().into_owned())
                });
                let (prompts, defaults) = repo.ignore_prompts(&op, file);
                if prompts.is_empty() {
                    let next = repo.ignore_step(op, &[], &args)?;
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Ignore(op), defaults, prompts))
            });
            return;
        }
        if let Action::Answered(repo, Question::Refs(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'y'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .refs_focus(&op, merged.first().map(String::as_str))
                    .map(|focus| {
                        crate::magit::branch::Next::View(Kind::Refs(
                            focus,
                            args,
                            Default::default(),
                        ))
                    })
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Refs(op) = action {
            use crate::magit::refs::Op as Y;
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'y');
            if op == Y::Count {
                // magit-refs-set-show-commit-count: only in refs buffers.
                let next = match self.ed.magit.as_mut().map(|v| &mut v.kind) {
                    Some(Kind::Refs(_, _, count)) => {
                        *count = count.next();
                        Some(*count)
                    }
                    _ => None,
                };
                match next {
                    Some(count) => {
                        self.magit_action(Action::Refresh);
                        self.ed.set_msg(format!("Show commit counts for {count:?}"));
                    }
                    None => self.ed.set_err("Not in a refs buffer"),
                }
                return;
            }
            let count = match self.ed.magit.as_ref().map(|v| &v.kind) {
                Some(Kind::Refs(_, _, count)) => *count,
                _ => crate::magit::refs::initial_count(),
            };
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                if op == Y::Other {
                    return Ok(Outcome::Ask(
                        repo,
                        Question::Refs(op),
                        vec![String::new()],
                        vec!["Compare with: ".into()],
                    ));
                }
                let focus = repo.refs_focus(&op, None)?;
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::View(Kind::Refs(focus, args, count)),
                    origin,
                ))
            });
            return;
        }
        if let Action::Answered(repo, Question::Bundle(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'j'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .bundle_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Bundle(op) = action {
            let from = self.magit_from();
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::File(p, _)) => Some(p.to_string_lossy().into_owned()),
                        _ => None,
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.bundle_prompts(&op, at_point);
                Ok(Outcome::Ask(repo, Question::Bundle(op), defaults, prompts))
            });
            return;
        }
        if let Action::Answered(base, Question::Clone(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'k'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = base
                    .clone_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(base, next, origin))
            });
            return;
        }
        if let Action::Clone(op) = action {
            // magit-clone reads relative to the current buffer's directory.
            let from = self.magit_from();
            let base = if from.is_dir() {
                from
            } else {
                from.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            let base = std::path::absolute(&base).unwrap_or(base);
            let (prompts, defaults) = Repo::clone_prompts(&op);
            crate::magit::prompt(
                &mut self.ed,
                crate::magit::Prompt::Ask(
                    Repo { root: base },
                    Question::Clone(op),
                    defaults,
                    prompts,
                    vec![],
                ),
            );
            return;
        }
        if let Action::Answered(repo, Question::Patch(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, op.menu()));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .patch_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Patch(op) = action {
            use crate::magit::patch::Op as W;
            let (origin, from) = (self.cur, self.magit_from());
            let view = self.ed.magit.as_ref();
            // magit-patch-save works on the diff buffer's range and arguments.
            // Revision and stash buffers are diff buffers upstream too.
            let op = match (op, view.map(|v| &v.kind)) {
                (W::Save, Some(Kind::Diff(target, args))) => {
                    W::SaveDiff(target.clone(), args.clone())
                }
                (W::Save, Some(Kind::Patch(id))) => {
                    W::SaveDiff(crate::magit::diff::Target::Commit(id.clone()), vec![])
                }
                (W::Save, Some(Kind::StashPatch(stash))) => W::SaveDiff(
                    crate::magit::diff::Target::Range(format!("{0}^1..{0}", stash.id)),
                    vec![],
                ),
                (op, _) => op,
            };
            // The am menu's idle "a" is magit-patch-apply's own menu.
            // ponytail: synchronous git-path check; local and fast.
            if op == W::AmApply && !Repo::discover(&from).is_ok_and(|r| r.am_in_progress()) {
                crate::magit::open_menu(&mut self.ed, 'a');
                return;
            }
            let commit = view.and_then(|v| match v.action_at(self.ed.cur.line) {
                Some(RowAction::Commit(id)) => Some(id),
                _ => None,
            });
            let file = view.and_then(|v| match v.action_at(self.ed.cur.line) {
                Some(RowAction::File(p, _)) | Some(RowAction::Hunk(p, ..)) => v
                    .repo
                    .root
                    .join(&p)
                    .is_file()
                    .then(|| p.to_string_lossy().into_owned()),
                _ => None,
            });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let op = repo.patch_resolve(op)?;
                let args = vec![];
                let (prompts, defaults) = repo.patch_prompts(&op, commit, file);
                if prompts.is_empty() {
                    let next = repo.patch_step(op, &[], &args)?;
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Patch(op), defaults, prompts))
            });
            return;
        }
        if let Action::Answered(repo, Question::Subtree(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, op.menu()));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .subtree_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Subtree(op) = action {
            let from = self.magit_from();
            let args = crate::magit::menu_arguments(&self.ed, op.menu());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.subtree_prompts(&op, &args);
                Ok(Outcome::Ask(repo, Question::Subtree(op), defaults, prompts))
            });
            return;
        }
        if let Action::Submodule(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'o');
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Module(m)) => Some(m),
                        Some(RowAction::File(p, _)) => Some(p.to_string_lossy().into_owned()),
                        _ => None,
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let at_point =
                    at_point.filter(|m| repo.module_paths().is_ok_and(|v| v.contains(m)));
                let (prompts, defaults) = repo.submodule_prompts(&op, at_point);
                if prompts.is_empty() {
                    let next = repo.submodule_step(op, &[], &args)?;
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(
                    repo,
                    Question::Submodule(op),
                    defaults,
                    prompts,
                ))
            });
            return;
        }
        if let Action::LogOp(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'l');
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Commit(id)) => Some(id),
                        _ => None,
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.log_prompts(&op, at_point);
                if prompts.is_empty() {
                    let next = repo.log_step(op, &[], &args)?;
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Log(op), defaults, prompts))
            });
            return;
        }
        if let Action::Answered(repo, Question::Bisect(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'G'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .bisect_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Bisect(op) = action {
            use crate::magit::bisect::Op as G;
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'G');
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let op = match (repo.bisecting(), op) {
                    (true, G::Start) => G::Bad,
                    (false, G::Good | G::Mark | G::Skip | G::Reset) => {
                        return Err("Not bisecting".into());
                    }
                    (_, op) => op,
                };
                let (prompts, defaults) = repo.bisect_prompts(&op);
                if prompts.is_empty() {
                    let next = repo
                        .bisect_step(op, &[], &args)
                        .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Bisect(op), defaults, prompts))
            });
            return;
        }
        if let Action::Answered(repo, Question::Notes(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'N'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .notes_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Notes(op) = action {
            use crate::magit::notes::Op as N;
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'N');
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Commit(id)) => Some(id),
                        Some(RowAction::Stash(s)) => Some(s.id),
                        _ => None,
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let op = match (repo.notes_merging(), op) {
                    (true, N::NotesRef(false)) => N::MergeCommit,
                    (true, op @ (N::MergeAbort | N::NotesRef(true) | N::DisplayRef(_))) => op,
                    (true, _) => {
                        return Err("A notes merge is in progress: c commit, a abort".into());
                    }
                    (false, N::MergeAbort | N::MergeCommit) => {
                        return Err("No notes merge in progress".into());
                    }
                    (false, op) => op,
                };
                let (prompts, defaults) = repo.notes_prompts(op, at_point);
                if prompts.is_empty() {
                    let next = repo
                        .notes_step(op, &[], &args)
                        .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Notes(op), defaults, prompts))
            });
            return;
        }
        if let Action::Answered(repo, Question::Worktree(op), answers, defaults) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .worktree_step(op, &merged)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Worktree(op) = action {
            let from = self.magit_from();
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.worktree_prompts(&op, None);
                Ok(Outcome::Ask(
                    repo,
                    Question::Worktree(op),
                    defaults,
                    prompts,
                ))
            });
            return;
        }
        if let Action::Answered(repo, Question::Reflog, answers, _) = action {
            let origin = self.cur;
            let target = answers.first().cloned().unwrap_or_default();
            self.start_magit(move || reflog_view(repo, target, origin));
            return;
        }
        if let Action::Reflog(target) = action {
            let (origin, from) = (self.cur, self.magit_from());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                match target {
                    None => Ok(Outcome::Ask(
                        repo,
                        Question::Reflog,
                        vec![String::new()],
                        vec!["Show reflog for: ".into()],
                    )),
                    // magit-reflog-current: the branch, or HEAD when detached.
                    Some(r) if r.is_empty() => {
                        let r = repo.current_branch().unwrap_or_else(|_| "HEAD".into());
                        reflog_view(repo, r, origin)
                    }
                    Some(r) => reflog_view(repo, r, origin),
                }
            });
            return;
        }
        if let Action::Answered(repo, Question::Stash(op), answers, defaults) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .stash_step(op, &merged)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::StashOp(op) = action {
            let from = self.magit_from();
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        // The object id names the stash even if selectors renumber.
                        Some(RowAction::Stash(stash)) => Some(stash.id),
                        _ => match &v.kind {
                            Kind::StashPatch(stash) => Some(stash.id.clone()),
                            _ => None,
                        },
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.stash_prompts(&op, at_point);
                Ok(Outcome::Ask(repo, Question::Stash(op), defaults, prompts))
            });
            return;
        }
        if let Action::Answered(
            _,
            Question::Commit(crate::magit::commit::Op::DraftAll),
            answers,
            _,
        ) = action
        {
            if !matches!(answers.first().map(|a| a.trim()), Some("y" | "yes")) {
                return self.ed.set_err("Nothing staged");
            }
            // This commit only: --all, as upstream's (cons "--all" args).
            let added = self
                .ed
                .magit_options
                .insert(crate::magit::MenuOption::CommitAll);
            self.magit_action(Action::Commit);
            if added {
                self.ed
                    .magit_options
                    .remove(&crate::magit::MenuOption::CommitAll);
            }
            return;
        }
        if let Action::Answered(repo, Question::Commit(op), answers, defaults) = action {
            let args = match op.arg_menu() {
                Some(menu) => crate::magit::menu_arguments(&self.ed, menu),
                None => crate::magit::commit_arguments(&self.ed),
            };
            let origin = self.cur;
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .commit_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::CommitEdit(op) = action {
            let from = self.magit_from();
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Commit(id)) => Some(id),
                        _ => match &v.kind {
                            Kind::Patch(id) | Kind::Diff(Target::Commit(id), _) => Some(id.clone()),
                            _ => None,
                        },
                    });
            let origin = self.cur;
            self.start_magit(move || {
                use crate::magit::commit::Op as C;
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.commit_prompts(&op, at_point.clone());
                let pick =
                    |verb: &str| format!("Type . or C-c C-c on a commit to {verb}, or q to abort");
                match &op {
                    // magit-commit-squash-internal: the commit at point, unless
                    // magit-commit-squash-confirm (or an instant variant) asks
                    // for it in magit-log-select, starting on that commit.
                    C::Fixup
                    | C::Squash
                    | C::Alter
                    | C::Augment
                    | C::Revise
                    | C::InstantFixup
                    | C::InstantSquash => {
                        let instant = matches!(op, C::InstantFixup | C::InstantSquash);
                        let confirm = instant
                            || crate::magit::options::flag("magit-commit-squash-confirm", true);
                        match at_point {
                            Some(id) if !confirm => Ok(Outcome::Answer(
                                repo,
                                Question::Commit(op),
                                vec![id.clone()],
                                vec![id],
                            )),
                            initial => {
                                let msg = pick(&format!("{} it", op_verb(&op)));
                                log_select(
                                    repo,
                                    Question::Commit(op),
                                    vec![],
                                    defaults,
                                    msg,
                                    initial,
                                    origin,
                                )
                            }
                        }
                    }
                    // Absorb into commits since the one picked, starting at the
                    // upstream's merge base.
                    C::Autofixup | C::Absorb | C::AbsorbModules => {
                        let initial = defaults
                            .first()
                            .filter(|d| !d.is_empty())
                            .and_then(|d| {
                                repo.read(&["merge-base", "--end-of-options", d, "HEAD"])
                                    .ok()
                            })
                            .map(|o| String::from_utf8_lossy(&o).trim().to_owned());
                        let msg = pick("absorb into commits since it");
                        log_select(
                            repo,
                            Question::Commit(op),
                            vec![],
                            defaults,
                            msg,
                            initial,
                            origin,
                        )
                    }
                    C::ReshelveSince => {
                        let msg = pick("reshelve it and the commits above it");
                        log_select(
                            repo,
                            Question::Commit(op),
                            vec![],
                            vec![String::new()],
                            msg,
                            None,
                            origin,
                        )
                    }
                    _ => Ok(Outcome::Ask(repo, Question::Commit(op), defaults, prompts)),
                }
            });
            return;
        }
        if let Action::Answered(repo, Question::Rebase(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'r'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .rebase_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Rebase(op) = action {
            use crate::magit::rebase::Op as R;
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'r');
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Commit(id)) => Some(id),
                        _ => match &v.kind {
                            Kind::Patch(id) | Kind::Diff(Target::Commit(id), _) => Some(id.clone()),
                            _ => None,
                        },
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                // Upstream's in-progress group: r continue, s skip, e edit todo, a abort.
                let op = match (repo.rebase_in_progress(), op) {
                    (true, op @ (R::Continue | R::Abort)) => op,
                    (true, R::Subset) => R::Skip,
                    (true, R::Elsewhere) => R::EditTodo,
                    (true, _) => {
                        return Err(
                            "A rebase is in progress: r continue, s skip, e edit, a abort".into(),
                        );
                    }
                    (false, R::Continue | R::Abort) => return Err("No rebase in progress".into()),
                    (false, op) => op,
                };
                // magit-rebase-interactive-1: the commit at point, else
                // magit-log-select.
                if matches!(
                    op,
                    R::Interactive | R::EditCommit | R::RewordCommit | R::RemoveCommit
                ) {
                    return match at_point {
                        Some(id) => Ok(Outcome::Answer(
                            repo,
                            Question::Rebase(op),
                            vec![id.clone()],
                            vec![id],
                        )),
                        None => {
                            let verb = match op {
                                R::Interactive => "rebase it and all commits above it",
                                R::EditCommit => "edit it",
                                R::RewordCommit => "reword its message",
                                _ => "remove it",
                            };
                            let msg =
                                format!("Type . or C-c C-c on a commit to {verb}, or q to abort");
                            log_select(
                                repo,
                                Question::Rebase(op),
                                vec![],
                                vec![String::new()],
                                msg,
                                None,
                                origin,
                            )
                        }
                    };
                }
                let (prompts, defaults) = repo.rebase_prompts(&op, at_point)?;
                if prompts.is_empty() {
                    let next = repo
                        .rebase_step(op, &[], &args)
                        .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Rebase(op), defaults, prompts))
            });
            return;
        }
        // git-rebase-confirm-cancel: an edited todo list asks first.
        if action == Action::RebaseCancel
            && self.ed.rebase_todo.is_some()
            && self.ed.buf.modified
            && crate::magit::options::flag("git-rebase-confirm-cancel", true)
            && crate::magit::options::confirm("abort-rebase")
        {
            return crate::magit::prompt(&mut self.ed, crate::magit::Prompt::TodoCancel);
        }
        let action = if action == Action::RebaseCancelConfirmed {
            Action::RebaseCancel
        } else {
            action
        };
        if action == Action::RebaseFinish || action == Action::RebaseCancel {
            let Some(plan) = self.ed.rebase_todo.take() else {
                return self.ed.set_err("Not a rebase todo buffer");
            };
            if action == Action::RebaseCancel {
                let _ = std::fs::remove_file(&plan.todo);
                self.buffer(BufCmd::Delete, "", true);
                return self.ed.set_msg("Rebase cancelled");
            }
            // with-editor-finish: save the list, then let Git run it.
            if let Err(e) = std::fs::write(&plan.todo, self.ed.buf.to_bytes()) {
                self.ed.rebase_todo = Some(plan);
                return self.ed.set_err(e.to_string());
            }
            self.ed.buf.modified = false;
            match plan.replay() {
                Ok(inv) => {
                    self.buffer(BufCmd::Delete, "", true);
                    self.pending_git = Some(inv);
                }
                Err(e) => {
                    self.ed.rebase_todo = Some(plan);
                    self.ed.set_err(e);
                }
            }
            return;
        }
        if let Action::Answered(repo, Question::Sequence(op), answers, defaults) = action {
            let menu = sequence_menu(&op);
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, menu));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .sequence_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Sequence(op) = action {
            use crate::magit::sequence::Op as S;
            let origin = self.cur;
            let from = self.magit_from();
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Commit(id)) => Some(id),
                        _ => match &v.kind {
                            Kind::Patch(id) | Kind::Diff(Target::Commit(id), _) => Some(id.clone()),
                            _ => None,
                        },
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                // Upstream's in-progress group shares keys with the suffixes.
                // The in-progress group acts on whichever sequence is running.
                let op = match (repo.sequencer(), op) {
                    (Some(_), S::Pick | S::Revert | S::Continue) => S::Continue,
                    (Some(_), S::Spinoff | S::Skip) => S::Skip,
                    (Some(_), S::Apply | S::Abort) => S::Abort,
                    (Some(kind), _) => {
                        return Err(format!(
                            "A {kind} is in progress: continue, skip or abort it first"
                        ));
                    }
                    (None, S::Continue | S::Skip | S::Abort) => {
                        return Err("No cherry-pick or revert in progress".into());
                    }
                    (None, op) => op,
                };
                if matches!(op, S::Continue | S::Skip) {
                    let next = repo
                        .sequence_step(op, &[], &[])
                        .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                    return Ok(branch_outcome(repo, next, origin));
                }
                let (prompts, defaults) = repo.sequence_prompts(&op, at_point)?;
                Ok(Outcome::Ask(
                    repo,
                    Question::Sequence(op),
                    defaults,
                    prompts,
                ))
            });
            return;
        }
        if let Action::Answered(repo, Question::Remote(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'O'));
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo
                    .remote_step(op, &merged, &args)
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Action::Remote(op) = action {
            let from = self.magit_from();
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.remote_prompts(&op)?;
                Ok(Outcome::Ask(repo, Question::Remote(op), defaults, prompts))
            });
            return;
        }
        if let Action::Reset(op) = action {
            let from = self.magit_from();
            // magit-read-branch-or-commit defaults to the commit at point.
            let at_point =
                self.ed
                    .magit
                    .as_ref()
                    .and_then(|v| match v.action_at(self.ed.cur.line) {
                        Some(RowAction::Commit(id)) => Some(id),
                        _ => match &v.kind {
                            Kind::Patch(id) | Kind::Diff(Target::Commit(id), _) => Some(id.clone()),
                            _ => None,
                        },
                    });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.reset_prompt(op, at_point);
                Ok(Outcome::Ask(repo, Question::Reset(op), defaults, prompts))
            });
            return;
        }
        if let Action::Merge(op) = action {
            use crate::magit::merge::Op as M;
            let (origin, from) = (self.cur, self.magit_from());
            let commit_args = crate::magit::commit_arguments(&self.ed);
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                // Upstream's in-progress group: m commits the merge, a aborts it.
                let op = match op {
                    M::Plain if repo.merge_in_progress() => {
                        let message = repo.merge_message();
                        return Ok(Outcome::Draft(
                            repo,
                            crate::magit::CommitMode::New,
                            message,
                            commit_args,
                        ));
                    }
                    M::Absorb if repo.merge_in_progress() => M::Abort,
                    _ if repo.merge_in_progress() => {
                        return Err("A merge is in progress: m commits it, a aborts it".into());
                    }
                    op => op,
                };
                let (prompts, defaults) = repo.merge_prompts(&op)?;
                let _ = origin;
                Ok(Outcome::Ask(repo, Question::Merge(op), defaults, prompts))
            });
            return;
        }
        if let Action::Tag(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 't');
            // The tag at point in the tags list.
            let at_point = self
                .ed
                .magit
                .as_ref()
                .filter(|v| v.kind == Kind::Tags)
                .and_then(|v| {
                    v.action_at(self.ed.cur.line)
                        .map(|_| self.ed.buf.line(self.ed.cur.line).to_string())
                });
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.tag_prompts(&op, &args, at_point)?;
                if prompts.is_empty() {
                    let next = repo.tag_step(op, &defaults, &args)?;
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Tag(op), defaults, prompts))
            });
            return;
        }
        if let Action::Branch(op) = action {
            let (origin, from) = (self.cur, self.magit_from());
            let args = crate::magit::menu_arguments(&self.ed, 'b');
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.branch_prompts(&op)?;
                if prompts.is_empty() {
                    let next = repo.branch_step_args(op, &[], &[], &args);
                    return Ok(branch_outcome(repo, next, origin));
                }
                Ok(Outcome::Ask(repo, Question::Branch(op), defaults, prompts))
            });
            return;
        }
        if let Action::File(op) = action {
            return self.file_action(op);
        }
        if let Action::Answered(repo, question, answers, args) = action {
            let origin = self.cur;
            let line = self.ed.cur.line;
            self.start_magit(move || match question {
                Question::File(_)
                | Question::Branch(_)
                | Question::Tag(_)
                | Question::Merge(_)
                | Question::Reset(_)
                | Question::Remote(_)
                | Question::Sequence(_)
                | Question::Rebase(_)
                | Question::Commit(_)
                | Question::Stash(_)
                | Question::Worktree(_)
                | Question::Reflog
                | Question::Notes(_)
                | Question::Bisect(_)
                | Question::Log(_)
                | Question::Submodule(_)
                | Question::Subtree(_)
                | Question::Patch(_)
                | Question::Bundle(_)
                | Question::Clone(_)
                | Question::Refs(_)
                | Question::Ignore(_)
                | Question::Configure(_)
                | Question::Apply(_)
                | Question::Misc(_)
                | Question::Wip(_)
                | Question::Ediff(_)
                | Question::ReverseDiff(..) => {
                    unreachable!("handled before the worker")
                }
                Question::FindFile => {
                    let pick = |i: usize| {
                        answers
                            .get(i)
                            .filter(|a| !a.is_empty())
                            .or(args.get(i))
                            .cloned()
                            .unwrap_or_default()
                    };
                    let rev = repo.blob_rev(&pick(0))?;
                    let file = PathBuf::from(pick(1));
                    if file.as_os_str().is_empty()
                        || file
                            .components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                    {
                        return Err("file must be repository-relative".into());
                    }
                    blob_outcome(repo, rev, file, line, None, None)
                }
                Question::Net(op) => Ok(Outcome::ConfiguredGit(repo.network(op, &answers, &args)?)),
                Question::Diff(DiffOp::ShowStash) => {
                    let wanted = answers.first().map(String::as_str).unwrap_or("");
                    let stash = repo
                        .stashes()?
                        .into_iter()
                        .find(|s| s.selector == wanted || s.id == wanted)
                        .ok_or_else(|| format!("no stash {wanted:?}"))?;
                    stash_view(repo, stash, origin)
                }
                Question::Diff(op) => {
                    let target = repo.diff_target(op, &answers)?;
                    diff_view(repo, target, args, origin)
                }
            });
            return;
        }
        if let Action::Blame(kind) = action {
            use crate::magit::blame::Kind;
            // magit-blame--pre-blame-setup: the same type recurses from any entry point.
            if let Some(b) = self
                .ed
                .blame
                .as_ref()
                .filter(|b| b.kind == kind && !b.echo())
            {
                match b.chunk_at(self.ed.cur.line).cloned() {
                    Some(crate::magit::blame::Chunk {
                        prev: Some((rev, file)),
                        orig_line,
                        ..
                    }) => {
                        return self.magit_action(Action::BlobVisitBlame(
                            rev,
                            file,
                            kind,
                            orig_line.saturating_sub(1),
                        ));
                    }
                    _ => return self.ed.set_err("Chunk has no further history"),
                }
            }
            let blob = self.ed.blob.clone();
            if blob
                .as_ref()
                .is_some_and(|b| b.rev == crate::magit::blob::INDEX)
                && matches!(kind, Kind::Removal | Kind::Reverse)
            {
                return self
                    .ed
                    .set_err("The index cannot be blamed in reverse; visit a commit's blob");
            }
            let path = if blob.is_some() {
                None
            } else if let Some(path) = self.ed.path.clone().filter(|_| {
                self.ed.magit.is_none() && self.ed.dired.is_none() && self.ed.commit_repo.is_none()
            }) {
                Some(path)
            } else {
                self.ed.set_err("Buffer isn't visiting a file");
                return;
            };
            if blob.is_none() && matches!(kind, Kind::Removal | Kind::Reverse) {
                self.ed
                    .set_err("Only blob buffers can be blamed in reverse");
                return;
            }
            if blob.is_none() && self.ed.buf.modified {
                // ponytail: Git blames the saved file; unsaved lines would be misattributed.
                self.ed.set_err("Save the buffer before blaming");
                return;
            }
            let args = crate::magit::menu_arguments(&self.ed, 'B');
            let version = self.ed.buf.version;
            let was_readonly = self
                .ed
                .blame
                .as_ref()
                .map_or(self.ed.readonly, |b| b.was_readonly);
            self.start_magit(move || {
                let (repo, file, rev) = match (blob, path) {
                    // magit-blame--run: the index blob blames without a revision.
                    (Some(b), _) => (
                        b.repo,
                        b.file,
                        Some(b.rev).filter(|r| r != crate::magit::blob::INDEX),
                    ),
                    (None, Some(ref path)) => {
                        let repo = Repo::discover(path)?;
                        let file = repo_relative(&repo, path)?;
                        (repo, file, None)
                    }
                    (None, None) => return Err("Buffer isn't visiting a file".into()),
                };
                let (chunks, info) = repo.blame(&file, rev.as_deref(), kind, &args)?;
                Ok(Outcome::Blame(Box::new(crate::magit::blame::Blame {
                    repo,
                    file,
                    args,
                    chunks,
                    info,
                    style: 0,
                    kind,
                    rev,
                    version,
                    was_readonly,
                })))
            });
            return;
        }
        if let Some(outcome) = self.blob_action(&action) {
            let origin_line = self.ed.cur.line;
            self.start_magit(move || outcome(origin_line));
            return;
        }
        if action == Action::BlobQuit {
            self.buffer(BufCmd::Delete, "", false);
            return;
        }
        if action == Action::Init {
            let from = self.magit_from();
            let base = if from.is_dir() {
                from
            } else {
                from.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            let base = std::path::absolute(&base).unwrap_or(base);
            crate::magit::prompt(&mut self.ed, crate::magit::Prompt::InitDir(base));
            return;
        }
        if let Action::InitDir(dir, confirmed) = action {
            let origin = self.cur;
            self.start_magit(move || {
                let dir = std::path::absolute(&dir).map_err(|e| e.to_string())?;
                if !confirmed && let Ok(existing) = Repo::discover(&dir) {
                    let same = dir.canonicalize().is_ok_and(|d| d == existing.root);
                    let question = if same {
                        format!(
                            "Reinitialize existing repository {}? (y or n) ",
                            label(&dir)
                        )
                    } else {
                        format!(
                            "{} is a repository.  Create another in {}? (y or n) ",
                            label(&existing.root),
                            label(&dir)
                        )
                    };
                    return Ok(Outcome::InitConfirm(dir, question));
                }
                // Inside a .git directory or a bare repository there is no toplevel.
                let existing = dir.ancestors().find(|p| p.is_dir()).unwrap_or(&dir);
                let in_git_dir = std::process::Command::new("git")
                    .arg("-C")
                    .arg(existing)
                    .args(["rev-parse", "--git-dir"])
                    .stdin(std::process::Stdio::null())
                    .output()
                    .is_ok_and(|o| o.status.success());
                if !confirmed && in_git_dir {
                    return Ok(Outcome::InitConfirm(
                        dir.clone(),
                        format!(
                            "{} is inside a Git directory.  Create a repository there? (y or n) ",
                            label(&dir)
                        ),
                    ));
                }
                let out = std::process::Command::new("git")
                    .arg("init")
                    .arg("--")
                    .arg(&dir)
                    .stdin(std::process::Stdio::null())
                    .output()
                    .map_err(|e| format!("git: {e}"))?;
                if !out.status.success() {
                    return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
                }
                let repo = Repo::discover(&dir)?;
                let mut view = View::status(repo.clone(), repo.status()?);
                view.return_to = origin;
                Ok(Outcome::View(Box::new(view), None, 0))
            });
            return;
        }
        if let Action::Diff(op) = action {
            let args = crate::magit::menu_arguments(&self.ed, 'd');
            let resolved = diff_context(
                op,
                self.ed.magit.as_deref().map(|v| &v.kind),
                self.ed
                    .magit
                    .as_deref()
                    .and_then(|v| v.action_at(self.ed.cur.line)),
            );
            let from = self.magit_from();
            let origin = self.cur;
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                match resolved {
                    Some(Ok(target)) => diff_view(repo, target, args, origin),
                    Some(Err(stash)) => stash_view(repo, stash, origin),
                    None => Ok(Outcome::Ask(
                        repo,
                        Question::Diff(op),
                        args,
                        Repo::diff_prompts(op),
                    )),
                }
            });
            return;
        }
        if let Action::Submit(repo, operation, value, args) = action {
            self.start_magit(move || {
                if operation == crate::magit::workflows::Operation::StashWorktree {
                    let result = repo.save_stash(operation, &value, &args);
                    return Ok(Outcome::Saved(repo, result));
                }
                let mut inv = repo.operation(operation, &value)?;
                inv.args.extend(args.into_iter().map(OsString::from));
                Ok(Outcome::Git(inv))
            });
            return;
        }
        // git-commit-finish-query-functions: style questions first.
        if matches!(action, Action::Commit | Action::CommitAnyway) && self.ed.commit_repo.is_some()
        {
            if action == Action::Commit {
                let qs = crate::magit::message::style_questions(&self.ed.buf.text());
                if !qs.is_empty() {
                    return crate::magit::prompt(
                        &mut self.ed,
                        crate::magit::Prompt::CommitStyle(qs),
                    );
                }
            }
            if action == Action::CommitAnyway {
                // Skip the questions this once.
                return self.commit_draft();
            }
        }
        // with-editor-finish in a message Git is waiting for.
        if action == Action::Commit && self.ed.commit_repo.is_some() {
            return self.commit_draft();
        }
        if let Action::Switch(row, typed) = action {
            let Some(repo) = self.magit_picker_repo.clone() else {
                return self.ed.set_err("no repository selected for branch switch");
            };
            // A typed revision (tag, hash) wins over a fuzzy-matched branch row.
            let resolves = |r: &str| {
                !r.is_empty()
                    && !r.starts_with('-')
                    && repo
                        .read(&[
                            "rev-parse",
                            "--verify",
                            "-q",
                            "--end-of-options",
                            &format!("{r}^{{commit}}"),
                        ])
                        .is_ok()
            };
            let name = match row {
                Some(row) if row == typed || !resolves(&typed) => row,
                _ => typed,
            };
            // Validate after stripping, so "heads/--orphan=x" cannot become an option.
            let name = name.strip_prefix("heads/").unwrap_or(&name).to_owned();
            if name.is_empty() || name.starts_with('-') || name.chars().any(char::is_control) {
                return self.ed.set_err(format!("invalid revision {name:?}"));
            }
            if let Some(repo) = self.magit_picker_repo.take() {
                self.ed.mode = Mode::Normal;
                // magit-checkout: a branch, or any revision (detaching HEAD),
                // with magit-branch-arguments.
                let mut args: Vec<OsString> = vec!["checkout".into()];
                args.extend(
                    crate::magit::menu_arguments(&self.ed, 'b')
                        .into_iter()
                        .map(OsString::from),
                );
                args.extend([name.into(), "--".into()]);
                self.pending_git = Some(GitInvocation {
                    expected_head: None,
                    repo,
                    args,
                    input: None,
                    draft: None,
                    draft_stamp: None,
                    editor: false,
                    env: vec![],
                    after: None,
                });
            }
            return;
        }
        if matches!(
            action,
            Action::Toggle
                | Action::Stage
                | Action::Unstage
                | Action::Visit
                | Action::VisitWorktree
                | Action::Refresh
        ) {
            let Some(mut view) = self.ed.magit.as_deref().cloned() else {
                self.ed.set_err("open Git status first");
                return;
            };
            let selected = view.action_at(self.ed.cur.line);
            let fallback = self.ed.cur.line;
            if matches!(action, Action::Visit | Action::VisitWorktree) && selected.is_none() {
                // magit-diff-visit-file / -worktree-file on a diff line.
                match diff_visit(&view, self.ed.cur.line, action == Action::VisitWorktree) {
                    Some((rev, file, line)) => {
                        let repo = view.repo.clone();
                        self.start_magit(move || blob_outcome(repo, rev, file, line, None, None));
                    }
                    None => self.ed.set_err("Nothing to visit here"),
                }
                return;
            }
            if action == Action::Visit {
                match selected {
                    Some(RowAction::File(ref path, _))
                    | Some(RowAction::Hunk(ref path, _, _, _)) => {
                        let line = if let Some(RowAction::Hunk(_, _, _, line)) = selected {
                            line
                        } else {
                            0
                        };
                        self.open_pick(
                            view.repo.root.join(path),
                            Some(Goto {
                                line,
                                col: 0,
                                pattern: None,
                            }),
                        );
                    }
                    Some(RowAction::Stash(stash)) => {
                        view.return_to = self.cur;
                        let origin = view.return_to;
                        self.start_magit(move || stash_view(view.repo, stash, origin));
                    }
                    // magit-repolist-status.
                    Some(RowAction::Repo(dir)) => {
                        let origin = self.cur;
                        self.start_magit(move || {
                            Ok(branch_outcome(
                                Repo { root: dir.clone() },
                                crate::magit::branch::Next::Status(dir),
                                origin,
                            ))
                        });
                    }
                    Some(RowAction::Module(module)) => {
                        let origin = self.cur;
                        self.start_magit(move || {
                            let dir = view.repo.module_dir(&module)?;
                            Ok(branch_outcome(
                                view.repo,
                                crate::magit::branch::Next::Status(dir),
                                origin,
                            ))
                        });
                    }
                    Some(RowAction::Commit(id)) => {
                        view.return_to = self.cur;
                        self.start_magit(move || {
                            let bytes = view.repo.commit_patch(&id)?;
                            view.kind = Kind::Patch(id);
                            view.rows = display_patch(&bytes);
                            Ok(Outcome::View(Box::new(view), None, 0))
                        });
                    }
                    _ => {}
                }
                return;
            }
            if action == Action::Toggle {
                match &selected {
                    Some(RowAction::Section(s)) => {
                        if !view.closed.remove(s) {
                            view.closed.insert(*s);
                        }
                        view.rebuild();
                        self.install_magit(view, selected, fallback);
                        return;
                    }
                    Some(RowAction::File(path, s))
                        if !matches!(s, Section::Conflicts | Section::Untracked) =>
                    {
                        let key = (path.clone(), *s == Section::Staged);
                        if view.expanded.remove(&key) {
                            view.rebuild();
                            self.install_magit(view, selected, fallback);
                            return;
                        }
                        self.start_magit(move || {
                            let diff = view.repo.diff(&key.0, key.1)?;
                            view.diffs.insert(key.clone(), diff);
                            view.expanded.insert(key);
                            view.rebuild();
                            Ok(Outcome::View(Box::new(view), selected, fallback))
                        });
                        return;
                    }
                    _ => {
                        self.ed
                            .set_msg("select a tracked file or section to expand");
                        return;
                    }
                }
            }
            self.start_magit(move || {
                let operation: Result<(),String> = (|| {
                if matches!(action,Action::Stage|Action::Unstage) {
                    match &selected {
                        Some(RowAction::File(path,section)) => {
                            if action == Action::Stage && *section != Section::Staged { view.repo.stage_file(path)?; }
                            else if action == Action::Unstage && *section == Section::Staged { view.repo.unstage_file(path)?; }
                            else { return Err("select the unstaged side to stage, or staged side to unstage".into()); }
                        }
                        Some(RowAction::Hunk(path,staged,hunk,_)) if *staged == (action == Action::Unstage) => {
                            let diff = view.diffs.get(&(path.clone(),*staged)).ok_or("refresh the diff first")?;
                            view.repo.apply_hunk(diff,*hunk)?;
                        }
                        _ => return Err("select a file or hunk for this operation".into()),
                    }
                }
                Ok(()) })();
                // magit-post-stage-hook / magit-post-unstage-hook.
                if operation.is_ok() {
                    let hook = if action == Action::Stage { "magit-post-stage-hook" } else { "magit-post-unstage-hook" };
                    crate::magit::options::run_hook(hook, &view.repo.root);
                }
                refresh_view(&mut view)?;
                match operation { Ok(()) => Ok(Outcome::View(Box::new(view),selected,fallback)), Err(e) => Ok(Outcome::ErrorView(Box::new(view),e,selected,fallback)) }
            });
            return;
        }
        let inherited = self.ed.magit.as_ref().and_then(|view| match &view.kind {
            Kind::FileLog(path, _) => Some(view.repo.root.join(path)),
            _ => None,
        });
        let file = if matches!(action, Action::Log | Action::LogHead) {
            inherited
        } else if action == Action::Status {
            // magit-status-goto-file-position: the visited file.
            self.ed
                .path
                .clone()
                .filter(|_| self.ed.magit.is_none() && !self.ed.generated())
        } else if action == Action::FileLog {
            let candidate = self
                .ed
                .magit
                .as_ref()
                .and_then(|view| match &view.kind {
                    Kind::FileLog(path, _) => Some(view.repo.root.join(path)),
                    _ => None,
                })
                .or_else(|| self.ed.blob.as_ref().map(|b| b.repo.root.join(&b.file)))
                .or_else(|| {
                    if self.ed.magit.is_none()
                        && self.ed.dired.is_none()
                        && self.ed.commit_repo.is_none()
                    {
                        self.ed.path.clone()
                    } else {
                        None
                    }
                });
            let candidate = candidate.filter(|p| !p.is_dir());
            if candidate.is_none() && action == Action::FileLog {
                self.ed.set_err("Buffer isn't visiting a file");
                return;
            }
            candidate
        } else {
            None
        };
        let follow = self
            .ed
            .magit_options
            .contains(&crate::magit::MenuOption::LogFollow);
        let commit_args = crate::magit::commit_arguments(&self.ed);
        // -C / -c belong to this one new commit; they don't stick.
        let reuse = |ed: &mut crate::editor::Editor, p: &'static str| {
            if action == Action::Commit {
                ed.magit_values.remove(&('C', p))
            } else {
                None
            }
        };
        let (reuse, reedit) = (
            reuse(&mut self.ed, "--reuse-message="),
            reuse(&mut self.ed, "--reedit-message="),
        );
        let log_args = crate::magit::menu_arguments(&self.ed, 'l');
        let stash_args = crate::magit::menu_arguments(&self.ed, 'z');
        let net_args = match action {
            Action::Net(op) => crate::magit::menu_arguments(&self.ed, op.menu()),
            _ => vec![],
        };
        let from = self.magit_from();
        let origin = self.cur;
        let prior: Vec<_> = (0..self.bufs.len())
            .filter_map(|i| {
                let ed = self.ed_at(i);
                ed.magit.as_deref().cloned().map(|view| (view, ed.cur.line))
            })
            .collect();
        self.start_magit(move || {
            let repo = Repo::discover(&from)?;
            match action {
                Action::Status => {
                    if let Some((mut view, line)) = prior
                        .into_iter()
                        .find(|(v, _)| v.repo == repo && v.kind == Kind::Status)
                    {
                        let selected = view.action_at(line);
                        refresh_view(&mut view)?;
                        Ok(Outcome::View(Box::new(view), selected, line))
                    } else {
                        let snapshot = repo.status()?;
                        let rel = file.as_deref().and_then(|p| repo_relative(&repo, p).ok());
                        let mut view = View::new_status(repo, snapshot);
                        view.return_to = origin;
                        let line = view.initial_line(rel.as_deref());
                        Ok(Outcome::View(Box::new(view), None, line))
                    }
                }
                Action::Stashes | Action::Tags => {
                    let mut view = View::status(repo.clone(), repo.status()?);
                    view.kind = if action == Action::Stashes {
                        Kind::Stashes
                    } else {
                        Kind::Tags
                    };
                    view.return_to = origin;
                    refresh_refs(&mut view)?;
                    Ok(Outcome::View(Box::new(view), None, 0))
                }
                Action::Log | Action::FileLog | Action::LogHead => {
                    let Some(file) = file else {
                        let op = if action == Action::Log {
                            crate::magit::log::Op::Current
                        } else {
                            crate::magit::log::Op::Head
                        };
                        let next = repo.log_step(op, &[], &log_args)?;
                        return Ok(branch_outcome(repo, next, origin));
                    };
                    let mut view = View::status(repo.clone(), repo.status()?);
                    let relative = repo_relative(&repo, &file)?;
                    view.kind = Kind::FileLog(relative, follow);
                    view.return_to = origin;
                    refresh_log(&mut view)?;
                    Ok(Outcome::View(Box::new(view), None, 0))
                }
                Action::Workflow(operation) => {
                    if operation.prompt().is_some() {
                        let args = if matches!(
                            operation,
                            crate::magit::workflows::Operation::Stash
                                | crate::magit::workflows::Operation::StashUntracked
                                | crate::magit::workflows::Operation::StashKeepIndex
                                | crate::magit::workflows::Operation::StashWorktree
                        ) {
                            stash_args
                        } else if operation == crate::magit::workflows::Operation::Fixup {
                            commit_args
                        } else {
                            vec![]
                        };
                        Ok(Outcome::Prompt(repo, operation, args))
                    } else {
                        if matches!(
                            operation,
                            crate::magit::workflows::Operation::SnapshotBoth
                                | crate::magit::workflows::Operation::SnapshotIndex
                                | crate::magit::workflows::Operation::SnapshotWorktree
                        ) {
                            let result = repo.save_stash(operation, "", &stash_args);
                            return Ok(Outcome::Saved(repo, result));
                        }
                        let mut inv = repo.operation(operation, "")?;
                        if operation == crate::magit::workflows::Operation::Amend {
                            inv.args.extend(commit_args.into_iter().map(OsString::from));
                            // magit-commit-extend-override-date nil: keep the date.
                            if !crate::magit::options::flag("magit-commit-extend-override-date", true) {
                                inv.env.extend(keep_committer_date(&repo));
                            }
                        }
                        Ok(Outcome::Git(inv))
                    }
                }
                Action::Commit => {
                    // --reuse-message commits at once; --reedit-message starts
                    // the draft from that message (Fred owns the message buffer).
                    let rest = commit_args.clone();
                    if let Some(r) = reuse {
                        let mut args: Vec<OsString> = vec!["commit".into()];
                        args.extend(rest.iter().map(OsString::from));
                        args.push(format!("--reuse-message={r}").into());
                        return Ok(Outcome::Git(GitInvocation {
                            expected_head: None,
                            repo,
                            args,
                            input: None,
                            draft: None,
                            draft_stamp: None,
                            editor: false,
                            env: vec![],
                            after: None,
                        }));
                    }
                    let message = match reedit {
                        Some(r) => repo
                            .read(&["log", "-1", "--format=%B", "--end-of-options", &r])
                            .map_err(|_| format!("unknown commit {r:?}"))?,
                        None => vec![],
                    };
                    // magit-commit-assert with magit-commit-ask-to-stage.
                    let mut rest = rest;
                    if !repo.commit_ready(&rest, false)? {
                        match crate::magit::commit::ask_to_stage() {
                            None => return Err("Nothing staged".into()),
                            Some(false) => rest.push("--all".into()),
                            Some(true) => {
                                return Ok(Outcome::Ask(
                                    repo,
                                    Question::Commit(crate::magit::commit::Op::DraftAll),
                                    vec![String::new()],
                                    vec!["Nothing staged.  Commit all uncommitted changes? (y or n) ".into()],
                                ));
                            }
                        }
                    }
                    Ok(Outcome::Draft(
                        repo,
                        crate::magit::CommitMode::New,
                        message,
                        rest,
                    ))
                }
                Action::AmendDraft | Action::RewordDraft => {
                    let head =
                        String::from_utf8_lossy(&repo.read(&["rev-parse", "--verify", "HEAD"])?)
                            .trim()
                            .to_owned();
                    let message = repo.read(&["log", "-1", "--format=%B", &head])?;
                    let mode = if action == Action::AmendDraft {
                        crate::magit::CommitMode::Amend(head)
                    } else {
                        crate::magit::CommitMode::Reword(head)
                    };
                    Ok(Outcome::Draft(repo, mode, message, commit_args))
                }
                Action::Branches => {
                    let names = repo.branch_choices();
                    Ok(Outcome::Branches(repo, names))
                }
                Action::Net(op) => {
                    let prompts = repo.network_prompts(op)?;
                    if prompts.is_empty() {
                        Ok(Outcome::Git(repo.network(op, &[], &net_args)?))
                    } else {
                        Ok(Outcome::Ask(repo, Question::Net(op), net_args, prompts))
                    }
                }
                _ => Err("unsupported Git action".into()),
            }
        });
    }
    pub fn tick_magit(&mut self) -> bool {
        if !self
            .magit_job
            .as_ref()
            .is_some_and(|j| j.work.is_finished())
        {
            return false;
        }
        let job = self.magit_job.take().expect("finished job");
        let result = job
            .work
            .join()
            .unwrap_or_else(|_| Err("Git worker failed".into()));
        self.refresh_gutters();
        // Saved mutations have already run: unlike stale read/draft requests,
        // their completion and errors must survive a buffer switch.
        if (self.cur != job.slot || self.clock != job.clock)
            && !matches!(result, Ok(Outcome::Saved(..) | Outcome::ConfiguredGit(_)))
        {
            return false;
        }
        match result {
            Err(e) => self.ed.set_err(e),
            Ok(Outcome::View(view, selected, fallback)) => {
                self.install_magit(*view, selected, fallback)
            }
            Ok(Outcome::NoticeView(view, message, selected, fallback)) => {
                self.install_magit(*view, selected, fallback);
                self.ed.set_msg(message);
            }
            Ok(Outcome::ErrorView(view, error, selected, fallback)) => {
                self.install_magit(*view, selected, fallback);
                self.ed.set_err(error);
            }
            Ok(Outcome::Prompt(repo, operation, args)) => {
                if self.ed.magit_input_generation != job.input_generation
                    || self.ed.mode != Mode::Normal
                {
                    self.ed.set_msg("Git prompt cancelled");
                    return true;
                }
                crate::magit::prompt(
                    &mut self.ed,
                    crate::magit::Prompt::Workflow(repo, operation, args),
                );
            }
            Ok(Outcome::Blame(blame)) => {
                if blame.version != self.ed.buf.version {
                    self.ed.set_msg("Blaming...aborted: buffer changed");
                    return true;
                }
                // magit-blame-read-only, except for echo.
                self.ed.readonly = blame.was_readonly || !blame.echo();
                self.ed.blame = Some(*blame);
                self.ed.set_msg("Blaming...done");
            }
            Ok(Outcome::Moved(repo, result, from, to)) => {
                if result.is_ok() {
                    // set-visited-file-name for buffers of the moved file.
                    for ed in self.editors_mut() {
                        // Buffers of the file, or inside a renamed directory.
                        if let Some(rest) = ed
                            .path
                            .as_deref()
                            .and_then(canonical_file)
                            .and_then(|p| p.strip_prefix(&from).ok().map(Path::to_path_buf))
                        {
                            ed.path = Some(if rest.as_os_str().is_empty() {
                                to.clone()
                            } else {
                                to.join(rest)
                            });
                        }
                    }
                }
                self.finish_git(
                    GitInvocation {
                        expected_head: None,
                        repo,
                        args: vec![],
                        input: None,
                        draft: None,
                        draft_stamp: None,
                        editor: false,
                        env: vec![],
                        after: None,
                    },
                    result,
                );
            }
            Ok(Outcome::VisitFile(path, line)) => self.open_pick(
                path,
                Some(Goto {
                    line,
                    col: 0,
                    pattern: None,
                }),
            ),
            Ok(Outcome::Blob(blob, bytes, line, message, then)) => {
                self.install_blob(blob, &bytes, line);
                if let Some(message) = message {
                    self.ed.set_msg(message);
                }
                if let Some(kind) = then {
                    self.magit_action(Action::Blame(kind));
                }
            }
            Ok(Outcome::Shell(cmd)) => {
                self.pending_shell = Some(cmd);
            }
            Ok(Outcome::Answer(repo, question, answers, defaults)) => {
                self.magit_action(Action::Answered(repo, question, answers, defaults));
            }
            Ok(Outcome::Copy(text, message)) => {
                crate::vim::ops::set_reg(
                    &mut self.ed,
                    crate::editor::Register {
                        text,
                        linewise: false,
                    },
                );
                self.ed.set_msg(message);
            }
            Ok(Outcome::Done(repo, message)) => {
                self.finish_git(
                    GitInvocation {
                        expected_head: None,
                        repo,
                        args: vec![],
                        input: None,
                        draft: None,
                        draft_stamp: None,
                        editor: false,
                        env: vec![],
                        after: None,
                    },
                    Ok(()),
                );
                self.ed.set_msg(message);
            }
            Ok(Outcome::Todo(plan)) => {
                self.open_pick(plan.todo.clone(), None);
                // Only the todo buffer itself gets the plan (a swap prompt may
                // have left another buffer current).
                let opened = self
                    .ed
                    .path
                    .as_deref()
                    .is_some_and(|p| swap::canonical(p) == swap::canonical(&plan.todo));
                if !opened {
                    self.ed.set_err("Could not open the rebase todo list");
                    return true;
                }
                self.ed.rebase_todo = Some(plan);
                self.ed.set_msg(
                    "Rebase todo: p r e s f d set action  x exec  M-j/M-k move  ZZ run  ZQ cancel",
                );
            }
            Ok(Outcome::InitConfirm(dir, question)) => {
                if self.ed.magit_input_generation != job.input_generation
                    || self.ed.mode != Mode::Normal
                {
                    self.ed.set_msg("Git prompt cancelled");
                    return true;
                }
                crate::magit::prompt(
                    &mut self.ed,
                    crate::magit::Prompt::InitConfirm(dir, question),
                );
            }
            Ok(Outcome::Ask(repo, question, args, prompts)) => {
                if self.ed.magit_input_generation != job.input_generation
                    || self.ed.mode != Mode::Normal
                {
                    self.ed.set_msg("Git prompt cancelled");
                    return true;
                }
                crate::magit::prompt(
                    &mut self.ed,
                    crate::magit::Prompt::Ask(repo, question, args, prompts, vec![]),
                );
            }
            Ok(Outcome::Draft(repo, mode, message, args)) => {
                if self.ed.magit_input_generation != job.input_generation
                    || self.ed.mode != Mode::Normal
                {
                    self.ed.set_msg("Git draft request cancelled");
                    return true;
                }
                let mut h = std::collections::hash_map::DefaultHasher::new();
                repo.root.hash(&mut h);
                let mut dir = self
                    .swap_dir
                    .with_file_name("magit")
                    .join(format!("{:016x}", h.finish()));
                if let Some(target) = mode.target() {
                    dir = dir
                        .join(if matches!(mode, crate::magit::CommitMode::Amend(_)) {
                            "amend"
                        } else {
                            "reword"
                        })
                        .join(target);
                }
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    self.ed.set_err(e.to_string());
                    return true;
                }
                let path = dir.join("COMMIT_EDITMSG");
                // A merge/squash message seeds a new draft only when the draft is
                // blank on disk and not open with unsaved text: the user's words win.
                // An open draft buffer is never written underneath (compare canonical
                // paths, as buffer paths may be relative).
                let canonical = swap::canonical(&path);
                let open = self.editors_mut().any(|ed| {
                    ed.path
                        .as_deref()
                        .is_some_and(|p| swap::canonical(p) == canonical)
                });
                let blank_draft = !open
                    && std::fs::read(&path).map_or(true, |b| b.iter().all(u8::is_ascii_whitespace));
                let seed = match mode.target() {
                    Some(_) => !path.exists(),
                    None => !message.is_empty() && blank_draft,
                };
                if seed && let Err(error) = fileio::write(&path, &message, None, false) {
                    self.ed.set_err(error);
                    return true;
                }
                self.magit_drafts
                    .insert(path.clone(), (repo.clone(), mode.clone(), args));
                self.open_pick(path, None);
                self.attach_commit_repo();
                if let Some((file, defun)) = self.pending_add_log.take() {
                    let (text, line) = crate::magit::message::add_log_insert(
                        &self.ed.buf.text(),
                        &file,
                        defun.as_deref(),
                    );
                    crate::magit::message::replace_text(&mut self.ed, &text);
                    let len = self.ed.buf.line(line).len();
                    self.ed.set_cursor(line, len);
                    return true;
                }
                self.ed.set_msg(match mode {
                    crate::magit::CommitMode::New => {
                        "Commit draft: :w save; Space m c c commit staged changes"
                    }
                    crate::magit::CommitMode::Amend(_) => {
                        "Amend draft: :w save; Space m c c amend staged changes and message"
                    }
                    crate::magit::CommitMode::Reword(_) => {
                        "Reword draft: :w save; Space m c c replace message, keep HEAD tree"
                    }
                });
            }
            Ok(Outcome::Branches(repo, names)) => {
                self.magit_picker_repo = Some(repo);
                crate::pick::branches(&mut self.ed, names);
            }
            Ok(Outcome::Git(inv) | Outcome::ConfiguredGit(inv)) => self.pending_git = Some(inv),
            Ok(Outcome::Saved(repo, result)) => self.finish_git(
                GitInvocation {
                    expected_head: None,
                    repo,
                    args: vec![],
                    input: None,
                    draft: None,
                    draft_stamp: None,
                    editor: false,
                    env: vec![],
                    after: None,
                },
                result,
            ),
        }
        true
    }
    /// global-git-commit-mode: a message file Git asked Fred to edit
    /// (git-commit-filename-regexp) gets the commit draft keys; finishing it
    /// (Space m c c) saves and quits, as with-editor-finish.
    pub(super) fn attach_git_commit_mode(&mut self) {
        const NAMES: [&str; 9] = [
            "COMMIT_EDITMSG",
            "MERGE_MSG",
            "TAG_EDITMSG",
            "PULLREQ_EDITMSG",
            "MERGEREQ_EDITMSG",
            "SQUASH_MSG",
            "NOTES_EDITMSG",
            "EDIT_DESCRIPTION",
            "BRANCH_DESCRIPTION",
        ];
        let Some(path) = self.ed.path.clone() else {
            return;
        };
        let named = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| NAMES.contains(&n));
        // Fred's own drafts (also after a restart) live in its state directory.
        let own = path.starts_with(self.swap_dir.with_file_name("magit"))
            || self.magit_drafts.contains_key(&path);
        if !named
            || own
            || self.ed.commit_repo.is_some()
            || !crate::magit::options::flag("global-git-commit-mode", true)
        {
            return;
        }
        // Git runs the editor in the worktree; the file is in the git dir.
        let worktree = path
            .ancestors()
            .find(|d| d.file_name().is_some_and(|n| n == ".git"))
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok());
        if let Some(repo) = worktree.and_then(|d| Repo::discover(&d).ok()) {
            self.ed.commit_repo = Some(repo);
            self.ed.commit_mode = crate::magit::CommitMode::New;
            self.ed.commit_args = vec![];
        }
    }
    pub(super) fn attach_commit_repo(&mut self) {
        if let Some((repo, mode, args)) =
            self.ed.path.as_ref().and_then(|p| self.magit_drafts.get(p))
        {
            self.ed.commit_repo = Some(repo.clone());
            self.ed.commit_mode = mode.clone();
            self.ed.commit_args = args.clone();
            crate::magit::sync_commit_options(&mut self.ed);
        }
    }
    /// Worker closures for blob-mode commands, resolved against this buffer.
    #[allow(clippy::type_complexity)]
    fn blob_action(
        &mut self,
        action: &Action,
    ) -> Option<Box<dyn FnOnce(usize) -> Result<Outcome, String> + Send>> {
        use crate::magit::blob::WORKTREE;
        let from = self.magit_from();
        let current = self.ed.blob.clone();
        let path = self.ed.path.clone().filter(|_| {
            current.is_none()
                && self.ed.magit.is_none()
                && self.ed.dired.is_none()
                && self.ed.commit_repo.is_none()
        });
        // The visited blob, or the worktree file of a file buffer.
        let here = move || -> Result<(Repo, String, PathBuf), String> {
            match (&current, &path) {
                (Some(b), _) => Ok((b.repo.clone(), b.rev.clone(), b.file.clone())),
                (None, Some(p)) => {
                    let repo = Repo::discover(p)?;
                    let file = repo_relative(&repo, p)?;
                    Ok((repo, WORKTREE.into(), file))
                }
                _ => Err("Buffer isn't visiting a file or blob".into()),
            }
        };
        Some(match action.clone() {
            Action::FindFile => Box::new(move |_| {
                let (repo, rev, file) = match here() {
                    Ok((repo, rev, file)) => (repo, rev, file.to_string_lossy().into_owned()),
                    Err(_) => (Repo::discover(&from)?, String::new(), String::new()),
                };
                let rev = if rev.is_empty() || rev == WORKTREE {
                    "HEAD".into()
                } else {
                    rev
                };
                Ok(Outcome::Ask(
                    repo,
                    Question::FindFile,
                    vec![rev.clone(), file.clone()],
                    vec![
                        format!("Find file from revision (default {rev}): "),
                        format!("Find file (default {file}): "),
                    ],
                ))
            }),
            Action::BlobVisit(rev, file) => Box::new(move |line| {
                let repo = here().map(|h| h.0).or_else(|_| Repo::discover(&from))?;
                blob_outcome(repo, rev, file, line, None, None)
            }),
            // magit-blame-visit-other-file goes to the chunk's orig-line.
            Action::BlobVisitBlame(rev, file, kind, line) => Box::new(move |_| {
                let repo = here().map(|h| h.0).or_else(|_| Repo::discover(&from))?;
                let rev = repo.blob_rev(&rev)?;
                blob_outcome(repo, rev, file, line, None, Some(kind))
            }),
            Action::BlobPrevious | Action::BlobNext => {
                let previous = *action == Action::BlobPrevious;
                Box::new(move |line| {
                    let (repo, rev, file) = here()?;
                    let next = if previous {
                        repo.blob_ancestor(&rev, &file)
                            .ok_or("You have reached the beginning of time")?
                    } else {
                        repo.blob_successor(&rev, &file)
                            .ok_or("You have reached the end of time")?
                    };
                    // magit-blob-visit: report the commit being shown.
                    let message = (!next.0.starts_with('{'))
                        .then(|| {
                            repo.read(&["log", "-1", "--format=%s (%cr)", &next.0, "--"])
                                .ok()
                                .map(|o| label(Path::new(String::from_utf8_lossy(&o).trim())))
                        })
                        .flatten();
                    blob_outcome(repo, next.0, next.1, line, message, None)
                })
            }
            Action::BlobVisitFile => {
                let blob = self.ed.blob.clone();
                Box::new(move |line| match blob {
                    Some(b) => Ok(Outcome::VisitFile(b.repo.root.join(b.file), line)),
                    None => Err("Not visiting a blob".into()),
                })
            }
            _ => return None,
        })
    }
    /// The visited file (not a blob) for file-dispatch commands.
    /// Apply or reverse the hunk (or file) at LINE of this diff, commit or
    /// stash buffer, from Git's raw output (display text is escaped).
    fn apply_diff(&mut self, how: PatchUse, line: usize) {
        let Some(view) = self.ed.magit.as_deref().cloned() else {
            return;
        };
        let origin = self.cur;
        self.start_magit(move || {
            let repo = view.repo.clone();
            let patch = diff_patch_at(&view, line)?;
            let mut argv: Vec<OsString> = vec!["apply".into(), "--whitespace=nowarn".into()];
            match how {
                PatchUse::Reverse => argv.push("--reverse".into()),
                PatchUse::ReverseIndex => argv.extend(["--reverse".into(), "--cached".into()]),
                _ => {}
            }
            // magit-reverse-atomically nil: --reject reverses what applies.
            let partial = how == PatchUse::Reverse
                && !crate::magit::options::flag("magit-reverse-atomically", false);
            let r = if partial {
                argv.push("--reject".into());
                repo.run(&argv, Some(&patch))
            } else {
                let mut check = argv.clone();
                check.push("--check".into());
                repo.run(&check, Some(&patch))
                    .and_then(|_| repo.run(&argv, Some(&patch)))
            };
            let next = crate::magit::branch::Next::Done(r.map(|_| {
                match how {
                    PatchUse::Reverse => "Reversed in the worktree",
                    PatchUse::ReverseIndex => "Reversed in the index",
                    _ => "Applied to the worktree",
                }
                .into()
            }));
            Ok(branch_outcome(repo, next, origin))
        });
    }
    /// magit-auto-revert-mode: reload unmodified buffers of files under ROOT
    /// that changed on disk (auto-revert-buffer, without its timer).
    fn auto_revert_buffers(&mut self, root: &Path) {
        let origin = self.cur;
        let stale: Vec<usize> = (0..self.bufs.len())
            .filter(|&i| {
                let (ed, stamp) = if i == self.cur {
                    (&self.ed, self.stamp.as_ref())
                } else {
                    match &self.bufs[i] {
                        Some(p) => (&p.ed, p.stamp.as_ref()),
                        None => return false,
                    }
                };
                !ed.buf.modified
                    && ed.magit.is_none()
                    && ed.commit_repo.is_none()
                    && !ed.generated()
                    && ed.path.as_ref().is_some_and(|p| {
                        swap::canonical(p).starts_with(root)
                            && p.is_file()
                            && fileio::changed_on_disk(p, stamp)
                            // magit-auto-revert-tracked-only.
                            && (!crate::magit::options::flag("magit-auto-revert-tracked-only", true)
                                || Repo { root: root.to_path_buf() }
                                    .run(
                                        &[
                                            "ls-files".into(),
                                            "--error-unmatch".into(),
                                            "--".into(),
                                            crate::magit::repo::literal_pathspec(
                                                &swap::canonical(p)
                                                    .strip_prefix(root)
                                                    .map(Path::to_path_buf)
                                                    .unwrap_or_default(),
                                            ),
                                        ],
                                        None,
                                    )
                                    .is_ok())
                    })
            })
            .collect();
        for i in stale {
            self.show(i);
            self.edit(None, false);
        }
        if self.cur != origin {
            self.show(origin);
        }
    }
    /// magit-save-repository-buffers t: ask about each buffer, then run ACTION.
    fn ask_save(&mut self, mut rest: Vec<PathBuf>, action: Box<Action>) {
        if rest.is_empty() {
            return self.magit_action(Action::Saved(action));
        }
        let path = rest.remove(0);
        crate::magit::prompt(
            &mut self.ed,
            crate::magit::Prompt::SaveBuffer(path, rest, action),
        );
    }
    /// browse-url: the system's opener, detached.
    fn browse(&mut self, url: &str) {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        match std::process::Command::new(opener)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => self.ed.set_msg(format!("Opened {url}")),
            Err(e) => self.ed.set_err(format!("{opener}: {e}")),
        }
    }
    /// magit-wip-after-save-local-mode: the saved file's worktree wip ref.
    pub(super) fn wip_after_save(&mut self, path: &Path) {
        // ponytail: synchronous; a few local plumbing calls per save.
        let result = Repo::discover(path).and_then(|r| {
            let file = repo_relative(&r, path)?;
            // Only tracked files have wip state.
            r.read(&["ls-files", "--error-unmatch", "--", &file.to_string_lossy()])?;
            r.wip_commit_file(&file)
        });
        if let Err(e) = result
            && !e.contains("did not match")
            && !e.contains("not a git repository")
        {
            self.ed.set_err(format!("magit-wip: {e}"));
        }
    }
    /// Finish the commit draft (or the message Git is waiting for).
    fn commit_draft(&mut self) {
        if self
            .ed
            .path
            .as_ref()
            .is_some_and(|p| !self.magit_drafts.contains_key(p))
            && !self
                .ed
                .path
                .as_ref()
                .is_some_and(|p| p.starts_with(self.swap_dir.with_file_name("magit")))
        {
            return self.perform(crate::ex::ExEffect::Write {
                path: None,
                force: false,
                range: None,
                then_quit: true,
            });
        }
        if let Some(repo) = self.ed.commit_repo.clone() {
            if self.ed.readonly {
                self.ed.set_err("commit draft is read-only");
                return;
            }
            if self
                .ed
                .path
                .as_ref()
                .is_some_and(|p| fileio::changed_on_disk(p, self.stamp.as_ref()))
            {
                self.ed
                    .set_err("commit draft changed on disk; reload before submitting");
                return;
            }
            if let Some(target) = self.ed.commit_mode.target() {
                match repo.read(&["rev-parse", "--verify", "HEAD"]) {
                    Ok(head) if String::from_utf8_lossy(&head).trim() == target => (),
                    _ => {
                        self.ed
                            .set_err("HEAD changed; reopen amend/reword for the current commit");
                        return;
                    }
                }
            }
            let message = self.ed.buf.to_bytes();
            let draft = self.ed.path.clone().unwrap_or_default();
            match repo.commit_invocation(message, draft) {
                Ok(mut inv) => {
                    inv.args
                        .extend(self.ed.commit_args.iter().map(OsString::from));
                    if let Some(target) = self.ed.commit_mode.target() {
                        inv.args.push("--amend".into());
                        inv.expected_head = Some(target.into());
                    }
                    if matches!(self.ed.commit_mode, crate::magit::CommitMode::Reword(_)) {
                        inv.args.push("--only".into());
                        inv.args.push("--allow-empty".into());
                        // magit-commit-reword-override-date nil: keep the date.
                        if !crate::magit::options::flag("magit-commit-reword-override-date", true) {
                            inv.env.extend(keep_committer_date(&repo));
                        }
                    }
                    self.pending_git = Some(inv);
                }
                Err(e) => self.ed.set_err(e),
            }
        }
    }
    /// magit-repolist-fetch (git remote update in each, in the terminal) or
    /// -find-file-other-frame (open the file in each repository).
    fn repolist_act(&mut self, fetch: bool, repos: Vec<PathBuf>) {
        if fetch {
            let q = crate::magit::misc::shell_quote;
            let n = repos.len();
            let cmd = repos
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let dir = r.to_string_lossy();
                    format!(
                        "echo {}; (cd {} && git remote update)",
                        q(&format!("({}/{n}) Fetching in {dir}...", i + 1)),
                        q(&dir)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            self.pending_shell = Some(cmd);
            return;
        }
        self.repolist_files = Some(repos);
        crate::magit::prompt(
            &mut self.ed,
            crate::magit::Prompt::Ask(
                Repo {
                    root: PathBuf::new(),
                },
                Question::Misc(crate::magit::misc::Op::RepolistFile),
                vec![],
                vec!["Find file in repositories: ".into()],
                vec![],
            ),
        );
    }
    /// magit-log-move-to-revision: in this log buffer, else the log of all
    /// branches.
    fn log_jump(&mut self, repo: Repo, answers: Vec<String>) {
        let rev = answers.first().cloned().unwrap_or_default();
        let in_log = self
            .ed
            .magit
            .as_deref()
            .filter(|v| v.repo == repo && matches!(v.kind, Kind::Log(..)))
            .cloned();
        let origin = self.cur;
        let args = crate::magit::menu_arguments(&self.ed, 'l');
        self.start_magit(move || {
            if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
                return Err(format!("invalid revision {rev:?}"));
            }
            let id = repo
                .read(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    "--end-of-options",
                    &format!("{rev}^{{commit}}"),
                ])
                .map_err(|_| format!("unknown revision {rev:?}"))?;
            let id = String::from_utf8_lossy(&id).trim().to_owned();
            let view = match in_log {
                Some(v) => v,
                None => {
                    let mut v = View::status(repo.clone(), Default::default());
                    v.kind = Kind::Log(vec!["--branches".into()], args);
                    v.rows.clear();
                    v.return_to = origin;
                    refresh_view(&mut v)?;
                    v
                }
            };
            if !view
                .rows
                .iter()
                .any(|r| r.action == Some(RowAction::Commit(id.clone())))
            {
                return Err(format!("{rev} isn't visible in the current log buffer"));
            }
            Ok(Outcome::View(
                Box::new(view),
                Some(RowAction::Commit(id)),
                0,
            ))
        });
    }
    /// magit-next-reference: the next line naming a ref (a log's
    /// decorations, a refs buffer's refs, status headers).
    fn next_reference(&mut self, previous: bool) {
        let Some(view) = self.ed.magit.as_deref() else {
            return;
        };
        let is_ref = |i: usize| {
            let text = view.rows.get(i).map_or("", |r| r.text.as_str());
            match &view.kind {
                Kind::Refs(..) => matches!(view.rows[i].action, Some(RowAction::Commit(_))),
                Kind::Status => ["Head:", "Merge:", "Rebase:", "Push:", "Tag:"]
                    .iter()
                    .any(|p| text.starts_with(p)),
                _ => {
                    // "hash (refs) subject", after any graph.
                    let t = text.trim_start_matches(|c: char| "*|/\\ _-.".contains(c));
                    t.split_once(' ').is_some_and(|(h, rest)| {
                        h.len() >= 7
                            && h.bytes().all(|b| b.is_ascii_hexdigit())
                            && rest.starts_with('(')
                    })
                }
            }
        };
        let cur = self.ed.cur.line;
        let found = if previous {
            (0..cur).rev().find(|&i| is_ref(i))
        } else {
            (cur + 1..view.rows.len()).find(|&i| is_ref(i))
        };
        match found {
            Some(i) => self.ed.set_cursor(i, 0),
            None => self.ed.set_msg("No more references"),
        }
    }
    /// magit-section-cycle-diffs: in status, show every staged and unstaged
    /// file's diff, or hide them all when they are all shown.
    fn cycle_diffs(&mut self) {
        let Some(mut view) = self.ed.magit.as_deref().cloned() else {
            return;
        };
        if view.kind != Kind::Status {
            return self.ed.set_msg("No diff sections to cycle here");
        }
        let files: Vec<(PathBuf, bool)> = view
            .snapshot
            .entries
            .iter()
            .flat_map(|e| {
                let mut v = vec![];
                if e.staged && !e.conflict {
                    v.push((e.path.clone(), true));
                }
                if e.unstaged && !e.conflict && !e.untracked {
                    v.push((e.path.clone(), false));
                }
                v
            })
            .collect();
        let selected = view.action_at(self.ed.cur.line);
        let fallback = self.ed.cur.line;
        view.closed.remove(&Section::Staged);
        view.closed.remove(&Section::Unstaged);
        if files.iter().all(|f| view.expanded.contains(f)) {
            view.expanded.clear();
            view.rebuild();
            return self.install_magit(view, selected, fallback);
        }
        self.start_magit(move || {
            for key in files {
                let diff = view.repo.diff(&key.0, key.1)?;
                view.diffs.insert(key.clone(), diff);
                view.expanded.insert(key);
            }
            view.rebuild();
            Ok(Outcome::View(Box::new(view), selected, fallback))
        });
    }
    /// The file and function (from the hunk header) of the change at point.
    fn hunk_defun(&self) -> Option<(String, Option<String>)> {
        let view = self.ed.magit.as_deref()?;
        let line = self.ed.cur.line;
        match view.action_at(line) {
            Some(RowAction::Hunk(path, staged, hunk, _)) => {
                let diff = view.diffs.get(&(path.clone(), staged))?;
                let h = diff.hunks.get(hunk)?;
                let text = String::from_utf8_lossy(&diff.bytes[h.start..h.end]);
                let ctx = crate::magit::message::hunk_defun(text.lines());
                return Some((path.to_string_lossy().into_owned(), ctx));
            }
            Some(RowAction::File(path, _)) => {
                return Some((path.to_string_lossy().into_owned(), None));
            }
            _ => {}
        }
        // A diff buffer's raw lines: the nearest hunk header and file above.
        let rows = &view.rows;
        let mut defun = None;
        for i in (0..=line.min(rows.len().checked_sub(1)?)).rev() {
            let t = rows[i].text.as_str();
            if defun.is_none() && t.starts_with("@@") {
                defun = Some(crate::magit::message::hunk_defun(
                    rows[i..]
                        .iter()
                        .map(|r| r.text.as_str())
                        .take_while(|l| !l.starts_with("diff ")),
                ));
            }
            if let Some(f) = t.strip_prefix("+++ b/") {
                return Some((f.to_owned(), defun.flatten()));
            }
            if let Some(rest) = t.strip_prefix("diff --git a/") {
                let f = rest.rsplit_once(" b/").map_or(rest, |(_, b)| b);
                return Some((f.to_owned(), defun.flatten()));
            }
        }
        None
    }
    /// magit-add-change-log-entry: a dated ChangeLog item for the change at
    /// point, in the nearest ChangeLog (add-log's find-change-log).
    fn add_change_log_entry(&mut self, file: String, defun: Option<String>) {
        let Some(view) = self.ed.magit.as_deref() else {
            return;
        };
        let repo = view.repo.clone();
        let dir = repo.root.join(&file);
        let log = dir
            .ancestors()
            .skip(1)
            .take_while(|d| d.starts_with(&repo.root))
            .map(|d| d.join("ChangeLog"))
            .find(|p| p.is_file())
            .unwrap_or_else(|| repo.root.join("ChangeLog"));
        let rel = log
            .parent()
            .and_then(|d| {
                repo.root
                    .join(&file)
                    .strip_prefix(d)
                    .ok()
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(|| PathBuf::from(&file));
        let ident = crate::magit::message::ident(&repo);
        let (name, email) = ident
            .rsplit_once(" <")
            .map(|(n, e)| (n.to_owned(), e.trim_end_matches('>').to_owned()))
            .unwrap_or((ident.clone(), String::new()));
        let today = crate::magit::message::today();
        let old = std::fs::read_to_string(&log).unwrap_or_default();
        let text = crate::magit::message::change_log_add(
            &old,
            &format!("{today}  {name}  <{email}>"),
            &rel.to_string_lossy(),
            defun.as_deref(),
        );
        if let Err(e) = crate::fileio::write(&log, text.as_bytes(), None, false) {
            return self.ed.set_err(e);
        }
        self.open_pick(log, None);
        if let Some(i) = self
            .ed
            .buf
            .text()
            .lines()
            .position(|l| l.starts_with('\t') && l.ends_with(": "))
        {
            let len = self.ed.buf.line(i).len();
            self.ed.set_cursor(i, len);
        }
    }
    /// magit-copy-diff-as-kill: the hunk or file diff at point, else the
    /// commit at point (or the buffer's revision) as a patch.
    fn copy_diff(&mut self) {
        let Some(view) = self.ed.magit.as_deref().cloned() else {
            return self.ed.set_err("Cannot copy this as a diff");
        };
        let line = self.ed.cur.line;
        let revision = crate::magit::buffer_revision(&self.ed)
            .filter(|_| matches!(view.kind, Kind::Patch(_) | Kind::Diff(Target::Commit(_), _)));
        self.start_magit(move || {
            let repo = view.repo.clone();
            let text = match view.action_at(line) {
                Some(RowAction::Hunk(path, staged, hunk, _)) => {
                    let diff = view
                        .diffs
                        .get(&(path, staged))
                        .ok_or("refresh the diff first")?;
                    let h = diff.hunks.get(hunk).ok_or("Cannot copy this as a diff")?;
                    let header = diff.hunks.first().map_or(0, |h| h.start);
                    let mut patch = diff.bytes[..header].to_vec();
                    patch.extend_from_slice(&diff.bytes[h.start..h.end]);
                    patch
                }
                Some(RowAction::File(_, Section::Untracked | Section::Conflicts)) => {
                    return Err("Cannot copy this as a diff".into());
                }
                Some(RowAction::File(path, section)) => {
                    repo.diff(&path, section == Section::Staged)?.bytes
                }
                Some(RowAction::Commit(id)) => repo.read(&["show", "-p", "--format=", &id])?,
                _ => match diff_patch_at(&view, line) {
                    Ok(patch) => patch,
                    Err(_) => match &revision {
                        Some(rev) => repo.read(&["show", "-p", "--format=", rev])?,
                        None => return Err("Cannot copy this as a diff".into()),
                    },
                },
            };
            Ok(Outcome::Copy(
                String::from_utf8_lossy(&text).into_owned(),
                "Copied diff".into(),
            ))
        });
    }
    fn file_action(&mut self, op: crate::magit::blob::FileOp) {
        use crate::magit::blob::FileOp as O;
        use crate::magit::misc::Op as M;
        // magit-file-dispatch offers magit-stage-files / -unstage-files when
        // no file is visited.
        let visiting = self.ed.blob.is_some()
            || self.ed.dired.is_some()
            || (self.ed.path.is_some() && !self.ed.generated());
        if !visiting && matches!(op, O::Stage | O::Unstage) {
            let files = if op == O::Stage {
                M::StageFiles(false)
            } else {
                M::UnstageFiles
            };
            return self.magit_action(Action::Misc(files));
        }
        // magit-dired-stage / -unstage: the marked files or the one at point.
        if self.ed.dired.is_some() && matches!(op, O::Stage | O::Unstage) {
            let files = crate::dired::selection(&self.ed);
            let (origin, from) = (self.cur, self.magit_from());
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                for f in &files {
                    let rel = repo_relative(&repo, f)?;
                    if op == O::Stage {
                        repo.stage_file(&rel)?;
                    } else {
                        repo.unstage_file(&rel)?;
                    }
                }
                let verb = if op == O::Stage { "Staged" } else { "Unstaged" };
                Ok(branch_outcome(
                    repo,
                    crate::magit::branch::Next::Done(Ok(format!("{verb} {} files", files.len()))),
                    origin,
                ))
            });
            return;
        }
        let from = self.magit_from();
        let blob = self.ed.blob.clone();
        let path = self
            .ed
            .path
            .clone()
            .filter(|_| !self.ed.generated() && self.ed.dired.is_none());
        let rev = blob.as_ref().map_or("HEAD".into(), |b| b.rev.clone());
        self.start_magit(move || {
            let repo = Repo::discover(&from)?;
            let current = match (&blob, &path) {
                (Some(b), _) => Some(b.file.clone()),
                (None, Some(p)) => repo_relative(&repo, p).ok(),
                _ => None,
            };
            let name = current
                .as_deref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ask = |prompts: Vec<String>, args: Vec<String>| {
                Ok(Outcome::Ask(
                    repo.clone(),
                    Question::File(op),
                    args,
                    prompts,
                ))
            };
            match op {
                O::Stage | O::Unstage => {
                    let (Some(file), Some(_)) = (current.clone(), path) else {
                        return Err("Not visiting a file".into());
                    };
                    if op == O::Stage && repo.ignored(&file) {
                        return Ok(Outcome::Ask(
                            repo,
                            Question::File(O::StageIgnored),
                            vec![name],
                            vec!["Visited file is ignored; stage anyway? (y or n) ".into()],
                        ));
                    }
                    let result = repo.file_op(op, &[file], "");
                    Ok(Outcome::Saved(repo, result))
                }
                O::Untrack => ask(vec![format!("Untrack file (default {name}): ")], vec![name]),
                O::Delete => ask(vec![format!("Delete file (default {name}): ")], vec![name]),
                O::Rename => ask(
                    vec![
                        format!("Rename file (default {name}): "),
                        "Move to destination: ".into(),
                    ],
                    vec![name, String::new()],
                ),
                O::Checkout => {
                    let rev = if rev.starts_with('{') {
                        "HEAD".into()
                    } else {
                        rev
                    };
                    ask(
                        vec![
                            format!("Checkout from revision (default {rev}): "),
                            format!("Checkout file (default {name}): "),
                        ],
                        vec![rev, name],
                    )
                }
                O::StageIgnored | O::DeleteDir => Err("confirmation without a question".into()),
            }
        });
    }
    fn file_answered(
        &mut self,
        repo: Repo,
        op: crate::magit::blob::FileOp,
        answers: Vec<String>,
        args: Vec<String>,
    ) {
        use crate::magit::blob::{FileOp as O, relative};
        let pick = |i: usize| {
            answers
                .get(i)
                .filter(|a| !a.is_empty())
                .or(args.get(i))
                .cloned()
                .unwrap_or_default()
        };
        let confirmed = |a: &str| matches!(a.trim(), "y" | "yes");
        let (file, rev) = match op {
            O::StageIgnored | O::DeleteDir => {
                let yes = answers.first().is_some_and(|a| confirmed(a))
                    && (op == O::StageIgnored || answers[0].trim() == "yes");
                if !yes {
                    return self.ed.set_msg("Abort");
                }
                (args.first().cloned().unwrap_or_default(), String::new())
            }
            O::Checkout => (pick(1), pick(0)),
            _ => (pick(0), String::new()),
        };
        let file = match relative(&file) {
            Ok(f) => f,
            Err(e) => return self.ed.set_err(e),
        };
        let root = repo.root.clone();
        let checked = |rel: &Path, must_exist| crate::magit::blob::exact(&root, rel, must_exist);
        let mut paths = vec![file.clone()];
        if matches!(op, O::Rename | O::Delete | O::DeleteDir)
            && let Err(e) = checked(&file, true)
        {
            return self.ed.set_err(e);
        }
        if op == O::Rename {
            // magit-file-rename: a directory destination keeps the file name.
            let raw = pick(1);
            // An existing destination must be an exact, real directory; the file
            // then keeps its name inside it, which must not exist yet.
            let dest = match raw.trim_end_matches('/') {
                "" | "." => Ok(PathBuf::new()),
                d => relative(d),
            }
            .and_then(|d| {
                let existing = d.as_os_str().is_empty() || root.join(&d).symlink_metadata().is_ok();
                if !existing {
                    if raw.ends_with('/') {
                        return Err("Destination directory does not exist".into());
                    }
                    return checked(&d, false);
                }
                let dir = if d.as_os_str().is_empty() {
                    root.clone()
                } else {
                    checked(&d, true)?
                };
                if !dir.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                    return Err(format!("{} already exists", label(&d)));
                }
                checked(&d.join(file.file_name().unwrap_or_default()), false)
            });
            match dest {
                Ok(d) => paths.push(d.strip_prefix(&root).unwrap_or(&d).to_path_buf()),
                Err(e) => return self.ed.set_err(e),
            }
        }
        // Never overwrite or orphan unsaved source buffers.
        let abs = repo.root.join(&file);
        if matches!(op, O::Rename | O::Delete | O::DeleteDir | O::Checkout)
            && self.editors_mut().any(|ed| {
                ed.buf.modified && ed.path.as_deref().is_some_and(|p| same_or_inside(p, &abs))
            })
        {
            return self
                .ed
                .set_err(format!("Save {} before changing it", label(&file)));
        }
        // Fred has no trash: untracked files and directories need a typed "yes",
        // and ignored files (outside upstream's completion list) are refused.
        if op == O::Delete && (abs.is_dir() || !repo.tracked(&file)) {
            if repo.ignored(&file) {
                return self
                    .ed
                    .set_err(format!("{} is ignored; not deleting", label(&file)));
            }
            let question = if abs.is_dir() {
                format!(
                    "Recursively delete directory {}? (yes or no) ",
                    label(&file)
                )
            } else {
                format!(
                    "Delete untracked {} permanently? (yes or no) ",
                    label(&file)
                )
            };
            return crate::magit::prompt(
                &mut self.ed,
                crate::magit::Prompt::Ask(
                    repo,
                    Question::File(O::DeleteDir),
                    vec![file.to_string_lossy().into_owned()],
                    vec![question],
                    vec![],
                ),
            );
        }
        let op = if op == O::DeleteDir { O::Delete } else { op };
        self.start_magit(move || {
            let result = repo.file_op(op, &paths, &rev);
            if op == O::Rename {
                let (from, to) = (repo.root.join(&paths[0]), repo.root.join(&paths[1]));
                return Ok(Outcome::Moved(repo, result, from, to));
            }
            Ok(Outcome::Saved(repo, result))
        });
    }
    fn install_blob(&mut self, blob: crate::magit::blob::Blob, bytes: &[u8], line: usize) {
        let text = String::from_utf8_lossy(bytes);
        if let Some(i) = (0..self.bufs.len()).find(|i| self.ed_at(*i).blob.as_ref() == Some(&blob))
        {
            self.show(i);
        } else {
            // Synthetic identity; the real file name keeps syntax highlighting.
            let mut h = std::collections::hash_map::DefaultHasher::new();
            blob.repo.root.hash(&mut h);
            blob.rev.hash(&mut h);
            blob.file.hash(&mut h);
            let path = self
                .swap_dir
                .with_file_name("magit-blobs")
                .join(format!("{:016x}", h.finish()))
                .join(blob.file.file_name().unwrap_or_default());
            let mut ed = make_editor(Buffer::default(), &self.cfg);
            ed.path = Some(path);
            let swap = swap::swap_path_in(&self.swap_dir, ed.path.as_deref());
            self.switch_to(
                Opened {
                    ed,
                    stamp: None,
                    lossy: false,
                },
                swap,
            );
        }
        self.no_swap = true;
        self.ed.git = crate::git::Gutter::default();
        self.ed.buf = Buffer::from_text(&text);
        self.ed.undo = crate::undo::Undo::default();
        self.ed.saved_state = 0;
        self.ed.readonly = true;
        self.ed.blame = None;
        self.ed.blob = Some(blob);
        let last = self.ed.line_count().saturating_sub(1);
        self.ed.set_cursor(line.min(last), 0);
        self.reloaded = true;
    }
    fn install_magit(&mut self, mut view: View, selected: Option<RowAction>, fallback: usize) {
        // Like magit-diff-mode, one diff buffer per repository is refreshed in
        // place, unless locked to its value (magit-toggle-buffer-lock).
        let slot = |v: &View| match &v.kind {
            _ if v.select.is_some() => "Select".to_owned(),
            Kind::Diff(..) if !v.locked => "Diff".to_owned(),
            kind => format!("{kind:?}"),
        };
        let kind = slot(&view);
        let repo = view.repo.clone();
        let same = |ed: &Editor| {
            ed.magit
                .as_ref()
                .is_some_and(|v| v.repo == repo && slot(v) == kind)
        };
        if !same(&self.ed) {
            if let Some(i) = self
                .bufs
                .iter()
                .position(|p| p.as_ref().is_some_and(|p| same(&p.ed)))
            {
                let origin = self.cur;
                self.show(i);
                if self.ed.magit.is_some() {
                    view.return_to = origin;
                }
            } else {
                // Synthetic state-directory identity, not a repository source path.
                let mut h = std::collections::hash_map::DefaultHasher::new();
                repo.root.hash(&mut h);
                kind.hash(&mut h);
                let path = self
                    .swap_dir
                    .with_file_name("magit-views")
                    .join(format!("{:016x}", h.finish()));
                let mut ed = make_editor(Buffer::default(), &self.cfg);
                ed.path = Some(path);
                ed.magit = Some(Box::new(view.clone()));
                ed.readonly = true;
                let swap = swap::swap_path_in(&self.swap_dir, ed.path.as_deref());
                self.switch_to(
                    Opened {
                        ed,
                        stamp: None,
                        lossy: false,
                    },
                    swap,
                );
            }
        } else if let Some(old) = &self.ed.magit {
            view.return_to = old.return_to;
        }
        // A reused buffer keeps its settings and records the value it showed
        // (magit-go-backward), unless this is that navigation.
        if let Some(old) = self
            .ed
            .magit
            .as_deref()
            .filter(|o| o.repo == repo && slot(o) == kind)
        {
            if old.kind != view.kind && view.back.is_empty() && view.forward.is_empty() {
                view.back = old.back.clone();
                view.back.push(old.kind.clone());
            }
            view.refine = old.refine;
            view.fontify = old.fontify;
            view.locked |= old.locked;
            if old.margin.is_some() {
                view.margin = old.margin.clone();
            }
        }
        self.no_swap = true;
        self.ed.git = crate::git::Gutter::default();
        self.ed.buf = Buffer::from_text(&view.text());
        self.ed.undo = crate::undo::Undo::default();
        self.ed.saved_state = 0;
        self.ed.set_cursor(view.selection(&selected, fallback), 0);
        self.ed.magit = Some(Box::new(view));
        self.reloaded = true;
        let unsaved = self
            .editors_mut()
            .filter(|ed| {
                ed.magit.is_none()
                    && ed.commit_repo.is_none()
                    && ed.buf.modified
                    && ed
                        .path
                        .as_ref()
                        .is_some_and(|p| swap::canonical(p).starts_with(&repo.root))
            })
            .count();
        self.ed.set_msg(if unsaved > 0 {
            format!("{unsaved} unsaved source buffer(s); Git uses saved files")
        } else {
            "Git status refreshed".into()
        });
        crate::magit::options::run_hook("magit-post-refresh-hook", &repo.root);
        // magit-log-select-show-usage: also in the echo area.
        if let Some(select) = self.ed.magit.as_ref().and_then(|v| v.select.as_deref()) {
            let usage = crate::magit::options::string("magit-log-select-show-usage", Some("both"));
            if matches!(usage.as_deref(), Some("both" | "echo-area")) {
                let msg = select.message.clone();
                self.ed.set_msg(msg);
            }
        }
        // magit-refresh-verbose / magit-profile-refresh-buffer.
        let profile = self.profile_once.take();
        if self.refresh_verbose || profile.is_some() {
            let title = self
                .ed
                .magit
                .as_ref()
                .map(|v| v.title())
                .unwrap_or_default();
            let mut msg = format!("Refreshing buffer `{title}'...done");
            if let Some((start, calls)) = profile {
                let n = crate::magit::repo::CALLS_COUNT
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .saturating_sub(calls);
                msg.push_str(&format!(
                    " ({:.3}s, {n} Git calls)",
                    start.elapsed().as_secs_f64()
                ));
            }
            self.ed.set_msg(msg);
        }
    }
    fn refresh_gutters(&mut self) {
        for ed in self.editors_mut() {
            if ed.magit.is_none()
                && ed.commit_repo.is_none()
                && let Some(path) = &ed.path
            {
                ed.git = crate::git::Gutter::load(path);
            }
        }
    }
    pub fn finish_git(&mut self, inv: GitInvocation, result: Result<(), String>) {
        self.git_busy = false;
        // ponytail: last 100 commands; upstream keeps magit-process-log-max.
        let line = inv
            .args
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        self.git_log
            .push((inv.repo.root.clone(), line, result.clone()));
        // magit-process-log-max sections (nil keeps them all).
        let max = crate::magit::options::int_or_nil("magit-process-log-max", Some(32))
            .map_or(usize::MAX, |n| n.max(0) as usize);
        if self.git_log.len() > max {
            let excess = self.git_log.len() - max;
            self.git_log.drain(..excess);
        }
        // magit-refresh-status-buffer nil: only the current buffer refreshes.
        let refresh_status = crate::magit::options::flag("magit-refresh-status-buffer", true);
        for ed in self.editors_mut() {
            if let Some(view) = &mut ed.magit
                && view.repo == inv.repo
                && (refresh_status || view.kind != Kind::Status)
            {
                view.dirty = true;
            }
        }
        self.refresh_gutters();
        if let Err(e) = result {
            if let Some(mut view) = self.ed.magit.as_deref().cloned() {
                let selected = view.action_at(self.ed.cur.line);
                let fallback = self.ed.cur.line;
                self.start_magit(move || {
                    refresh_view(&mut view)?;
                    Ok(Outcome::ErrorView(Box::new(view), e, selected, fallback))
                });
            } else {
                self.ed.set_err(e);
            }
            return;
        }
        if self.auto_revert {
            self.auto_revert_buffers(&inv.repo.root);
        }
        // git-commit-post-finish-hook after a draft's commit, and
        // magit-post-commit-hook after commits made without one.
        if inv.draft.is_some() {
            crate::magit::options::run_hook("git-commit-post-finish-hook", &inv.repo.root);
        } else if inv.args.first().is_some_and(|a| a == "commit") {
            crate::magit::options::run_hook("magit-post-commit-hook", &inv.repo.root);
        }
        // magit-wip-after-apply-mode: record the state Git left behind.
        if self.wip_mode
            && inv.draft.is_none()
            && let Err(e) = inv.repo.wip_commit("wip-save tracked files after Git")
            && !e.contains("No commit")
        {
            self.ed.set_err(format!("magit-wip: {e}"));
        }
        if let Some(crate::magit::repo::After::Git(args)) = inv.after {
            self.pending_git = Some(GitInvocation {
                expected_head: None,
                repo: inv.repo,
                args: args.into_iter().map(OsString::from).collect(),
                input: None,
                draft: None,
                draft_stamp: None,
                editor: false,
                env: vec![],
                after: None,
            });
            return;
        }
        if let Some(crate::magit::repo::After::Clone(after)) = inv.after {
            let (origin, repo) = (self.cur, inv.repo);
            self.start_magit(move || {
                let next = after
                    .finish()
                    .unwrap_or_else(|e| crate::magit::branch::Next::Done(Err(e)));
                Ok(branch_outcome(repo, next, origin))
            });
            return;
        }
        if let Some(path) = inv.draft {
            let stamp = match fileio::write(&path, &[], inv.draft_stamp.as_ref(), false) {
                Ok(stamp) => stamp,
                Err(e) => {
                    self.ed.set_err(format!("committed; draft preserved: {e}"));
                    return;
                }
            };
            for ed in self.editors_mut() {
                if ed.path.as_ref() == Some(&path) {
                    ed.buf = Buffer::default();
                    ed.undo = crate::undo::Undo::default();
                    ed.saved_state = 0;
                    ed.set_cursor(0, 0);
                }
            }
            if self.ed.path.as_ref() == Some(&path) {
                self.stamp = Some(stamp.clone());
                self.release_swap();
                self.lock();
                self.magit_action(Action::Status);
            } else {
                self.ed.set_msg("Committed staged changes");
            }
            for parked in self.bufs.iter_mut().flatten() {
                if parked.ed.path.as_ref() == Some(&path) {
                    parked.stamp = Some(stamp.clone());
                    release(&parked.swap_path);
                    parked.swap_state = SwapState::None;
                }
            }
        } else if self.ed.magit.is_some() {
            self.magit_action(Action::Refresh);
        } else {
            self.ed.set_msg("Git operation completed");
        }
    }
}
/// The prefix whose arguments a sequence suffix reads.
fn sequence_menu(op: &crate::magit::sequence::Op) -> char {
    use crate::magit::sequence::Op as S;
    match op {
        S::Revert | S::RevertNoCommit => 'v',
        S::Mainline(inner, _) => sequence_menu(inner),
        _ => 'x',
    }
}
/// Prompt answers with empty ones replaced by their defaults.
fn merge_answers(answers: &[String], defaults: &[String]) -> Vec<String> {
    answers
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if a.is_empty() {
                defaults.get(i).cloned().unwrap_or_default()
            } else {
                a.clone()
            }
        })
        .collect()
}
fn branch_outcome(repo: Repo, next: crate::magit::branch::Next, origin: usize) -> Outcome {
    use crate::magit::branch::Next;
    match next {
        Next::Done(Ok(message)) => Outcome::Done(repo, message),
        Next::Done(Err(e)) => Outcome::Saved(repo, Err(e)),
        Next::Ask(question, prompts, defaults) => Outcome::Ask(repo, question, defaults, prompts),
        Next::Git(args) => Outcome::Git(GitInvocation {
            expected_head: None,
            repo,
            args: args.into_iter().map(OsString::from).collect(),
            input: None,
            draft: None,
            draft_stamp: None,
            editor: false,
            env: vec![],
            after: None,
        }),
        Next::GitEditor(args) => Outcome::Git(GitInvocation {
            expected_head: None,
            repo,
            args: args.into_iter().map(OsString::from).collect(),
            input: None,
            draft: None,
            draft_stamp: None,
            editor: true,
            env: vec![],
            after: None,
        }),
        Next::Todo(plan) => Outcome::Todo(plan),
        Next::Status(dir) => {
            let view = Repo::discover(&dir).and_then(|repo| {
                let snapshot = repo.status()?;
                Ok(View::new_status(repo, snapshot))
            });
            match view {
                Ok(mut view) => {
                    view.return_to = origin;
                    let line = view.initial_line(None);
                    Outcome::View(Box::new(view), None, line)
                }
                Err(e) => Outcome::Saved(repo, Err(e)),
            }
        }
        Next::Visit(path) => Outcome::VisitFile(path, 0),
        Next::Shell(cmd) => Outcome::Shell(cmd),
        Next::Invoke(inv) => Outcome::Git(inv),
        Next::View(kind) => {
            let mut view = View::status(repo.clone(), Default::default());
            view.kind = kind;
            view.rows.clear();
            view.return_to = origin;
            match refresh_view(&mut view) {
                Ok(()) => Outcome::View(Box::new(view), None, 0),
                Err(e) => Outcome::Saved(repo, Err(e)),
            }
        }
        Next::Replay(plan) => match plan.replay() {
            Ok(inv) => Outcome::Git(inv),
            Err(e) => Outcome::Saved(repo, Err(e)),
        },
        Next::Draft(message) => {
            Outcome::Draft(repo, crate::magit::CommitMode::New, message, vec![])
        }
        Next::Show(target) => match diff_view(repo.clone(), target, vec![], origin) {
            Ok(outcome) => outcome,
            Err(e) => Outcome::Saved(repo, Err(e)),
        },
    }
}
/// The revision (or worktree) and 0-based line a diff line points to, by
/// magit-diff-visit--sides: the old side for removed lines, else the new.
fn diff_visit(view: &View, line: usize, worktree: bool) -> Option<(String, PathBuf, usize)> {
    use crate::magit::blob::{INDEX, WORKTREE};
    use crate::magit::diff::Target;
    let lines: Vec<&str> = view.rows.iter().map(|r| r.text.as_str()).collect();
    let loc = crate::magit::diff::location(&lines, line)?;
    // magit-diff-visit-worktree-file: the worktree at the new side's line.
    if worktree {
        return Some((WORKTREE.to_owned(), loc.file, loc.line - 1));
    }
    let old = |rev: String| Some((rev, loc.old_file.clone(), loc.old_line - 1));
    let new = |rev: String| Some((rev, loc.file.clone(), loc.line - 1));
    let pick = |a: String, b: String| if loc.removed { old(a) } else { new(b) };
    match &view.kind {
        Kind::Diff(Target::Staged, _) => pick("HEAD".into(), INDEX.into()),
        Kind::Diff(Target::Unstaged, _) => pick(INDEX.into(), WORKTREE.into()),
        Kind::Diff(Target::Range(r), _) => {
            let or_head = |s: &str| {
                if s.is_empty() {
                    "HEAD".to_owned()
                } else {
                    s.to_owned()
                }
            };
            if let Some((a, b)) = r.split_once("...") {
                // A...B compares B with the merge base.
                let base = view
                    .repo
                    .read(&["merge-base", "--end-of-options", &or_head(a), &or_head(b)])
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .unwrap_or_else(|_| or_head(a));
                pick(base, or_head(b))
            } else if let Some((a, b)) = r.split_once("..") {
                pick(or_head(a), or_head(b))
            } else {
                // One revision compared with the worktree.
                pick(r.clone(), WORKTREE.into())
            }
        }
        Kind::Diff(Target::Commit(id), _) | Kind::Patch(id) => pick(format!("{id}^"), id.clone()),
        // stash_patch's sections: Unstaged ^2..stash, Staged ^1..^2,
        // Untracked files ^3.
        Kind::StashPatch(stash) => {
            let id = &stash.id;
            let section = lines[..=line]
                .iter()
                .rev()
                .find(|l| matches!(**l, "Unstaged" | "Staged" | "Untracked files"))?;
            match *section {
                "Unstaged" => pick(format!("{id}^2"), id.clone()),
                "Staged" => pick(format!("{id}^1"), format!("{id}^2")),
                _ => new(format!("{id}^3")),
            }
        }
        _ => None,
    }
}
fn blob_outcome(
    repo: Repo,
    rev: String,
    file: PathBuf,
    line: usize,
    message: Option<String>,
    then: Option<crate::magit::blame::Kind>,
) -> Result<Outcome, String> {
    if rev == crate::magit::blob::WORKTREE {
        return Ok(Outcome::VisitFile(repo.root.join(file), line));
    }
    // magit-find-file-noselect: a conflicted index has no single blob.
    if rev == crate::magit::blob::INDEX && repo.conflicted(&file) {
        return Ok(Outcome::VisitFile(repo.root.join(file), line));
    }
    let bytes = repo.blob_bytes(&rev, &file)?;
    let blob = crate::magit::blob::Blob { repo, rev, file };
    Ok(Outcome::Blob(blob, bytes, line, message, then))
}
/// An absolute path with its directory aliases resolved (macOS /var -> /private/var),
/// keeping the file's own name so a symlink is not followed.
fn canonical_file(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let parent = absolute.parent()?;
    // Directories removed since (e.g. just renamed) are kept literally.
    let existing = parent.ancestors().find(|p| p.is_dir())?;
    let rest = parent.strip_prefix(existing).ok()?;
    Some(
        existing
            .canonicalize()
            .ok()?
            .join(rest)
            .join(absolute.file_name()?),
    )
}
/// Whether a buffer's file is `target` or lies inside it, through case aliases
/// or symlinks too: compare fully resolved paths and file identity.
fn same_or_inside(buffer: &Path, target: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    if let (Ok(b), Ok(t)) = (buffer.canonicalize(), target.canonicalize())
        && b.starts_with(&t)
    {
        return true;
    }
    if let (Ok(b), Ok(t)) = (std::fs::metadata(buffer), std::fs::metadata(target))
        && (b.dev(), b.ino()) == (t.dev(), t.ino())
    {
        return true;
    }
    // A directory reached through another spelling: check the buffer's ancestors.
    std::fs::metadata(target).is_ok_and(|t| {
        t.is_dir()
            && buffer.ancestors().skip(1).any(|a| {
                std::fs::metadata(a).is_ok_and(|m| (m.dev(), m.ino()) == (t.dev(), t.ino()))
            })
    })
}
/// A visited path relative to its repository. Directory aliases resolve, a
/// tracked symlink keeps its own name, and since-deleted directories stay literal.
fn repo_relative(repo: &Repo, path: &Path) -> Result<PathBuf, String> {
    canonical_file(path)
        .ok_or("file has no existing parent")?
        .strip_prefix(&repo.root)
        .map(Path::to_path_buf)
        .map_err(|_| "file is outside the repository".into())
}
/// magit-diff--dwim and the at-point defaults of show-commit/stash-show.
/// Ok is a diff target, Err a stash to show; None means ask.
fn diff_context(
    op: DiffOp,
    kind: Option<&Kind>,
    selected: Option<RowAction>,
) -> Option<Result<Target, crate::magit::workflows::Stash>> {
    let commit = |id: &str| Ok(Target::Range(format!("{id}^..{id}")));
    match op {
        DiffOp::Unstaged => Some(Ok(Target::Unstaged)),
        DiffOp::Staged => Some(Ok(Target::Staged)),
        DiffOp::Worktree => Some(Ok(Target::Range("HEAD".into()))),
        DiffOp::Range | DiffOp::Paths => None,
        DiffOp::ShowCommit => match (selected.as_ref(), kind) {
            (Some(RowAction::Commit(id)), _) | (_, Some(Kind::Patch(id))) => {
                Some(Ok(Target::Commit(id.clone())))
            }
            (_, Some(Kind::Diff(Target::Commit(id), _))) => Some(Ok(Target::Commit(id.clone()))),
            _ => None,
        },
        DiffOp::ShowStash => match (selected, kind) {
            (Some(RowAction::Stash(stash)), _) => Some(Err(stash)),
            (_, Some(Kind::StashPatch(stash))) => Some(Err(stash.clone())),
            _ => None,
        },
        DiffOp::Dwim => match (selected, kind) {
            (Some(RowAction::Commit(id)), _) => Some(commit(&id)),
            (Some(RowAction::Stash(stash)), _) => Some(Err(stash)),
            (Some(RowAction::Section(Section::Staged)), _)
            | (Some(RowAction::File(_, Section::Staged)), _)
            | (Some(RowAction::Hunk(_, true, ..)), _) => Some(Ok(Target::Staged)),
            // Log sections diff their range as endpoints (magit-diff--range-to-endpoints).
            (Some(RowAction::Section(Section::UnpushedUpstream)), _) => {
                Some(Ok(Target::Range("@{upstream}...".into())))
            }
            (Some(RowAction::Section(Section::UnpulledUpstream)), _) => {
                Some(Ok(Target::Range("...@{upstream}".into())))
            }
            (Some(RowAction::Section(Section::UnpushedPush)), _) => {
                Some(Ok(Target::Range("@{push}...".into())))
            }
            (Some(RowAction::Section(Section::UnpulledPush)), _) => {
                Some(Ok(Target::Range("...@{push}".into())))
            }
            (Some(RowAction::Section(Section::Stashes)), _) => None,
            // magit-diff--dwim has no untracked case: fall through to the range prompt.
            (Some(RowAction::Section(Section::Untracked)), _)
            | (Some(RowAction::File(_, Section::Untracked)), _) => None,
            // ponytail: conflicts show the unstaged diff; magit-diff-unmerged's merge range is open.
            (Some(RowAction::Section(_) | RowAction::File(..) | RowAction::Hunk(..)), _) => {
                Some(Ok(Target::Unstaged))
            }
            (_, Some(Kind::Patch(id))) => Some(commit(id)),
            // In magit-stash-mode dwim is (commit . stash): the worktree part.
            (_, Some(Kind::StashPatch(stash))) => Some(commit(&stash.id)),
            (_, Some(Kind::Diff(target, _))) => Some(Ok(target.clone())),
            _ => None,
        },
    }
}
fn diff_view(
    repo: Repo,
    target: Target,
    args: Vec<String>,
    origin: usize,
) -> Result<Outcome, String> {
    let mut view = View::status(repo.clone(), repo.status()?);
    view.kind = Kind::Diff(target, args);
    view.return_to = origin;
    refresh_view(&mut view)?;
    Ok(Outcome::View(Box::new(view), None, 0))
}
fn stash_view(
    repo: Repo,
    stash: crate::magit::workflows::Stash,
    origin: usize,
) -> Result<Outcome, String> {
    let mut view = View::status(repo.clone(), repo.status()?);
    view.rows = display_patch(&repo.stash_patch(&stash)?);
    view.kind = Kind::StashPatch(stash);
    view.return_to = origin;
    Ok(Outcome::View(Box::new(view), None, 0))
}
fn refresh_view(view: &mut View) -> Result<(), String> {
    refresh_rows(view)?;
    view.load_stamps()
}
fn refresh_rows(view: &mut View) -> Result<(), String> {
    view.dirty = false;
    if view.kind == Kind::Repos {
        // tabulated-list: a header line, then a marker column and the cells.
        let (cols, table) = crate::magit::repos::table()?;
        let header: Vec<String> = cols
            .iter()
            .map(|c| crate::magit::repos::pad(&c.header, c))
            .collect();
        let mut rows = vec![Row {
            text: format!("  {}", header.join(" ").trim_end()),
            action: None,
        }];
        view.marked.retain(|p| table.iter().any(|(_, q, _)| q == p));
        for (_, path, cells) in table {
            let text: Vec<String> = cols
                .iter()
                .zip(&cells)
                .map(|(c, v)| crate::magit::repos::pad(&label(Path::new(v)), c))
                .collect();
            let mark = if view.marked.contains(&path) {
                "*"
            } else {
                " "
            };
            rows.push(Row {
                text: format!("{mark} {}", text.join(" ").trim_end()),
                action: Some(RowAction::Repo(path)),
            });
        }
        view.rows = rows;
        return Ok(());
    }
    if let Kind::Diff(target, args) = &view.kind {
        let mut rows = vec![Row {
            text: format!("{} (gr refresh, q return)", target.title()),
            action: None,
        }];
        rows.extend(display_patch(&view.repo.diff_output(target, args)?));
        view.rows = rows;
        return Ok(());
    }
    if matches!(
        view.kind,
        Kind::Log(..) | Kind::FileLog(..) | Kind::Reflog(_)
    ) {
        return refresh_log(view);
    }
    // Output views run their command once (request-pull reaches the network);
    // later refreshes keep the text, like upstream's mail buffer.
    if let Kind::Output(title, argv) = &view.kind
        && view.rows.is_empty()
    {
        let mut rows = vec![Row {
            text: format!("{} (gr refresh, q return)", label(Path::new(title))),
            action: None,
        }];
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        rows.extend(display_patch(&view.repo.read_network(&argv)?));
        view.rows = rows;
        return Ok(());
    }
    if matches!(view.kind, Kind::Output(..) | Kind::Process) {
        return Ok(());
    }
    if let Kind::Refs(focus, args, count) = &view.kind {
        let mut rows = vec![Row {
            text: format!(
                "References compared with {} (Enter visit, Space m y menu, gr refresh, q return)",
                label(Path::new(focus))
            ),
            action: None,
        }];
        for (text, id) in view.repo.refs_rows(focus, args, *count)? {
            rows.push(Row {
                text: label(Path::new(&text)),
                action: id.map(RowAction::Commit),
            });
        }
        view.rows = rows;
        return Ok(());
    }
    if view.kind == Kind::Modules {
        let mut rows = vec![Row {
            text: "Modules (Enter visit, Space m o actions, gr refresh, q return)".into(),
            action: None,
        }];
        for (text, module) in view.repo.module_rows()? {
            rows.push(Row {
                text: label(Path::new(&text)),
                action: Some(RowAction::Module(module)),
            });
        }
        if rows.len() == 1 {
            rows.push(Row {
                text: "No modules".into(),
                action: None,
            });
        }
        view.rows = rows;
        return Ok(());
    }
    if let Kind::Cherry(head, upstream) = &view.kind {
        // magit-insert-cherry-headers and -commits.
        let mut rows = vec![
            Row {
                text: format!("Head:     {}", label(Path::new(head))),
                action: None,
            },
            Row {
                text: format!("Upstream: {}", label(Path::new(upstream))),
                action: None,
            },
            Row {
                text: "Cherry commits (+ not in upstream, - equivalent change upstream)".into(),
                action: None,
            },
        ];
        for (sign, id, subject) in view.repo.cherry(head, upstream)? {
            rows.push(Row {
                text: format!(
                    "{sign} {} {}",
                    &id[..id.len().min(8)],
                    label(Path::new(&subject))
                ),
                action: Some(RowAction::Commit(id)),
            });
        }
        view.rows = rows;
        return Ok(());
    }
    if let Kind::Shortlog(rev, args) = &view.kind {
        let mut rows = vec![Row {
            text: format!(
                "git shortlog {} {} (gr refresh, q return)",
                label(Path::new(&args.join(" "))),
                label(Path::new(rev))
            ),
            action: None,
        }];
        rows.extend(display_patch(&view.repo.shortlog(rev, args)?));
        view.rows = rows;
        return Ok(());
    }
    if let Kind::StashPatch(stash) = &view.kind {
        view.rows = display_patch(&view.repo.stash_patch(stash)?);
        return Ok(());
    }
    if matches!(view.kind, Kind::Stashes | Kind::Tags) {
        view.snapshot = view.repo.status()?;
        return refresh_refs(view);
    }
    if view.kind != Kind::Status {
        return Ok(());
    }
    view.snapshot = view.repo.status()?;
    view.snapshot.extra = view.repo.status_extra();
    view.diffs.clear();
    for (path, staged) in view.expanded.clone() {
        if view
            .snapshot
            .entries
            .iter()
            .any(|e| e.path == path && if staged { e.staged } else { e.unstaged })
        {
            view.diffs
                .insert((path.clone(), staged), view.repo.diff(&path, staged)?);
        } else {
            view.expanded.remove(&(path, staged));
        }
    }
    view.rebuild();
    Ok(())
}
fn reflog_view(repo: Repo, target: String, origin: usize) -> Result<Outcome, String> {
    if target.is_empty() || target.starts_with('-') || target.chars().any(char::is_control) {
        return Err(format!("invalid ref {target:?}"));
    }
    repo.read(&["rev-parse", "--verify", "-q", "--end-of-options", &target])
        .map_err(|_| format!("unknown ref {target:?}"))?;
    let mut view = View::status(repo.clone(), Default::default());
    view.kind = Kind::Reflog(target);
    view.return_to = origin;
    refresh_log(&mut view)?;
    Ok(Outcome::View(Box::new(view), None, 0))
}
fn refresh_log(view: &mut View) -> Result<(), String> {
    if let Kind::Reflog(target) = &view.kind {
        // magit-reflog-refresh-buffer, limited by magit-reflog-limit (256).
        let limit = format!(
            "-n{}",
            crate::magit::options::int("magit-reflog-limit", 256).max(1)
        );
        let out = view.repo.read(&[
            "reflog",
            "show",
            "--format=%H%x00%gd%x00%gs",
            &limit,
            "--end-of-options",
            target,
            "--",
        ])?;
        let mut rows = vec![Row {
            text: format!(
                "Reflog for {} (Enter inspect, gr refresh, q return)",
                label(Path::new(target))
            ),
            action: None,
        }];
        let fields: Vec<_> = out.split(|b| *b == 0 || *b == b'\n').collect();
        for c in fields
            .chunks(3)
            .filter(|c| c.len() == 3 && !c[0].is_empty())
        {
            let id = String::from_utf8_lossy(c[0]).trim().to_owned();
            rows.push(Row {
                text: format!(
                    "{} {} {}",
                    &id[..id.len().min(8)],
                    label(Path::new(&String::from_utf8_lossy(c[1]).into_owned())),
                    label(Path::new(&String::from_utf8_lossy(c[2]).into_owned()))
                ),
                action: Some(RowAction::Commit(id)),
            });
        }
        view.rows = rows;
        return Ok(());
    }
    if let Kind::Log(revs, args) = &view.kind {
        // magit-log-refresh-buffer: header line, then the washed log; a
        // magit-log-select buffer shows its usage instead.
        let mut rows = vec![Row {
            text: match &view.select {
                // magit-log-select-show-usage: header-line (or both).
                Some(select)
                    if matches!(
                        crate::magit::options::string("magit-log-select-show-usage", Some("both"))
                            .as_deref(),
                        Some("both" | "header-line")
                    ) =>
                {
                    select.message.clone()
                }
                Some(_) => "Select a commit".into(),
                None => format!(
                    "Commits in {}{} (Enter inspect, = limit, + more, gr refresh, q return)",
                    label(Path::new(&revs.join(" "))),
                    if args.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", label(Path::new(&args.join(" "))))
                    }
                ),
            },
            action: None,
        }];
        let lines = view.repo.log_lines(revs, args, &[])?;
        if lines.is_empty() {
            rows.push(Row {
                text: "No commits for this history".into(),
                action: None,
            });
        }
        rows.extend(lines.into_iter().map(|l| Row {
            text: l.text,
            action: l.commit.map(RowAction::Commit),
        }));
        view.rows = rows;
        return Ok(());
    }
    let (heading, commits) = match &view.kind {
        Kind::FileLog(path, follow) => (
            format!(
                "History: {}{} (Enter inspect, gr refresh, q return)",
                label(path),
                if *follow { " --follow" } else { "" }
            ),
            view.repo.file_history(path, *follow)?,
        ),
        _ => (
            "Recent commits (Enter inspect, gr refresh, q return)".into(),
            view.repo.history()?,
        ),
    };
    view.rows = vec![Row {
        text: heading,
        action: None,
    }];
    for c in commits {
        view.rows.push(Row {
            text: format!(
                "{} {}",
                &c.id[..c.id.len().min(8)],
                label(Path::new(&c.subject))
            ),
            action: Some(RowAction::Commit(c.id)),
        });
    }
    if view.rows.len() == 1 {
        view.rows.push(Row {
            text: "No commits for this history".into(),
            action: None,
        });
    }
    Ok(())
}

fn display_patch(bytes: &[u8]) -> Vec<Row> {
    let limit = bytes.len().min(1024 * 1024);
    let text = String::from_utf8_lossy(&bytes[..limit]);
    let mut rows: Vec<_> = text
        .lines()
        .map(|line| Row {
            text: label(Path::new(&line.chars().take(20_000).collect::<String>())),
            action: None,
        })
        .collect();
    if bytes.len() > limit {
        rows.push(Row {
            text: "[output truncated]".into(),
            action: None,
        });
    }
    rows
}

fn refresh_refs(view: &mut View) -> Result<(), String> {
    if view.kind == Kind::Stashes {
        view.rows = vec![Row {
            text: "a apply  p pop  d drop  Enter inspect  gr refresh  q return".into(),
            action: None,
        }];
        for stash in view.repo.stashes()? {
            view.rows.push(Row {
                text: format!("{} {}", stash.selector, label(Path::new(&stash.subject))),
                action: Some(RowAction::Stash(stash)),
            });
        }
        if view.rows.len() == 1 {
            view.rows.push(Row {
                text: "No entries".into(),
                action: None,
            });
        }
        let conflicts = view.snapshot.entries.iter().filter(|e| e.conflict).count();
        if conflicts > 0 {
            view.rows.push(Row {
                text: format!("Conflicts: {conflicts}; Space m s to resolve"),
                action: None,
            });
        }
        return Ok(());
    }
    let bytes = view.repo.read(&[
        "for-each-ref",
        "--format=%(objectname)%00%(refname:short)",
        "refs/tags/",
    ])?;
    view.rows = vec![Row {
        text: "Enter inspect  gr refresh  q return".into(),
        action: None,
    }];
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let mut parts = line.splitn(2, |b| *b == 0);
        let id = String::from_utf8_lossy(parts.next().unwrap_or_default()).to_string();
        let name = String::from_utf8_lossy(parts.next().ok_or("invalid reference listing")?);
        view.rows.push(Row {
            text: label(Path::new(name.as_ref())),
            action: Some(RowAction::Commit(id)),
        });
    }
    if view.rows.len() == 1 {
        view.rows.push(Row {
            text: "No entries".into(),
            action: None,
        });
    }
    Ok(())
}

/// GIT_COMMITTER_DATE set to HEAD's (magit-rev-format "%cD").
fn keep_committer_date(repo: &Repo) -> Option<(String, String)> {
    repo.read(&["log", "-1", "--format=%cD", "HEAD"])
        .ok()
        .map(|o| {
            (
                "GIT_COMMITTER_DATE".into(),
                String::from_utf8_lossy(&o).trim().to_owned(),
            )
        })
}
/// The verb magit-commit-squash-internal's log-select message uses.
fn op_verb(op: &crate::magit::commit::Op) -> &'static str {
    use crate::magit::commit::Op as C;
    match op {
        C::Fixup | C::InstantFixup => "fixup",
        C::Squash | C::InstantSquash => "squash into",
        C::Alter => "alter",
        C::Augment => "augment",
        _ => "revise",
    }
}
