//! Editor state and key routing.

use crate::buffer::{Buffer, Edit};
use crate::complete::index::WordIndex;
use crate::complete::{self, Popup};
use crate::ex::{self, ExEffect, ExState};
use crate::key::{Key, KeyCode};
use crate::search;
use crate::text;
use crate::undo::Undo;
use crate::vim;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    /// Byte offset into the line, always on a grapheme boundary.
    pub byte: usize,
    /// Screen column that j/k try to keep (`usize::MAX` = end of line).
    pub want_col: usize,
}

impl Cursor {
    pub fn pos(&self) -> (usize, usize) {
        (self.line, self.byte)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    VisualLine {
        anchor: usize,
    },
    Command(CmdLine),
    /// File or grep picker (`Space p`, `Space g`).
    Pick(Box<crate::pick::Picker>),
}

/// The `:`, `/` or `?` line being typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CmdLine {
    pub kind: char,
    /// Shown before the text in place of `kind` (dired's questions).
    pub prompt: String,
    pub text: String,
    /// Byte offset of the cursor in `text`.
    pub cursor: usize,
    hist: Option<usize>,
    stash: String,
    /// Tab completion in progress: candidates, current index, text before Tab.
    pub(crate) comp: Option<(Vec<String>, usize, String)>,
}

