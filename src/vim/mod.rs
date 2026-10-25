//! vim key handling: Normal and Visual-line mode parsing and dispatch.

pub mod excmd;
pub mod insert;
pub mod marks;
pub mod motion;
pub mod ops;
pub mod register;
pub(crate) mod select;
pub(crate) mod textobj;

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
    pub(crate) pending_op: Option<(char, Option<usize>, Option<char>)>,
    /// The last Insert-mode key was a typed `j` (a second one leaves Insert).
    pub(crate) after_j: bool,
    /// Where this Insert session began: `CTRL-W` and `CTRL-U` stop there
    /// once, as with Neovim's default 'backspace'.
    pub(crate) insert_start: (usize, usize),
    /// The Visual selection is in Select mode (Neovim's `VIsual_select`).
    pub select: bool,
    /// The selection began in Insert mode and goes back to it when it ends
    /// without a change (Neovim's `restart_edit`).
    pub(crate) restart_insert: bool,
    /// The line that Insert was left at the end of, for that restart.
    pub(crate) insert_left_eol: Option<usize>,
    /// Select mode is replacing or deleting: the unnamed register gets the
    /// text, the system clipboard doesn't (as in other Mac editors).
    pub(crate) keep_clipboard: bool,
    /// The register `"x` named for the command being run.
    pub(crate) register: Option<char>,
    /// The delete goes to `"1` even within a line (motions `% ( ) / ? n N
    /// { }`).
    pub(crate) numbered_delete: bool,
    /// A command failed (Vim's beep): a running macro stops.
    pub(crate) failed: bool,
    /// The text typed in the last Insert session (`".`).
    pub(crate) last_insert: String,
    /// `Editor::handle_key` calls in progress.
    pub(crate) key_depth: usize,
    /// CTRL-A writes hex digits in upper case (the last ones seen were).
    pub(crate) hexupper: bool,
    /// Replace mode (`R`): what each typed character replaced (None: it
    /// was added), for `<BS>` to put back.
    pub(crate) replace: Option<Vec<Option<String>>>,
    /// `3ix<Esc>`: the insert command and its count, to type the text
    /// again when Insert mode ends.
    pub(crate) insert_repeat: Option<(char, usize)>,
    /// The last Visual selection (`gv`): its mode and cursor.
    pub(crate) last_visual: Option<(Mode, (usize, usize))>,
    /// Where Insert mode last ended (`gi`, Vim's `'^` mark).
    pub(crate) insert_stop: Option<(usize, usize)>,
    /// An Insert-mode key waiting for the next one (CTRL-R, CTRL-V, CTRL-G).
    pub(crate) ins_pending: Option<insert::Pending>,
    /// The previous context mark, jumplist and changelist.
    pub(crate) lists: marks::Lists,
    /// CTRL-O: one Normal-mode command, then back to Insert mode (at the
    /// line's end if it was there).
    pub(crate) insert_once: Option<(bool, usize)>,
}

/// The text a run of Insert-mode keys typed.
pub(crate) fn typed_text(keys: &[Key]) -> String {
    let mut text = String::new();
    for k in keys {
        match (k.code, k.ctrl) {
            (KeyCode::Char(c), false) => text.push(c),
            (KeyCode::Enter, _) => text.push('\n'),
            (KeyCode::Tab, _) => text.push('\t'),
            (KeyCode::Backspace, _) | (KeyCode::Char('h'), true) => {
                text.pop();
            }
            _ => {}
        }
    }
    text
}

