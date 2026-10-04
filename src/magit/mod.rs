//! A status-centered Git component; rendered text is never used as an operation path.
pub mod repo;
pub mod workflows;
use crate::{
    editor::{Editor, Mode},
    ex::ExEffect,
    key::{Key, KeyCode},
};
use repo::{Diff, Repo, Snapshot, label};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub const HELP: &str = "Magit: s status  p push  P pull  f fetch  c commit  l log  b branches  z stash  B branch  t tag  C commit  M merge  r rebase  x cherry-pick  v revert";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Menu(char),
    ToggleOption(MenuOption),
    AmendDraft,
    RewordDraft,
    Stash(workflows::StashAction),
    DropStash(repo::Repo, workflows::Stash),
    Stashes,
    Tags,
    Workflow(workflows::Operation),
    Submit(repo::Repo, workflows::Operation, String, Vec<String>),
    Status,
    Push,
    Pull,
    Fetch,
    Commit,
    Log,
    Branches,
    Switch(String),
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
}
impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Untracked => "Untracked",
            Self::Unstaged => "Unstaged",
            Self::Staged => "Staged",
        }
    }
    fn contains(self, e: &repo::Entry) -> bool {
        match self {
            Self::Conflicts => e.conflict,
            Self::Untracked => e.untracked,
            Self::Unstaged => e.unstaged,
            Self::Staged => e.staged,
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
    Stashes,
    Tags,
    StashPatch(workflows::Stash),
    Patch(String),
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
            Kind::Stashes => "Magit stashes".into(),
            Kind::Tags => "Magit tags".into(),
            Kind::StashPatch(stash) => format!("Magit {}", stash.selector),
            Kind::Patch(id) => format!("Magit {id}"),
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
        self.rows.push(Row {
            text: format!("Head: {}", self.snapshot.branch),
            action: None,
        });
        if let Some(operation) = &self.snapshot.operation {
            self.rows.push(Row {
                text: format!("In progress: {operation}"),
                action: None,
            });
        }
        if let Some(u) = &self.snapshot.upstream {
            self.rows.push(Row {
                text: format!(
                    "Upstream: {u} {}",
                    self.snapshot.ahead_behind.as_deref().unwrap_or("")
                ),
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
    }
    pub fn selection(&self, action: &Option<RowAction>, fallback: usize) -> usize {
        action
            .as_ref()
            .and_then(|a| self.rows.iter().position(|r| r.action.as_ref() == Some(a)))
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
        if !k.ctrl && matches!(k.char(), Some('-' | '+')) {
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
                if let Action::ToggleOption(option) = action {
                    if !ed.magit_options.remove(&option) {
                        ed.magit_options.insert(option);
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
                if let Action::Menu(menu) = action {
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
            Some('d' | 'k') if !k.ctrl => Some(workflows::StashAction::Drop),
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
        'B' => "Branch: c create  s create and switch  r rename current  d delete merged",
        't' => "Tag: c create lightweight tag  l list",
        'C' => "Commit: a amend  e extend  w reword  f fixup",
        'M' => "Merge: m merge  s squash  c continue  a abort",
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
}
pub fn prompt(ed: &mut Editor, question: Prompt) {
    let text = match &question {
        Prompt::Workflow(_, operation, _) => operation.prompt().unwrap_or("").to_owned(),
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
        None => (),
    }
}

fn open_menu(ed: &mut Editor, menu: char) {
    if menu == 'C' && ed.commit_repo.is_some() {
        sync_commit_options(ed);
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
            ("l", "Inspect", "Log", Log),
            ("b", "Branch", "Switch local branch", Branches),
            ("c", "Commit", "Create commit", Commit),
            ("C", "Commit", "Amend / fixup", Menu('C')),
            ("p", "Network", "Push", Push),
            ("P", "Network", "Pull", Pull),
            ("f", "Network", "Fetch", Fetch),
            ("z", "Change", "Stash", Menu('z')),
            ("B", "Branch", "Branch operations", Menu('B')),
            ("t", "Tag", "Tags", Menu('t')),
            ("M", "History", "Merge", Menu('M')),
            ("r", "History", "Rebase", Menu('r')),
            ("x", "History", "Cherry-pick", Menu('x')),
            ("v", "History", "Revert", Menu('v')),
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
        'B' => vec![
            ("c", "Create", "create", Workflow(CreateBranch)),
            ("s", "Create", "create and switch", Workflow(CreateSwitch)),
            ("r", "Edit", "Rename current", Workflow(RenameBranch)),
            ("d", "Delete", "Delete merged", Workflow(DeleteBranch)),
        ],
        't' => vec![
            ("c", "Create", "Lightweight tag", Workflow(Tag)),
            ("l", "Inspect", "List", Tags),
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
        'M' => vec![
            ("m", "Merge", "Merge revision", Workflow(Merge)),
            ("s", "Merge", "Squash", Workflow(Squash)),
            ("c", "Sequence", "Continue", Workflow(MergeContinue)),
            ("a", "Sequence", "Abort", Workflow(MergeAbort)),
        ],
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
    StashUntracked,
    StashAll,
    CommitAll,
    CommitEmpty,
    CommitNoVerify,
    CommitResetAuthor,
    CommitSignoff,
}
impl MenuOption {
    pub fn menu(self) -> char {
        match self {
            Self::StashUntracked | Self::StashAll => 'z',
            _ => 'C',
        }
    }
    pub fn argument(self) -> &'static str {
        match self {
            Self::StashUntracked => "--include-untracked",
            Self::StashAll => "--all",
            Self::CommitAll => "--all",
            Self::CommitEmpty => "--allow-empty",
            Self::CommitNoVerify => "--no-verify",
            Self::CommitResetAuthor => "--reset-author",
            Self::CommitSignoff => "--signoff",
        }
    }
}
pub fn menu_arguments(ed: &Editor, menu: char) -> Vec<String> {
    let mut args: Vec<_> = ed
        .magit_options
        .iter()
        .filter(|option| option.menu() == menu)
        .map(|option| option.argument().to_owned())
        .collect();
    args.sort();
    args
}

/// A draft's execution arguments are also its displayed transient state.
pub fn sync_commit_options(ed: &mut Editor) {
    ed.magit_options.retain(|option| option.menu() != 'C');
    for (_, _, _, action) in menu_entries('C') {
        if let Action::ToggleOption(option) = action
            && ed.commit_args.iter().any(|arg| arg == option.argument())
        {
            ed.magit_options.insert(option);
        }
    }
}
