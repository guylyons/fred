//! vim key handling: Normal and Visual-line mode parsing and dispatch.

pub mod insert;
pub mod motion;
pub mod ops;

use crate::editor::{Editor, Mode};
use crate::ex::ExEffect;
use crate::key::{Key, KeyCode};
use motion::{Motion, Target};

#[derive(Debug, Default)]
pub struct State {
    pub(crate) pending: Vec<Key>,
    pub last_find: Option<(char, char)>,
    /// Keys of the last change, replayed by `.`.
    pub last_change: Vec<Key>,
    /// Keys of a change still in its insert session.
    pub recording: Option<Vec<Key>>,
    /// How many of the recorded keys are the command (the rest is the
    /// text typed in Insert mode).
    recording_split: usize,
    /// `last_change` entered Insert mode after this many keys.
    last_change_split: Option<usize>,
    replaying: bool,
    /// Operator waiting for the search line to finish (`d/pat<Enter>`).
    pub(crate) pending_op: Option<(char, Option<usize>)>,
    /// The last Insert-mode key was a typed `j` (a second one leaves Insert).
    pub(crate) after_j: bool,
}

impl State {
    /// A change that went through Insert mode is complete.
    pub(crate) fn finish_recording(&mut self, keys: Vec<Key>) {
        self.last_change = keys;
        self.last_change_split = Some(self.recording_split);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cmd {
    Move {
        count: Option<usize>,
        motion: Motion,
    },
    /// Operator `op` over a motion, or over whole lines when `motion` is None (`dd`).
    Op {
        op: char,
        count: Option<usize>,
        motion: Option<Motion>,
    },
    Simple {
        count: Option<usize>,
        key: Key,
        arg: Option<char>,
    },
    /// Operator over a `/` or `?` search: opens the search line.
    OpSearch {
        op: char,
        count: Option<usize>,
        kind: char,
    },
}

enum Parse<T> {
    Incomplete,
    Invalid,
    Done(T),
}

fn count(keys: &[Key]) -> (Option<usize>, usize) {
    let mut i = 0;
    let mut n: Option<usize> = None;
    while let Some(c) = keys.get(i).and_then(Key::char) {
        let Some(d) = c.to_digit(10) else { break };
        if d == 0 && n.is_none() {
            break;
        }
        n = Some(
            n.unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(d as usize)
                .min(1_000_000),
        );
        i += 1;
    }
    (n, i)
}

fn mul(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(1).saturating_mul(b.unwrap_or(1)).min(1_000_000)),
    }
}

fn parse_motion(keys: &[Key]) -> Parse<(Motion, usize)> {
    let Some(k) = keys.first() else {
        return Parse::Incomplete;
    };
    let arg = |used: usize, f: &dyn Fn(char) -> Motion| match keys.get(1) {
        None => Parse::Incomplete,
        Some(a) => match a.char() {
            Some(c) => Parse::Done((f(c), used)),
            None => Parse::Invalid,
        },
    };
    let m = match k.code {
        _ if k.alt => return Parse::Invalid,
        KeyCode::Left | KeyCode::Backspace => Motion::Left,
        KeyCode::Right => Motion::Right,
        KeyCode::Up => Motion::Up,
        KeyCode::Down => Motion::Down,
        KeyCode::Home => Motion::LineStart,
        KeyCode::End => Motion::LineEnd,
        KeyCode::Enter => Motion::NextLine,
        KeyCode::Char('n') if k.ctrl => Motion::Down,
        KeyCode::Char('p') if k.ctrl => Motion::Up,
        _ if k.ctrl => return Parse::Invalid,
        KeyCode::Char(c) => match c {
            'h' => Motion::Left,
            'l' => Motion::Right,
            'j' => Motion::Down,
            'k' => Motion::Up,
            'w' => Motion::WordFwd(false),
            'W' => Motion::WordFwd(true),
            'b' => Motion::WordBack(false),
            'B' => Motion::WordBack(true),
            'e' => Motion::WordEnd(false),
            'E' => Motion::WordEnd(true),
            '0' => Motion::LineStart,
            '^' => Motion::FirstNonBlank,
            '$' => Motion::LineEnd,
            'G' => Motion::GotoLine,
            '+' => Motion::NextLine,
            '-' => Motion::PrevLine,
            '}' => Motion::ParaFwd,
            '{' => Motion::ParaBack,
            ';' => Motion::RepeatFind { reverse: false },
            ',' => Motion::RepeatFind { reverse: true },
            'n' => Motion::SearchNext { reverse: false },
            'N' => Motion::SearchNext { reverse: true },
            'g' => {
                return match keys.get(1) {
                    None => Parse::Incomplete,
                    Some(k2) if k2.char() == Some('g') => Parse::Done((Motion::FileStart, 2)),
                    Some(_) => Parse::Invalid,
                };
            }
            'f' | 'F' | 't' | 'T' => return arg(2, &|ch| Motion::Find { kind: c, ch }),
            '\'' => return arg(2, &|ch| Motion::Mark(ch)),
            _ => return Parse::Invalid,
        },
        _ => return Parse::Invalid,
    };
    Parse::Done((m, 1))
}