impl State {
    /// A change that went through Insert mode is complete.
    pub(crate) fn finish_recording(&mut self, keys: Vec<Key>) {
        let text = typed_text(&keys[self.recording_split.min(keys.len())..]);
        self.last_insert = text;
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

fn parse_motion(keys: &[Key], objects: bool) -> Parse<(Motion, usize)> {
    let Some(k) = keys.first() else {
        return Parse::Incomplete;
    };
    // A motion that takes one more key: `fx`, `'a`, `iw`, `]]`.
    let arg = |f: &dyn Fn(char) -> Option<Motion>| match keys.get(1) {
        None => Parse::Incomplete,
        Some(a) => match a.char().and_then(f) {
            Some(m) => Parse::Done((m, 2)),
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
        KeyCode::Char('n' | 'j') if k.ctrl => Motion::Down,
        KeyCode::Char('p') if k.ctrl => Motion::Up,
        KeyCode::Char('h') if k.ctrl => Motion::Left,
        KeyCode::Char('m') if k.ctrl => Motion::NextLine,
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
            '_' => Motion::FirstNonBlankDown,
            '}' => Motion::ParaFwd,
            '{' => Motion::ParaBack,
            ')' => Motion::Sentence(true),
            '(' => Motion::Sentence(false),
            'H' => Motion::ScreenTop,
            'M' => Motion::ScreenMiddle,
            'L' => Motion::ScreenBottom,
            '|' => Motion::Column,
            '%' => Motion::Percent,
            '*' => Motion::Star {
                fwd: true,
                whole: true,
            },
            '#' => Motion::Star {
                fwd: false,
                whole: true,
            },
            ';' => Motion::RepeatFind { reverse: false },
            ',' => Motion::RepeatFind { reverse: true },
            'n' => Motion::SearchNext { reverse: false },
            'N' => Motion::SearchNext { reverse: true },
            // Without 'wrap' the screen-line motions are the line ones.
            'g' => {
                return arg(&|c| {
                    Some(match c {
                        'g' => Motion::FileStart,
                        'e' => Motion::WordEndBack(false),
                        'E' => Motion::WordEndBack(true),
                        '_' => Motion::LastNonBlank,
                        'o' => Motion::Byte,
                        '0' => Motion::LineStart,
                        '^' => Motion::FirstNonBlank,
                        '$' => Motion::LineEnd,
                        'j' => Motion::Down,
                        'k' => Motion::Up,
                        '*' => Motion::Star {
                            fwd: true,
                            whole: false,
                        },
                        '#' => Motion::Star {
                            fwd: false,
                            whole: false,
                        },
                        _ => return None,
                    })
                });
            }
            '[' | ']' => {
                let fwd = c == ']';
                return arg(&|a| match a {
                    '[' | ']' => Some(Motion::Section {
                        fwd,
                        what: if (a == '[') == fwd { '}' } else { '{' },
                    }),
                    '(' | '{' if !fwd => Some(Motion::Unmatched(a)),
                    ')' | '}' if fwd => Some(Motion::Unmatched(a)),
                    _ => None,
                });
            }
            'i' | 'a' if objects => {
                return arg(&|kind| {
                    "wWspbB()[]{}<>\"'`"
                        .contains(kind)
                        .then_some(Motion::Object {
                            include: c == 'a',
                            kind,
                        })
                });
            }
            'f' | 'F' | 't' | 'T' => return arg(&|ch| Some(Motion::Find { kind: c, ch })),
            '\'' => return arg(&|ch| Some(Motion::Mark(ch))),
            '`' => return arg(&|ch| Some(Motion::MarkExact(ch))),
            _ => return Parse::Invalid,
        },
        _ => return Parse::Invalid,
    };
    Parse::Done((m, 1))
}

/// An operator at the start of `keys`: its name (`d c y < > ~ u U ?`, the
/// last four for `g~ gu gU g?`) and the keys it took.
fn parse_op(keys: &[Key]) -> Parse<(char, usize)> {
    match keys.first().and_then(Key::char) {
        Some(c @ ('d' | 'c' | 'y' | '<' | '>')) if !keys[0].ctrl => Parse::Done((c, 1)),
        Some('g') if !keys[0].ctrl => match keys.get(1) {
            None => Parse::Incomplete,
            Some(k) => match k.char() {
                Some(c @ ('~' | 'u' | 'U' | '?')) if !k.ctrl => Parse::Done((c, 2)),
                _ => Parse::Invalid,
            },
        },
        _ => Parse::Invalid,
    }
}

/// Commands that take one character argument.
const ARG_CMDS: &[char] = &['r', 'm', 'Z', 'z', ' ', 'g', 'q', '@'];
const SIMPLE: &[char] = &[
    'x', 'X', 's', 'S', 'J', 'p', 'P', 'o', 'O', 'i', 'a', 'I', 'A', 'u', 'D', 'C', 'Y', 'V', 'v',
    '~', ':', '/', '?', '.', 'r', 'm', 'Z', 'z', ' ', 'g', 'q', '@', 'Q', 'R', '&',
];

/// A command, with the register `"x` before it named.
fn parse(keys: &[Key], visual: bool) -> Parse<(Cmd, Option<char>)> {
    let (c1, i) = count(keys);
    if keys.get(i) != Some(&Key::ch('"')) {
        return match parse_cmd(keys, visual) {
            Parse::Done(c) => Parse::Done((c, None)),
            Parse::Incomplete => Parse::Incomplete,
            Parse::Invalid => Parse::Invalid,
        };
    }
    let Some(r) = keys.get(i + 1) else {
        return Parse::Incomplete;
    };
    let Some(reg) = r.char().filter(|&c| register::valid(c)) else {
        return Parse::Invalid;
    };
    // A count on both sides of `"x` multiplies.
    let rest = &keys[i + 2..];
    let (c2, j) = count(rest);
    let mut cmd_keys: Vec<Key> = match mul(c1, c2) {
        Some(n) => n.to_string().chars().map(Key::ch).collect(),
        None => vec![],
    };
    cmd_keys.extend_from_slice(&rest[j..]);
    match parse_cmd(&cmd_keys, visual) {
        Parse::Done(c) => Parse::Done((c, Some(reg))),
        Parse::Incomplete => Parse::Incomplete,
        Parse::Invalid => Parse::Invalid,
    }
}

fn parse_cmd(keys: &[Key], visual: bool) -> Parse<Cmd> {
    let (c1, mut i) = count(keys);
    let Some(k) = keys.get(i) else {
        return Parse::Incomplete;
    };
    let op = if visual {
        Parse::Invalid
    } else {
        parse_op(&keys[i..])
    };
    match op {
        Parse::Incomplete => return Parse::Incomplete,
        Parse::Invalid => {}
        Parse::Done((op, used)) => {
            i += used;
            let (c2, j) = count(&keys[i..]);
            i += j;
            let count = mul(c1, c2);
            let Some(k2) = keys.get(i) else {
                return Parse::Incomplete;
            };
            // Doubled: `dd`, `>>`, `guu` and `gugu`.
            let linewise = Parse::Done(Cmd::Op {
                op,
                count,
                motion: None,
            });
            if k2.char() == Some(op) && !k2.ctrl {
                return linewise;
            }
            if let Some(kind @ ('/' | '?')) = k2.char() {
                return Parse::Done(Cmd::OpSearch { op, count, kind });
            }
            if used == 2 && k2.char() == Some('g') {
                match keys.get(i + 1) {
                    None => return Parse::Incomplete,
                    Some(k3) if k3.char() == Some(op) => return linewise,
                    _ => {}
                }
            }
            return match parse_motion(&keys[i..], true) {
                Parse::Done((motion, _)) => Parse::Done(Cmd::Op {
                    op,
                    count,
                    motion: Some(motion),
                }),
                Parse::Incomplete => Parse::Incomplete,
                Parse::Invalid => Parse::Invalid,
            };
        }
    }
    // CTRL-W and one more key: CTRL-W CTRL-J is CTRL-W j, arrows are h j k l.
    if *k == Key::ctrl('w') {
        let Some(a) = keys.get(i + 1) else {
            return Parse::Incomplete;
        };
        let c = match a.code {
            KeyCode::Left => 'h',
            KeyCode::Down => 'j',
            KeyCode::Up => 'k',
            KeyCode::Right => 'l',
            KeyCode::Char('6') if a.ctrl => '^',
            KeyCode::Char(c) => c,
            _ => return Parse::Invalid,
        };
        return Parse::Done(Cmd::Simple {
            count: c1,
            key: *k,
            arg: Some(c),
        });
    }
    match parse_motion(&keys[i..], visual) {
        Parse::Done((motion, _)) => return Parse::Done(Cmd::Move { count: c1, motion }),
        Parse::Incomplete => return Parse::Incomplete,
        Parse::Invalid => {}
    }
    let simple_ctrl = k.ctrl
        && matches!(
            k.code,
            KeyCode::Char('r' | 'd' | 'u' | 'f' | 'b' | '^' | '6' | 'a' | 'x' | 'o' | 'i')
        );
    let simple_key = matches!(
        k.code,
        KeyCode::PageUp | KeyCode::PageDown | KeyCode::Delete | KeyCode::Tab
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

/// What the `Space` keys do, for the which-key popup. Keep it in step with
/// the `Space` arm of `simple` (and Org's `Space o`, Magit's `Space m`).
const LEADER: &[(char, &str)] = &[
    ('s', "zap to a word"),
    ('p', "find a file"),
    ('j', "browse files here"),
    ('r', "recent files"),
    ('g', "grep"),
    ('k', "search this file's lines"),
    ('b', "switch buffer"),
    ('B', "search all buffers' lines"),
    ('d', "go to definition"),
    ('-', "dired this directory"),
    ('O', "show in the file manager"),
    ('w', "pick a window; c N: N windows in a grid"),
    ('i', "imenu: this file's functions, types…"),
    ('h', "help: search commands"),
    ('t', "terminal in a new pane"),
    ('c', "+claude"),
    ('m', "+magit"),
    ('o', "+org"),
    ('u', "universal argument"),
    ('\n', "save"),
    (';', "close"),
];

/// After `Space` (or `Space o`), its keys and what they do, for the
/// which-key popup: the prefix's name and the entries.
pub fn which_key(ed: &Editor) -> Option<(&'static str, Vec<(String, &'static str)>)> {
    if ed.mode != Mode::Normal {
        return None;
    }
    let claude: Vec<(char, &str)> = crate::claude::LEADER.iter().map(|l| (l.0, l.2)).collect();
    let (title, table) = match ed.vim.pending.as_slice() {
        [s] if *s == Key::ch(' ') => ("SPC", LEADER),
        [s, o] if *s == Key::ch(' ') && *o == Key::ch('o') => ("SPC o", crate::org::LEADER),
        [s, c] if *s == Key::ch(' ') && *c == Key::ch('c') => ("SPC c", &claude[..]),
        _ => return None,
    };
    let name = |c: char| {
        if c == '\n' {
            "RET".into()
        } else {
            c.to_string()
        }
    };
    Some((
        title,
        table.iter().map(|&(c, what)| (name(c), what)).collect(),
    ))
}

/// Handle a key in Normal or Visual-line mode.
pub fn normal_key(ed: &mut Editor, k: Key) {
    let k = if ed.vim.pending.is_empty() {
        match select::key(ed, k) {
            Some(k) => k,
            None => return,
        }
    } else {
        k
    };
    if k.is(KeyCode::Esc) || k == Key::ctrl('c') {
        ed.vim.pending.clear();
        ed.vim.register = None;
        if ed.visual_lines().is_some() {
            ed.mode = Mode::Normal;
        }
        return;
    }
    // `q` stops recording a macro.
    if ed.vim.pending.is_empty() && k == Key::ch('q') && ed.regs.rec.is_some() {
        register::stop_recording(ed);
        return;
    }
    if ed.visual_lines().is_some() && visual_key(ed, k) {
        return;
    }
    ed.vim.pending.push(k);
    match parse(&ed.vim.pending, ed.visual_lines().is_some()) {
        Parse::Incomplete => {}
        Parse::Invalid => ed.vim.pending.clear(),
        Parse::Done((cmd, reg)) => {
            let keys = std::mem::take(&mut ed.vim.pending);
            ed.vim.register = reg;
            if ed.visual_lines().is_some() {
                let allowed = match &cmd {
                    Cmd::Move { .. } => true,
                    Cmd::Simple { key, .. } => {
                        key.ctrl
                            || matches!(key.char(), Some('m' | 'z'))
                            || !matches!(key.code, KeyCode::Char(_))
                    }
                    Cmd::Op { .. } | Cmd::OpSearch { .. } => false,
                };
                if allowed {
                    execute(ed, cmd);
                }
                ed.vim.register = None;
                return;
            }
            run_change(ed, cmd, keys);
            ed.vim.register = None;
        }
    }
}

pub use insert::insert_key;

/// Keys that act on the selection in Visual and Visual-line mode.
const VISUAL: &[char] = &[
    'd', 'x', 'X', 'D', 'y', 'Y', 'c', 's', 'S', 'C', 'J', ':', 'V', 'v', 'o', 'K',
];

/// A key in Visual mode that acts on the selection, after the count, `"x`
/// or `g` that may be pending. False if it isn't one: the parser gets it.
fn visual_key(ed: &mut Editor, k: Key) -> bool {
    let (count, used) = count(&ed.vim.pending);
    let rest = ed.vim.pending[used..].to_vec();
    if rest == [Key::ch('"')] {
        ed.vim.pending.truncate(used);
        ed.vim.register = k.char().filter(|&c| register::valid(c));
        return true;
    }
    let g = rest == [Key::ch('g')];
    if !rest.is_empty() && !g {
        return false;
    }
    let mode = ed.mode.clone();
    match (g, k.char()) {
        _ if k == Key::ctrl('a') || k == Key::ctrl('x') => {
            visual_addsub(ed, k == Key::ctrl('x'), g, count)
        }
        (true, Some(c @ ('~' | 'u' | 'U' | '?' | 'J'))) => {
            visual_op(ed, if c == 'J' { 'j' } else { c }, count)
        }
        (true, _) => return false,
        (false, Some(c @ ('p' | 'P'))) => visual_put(ed, c == 'P'),
        (false, Some(c @ ('~' | 'u' | 'U' | '<' | '>'))) => visual_op(ed, c, count),
        (false, Some(c)) if VISUAL.contains(&c) => match mode {
            Mode::Visual { anchor } => visual_chars(ed, anchor, c),
            Mode::VisualLine { anchor } => visual(ed, anchor, c),
            _ => return false,
        },
        _ => return false,
    }
    ed.vim.pending.clear();
    ed.vim.register = None;
    true
}

/// CTRL-A and CTRL-X on the selection: the first number of each line;
/// with `g` (`g CTRL-A`) each one by `count` more than the one before.
fn visual_addsub(ed: &mut Editor, sub: bool, g: bool, count: Option<usize>) {
    let mode = ed.mode.clone();
    let cur = ed.cur.pos();
    let (from, to, linewise) = match mode {
        Mode::Visual { anchor } => (anchor.min(cur), anchor.max(cur), false),
        Mode::VisualLine { anchor } => ((anchor.min(cur.0), 0), (anchor.max(cur.0), 0), true),
        _ => return,
    };
    ed.mode = Mode::Normal;
    if ed.generated() {
        ed.set_err("generated Git buffer is read-only");
        return;
    }
    let amount = count.unwrap_or(1).max(1) as u64;
    let mut step = amount;
    let mut changed = 0;
    ed.undo.begin(cur);
    for l in from.0..=to.0 {
        let len = ed.buf.line_len(l);
        let (col, length) = if linewise {
            (0, len)
        } else {
            let col = if l == from.0 { from.1 } else { 0 };
            let end = if l == to.0 {
                to.1.min(len.saturating_sub(1)) + 1
            } else {
                len
            };
            (col, end.saturating_sub(col))
        };
        if ops::addsub(ed, l, col, Some(length), linewise, sub, step) {
            changed += 1;
            if g {
                step += amount;
            }
        }
    }
    ed.set_cursor(from.0, from.1);
    if changed > 2 {
        ed.set_msg(format!("{changed} lines changed"));
    }
    ed.undo.end(ed.cur.pos());
}

/// An operator on the selection: `~ u U ?` change case, `< >` shift
/// (`count` times) and `j` joins without spaces (`gJ`).
fn visual_op(ed: &mut Editor, op: char, count: Option<usize>) {
    let mode = ed.mode.clone();
    let Some(span) = selection(ed, &mode) else {
        return;
    };
    ed.mode = Mode::Normal;
    if ed.generated() {
        ed.set_err("generated Git buffer is read-only");
        return;
    }
    let (lo, hi) = match span {
        ops::Span::Lines(lo, hi) => (lo, hi),
        ops::Span::Chars(a, b) => (a.0, if b.1 == 0 && b.0 > a.0 { b.0 - 1 } else { b.0 }),
    };
    ed.undo.begin(ed.cur.pos());
    match op {
        '<' | '>' => ops::shift(ed, lo, hi, op == '<', count.unwrap_or(1).max(1)),
        'j' => {
            ops::join_with(ed, lo, (hi - lo + 1).max(2), false);
        }
        _ => {
            if let ops::Span::Chars(from, _) = span {
                ed.set_cursor(from.0, from.1);
            } else {
                ed.set_cursor(lo, ed.cur.byte);
            }
            ops::apply_op(ed, op, span);
        }
    }
    ed.undo.end(ed.cur.pos());
}

/// `p` and `P` in Visual mode: replace the selection with a register.
/// With `p` the replaced text goes to the unnamed register.
fn visual_put(ed: &mut Editor, keep: bool) {
    let mode = ed.mode.clone();
    let Some(span) = selection(ed, &mode) else {
        return;
    };
    ed.mode = Mode::Normal;
    if ed.generated() {
        ed.set_err("generated Git buffer is read-only");
        return;
    }
    let reg = match ed.vim.register.filter(|&c| c != '"') {
        Some(c) => register::get(ed, c),
        None => Some(ed.reg.clone()),
    };
    let Some(reg) = reg.filter(|r| r.linewise || !r.text.is_empty()) else {
        ed.set_err("E353: Nothing in register");
        return;
    };
    ed.undo.begin(ed.cur.pos());
    let name = ed.vim.register.take();
    let lines_left = match span {
        ops::Span::Lines(lo, hi) => hi - lo + 1 < ed.line_count(),
        _ => true,
    };
    if keep {
        ops::delete_black_hole(ed, span);
    } else {
        ops::apply_op(ed, 'd', span);
    }
    let saved = std::mem::replace(&mut ed.reg, reg.clone());
    let saved_clip = std::mem::replace(&mut ed.vim.keep_clipboard, true);
    match span {
        ops::Span::Lines(lo, _) => {
            let lines: Vec<String> = reg.text.split('\n').map(String::from).collect();
            if !lines_left {
                // The whole buffer was replaced: its one empty line goes.
                ops::splice_lines(ed, 0, 1, &lines);
                ed.set_cursor(0, ed.first_nonblank(0));
            } else {
                let at = lo.min(ed.line_count());
                ops::splice_lines(ed, at, 0, &lines);
                ed.set_cursor(at, ed.first_nonblank(at));
            }
        }
        ops::Span::Chars(from, _) => {
            if reg.linewise {
                // The lines go between the two halves of the line.
                ed.set_cursor(from.0, from.1);
                ed.cur.byte = from.1;
                ops::insert_text(ed, &format!("\n{}\n", reg.text));
                let at = from.0 + 1;
                ed.set_cursor(at, ed.first_nonblank(at));
            } else {
                let len = ed.buf.line_len(from.0);
                let after = from.1 >= len && len > 0;
                ed.set_cursor(from.0, if after { len } else { from.1 });
                ops::put_with(ed, 1, after, false);
            }
        }
    }
    ed.vim.keep_clipboard = saved_clip;
    // The unnamed register: what `p` replaced, or as it was for `P`.
    ed.reg = saved;
    ed.vim.register = name;
    ed.undo.end(ed.cur.pos());
}

/// What a Visual or Visual-line selection (`mode`) covers.
pub(crate) fn selection(ed: &Editor, mode: &Mode) -> Option<ops::Span> {
    match *mode {
        Mode::VisualLine { anchor } => Some(ops::Span::Lines(
            anchor.min(ed.cur.line),
            anchor.max(ed.cur.line),
        )),
        Mode::Visual { anchor } => {
            let (from, to) = (anchor.min(ed.cur.pos()), anchor.max(ed.cur.pos()));
            let line = ed.buf.line(to.0);
            // On an empty line the selection takes its line break.
            let to = if to.1 < line.len() {
                (to.0, crate::text::next_grapheme(&line, to.1))
            } else if to.0 + 1 < ed.line_count() {
                (to.0 + 1, 0)
            } else {
                (to.0, line.len())
            };
            Some(ops::Span::Chars(from, to))
        }
        _ => None,
    }
}

/// A key in charwise Visual mode; the uppercase ones act on whole lines, as in vim.
fn visual_chars(ed: &mut Editor, anchor: (usize, usize), c: char) {
    match c {
        'v' => ed.mode = Mode::Normal,
        'V' => ed.mode = Mode::VisualLine { anchor: anchor.0 },
        'o' => {
            ed.mode = Mode::Visual {
                anchor: ed.cur.pos(),
            };
            ed.set_cursor(anchor.0, anchor.1);
        }
        'd' | 'x' | 'y' | 'c' | 's' => {
            ed.mode = Mode::Normal;
            if ed.generated() && c != 'y' {
                ed.set_err("generated Git buffer is read-only");
                return;
            }
            let Some(span) = selection(ed, &Mode::Visual { anchor }) else {
                return;
            };
            let op = match c {
                'x' => 'd',
                's' => 'c',
                c => c,
            };
            ed.undo.begin(ed.cur.pos());
            ops::apply_op(ed, op, span);
            if ed.mode != Mode::Insert {
                ed.undo.end(ed.cur.pos());
            }
        }
        _ => visual(ed, anchor.0, c),
    }
}

fn visual(ed: &mut Editor, anchor: usize, c: char) {
    if ed.generated() && !matches!(c, 'y' | 'Y' | ':' | 'V' | 'v' | 'o') {
        ed.mode = Mode::Normal;
        ed.set_err("generated Git buffer is read-only");
        return;
    }
    let (lo, hi) = (anchor.min(ed.cur.line), anchor.max(ed.cur.line));
    ed.mode = Mode::Normal;
    match c {
        'V' => {}
        'v' => {
            ed.mode = Mode::Visual {
                anchor: (anchor, 0),
            }
        }
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
    let g = matches!(cmd, Cmd::Simple { key, arg: Some('J' | 'p' | 'P' | 'i' | 'I' | '&'), .. } if *key == Key::ch('g'))
        || matches!(cmd, Cmd::Simple { key, .. } if *key == Key::ctrl('a') || *key == Key::ctrl('x'));
    g || match cmd {
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
                            | '~'
                            | 'R'
                            | '&'
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
    // After `"1p`, `.` puts from `"2` (and so on).
    let (_, used) = self::count(&ed.vim.last_change);
    if let [q, d, ..] = ed.vim.last_change[used..]
        && q == Key::ch('"')
        && let Some(n @ '1'..='8') = d.char()
    {
        ed.vim.last_change[used + 1] = Key::ch(char::from(n as u8 + 1));
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
        if let Some(c) = t.want {
            ed.cur.want_col = c;
        }
    }
}

pub(crate) fn execute(ed: &mut Editor, cmd: Cmd) {
    match cmd {
        Cmd::Move {
            count,
            motion: Motion::Object { include, kind },
        } => visual_object(ed, count, include, kind),
        Cmd::Move { count, motion } => {
            let jump = motion.is_jump();
            let from = ed.cur.pos();
            match motion::target(ed, motion, count) {
                Some(t) => {
                    if jump {
                        marks::set(ed, '\'', from);
                    }
                    move_to(ed, t);
                    if jump {
                        marks::checkpcmark(ed);
                    }
                }
                None => ed.vim.failed = true,
            }
        }
        Cmd::Op { op, count, motion } => {
            let sp = ops::span(ed, op, count, motion);
            ed.vim.failed |= sp.is_none();
            if let Some(sp) = sp {
                // `gUU` and `g~~` leave the cursor where the operated text
                // starts: at the first non-blank of the last line if that
                // comes first.
                if let (None, ops::Span::Lines(_, hi), '~' | 'u' | 'U' | '?') = (motion, sp, op) {
                    let end = (hi, ed.first_nonblank(hi));
                    if end < ed.cur.pos() {
                        ed.set_cursor(end.0, end.1);
                    }
                }
                ops::apply_op(ed, op, sp);
            }
        }
        Cmd::Simple { count, key, arg } => simple(ed, count, key, arg),
        Cmd::OpSearch { op, count, kind } => {
            ed.vim.pending_op = Some((op, count, ed.vim.register));
            ed.open_cmdline(kind, "");
        }
    }
}

/// `iw`, `ap`… in Visual mode: select the object, or extend the selection
/// by it.
fn visual_object(ed: &mut Editor, count: Option<usize>, include: bool, kind: char) {
    let cur = ed.cur.pos();
    let (vis, lines) = match ed.mode {
        Mode::Visual { anchor } => (anchor, false),
        Mode::VisualLine { anchor } if anchor == cur.0 => (cur, true),
        Mode::VisualLine { anchor } => ((anchor, 0), true),
        _ => return,
    };
    let Some(o) = textobj::object(
        &ed.buf,
        cur,
        count.unwrap_or(1),
        include,
        kind,
        Some(vis),
        lines,
    ) else {
        return;
    };
    ed.mode = if o.linewise {
        Mode::VisualLine { anchor: o.start.0 }
    } else {
        Mode::Visual { anchor: o.start }
    };
    ed.cur.line = o.end.0;
    ed.cur.byte = o.end.1;
    let want = crate::text::col_of_byte(&ed.buf.line(o.end.0), o.end.1, ed.tabstop());
    ed.cur.want_col = want;
}

/// Finish `d/pat<Enter>`: apply the pending operator up to the match.
pub(crate) fn finish_op_search(ed: &mut Editor, op: char, count: Option<usize>, reg: Option<char>) {
    ed.vim.register = reg;
    finish_op_search_with(ed, op, count);
    ed.vim.register = None;
}

fn finish_op_search_with(ed: &mut Editor, op: char, count: Option<usize>) {
    let start = ed.cur.pos();
    let Some(t) = ed.search_target_n(false, count.unwrap_or(1)) else {
        return;
    };
    let (from, to) = if t < start { (t, start) } else { (start, t) };
    ed.undo.begin(start);
    ed.vim.numbered_delete = true;
    ops::apply_op(ed, op, ops::Span::Chars(from, to));
    ed.vim.numbered_delete = false;
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
        (KeyCode::Char('w'), true) => {
            if let Some(cmd) = arg {
                ed.pending_effect = Some(ExEffect::Window { cmd, count });
            }
        }
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
        // `&`: the last :s again on this line; `g&`: on every line, with
        // the last search pattern and the same flags.
        (KeyCode::Char('&'), false) => ed.run_ex("s"),
        (KeyCode::Char('g'), false) if arg == Some('&') => ed.run_ex("%s//~/&"),
        (KeyCode::Char(c @ ('a' | 'x')), true) => {
            let (l, b) = ed.cur.pos();
            ops::addsub(ed, l, b, None, false, c == 'x', n as u64);
        }
        (KeyCode::Char('x'), false) | (KeyCode::Delete, _) => {
            op(ed, 'd', count, Some(Motion::Right))
        }
        (KeyCode::Char('X'), false) => op(ed, 'd', count, Some(Motion::Left)),
        (KeyCode::Char('D'), false) => op(ed, 'd', count, Some(Motion::LineEnd)),
        (KeyCode::Char('C'), false) => op(ed, 'c', count, Some(Motion::LineEnd)),
        // Neovim's default mapping: `Y` is `y$`.
        (KeyCode::Char('Y'), false) => op(ed, 'y', count, Some(Motion::LineEnd)),
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
        (KeyCode::Char('~'), false) => {
            ops::tilde(ed, n);
        }
        (KeyCode::Char('r'), false) => {
            if let Some(c) = arg {
                ops::replace_chars(ed, n, c);
            }
        }
        (KeyCode::Char(c @ ('o' | 'O')), false) => {
            ops::open_line(ed, c == 'o');
            ed.vim.insert_repeat = (n > 1).then_some((c, n));
        }
        // Replace mode.
        (KeyCode::Char('R'), false) => {
            ed.mode = Mode::Insert;
            ed.vim.replace = Some(vec![]);
            ed.vim.insert_repeat = (n > 1).then_some(('R', n));
        }
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
            ed.vim.insert_repeat = (n > 1).then_some((c, n));
        }
        // 'selectmode' cmd: `v` and `V` start Select mode.
        (KeyCode::Char(c @ ('v' | 'V')), false) => {
            let select = ed.opts.has(crate::options::Opt::Selectmode, "cmd");
            select::start(ed, ed.cur.pos(), c == 'V', select);
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
        // `Space -` lists the file's directory (dired), `Space O` shows it in
        // the file manager, `Space Enter` saves,
        // `Space h` opens help, `Space i` lists this file's symbols, `Space t` opens the terminal, `Space ;` closes, `Space w N` goes to window N.
        (KeyCode::Char(' '), false) => match arg {
            Some('s') => crate::zap::open(ed),
            Some('p') => crate::pick::open(ed, crate::pick::Kind::Files),
            Some('g') => crate::pick::open(ed, crate::pick::Kind::Grep),
            Some('k') => crate::pick::open(ed, crate::pick::Kind::Lines),
            Some('r') => crate::pick::open(ed, crate::pick::Kind::Recent),
            Some('d') => crate::pick::definition(ed),
            Some('h') => ed.help(""),
            Some('i') => crate::pick::imenu::open(ed),
            Some('t') => ed.pending_effect = Some(ExEffect::Terminal { split: true }),
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
            // Find-file from the directory you are in.
            Some('j') => {
                let dir = here(ed);
                crate::pick::browse(ed, &dir);
            }
            // That directory in the system's file manager (Finder).
            Some('O') => ed.pending_effect = Some(ExEffect::Reveal(here(ed))),
            Some('\n') => {
                ed.pending_effect = Some(ExEffect::Write {
                    path: None,
                    force: false,
                    range: None,
                    then_quit: false,
                })
            }
            Some(';') => ed.pending_effect = Some(ExEffect::Quit { force: false }),
            // Number the windows; the next digit goes to one.
            Some('w') => {
                ed.pending_effect = Some(ExEffect::Window {
                    cmd: '#',
                    count: None,
                })
            }
            _ => {}
        },
        (KeyCode::Char(c @ ('/' | '?')), false) => ed.open_cmdline(c, ""),
        // `gd`, as in vim (`gg` is a motion).
        (KeyCode::Char('g'), false) if arg == Some('d') => crate::pick::definition(ed),
        (KeyCode::Char('q'), false) => {
            if let Some(c) = arg {
                register::start_recording(ed, c);
            }
        }
        (KeyCode::Char('@'), false) => {
            if let Some(c) = arg {
                register::execute(ed, c, n);
            }
        }
        // `Q`: run the last recorded macro.
        (KeyCode::Char('Q'), false) => match ed.regs.last_rec {
            Some(c) => register::execute(ed, c, n),
            None => ed.set_err("E748: No previously used register"),
        },
        (KeyCode::Char('g'), false) if matches!(arg, Some('p' | 'P')) => {
            ops::put_with(ed, n, arg == Some('p'), true)
        }
        // `gI`: insert in column 1; `gi`: where Insert mode last ended.
        (KeyCode::Char('g'), false) if matches!(arg, Some('I' | 'i')) => {
            if arg == Some('i')
                && let Some((l, b)) = ed.vim.insert_stop
            {
                let l = l.min(ed.line_count() - 1);
                ed.cur.line = l;
                ed.cur.byte = b.min(ed.buf.line_len(l));
            } else if arg == Some('I') {
                ed.cur.byte = 0;
            }
            ed.mode = Mode::Insert;
            ed.vim.insert_repeat = (n > 1).then_some(('i', n));
        }
        // `gv`: the last Visual selection again.
        (KeyCode::Char('g'), false) if arg == Some('v') => match ed.vim.last_visual.clone() {
            Some((mut mode, (l, b))) => {
                let last = ed.line_count() - 1;
                match &mut mode {
                    Mode::VisualLine { anchor } => *anchor = (*anchor).min(last),
                    Mode::Visual { anchor } => {
                        anchor.0 = anchor.0.min(last);
                        anchor.1 = anchor.1.min(ed.buf.line_len(anchor.0));
                    }
                    _ => {}
                }
                ed.mode = mode;
                ed.set_cursor(l.min(last), b);
            }
            None => ed.set_err("E173: No previous Visual area"),
        },
        // `g;` and `g,`: older and newer places in the changelist.
        (KeyCode::Char('g'), false) if matches!(arg, Some(';' | ',')) => {
            let n = n as isize;
            match marks::change(ed, if arg == Some(';') { -n } else { n }) {
                Ok((l, b)) => {
                    let l = l.min(ed.line_count() - 1);
                    ed.set_cursor(l, b);
                }
                Err(e) => ed.set_err(e),
            }
        }
        // `gJ`: join without inserting or removing spaces.
        (KeyCode::Char('g'), false) if arg == Some('J') => {
            ops::join_with(ed, ed.cur.line, n, false);
        }
        // `gh` and `gH`: Select mode, charwise and linewise.
        (KeyCode::Char('g'), false) if matches!(arg, Some('h' | 'H')) => {
            select::start(ed, ed.cur.pos(), arg == Some('H'), true)
        }
        // ZZ writes if modified and quits; ZQ quits without writing.
        (KeyCode::Char('Z'), false) => match arg {
            Some('Z') => ed.pending_effect = Some(ExEffect::WriteIfModifiedQuit),
            Some('Q') => ed.pending_effect = Some(ExEffect::Quit { force: true }),
            _ => {}
        },
        // zz centres the cursor line in the window.
        (KeyCode::Char('z'), false) if arg == Some('z') => ed.recenter.set(true),
        (KeyCode::Char('m'), false) => {
            if let Some(c) = arg.filter(|c| c.is_ascii_alphabetic() || "'`[]<>".contains(*c)) {
                marks::set(ed, c, ed.cur.pos());
            }
        }
        // CTRL-O and CTRL-I (Tab): back and forward in the jumplist.
        (KeyCode::Char('o' | 'i'), true) | (KeyCode::Tab, _) => {
            let back = key == Key::ctrl('o');
            let n = n as isize;
            match marks::jump(ed, if back { -n } else { n }) {
                Some((l, b)) => {
                    let l = l.min(ed.line_count() - 1);
                    ed.set_cursor(l, b);
                }
                None => ed.vim.failed = true,
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;

/// The directory you are in: the listed one (dired), else the current
/// file's, else the working directory.
fn here(ed: &Editor) -> std::path::PathBuf {
    match &ed.dired {
        Some(d) => d.dir.clone(),
        None => ed
            .path
            .as_deref()
            .and_then(|p| std::path::absolute(p).ok())
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into())),
    }
}
