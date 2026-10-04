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
    Stashes,
    Tags,
    Workflow(workflows::Operation),
    Submit(repo::Repo, workflows::Operation, String),
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
    Patch(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
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
    if ed.mode != Mode::Normal {
        return false;
    }
    if ed.vim.pending.len() == 3 && ed.vim.pending[..2] == [Key::ch(' '), Key::ch('m')] {
        let menu = ed.vim.pending[2].char().unwrap_or_default();
        ed.vim.pending.clear();
        if k.is(KeyCode::Esc) || k == Key::ctrl('g') || k == Key::ctrl('c') {
            ed.msg = None;
            return true;
        }
        if k.char() == Some('l') && matches!(menu, 'z' | 't') {
            ed.pending_effect = Some(ExEffect::Magit(if menu == 'z' {
                Action::Stashes
            } else {
                Action::Tags
            }));
        } else if let Some(operation) = workflow_key(menu, k.char().unwrap_or_default()) {
            ed.pending_effect = Some(ExEffect::Magit(Action::Workflow(operation)));
        } else {
            ed.set_err("unknown Git menu key");
        }
        return true;
    }
    if ed.vim.pending == [Key::ch(' ')] && k.char() == Some('m') {
        ed.vim.pending.push(k);
        ed.set_msg(HELP);
        return true;
    }
    if ed.vim.pending == [Key::ch(' '), Key::ch('m')] {
        if let Some(help) = k.char().and_then(menu_help) {
            ed.vim.pending.push(k);
            ed.set_msg(help);
            return true;
        }
        ed.vim.pending.clear();
        if k.is(KeyCode::Esc) || k == Key::ctrl('g') || k == Key::ctrl('c') {
            ed.msg = None;
            return true;
        }
        let action = match k.char() {
            Some('s') => Some(Action::Status),
            Some('p') => Some(Action::Push),
            Some('P') => Some(Action::Pull),
            Some('f') => Some(Action::Fetch),
            Some('c') => Some(Action::Commit),
            Some('l') => Some(Action::Log),
            Some('b') => Some(Action::Branches),
            _ => None,
        };
        if let Some(a) = action {
            ed.pending_effect = Some(ExEffect::Magit(a));
        } else {
            ed.set_err(HELP);
        }
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
        'z' => "Stash: z save  u include untracked  i staged  k keep index  a apply  l list",
        'B' => "Branch: c create  s create and switch  r rename current  d delete merged",
        't' => "Tag: c create lightweight tag  l list",
        'C' => "Commit: a amend without editing message  f fixup",
        'M' => "Merge: m merge  s squash  c continue  a abort",
        'r' => "Rebase: r onto revision  c continue  s skip  a abort",
        'x' => "Cherry-pick: p pick  c continue  s skip  a abort",
        'v' => "Revert: v revert  c continue  s skip  a abort",
        _ => return None,
    })
}
fn workflow_key(menu: char, key: char) -> Option<workflows::Operation> {
    use workflows::Operation::*;
    Some(match (menu, key) {
        ('z', 'z') => Stash,
        ('z', 'u') => StashUntracked,
        ('z', 'i') => StashStaged,
        ('z', 'k') => StashKeepIndex,
        ('z', 'a') => StashApply,
        ('B', 'c') => CreateBranch,
        ('B', 's') => CreateSwitch,
        ('B', 'r') => RenameBranch,
        ('B', 'd') => DeleteBranch,
        ('t', 'c') => Tag,
        ('C', 'a') => Amend,
        ('C', 'f') => Fixup,
        ('M', 'm') => Merge,
        ('M', 's') => Squash,
        ('M', 'c') => MergeContinue,
        ('M', 'a') => MergeAbort,
        ('r', 'r') => Rebase,
        ('r', 'c') => RebaseContinue,
        ('r', 's') => RebaseSkip,
        ('r', 'a') => RebaseAbort,
        ('x', 'p') => CherryPick,
        ('x', 'c') => CherryContinue,
        ('x', 's') => CherrySkip,
        ('x', 'a') => CherryAbort,
        ('v', 'v') => Revert,
        ('v', 'c') => RevertContinue,
        ('v', 's') => RevertSkip,
        ('v', 'a') => RevertAbort,
        _ => return None,
    })
}