/// Commands that take one character argument.
const ARG_CMDS: &[char] = &['r', 'm', 'Z', ' ', 'g'];
const SIMPLE: &[char] = &[
    'x', 'X', 's', 'S', 'J', 'p', 'P', 'o', 'O', 'i', 'a', 'I', 'A', 'u', 'D', 'C', 'Y', 'V', ':',
    '/', '?', '.', 'r', 'm', 'Z', ' ', 'g',
];

fn parse(keys: &[Key]) -> Parse<Cmd> {
    let (c1, mut i) = count(keys);
    let Some(k) = keys.get(i) else {
        return Parse::Incomplete;
    };
    if let Some(op @ ('d' | 'c' | 'y')) = k.char() {
        i += 1;
        let (c2, j) = count(&keys[i..]);
        i += j;
        let count = mul(c1, c2);
        let Some(k2) = keys.get(i) else {
            return Parse::Incomplete;
        };
        if let Some(kind @ ('/' | '?')) = k2.char() {
            return Parse::Done(Cmd::OpSearch { op, count, kind });
        }
        if k2.char() == Some(op) {
            return Parse::Done(Cmd::Op {
                op,
                count,
                motion: None,
            });
        }
        return match parse_motion(&keys[i..]) {
            Parse::Done((motion, _)) => Parse::Done(Cmd::Op {
                op,
                count,
                motion: Some(motion),
            }),
            Parse::Incomplete => Parse::Incomplete,
            Parse::Invalid => Parse::Invalid,
        };
    }
    match parse_motion(&keys[i..]) {
        Parse::Done((motion, _)) => return Parse::Done(Cmd::Move { count: c1, motion }),
        Parse::Incomplete => return Parse::Incomplete,
        Parse::Invalid => {}
    }
    let simple_ctrl = k.ctrl
        && matches!(
            k.code,
            KeyCode::Char('r' | 'd' | 'u' | 'f' | 'b' | '^' | '6')
        );
    let simple_key = matches!(
        k.code,
        KeyCode::PageUp | KeyCode::PageDown | KeyCode::Delete
    );
    match k.char() {
        Some(c) if SIMPLE.contains(&c) => {
            if ARG_CMDS.contains(&c) {
                return match keys.get(i + 1) {
                    None => Parse::Incomplete,
                    // `Space Enter` saves: Enter is the leader's one non-char key.
                    Some(a) => match a
                        .char()
                        .or((c == ' ' && a.is(KeyCode::Enter)).then_some('\n'))
                    {
                        Some(ch) => Parse::Done(Cmd::Simple {
                            count: c1,
                            key: *k,
                            arg: Some(ch),
                        }),
                        None => Parse::Invalid,
                    },
                };
            }
            Parse::Done(Cmd::Simple {
                count: c1,
                key: *k,
                arg: None,
            })
        }
        _ if simple_ctrl || simple_key => Parse::Done(Cmd::Simple {
            count: c1,
            key: *k,
            arg: None,
        }),
        _ => Parse::Invalid,
    }
}

