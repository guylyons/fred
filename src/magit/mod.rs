//! A status-centered Git component; rendered text is never used as an operation path.
pub mod apply;
pub mod bisect;
pub mod blame;
pub mod blob;
pub mod branch;
pub mod bundle;
pub mod clone;
pub mod commit;
pub mod configure;
pub mod diff;
pub mod ignore;
pub mod log;
pub mod merge;
pub mod network;
pub mod notes;
pub mod patch;
pub mod rebase;
pub mod refs;
pub mod remote;
pub mod repo;
pub mod reset;
pub mod sequence;
pub mod stash;
pub mod status;
pub mod submodule;
pub mod subtree;
pub mod tag;
pub mod workflows;
pub mod worktree;
use crate::{
    editor::{Editor, Mode},
    ex::ExEffect,
    key::{Key, KeyCode},
};
use repo::{Diff, Repo, Snapshot, label};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub const HELP: &str = "Magit: s status  p push  P pull  f fetch  c commit menu  l log menu  L file log  b branch menu  r revert  z stash  t tag  C commit  M merge  R rebase  x cherry-pick";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Menu(char),
    ToggleOption(MenuOption),
    CycleOption(&'static str),
    /// A transient-option read from the minibuffer: its argument prefix.
    ReadOption(&'static str),
    AmendDraft,
    RewordDraft,
    Stash(workflows::StashAction),
    DropStash(repo::Repo, workflows::Stash),
    Stashes,
    Tags,
    Workflow(workflows::Operation),
    Submit(repo::Repo, workflows::Operation, String, Vec<String>),
    Status,
    Net(network::Op),
    Diff(diff::Op),
    Init,
    /// Blame the visited file or blob.
    Blame(blame::Kind),
    BlameQuit,
    /// magit-find-file: prompt for revision and file.
    FindFile,
    /// Visit REV:FILE ({worktree} visits the file itself).
    BlobVisit(String, PathBuf),
    /// magit-blame-visit-other-file, then blame that blob the same way.
    BlobVisitBlame(String, PathBuf, blame::Kind, usize),
    BlobPrevious,
    BlobNext,
    /// magit-blob-visit-file: the worktree file of this blob.
    BlobVisitFile,
    BlobQuit,
    /// A magit-branch.el suffix.
    Branch(branch::Op),
    /// A magit-tag.el suffix.
    Tag(tag::Op),
    /// A magit-merge.el suffix.
    Merge(merge::Op),
    /// A magit-reset.el suffix.
    Reset(reset::Op),
    /// A magit-remote.el suffix.
    Remote(remote::Op),
    /// A cherry-pick or revert suffix from magit-sequence.el.
    Sequence(sequence::Op),
    /// A rebase suffix from magit-sequence.el.
    Rebase(rebase::Op),
    /// A fixup/squash suffix from magit-commit.el.
    CommitEdit(commit::Op),
    /// A stash transform from magit-stash.el.
    StashOp(stash::Op),
    /// A magit-worktree.el suffix.
    Worktree(worktree::Op),
    /// A magit-notes.el suffix.
    Notes(notes::Op),
    /// A magit-bisect.el suffix.
    Bisect(bisect::Op),
    /// A magit-log.el suffix.
    LogOp(log::Op),
    /// A magit-submodule.el suffix.
    Submodule(submodule::Op),
    /// A magit-subtree.el suffix.
    Subtree(subtree::Op),
    /// A magit-patch.el or magit-am suffix.
    Patch(patch::Op),
    /// A magit-bundle.el suffix.
    Bundle(bundle::Op),
    /// A magit-refs.el suffix.
    Refs(refs::Op),
    /// Branch/remote configuration, orphan, shelve and unshallow.
    Configure(configure::Op),
    /// magit-discard / -reverse / -stage-modified / -unstage-all at point.
    ApplyOp(apply::Kind),
    /// magit-diff-refresh suffixes for the diff buffer.
    DiffRefresh(diff::Refresh),
    /// A magit-gitignore.el or magit-sparse-checkout.el suffix.
    Ignore(ignore::Op),
    /// A magit-clone.el suffix.
    Clone(clone::Op),
    /// magit-reflog-current / -head / -other (None asks for a ref).
    Reflog(Option<String>),
    /// git-rebase-show-commit: the commit on the todo line at point.
    RebaseShowCommit,
    /// ZZ / ZQ in a rebase todo buffer.
    RebaseFinish,
    RebaseCancel,
    /// magit-file-stage/unstage/untrack/rename/delete/checkout.
    File(blob::FileOp),
    BlameCycle,
    /// Directory to initialize, and whether nesting/reinitializing was confirmed.
    InitDir(PathBuf, bool),
    /// All of a question's prompts answered: repo, question, answers, arguments.
    Answered(repo::Repo, Question, Vec<String>, Vec<String>),
    Commit,
    Log,
    FileLog,
    LogHead,
    Branches,
    /// Branch picker result: the selected row and the typed text.
    Switch(Option<String>, String),
    Refresh,
    /// magit-diff-visit-worktree-file (C-j under evil-collection).
    VisitWorktree,
    /// magit-section show/hide/children/levels (evil-collection z keys).
    Fold(Fold),
    /// magit-diff-buffer-file.
    DiffBufferFile,
    /// magit-diff-while-committing (C-c C-d in a commit draft).
    DiffWhileCommitting,
    Toggle,
    Stage,
    Unstage,
    Visit,
    Return,
}
/// magit-section visibility commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Show,
    Hide,
    ShowChildren,
    HideChildren,
    /// magit-section-show-level-N-all.
    Level(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Conflicts,
    Untracked,
    Unstaged,
    Staged,
    Stashes,
    UnpushedPush,
    /// Unmerged into upstream, or recent commits.
    UnpushedUpstream,
    UnpulledPush,
    UnpulledUpstream,
}
impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Untracked => "Untracked files",
            Self::Unstaged => "Unstaged changes",
            Self::Staged => "Staged changes",
            Self::Stashes => "Stashes",
            Self::UnpushedPush => "Unpushed to <push-remote>",
            Self::UnpushedUpstream => "Unpushed to @{upstream}",
            Self::UnpulledPush => "Unpulled from <push-remote>",
            Self::UnpulledUpstream => "Unpulled from @{upstream}",
        }
    }
    pub(crate) fn contains(self, e: &repo::Entry) -> bool {
        match self {
            Self::Conflicts => e.conflict,
            Self::Untracked => e.untracked,
            Self::Unstaged => e.unstaged,
            Self::Staged => e.staged,
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowAction {
    Section(Section),
    File(PathBuf, Section),
    Hunk(PathBuf, bool, usize, usize),
    Stash(workflows::Stash),
    Commit(String),
    /// A module path (magit-list-submodules).
    Module(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub action: Option<RowAction>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Status,
    /// magit-log-mode: revisions and git-log arguments.
    Log(Vec<String>, Vec<String>),
    FileLog(PathBuf, bool),
    Stashes,
    Tags,
    StashPatch(workflows::Stash),
    Patch(String),
    Diff(diff::Target, Vec<String>),
    /// magit-reflog-mode for a ref.
    Reflog(String),
    /// magit-submodule-list-mode.
    Modules,
    /// magit-refs-mode: focus ref, arguments and commit-count display.
    Refs(String, Vec<String>, refs::Count),
    /// Output of a read-only Git command: title and arguments.
    Output(String, Vec<String>),
    /// magit-cherry-mode: head and upstream.
    Cherry(String, String),
    /// *magit-shortlog*: revision or range and arguments.
    Shortlog(String, Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub dirty: bool,
    pub repo: Repo,
    pub kind: Kind,
    pub snapshot: Snapshot,
    pub rows: Vec<Row>,
    pub return_to: usize,
    pub closed: HashSet<Section>,
    pub expanded: HashSet<(PathBuf, bool)>,
    pub diffs: HashMap<(PathBuf, bool), Diff>,
}
impl View {
    /// A new status buffer: sections start hidden as upstream inserts them
    /// (magit-section-initial-visibility-alist and the log sections' HIDE).
    pub fn new_status(repo: Repo, mut snapshot: Snapshot) -> Self {
        snapshot.extra = repo.status_extra();
        let mut v = Self::status(repo, snapshot);
        v.closed.insert(Section::Stashes);
        for (section, heading, _) in &v.snapshot.extra.logs {
            if *section != Section::UnpushedUpstream || heading == "Recent commits" {
                v.closed.insert(*section);
            }
        }
        v.rebuild();
        v
    }
    pub fn status(repo: Repo, snapshot: Snapshot) -> Self {
        let mut v = Self {
            dirty: false,
            repo,
            snapshot,
            kind: Kind::Status,
            rows: vec![],
            return_to: 0,
            closed: HashSet::new(),
            expanded: HashSet::new(),
            diffs: HashMap::new(),
        };
        v.rebuild();
        v
    }
    pub fn title(&self) -> String {
        match &self.kind {
            Kind::Status => "Magit status".into(),
            Kind::Log(revs, _) => {
                format!("Magit log {}", label(std::path::Path::new(&revs.join(" "))))
            }
            Kind::FileLog(path, _) => format!("Magit file log {}", label(path)),
            Kind::Stashes => "Magit stashes".into(),
            Kind::Tags => "Magit tags".into(),
            Kind::StashPatch(stash) => format!("Magit {}", stash.selector),
            Kind::Patch(id) => format!("Magit {id}"),
            Kind::Diff(target, _) => format!("Magit diff: {}", target.title()),
            Kind::Reflog(r) => format!("Magit reflog {}", label(std::path::Path::new(r))),
            Kind::Modules => "Magit modules".into(),
            Kind::Refs(focus, ..) => format!("Magit refs {}", label(std::path::Path::new(focus))),
            Kind::Output(title, _) => format!("Magit {}", label(std::path::Path::new(title))),
            Kind::Cherry(h, u) => format!(
                "Magit cherry {}",
                label(std::path::Path::new(&format!("{u}..{h}")))
            ),
            Kind::Shortlog(r, _) => format!("Magit shortlog {}", label(std::path::Path::new(r))),
        }
    }
    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn action_at(&self, line: usize) -> Option<RowAction> {
        self.rows.get(line).and_then(|r| r.action.clone())
    }
    pub fn rebuild(&mut self) {
        if self.kind != Kind::Status {
            return;
        }
        self.rows.clear();
        for header in &self.snapshot.extra.headers {
            self.rows.push(Row {
                text: header.clone(),
                action: None,
            });
        }
        if let Some(operation) = &self.snapshot.operation {
            self.rows.push(Row {
                text: format!("In progress: {operation}"),
                action: None,
            });
        }
        self.rows.push(Row {
            text: "Tab expand  s stage  u unstage  gr refresh  Enter visit  q return".into(),
            action: None,
        });
        if self.snapshot.entries.is_empty() {
            self.rows.push(Row {
                text: "Working tree clean".into(),
                action: None,
            });
        }
        for section in [
            Section::Conflicts,
            Section::Untracked,
            Section::Unstaged,
            Section::Staged,
        ] {
            let entries: Vec<_> = self
                .snapshot
                .entries
                .iter()
                .filter(|e| section.contains(e))
                .collect();
            if entries.is_empty() {
                continue;
            }
            let shut = self.closed.contains(&section);
            self.rows.push(Row {
                text: String::new(),
                action: None,
            });
            self.rows.push(Row {
                text: format!(
                    "{} {} ({})",
                    if shut { ">" } else { "v" },
                    section.name(),
                    entries.len()
                ),
                action: Some(RowAction::Section(section)),
            });
            if shut {
                continue;
            }
            for e in entries {
                let staged = section == Section::Staged;
                let key = (e.path.clone(), staged);
                let expanded = self.expanded.contains(&key);
                let name = e.old_path.as_ref().map_or_else(
                    || label(&e.path),
                    |old| format!("{} -> {}", label(old), label(&e.path)),
                );
                self.rows.push(Row {
                    text: format!("  {} {} {}", if expanded { "v" } else { ">" }, e.xy, name),
                    action: Some(RowAction::File(e.path.clone(), section)),
                });
                if expanded && let Some(d) = self.diffs.get(&key) {
                    let mut off = 0;
                    let mut hunk = None;
                    let mut source_line = 0;
                    for bytes in d.bytes.split_inclusive(|b| *b == b'\n') {
                        if off >= 1024 * 1024 {
                            self.rows.push(Row {
                                text: "    [diff display truncated]".into(),
                                action: None,
                            });
                            break;
                        }
                        if let Some((i, h)) =
                            d.hunks.iter().enumerate().find(|(_, h)| h.start == off)
                        {
                            hunk = Some(i);
                            source_line = h.line;
                        }
                        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(20_000)])
                            .trim_end_matches('\n')
                            .to_string();
                        // Keep control sequences from repository text out of the terminal.
                        let text = label(std::path::Path::new(&text));
                        let action =
                            hunk.map(|i| RowAction::Hunk(e.path.clone(), staged, i, source_line));
                        self.rows.push(Row {
                            text: format!("    {text}"),
                            action,
                        });
                        if !bytes.starts_with(b"-") && !bytes.starts_with(b"@@") && hunk.is_some() {
                            source_line += 1;
                        }
                        off += bytes.len();
                    }
                    if d.bytes.is_empty() {
                        self.rows.push(Row {
                            text: "    No textual diff; use whole-file staging".into(),
                            action: None,
                        });
                    }
                }
            }
        }
        // magit-insert-stashes and the log sections of magit-status-sections-hook.
        let mut sections: Vec<(Section, String, Vec<Row>)> = vec![];
        if !self.snapshot.extra.stashes.is_empty() {
            let rows = self
                .snapshot
                .extra
                .stashes
                .iter()
                .map(|stash| Row {
                    text: format!(
                        "  {} {}",
                        stash.selector,
                        label(std::path::Path::new(&stash.subject))
                    ),
                    action: Some(RowAction::Stash(stash.clone())),
                })
                .collect();
            sections.push((Section::Stashes, "Stashes".into(), rows));
        }
        for (section, heading, commits) in &self.snapshot.extra.logs {
            let rows = commits
                .iter()
                .map(|c| Row {
                    text: format!(
                        "  {} {}",
                        &c.id[..c.id.len().min(7)],
                        label(std::path::Path::new(&c.subject))
                    ),
                    action: Some(RowAction::Commit(c.id.clone())),
                })
                .collect();
            sections.push((*section, heading.clone(), rows));
        }
        for (section, heading, rows) in sections {
            let shut = self.closed.contains(&section);
            self.rows.push(Row {
                text: String::new(),
                action: None,
            });
            // magit-section-show-child-count, except for recent commits.
            let count = if heading == "Recent commits" {
                String::new()
            } else if section != Section::Stashes && rows.len() >= status::LIMIT {
                format!(" ({}+)", rows.len())
            } else {
                format!(" ({})", rows.len())
            };
            self.rows.push(Row {
                text: format!("{} {heading}{count}", if shut { ">" } else { "v" }),
                action: Some(RowAction::Section(section)),
            });
            if !shut {
                self.rows.extend(rows);
            }
        }
    }
    pub fn selection(&self, action: &Option<RowAction>, fallback: usize) -> usize {
        // A commit can appear in several sections: keep the occurrence nearest
        // the previous position.
        action
            .as_ref()
            .and_then(|a| {
                (0..self.rows.len())
                    .filter(|&i| self.rows[i].action.as_ref() == Some(a))
                    .min_by_key(|&i| i.abs_diff(fallback))
            })
            .unwrap_or(fallback.min(self.rows.len().saturating_sub(1)))
    }
}
/// Intercept the extra prefix and component-local keys before Vim editing.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    if let Mode::Pick(picker) = &ed.mode
        && picker.kind == crate::pick::Kind::MagitMenu
    {
        if k.is(KeyCode::Esc) || k == Key::ctrl('g') || k == Key::ctrl('c') {
            ed.mode = Mode::Normal;
            ed.magit_menu = None;
            ed.vim.pending.clear();
            ed.msg = None;
            return true;
        }
        let entries = menu_entries(ed.magit_menu.unwrap_or('*'));
        if !k.ctrl
            && ed.vim.pending.is_empty()
            && matches!(k.char(), Some('-' | '+' | '=' | ',' | '/'))
        {
            ed.vim.pending = vec![k];
            return true;
        }
        let suffix = if k.is(KeyCode::Enter) {
            picker
                .rows
                .get(picker.sel)
                .and_then(|r| entries.get(r.line))
                .map(|entry| entry.0.to_owned())
        } else if !k.ctrl && !k.alt {
            k.char().map(|c| {
                format!(
                    "{}{c}",
                    match ed.vim.pending.as_slice() {
                        [key] if *key == Key::ch('-') => "-",
                        [key] if *key == Key::ch('+') => "+",
                        [key] if *key == Key::ch('=') => "=",
                        [key] if *key == Key::ch(',') => ",",
                        [key] if *key == Key::ch('/') => "/",
                        _ => "",
                    }
                )
            })
        } else {
            None
        };
        if let Some(suffix) = suffix {
            let menu = ed.magit_menu.unwrap_or('*');
            ed.vim.pending.clear();
            if let Some((_, _, _, action)) = entries.into_iter().find(|(key, ..)| *key == suffix) {
                if let Action::CycleOption(prefix) = action {
                    cycle_choice(ed, arg_menu(menu), prefix);
                    // magit-pull :incompatible --ff-only with rebasing choices.
                    if ed.magit_options.iter().any(
                        |o| matches!(o, MenuOption::Choice(_, "--rebase=", v) if *v != "false"),
                    ) {
                        ed.magit_options.remove(&MenuOption::PullFfOnly);
                    }
                    crate::pick::magit_menu(ed, menu);
                    return true;
                }
                if let Action::ReadOption(prefix) = action {
                    // transient-infix-read: a set option is unset, else read.
                    if ed.magit_values.remove(&(arg_menu(menu), prefix)).is_none() {
                        ed.mode = Mode::Normal;
                        ed.magit_menu = None;
                        prompt(ed, Prompt::OptionValue(menu, prefix));
                    } else {
                        crate::pick::magit_menu(ed, menu);
                    }
                    return true;
                }
                if let Action::ToggleOption(option) = action {
                    if !ed.magit_options.remove(&option) {
                        ed.magit_options.insert(option);
                    }
                    for (a, b) in [
                        (MenuOption::CherryFf, MenuOption::CherryX),
                        (MenuOption::RevertEdit, MenuOption::RevertNoEdit),
                    ] {
                        if option == a {
                            ed.magit_options.remove(&b);
                        }
                        if option == b {
                            ed.magit_options.remove(&a);
                        }
                    }
                    if option == MenuOption::MergeFfOnly {
                        ed.magit_options.remove(&MenuOption::MergeNoFf);
                    }
                    if option == MenuOption::MergeNoFf {
                        ed.magit_options.remove(&MenuOption::MergeFfOnly);
                    }
                    if option == MenuOption::PullFfOnly {
                        ed.magit_options.retain(
                            |o| !matches!(o, MenuOption::Choice(_, "--rebase=", v) if *v != "false"),
                        );
                    }
                    if option == MenuOption::StashAll {
                        ed.magit_options.remove(&MenuOption::StashUntracked);
                    }
                    if option == MenuOption::StashUntracked {
                        ed.magit_options.remove(&MenuOption::StashAll);
                    }
                    if ed.commit_repo.is_some() && option.menu() == 'C' {
                        ed.commit_args = commit_arguments(ed);
                    }
                    crate::pick::magit_menu(ed, menu);
                    return true;
                }
                ed.mode = Mode::Normal;
                ed.magit_menu = None;
                if action == Action::BlameQuit {
                    blame::quit(ed);
                } else if action == Action::BlameCycle {
                    blame::cycle(ed);
                } else if let Action::Menu(menu) = action {
                    open_menu(ed, menu);
                } else {
                    ed.pending_effect = Some(ExEffect::Magit(action));
                }
            } else {
                ed.set_err("unknown Git menu key");
            }
            return true;
        }
        return false;
    }
    if ed.mode != Mode::Normal {
        return false;
    }
    if ed.vim.pending == [Key::ch(' ')] && k.char() == Some('m') {
        ed.vim.pending.clear();
        open_menu(ed, '*');
        return true;
    }
    // magit-diff-while-committing: C-c C-d in a commit message draft.
    if ed.commit_repo.is_some() && ed.magit.is_none() {
        if ed.vim.pending.is_empty() && k == Key::ctrl('c') {
            ed.vim.pending = vec![k];
            return true;
        }
        if ed.vim.pending == [Key::ctrl('c')] {
            ed.vim.pending.clear();
            if k == Key::ctrl('d') {
                ed.pending_effect = Some(ExEffect::Magit(Action::DiffWhileCommitting));
                return true;
            }
            // Not C-c C-d: the key keeps its usual meaning.
            return false;
        }
    }
    if ed.magit.is_none() {
        return false;
    }
    // evil-collection's status jumpers: gz gn gu gs, gfu gfp, gpu gpp.
    if ed.magit.as_ref().is_some_and(|v| v.kind == Kind::Status) && !k.ctrl && !k.alt {
        let target = match (ed.vim.pending.as_slice(), k.char()) {
            ([g], Some(c @ ('f' | 'p'))) if *g == Key::ch('g') => {
                ed.vim.pending.push(Key::ch(c));
                return true;
            }
            ([g], Some('z')) if *g == Key::ch('g') => Some(Section::Stashes),
            ([g], Some('n')) if *g == Key::ch('g') => Some(Section::Untracked),
            ([g], Some('u')) if *g == Key::ch('g') => Some(Section::Unstaged),
            ([g], Some('s')) if *g == Key::ch('g') => Some(Section::Staged),
            ([g, f], Some('u')) if *g == Key::ch('g') && *f == Key::ch('f') => {
                Some(Section::UnpulledUpstream)
            }
            ([g, f], Some('p')) if *g == Key::ch('g') && *f == Key::ch('f') => {
                Some(Section::UnpulledPush)
            }
            ([g, p], Some('u')) if *g == Key::ch('g') && *p == Key::ch('p') => {
                Some(Section::UnpushedUpstream)
            }
            ([g, p], Some('p')) if *g == Key::ch('g') && *p == Key::ch('p') => {
                Some(Section::UnpushedPush)
            }
            ([g, _], _) if *g == Key::ch('g') => {
                ed.vim.pending.clear();
                return true;
            }
            _ => None,
        };
        if let Some(section) = target {
            ed.vim.pending.clear();
            let row = ed.magit.as_ref().and_then(|v| {
                v.rows
                    .iter()
                    .position(|r| r.action == Some(RowAction::Section(section)))
            });
            match row {
                Some(line) => ed.set_cursor(line, 0),
                None => ed.set_msg(format!("Section \"{}\" wasn't found", section.name())),
            }
            return true;
        }
    }
    // magit-section movement (evil-collection): C-j/C-k sections, gj gk ] [
    // M-j M-k siblings, gh parent; z folds.
    if !k.ctrl || matches!(k.char(), Some('j' | 'k')) {
        let pending = ed.vim.pending.clone();
        let mv = match (pending.as_slice(), k.char(), k.ctrl, k.alt) {
            ([], Some('j'), true, false) => {
                // C-j on a file or hunk visits the worktree file (section maps).
                let on_file = ed.magit.as_ref().is_some_and(|v| {
                    matches!(
                        v.action_at(ed.cur.line),
                        Some(RowAction::File(..) | RowAction::Hunk(..))
                    )
                });
                if on_file {
                    ed.pending_effect = Some(ExEffect::Magit(Action::Visit));
                    return true;
                }
                Some(Move::Next)
            }
            ([], Some('k'), true, false) => Some(Move::Prev),
            ([], Some('j'), false, true) | ([], Some(']'), false, false) => Some(Move::NextSibling),
            ([], Some('k'), false, true) | ([], Some('['), false, false) => Some(Move::PrevSibling),
            ([g], Some('j'), false, false) if *g == Key::ch('g') => Some(Move::NextSibling),
            ([g], Some('k'), false, false) if *g == Key::ch('g') => Some(Move::PrevSibling),
            ([g], Some('h'), false, false) if *g == Key::ch('g') => Some(Move::Up),
            _ => None,
        };
        if let Some(mv) = mv {
            ed.vim.pending.clear();
            section_move(ed, mv);
            return true;
        }
        // Fred's Vim has no z commands: z is a prefix here.
        if pending.is_empty() && k.char() == Some('z') && !k.ctrl && !k.alt {
            ed.vim.pending = vec![k];
            return true;
        }
        let fold = match (pending.as_slice(), k.char(), k.ctrl || k.alt) {
            ([z], Some('a'), false) if *z == Key::ch('z') => Some(Action::Toggle),
            ([z], Some('o'), false) if *z == Key::ch('z') => Some(Action::Fold(Fold::Show)),
            ([z], Some('c'), false) if *z == Key::ch('z') => Some(Action::Fold(Fold::Hide)),
            ([z], Some('O'), false) if *z == Key::ch('z') => Some(Action::Fold(Fold::ShowChildren)),
            ([z], Some('C'), false) if *z == Key::ch('z') => Some(Action::Fold(Fold::HideChildren)),
            ([z], Some('r'), false) if *z == Key::ch('z') => Some(Action::Fold(Fold::Level(4))),
            ([z], Some(c @ '1'..='4'), false) if *z == Key::ch('z') => {
                Some(Action::Fold(Fold::Level(c as u8 - b'0')))
            }
            _ => None,
        };
        if let Some(fold) = fold {
            ed.vim.pending.clear();
            ed.pending_effect = Some(ExEffect::Magit(fold));
            return true;
        }
        if pending == [Key::ch('z')] {
            ed.vim.pending.clear();
            return true;
        }
    }
    if ed.vim.pending == [Key::ch('g')] && k.char() == Some('r') {
        ed.vim.pending.clear();
        ed.pending_effect = Some(ExEffect::Magit(Action::Refresh));
        return true;
    }
    // magit-log-move-to-parent (C-c C-n).
    if ed
        .magit
        .as_ref()
        .is_some_and(|v| matches!(v.kind, Kind::Log(..)))
    {
        if ed.vim.pending.is_empty() && k == Key::ctrl('c') {
            ed.vim.pending = vec![k];
            return true;
        }
        if ed.vim.pending == [Key::ctrl('c')] {
            ed.vim.pending.clear();
            if k == Key::ctrl('n') {
                log_move_to_parent(ed);
            }
            return true;
        }
    }
    if !ed.vim.pending.is_empty() {
        return false;
    }
    // magit-diff-less/more/default-context: = + ~ under evil-collection.
    if let Some(Kind::Diff(_, args)) = ed.magit.as_mut().map(|v| &mut v.kind)
        && !k.ctrl
        && matches!(k.char(), Some('=' | '+' | '~'))
    {
        let current = args
            .iter()
            .find_map(|a| a.strip_prefix("-U").and_then(|n| n.parse::<usize>().ok()))
            .unwrap_or(3);
        let next = match k.char() {
            Some('=') => Some(current.saturating_sub(1)),
            Some('+') => Some(current + 1),
            _ => None,
        };
        args.retain(|a| !a.starts_with("-U"));
        if let Some(n) = next {
            args.push(format!("-U{n}"));
        }
        ed.pending_effect = Some(ExEffect::Magit(Action::Refresh));
        return true;
    }
    // magit-log-toggle-commit-limit (=) and -double-commit-limit (+);
    // evil-collection leaves - to revert.
    if let Some(Kind::Log(_, args)) = ed.magit.as_mut().map(|v| &mut v.kind)
        && !k.ctrl
        && matches!(k.char(), Some('=' | '+'))
    {
        let limit = log::limit(args);
        let next = match (k.char(), limit) {
            (Some('='), Some(_)) => None,
            (Some('='), None) => Some(256),
            (_, Some(n)) => Some(n.saturating_mul(2)),
            (_, None) => Some(256),
        };
        // magit-log-set-commit-limit: a limit of 0 or less is no limit.
        let next = next.filter(|n| *n > 0);
        *args = log::with_limit(args, next);
        ed.pending_effect = Some(ExEffect::Magit(Action::Refresh));
        return true;
    }
    if ed
        .magit
        .as_ref()
        .is_some_and(|v| matches!(v.kind, Kind::Stashes | Kind::StashPatch(_)))
    {
        let action = match k.char() {
            Some('a') if !k.ctrl => Some(workflows::StashAction::Apply),
            Some('p') if !k.ctrl => Some(workflows::StashAction::Pop),
            Some('d' | 'x') if !k.ctrl => Some(workflows::StashAction::Drop),
            _ => None,
        };
        if let Some(action) = action {
            ed.pending_effect = Some(ExEffect::Magit(Action::Stash(action)));
            return true;
        }
    }
    let action = match k.code {
        KeyCode::Tab => Some(Action::Toggle),
        KeyCode::Enter => Some(Action::Visit),
        KeyCode::Char('j')
            if k.ctrl
                && ed.magit.as_ref().is_some_and(|v| {
                    matches!(
                        v.kind,
                        Kind::Diff(..) | Kind::Patch(_) | Kind::StashPatch(_)
                    )
                }) =>
        {
            Some(Action::VisitWorktree)
        }
        KeyCode::Char('s') if !k.ctrl => Some(Action::Stage),
        KeyCode::Char('u') if !k.ctrl => Some(Action::Unstage),
        // evil-collection: x discard (magit-delete-thing), - reverse,
        // S stage all modified, U unstage all.
        KeyCode::Char(c @ ('x' | '-' | 'S' | 'U'))
            if !k.ctrl && ed.magit.as_ref().is_some_and(|v| v.kind == Kind::Status) =>
        {
            Some(Action::ApplyOp(match c {
                'x' => apply::Kind::Discard,
                '-' => apply::Kind::Reverse,
                'S' => apply::Kind::StageModified,
                _ => apply::Kind::UnstageAll,
            }))
        }
        KeyCode::Char('q') if !k.ctrl => Some(Action::Return),
        _ => None,
    };
    if let Some(a) = action {
        ed.pending_effect = Some(ExEffect::Magit(a));
        return true;
    }
    false
}
fn log_move_to_parent(ed: &mut Editor) {
    let Some(view) = ed.magit.as_ref() else {
        return;
    };
    let line = ed.cur.line;
    let Some(RowAction::Commit(id)) = view.action_at(line) else {
        return;
    };
    // ponytail: synchronous rev-parse; local and fast, move to the worker if not.
    let parent = view
        .repo
        .read(&["rev-parse", "--verify", "-q", &format!("{id}^1")])
        .map(|o| String::from_utf8_lossy(&o).trim().to_owned());
    let Ok(parent) = parent else {
        return ed.set_err(format!(
            "Parent {}^1 does not exist",
            &id[..id.len().min(8)]
        ));
    };
    let target = (line + 1..view.rows.len())
        .find(|&i| view.rows[i].action.as_ref() == Some(&RowAction::Commit(parent.clone())));
    match target {
        Some(i) => ed.set_cursor(i, 0),
        None => ed.set_err(format!(
            "Parent {} not found.  Try typing + first",
            &parent[..parent.len().min(8)]
        )),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Move {
    Next,
    Prev,
    NextSibling,
    PrevSibling,
    Up,
}
/// A section's depth: headings, then files/commits/stashes/modules, then hunks.
fn level(a: &RowAction) -> u8 {
    match a {
        RowAction::Section(_) => 1,
        RowAction::Hunk(..) => 3,
        _ => 2,
    }
}
/// magit-section-forward/backward/-sibling/up over the rows' sections: a
/// section starts where a row's action differs from the row above.
fn section_move(ed: &mut Editor, mv: Move) {
    let Some(view) = ed.magit.as_ref() else {
        return;
    };
    let rows = &view.rows;
    let start =
        |i: usize| rows[i].action.is_some() && (i == 0 || rows[i - 1].action != rows[i].action);
    let cur = ed.cur.line.min(rows.len().saturating_sub(1));
    // The section the cursor is in: the nearest start at or above it.
    let here = (0..=cur).rev().find(|&i| start(i));
    let lvl = here.and_then(|i| rows[i].action.as_ref()).map_or(1, level);
    let target = match mv {
        Move::Next => (cur + 1..rows.len()).find(|&i| start(i)),
        // magit-section-backward: this section's start, else the previous one.
        Move::Prev => match here {
            Some(h) if h < cur => Some(h),
            _ => (0..cur).rev().find(|&i| start(i)),
        },
        Move::NextSibling => (cur + 1..rows.len())
            .filter(|&i| start(i))
            .take_while(|&i| rows[i].action.as_ref().map_or(0, level) >= lvl)
            .find(|&i| rows[i].action.as_ref().map(level) == Some(lvl)),
        Move::PrevSibling => (0..here.unwrap_or(cur))
            .rev()
            .filter(|&i| start(i))
            .take_while(|&i| rows[i].action.as_ref().map_or(0, level) >= lvl)
            .find(|&i| rows[i].action.as_ref().map(level) == Some(lvl)),
        Move::Up => (0..here.unwrap_or(cur))
            .rev()
            .find(|&i| start(i) && rows[i].action.as_ref().map_or(0, level) < lvl),
    };
    match target {
        Some(i) => ed.set_cursor(i, 0),
        None => ed.set_msg("No more sections"),
    }
}
#[cfg(test)]
mod tests;

fn menu_help(menu: char) -> Option<&'static str> {
    Some(match menu {
        'O' => {
            "Remote: a add  r rename  k remove  p prune branches  P prune refspecs; -f fetch after add"
        }
        'X' => "Reset: b branch  f file  m mixed  s soft  h hard  k keep  i index  w worktree",
        'z' => {
            "Stash: z both  i index  w worktree  x keep index; Snapshot: Z both  I index  W worktree"
        }
        'F' => {
            "File: d diff  l log  b blame  r removal  f reverse  p/n prev/next blob  v goto blob  V goto file  g status"
        }
        'B' => {
            "Blame: b addition  m echo  q quit  c cycle style; in blame n/p chunks  N/P same commit  RET commit"
        }
        'b' => {
            "Branch: b checkout  l local  c new  s spin-off  n create  S spin-out  m rename  x reset  k delete"
        }
        'd' => {
            "Diff: d dwim  r range  p paths  u unstaged  s staged  w worktree  c commit  t stash"
        }
        'p' => {
            "Push: p pushRemote  u upstream  e elsewhere  o other  r refspecs  m matching  T tag  t tags"
        }
        'f' => {
            "Fetch: p pushRemote  u current remote  e elsewhere  a all  o branch  r refspec  m submodules"
        }
        'P' => "Pull: p pushRemote  u upstream  e elsewhere; -r cycles --rebase choices",
        'l' => {
            "Log: l current  o other  h HEAD  u related  L/b/a/R branches, all, reflog objects  B/T matching  m merged; = limit, + more in a log"
        }
        'u' => "Subtree: i import  e export",
        'g' => {
            "Ignore: t toplevel  s subdirectory  p private  g global; w/W skip worktree  u/U assume unchanged"
        }
        '>' => "Sparse checkout: e enable  d disable  r reapply  s set  a add; -i sparse index",
        'y' => {
            "Refs: y HEAD  c current  o other  v commit counts; -c contains  -M/-m merged  -N/-n not merged  -s sort"
        }
        'k' => "Clone: C regular  s shallow  d since  e excluding  > sparse  b bare  m mirror",
        'J' => "Bundle: c create  v verify  l list-heads",
        'j' => "Bundle create: c regular  t tracked  u update tracked",
        'W' => {
            "Patch: c create  w apply patches (am)  a apply plain patch  s save diff  r request pull"
        }
        'K' => "Create patches: c create; =x for upstream's C-m x arguments",
        'a' => "Apply patch: a apply; -i index  -c cached  -3 3way",
        'w' => "Am: m maildir  w patches  a plain patch; applying: w continue  s skip  a abort",
        'I' => {
            "Subtree import: a add  c add commit  m merge  f pull; -P prefix  -m message  -s squash"
        }
        'E' => {
            "Subtree export: p push  s split; -P prefix  -a annotate  -b branch  -o onto  -i ignore joins  -j rejoin"
        }
        'o' => {
            "Submodule: a add  r register  p populate  u update  s sync  d unpopulate  k remove  l list  f fetch"
        }
        'S' => {
            "Shortlog: s since  r range; -n numbered  -s summary  -e email  -g group  -f format  -w wrap"
        }
        't' => "Tag: t tag  r release  k delete  p prune; -a annotate -s sign -e message -f force",
        'C' => "Commit: a amend  e extend  w reword  f fixup",
        'M' => {
            "Merge: m merge  e edit msg  n no commit  a absorb  p preview  s squash  d dissolve; merging: m commit  a abort"
        }
        'r' => {
            "Rebase onto: p pushRemote  u upstream  e elsewhere; i interactive  s subset  m modify  w reword  k remove  f autosquash; rebasing: r continue  s skip  e edit  a abort"
        }
        'x' => {
            "Cherry-pick: A pick  a apply  h harvest  m squash  d donate  n spinout  s spinoff; picking: A continue  s skip  a abort"
        }
        'v' => "Revert: V revert commits  v revert changes; reverting: V continue  s skip  a abort",
        _ => return None,
    })
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CommitMode {
    #[default]
    New,
    Amend(String),
    Reword(String),
}
impl CommitMode {
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::New => None,
            Self::Amend(id) | Self::Reword(id) => Some(id),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    Workflow(Repo, workflows::Operation, Vec<String>),
    DropStash(Repo, workflows::Stash),
    /// magit-init: base directory for a relative answer.
    InitDir(PathBuf),
    /// Confirm initializing inside (or re-initializing) a repository.
    InitConfirm(PathBuf, String),
    /// A chain of prompts: repo, question, arguments, prompts, answers so far.
    Ask(Repo, Question, Vec<String>, Vec<String>, Vec<String>),
    /// A todo line to add below point: its verb (exec, label, reset, merge,
    /// pick) and prompt.
    RebaseLine(&'static str, &'static str),
    /// A transient-option value for (menu, argument prefix); returns to the menu.
    OptionValue(char, &'static str),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    Branch(branch::Op),
    Tag(tag::Op),
    Merge(merge::Op),
    Reset(reset::Op),
    Remote(remote::Op),
    Sequence(sequence::Op),
    Rebase(rebase::Op),
    Commit(commit::Op),
    Stash(stash::Op),
    Worktree(worktree::Op),
    Reflog,
    Notes(notes::Op),
    Bisect(bisect::Op),
    Log(log::Op),
    Submodule(submodule::Op),
    Subtree(subtree::Op),
    Patch(patch::Op),
    Bundle(bundle::Op),
    Clone(clone::Op),
    Refs(refs::Op),
    Ignore(ignore::Op),
    Configure(configure::Op),
    Apply(apply::Op),
    Net(network::Op),
    Diff(diff::Op),
    FindFile,
    File(blob::FileOp),
}
pub fn prompt(ed: &mut Editor, question: Prompt) {
    let text = match &question {
        Prompt::Workflow(_, operation, _) => operation.prompt().unwrap_or("").to_owned(),
        Prompt::Ask(_, _, _, prompts, answers) => prompts[answers.len()].clone(),
        Prompt::InitDir(_) => "Create repository in: ".into(),
        Prompt::RebaseLine(_, prompt) => (*prompt).into(),
        Prompt::OptionValue(_, prefix) => (*prefix).into(),
        Prompt::InitConfirm(_, question) => question.clone(),
        Prompt::DropStash(_, stash) => format!(
            "Drop {} ({})? Type yes: ",
            stash.selector,
            &stash.id[..stash.id.len().min(8)]
        ),
    };
    ed.magit_prompt = Some(question);
    ed.open_cmdline('=', "");
    if let Mode::Command(cl) = &mut ed.mode {
        cl.prompt = text;
    }
}
pub fn answer(ed: &mut Editor, text: &str) {
    match ed.magit_prompt.take() {
        Some(Prompt::Workflow(repo, operation, args)) => {
            ed.pending_effect = Some(ExEffect::Magit(Action::Submit(
                repo,
                operation,
                text.into(),
                args,
            )))
        }
        Some(Prompt::DropStash(repo, stash)) if text == "yes" => {
            ed.pending_effect = Some(ExEffect::Magit(Action::DropStash(repo, stash)))
        }
        Some(Prompt::DropStash(..)) => ed.set_msg("Stash drop cancelled"),
        Some(Prompt::InitDir(base)) => {
            let text = text.trim();
            let dir = match text.strip_prefix("~/").or((text == "~").then_some("")) {
                Some(rest) => std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join(rest),
                None => base.join(text),
            };
            ed.pending_effect = Some(ExEffect::Magit(Action::InitDir(dir, false)));
        }
        Some(Prompt::InitConfirm(dir, _)) if matches!(text.trim(), "y" | "yes") => {
            ed.pending_effect = Some(ExEffect::Magit(Action::InitDir(dir, true)))
        }
        Some(Prompt::InitConfirm(..)) => ed.set_msg("Abort"),
        Some(Prompt::RebaseLine(verb, _)) => rebase::insert_line(ed, verb, text),
        Some(Prompt::OptionValue(menu, prefix)) => {
            if !text.is_empty() {
                ed.magit_values
                    .insert((arg_menu(menu), prefix), text.to_owned());
            }
            // Return to the open transient, not a fresh one seeded from the buffer.
            ed.magit_menu = Some(menu);
            crate::pick::magit_menu(ed, menu);
        }
        Some(Prompt::Ask(repo, question, args, prompts, mut answers)) => {
            answers.push(text.trim().to_owned());
            if answers.len() < prompts.len() {
                prompt(ed, Prompt::Ask(repo, question, args, prompts, answers));
            } else {
                ed.pending_effect = Some(ExEffect::Magit(Action::Answered(
                    repo, question, answers, args,
                )));
            }
        }
        None => (),
    }
}

pub(crate) fn open_menu(ed: &mut Editor, menu: char) {
    if menu == 'C' && ed.commit_repo.is_some() {
        sync_commit_options(ed);
    }
    // magit-prefix-use-buffer-arguments: a log buffer's own arguments seed its menu.
    if menu == 'l'
        && let Some(Kind::FileLog(_, follow)) = ed.magit.as_ref().map(|v| &v.kind)
    {
        if *follow {
            ed.magit_options.insert(MenuOption::LogFollow);
        } else {
            ed.magit_options.remove(&MenuOption::LogFollow);
        }
    }
    // magit-log-buffer arguments, else the default ("-n256" "--graph" "--decorate").
    if menu == 'l' {
        if let Some(Kind::Log(_, args)) = ed.magit.as_ref().map(|v| &v.kind) {
            let args = args.clone();
            let follow = ed.magit_options.contains(&MenuOption::LogFollow);
            set_menu_arguments(ed, 'l', &args);
            if follow {
                ed.magit_options.insert(MenuOption::LogFollow);
            }
        } else if ed.magit_seeded.insert('l') {
            ed.magit_values.insert(('l', "-n"), "256".into());
            ed.magit_options.insert(MenuOption::Switch('l', "--graph"));
            ed.magit_options
                .insert(MenuOption::Switch('l', "--decorate"));
        }
    }
    // magit-show-refs in a refs buffer uses the buffer's arguments.
    if menu == 'y'
        && let Some(Kind::Refs(_, args, _)) = ed.magit.as_ref().map(|v| &v.kind)
    {
        let args = args.clone();
        set_menu_arguments(ed, 'y', &args);
    }
    // magit-commit :value '("--verbose").
    if menu == 'C' && ed.commit_repo.is_none() && ed.magit_seeded.insert('C') {
        ed.magit_options
            .insert(MenuOption::Switch('C', "--verbose"));
    }
    // magit-am :value '("--3way").
    if menu == 'w' && ed.magit_seeded.insert('w') {
        ed.magit_options.insert(MenuOption::Switch('w', "--3way"));
    }
    // magit-shortlog :value '("--numbered" "--summary").
    if menu == 'S' && ed.magit_seeded.insert('S') {
        ed.magit_options
            .insert(MenuOption::Switch('S', "--numbered"));
        ed.magit_options
            .insert(MenuOption::Switch('S', "--summary"));
    }
    // magit-rebase :value '("--autostash").
    if menu == 'r' && ed.magit_seeded.insert('r') {
        ed.magit_options.insert(MenuOption::RebaseAutostash);
    }
    // magit-cherry-pick :value '("--ff") and magit-revert :value '("--edit").
    if menu == 'x' && ed.magit_seeded.insert('x') {
        ed.magit_options.insert(MenuOption::CherryFf);
    }
    if menu == 'v' && ed.magit_seeded.insert('v') {
        ed.magit_options.insert(MenuOption::RevertEdit);
    }
    // magit-remote :value '("-f").
    if menu == 'O' && ed.magit_seeded.insert('O') {
        ed.magit_options.insert(MenuOption::RemoteFetch);
    }
    // magit-blame :value '("-w").
    if menu == 'B' && ed.magit_seeded.insert('B') {
        ed.magit_options.insert(MenuOption::BlameWhitespace);
    }
    if menu == 'd' || menu == 'D' {
        let args = match ed.magit.as_ref().map(|v| &v.kind) {
            // A buffer's file limit stays with that buffer.
            Some(Kind::Diff(_, args)) => Some(
                args.iter()
                    .filter(|a| !a.starts_with("-- ") && *a != "--cached")
                    .cloned()
                    .collect(),
            ),
            // magit-diff-mode's default arguments, applied once per buffer.
            _ if ed.magit_seeded.insert('d') => Some(vec!["--stat".into(), "--no-ext-diff".into()]),
            _ => None,
        };
        if let Some(args) = args {
            set_menu_arguments(ed, 'd', &args);
        }
    }
    ed.magit_menu = Some(menu);
    crate::pick::magit_menu(ed, menu);
    if let Some(help) = menu_help(menu) {
        ed.set_msg(help);
    } else {
        ed.set_msg(HELP);
    }
}
pub fn menu_entries(menu: char) -> Vec<(&'static str, &'static str, &'static str, Action)> {
    use Action::*;
    use workflows::{Operation::*, StashAction};
    match menu {
        '*' => vec![
            ("s", "Inspect", "Status", Status),
            ("l", "Inspect", "Log menu", Menu('l')),
            ("L", "Inspect", "Current file log", FileLog),
            ("d", "Inspect", "Diff", Menu('d')),
            ("i", "Repository", "Init", Init),
            ("F", "Inspect", "File dispatch", Menu('F')),
            ("X", "History", "Reset", Menu('X')),
            ("Z", "Repository", "Worktree", Menu('Y')),
            ("T", "Inspect", "Notes", Menu('N')),
            ("Y", "Inspect", "Cherries", LogOp(log::Op::Cherry)),
            ("y", "Inspect", "Show Refs", Menu('y')),
            ("D", "Inspect", "Diff (change)", Menu('D')),
            // Upstream's i is the user's init key: gitignore takes upstream's I.
            ("I", "Repository", "Ignore", Menu('g')),
            (">", "Repository", "Sparse checkout", Menu('>')),
            ("o", "Repository", "Submodules", Menu('o')),
            ("O", "Repository", "Subtrees", Menu('u')),
            ("W", "Repository", "Patches", Menu('W')),
            // Upstream's B is the user's blame key, so bisect lives on G.
            ("G", "History", "Bisect", Menu('G')),
            ("b", "Branch", "Branch operations", Menu('b')),
            ("B", "Inspect", "Blame", Menu('B')),
            ("c", "Commit", "Commit menu", Menu('C')),
            ("C", "Repository", "Clone", Menu('k')),
            ("&", "Repository", "Bundle (M-x upstream)", Menu('J')),
            ("w", "Repository", "Apply patches", Menu('w')),
            ("p", "Network", "Push", Menu('p')),
            ("P", "Network", "Pull", Menu('P')),
            ("f", "Network", "Fetch", Menu('f')),
            ("z", "Change", "Stash", Menu('z')),
            ("t", "Tag", "Tags", Menu('t')),
            // magit-dispatch keys: m merge, M remote.
            ("m", "History", "Merge", Menu('M')),
            ("M", "Network", "Remote", Menu('O')),
            ("R", "History", "Rebase", Menu('r')),
            ("r", "History", "Revert", Menu('v')),
            ("x", "History", "Cherry-pick", Menu('x')),
            ("v", "History", "Revert", Menu('v')),
            // magit-dispatch keys.
            ("A", "History", "Cherry-pick", Menu('x')),
            ("V", "History", "Revert", Menu('v')),
        ],
        'p' => {
            use network::Op::*;
            vec![
                (
                    "-f",
                    "Arguments",
                    "Force with lease",
                    ToggleOption(MenuOption::PushForceWithLease),
                ),
                (
                    "-F",
                    "Arguments",
                    "Force",
                    ToggleOption(MenuOption::PushForce),
                ),
                (
                    "-h",
                    "Arguments",
                    "Disable hooks",
                    ToggleOption(MenuOption::PushNoVerify),
                ),
                (
                    "-n",
                    "Arguments",
                    "Dry run",
                    ToggleOption(MenuOption::PushDryRun),
                ),
                (
                    "-u",
                    "Arguments",
                    "Set upstream",
                    ToggleOption(MenuOption::PushSetUpstream),
                ),
                (
                    "-T",
                    "Arguments",
                    "Include all tags",
                    ToggleOption(MenuOption::PushTags),
                ),
                (
                    "-t",
                    "Arguments",
                    "Include related annotated tags",
                    ToggleOption(MenuOption::PushFollowTags),
                ),
                ("p", "Push current to", "pushRemote", Net(PushRemote)),
                ("u", "Push current to", "@{upstream}", Net(PushUpstream)),
                ("e", "Push current to", "elsewhere", Net(PushElsewhere)),
                ("o", "Push", "another branch", Net(PushOther)),
                ("r", "Push", "explicit refspecs", Net(PushRefspecs)),
                ("m", "Push", "matching branches", Net(PushMatching)),
                ("T", "Push", "a tag", Net(PushTag)),
                ("t", "Push", "all tags", Net(PushTags)),
            ]
        }
        'f' => {
            use network::Op::*;
            vec![
                (
                    "-p",
                    "Arguments",
                    "Prune deleted branches",
                    ToggleOption(MenuOption::FetchPrune),
                ),
                (
                    "-t",
                    "Arguments",
                    "Fetch all tags",
                    ToggleOption(MenuOption::FetchTags),
                ),
                (
                    "-F",
                    "Arguments",
                    "Force",
                    ToggleOption(MenuOption::FetchForce),
                ),
                ("p", "Fetch from", "pushRemote", Net(FetchRemote)),
                ("u", "Fetch from", "current remote", Net(FetchUpstream)),
                ("e", "Fetch from", "elsewhere", Net(FetchElsewhere)),
                ("a", "Fetch from", "all remotes", Net(FetchAll)),
                ("o", "Fetch", "another branch", Net(FetchBranch)),
                ("r", "Fetch", "explicit refspec", Net(FetchRefspec)),
                ("m", "Fetch", "submodules", Net(FetchModules)),
            ]
        }
        'P' => {
            use network::Op::*;
            vec![
                (
                    "-f",
                    "Arguments",
                    "Fast-forward only",
                    ToggleOption(MenuOption::PullFfOnly),
                ),
                (
                    "-r",
                    "Arguments",
                    "Rebase local commits",
                    CycleOption("--rebase="),
                ),
                (
                    "-F",
                    "Arguments",
                    "Force",
                    ToggleOption(MenuOption::PullForce),
                ),
                ("p", "Pull into current from", "pushRemote", Net(PullRemote)),
                (
                    "u",
                    "Pull into current from",
                    "@{upstream}",
                    Net(PullUpstream),
                ),
                (
                    "e",
                    "Pull into current from",
                    "elsewhere",
                    Net(PullElsewhere),
                ),
            ]
        }
        'd' => {
            use diff::Op::*;
            vec![
                (
                    "-b",
                    "Limit arguments",
                    "Ignore whitespace changes",
                    ToggleOption(MenuOption::DiffIgnoreSpace),
                ),
                (
                    "-w",
                    "Limit arguments",
                    "Ignore all whitespace",
                    ToggleOption(MenuOption::DiffIgnoreAllSpace),
                ),
                (
                    "-i",
                    "Limit arguments",
                    "Ignore submodules",
                    CycleOption("--ignore-submodules="),
                ),
                (
                    "-W",
                    "Context arguments",
                    "Show surrounding functions",
                    ToggleOption(MenuOption::DiffFunctionContext),
                ),
                (
                    "-A",
                    "Tune arguments",
                    "Diff algorithm",
                    CycleOption("--diff-algorithm="),
                ),
                (
                    "-X",
                    "Tune arguments",
                    "Diff merges",
                    CycleOption("--diff-merges="),
                ),
                (
                    "-M",
                    "Tune arguments",
                    "Detect renames",
                    ToggleOption(MenuOption::DiffRenames),
                ),
                (
                    "-x",
                    "Tune arguments",
                    "Disallow external diff drivers",
                    ToggleOption(MenuOption::DiffNoExt),
                ),
                (
                    "-s",
                    "Tune arguments",
                    "Show stats",
                    ToggleOption(MenuOption::DiffStat),
                ),
                (
                    "=g",
                    "Tune arguments",
                    "Show signature",
                    ToggleOption(MenuOption::DiffSignature),
                ),
                (
                    "-D",
                    "Limit arguments",
                    "Omit preimage for deletes",
                    ToggleOption(MenuOption::Switch('d', "--irreversible-delete")),
                ),
                ("-U", "Context arguments", "Context lines", ReadOption("-U")),
                (
                    "-C",
                    "Tune arguments",
                    "Detect copies",
                    ToggleOption(MenuOption::Switch('d', "-C")),
                ),
                (
                    "-H",
                    "Tune arguments",
                    "Detect copies if source unmodified",
                    ToggleOption(MenuOption::Switch('d', "--find-copies-harder")),
                ),
                (
                    "-R",
                    "Tune arguments",
                    "Reverse sides",
                    ToggleOption(MenuOption::Switch('d', "-R")),
                ),
                (
                    "=m",
                    "Tune arguments",
                    "Color moved lines",
                    CycleOption("--color-moved="),
                ),
                (
                    "=w",
                    "Tune arguments",
                    "Whitespace for moved lines",
                    CycleOption("--color-moved-ws="),
                ),
                ("--", "Limit arguments", "Limit to files", ReadOption("-- ")),
                ("d", "Actions", "Dwim", Diff(Dwim)),
                ("r", "Actions", "Diff range", Diff(Range)),
                ("p", "Actions", "Diff paths", Diff(Paths)),
                ("u", "Actions", "Diff unstaged", Diff(Unstaged)),
                ("s", "Actions", "Diff staged", Diff(Staged)),
                ("w", "Actions", "Diff worktree", Diff(Worktree)),
                ("c", "Actions", "Show commit", Diff(ShowCommit)),
                ("t", "Actions", "Show stash", Diff(ShowStash)),
            ]
        }
        // magit-diff-refresh: the same arguments (stored as menu d's), then
        // refresh this buffer with them.
        'D' => {
            let mut v: Vec<_> = menu_entries('d')
                .into_iter()
                .filter(|e| e.1 != "Actions")
                .collect();
            v.extend([
                ("g", "Refresh", "buffer", DiffRefresh(diff::Refresh::Buffer)),
                (
                    "r",
                    "Do",
                    "switch range type",
                    DiffRefresh(diff::Refresh::SwitchRange),
                ),
                (
                    "f",
                    "Do",
                    "flip revisions",
                    DiffRefresh(diff::Refresh::Flip),
                ),
            ]);
            v
        }
        'l' => {
            use log::Op as L;
            let sw = |a| ToggleOption(MenuOption::Switch('l', a));
            vec![
                (
                    "-n",
                    "Commit limiting",
                    "Limit number of commits",
                    ReadOption("-n"),
                ),
                (
                    "-A",
                    "Commit limiting",
                    "Limit to author",
                    ReadOption("--author="),
                ),
                (
                    "=s",
                    "Commit limiting",
                    "Limit to commits since",
                    ReadOption("--since="),
                ),
                (
                    "=u",
                    "Commit limiting",
                    "Limit to commits until",
                    ReadOption("--until="),
                ),
                (
                    "-F",
                    "Commit limiting",
                    "Search messages",
                    ReadOption("--grep="),
                ),
                (
                    "-i",
                    "Commit limiting",
                    "Search case-insensitive",
                    sw("--regexp-ignore-case"),
                ),
                (
                    "-I",
                    "Commit limiting",
                    "Invert search pattern",
                    sw("--invert-grep"),
                ),
                ("-G", "Commit limiting", "Search changes", ReadOption("-G")),
                (
                    "-S",
                    "Commit limiting",
                    "Search occurrences",
                    ReadOption("-S"),
                ),
                (
                    "-L",
                    "Commit limiting",
                    "Trace line evolution",
                    ReadOption("-L"),
                ),
                ("=M", "Commit limiting", "Only merges", sw("--merges")),
                ("=m", "Commit limiting", "Omit merges", sw("--no-merges")),
                (
                    "=p",
                    "Commit limiting",
                    "First parent",
                    sw("--first-parent"),
                ),
                (
                    "-D",
                    "History simplification",
                    "Simplify by decoration",
                    sw("--simplify-by-decoration"),
                ),
                (
                    "-f",
                    "History simplification",
                    "Follow renames when showing single-file log",
                    ToggleOption(MenuOption::LogFollow),
                ),
                (
                    "/s",
                    "History simplification",
                    "Only commits changing given paths",
                    sw("--sparse"),
                ),
                (
                    "/d",
                    "History simplification",
                    "Only selected commits plus meaningful history",
                    sw("--dense"),
                ),
                (
                    "/a",
                    "History simplification",
                    "Only commits existing directly on ancestry path",
                    sw("--ancestry-path"),
                ),
                (
                    "/f",
                    "History simplification",
                    "Do not prune history",
                    sw("--full-history"),
                ),
                (
                    "/m",
                    "History simplification",
                    "Prune some history",
                    sw("--simplify-merges"),
                ),
                (
                    "-o",
                    "Commit ordering",
                    "Order commits by",
                    CycleOption("--"),
                ),
                ("-r", "Commit ordering", "Reverse order", sw("--reverse")),
                ("-g", "Formatting", "Show graph", sw("--graph")),
                ("-c", "Formatting", "Show graph in color", sw("--color")),
                ("-d", "Formatting", "Show refnames", sw("--decorate")),
                (
                    "=S",
                    "Formatting",
                    "Show signatures",
                    sw("--show-signature"),
                ),
                ("-h", "Formatting", "Show header", sw("++header")),
                ("-p", "Formatting", "Show diffs", sw("--patch")),
                ("-s", "Formatting", "Show diffstats", sw("--stat")),
                ("l", "Log", "current", Log),
                ("o", "Log", "other", LogOp(L::Other)),
                ("h", "Log", "HEAD", LogHead),
                ("u", "Log", "related", LogOp(L::Related)),
                ("L", "Log", "local branches", LogOp(L::LocalBranches)),
                ("b", "Log", "all branches", LogOp(L::AllBranches)),
                ("a", "Log", "all references", LogOp(L::All)),
                ("R", "Log", "reflog objects", LogOp(L::Reflog)),
                ("B", "Log", "matching branches", LogOp(L::MatchingBranches)),
                ("T", "Log", "matching tags", LogOp(L::MatchingTags)),
                ("m", "Log", "merged", LogOp(L::Merged)),
                ("r", "Reflog", "current", Reflog(Some(String::new()))),
                ("O", "Reflog", "other", Reflog(None)),
                ("H", "Reflog", "HEAD", Reflog(Some("HEAD".into()))),
                ("s", "Other", "shortlog", Menu('S')),
            ]
        }
        'o' => {
            use submodule::Op as O;
            let sw = |a| ToggleOption(MenuOption::Switch('o', a));
            vec![
                ("-f", "Arguments", "Force", sw("--force")),
                ("-r", "Arguments", "Recursive", sw("--recursive")),
                ("-N", "Arguments", "Do not fetch", sw("--no-fetch")),
                ("-C", "Arguments", "Checkout tip", sw("--checkout")),
                ("-R", "Arguments", "Rebase onto tip", sw("--rebase")),
                ("-M", "Arguments", "Merge tip", sw("--merge")),
                ("-U", "Arguments", "Use upstream tip", sw("--remote")),
                (
                    "a",
                    "One module actions",
                    "Add            git submodule add [--force]",
                    Submodule(O::Add),
                ),
                (
                    "r",
                    "One module actions",
                    "Register       git submodule init",
                    Submodule(O::Register),
                ),
                (
                    "p",
                    "One module actions",
                    "Populate       git submodule update --init [--recursive]",
                    Submodule(O::Populate),
                ),
                (
                    "u",
                    "One module actions",
                    "Update         git submodule update [--force] [--no-fetch] [--remote] [--recursive] [--checkout|--rebase|--merge]",
                    Submodule(O::Update),
                ),
                (
                    "s",
                    "One module actions",
                    "Synchronize    git submodule sync [--recursive]",
                    Submodule(O::Synchronize),
                ),
                (
                    "d",
                    "One module actions",
                    "Unpopulate     git submodule deinit [--force]",
                    Submodule(O::Unpopulate),
                ),
                ("k", "One module actions", "Remove", Submodule(O::Remove)),
                (
                    "l",
                    "Populated modules actions",
                    "List modules",
                    Submodule(O::List),
                ),
                (
                    "f",
                    "Populated modules actions",
                    "Fetch modules",
                    Net(network::Op::FetchModules),
                ),
            ]
        }
        'k' => {
            use clone::Op as K;
            let sw = |a| ToggleOption(MenuOption::Switch('k', a));
            vec![
                (
                    "-B",
                    "Fetch arguments",
                    "Clone a single branch",
                    sw("--single-branch"),
                ),
                (
                    "-n",
                    "Fetch arguments",
                    "Do not clone tags",
                    sw("--no-tags"),
                ),
                (
                    "-S",
                    "Fetch arguments",
                    "Clones submodules",
                    sw("--recurse-submodules"),
                ),
                ("-l", "Fetch arguments", "Do not optimize", sw("--no-local")),
                (
                    "-o",
                    "Setup arguments",
                    "Set name of remote",
                    ReadOption("--origin="),
                ),
                (
                    "-b",
                    "Setup arguments",
                    "Set HEAD branch",
                    ReadOption("--branch="),
                ),
                (
                    "-f",
                    "Setup arguments",
                    "Filter some objects",
                    ReadOption("--filter="),
                ),
                (
                    "-g",
                    "Setup arguments",
                    "Separate git directory",
                    ReadOption("--separate-git-dir="),
                ),
                (
                    "-t",
                    "Setup arguments",
                    "Use template directory",
                    ReadOption("--template="),
                ),
                (
                    "-s",
                    "Local sharing arguments",
                    "Share objects",
                    sw("--shared"),
                ),
                (
                    "-h",
                    "Local sharing arguments",
                    "Do not use hardlinks",
                    sw("--no-hardlinks"),
                ),
                ("C", "Clone", "regular", Clone(K::Regular)),
                ("s", "Clone", "shallow", Clone(K::Shallow)),
                ("d", "Clone", "shallow since date", Clone(K::ShallowSince)),
                ("e", "Clone", "shallow excluding", Clone(K::ShallowExclude)),
                (">", "Clone", "sparse checkout", Clone(K::Sparse)),
                ("b", "Clone", "bare", Clone(K::Bare)),
                ("m", "Clone", "mirror", Clone(K::Mirror)),
            ]
        }
        'y' => vec![
            ("-c", "Arguments", "Contains", ReadOption("--contains=")),
            ("-M", "Arguments", "Merged", ReadOption("--merged=")),
            (
                "-m",
                "Arguments",
                "Merged to HEAD",
                ToggleOption(MenuOption::Switch('y', "--merged")),
            ),
            ("-N", "Arguments", "Not merged", ReadOption("--no-merged=")),
            (
                "-n",
                "Arguments",
                "Not merged to HEAD",
                ToggleOption(MenuOption::Switch('y', "--no-merged")),
            ),
            ("-s", "Arguments", "Sort", CycleOption("--sort=")),
            (
                "y",
                "Actions",
                "Show refs, comparing them with HEAD",
                Refs(refs::Op::Head),
            ),
            (
                "c",
                "Actions",
                "Show refs, comparing them with current branch",
                Refs(refs::Op::Current),
            ),
            (
                "o",
                "Actions",
                "Show refs, comparing them with other branch",
                Refs(refs::Op::Other),
            ),
            (
                "v",
                "Actions",
                "Change verbosity (commit counts)",
                Refs(refs::Op::Count),
            ),
        ],
        'g' => {
            use ignore::Op as G;
            vec![
                (
                    "t",
                    "Gitignore",
                    "shared at toplevel (.gitignore)",
                    Ignore(G::Topdir),
                ),
                (
                    "s",
                    "Gitignore",
                    "shared in subdirectory (path/to/.gitignore)",
                    Ignore(G::Subdir),
                ),
                (
                    "p",
                    "Gitignore",
                    "privately (.git/info/exclude)",
                    Ignore(G::Gitdir),
                ),
                (
                    "g",
                    "Gitignore",
                    "privately for all repositories (core.excludesFile)",
                    Ignore(G::System),
                ),
                (
                    "w",
                    "Skip worktree",
                    "do skip worktree",
                    Ignore(G::SkipWorktree),
                ),
                (
                    "W",
                    "Skip worktree",
                    "do not skip worktree",
                    Ignore(G::NoSkipWorktree),
                ),
                (
                    "u",
                    "Assume unchanged",
                    "do assume unchanged",
                    Ignore(G::AssumeUnchanged),
                ),
                (
                    "U",
                    "Assume unchanged",
                    "do not assume unchanged",
                    Ignore(G::NoAssumeUnchanged),
                ),
            ]
        }
        '>' => {
            use ignore::Op as G;
            vec![
                (
                    "-i",
                    "Arguments for enabling",
                    "Use sparse index",
                    ToggleOption(MenuOption::Switch('>', "--sparse-index")),
                ),
                (
                    "e",
                    "Actions",
                    "Enable sparse checkout",
                    Ignore(G::SparseEnable),
                ),
                (
                    "d",
                    "Actions",
                    "Disable sparse checkout",
                    Ignore(G::SparseDisable),
                ),
                ("r", "Actions", "Reapply rules", Ignore(G::SparseReapply)),
                ("s", "Actions", "Set directories", Ignore(G::SparseSet)),
                ("a", "Actions", "Add directories", Ignore(G::SparseAdd)),
            ]
        }
        'J' => vec![
            ("c", "Actions", "create", Menu('j')),
            ("v", "Actions", "verify", Bundle(bundle::Op::Verify)),
            ("l", "Actions", "list-heads", Bundle(bundle::Op::ListHeads)),
        ],
        'j' => vec![
            (
                "-a",
                "Arguments",
                "Include all refs",
                ToggleOption(MenuOption::Switch('j', "--all")),
            ),
            (
                "-b",
                "Arguments",
                "Include branches",
                ReadOption("--branches="),
            ),
            ("-t", "Arguments", "Include tags", ReadOption("--tags=")),
            (
                "-r",
                "Arguments",
                "Include remotes",
                ReadOption("--remotes="),
            ),
            ("-g", "Arguments", "Include refs", ReadOption("--glob=")),
            ("-e", "Arguments", "Exclude refs", ReadOption("--exclude=")),
            (
                "-n",
                "Arguments",
                "Limit number of commits",
                ReadOption("-n"),
            ),
            (
                "=s",
                "Arguments",
                "Limit to commits since",
                ReadOption("--since="),
            ),
            (
                "=u",
                "Arguments",
                "Limit to commits until",
                ReadOption("--until="),
            ),
            (
                "c",
                "Actions",
                "create regular bundle",
                Bundle(bundle::Op::Create),
            ),
            (
                "t",
                "Actions",
                "create tracked bundle",
                Bundle(bundle::Op::CreateTracked),
            ),
            (
                "u",
                "Actions",
                "update tracked bundle",
                Bundle(bundle::Op::UpdateTracked),
            ),
        ],
        'W' => vec![
            ("c", "Actions", "Create patches", Menu('K')),
            ("w", "Actions", "Apply patches", Menu('w')),
            ("a", "Actions", "Apply plain patch", Menu('a')),
            ("s", "Actions", "Save diff as patch", Patch(patch::Op::Save)),
            (
                "r",
                "Actions",
                "Request pull",
                Patch(patch::Op::RequestPull),
            ),
        ],
        // magit-patch-create: upstream's "C-m x" keys are "=x" here.
        'K' => {
            let sw = |a| ToggleOption(MenuOption::Switch('K', a));
            vec![
                (
                    "=R",
                    "Mail arguments",
                    "In reply to",
                    ReadOption("--in-reply-to="),
                ),
                (
                    "=s",
                    "Mail arguments",
                    "Thread style",
                    CycleOption("--thread="),
                ),
                ("=f", "Mail arguments", "From", ReadOption("--from=")),
                ("=t", "Mail arguments", "To", ReadOption("--to=")),
                ("=c", "Mail arguments", "CC", ReadOption("--cc=")),
                (
                    "=b",
                    "Patch arguments",
                    "Insert base commit",
                    ReadOption("--base="),
                ),
                (
                    "=v",
                    "Patch arguments",
                    "Reroll count",
                    ReadOption("--reroll-count="),
                ),
                (
                    "=i",
                    "Patch arguments",
                    "Insert interdiff",
                    ReadOption("--interdiff="),
                ),
                (
                    "=d",
                    "Patch arguments",
                    "Insert range-diff",
                    ReadOption("--range-diff="),
                ),
                (
                    "=p",
                    "Patch arguments",
                    "Subject Prefix",
                    ReadOption("--subject-prefix="),
                ),
                ("=r", "Patch arguments", "RFC subject prefix", sw("--rfc")),
                (
                    "=l",
                    "Patch arguments",
                    "Add cover letter",
                    sw("--cover-letter"),
                ),
                (
                    "=D",
                    "Patch arguments",
                    "Use branch description",
                    CycleOption("--cover-from-description="),
                ),
                (
                    "=n",
                    "Patch arguments",
                    "Insert commentary from notes",
                    ReadOption("--notes="),
                ),
                (
                    "=o",
                    "Patch arguments",
                    "Output directory",
                    ReadOption("--output-directory="),
                ),
                ("-U", "Diff arguments", "Context lines", ReadOption("-U")),
                ("-M", "Diff arguments", "Detect renames", sw("-M")),
                ("-C", "Diff arguments", "Detect copies", sw("-C")),
                (
                    "-A",
                    "Diff arguments",
                    "Diff algorithm",
                    CycleOption("--diff-algorithm="),
                ),
                (
                    "-b",
                    "Diff arguments",
                    "Ignore whitespace changes",
                    sw("--ignore-space-change"),
                ),
                (
                    "-w",
                    "Diff arguments",
                    "Ignore all whitespace",
                    sw("--ignore-all-space"),
                ),
                ("c", "Actions", "Create patches", Patch(patch::Op::Create)),
            ]
        }
        'a' => {
            let sw = |a| ToggleOption(MenuOption::Switch('a', a));
            vec![
                ("-i", "Arguments", "Also apply to index", sw("--index")),
                ("-c", "Arguments", "Only apply to index", sw("--cached")),
                ("-3", "Arguments", "Fall back on 3way merge", sw("--3way")),
                ("a", "Actions", "Apply patch", Patch(patch::Op::Apply)),
            ]
        }
        // magit-am: w/a are continue/abort and s skips while applying.
        'w' => {
            let sw = |a| ToggleOption(MenuOption::Switch('w', a));
            vec![
                ("-3", "Arguments", "Fall back on 3way merge", sw("--3way")),
                (
                    "-R",
                    "Arguments",
                    "Reject only failed hunks",
                    sw("--reject"),
                ),
                (
                    "-p",
                    "Arguments",
                    "Remove leading slashes from paths",
                    ReadOption("-p"),
                ),
                (
                    "-c",
                    "Arguments",
                    "Remove text before scissors line",
                    sw("--scissors"),
                ),
                (
                    "-k",
                    "Arguments",
                    "Inhibit removal of email cruft",
                    sw("--keep"),
                ),
                (
                    "-b",
                    "Arguments",
                    "Limit removal of email cruft",
                    sw("--keep-non-patch"),
                ),
                (
                    "-d",
                    "Arguments",
                    "Use author date as committer date",
                    sw("--committer-date-is-author-date"),
                ),
                (
                    "-t",
                    "Arguments",
                    "Use current time as author date",
                    sw("--ignore-date"),
                ),
                (
                    "-S",
                    "Arguments",
                    "Sign using gpg",
                    ReadOption("--gpg-sign="),
                ),
                (
                    "-s",
                    "Arguments",
                    "Add Signed-off-by lines",
                    sw("--signoff"),
                ),
                ("m", "Apply", "maildir", Patch(patch::Op::AmMaildir)),
                (
                    "w",
                    "Apply",
                    "patches / continue",
                    Patch(patch::Op::AmPatches),
                ),
                (
                    "a",
                    "Apply",
                    "plain patch / abort",
                    Patch(patch::Op::AmApply),
                ),
                ("s", "Actions", "Skip", Patch(patch::Op::AmSkip)),
            ]
        }
        'u' => vec![
            ("i", "Subtree actions", "Import", Menu('I')),
            ("e", "Subtree actions", "Export", Menu('E')),
        ],
        'I' => {
            use subtree::Op as U;
            vec![
                ("-P", "Arguments", "Prefix", ReadOption("--prefix=")),
                ("-m", "Arguments", "Message", ReadOption("--message=")),
                (
                    "-s",
                    "Arguments",
                    "Squash",
                    ToggleOption(MenuOption::Switch('I', "--squash")),
                ),
                ("a", "Subtree import actions", "Add", Subtree(U::Add)),
                (
                    "c",
                    "Subtree import actions",
                    "Add commit",
                    Subtree(U::AddCommit),
                ),
                ("m", "Subtree import actions", "Merge", Subtree(U::Merge)),
                ("f", "Subtree import actions", "Pull", Subtree(U::Pull)),
            ]
        }
        'E' => {
            use subtree::Op as U;
            vec![
                ("-P", "Arguments", "Prefix", ReadOption("--prefix=")),
                ("-a", "Arguments", "Annotate", ReadOption("--annotate=")),
                ("-b", "Arguments", "Branch", ReadOption("--branch=")),
                ("-o", "Arguments", "Onto", ReadOption("--onto=")),
                (
                    "-i",
                    "Arguments",
                    "Ignore joins",
                    ToggleOption(MenuOption::Switch('E', "--ignore-joins")),
                ),
                (
                    "-j",
                    "Arguments",
                    "Rejoin",
                    ToggleOption(MenuOption::Switch('E', "--rejoin")),
                ),
                ("p", "Subtree export actions", "Push", Subtree(U::Push)),
                ("s", "Subtree export actions", "Split", Subtree(U::Split)),
            ]
        }
        'S' => {
            let sw = |a| ToggleOption(MenuOption::Switch('S', a));
            vec![
                (
                    "-n",
                    "Arguments",
                    "Sort by number of commits",
                    sw("--numbered"),
                ),
                (
                    "-s",
                    "Arguments",
                    "Show commit count summary only",
                    sw("--summary"),
                ),
                ("-e", "Arguments", "Show email addresses", sw("--email")),
                (
                    "-g",
                    "Arguments",
                    "Group commits by",
                    CycleOption("--group="),
                ),
                ("-f", "Arguments", "Format string", ReadOption("--format=")),
                ("-w", "Arguments", "Linewrap", ReadOption("-w")),
                ("s", "Shortlog", "since", LogOp(log::Op::ShortlogSince)),
                ("r", "Shortlog", "range", LogOp(log::Op::ShortlogRange)),
            ]
        }
        'z' => vec![
            (
                "-u",
                "Arguments",
                "Also save untracked files",
                ToggleOption(MenuOption::StashUntracked),
            ),
            (
                "-a",
                "Arguments",
                "Also save untracked and ignored files",
                ToggleOption(MenuOption::StashAll),
            ),
            ("z", "Stash", "Both", Workflow(workflows::Operation::Stash)),
            ("i", "Stash", "Index", Workflow(StashStaged)),
            ("w", "Stash", "Worktree", Workflow(StashWorktree)),
            ("x", "Stash", "Keeping index", Workflow(StashKeepIndex)),
            ("Z", "Snapshot", "Both", Workflow(SnapshotBoth)),
            ("I", "Snapshot", "Index", Workflow(SnapshotIndex)),
            ("W", "Snapshot", "Worktree", Workflow(SnapshotWorktree)),
            (
                "u",
                "Stash",
                "Both including untracked",
                Workflow(StashUntracked),
            ),
            ("a", "Use", "Apply", Action::Stash(StashAction::Apply)),
            ("p", "Use", "Pop", Action::Stash(StashAction::Pop)),
            ("k", "Use", "Drop", Action::Stash(StashAction::Drop)),
            (
                "d",
                "Use",
                "Drop (Fred alias)",
                Action::Stash(StashAction::Drop),
            ),
            ("l", "Inspect", "List", Stashes),
            ("v", "Inspect", "Show selected stash", Visit),
            ("b", "Transform", "Branch", StashOp(stash::Op::Branch)),
            (
                "B",
                "Transform",
                "Branch here",
                StashOp(stash::Op::BranchHere),
            ),
            (
                "f",
                "Transform",
                "Format patch",
                StashOp(stash::Op::FormatPatch),
            ),
            (
                "C",
                "Transform",
                "Clear all (Fred)",
                StashOp(stash::Op::Clear),
            ),
        ],
        // magit-file-dispatch: the visited file or blob.
        'F' => {
            use blame::Kind as K;
            use blob::FileOp as O;
            vec![
                ("s", "File actions", "Stage", File(O::Stage)),
                ("u", "File actions", "Unstage", File(O::Unstage)),
                (",x", "File actions", "Untrack", File(O::Untrack)),
                (",r", "File actions", "Rename", File(O::Rename)),
                (",k", "File actions", "Delete", File(O::Delete)),
                (",c", "File actions", "Checkout", File(O::Checkout)),
                ("D", "Inspect", "Diff...", Menu('d')),
                ("d", "Inspect", "Diff", DiffBufferFile),
                ("L", "Log", "Log...", Menu('l')),
                ("l", "Log", "Log", FileLog),
                ("B", "Blame", "Blame...", Menu('B')),
                ("b", "Blame", "Blame", Blame(K::Addition)),
                ("r", "Blame", "...removal", Blame(K::Removal)),
                ("f", "Blame", "...reverse", Blame(K::Reverse)),
                ("m", "Blame", "Blame echo", Blame(K::Echo)),
                ("q", "Blame", "Quit blame", BlameQuit),
                ("p", "Navigate", "Prev blob", BlobPrevious),
                ("n", "Navigate", "Next blob", BlobNext),
                ("v", "Navigate", "Goto blob", FindFile),
                ("V", "Navigate", "Goto file", BlobVisitFile),
                ("g", "Navigate", "Goto status", Status),
                ("c", "More actions", "Commit", Menu('C')),
            ]
        }
        'B' => vec![
            (
                "-w",
                "Arguments",
                "Ignore whitespace",
                ToggleOption(MenuOption::BlameWhitespace),
            ),
            (
                "-r",
                "Arguments",
                "Do not treat root commits as boundaries",
                ToggleOption(MenuOption::BlameRoot),
            ),
            (
                "-P",
                "Arguments",
                "Follow only first parent",
                ToggleOption(MenuOption::BlameFirstParent),
            ),
            (
                "-M",
                "Arguments",
                "Detect lines moved or copied within a file",
                ToggleOption(MenuOption::BlameMoved),
            ),
            (
                "-C",
                "Arguments",
                "Detect lines moved or copied between files",
                ToggleOption(MenuOption::BlameCopied),
            ),
            (
                "b",
                "Actions",
                "Show commits adding lines",
                Blame(blame::Kind::Addition),
            ),
            (
                "r",
                "Actions",
                "Show commits removing lines",
                Blame(blame::Kind::Removal),
            ),
            (
                "f",
                "Actions",
                "Show last commits that still have lines",
                Blame(blame::Kind::Reverse),
            ),
            ("m", "Actions", "Blame echo", Blame(blame::Kind::Echo)),
            ("q", "Actions", "Quit blaming", BlameQuit),
            ("c", "Refresh", "Cycle style", BlameCycle),
        ],
        'b' => vec![
            ("b", "Checkout", "branch/revision", Branches),
            (
                "l",
                "Checkout",
                "local branch",
                Branch(branch::Op::CheckoutLocal),
            ),
            (
                "c",
                "Checkout",
                "new branch",
                Branch(branch::Op::CreateCheckout),
            ),
            ("s", "Checkout", "new spin-off", Branch(branch::Op::Spinoff)),
            ("n", "Create", "new branch", Branch(branch::Op::Create)),
            ("S", "Create", "new spin-out", Branch(branch::Op::Spinout)),
            ("m", "Do", "rename", Branch(branch::Op::Rename)),
            ("x", "Do", "reset", Branch(branch::Op::Reset)),
            ("k", "Do", "delete", Branch(branch::Op::Delete)),
            (
                "o",
                "Checkout",
                "new orphan",
                Configure(configure::Op::Orphan),
            ),
            (
                "w",
                "Checkout",
                "new worktree",
                Action::Worktree(worktree::Op::Checkout),
            ),
            (
                "W",
                "Create",
                "new worktree",
                Action::Worktree(worktree::Op::Branch),
            ),
            ("C", "Do", "configure...", Menu('c')),
            ("h", "Do", "shelve", Configure(configure::Op::Shelve)),
            ("H", "Do", "unshelve", Configure(configure::Op::Unshelve)),
        ],
        // magit-branch-configure for the current branch; choices cycle.
        'c' => {
            use configure::Op as C;
            vec![
                (
                    "d",
                    "Configure branch",
                    "branch.<branch>.description",
                    Configure(C::Description),
                ),
                (
                    "u",
                    "Configure branch",
                    "branch.<branch>.merge/remote",
                    Configure(C::Upstream),
                ),
                (
                    "r",
                    "Configure branch",
                    "branch.<branch>.rebase",
                    Configure(C::BranchRebase),
                ),
                (
                    "p",
                    "Configure branch",
                    "branch.<branch>.pushRemote",
                    Configure(C::BranchPushRemote),
                ),
                (
                    "R",
                    "Configure repository defaults",
                    "pull.rebase",
                    Configure(C::PullRebase),
                ),
                (
                    "P",
                    "Configure repository defaults",
                    "remote.pushDefault",
                    Configure(C::PushDefault),
                ),
                (
                    "=m",
                    "Configure branch creation",
                    "branch.autoSetupMerge",
                    Configure(C::AutoSetupMerge),
                ),
                (
                    "=r",
                    "Configure branch creation",
                    "branch.autoSetupRebase",
                    Configure(C::AutoSetupRebase),
                ),
            ]
        }
        // magit-remote-configure for the current remote.
        'e' => {
            use configure::Op as C;
            vec![
                (
                    "u",
                    "Configure remote",
                    "remote.<remote>.url",
                    Configure(C::RemoteUrl),
                ),
                (
                    "U",
                    "Configure remote",
                    "remote.<remote>.fetch",
                    Configure(C::RemoteFetch),
                ),
                (
                    "s",
                    "Configure remote",
                    "remote.<remote>.pushurl",
                    Configure(C::RemotePushurl),
                ),
                (
                    "S",
                    "Configure remote",
                    "remote.<remote>.push",
                    Configure(C::RemotePush),
                ),
                (
                    "O",
                    "Configure remote",
                    "remote.<remote>.tagOpt",
                    Configure(C::RemoteTagopt),
                ),
                (
                    "h",
                    "Configure remote",
                    "remote.<remote>.followRemoteHEAD",
                    Configure(C::RemoteFollowHead),
                ),
            ]
        }
        't' => vec![
            (
                "-f",
                "Arguments",
                "Force",
                ToggleOption(MenuOption::TagForce),
            ),
            (
                "-e",
                "Arguments",
                "Edit message",
                ToggleOption(MenuOption::TagEdit),
            ),
            (
                "-a",
                "Arguments",
                "Annotate",
                ToggleOption(MenuOption::TagAnnotate),
            ),
            ("-s", "Arguments", "Sign", ToggleOption(MenuOption::TagSign)),
            ("t", "Create", "tag", Action::Tag(tag::Op::Create)),
            ("r", "Create", "release", Action::Tag(tag::Op::Release)),
            ("k", "Do", "delete", Action::Tag(tag::Op::Delete)),
            ("p", "Do", "prune", Action::Tag(tag::Op::Prune)),
            ("l", "Inspect", "List (Fred)", Tags),
        ],
        'C' => vec![
            (
                "-a",
                "Arguments",
                "Stage all modified and deleted files",
                ToggleOption(MenuOption::CommitAll),
            ),
            (
                "-e",
                "Arguments",
                "Allow empty commit",
                ToggleOption(MenuOption::CommitEmpty),
            ),
            (
                "-n",
                "Arguments",
                "Disable hooks",
                ToggleOption(MenuOption::CommitNoVerify),
            ),
            (
                "-R",
                "Arguments",
                "Claim authorship and reset author date",
                ToggleOption(MenuOption::CommitResetAuthor),
            ),
            (
                "+s",
                "Arguments",
                "Add Signed-off-by trailer",
                ToggleOption(MenuOption::CommitSignoff),
            ),
            (
                "-v",
                "Arguments",
                "Show diff of changes to be committed",
                ToggleOption(MenuOption::Switch('C', "--verbose")),
            ),
            (
                "-A",
                "Arguments",
                "Override the author",
                ReadOption("--author="),
            ),
            (
                "-D",
                "Arguments",
                "Override the author date",
                ReadOption("--date="),
            ),
            (
                "-S",
                "Arguments",
                "Sign using gpg",
                ReadOption("--gpg-sign="),
            ),
            (
                "-C",
                "Arguments",
                "Reuse commit message",
                ReadOption("--reuse-message="),
            ),
            (
                "-c",
                "Arguments",
                "Reedit commit message",
                ReadOption("--reedit-message="),
            ),
            ("a", "Edit HEAD", "Amend", AmendDraft),
            (
                "d",
                "Edit HEAD",
                "Reshelve",
                CommitEdit(commit::Op::Reshelve),
            ),
            (
                "R",
                "Edit and rebase",
                "Reword past",
                Action::Rebase(rebase::Op::RewordCommit),
            ),
            (
                "x",
                "Spread across commits",
                "Modified files",
                CommitEdit(commit::Op::Autofixup),
            ),
            (
                "X",
                "Spread across commits",
                "Updated modules",
                CommitEdit(commit::Op::AbsorbModules),
            ),
            ("f", "Edit", "Fixup", CommitEdit(commit::Op::Fixup)),
            ("s", "Edit", "Squash", CommitEdit(commit::Op::Squash)),
            ("A", "Edit", "Alter", CommitEdit(commit::Op::Alter)),
            ("n", "Edit", "Augment", CommitEdit(commit::Op::Augment)),
            ("W", "Edit", "Revise", CommitEdit(commit::Op::Revise)),
            (
                "F",
                "Edit and rebase",
                "Instant fixup",
                CommitEdit(commit::Op::InstantFixup),
            ),
            (
                "S",
                "Edit and rebase",
                "Instant squash",
                CommitEdit(commit::Op::InstantSquash),
            ),
            ("e", "Edit HEAD", "Extend (keep message)", Workflow(Amend)),
            ("w", "Edit HEAD", "Reword (keep tree)", RewordDraft),
            ("c", "Create", "Commit", Commit),
        ],
        'O' => {
            use remote::Op as O;
            vec![
                (
                    "-f",
                    "Arguments for add",
                    "Fetch after add",
                    ToggleOption(MenuOption::RemoteFetch),
                ),
                ("a", "Actions", "Add", Action::Remote(O::Add)),
                ("r", "Actions", "Rename", Action::Remote(O::Rename)),
                ("k", "Actions", "Remove", Action::Remote(O::Remove)),
                (
                    "p",
                    "Actions",
                    "Prune stale branches",
                    Action::Remote(O::Prune),
                ),
                (
                    "P",
                    "Actions",
                    "Prune stale refspecs",
                    Action::Remote(O::PruneRefspecs),
                ),
                ("C", "Actions", "Configure...", Menu('e')),
                (
                    "z",
                    "Actions",
                    "Unshallow remote",
                    Configure(configure::Op::Unshallow),
                ),
            ]
        }
        // magit-bisect; while bisecting, B marks bad (resolved when run).
        'G' => {
            use bisect::Op as O;
            vec![
                (
                    "-n",
                    "Arguments",
                    "Don't checkout commits",
                    ToggleOption(MenuOption::BisectNoCheckout),
                ),
                (
                    "-p",
                    "Arguments",
                    "Follow only first parent of a merge",
                    ToggleOption(MenuOption::BisectFirstParent),
                ),
                ("B", "Actions", "Start / bad", Action::Bisect(O::Start)),
                (
                    "s",
                    "Actions",
                    "Start script / run script",
                    Action::Bisect(O::Run),
                ),
                ("g", "Actions (bisecting)", "Good", Action::Bisect(O::Good)),
                ("m", "Actions (bisecting)", "Mark", Action::Bisect(O::Mark)),
                ("k", "Actions (bisecting)", "Skip", Action::Bisect(O::Skip)),
                (
                    "r",
                    "Actions (bisecting)",
                    "Reset",
                    Action::Bisect(O::Reset),
                ),
            ]
        }
        // magit-notes (internal id N; leader key T). While merging notes,
        // c commits and a aborts the merge (resolved when run).
        'N' => {
            use notes::Op as O;
            vec![
                (
                    "c",
                    "Configure local settings",
                    "core.notesRef / commit merge",
                    Action::Notes(O::NotesRef(false)),
                ),
                (
                    "d",
                    "Configure local settings",
                    "notes.displayRef",
                    Action::Notes(O::DisplayRef(false)),
                ),
                (
                    "C",
                    "Configure global settings",
                    "core.notesRef",
                    Action::Notes(O::NotesRef(true)),
                ),
                (
                    "D",
                    "Configure global settings",
                    "notes.displayRef",
                    Action::Notes(O::DisplayRef(true)),
                ),
                (
                    "-n",
                    "Arguments for prune",
                    "Dry run",
                    ToggleOption(MenuOption::NotesDryRun),
                ),
                (
                    "-s",
                    "Arguments for merge",
                    "Merge strategy",
                    CycleOption("--strategy="),
                ),
                ("T", "Actions", "Edit", Action::Notes(O::Edit)),
                ("r", "Actions", "Remove", Action::Notes(O::Remove)),
                ("m", "Actions", "Merge", Action::Notes(O::Merge)),
                ("p", "Actions", "Prune", Action::Notes(O::Prune)),
                (
                    "a",
                    "Actions (merging)",
                    "Abort merge",
                    Action::Notes(O::MergeAbort),
                ),
            ]
        }
        // magit-worktree (internal id Y; leader key Z as in magit-dispatch).
        'Y' => {
            use worktree::Op as O;
            vec![
                ("b", "Create new", "worktree", Action::Worktree(O::Checkout)),
                (
                    "c",
                    "Create new",
                    "branch and worktree",
                    Action::Worktree(O::Branch),
                ),
                ("m", "Commands", "Move worktree", Action::Worktree(O::Move)),
                (
                    "k",
                    "Commands",
                    "Delete worktree",
                    Action::Worktree(O::Delete),
                ),
                (
                    "g",
                    "Commands",
                    "Visit worktree",
                    Action::Worktree(O::Visit),
                ),
            ]
        }
        'X' => {
            use reset::Op as O;
            vec![
                ("b", "Reset", "branch", Branch(branch::Op::Reset)),
                ("f", "Reset", "file", File(blob::FileOp::Checkout)),
                (
                    "m",
                    "Reset this",
                    "mixed    (HEAD and index)",
                    Action::Reset(O::Mixed),
                ),
                (
                    "s",
                    "Reset this",
                    "soft     (HEAD only)",
                    Action::Reset(O::Soft),
                ),
                (
                    "h",
                    "Reset this",
                    "hard     (HEAD, index and worktree)",
                    Action::Reset(O::Hard),
                ),
                (
                    "k",
                    "Reset this",
                    "keep     (HEAD and index, keeping uncommitted)",
                    Action::Reset(O::Keep),
                ),
                (
                    "i",
                    "Reset this",
                    "index    (only)",
                    Action::Reset(O::Index),
                ),
                (
                    "w",
                    "Reset this",
                    "worktree (only)",
                    Action::Reset(O::Worktree),
                ),
            ]
        }
        // magit-merge; while merging, m commits and a aborts (as upstream's
        // in-progress group), resolved when run.
        'M' => {
            use merge::Op as O;
            vec![
                (
                    "-f",
                    "Arguments",
                    "Fast-forward only",
                    ToggleOption(MenuOption::MergeFfOnly),
                ),
                (
                    "-n",
                    "Arguments",
                    "No fast-forward",
                    ToggleOption(MenuOption::MergeNoFf),
                ),
                ("-s", "Arguments", "Strategy", CycleOption("--strategy=")),
                (
                    "m",
                    "Actions",
                    "Merge / commit merge",
                    Action::Merge(O::Plain),
                ),
                (
                    "e",
                    "Actions",
                    "Merge and edit message",
                    Action::Merge(O::EditMsg),
                ),
                (
                    "n",
                    "Actions",
                    "Merge but don't commit",
                    Action::Merge(O::NoCommit),
                ),
                (
                    "a",
                    "Actions",
                    "Absorb / abort merge",
                    Action::Merge(O::Absorb),
                ),
                ("p", "Actions", "Preview merge", Action::Merge(O::Preview)),
                ("s", "Actions", "Squash merge", Action::Merge(O::Squash)),
                ("d", "Actions", "Dissolve", Action::Merge(O::Dissolve)),
            ]
        }
        // magit-rebase; while rebasing, r continues, s skips, e edits the
        // todo list and a aborts (resolved when run).
        'r' => {
            use rebase::Op as O;
            vec![
                (
                    "-k",
                    "Arguments",
                    "Keep empty commits",
                    ToggleOption(MenuOption::RebaseKeepEmpty),
                ),
                (
                    "-r",
                    "Arguments",
                    "Rebase merges",
                    CycleOption("--rebase-merges="),
                ),
                (
                    "-u",
                    "Arguments",
                    "Update branches",
                    ToggleOption(MenuOption::RebaseUpdateRefs),
                ),
                (
                    "-d",
                    "Arguments",
                    "Use author date as committer date",
                    ToggleOption(MenuOption::RebaseAuthorDate),
                ),
                (
                    "-t",
                    "Arguments",
                    "Use current time as author date",
                    ToggleOption(MenuOption::RebaseIgnoreDate),
                ),
                (
                    "-a",
                    "Arguments",
                    "Autosquash",
                    ToggleOption(MenuOption::RebaseAutosquash),
                ),
                (
                    "-A",
                    "Arguments",
                    "Autostash",
                    ToggleOption(MenuOption::RebaseAutostash),
                ),
                (
                    "-i",
                    "Arguments",
                    "Interactive",
                    ToggleOption(MenuOption::RebaseInteractive),
                ),
                (
                    "-h",
                    "Arguments",
                    "Disable hooks",
                    ToggleOption(MenuOption::RebaseNoVerify),
                ),
                (
                    "p",
                    "Rebase onto",
                    "pushRemote",
                    Action::Rebase(O::OntoPushRemote),
                ),
                (
                    "u",
                    "Rebase onto",
                    "@{upstream}",
                    Action::Rebase(O::OntoUpstream),
                ),
                (
                    "e",
                    "Rebase onto",
                    "elsewhere / edit todo",
                    Action::Rebase(O::Elsewhere),
                ),
                (
                    "i",
                    "Rebase",
                    "interactively",
                    Action::Rebase(O::Interactive),
                ),
                ("s", "Rebase", "a subset / skip", Action::Rebase(O::Subset)),
                (
                    "m",
                    "Rebase",
                    "to modify a commit",
                    Action::Rebase(O::EditCommit),
                ),
                (
                    "w",
                    "Rebase",
                    "to reword a commit",
                    Action::Rebase(O::RewordCommit),
                ),
                (
                    "k",
                    "Rebase",
                    "to remove a commit",
                    Action::Rebase(O::RemoveCommit),
                ),
                (
                    "f",
                    "Rebase",
                    "to autosquash",
                    Action::Rebase(O::Autosquash),
                ),
                (
                    "r",
                    "Actions (rebasing)",
                    "Continue",
                    Action::Rebase(O::Continue),
                ),
                ("a", "Actions (rebasing)", "Abort", Action::Rebase(O::Abort)),
                (
                    "-f",
                    "Arguments",
                    "Force rebase",
                    ToggleOption(MenuOption::Switch('r', "--force-rebase")),
                ),
                (
                    "-x",
                    "Arguments",
                    "Run command after commits",
                    ReadOption("--exec="),
                ),
                (
                    "-S",
                    "Arguments",
                    "Sign using gpg",
                    ReadOption("--gpg-sign="),
                ),
                (
                    "+s",
                    "Arguments",
                    "Add Signed-off-by lines",
                    ToggleOption(MenuOption::Switch('r', "--signoff")),
                ),
            ]
        }
        // magit-cherry-pick; while a sequence runs, A continues, s skips, a aborts.
        'x' => {
            use sequence::Op as O;
            vec![
                ("=s", "Arguments", "Strategy", CycleOption("--strategy=")),
                (
                    "-F",
                    "Arguments",
                    "Attempt fast-forward",
                    ToggleOption(MenuOption::CherryFf),
                ),
                (
                    "-x",
                    "Arguments",
                    "Reference cherry in commit message",
                    ToggleOption(MenuOption::CherryX),
                ),
                (
                    "-e",
                    "Arguments",
                    "Edit commit messages",
                    ToggleOption(MenuOption::CherryEdit),
                ),
                (
                    "A",
                    "Apply here",
                    "Pick / continue",
                    Action::Sequence(O::Pick),
                ),
                (
                    "a",
                    "Apply here",
                    "Apply / abort",
                    Action::Sequence(O::Apply),
                ),
                ("h", "Apply here", "Harvest", Action::Sequence(O::Harvest)),
                (
                    "m",
                    "Apply here",
                    "Squash",
                    Action::Merge(merge::Op::Squash),
                ),
                (
                    "d",
                    "Apply elsewhere",
                    "Donate",
                    Action::Sequence(O::Donate),
                ),
                (
                    "n",
                    "Apply elsewhere",
                    "Spinout",
                    Action::Sequence(O::Spinout),
                ),
                (
                    "s",
                    "Apply elsewhere",
                    "Spinoff / skip",
                    Action::Sequence(O::Spinoff),
                ),
                (
                    "-m",
                    "Arguments",
                    "Replay merge relative to parent",
                    ReadOption("--mainline="),
                ),
                (
                    "-S",
                    "Arguments",
                    "Sign using gpg",
                    ReadOption("--gpg-sign="),
                ),
                (
                    "+s",
                    "Arguments",
                    "Add Signed-off-by lines",
                    ToggleOption(MenuOption::Switch('x', "--signoff")),
                ),
            ]
        }
        // magit-revert; while a sequence runs, V continues, s skips, a aborts.
        'v' => {
            use sequence::Op as O;
            vec![
                (
                    "-e",
                    "Arguments",
                    "Edit commit message",
                    ToggleOption(MenuOption::RevertEdit),
                ),
                (
                    "-E",
                    "Arguments",
                    "Don't edit commit message",
                    ToggleOption(MenuOption::RevertNoEdit),
                ),
                ("=s", "Arguments", "Strategy", CycleOption("--strategy=")),
                (
                    "V",
                    "Actions",
                    "Revert commit(s) / continue",
                    Action::Sequence(O::Revert),
                ),
                (
                    "v",
                    "Actions",
                    "Revert changes",
                    Action::Sequence(O::RevertNoCommit),
                ),
                ("s", "Sequence", "Skip", Action::Sequence(O::Skip)),
                ("a", "Sequence", "Abort", Action::Sequence(O::Abort)),
                (
                    "-m",
                    "Arguments",
                    "Replay merge relative to parent",
                    ReadOption("--mainline="),
                ),
                (
                    "-S",
                    "Arguments",
                    "Sign using gpg",
                    ReadOption("--gpg-sign="),
                ),
                (
                    "+s",
                    "Arguments",
                    "Add Signed-off-by lines",
                    ToggleOption(MenuOption::Switch('v', "--signoff")),
                ),
            ]
        }
        _ => vec![],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuOption {
    LogFollow,
    StashUntracked,
    StashAll,
    CommitAll,
    CommitEmpty,
    CommitNoVerify,
    CommitResetAuthor,
    CommitSignoff,
    PushForceWithLease,
    PushForce,
    PushNoVerify,
    PushDryRun,
    PushSetUpstream,
    PushTags,
    PushFollowTags,
    FetchPrune,
    FetchTags,
    FetchForce,
    PullFfOnly,
    PullForce,
    DiffIgnoreSpace,
    DiffIgnoreAllSpace,
    DiffFunctionContext,
    DiffRenames,
    DiffNoExt,
    DiffStat,
    DiffSignature,
    RemoteFetch,
    NotesDryRun,
    BisectNoCheckout,
    BisectFirstParent,
    RebaseKeepEmpty,
    RebaseUpdateRefs,
    RebaseAuthorDate,
    RebaseIgnoreDate,
    RebaseAutosquash,
    RebaseAutostash,
    RebaseInteractive,
    RebaseNoVerify,
    CherryFf,
    CherryX,
    CherryEdit,
    RevertEdit,
    RevertNoEdit,
    MergeFfOnly,
    MergeNoFf,
    TagForce,
    TagEdit,
    TagAnnotate,
    TagSign,
    BlameWhitespace,
    BlameRoot,
    BlameFirstParent,
    BlameMoved,
    BlameCopied,
    /// A transient-switch of this menu by its argument.
    Switch(char, &'static str),
    /// A transient-option with fixed choices: argument prefix and selected value.
    Choice(char, &'static str, &'static str),
}
/// Fred cycles a transient-option's choices instead of reading one, then turns it off.
pub fn choices(menu: char, prefix: &str) -> &'static [&'static str] {
    match prefix {
        "--strategy=" if menu == 'N' => &["manual", "ours", "theirs", "union", "cat_sort_uniq"],
        "--rebase=" => &["true", "merges", "interactive", "false"],
        "--diff-algorithm=" => &["default", "minimal", "patience", "histogram"],
        "--diff-merges=" => &["off", "first-parent", "combined", "dense-combined"],
        "--ignore-submodules=" => &["none", "untracked", "dirty", "all"],
        "--strategy=" => &["resolve", "recursive", "octopus", "ours", "subtree"],
        "--rebase-merges=" => &["no-rebase-cousins", "rebase-cousins"],
        // "trailer:" takes a key Fred cannot read in a cycle.
        "--group=" => &["author", "committer"],
        "--thread=" => &["deep", "shallow"],
        "--color-moved=" => &["default", "plain", "blocks", "zebra", "dimmed-zebra"],
        "--color-moved-ws=" => &[
            "ignore-space-at-eol",
            "ignore-space-change",
            "ignore-all-space",
            "allow-indentation-change",
        ],
        "--sort=" => &[
            "-committerdate",
            "-authordate",
            "committerdate",
            "authordate",
        ],
        "--cover-from-description=" => &["message", "subject", "auto", "none"],
        // magit-log:--*-order.
        "--" if menu == 'l' => &["topo-order", "author-date-order", "date-order"],
        _ => &[],
    }
}
impl MenuOption {
    pub fn menu(self) -> char {
        use MenuOption::*;
        match self {
            PushForceWithLease | PushForce | PushNoVerify | PushDryRun | PushSetUpstream
            | PushTags | PushFollowTags => 'p',
            FetchPrune | FetchTags | FetchForce => 'f',
            MergeFfOnly | MergeNoFf => 'M',
            RemoteFetch => 'O',
            NotesDryRun => 'N',
            BisectNoCheckout | BisectFirstParent => 'G',
            CherryFf | CherryX | CherryEdit => 'x',
            RebaseKeepEmpty | RebaseUpdateRefs | RebaseAuthorDate | RebaseIgnoreDate
            | RebaseAutosquash | RebaseAutostash | RebaseInteractive | RebaseNoVerify => 'r',
            RevertEdit | RevertNoEdit => 'v',
            PullFfOnly | PullForce => 'P',
            Choice(menu, ..) | Switch(menu, _) => menu,
            DiffIgnoreSpace | DiffIgnoreAllSpace | DiffFunctionContext | DiffRenames
            | DiffNoExt | DiffStat | DiffSignature => 'd',
            BlameWhitespace | BlameRoot | BlameFirstParent | BlameMoved | BlameCopied => 'B',
            TagForce | TagEdit | TagAnnotate | TagSign => 't',
            Self::LogFollow => 'l',
            Self::StashUntracked | Self::StashAll => 'z',
            _ => 'C',
        }
    }
    pub fn argument(self) -> String {
        if let Self::Choice(_, prefix, value) = self {
            return format!("{prefix}{value}");
        }
        if let Self::Switch(_, argument) = self {
            return argument.into();
        }
        match self {
            Self::LogFollow => "--follow",
            Self::StashUntracked => "--include-untracked",
            Self::StashAll => "--all",
            Self::CommitAll => "--all",
            Self::CommitEmpty => "--allow-empty",
            Self::CommitNoVerify => "--no-verify",
            Self::CommitResetAuthor => "--reset-author",
            Self::CommitSignoff => "--signoff",
            Self::PushForceWithLease => "--force-with-lease",
            Self::PushForce | Self::FetchForce | Self::PullForce => "--force",
            Self::PushNoVerify => "--no-verify",
            Self::PushDryRun => "--dry-run",
            Self::PushSetUpstream => "--set-upstream",
            Self::PushTags | Self::FetchTags => "--tags",
            Self::PushFollowTags => "--follow-tags",
            Self::FetchPrune => "--prune",
            Self::PullFfOnly => "--ff-only",
            Self::DiffIgnoreSpace => "--ignore-space-change",
            Self::DiffIgnoreAllSpace => "--ignore-all-space",
            Self::DiffFunctionContext => "--function-context",
            Self::DiffRenames => "-M",
            Self::DiffNoExt => "--no-ext-diff",
            Self::DiffStat => "--stat",
            Self::DiffSignature => "--show-signature",
            Self::RemoteFetch => "-f",
            Self::NotesDryRun => "--dry-run",
            Self::BisectNoCheckout => "--no-checkout",
            Self::BisectFirstParent => "--first-parent",
            Self::RebaseKeepEmpty => "--keep-empty",
            Self::RebaseUpdateRefs => "--update-refs",
            Self::RebaseAuthorDate => "--committer-date-is-author-date",
            Self::RebaseIgnoreDate => "--ignore-date",
            Self::RebaseAutosquash => "--autosquash",
            Self::RebaseAutostash => "--autostash",
            Self::RebaseInteractive => "--interactive",
            Self::RebaseNoVerify => "--no-verify",
            Self::CherryFf => "--ff",
            Self::CherryX => "-x",
            Self::CherryEdit | Self::RevertEdit => "--edit",
            Self::RevertNoEdit => "--no-edit",
            Self::MergeFfOnly => "--ff-only",
            Self::MergeNoFf => "--no-ff",
            Self::TagForce => "--force",
            Self::TagEdit => "--edit",
            Self::TagAnnotate => "--annotate",
            Self::TagSign => "--sign",
            Self::BlameWhitespace => "-w",
            Self::BlameRoot => "--root",
            Self::BlameFirstParent => "--first-parent",
            Self::BlameMoved => "-M",
            Self::BlameCopied => "-C",
            Self::Choice(..) | Self::Switch(..) => unreachable!(),
        }
        .into()
    }
}
/// Menus that share another menu's arguments (magit-diff-refresh uses magit-diff's).
pub fn arg_menu(menu: char) -> char {
    if menu == 'D' { 'd' } else { menu }
}
pub fn current_choice(ed: &Editor, menu: char, prefix: &str) -> Option<&'static str> {
    ed.magit_options.iter().find_map(|o| match o {
        MenuOption::Choice(m, p, v) if *m == menu && *p == prefix => Some(*v),
        _ => None,
    })
}
fn cycle_choice(ed: &mut Editor, menu: char, prefix: &'static str) {
    let all = choices(menu, prefix);
    let next = match current_choice(ed, menu, prefix) {
        None => all.first(),
        Some(v) => all.iter().skip_while(|c| **c != v).nth(1),
    };
    ed.magit_options
        .retain(|o| !matches!(o, MenuOption::Choice(m, p, _) if *m == menu && *p == prefix));
    if let Some(next) = next {
        ed.magit_options
            .insert(MenuOption::Choice(menu, prefix, next));
    }
}
pub fn menu_arguments(ed: &Editor, menu: char) -> Vec<String> {
    let mut args: Vec<_> = ed
        .magit_options
        .iter()
        .filter(|option| option.menu() == menu)
        .map(|option| option.argument())
        .chain(
            ed.magit_values
                .iter()
                .filter(|((m, _), _)| *m == menu)
                .map(|((_, prefix), value)| format!("{prefix}{value}")),
        )
        .collect();
    args.sort();
    args
}

/// The commit menu's arguments for drafts and commands: -C/-c only apply to
/// a new commit (magit-commit-create), where they are read separately.
pub fn commit_arguments(ed: &Editor) -> Vec<String> {
    menu_arguments(ed, 'C')
        .into_iter()
        .filter(|a| !a.starts_with("--reuse-message=") && !a.starts_with("--reedit-message="))
        .collect()
}
/// Make a menu's displayed switches and choices match explicit arguments.
pub fn set_menu_arguments(ed: &mut Editor, menu: char, args: &[String]) {
    ed.magit_options.retain(|option| option.menu() != menu);
    ed.magit_values.retain(|(m, _), _| *m != menu);
    for (_, _, _, action) in menu_entries(menu) {
        match action {
            Action::ReadOption(prefix) => {
                if let Some(value) = args.iter().find_map(|a| a.strip_prefix(prefix)) {
                    ed.magit_values.insert((menu, prefix), value.to_owned());
                }
            }
            Action::ToggleOption(option) if args.contains(&option.argument()) => {
                ed.magit_options.insert(option);
            }
            Action::CycleOption(prefix) => {
                if let Some(value) = choices(menu, prefix)
                    .iter()
                    .find(|v| args.contains(&format!("{prefix}{v}")))
                {
                    ed.magit_options
                        .insert(MenuOption::Choice(menu, prefix, value));
                }
            }
            _ => (),
        }
    }
}
/// A draft's execution arguments are also its displayed transient state.
pub fn sync_commit_options(ed: &mut Editor) {
    let args = ed.commit_args.clone();
    set_menu_arguments(ed, 'C', &args);
}
