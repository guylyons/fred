//! Registers (`:h registers`): the unnamed one (`Editor::reg`), named
//! `a-z` (`A-Z` appends), numbered `0-9`, the small delete `-`, the black
//! hole `_`, the clipboard `+` and `*`, and the read-only `. : / %`.
//! Also the macro recorder (`q`, `@`).

use crate::editor::{Editor, Register};
use crate::key::{Key, KeyCode};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Registers {
    map: HashMap<char, Register>,
    /// `q{reg}` is recording these keys.
    pub(crate) rec: Option<(char, Vec<Key>)>,
    /// The register `@@` runs again, and the one `Q` runs.
    pub(crate) last_exec: Option<char>,
    pub(crate) last_rec: Option<char>,
    /// Depth of `@` runs in progress.
    pub(crate) running: usize,
}

/// A register name `"x` can take.
pub fn valid(c: char) -> bool {
    c.is_ascii_alphanumeric() || "\"-_+*.:/%".contains(c)
}

fn multiline(r: &Register) -> bool {
    r.linewise || r.text.contains('\n')
}

/// Text `"Ay` adds to what register `a` holds.
fn append(old: Option<&Register>, new: Register) -> Register {
    match old {
        None => new,
        Some(o) if o.linewise || new.linewise => Register {
            text: format!("{}\n{}", o.text, new.text),
            linewise: true,
            block: false,
        },
        Some(o) => Register {
            text: o.text.clone() + &new.text,
            linewise: false,
            block: false,
        },
    }
}

/// Store what a yank or a delete took, where Vim does: the named register
/// if one was given (`ed.vim.register`), `"0` for a yank, `"1` (shifting
/// the others up) for a delete of more than a line or by one of the
/// motions `% ( ) / ? n N { }` (`numbered`), else `"-`; and the unnamed
/// register.
pub(crate) fn store(ed: &mut Editor, mut r: Register, yank: bool, numbered: bool) {
    let name = ed.vim.register.filter(|&c| c != '"');
    match name {
        Some('_') => return,
        Some(c) if c.is_ascii_lowercase() => {
            ed.regs.map.insert(c, r.clone());
        }
        Some(c) if c.is_ascii_uppercase() => {
            let c = c.to_ascii_lowercase();
            r = append(ed.regs.map.get(&c), r);
            ed.regs.map.insert(c, r.clone());
        }
        Some('+' | '*') => crate::clipboard::set(&r),
        _ => {}
    }
    if yank {
        if name.is_none() {
            ed.regs.map.insert('0', r.clone());
        }
    } else if numbered || multiline(&r) {
        for i in (1..9).rev() {
            let from = char::from(b'0' + i);
            if let Some(v) = ed.regs.map.remove(&from) {
                ed.regs.map.insert(char::from(b'1' + i), v);
            }
        }
        ed.regs.map.insert('1', r.clone());
    } else if name.is_none() {
        ed.regs.map.insert('-', r.clone());
    }
    if name.is_none() && ed.clipboard() && !ed.vim.keep_clipboard {
        crate::clipboard::set(&r);
    }
    ed.reg = r;
}

