//! A status-centered Git component; rendered text is never used as an operation path.
pub mod blame;
pub mod blob;
pub mod branch;
pub mod diff;
pub mod merge;
pub mod network;
pub mod repo;
pub mod status;
pub mod tag;
pub mod workflows;
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
    Toggle,
    Stage,
    Unstage,
    Visit,
    Return,
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
    fn contains(self, e: &repo::Entry) -> bool {
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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub action: Option<RowAction>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Status,
    Log,
    FileLog(PathBuf, bool),
    Stashes,
    Tags,
    StashPatch(workflows::Stash),
    Patch(String),
    Diff(diff::Target, Vec<String>),
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
            Kind::Log => "Magit log".into(),
            Kind::FileLog(path, _) => format!("Magit file log {}", label(path)),
            Kind::Stashes => "Magit stashes".into(),
            Kind::Tags => "Magit tags".into(),
            Kind::StashPatch(stash) => format!("Magit {}", stash.selector),
            Kind::Patch(id) => format!("Magit {id}"),
            Kind::Diff(target, _) => format!("Magit diff: {}", target.title()),
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
        if !k.ctrl && ed.vim.pending.is_empty() && matches!(k.char(), Some('-' | '+' | '=' | ',')) {
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
                    cycle_choice(ed, prefix);
                    // magit-pull :incompatible --ff-only with rebasing choices.
                    if ed
                        .magit_options
                        .iter()
                        .any(|o| matches!(o, MenuOption::Choice("--rebase=", v) if *v != "false"))
                    {
                        ed.magit_options.remove(&MenuOption::PullFfOnly);
                    }
                    crate::pick::magit_menu(ed, menu);
                    return true;
                }
                if let Action::ToggleOption(option) = action {
                    if !ed.magit_options.remove(&option) {
                        ed.magit_options.insert(option);
                    }
                    if option == MenuOption::MergeFfOnly {
                        ed.magit_options.remove(&MenuOption::MergeNoFf);
                    }
                    if option == MenuOption::MergeNoFf {
                        ed.magit_options.remove(&MenuOption::MergeFfOnly);
                    }
                    if option == MenuOption::PullFfOnly {
                        ed.magit_options.retain(
                            |o| !matches!(o, MenuOption::Choice("--rebase=", v) if *v != "false"),
                        );
                    }
                    if option == MenuOption::StashAll {
                        ed.magit_options.remove(&MenuOption::StashUntracked);
                    }
                    if option == MenuOption::StashUntracked {
                        ed.magit_options.remove(&MenuOption::StashAll);
                    }
                    if ed.commit_repo.is_some() && option.menu() == 'C' {
                        ed.commit_args = menu_arguments(ed, 'C');
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
    if ed.vim.pending == [Key::ch('g')] && k.char() == Some('r') {
        ed.vim.pending.clear();
        ed.pending_effect = Some(ExEffect::Magit(Action::Refresh));
        return true;
    }
    if !ed.vim.pending.is_empty() {
        return false;
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
        KeyCode::Char('s') if !k.ctrl => Some(Action::Stage),
        KeyCode::Char('u') if !k.ctrl => Some(Action::Unstage),
        KeyCode::Char('q') if !k.ctrl => Some(Action::Return),
        _ => None,
    };
    if let Some(a) = action {
        ed.pending_effect = Some(ExEffect::Magit(a));
        return true;
    }
    false
}
#[cfg(test)]
mod tests;

fn menu_help(menu: char) -> Option<&'static str> {
    Some(match menu {
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
        'l' => "Log: l current  h HEAD  -f follow renames for file log; Space m L current file",
        't' => "Tag: t tag  r release  k delete  p prune; -a annotate -s sign -e message -f force",
        'C' => "Commit: a amend  e extend  w reword  f fixup",
        'M' => {
            "Merge: m merge  e edit msg  n no commit  a absorb  p preview  s squash  d dissolve; merging: m commit  a abort"
        }
        'r' => "Rebase: r onto revision  c continue  s skip  a abort",
        'x' => "Cherry-pick: p pick  c continue  s skip  a abort",
        'v' => "Revert: v revert  c continue  s skip  a abort",
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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    Branch(branch::Op),
    Tag(tag::Op),
    Merge(merge::Op),
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
    // magit-blame :value '("-w").
    if menu == 'B' && ed.magit_seeded.insert('B') {
        ed.magit_options.insert(MenuOption::BlameWhitespace);
    }
    if menu == 'd' {
        let args = match ed.magit.as_ref().map(|v| &v.kind) {
            Some(Kind::Diff(_, args)) => Some(args.clone()),
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
            ("b", "Branch", "Branch operations", Menu('b')),
            ("B", "Inspect", "Blame", Menu('B')),
            ("c", "Commit", "Commit menu", Menu('C')),
            ("C", "Commit", "Amend / fixup", Menu('C')),
            ("p", "Network", "Push", Menu('p')),
            ("P", "Network", "Pull", Menu('P')),
            ("f", "Network", "Fetch", Menu('f')),
            ("z", "Change", "Stash", Menu('z')),
            ("t", "Tag", "Tags", Menu('t')),
            ("M", "History", "Merge", Menu('M')),
            ("R", "History", "Rebase", Menu('r')),
            ("r", "History", "Revert", Menu('v')),
            ("x", "History", "Cherry-pick", Menu('x')),
            ("v", "History", "Revert", Menu('v')),
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
        'l' => vec![
            (
                "-f",
                "Arguments",
                "Follow renames in file log",
                ToggleOption(MenuOption::LogFollow),
            ),
            ("l", "Log", "Current (HEAD)", Log),
            ("h", "Log", "HEAD", LogHead),
        ],
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
                ("d", "Inspect", "Diff", Diff(diff::Op::Unstaged)),
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
        ],
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
            ("a", "Commit", "Amend", AmendDraft),
            ("f", "Commit", "Fixup", Workflow(Fixup)),
            ("e", "Edit HEAD", "Extend (keep message)", Workflow(Amend)),
            ("w", "Edit HEAD", "Reword (keep tree)", RewordDraft),
            ("c", "Create", "Commit", Commit),
        ],
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
        'r' => vec![
            ("r", "Rebase", "Onto revision", Workflow(Rebase)),
            ("c", "Sequence", "Continue", Workflow(RebaseContinue)),
            ("s", "Sequence", "Skip", Workflow(RebaseSkip)),
            ("a", "Sequence", "Abort", Workflow(RebaseAbort)),
        ],
        'x' => vec![
            ("p", "Cherry-pick", "Pick revision", Workflow(CherryPick)),
            ("c", "Sequence", "Continue", Workflow(CherryContinue)),
            ("s", "Sequence", "Skip", Workflow(CherrySkip)),
            ("a", "Sequence", "Abort", Workflow(CherryAbort)),
        ],
        'v' => vec![
            ("v", "Revert", "Revert revision", Workflow(Revert)),
            ("c", "Sequence", "Continue", Workflow(RevertContinue)),
            ("s", "Sequence", "Skip", Workflow(RevertSkip)),
            ("a", "Sequence", "Abort", Workflow(RevertAbort)),
        ],
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
    /// A transient-option with fixed choices: argument prefix and selected value.
    Choice(&'static str, &'static str),
}
/// Fred cycles a transient-option's choices instead of reading one, then turns it off.
pub fn choices(prefix: &str) -> &'static [&'static str] {
    match prefix {
        "--rebase=" => &["true", "merges", "interactive", "false"],
        "--diff-algorithm=" => &["default", "minimal", "patience", "histogram"],
        "--diff-merges=" => &["off", "first-parent", "combined", "dense-combined"],
        "--ignore-submodules=" => &["none", "untracked", "dirty", "all"],
        "--strategy=" => &["resolve", "recursive", "octopus", "ours", "subtree"],
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
            MergeFfOnly | MergeNoFf | Choice("--strategy=", _) => 'M',
            PullFfOnly | PullForce | Choice("--rebase=", _) => 'P',
            DiffIgnoreSpace | DiffIgnoreAllSpace | DiffFunctionContext | DiffRenames
            | DiffNoExt | DiffStat | DiffSignature | Choice(..) => 'd',
            BlameWhitespace | BlameRoot | BlameFirstParent | BlameMoved | BlameCopied => 'B',
            TagForce | TagEdit | TagAnnotate | TagSign => 't',
            Self::LogFollow => 'l',
            Self::StashUntracked | Self::StashAll => 'z',
            _ => 'C',
        }
    }
    pub fn argument(self) -> String {
        if let Self::Choice(prefix, value) = self {
            return format!("{prefix}{value}");
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
            Self::Choice(..) => unreachable!(),
        }
        .into()
    }
}
pub fn current_choice(ed: &Editor, prefix: &str) -> Option<&'static str> {
    ed.magit_options.iter().find_map(|o| match o {
        MenuOption::Choice(p, v) if *p == prefix => Some(*v),
        _ => None,
    })
}
fn cycle_choice(ed: &mut Editor, prefix: &'static str) {
    let all = choices(prefix);
    let next = match current_choice(ed, prefix) {
        None => all.first(),
        Some(v) => all.iter().skip_while(|c| **c != v).nth(1),
    };
    ed.magit_options
        .retain(|o| !matches!(o, MenuOption::Choice(p, _) if *p == prefix));
    if let Some(next) = next {
        ed.magit_options.insert(MenuOption::Choice(prefix, next));
    }
}
pub fn menu_arguments(ed: &Editor, menu: char) -> Vec<String> {
    let mut args: Vec<_> = ed
        .magit_options
        .iter()
        .filter(|option| option.menu() == menu)
        .map(|option| option.argument())
        .collect();
    args.sort();
    args
}

/// Make a menu's displayed switches and choices match explicit arguments.
pub fn set_menu_arguments(ed: &mut Editor, menu: char, args: &[String]) {
    ed.magit_options.retain(|option| option.menu() != menu);
    for (_, _, _, action) in menu_entries(menu) {
        match action {
            Action::ToggleOption(option) if args.contains(&option.argument()) => {
                ed.magit_options.insert(option);
            }
            Action::CycleOption(prefix) => {
                if let Some(value) = choices(prefix)
                    .iter()
                    .find(|v| args.contains(&format!("{prefix}{v}")))
                {
                    ed.magit_options.insert(MenuOption::Choice(prefix, value));
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
