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
        if self.magit_job.is_some() || self.pending_git.is_some() || self.git_busy {
            self.ed.set_err("Git operation in progress");
            return;
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
            let origin = self.cur;
            self.start_magit(move || {
                let merged = merge_answers(&answers, &defaults);
                let next = repo.branch_step(op, &merged, &defaults);
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
        if let Action::Answered(repo, Question::Commit(op), answers, defaults) = action {
            let (origin, args) = (self.cur, crate::magit::menu_arguments(&self.ed, 'C'));
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
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.commit_prompts(&op, at_point);
                Ok(Outcome::Ask(repo, Question::Commit(op), defaults, prompts))
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
            let commit_args = crate::magit::menu_arguments(&self.ed, 'C');
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
            let from = self.magit_from();
            self.start_magit(move || {
                let repo = Repo::discover(&from)?;
                let (prompts, defaults) = repo.branch_prompts(&op)?;
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
                | Question::Clone(_) => {
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
        if action == Action::Commit
            && let Some(repo) = self.ed.commit_repo.clone()
        {
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
                    }
                    self.pending_git = Some(inv);
                }
                Err(e) => self.ed.set_err(e),
            }
            return;
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
                self.pending_git = Some(GitInvocation {
                    expected_head: None,
                    repo,
                    // magit-checkout: a branch, or any revision (detaching HEAD).
                    args: vec!["checkout".into(), name.into(), "--".into()],
                    input: None,
                    draft: None,
                    draft_stamp: None,
                    editor: false,
                });
            }
            return;
        }
        if matches!(
            action,
            Action::Toggle | Action::Stage | Action::Unstage | Action::Visit | Action::Refresh
        ) {
            let Some(mut view) = self.ed.magit.as_deref().cloned() else {
                self.ed.set_err("open Git status first");
                return;
            };
            let selected = view.action_at(self.ed.cur.line);
            let fallback = self.ed.cur.line;
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
        let commit_args = crate::magit::menu_arguments(&self.ed, 'C');
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
                        let mut view = View::new_status(repo, snapshot);
                        view.return_to = origin;
                        Ok(Outcome::View(Box::new(view), None, 0))
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
                        }
                        Ok(Outcome::Git(inv))
                    }
                }
                Action::Commit => Ok(Outcome::Draft(
                    repo,
                    crate::magit::CommitMode::New,
                    vec![],
                    commit_args,
                )),
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
                },
                result,
            ),
        }
        true
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
    fn file_action(&mut self, op: crate::magit::blob::FileOp) {
        use crate::magit::blob::FileOp as O;
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
        // Like magit-diff-mode, one diff buffer per repository is refreshed in place.
        let slot = |kind: &Kind| match kind {
            Kind::Diff(..) => "Diff".to_owned(),
            kind => format!("{kind:?}"),
        };
        let kind = slot(&view.kind);
        let repo = view.repo.clone();
        let same = |ed: &Editor| {
            ed.magit
                .as_ref()
                .is_some_and(|v| v.repo == repo && slot(&v.kind) == kind)
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
        for ed in self.editors_mut() {
            if let Some(view) = &mut ed.magit
                && view.repo == inv.repo
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
        }),
        Next::GitEditor(args) => Outcome::Git(GitInvocation {
            expected_head: None,
            repo,
            args: args.into_iter().map(OsString::from).collect(),
            input: None,
            draft: None,
            draft_stamp: None,
            editor: true,
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
                    Outcome::View(Box::new(view), None, 0)
                }
                Err(e) => Outcome::Saved(repo, Err(e)),
            }
        }
        Next::Visit(path) => Outcome::VisitFile(path, 0),
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
    view.dirty = false;
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
    if matches!(view.kind, Kind::Output(..)) {
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
        let out = view.repo.read(&[
            "reflog",
            "show",
            "--format=%H%x00%gd%x00%gs",
            "-n256",
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
        // magit-log-refresh-buffer: header line, then the washed log.
        let mut rows = vec![Row {
            text: format!(
                "Commits in {}{} (Enter inspect, = limit, + more, gr refresh, q return)",
                label(Path::new(&revs.join(" "))),
                if args.is_empty() {
                    String::new()
                } else {
                    format!(" {}", label(Path::new(&args.join(" "))))
                }
            ),
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
                "{} {} {} {}",
                &c.id[..c.id.len().min(8)],
                c.date,
                label(Path::new(&c.author)),
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