/// Handle a key in Normal or Visual-line mode.
pub fn normal_key(ed: &mut Editor, k: Key) {
    if k.is(KeyCode::Esc) || k == Key::ctrl('c') {
        ed.vim.pending.clear();
        if matches!(ed.mode, Mode::VisualLine { .. }) {
            ed.mode = Mode::Normal;
        }
        return;
    }
    if let Mode::VisualLine { anchor } = ed.mode
        && ed.vim.pending.is_empty()
        && let Some(c) = k.char()
        && VISUAL.contains(&c)
    {
        visual(ed, anchor, c);
        return;
    }
    ed.vim.pending.push(k);
    match parse(&ed.vim.pending) {
        Parse::Incomplete => {}
        Parse::Invalid => ed.vim.pending.clear(),
        Parse::Done(cmd) => {
            let keys = std::mem::take(&mut ed.vim.pending);
            if matches!(ed.mode, Mode::VisualLine { .. }) {
                let allowed = match &cmd {
                    Cmd::Move { .. } => true,
                    Cmd::Simple { key, .. } => {
                        key.ctrl || key.char() == Some('m') || !matches!(key.code, KeyCode::Char(_))
                    }
                    Cmd::Op { .. } | Cmd::OpSearch { .. } => false,
                };
                if allowed {
                    execute(ed, cmd);
                }
                return;
            }
            run_change(ed, cmd, keys);
        }
    }
}

pub use insert::insert_key;

/// Keys that act on the selection in Visual-line mode.
const VISUAL: &[char] = &[
    'd', 'x', 'X', 'D', 'y', 'Y', 'c', 's', 'S', 'C', 'J', ':', 'V', 'o', 'K',
];

fn visual(ed: &mut Editor, anchor: usize, c: char) {
    if ed.generated() && !matches!(c, 'y' | 'Y' | ':' | 'V' | 'o') {
        ed.mode = Mode::Normal;
        ed.set_err("generated Git buffer is read-only");
        return;
    }
    let (lo, hi) = (anchor.min(ed.cur.line), anchor.max(ed.cur.line));
    ed.mode = Mode::Normal;
    match c {
        'V' => {}
        'o' => {
            ed.mode = Mode::VisualLine {
                anchor: ed.cur.line,
            };
            ed.set_line_keep_col(anchor);
        }
        ':' => {
            ed.marks.insert('<', lo);
            ed.marks.insert('>', hi);
            ed.open_cmdline(':', "'<,'>");
        }
        'K' => {
            ed.marks.insert('<', lo);
            ed.marks.insert('>', hi);
            ed.run_ex("'<,'>explain");
        }
        'J' => {
            ed.undo.begin(ed.cur.pos());
            ops::join(ed, lo, (hi - lo + 1).max(2));
            ed.undo.end(ed.cur.pos());
        }
        _ => {
            let op = match c {
                'y' | 'Y' => 'y',
                'c' | 's' | 'S' | 'C' => 'c',
                _ => 'd',
            };
            ed.undo.begin(ed.cur.pos());
            if op == 'y' {
                ed.set_cursor(lo, 0);
            }
            ops::apply_op(ed, op, ops::Span::Lines(lo, hi));
            if ed.mode != Mode::Insert {
                ed.undo.end(ed.cur.pos());
            }
        }
    }
}

/// Commands that change text (and so are repeated by `.`).
fn is_change(cmd: &Cmd) -> bool {
    match cmd {
        Cmd::Op { op, .. } => *op != 'y',
        // Applied when the search line finishes; not repeatable with `.`.
        Cmd::OpSearch { .. } => false,
        Cmd::Simple { key, .. } => {
            !key.ctrl
                && matches!(
                    key.char(),
                    Some(
                        'x' | 'X'
                            | 's'
                            | 'S'
                            | 'J'
                            | 'p'
                            | 'P'
                            | 'o'
                            | 'O'
                            | 'i'
                            | 'a'
                            | 'I'
                            | 'A'
                            | 'D'
                            | 'C'
                            | 'r'
                    )
                )
        }
        Cmd::Move { .. } => false,
    }
}

