//! vim key handling: Normal and Visual-line mode parsing and dispatch.

pub mod motion;

use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use motion::{Motion, Target};

#[derive(Debug, Default)]
pub struct State {
    pending: Vec<Key>,
    pub last_find: Option<(char, char)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cmd {
    Move { count: Option<usize>, motion: Motion },
    /// Operator `op` over a motion, or over whole lines when `motion` is None (`dd`).
    Op { op: char, count: Option<usize>, motion: Option<Motion> },
    Simple { count: Option<usize>, key: Key, arg: Option<char> },
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
        n = Some(n.unwrap_or(0).saturating_mul(10).saturating_add(d as usize).min(1_000_000));
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
    let Some(k) = keys.first() else { return Parse::Incomplete };
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
            'l' | ' ' => Motion::Right,
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
const ARG_CMDS: &[char] = &['r', 'm'];
const SIMPLE: &[char] = &['x', 'X', 's', 'S', 'J', 'p', 'P', 'o', 'O', 'i', 'a', 'I', 'A', 'u', 'D', 'C', 'Y', 'V', ':', '/', '?', '.', 'r', 'm'];

fn parse(keys: &[Key]) -> Parse<Cmd> {
    let (c1, mut i) = count(keys);
    let Some(k) = keys.get(i) else { return Parse::Incomplete };
    if let Some(op @ ('d' | 'c' | 'y')) = k.char() {
        i += 1;
        let (c2, j) = count(&keys[i..]);
        i += j;
        let count = mul(c1, c2);
        let Some(k2) = keys.get(i) else { return Parse::Incomplete };
        if k2.char() == Some(op) {
            return Parse::Done(Cmd::Op { op, count, motion: None });
        }
        return match parse_motion(&keys[i..]) {
            Parse::Done((motion, _)) => Parse::Done(Cmd::Op { op, count, motion: Some(motion) }),
            Parse::Incomplete => Parse::Incomplete,
            Parse::Invalid => Parse::Invalid,
        };
    }
    match parse_motion(&keys[i..]) {
        Parse::Done((motion, _)) => return Parse::Done(Cmd::Move { count: c1, motion }),
        Parse::Incomplete => return Parse::Incomplete,
        Parse::Invalid => {}
    }
    let simple_ctrl = k.ctrl && matches!(k.code, KeyCode::Char('r' | 'd' | 'u' | 'f' | 'b'));
    let simple_key = matches!(k.code, KeyCode::PageUp | KeyCode::PageDown | KeyCode::Delete);
    match k.char() {
        Some(c) if SIMPLE.contains(&c) => {
            if ARG_CMDS.contains(&c) {
                return match keys.get(i + 1) {
                    None => Parse::Incomplete,
                    Some(a) => match a.char() {
                        Some(ch) => Parse::Done(Cmd::Simple { count: c1, key: *k, arg: Some(ch) }),
                        None => Parse::Invalid,
                    },
                };
            }
            Parse::Done(Cmd::Simple { count: c1, key: *k, arg: None })
        }
        _ if simple_ctrl || simple_key => Parse::Done(Cmd::Simple { count: c1, key: *k, arg: None }),
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
    ed.vim.pending.push(k);
    match parse(&ed.vim.pending) {
        Parse::Incomplete => {}
        Parse::Invalid => ed.vim.pending.clear(),
        Parse::Done(cmd) => {
            ed.vim.pending.clear();
            execute(ed, cmd);
        }
    }
}

/// Handle a key in Insert mode.
pub fn insert_key(_ed: &mut Editor, _k: Key) {}

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
        Cmd::Op { .. } => {}
        Cmd::Simple { count, key, arg } => simple(ed, count, key, arg),
    }
}

fn simple(ed: &mut Editor, count: Option<usize>, key: Key, _arg: Option<char>) {
    let half = (ed.win_height / 2).max(1);
    match (key.code, key.ctrl) {
        (KeyCode::Char('d'), true) | (KeyCode::PageDown, _) | (KeyCode::Char('f'), true) => {
            let n = count.unwrap_or(if matches!(key.code, KeyCode::Char('d')) { half } else { ed.win_height.max(1) });
            ed.set_line_keep_col(ed.cur.line + n);
        }
        (KeyCode::Char('u'), true) | (KeyCode::PageUp, _) | (KeyCode::Char('b'), true) => {
            let n = count.unwrap_or(if matches!(key.code, KeyCode::Char('u')) { half } else { ed.win_height.max(1) });
            ed.set_line_keep_col(ed.cur.line.saturating_sub(n));
        }
        (KeyCode::Char(':'), false) => ed.open_cmdline(':', ""),
        (KeyCode::Char(c @ ('/' | '?')), false) => ed.open_cmdline(c, ""),
        (KeyCode::Char('m'), false) => {
            if let Some(c) = _arg.filter(char::is_ascii_lowercase) {
                ed.marks.insert(c, ed.cur.line);
            }
        }
        _ => {}
    }
}