/// What register `c` holds (`None` if it's empty).
pub(crate) fn get(ed: &Editor, c: char) -> Option<Register> {
    let chars = |text: String| {
        (!text.is_empty()).then_some(Register {
            text,
            linewise: false,
            block: false,
        })
    };
    match c {
        '"' => Some(ed.reg.clone()),
        '_' => None,
        '+' | '*' => crate::clipboard::get().map(|t| crate::clipboard::from_clip(&t)),
        '/' => chars(ed.last_pat.clone().unwrap_or_default()),
        ':' => chars(ed.last_cmd().unwrap_or_default()),
        '%' => chars(
            ed.path
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
        '.' => chars(ed.vim.last_insert.clone()),
        c => ed.regs.map.get(&c.to_ascii_lowercase()).cloned(),
    }
}

/// Keys as register text: characters as typed, CTRL keys as control
/// characters, Esc, Enter and Tab as their bytes; other keys as `<Name>`
/// between U+0080 marks.
pub fn keys_to_text(keys: &[Key]) -> String {
    let mut s = String::new();
    for k in keys {
        match (k.code, k.ctrl, k.alt, k.shift) {
            (KeyCode::Char(c), false, false, _) => s.push(c),
            (KeyCode::Char(c @ 'a'..='z'), true, false, _) => {
                s.push(char::from(c as u8 - b'a' + 1))
            }
            (KeyCode::Esc, false, false, false) => s.push('\x1b'),
            (KeyCode::Enter, false, false, false) => s.push('\r'),
            (KeyCode::Tab, false, false, false) => s.push('\t'),
            _ => {
                s.push('\u{80}');
                s.push_str(&crate::key::notation(k));
                s.push('\u{80}');
            }
        }
    }
    s
}

/// Register text as keys, the inverse of `keys_to_text`. A line break is
/// Enter.
pub fn text_to_keys(text: &str) -> Vec<Key> {
    let mut out = vec![];
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        out.push(match c {
            '\u{80}' => {
                let name: String = chars.by_ref().take_while(|&c| c != '\u{80}').collect();
                out.extend(crate::key::parse_keys(&name));
                continue;
            }
            '\x1b' => Key::new(KeyCode::Esc),
            '\r' | '\n' => Key::new(KeyCode::Enter),
            '\t' => Key::new(KeyCode::Tab),
            '\x08' => Key::new(KeyCode::Backspace),
            c @ '\x01'..='\x1a' => Key::ctrl(char::from(c as u8 - 1 + b'a')),
            c => Key::ch(c),
        });
    }
    out
}

/// Keys go to the macro being recorded.
pub(crate) fn record(ed: &mut Editor, k: Key) {
    if ed.regs.running == 0
        && let Some((_, keys)) = ed.regs.rec.as_mut()
    {
        keys.push(k);
    }
}

/// `q{reg}`: start recording; `q` again stops (its own key is dropped).
pub(crate) fn start_recording(ed: &mut Editor, c: char) {
    if c.is_ascii_alphanumeric() || c == '"' {
        ed.regs.rec = Some((c, vec![]));
    }
}

pub(crate) fn stop_recording(ed: &mut Editor) {
    let Some((c, mut keys)) = ed.regs.rec.take() else {
        return;
    };
    keys.pop();
    let r = Register {
        text: keys_to_text(&keys),
        linewise: false,
        block: false,
    };
    match c {
        c if c.is_ascii_uppercase() => {
            let lc = c.to_ascii_lowercase();
            let new = append(ed.regs.map.get(&lc), r);
            ed.regs.map.insert(lc, new);
        }
        '"' => ed.reg = r,
        c => {
            ed.regs.map.insert(c, r);
        }
    }
    ed.regs.last_rec = Some(c.to_ascii_lowercase());
}

/// `@{reg}` (`@@` the last one, `@:` the last command line) `count`
/// times. A failing command ends the run, as in Vim.
pub(crate) fn execute(ed: &mut Editor, c: char, count: usize) {
    let c = match c {
        '@' => match ed.regs.last_exec {
            Some(c) => c,
            None => {
                ed.set_err("E748: No previously used register");
                return;
            }
        },
        c => c,
    };
    if c == ':' {
        ed.regs.last_exec = Some(':');
        if let Some(cmd) = ed.last_cmd() {
            for _ in 0..count.max(1) {
                ed.run_ex(&cmd);
            }
        }
        return;
    }
    let Some(r) = get(ed, c) else {
        return;
    };
    ed.regs.last_exec = Some(c);
    if ed.regs.running >= 100 {
        ed.set_err("E132: Function call depth is higher than 'maxfuncdepth'");
        return;
    }
    let mut keys = text_to_keys(&r.text);
    if r.linewise {
        keys.push(Key::new(KeyCode::Enter));
    }
    ed.regs.running += 1;
    ed.vim.failed = false;
    'runs: for _ in 0..count.max(1) {
        for &k in &keys {
            ed.handle_key(k);
            if ed.vim.failed {
                break 'runs;
            }
        }
    }
    ed.regs.running -= 1;
}