/// Run a command as one undo step, recording it for `.` if it changes text.
fn run_change(ed: &mut Editor, cmd: Cmd, keys: Vec<Key>) {
    if ed.generated() && (is_change(&cmd) || matches!(cmd, Cmd::OpSearch { op: 'd' | 'c', .. })) {
        ed.set_err("generated Git buffer is read-only");
        return;
    }
    if !is_change(&cmd) {
        execute(ed, cmd);
        return;
    }
    ed.undo.begin(ed.cur.pos());
    execute(ed, cmd);
    if ed.mode == Mode::Insert {
        if !ed.vim.replaying {
            ed.vim.recording_split = keys.len();
            ed.vim.recording = Some(keys);
        }
    } else {
        ed.undo.end(ed.cur.pos());
        if !ed.vim.replaying {
            ed.vim.last_change = keys;
            ed.vim.last_change_split = None;
        }
    }
}

/// `.`: replay the last change, with `count` replacing its own count.
fn repeat(ed: &mut Editor, count: Option<usize>) {
    if ed.vim.last_change.is_empty() || ed.vim.replaying {
        return;
    }
    let mut keys = ed.vim.last_change.clone();
    let mut split = ed.vim.last_change_split;
    if let Some(n) = count {
        let (_, used) = self::count(&keys);
        keys.drain(..used);
        let digits: Vec<Key> = n.to_string().chars().map(Key::ch).collect();
        split = split.map(|sp| sp - used + digits.len());
        keys.splice(0..0, digits);
    }
    ed.vim.replaying = true;
    ed.undo.begin(ed.cur.pos());
    let (command, typed) = keys.split_at(split.unwrap_or(keys.len()).min(keys.len()));
    for &k in command {
        ed.handle_key(k);
    }
    // The text typed in Insert mode is only replayed if the command got
    // there again (its motion can fail, e.g. `cfx` with no `x` left);
    // otherwise those keys would run as Normal-mode commands.
    if split.is_some() && ed.mode == Mode::Insert {
        for &k in typed {
            ed.handle_key(k);
        }
    }
    if ed.mode == Mode::Insert {
        insert::leave(ed);
    }
    ed.undo.end(ed.cur.pos());
    ed.vim.replaying = false;
}

fn move_to(ed: &mut Editor, t: Target) {
    if t.keep_col {
        ed.set_line_keep_col(t.line);
    } else {
        ed.set_cursor(t.line, t.byte);
        if t.eol {
            ed.cur.want_col = usize::MAX;
        }
    }
}

pub(crate) fn execute(ed: &mut Editor, cmd: Cmd) {
    match cmd {
        Cmd::Move { count, motion } => {
            if let Some(t) = motion::target(ed, motion, count) {
                move_to(ed, t);
            }
        }
        Cmd::Op { op, count, motion } => {
            if let Some(sp) = ops::span(ed, op, count, motion) {
                ops::apply_op(ed, op, sp);
            }
        }
        Cmd::Simple { count, key, arg } => simple(ed, count, key, arg),
        Cmd::OpSearch { op, count, kind } => {
            ed.vim.pending_op = Some((op, count));
            ed.open_cmdline(kind, "");
        }
    }
}

/// Finish `d/pat<Enter>`: apply the pending operator up to the match.
pub(crate) fn finish_op_search(ed: &mut Editor, op: char, count: Option<usize>) {
    let start = ed.cur.pos();
    let Some(t) = ed.search_target_n(false, count.unwrap_or(1)) else {
        return;
    };
    let (from, to) = if t < start { (t, start) } else { (start, t) };
    ed.undo.begin(start);
    ops::apply_op(ed, op, ops::Span::Chars(from, to));
    if ed.mode != Mode::Insert {
        ed.undo.end(ed.cur.pos());
    }
}

