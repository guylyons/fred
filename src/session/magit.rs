use super::*;
use crate::magit::repo::{GitInvocation, Repo, label};
use crate::magit::{Action, Kind, Row, RowAction, Section, View};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::thread::JoinHandle;

pub(super) enum Outcome {
    NoticeView(Box<View>, String, Option<RowAction>, usize),
    ErrorView(Box<View>, String, Option<RowAction>, usize),
    View(Box<View>, Option<RowAction>, usize),
    Prompt(Repo, crate::magit::workflows::Operation, Vec<String>),
    Draft(Repo, crate::magit::CommitMode, Vec<u8>, Vec<String>),
    Branches(Repo, Vec<String>),
    Git(GitInvocation),
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
        if let Action::Submit(repo, operation, value, args) = action {
            self.start_magit(move || {
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
        if let Action::Switch(name) = action {
            if let Some(repo) = self.magit_picker_repo.take() {
                self.ed.mode = Mode::Normal;
                self.pending_git = Some(GitInvocation {
                    expected_head: None,
                    repo,
                    args: vec!["switch".into(), "--".into(), name.into()],
                    input: None,
                    draft: None,
                    draft_stamp: None,
                });
            } else {
                self.ed.set_err("no repository selected for branch switch");
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
                        self.start_magit(move || {
                            let bytes = view.repo.stash_patch(&stash)?;
                            view.kind = Kind::StashPatch(stash);
                            view.rows = display_patch(&bytes);
                            Ok(Outcome::View(Box::new(view), None, 0))
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
        let commit_args = crate::magit::menu_arguments(&self.ed, 'C');
        let stash_args = crate::magit::menu_arguments(&self.ed, 'z');
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
                        let mut view = View::status(repo, snapshot);
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
                Action::Log => {
                    let mut view = View::status(repo.clone(), repo.status()?);
                    view.kind = Kind::Log;
                    view.return_to = origin;
                    view.rows = vec![Row {
                        text: "Recent commits (Enter inspect, q return)".into(),
                        action: None,
                    }];
                    for c in repo.history()? {
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
                            text: "No commits yet".into(),
                            action: None,
                        });
                    }
                    Ok(Outcome::View(Box::new(view), None, 0))
                }
                Action::Workflow(operation) => {
                    if operation.prompt().is_some() {
                        let args = if matches!(
                            operation,
                            crate::magit::workflows::Operation::Stash
                                | crate::magit::workflows::Operation::StashUntracked
                                | crate::magit::workflows::Operation::StashKeepIndex
                        ) {
                            stash_args
                        } else if operation == crate::magit::workflows::Operation::Fixup {
                            commit_args
                        } else {
                            vec![]
                        };
                        Ok(Outcome::Prompt(repo, operation, args))
                    } else {
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
                    let names = repo.branches()?;
                    Ok(Outcome::Branches(repo, names))
                }
                Action::Push | Action::Pull | Action::Fetch => {
                    let args: Vec<OsString> = match action {
                        Action::Push => vec!["push".into()],
                        Action::Pull => vec!["pull".into(), "--ff-only".into()],
                        _ => vec!["fetch".into()],
                    };
                    Ok(Outcome::Git(GitInvocation {
                        expected_head: None,
                        repo,
                        args,
                        input: None,
                        draft: None,
                        draft_stamp: None,
                    }))
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
        if self.cur != job.slot || self.clock != job.clock {
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
                if mode.target().is_some()
                    && !path.exists()
                    && let Err(error) = fileio::write(&path, &message, None, false)
                {
                    self.ed.set_err(error);
                    return true;
                }
                self.magit_drafts
                    .insert(path.clone(), (repo.clone(), mode.clone(), args));
                self.open_pick(path, None);
                self.attach_commit_repo();
                self.ed.set_msg(match mode {
                    crate::magit::CommitMode::New => {
                        "Commit draft: :w save; Space m c commit staged changes"
                    }
                    crate::magit::CommitMode::Amend(_) => {
                        "Amend draft: :w save; Space m c amend staged changes and message"
                    }
                    crate::magit::CommitMode::Reword(_) => {
                        "Reword draft: :w save; Space m c replace message, keep HEAD tree"
                    }
                });
            }
            Ok(Outcome::Branches(repo, names)) => {
                self.magit_picker_repo = Some(repo);
                crate::pick::branches(&mut self.ed, names);
            }
            Ok(Outcome::Git(inv)) => self.pending_git = Some(inv),
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
    fn install_magit(&mut self, mut view: View, selected: Option<RowAction>, fallback: usize) {
        let kind = view.kind.clone();
        let repo = view.repo.clone();
        let same = |ed: &Editor| {
            ed.magit
                .as_ref()
                .is_some_and(|v| v.repo == repo && v.kind == kind)
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
                format!("{kind:?}").hash(&mut h);
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
fn refresh_view(view: &mut View) -> Result<(), String> {
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