impl CmdLine {
    /// Keys that edit the text: typing, Backspace, Delete, Ctrl-W, Ctrl-U,
    /// and cursor movement.
    pub fn edit(&mut self, k: Key) {
        match k.code {
            KeyCode::Backspace if self.cursor > 0 => {
                let p = prev_char(&self.text, self.cursor);
                self.text.replace_range(p..self.cursor, "");
                self.cursor = p;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                let n = next_char(&self.text, self.cursor);
                self.text.replace_range(self.cursor..n, "");
            }
            KeyCode::Left => self.cursor = prev_char(&self.text, self.cursor),
            KeyCode::Right => self.cursor = next_char(&self.text, self.cursor),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Char('u') if k.ctrl => {
                self.text.replace_range(..self.cursor, "");
                self.cursor = 0;
            }
            KeyCode::Char('w') if k.ctrl => {
                let start = text::word_start_before(&self.text, self.cursor);
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            KeyCode::Char(c) if !k.ctrl && !k.alt => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    pub linewise: bool,
}

pub struct Editor {
    pub buf: Buffer,
    pub undo: Undo,
    pub cur: Cursor,
    pub mode: Mode,
    pub reg: Register,
    pub marks: HashMap<char, usize>,
    pub last_pat: Option<String>,
    pub last_search_fwd: bool,
    /// Status-row message and whether it is an error.
    pub msg: Option<(String, bool)>,
    pub path: Option<PathBuf>,
    pub readonly: bool,
    /// This buffer lists a directory (see `dired`).
    pub dired: Option<Box<crate::dired::Dired>>,
    pub magit: Option<Box<crate::magit::View>>,
    /// Hidden line ranges (Org visibility).
    pub folds: crate::fold::Folds,
    /// Org-mode state when this buffer visits an Org file.
    pub org: Option<Box<crate::org::Org>>,
    /// A generated Org buffer (agenda and other views).
    pub org_view: Option<Box<crate::org::View>>,
    /// An Org prompt or menu's continuation.
    pub org_then: Option<crate::org::Then>,
    pub org_require_match: bool,
    /// The open Org menu's entries (keys, label) and keys typed so far.
    pub org_menu: Vec<(String, String)>,
    pub org_menu_typed: String,
    /// An Org key sequence in progress (`C-c C-x`).
    pub org_keys: Vec<Key>,
    /// The prefix argument for the next Org command (`Space u`).
    pub org_arg: crate::org::Prefix,
    /// The active region (Visual-line selection) for an Org command.
    pub org_region: Option<(usize, usize)>,
    /// A special Org buffer's name (`*Org Note*`), its C-c C-c action
    /// and the buffer to return to.
    pub org_buffer_name: Option<String>,
    pub org_finish: Option<crate::org::FinishSlot>,
    pub org_return: Option<usize>,
    /// A path that only decides syntax highlighting (Org source edit buffers).
    pub syntax_path: Option<PathBuf>,
    /// The key Enter answers in the open Org menu (fast selection's RET).
    pub org_menu_enter: Option<String>,
    pub magit_input_generation: u64,
    pub magit_options: std::collections::HashSet<crate::magit::MenuOption>,
    /// Free-form transient-option values by (menu, argument prefix).
    pub magit_values: std::collections::BTreeMap<(char, &'static str), String>,
    /// Menus whose default arguments have been applied in this buffer.
    pub magit_seeded: std::collections::HashSet<char>,
    /// magit-blame-mode on this file buffer.
    pub blame: Option<crate::magit::blame::Blame>,
    /// magit-blob-mode: this buffer shows REV:FILE.
    pub blob: Option<crate::magit::blob::Blob>,
    /// git-rebase-mode: this buffer is an interactive rebase's todo list.
    pub rebase_todo: Option<crate::magit::rebase::Plan>,
    /// evil-collection-magit-toggle-text-mode: todo keys off, plain Vim on.
    pub rebase_text: bool,
    pub commit_args: Vec<String>,
    /// git-commit-prev-message position and the draft text it replaced.
    pub commit_history: Option<(usize, String)>,
    pub magit_menu: Option<char>,
    pub magit_prompt: Option<crate::magit::Prompt>,
    /// magit-revision-stack: (commit, repository toplevel), newest last.
    pub revision_stack: Vec<(String, std::path::PathBuf)>,
    pub commit_mode: crate::magit::CommitMode,
    pub commit_repo: Option<crate::magit::repo::Repo>,
    /// File operation requested by an ex command, performed by the app.
    pub pending_effect: Option<ExEffect>,
    pub win_height: usize,
    pub(crate) viewport: Option<crate::zap::Viewport>,
    pub(crate) zap: Option<crate::zap::Zap>,
    pub tabstop: usize,
    /// Insert spaces for Tab, this many per indent level (0 = insert a tab).
    pub indent_spaces: usize,
    /// Undo state of the text as last saved; `u64::MAX` = never matches.
    pub saved_state: u64,
    pub(crate) vim: vim::State,
    /// Completion menu (Insert mode).
    pub popup: Option<Popup>,
    /// `:explain`: Claude's explanation of these lines, shown over them
    /// until Esc or a click outside it (or on its ✕).
    pub explain: Option<(ex::addr::Range, String)>,
    pub autocomplete: bool,
    /// Share yanks and puts with the system clipboard.
    pub clipboard: bool,
    /// Words from nearby files, filled in by a background thread.
    pub nearby: Arc<Mutex<Vec<String>>>,
    pub(crate) word_index: WordIndex,
    /// Lines changed against git's staging area.
    pub git: crate::git::Gutter,
    /// The project the pickers search; outlives `:e`.
    pub project: Arc<crate::pick::Project>,
    cmd_history: Vec<String>,
    search_history: Vec<String>,
}

impl Editor {
    pub fn new(buf: Buffer) -> Editor {
        let indent_spaces = detect_indent(&buf);
        Editor {
            buf,
            undo: Undo::default(),
            cur: Cursor::default(),
            mode: Mode::Normal,
            reg: Register::default(),
            marks: HashMap::new(),
            last_pat: None,
            last_search_fwd: true,
            msg: None,
            path: None,
            readonly: false,
            dired: None,
            magit: None,
            folds: crate::fold::Folds::default(),
            org: None,
            org_view: None,
            org_then: None,
            org_require_match: false,
            org_menu: vec![],
            org_menu_typed: String::new(),
            org_keys: vec![],
            org_arg: crate::org::Prefix::None,
            org_region: None,
            org_buffer_name: None,
            org_finish: None,
            org_return: None,
            org_menu_enter: None,
            syntax_path: None,
            magit_input_generation: 0,
            magit_options: std::collections::HashSet::new(),
            magit_values: std::collections::BTreeMap::new(),
            magit_seeded: std::collections::HashSet::new(),
            blame: None,
            blob: None,
            rebase_todo: None,
            rebase_text: false,
            commit_args: vec![],
            commit_history: None,
            magit_menu: None,
            magit_prompt: None,
            revision_stack: vec![],
            commit_mode: crate::magit::CommitMode::New,
            commit_repo: None,
            pending_effect: None,
            win_height: 12,
            viewport: None,
            zap: None,
            tabstop: 8,
            indent_spaces,
            saved_state: 0,
            vim: vim::State::default(),
            popup: None,
            explain: None,
            autocomplete: true,
            clipboard: false,
            nearby: Arc::default(),
            word_index: WordIndex::default(),
            git: crate::git::Gutter::default(),
            project: Arc::default(),
            cmd_history: vec![],
            search_history: vec![],
        }
    }

    /// Take over from `old` what belongs to the session rather than one
    /// file: the register, last search, histories and project.
    pub fn inherit(&mut self, old: &mut Editor) {
        self.reg = std::mem::take(&mut old.reg);
        self.revision_stack = std::mem::take(&mut old.revision_stack);
        self.last_pat = old.last_pat.take();
        self.last_search_fwd = old.last_search_fwd;
        self.project = Arc::clone(&old.project);
        self.cmd_history = std::mem::take(&mut old.cmd_history);
        self.search_history = std::mem::take(&mut old.search_history);
        self.win_height = old.win_height;
    }

    pub fn handle_key(&mut self, k: Key) {
        self.magit_input_generation = self.magit_input_generation.wrapping_add(1);
        if crate::magit::key(self, k)
            || crate::org::key(self, k)
            || crate::magit::blame::key(self, k)
            || crate::magit::blob::key(self, k)
            || crate::magit::rebase::key(self, k)
        {
            return;
        }
        // A Shift key nothing took is the plain key.
        if k.shift {
            return self.handle_key(Key { shift: false, ..k });
        }
        // An Alt key nothing took: Fred's usual Esc, key.
        if k.alt {
            self.handle_key(Key::new(KeyCode::Esc));
            return self.handle_key(Key { alt: false, ..k });
        }
        if self.zap.is_some() {
            crate::zap::key(self, k);
            return;
        }
        if self.mode == Mode::Normal
            && self.vim.pending.is_empty()
            && crate::ui::splash::active(self)
            && !k.ctrl
            && !k.alt
        {
            match k.code {
                KeyCode::Char('e') => {
                    vim::normal_key(self, Key::ch('i'));
                    return;
                }
                KeyCode::Char('a') => {
                    self.open_cmdline(':', "ai ");
                    return;
                }
                KeyCode::Char('c') => {
                    self.pending_effect = Some(ExEffect::Open {
                        path: crate::config::config_path(),
                        line: 0,
                        col: 0,
                        pattern: None,
                    });
                    return;
                }
                KeyCode::Char('q') => {
                    self.run_ex("q");
                    return;
                }
                KeyCode::Char('f') => {
                    crate::pick::open(self, crate::pick::Kind::Files);
                    return;
                }
                KeyCode::Char('g') => {
                    crate::pick::open(self, crate::pick::Kind::Grep);
                    return;
                }
                KeyCode::Char('r') => {
                    crate::pick::open(self, crate::pick::Kind::Recent);
                    return;
                }
                KeyCode::Char('v') => {
                    self.set_msg(concat!("fred ", env!("CARGO_PKG_VERSION")));
                    return;
                }
                KeyCode::Char(c @ '1'..='5') => {
                    if let Some(path) =
                        crate::ui::splash::recent(self).get((c as u8 - b'1') as usize)
                    {
                        self.pending_effect = Some(ExEffect::Open {
                            path: path.clone(),
                            line: 0,
                            col: 0,
                            pattern: None,
                        });
                    }
                    return;
                }
                _ => {}
            }
        }
        if self.explain.is_some()
            && self.mode == Mode::Normal
            && self.vim.pending.is_empty()
            && k.is(KeyCode::Esc)
        {
            self.explain = None;
            return;
        }
        if !matches!(self.mode, Mode::Command(_)) {
            self.msg = None;
        }
        let was_insert = self.mode == Mode::Insert;
        match self.mode {
            Mode::Normal | Mode::VisualLine { .. } => {
                if !crate::dired::key(self, k) {
                    vim::normal_key(self, k)
                }
            }
            Mode::Insert => vim::insert_key(self, k),
            Mode::Command(_) => self.cmdline_key(k),
            Mode::Pick(_) => crate::pick::pick_key(self, k),
        }
        self.clamp_cursor();
        // Undo/redo in Visual-line mode can delete the anchor's line.
        if let Mode::VisualLine { anchor } = &mut self.mode {
            *anchor = (*anchor).min(self.buf.len_lines() - 1);
        }
        if self.mode == Mode::Insert && !was_insert {
            self.word_index.ensure(&self.buf, true);
        }
        if self.mode != Mode::Insert {
            self.popup = None;
        }
        self.sync_marks();
        if self.folds.hidden(self.cur.line) {
            self.reveal_cursor();
        }
        if !self.undo.in_group() {
            self.buf.modified = self.undo.state_id() != self.saved_state;
        }
    }

    /// Generated Git text (status/log/diff views and blobs) is never edited or saved.
    pub fn generated(&self) -> bool {
        self.magit.is_some() || self.blob.is_some()
    }

    /// Bracketed paste: insert text as-is (no autoindent or completion).
    pub fn paste(&mut self, text: &str) {
        self.magit_input_generation = self.magit_input_generation.wrapping_add(1);
        if self.generated()
            && matches!(
                self.mode,
                Mode::Normal | Mode::Insert | Mode::VisualLine { .. }
            )
        {
            self.set_err("generated Git buffer is read-only");
            return;
        }
        if self.zap.is_some() {
            crate::zap::paste(self, text);
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        match &mut self.mode {
            Mode::Command(cl) => {
                cl.comp = None;
                let first = text.lines().next().unwrap_or("");
                cl.text.insert_str(cl.cursor, first);
                cl.cursor += first.len();
            }
            Mode::Pick(_) => {
                for c in text.lines().next().unwrap_or("").chars() {
                    crate::pick::pick_key(self, Key::ch(c));
                }
            }
            Mode::Insert => {
                vim::ops::insert_text(self, &text);
                self.popup = None;
            }
            Mode::Normal => {
                self.undo.begin(self.cur.pos());
                vim::ops::insert_text(self, &text);
                self.undo.end(self.cur.pos());
                self.clamp_cursor();
                if !self.undo.in_group() {
                    self.buf.modified = self.undo.state_id() != self.saved_state;
                }
            }
            Mode::VisualLine { .. } => {}
        }
        self.sync_marks();
    }

    /// Move marks with their lines; a deleted line loses its mark.
    /// Show the hidden text the cursor moved into.
    pub fn reveal_cursor(&mut self) {
        if self.org.is_some() {
            crate::org::fold::show_context(self, self.cur.line);
        } else if let Some((s, e)) = self.folds.range_at(self.cur.line) {
            self.folds.show(s, e);
        }
    }

    pub(crate) fn sync_marks(&mut self) {
        for ch in self.buf.take_line_changes() {
            self.folds.line_change(ch.at, ch.removed, ch.inserted);
            if let Some(o) = &mut self.org {
                o.specs.line_change(ch.at, ch.removed, ch.inserted);
            }
            self.marks
                .retain(|_, l| *l < ch.at || *l >= ch.at + ch.removed);
            for l in self.marks.values_mut() {
                if *l >= ch.at + ch.removed {
                    *l = *l + ch.inserted - ch.removed;
                }
            }
        }
    }

    /// Mark the current text as saved.
    pub fn mark_saved(&mut self) {
        self.saved_state = self.undo.state_id();
        self.buf.modified = false;
    }

    pub fn apply(&mut self, e: Edit) {
        let inv = self.buf.apply(e);
        self.undo.record(inv);
    }

    /// Claude's reply to `:ai` in place of lines `r`, as one undo step.
    pub fn ai_reply(&mut self, r: ex::addr::Range, out: &str) {
        let lines: Vec<String> = ex::unfence(out).lines().map(String::from).collect();
        self.undo.begin((self.cur.line, self.cur.byte));
        vim::ops::splice_lines(self, r.start, r.end - r.start + 1, &lines);
        let last = (r.start + lines.len().saturating_sub(1)).min(self.line_count() - 1);
        self.undo.end((last, 0));
        self.set_cursor(last, self.first_nonblank(last));
    }

    pub fn set_msg(&mut self, m: impl Into<String>) {
        self.msg = Some((m.into(), false));
    }

    pub fn set_err(&mut self, m: impl Into<String>) {
        self.msg = Some((format!("? {}", m.into()), true));
    }

    pub fn line_count(&self) -> usize {
        self.buf.len_lines()
    }

    pub fn first_nonblank(&self, line: usize) -> usize {
        let l = self.buf.line(line);
        l.len() - l.trim_start_matches([' ', '\t']).len()
    }

    /// Byte of the last grapheme on the line (0 on an empty line).
    pub fn last_grapheme(&self, line: usize) -> usize {
        let l = self.buf.line(line);
        text::prev_grapheme(&l, l.len())
    }

    /// Move the cursor and remember its screen column.
    pub fn set_cursor(&mut self, line: usize, byte: usize) {
        self.cur.line = line.min(self.line_count() - 1);
        let l = self.buf.line(self.cur.line);
        self.cur.byte = text::floor_grapheme(&l, byte);
        self.cur.want_col = text::col_of_byte(&l, self.cur.byte, self.tabstop);
    }

    /// Move to `line`, keeping the remembered screen column.
    pub fn set_line_keep_col(&mut self, line: usize) {
        self.cur.line = line.min(self.line_count() - 1);
        let l = self.buf.line(self.cur.line);
        self.cur.byte = if self.cur.want_col == usize::MAX {
            text::prev_grapheme(&l, l.len())
        } else {
            text::byte_of_col(&l, self.cur.want_col, self.tabstop)
        };
    }

    pub fn clamp_cursor(&mut self) {
        let n = self.line_count();
        if self.cur.line >= n {
            self.cur.line = n - 1;
        }
        let l = self.buf.line(self.cur.line);
        let max = match self.mode {
            Mode::Insert => l.len(),
            _ => text::prev_grapheme(&l, l.len()),
        };
        self.cur.byte = text::floor_grapheme(&l, self.cur.byte.min(max));
    }

    // ---- command line ----

    pub fn open_cmdline(&mut self, kind: char, text: &str) {
        self.mode = Mode::Command(CmdLine {
            kind,
            prompt: String::new(),
            text: text.into(),
            cursor: text.len(),
            hist: None,
            stash: String::new(),
            comp: None,
        });
    }

    /// Ask a question on the command line; the answer goes to dired.
    pub fn open_prompt(&mut self, prompt: &str, text: &str) {
        self.open_cmdline('@', text);
        if let Mode::Command(cl) = &mut self.mode {
            cl.prompt = prompt.into();
        }
    }

    fn cmdline_key(&mut self, k: Key) {
        let Mode::Command(cl) = &mut self.mode else {
            return;
        };
        if cl.kind == '=' && k == Key::ctrl('g') {
            self.magit_prompt = None;
            self.mode = Mode::Normal;
            return;
        }
        if k.is(KeyCode::Esc) && cl.comp.take().is_some() {
            return;
        }
        if !matches!(k.code, KeyCode::Tab | KeyCode::BackTab) {
            cl.comp = None;
        }
        let leaving = matches!(k.code, KeyCode::Esc)
            || k == Key::ctrl('c')
            || (k.code == KeyCode::Backspace && cl.text.is_empty());
        if leaving {
            self.magit_prompt = None;
            self.org_then = None;
            self.vim.pending_op = None;
        }
        let hist = if cl.kind == ':' {
            &self.cmd_history
        } else {
            &self.search_history
        };
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char('c') if k.ctrl => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let cl = std::mem::take(cl);
                self.mode = Mode::Normal;
                self.run_cmdline(cl.kind, &cl.text);
            }
            KeyCode::Backspace if cl.text.is_empty() => self.mode = Mode::Normal,
            KeyCode::Up | KeyCode::Down if cl.kind != '=' => {
                let len = hist.len();
                let next = match (k.code, cl.hist) {
                    (KeyCode::Up, None) if len > 0 => Some(len - 1),
                    (KeyCode::Up, Some(i)) => Some(i.saturating_sub(1)),
                    (KeyCode::Down, Some(i)) if i + 1 < len => Some(i + 1),
                    (KeyCode::Down, Some(_)) => None,
                    (_, h) => h,
                };
                if cl.hist.is_none() {
                    cl.stash = cl.text.clone();
                }
                if next != cl.hist || next.is_none() {
                    cl.text = next.map_or_else(|| cl.stash.clone(), |i| hist[i].clone());
                    cl.cursor = cl.text.len();
                    cl.hist = next;
                }
            }
            KeyCode::Tab | KeyCode::BackTab => self.complete_cmdline(k.code == KeyCode::BackTab),
            // magit-whitespace-disallowed: a space in a branch name is a dash.
            KeyCode::Char(' ')
                if !k.ctrl
                    && !k.alt
                    && self
                        .magit_prompt
                        .as_ref()
                        .is_some_and(crate::magit::reads_branch_name) =>
            {
                cl.edit(Key::ch('-'))
            }
            _ => cl.edit(k),
        }
    }

    /// Tab on the command line (filled in by completion).
    fn complete_cmdline(&mut self, back: bool) {
        let Mode::Command(cl) = &mut self.mode else {
            return;
        };
        if cl.kind != ':' {
            return;
        }
        let (items, i, base) = match cl.comp.take() {
            Some((items, i, base)) => {
                let next = if back {
                    (i + items.len()) % (items.len() + 1)
                } else {
                    (i + 1) % (items.len() + 1)
                };
                (items, next, base)
            }
            None => {
                let cwd = std::env::current_dir().unwrap_or_default();
                let items = complete::cmdline_candidates(&cl.text, &cwd);
                if items.is_empty() {
                    return;
                }
                let i = if back { items.len() - 1 } else { 0 };
                (items, i, cl.text.clone())
            }
        };
        cl.text = items.get(i).cloned().unwrap_or_else(|| base.clone());
        cl.cursor = cl.text.len();
        cl.comp = Some((items, i, base));
    }

    fn run_cmdline(&mut self, kind: char, text: &str) {
        if kind == '=' {
            return crate::magit::answer(self, text);
        }
        if kind == '@' {
            return crate::dired::answer(self, text);
        }
        if kind == 'o' {
            return crate::org::answer(self, text);
        }
        let hist = if kind == ':' {
            &mut self.cmd_history
        } else {
            &mut self.search_history
        };
        if !text.is_empty() && hist.last().map(String::as_str) != Some(text) {
            hist.push(text.to_string());
        }
        match kind {
            ':' => self.run_ex(text),
            _ => {
                if !text.is_empty() {
                    self.last_pat = Some(text.to_string());
                }
                self.last_search_fwd = kind == '/';
                match self.vim.pending_op.take() {
                    Some((op, count)) => vim::finish_op_search(self, op, count),
                    None => self.search_next(false),
                }
            }
        }
    }

    /// Run an ex command line.
    pub fn run_ex(&mut self, text: &str) {
        if crate::org::ex(self, text) {
            return;
        }
        if self.generated() {
            let mut buf = self.buf.clone();
            let mut undo = Undo::default();
            let mut st = ExState::new(
                &mut buf,
                &mut undo,
                self.cur.line,
                &self.marks,
                &mut self.last_pat,
            );
            let result = ex::run(&mut st, text);
            let line = st.cur;
            if buf.version != self.buf.version {
                self.set_err("generated Git buffer is read-only");
                return;
            }
            match result {
                Ok(eff) => {
                    self.set_cursor(line.min(self.line_count() - 1), 0);
                    if eff != ExEffect::None {
                        self.pending_effect = Some(eff);
                    }
                }
                Err(e) => self.set_err(e),
            }
            return;
        }
        let line = self.cur.line;
        let before = self.undo.state_id();
        let mut st = ExState::new(
            &mut self.buf,
            &mut self.undo,
            line,
            &self.marks,
            &mut self.last_pat,
        );
        st.file = self.path.clone();
        let r = ex::run(&mut st, text);
        let new_cur = st.cur;
        match r {
            Ok(eff) => {
                if new_cur != line || self.undo.state_id() != before {
                    let b = self.first_nonblank(new_cur.min(self.line_count() - 1));
                    self.set_cursor(new_cur, b);
                }
                if eff != ExEffect::None {
                    self.pending_effect = Some(eff);
                }
            }
            Err(e) => self.set_err(e),
        }
    }

    /// Jump to the next match of the last pattern (`reverse` flips direction).
    pub fn search_next(&mut self, reverse: bool) {
        if let Some(p) = self.search_target(reverse) {
            self.set_cursor(p.0, p.1);
        }
    }

    pub fn search_target(&mut self, reverse: bool) -> Option<(usize, usize)> {
        self.search_target_n(reverse, 1)
    }

    /// The `count`-th match of the last pattern from the cursor. The regex is
    /// compiled once, and once the matches start repeating (the search wraps
    /// around the file) whole laps are skipped, so a huge count is cheap.
    pub fn search_target_n(&mut self, reverse: bool, count: usize) -> Option<(usize, usize)> {
        let Some(pat) = self.last_pat.clone() else {
            self.set_err("no previous pattern");
            return None;
        };
        let re = match search::compile(&pat) {
            Ok(re) => re,
            Err(e) => {
                self.set_err(e);
                return None;
            }
        };
        let fwd = self.last_search_fwd != reverse;
        let count = count.max(1);
        let mut p = self.cur.pos();
        let mut first = None;
        let mut wrapped = false;
        let mut i = 0;
        while i < count {
            let Some(q) = search::find(&self.buf, &re, p, fwd, true) else {
                self.set_err(format!("pattern not found: {pat}"));
                return None;
            };
            wrapped |= if fwd { q <= p } else { q >= p };
            i += 1;
            match first {
                None => first = Some(q),
                Some(f) if f == q => {
                    // A full lap of `i - 1` matches: skip the remaining laps.
                    let lap = i - 1;
                    i = count - (count - i) % lap;
                }
                _ => {}
            }
            p = q;
        }
        if wrapped {
            self.set_msg(if fwd {
                "search wrapped to top"
            } else {
                "search wrapped to bottom"
            });
        }
        Some(p)
    }
}

fn prev_char(s: &str, i: usize) -> usize {
    s[..i].char_indices().last().map_or(0, |(j, _)| j)
}

fn next_char(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

/// Spaces per indent level if the file indents with spaces, else 0 (tabs).
fn detect_indent(buf: &Buffer) -> usize {
    let (mut tabs, mut spaces, mut width) = (0, 0, 0usize);
    for i in 0..buf.len_lines().min(1000) {
        let l = buf.line(i);
        if l.starts_with('\t') {
            tabs += 1;
        } else {
            let n = l.len() - l.trim_start_matches(' ').len();
            // One leading space is alignment (e.g. ` * ` comments), not indent.
            if n >= 2 && n < l.len() {
                spaces += 1;
                width = gcd(width, n);
            }
        }
    }
    match (spaces > tabs, width) {
        (false, _) => 0,
        (true, w @ 2..=8) => w,
        (true, _) => 4,
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::key::parse_keys;

    pub(crate) fn ed(text: &str, keys: &str) -> Editor {
        let mut e = Editor::new(Buffer::from_text(text));
        for k in parse_keys(keys) {
            e.handle_key(k);
        }
        e
    }

    #[test]
    fn basic_motions() {
        assert_eq!(ed("abc def", "w").cur.pos(), (0, 4));
        assert_eq!(ed("abc def", "$").cur.pos(), (0, 6));
        assert_eq!(ed("abc def\nx", "$j").cur.pos(), (1, 0));
        assert_eq!(ed("abc def\nxyz", "$jk").cur.pos(), (0, 6));
        assert_eq!(ed("a\nb\nc", "G").cur.pos(), (2, 0));
        assert_eq!(ed("a\nb\nc", "G2G").cur.pos(), (1, 0));
        assert_eq!(ed("a\nb\nc", "Ggg").cur.pos(), (0, 0));
        assert_eq!(ed("foo.bar baz", "w").cur.pos(), (0, 3));
        assert_eq!(ed("foo.bar baz", "W").cur.pos(), (0, 8));
        assert_eq!(ed("foo bar", "e").cur.pos(), (0, 2));
        assert_eq!(ed("foo bar", "ee").cur.pos(), (0, 6));
        assert_eq!(ed("foo bar", "$b").cur.pos(), (0, 4));
        assert_eq!(ed("foo\n  bar", "w").cur.pos(), (1, 2));
        assert_eq!(ed("foo\n\nbar", "w").cur.pos(), (1, 0));
        assert_eq!(ed("foo\nbar", "jb").cur.pos(), (0, 0));
        assert_eq!(ed("a b c", "fc").cur.pos(), (0, 4));
        assert_eq!(ed("a b c", "tc").cur.pos(), (0, 3));
        assert_eq!(ed("a,b,c", "f,;").cur.pos(), (0, 3));
        assert_eq!(ed("a,b,c", "$F,,").cur.pos(), (0, 3));
        assert_eq!(ed("a,b,c", "2f,").cur.pos(), (0, 3));
        assert_eq!(ed("ab\n\ncd", "}").cur.pos(), (1, 0));
        assert_eq!(ed("ab\n\ncd", "G{").cur.pos(), (1, 0));
        assert_eq!(ed("  x", "^").cur.pos(), (0, 2));
        assert_eq!(ed("  x", "$0").cur.pos(), (0, 0));
        assert_eq!(ed("abc", "lll").cur.pos(), (0, 2));
        assert_eq!(ed("abc", "3lh").cur.pos(), (0, 1));
        assert_eq!(ed("a\nb\nc", "2j").cur.pos(), (2, 0));
        assert_eq!(ed("a\nb\nc", "9j").cur.pos(), (2, 0));
        assert_eq!(ed("a\nb", "<Down><Right>").cur.pos(), (1, 0));
    }

    #[test]
    fn search_motions() {
        assert_eq!(ed("abc\nfoo\nabc", "/abc<Enter>").cur.pos(), (2, 0));
        assert_eq!(ed("abc\nfoo\nabc", "/abc<Enter>n").cur.pos(), (0, 0));
        assert_eq!(ed("abc\nfoo\nabc", "/abc<Enter>N").cur.pos(), (0, 0));
        assert_eq!(ed("abc\nfoo\nabc", "?foo<Enter>").cur.pos(), (1, 0));
        assert_eq!(ed("x abc", "/b<Enter>").cur.pos(), (0, 3));
        let e = ed("abc", "/zzz<Enter>");
        assert_eq!(e.cur.pos(), (0, 0));
        assert!(e.msg.unwrap().1);
    }

    #[test]
    fn marks_motion() {
        assert_eq!(ed("a\n  b\nc", "jmaG'a").cur.pos(), (1, 2));
    }

    #[test]
    fn grapheme_motion() {
        assert_eq!(ed("e\u{301}漢x", "l").cur.pos(), (0, 3));
        assert_eq!(ed("e\u{301}漢x", "ll").cur.pos(), (0, 6));
        assert_eq!(ed("e\u{301}漢x", "$h").cur.pos(), (0, 3));
        // j/k keep the screen column across wide chars
        assert_eq!(ed("漢字x\nabcde", "$j").cur.pos(), (1, 4));
        assert_eq!(ed("abcd\n漢字", "lllj").cur.pos(), (1, 3));
    }

    #[test]
    fn half_page() {
        let t = (0..30)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut e = Editor::new(Buffer::from_text(&t));
        e.win_height = 10;
        e.handle_key(crate::key::Key::ctrl('d'));
        assert_eq!(e.cur.line, 5);
        e.handle_key(crate::key::Key::ctrl('u'));
        assert_eq!(e.cur.line, 0);
    }

    #[test]
    fn empty_buffer_motions_dont_panic() {
        for k in [
            "j", "k", "G", "gg", "w", "b", "e", "W", "B", "E", "$", "0", "^", "}", "{", "n", "N",
            "x", "X", "dd", "p", "P", "J", "u", "<C-r>", "D", "C<Esc>", "fz", ";", "'a", "o<Esc>",
            "dw", "db", "de", "cw<Esc>", "yy", "Vd", "~", "r", ".",
        ] {
            ed("", k);
            ed("\n\n", k);
        }
    }
}