fn op(ed: &mut Editor, op: char, count: Option<usize>, motion: Option<Motion>) {
    execute(ed, Cmd::Op { op, count, motion });
}

fn simple(ed: &mut Editor, count: Option<usize>, key: Key, arg: Option<char>) {
    let half = (ed.win_height / 2).max(1);
    let n = count.unwrap_or(1).max(1);
    match (key.code, key.ctrl) {
        // Undo/redo would corrupt the history while a change is open.
        (KeyCode::Char('r'), true) | (KeyCode::Char('u'), false) if ed.undo.in_group() => {}
        // Ctrl-^ (Ctrl-6 on most terminals): the buffer before this one.
        (KeyCode::Char('^' | '6'), true) => {
            ed.pending_effect = Some(ExEffect::Buffer {
                cmd: crate::ex::BufCmd::Go,
                arg: "#".into(),
                force: false,
            })
        }
        (KeyCode::Char('r'), true) => {
            for i in 0..n {
                match ed.undo.redo(&mut ed.buf) {
                    Some(p) => ed.set_cursor(p.0, p.1),
                    None => {
                        if i == 0 {
                            ed.set_err("already at newest change");
                        }
                        break;
                    }
                }
            }
        }
        (KeyCode::Char('u'), false) => {
            for i in 0..n {
                match ed.undo.undo(&mut ed.buf) {
                    Some(p) => ed.set_cursor(p.0, p.1),
                    None => {
                        if i == 0 {
                            ed.set_err("already at oldest change");
                        }
                        break;
                    }
                }
            }
        }
        (KeyCode::Char('.'), false) => repeat(ed, count),
        (KeyCode::Char('x'), false) | (KeyCode::Delete, _) => {
            op(ed, 'd', count, Some(Motion::Right))
        }
        (KeyCode::Char('X'), false) => op(ed, 'd', count, Some(Motion::Left)),
        (KeyCode::Char('D'), false) => op(ed, 'd', count, Some(Motion::LineEnd)),
        (KeyCode::Char('C'), false) => op(ed, 'c', count, Some(Motion::LineEnd)),
        (KeyCode::Char('Y'), false) => op(ed, 'y', count, None),
        (KeyCode::Char('S'), false) => op(ed, 'c', count, None),
        (KeyCode::Char('s'), false) => {
            if ed.buf.line_len(ed.cur.line) == 0 {
                ed.mode = Mode::Insert;
            } else {
                op(ed, 'c', count, Some(Motion::Right));
            }
        }
        (KeyCode::Char('p'), false) => ops::put(ed, n, true),
        (KeyCode::Char('P'), false) => ops::put(ed, n, false),
        (KeyCode::Char('J'), false) => {
            ops::join(ed, ed.cur.line, n);
        }
        (KeyCode::Char('r'), false) => {
            if let Some(c) = arg {
                ops::replace_chars(ed, n, c);
            }
        }
        (KeyCode::Char('o'), false) => ops::open_line(ed, true),
        (KeyCode::Char('O'), false) => ops::open_line(ed, false),
        (KeyCode::Char(c @ ('i' | 'a' | 'I' | 'A')), false) => {
            let line = ed.buf.line(ed.cur.line);
            let b = match c {
                'i' => ed.cur.byte,
                'a' if line.is_empty() => 0,
                'a' => crate::text::next_grapheme(&line, ed.cur.byte),
                'I' => ed.first_nonblank(ed.cur.line),
                _ => line.len(),
            };
            ed.mode = Mode::Insert;
            ed.cur.byte = b;
        }
        (KeyCode::Char('V'), false) => {
            ed.mode = Mode::VisualLine {
                anchor: ed.cur.line,
            }
        }
        (KeyCode::Char('d'), true) | (KeyCode::PageDown, _) | (KeyCode::Char('f'), true) => {
            let n = count.unwrap_or(if matches!(key.code, KeyCode::Char('d')) {
                half
            } else {
                ed.win_height.max(1)
            });
            ed.set_line_keep_col(ed.cur.line.saturating_add(n));
        }
        (KeyCode::Char('u'), true) | (KeyCode::PageUp, _) | (KeyCode::Char('b'), true) => {
            let n = count.unwrap_or(if matches!(key.code, KeyCode::Char('u')) {
                half
            } else {
                ed.win_height.max(1)
            });
            ed.set_line_keep_col(ed.cur.line.saturating_sub(n));
        }
        (KeyCode::Char(':'), false) => ed.open_cmdline(':', ""),
        // Space is the leader: `Space s` zaps to words, `Space p` finds files, `Space g` greps,
        // `Space k` searches this file's lines, `Space j` browses files,
        // `Space r` lists recent files, `Space b` lists open buffers,
        // `Space B` searches their lines, `Space d` goes to a definition,
        // `Space -` lists the file's directory (dired), `Space Enter` saves,
        // `Space ;` closes.
        (KeyCode::Char(' '), false) => match arg {
            Some('s') => crate::zap::open(ed),
            Some('p') => crate::pick::open(ed, crate::pick::Kind::Files),
            Some('g') => crate::pick::open(ed, crate::pick::Kind::Grep),
            Some('k') => crate::pick::open(ed, crate::pick::Kind::Lines),
            Some('r') => crate::pick::open(ed, crate::pick::Kind::Recent),
            Some('d') => crate::pick::definition(ed),
            Some(c @ ('b' | 'B')) => {
                ed.pending_effect = Some(ExEffect::Buffer {
                    cmd: if c == 'b' {
                        crate::ex::BufCmd::List
                    } else {
                        crate::ex::BufCmd::Search
                    },
                    arg: String::new(),
                    force: false,
                })
            }
            // Dired on the file's directory, the cursor on the file.
            Some('-') => {
                let me = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
                let dir = me.as_deref().and_then(std::path::Path::parent).map_or_else(
                    || std::path::PathBuf::from("."),
                    std::path::Path::to_path_buf,
                );
                ed.pending_effect = Some(ExEffect::Open {
                    path: dir,
                    line: 0,
                    col: 0,
                    pattern: me
                        .as_deref()
                        .and_then(std::path::Path::file_name)
                        .map(|n| n.to_string_lossy().into_owned()),
                });
            }
            // Find-file from the listed directory, else the current file's
            // directory, else the cwd.
            Some('j') => {
                let dir = match &ed.dired {
                    Some(d) => d.dir.clone(),
                    None => ed
                        .path
                        .as_deref()
                        .and_then(|p| std::path::absolute(p).ok())
                        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                        .unwrap_or_else(|| std::path::PathBuf::from(".")),
                };
                crate::pick::browse(ed, &dir);
            }
            Some('\n') => {
                ed.pending_effect = Some(ExEffect::Write {
                    path: None,
                    force: false,
                    range: None,
                    then_quit: false,
                })
            }
            Some(';') => ed.pending_effect = Some(ExEffect::Quit { force: false }),
            _ => {}
        },
        (KeyCode::Char(c @ ('/' | '?')), false) => ed.open_cmdline(c, ""),
        // `gd`, as in vim (`gg` is a motion).
        (KeyCode::Char('g'), false) if arg == Some('d') => crate::pick::definition(ed),
        // ZZ writes if modified and quits; ZQ quits without writing.
        (KeyCode::Char('Z'), false) => match arg {
            Some('Z') => ed.pending_effect = Some(ExEffect::WriteIfModifiedQuit),
            Some('Q') => ed.pending_effect = Some(ExEffect::Quit { force: true }),
            _ => {}
        },
        (KeyCode::Char('m'), false) => {
            if let Some(c) = arg.filter(char::is_ascii_lowercase) {
                ed.marks.insert(c, ed.cur.line);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
