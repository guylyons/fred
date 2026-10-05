//! ex command interpreter.

use super::addr::{AddrCtx, Range, parse_addr, parse_range, read_delimited, use_pattern};
use crate::buffer::{Buffer, Edit};
use crate::search;
use crate::undo::Undo;
use regex::{Captures, Regex};
use std::collections::HashMap;

/// File-level actions a command asks the caller to perform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExEffect {
    Magit(crate::magit::Action),
    /// Org work that needs the session (other buffers and files).
    Org(crate::org::Effect),
    None,
    Write {
        path: Option<String>,
        force: bool,
        range: Option<Range>,
        then_quit: bool,
    },
    WriteIfModifiedQuit,
    Quit {
        force: bool,
    },
    /// Edit `path`, or reload the current file when `path` is None.
    Edit {
        path: Option<String>,
        force: bool,
    },
    /// Open a picker result: `path` at `line`/`col` (0-based; `col` in
    /// bytes), searching for `pattern` next.
    Open {
        path: std::path::PathBuf,
        line: usize,
        col: usize,
        pattern: Option<String>,
    },
    /// `:!cmd`: run it with the terminal handed over.
    Shell(String),
    /// `:pwd`.
    Pwd,
    /// `:cd [dir]` (home without one).
    Cd(Option<String>),
    /// `:[range]ai ask`: send `prompt` to Claude; its reply replaces `range`.
    /// `:[range]explain`: its reply is shown over the lines instead.
    Ai {
        range: Range,
        prompt: String,
        explain: bool,
    },
    /// `:b [N|name|#]`, `:bn`, `:bp`, `:bd[!] [N|name]`, `:ls`.
    Buffer {
        cmd: BufCmd,
        arg: String,
        force: bool,
    },
}

/// Which buffer command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufCmd {
    Go,
    Next,
    Prev,
    Delete,
    List,
    /// Search the lines of every buffer (`Space b`).
    Search,
}

/// What ex commands operate on: borrowed pieces of the editor.
pub struct ExState<'a> {
    pub buf: &'a mut Buffer,
    pub undo: &'a mut Undo,
    /// Current line (0-based), updated by commands.
    pub cur: usize,
    pub marks: &'a HashMap<char, usize>,
    pub last_pat: &'a mut Option<String>,
    /// The file being edited, for `%` in shell commands.
    pub file: Option<std::path::PathBuf>,
    /// Line splices applied by the running command: (at, removed, inserted).
    log: Vec<(usize, usize, usize)>,
}

impl<'a> ExState<'a> {
    pub fn new(
        buf: &'a mut Buffer,
        undo: &'a mut Undo,
        cur: usize,
        marks: &'a HashMap<char, usize>,
        last_pat: &'a mut Option<String>,
    ) -> Self {
        ExState {
            buf,
            undo,
            cur,
            marks,
            last_pat,
            file: None,
            log: vec![],
        }
    }

    fn apply(&mut self, e: Edit) {
        let inv = self.buf.apply(e);
        self.undo.record(inv);
    }

    /// Replace `remove` lines starting at `at` with `insert` lines.
    fn splice(&mut self, at: usize, remove: usize, insert: Vec<String>) {
        if let Some((edit, inserted)) = self.buf.splice_edit(at, remove, &insert) {
            self.apply(edit);
            self.log.push((at, remove, inserted));
        }
    }

    fn lines(&self, r: Range) -> Vec<String> {
        (r.start..=r.end).map(|l| self.buf.line(l)).collect()
    }

    fn addr_ctx(&mut self) -> AddrCtx<'_> {
        AddrCtx {
            buf: &*self.buf,
            cur: self.cur,
            marks: self.marks,
            last_pat: &mut *self.last_pat,
        }
    }
}

/// Run one command line. The whole command is one undo group.
pub fn run(st: &mut ExState, line: &str) -> Result<ExEffect, String> {
    let pos = (st.cur, 0);
    st.undo.begin(pos);
    let mark = st.undo.open_len();
    let r = run_one(st, line, false);
    if r.is_err() {
        // Roll back anything a failing command (e.g. inside `g`) already
        // did, leaving the redo history as it was.
        st.undo.rollback_to(st.buf, mark);
        st.undo.end(pos);
    } else {
        st.undo.end((st.cur, 0));
    }
    st.log.clear();
    r
}

fn run_one(st: &mut ExState, line: &str, in_global: bool) -> Result<ExEffect, String> {
    let (range, rest) = {
        let mut ctx = st.addr_ctx();
        let r = parse_range(line, &mut ctx)?;
        st.cur = ctx.cur.min(st.buf.len_lines() - 1);
        r
    };
    let rest = rest.trim_start();
    let cur = st.cur;
    let Some(c) = rest.chars().next() else {
        if let Some(r) = range {
            st.cur = r.end;
        }
        return Ok(ExEffect::None);
    };
    if c == '!' {
        // `:!cmd` runs it; `:[range]!cmd` filters those lines through it.
        let cmd = crate::shell::expand(rest[1..].trim(), st.file.as_deref())?;
        if cmd.is_empty() {
            return Err("command expected".into());
        }
        return match range {
            Some(r) => filter(st, r, &cmd),
            None => Ok(ExEffect::Shell(cmd)),
        };
    }
    let name_end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    let cmd = lookup(name).ok_or_else(|| format!("unknown command: {rest}"))?;
    let mut tail = &rest[name_end..];
    let force = tail.starts_with('!');
    if force {
        if cmd.flags & BANG == 0 {
            return Err(format!("no ! allowed: {name}"));
        }
        tail = &tail[1..];
    }
    if range.is_some() && cmd.flags & RANGE == 0 {
        return Err(format!("no range allowed: {name}"));
    }
    let arg = tail.trim();
    if !arg.is_empty() && cmd.flags & EXTRA == 0 {
        return Err(format!("unexpected argument: {arg}"));
    }
    (cmd.run)(
        st,
        Args {
            range,
            at: range.unwrap_or(Range {
                start: cur,
                end: cur,
            }),
            force,
            arg,
            raw: tail,
            in_global,
        },
    )
}

/// Takes a range (`:1,5d`).
const RANGE: u8 = 1;
/// Takes `!` (`:q!`).
const BANG: u8 = 2;
/// Takes an argument.
const EXTRA: u8 = 4;
/// The argument is a file name (Tab completes paths).
pub const FILE: u8 = 8;

/// A named command, as in vim's `ex_cmds.lua`: `min` is how short an
/// abbreviation may be (`:w` for `:write`).
pub struct Command {
    pub name: &'static str,
    pub min: usize,
    pub flags: u8,
    run: fn(&mut ExState, Args) -> Result<ExEffect, String>,
}

/// What a command was given.
#[derive(Clone, Copy)]
struct Args<'a> {
    range: Option<Range>,
    /// The range, or the current line.
    at: Range,
    force: bool,
    /// The argument, trimmed.
    arg: &'a str,
    /// Everything after the name and `!`, untrimmed (`:s/x/y /`).
    raw: &'a str,
    in_global: bool,
}

/// Every named command. A prefix runs the first entry it abbreviates, so
/// order decides ties, as in vim (`:s` is `:substitute`).
pub const COMMANDS: &[Command] = &[
    cmd("substitute", 1, RANGE | EXTRA, |st, a| {
        substitute(st, a.at, a.raw)
    }),
    cmd("global", 1, RANGE | BANG | EXTRA, |st, a| {
        global_cmd(st, a, !a.force)
    }),
    cmd("vglobal", 1, RANGE | EXTRA, |st, a| {
        global_cmd(st, a, false)
    }),
    cmd("delete", 1, RANGE, |st, a| {
        st.splice(a.at.start, a.at.end - a.at.start + 1, vec![]);
        st.cur = a.at.start.min(st.buf.len_lines() - 1);
        Ok(ExEffect::None)
    }),
    cmd("join", 1, RANGE, join),
    cmd("move", 1, RANGE | EXTRA, |st, a| move_copy(st, a, false)),
    cmd("t", 1, RANGE | EXTRA, |st, a| move_copy(st, a, true)),
    cmd("copy", 2, RANGE | EXTRA, |st, a| move_copy(st, a, true)),
    cmd("write", 1, RANGE | BANG | EXTRA | FILE, |_, a| {
        write(a, false)
    }),
    cmd("wq", 2, RANGE | BANG | EXTRA | FILE, |_, a| write(a, true)),
    cmd("xit", 1, 0, |_, _| Ok(ExEffect::WriteIfModifiedQuit)),
    cmd("quit", 1, BANG, |_, a| {
        Ok(ExEffect::Quit { force: a.force })
    }),
    cmd("edit", 1, BANG | EXTRA | FILE, |_, a| {
        Ok(ExEffect::Edit {
            path: path_arg(a.arg)?,
            force: a.force,
        })
    }),
    cmd("read", 1, RANGE | EXTRA | FILE, |st, a| {
        let at = a.range.map_or(st.cur, |r| r.end);
        read(st, at, a.arg)
    }),
    cmd("pwd", 2, 0, |_, _| Ok(ExEffect::Pwd)),
    cmd("cd", 2, EXTRA | FILE, |_, a| {
        Ok(ExEffect::Cd(path_arg(a.arg)?))
    }),
    cmd("buffer", 1, BANG | EXTRA, |_, a| buffer(a, BufCmd::Go)),
    cmd("bnext", 2, BANG, |_, a| buffer(a, BufCmd::Next)),
    cmd("bprevious", 2, BANG, |_, a| buffer(a, BufCmd::Prev)),
    cmd("bNext", 2, BANG, |_, a| buffer(a, BufCmd::Prev)),
    cmd("bdelete", 2, BANG | EXTRA, |_, a| buffer(a, BufCmd::Delete)),
    cmd("ls", 2, BANG, |_, a| buffer(a, BufCmd::List)),
    cmd("buffers", 7, BANG, |_, a| buffer(a, BufCmd::List)),
    cmd("files", 5, BANG, |_, a| buffer(a, BufCmd::List)),
    cmd("ai", 2, RANGE | EXTRA, |st, a| ai(st, a.at, a.arg, false)),
    cmd("explain", 3, RANGE | EXTRA, |st, a| {
        ai(st, a.at, a.arg, true)
    }),
    // :Magit NAME runs an upstream Magit command by name (like M-x).
    cmd("Magit", 5, EXTRA, |_, a| {
        crate::magit::commands::by_name(a.arg)
            .map(ExEffect::Magit)
            .ok_or_else(|| format!("unknown Magit command: {}", a.arg))
    }),
];

const fn cmd(
    name: &'static str,
    min: usize,
    flags: u8,
    run: fn(&mut ExState, Args) -> Result<ExEffect, String>,
) -> Command {
    Command {
        name,
        min,
        flags,
        run,
    }
}

/// The command `name` names or abbreviates.
pub fn lookup(name: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|c| name.len() >= c.min && c.name.starts_with(name))
}

/// A file name argument; `:r` alone reads a command's output.
fn path_arg(arg: &str) -> Result<Option<String>, String> {
    if arg.starts_with('!') {
        return Err("shell commands are not supported here".into());
    }
    Ok((!arg.is_empty()).then(|| arg.to_string()))
}

fn write(a: Args, then_quit: bool) -> Result<ExEffect, String> {
    Ok(ExEffect::Write {
        path: path_arg(a.arg)?,
        force: a.force,
        range: a.range,
        then_quit,
    })
}

fn buffer(a: Args, cmd: BufCmd) -> Result<ExEffect, String> {
    Ok(ExEffect::Buffer {
        cmd,
        arg: a.arg.to_string(),
        force: a.force,
    })
}

/// `:g/pat/cmd` (`keep` = true), `:g!` and `:v` (false).
fn global_cmd(st: &mut ExState, a: Args, keep: bool) -> Result<ExEffect, String> {
    if a.in_global {
        return Err("nested global".into());
    }
    let all = Range {
        start: 0,
        end: st.buf.len_lines() - 1,
    };
    global(st, a.range.unwrap_or(all), a.raw, keep)
}

fn join(st: &mut ExState, a: Args) -> Result<ExEffect, String> {
    let cur = st.cur;
    let r = match a.range {
        Some(r) => r,
        None if cur + 1 < st.buf.len_lines() => Range {
            start: cur,
            end: cur + 1,
        },
        None => return Err("invalid address".into()),
    };
    if r.start < r.end {
        let joined = st.lines(r).concat();
        st.splice(r.start, r.end - r.start + 1, vec![joined]);
    }
    st.cur = r.start;
    Ok(ExEffect::None)
}

/// `:m {address}` and `:t {address}`.
fn move_copy(st: &mut ExState, a: Args, copy: bool) -> Result<ExEffect, String> {
    let r = a.at;
    let (dest, tail) = {
        let mut ctx = st.addr_ctx();
        parse_addr(a.arg, &mut ctx)?
    };
    if !tail.trim().is_empty() {
        return Err(format!("unexpected: {}", tail.trim()));
    }
    let dest = dest.ok_or("destination expected")? as usize;
    let lines = st.lines(r);
    let count = lines.len();
    if copy {
        st.splice(dest, 0, lines);
        st.cur = dest + count - 1;
    } else {
        if dest > r.start && dest <= r.end {
            return Err("invalid destination".into());
        }
        if dest == r.start || dest == r.end + 1 {
            st.cur = r.end;
            return Ok(ExEffect::None);
        }
        st.splice(r.start, count, vec![]);
        let d = if dest > r.end { dest - count } else { dest };
        st.splice(d, 0, lines);
        st.cur = d + count - 1;
    }
    Ok(ExEffect::None)
}

/// `:[range]!cmd`: replace the lines with what `cmd` prints for them.
fn filter(st: &mut ExState, r: Range, cmd: &str) -> Result<ExEffect, String> {
    let input: String = st.lines(r).iter().map(|l| format!("{l}\n")).collect();
    let out = crate::shell::capture(cmd, Some(input))?;
    replace(st, r, &out)
}

/// `:[range]ai what to do`: the prompt asking Claude to rewrite the lines,
/// with the whole file for context. The caller runs it (see `app::ask_claude`).
/// `:[range]explain [question]` asks for a short explanation instead.
fn ai(st: &mut ExState, r: Range, ask: &str, explain: bool) -> Result<ExEffect, String> {
    if ask.is_empty() && !explain {
        return Err("say what to change: :ai make this async".into());
    }
    let name = st
        .file
        .as_ref()
        .map_or("an unnamed file".into(), |p| p.display().to_string());
    let (a, b) = (r.start + 1, r.end + 1);
    let sel: String = st.lines(r).iter().map(|l| format!("{l}\n")).collect();
    let task = if explain {
        format!(
            "Explain what the selected lines do to someone reading the file. \
             Be brief: plain text, at most 8 short lines, no markdown headings, \
             no code fences.{}",
            match ask {
                "" => String::new(),
                q => format!("\n\nAsked: {q}"),
            }
        )
    } else {
        format!(
            "Rewrite only the selected lines as asked, matching the file's \
             style and indentation. Reply with just the new text for those \
             lines: no explanation, no code fences, no line numbers.\n\nAsked: {ask}"
        )
    };
    let prompt = format!(
        "You are editing {name} in a text editor. The whole file is below for \
         context, then the selected lines {a}-{b}. {task}\n\n<file>\n{}\n</file>\n\n\
         <selection lines=\"{a}-{b}\">\n{sel}</selection>\n",
        st.buf.text()
    );
    Ok(ExEffect::Ai {
        range: r,
        prompt,
        explain,
    })
}

/// The reply without a ```lang … ``` wrapper, if it added one anyway.
pub fn unfence(s: &str) -> &str {
    s.trim()
        .strip_prefix("```")
        .and_then(|t| t.strip_suffix("```"))
        .map_or(s, |t| t.split_once('\n').map_or("", |(_, body)| body))
}

/// Replace the lines with `out`'s, cursor on the last new one.
fn replace(st: &mut ExState, r: Range, out: &str) -> Result<ExEffect, String> {
    let lines: Vec<String> = out.lines().map(String::from).collect();
    let n = lines.len();
    st.splice(r.start, r.end - r.start + 1, lines);
    st.cur = (r.start + n.saturating_sub(1)).min(st.buf.len_lines() - 1);
    Ok(ExEffect::None)
}

/// `:r file` / `:r !cmd`: its lines, below line `at`.
fn read(st: &mut ExState, at: usize, arg: &str) -> Result<ExEffect, String> {
    let text = match arg.strip_prefix('!') {
        Some(cmd) => {
            let cmd = crate::shell::expand(cmd.trim(), st.file.as_deref())?;
            crate::shell::capture(&cmd, None)?
        }
        None if arg.is_empty() => return Err("file name expected".into()),
        None => std::fs::read_to_string(arg)
            .map_err(|e| format!("{arg}: {}", crate::fileio::err_msg(&e)))?,
    };
    let lines: Vec<String> = text.lines().map(String::from).collect();
    if lines.is_empty() {
        return Ok(ExEffect::None);
    }
    st.splice(at + 1, 0, lines);
    st.cur = at + 1;
    Ok(ExEffect::None)
}

fn expand(rep: &str, caps: &Captures) -> String {
    let mut out = String::new();
    let mut it = rep.chars();
    while let Some(c) = it.next() {
        match c {
            '&' => out.push_str(&caps[0]),
            '\\' => match it.next() {
                Some(d @ '0'..='9') => {
                    let i = d as usize - '0' as usize;
                    out.push_str(caps.get(i).map_or("", |m| m.as_str()));
                }
                // `\r` is vim's way to split a line; `\n` is ed's.
                Some('n' | 'r') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    out
}

fn substitute(st: &mut ExState, r: Range, spec: &str) -> Result<ExEffect, String> {
    let delim = spec.chars().next().ok_or("bad substitute")?;
    let (pat, rest) = read_delimited(&spec[delim.len_utf8()..], delim);
    let (rep, flags) = read_delimited(rest, delim);
    let mut global = false;
    for f in flags.trim().chars() {
        match f {
            'g' => global = true,
            _ => return Err(format!("unknown flag: {f}")),
        }
    }
    let pat = use_pattern(pat, st.last_pat)?;
    let re = search::compile_exact(&pat)?;
    let mut last = None;
    for l in (r.start..=r.end).rev() {
        let line = st.buf.line(l);
        if let Some(new) = subst_line(&re, &line, &rep, global) {
            let parts: Vec<String> = new.split('\n').map(String::from).collect();
            let added = parts.len() - 1;
            st.splice(l, 1, parts);
            if last.is_none() {
                last = Some(l + added);
            }
        }
    }
    st.cur = last.ok_or("no match")?;
    Ok(ExEffect::None)
}

fn subst_line(re: &Regex, line: &str, rep: &str, global: bool) -> Option<String> {
    let mut out = String::new();
    let mut prev = 0;
    let mut any = false;
    for caps in re.captures_iter(line) {
        let m = caps.get(0).expect("group 0");
        out.push_str(&line[prev..m.start()]);
        out.push_str(&expand(rep, &caps));
        prev = m.end();
        any = true;
        if !global {
            break;
        }
    }
    any.then(|| out + &line[prev..])
}

fn global(st: &mut ExState, r: Range, spec: &str, want: bool) -> Result<ExEffect, String> {
    let delim = spec.chars().next().ok_or("bad global")?;
    let (pat, cmd) = read_delimited(&spec[delim.len_utf8()..], delim);
    if cmd.trim().is_empty() {
        return Err("command expected".into());
    }
    let pat = use_pattern(pat, st.last_pat)?;
    let re = search::compile_exact(&pat)?;
    let mut marked: Vec<bool> = (0..st.buf.len_lines())
        .map(|l| l >= r.start && l <= r.end && re.is_match(&st.buf.line(l)) == want)
        .collect();
    if !marked.contains(&true) {
        return Err("no match".into());
    }
    // A substitute that finds nothing on one marked line is not an error,
    // as in ed; it is only an error if it found nothing on any line.
    let mut any_ok = false;
    let mut no_match = false;
    let mut last_cur = st.cur;
    while let Some(l) = marked.iter().position(|&m| m) {
        marked[l] = false;
        st.cur = l;
        let before = st.log.len();
        match run_one(st, cmd, true) {
            Ok(_) => {
                any_ok = true;
                last_cur = st.cur;
            }
            Err(e) if e == "no match" => no_match = true,
            Err(e) => return Err(e),
        }
        for &(at, removed, inserted) in &st.log[before..] {
            let end = (at + removed).min(marked.len());
            marked.splice(
                at.min(marked.len())..end,
                std::iter::repeat_n(false, inserted),
            );
        }
    }
    if no_match && !any_ok {
        return Err("no match".into());
    }
    st.cur = last_cur.min(st.buf.len_lines() - 1);
    Ok(ExEffect::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::undo::Undo;
    use std::collections::HashMap;

    struct T {
        buf: Buffer,
        undo: Undo,
        marks: HashMap<char, usize>,
        last_pat: Option<String>,
    }

    fn run_on(t: &str, cur: usize, cmd: &str) -> (T, usize, Result<ExEffect, String>) {
        let mut x = T {
            buf: Buffer::from_text(t),
            undo: Undo::default(),
            marks: HashMap::new(),
            last_pat: None,
        };
        let mut st = ExState::new(&mut x.buf, &mut x.undo, cur, &x.marks, &mut x.last_pat);
        let r = run(&mut st, cmd);
        let cur = st.cur;
        (x, cur, r)
    }

    fn ex(t: &str, cur: usize, cmd: &str) -> (String, usize) {
        let (x, cur, r) = run_on(t, cur, cmd);
        r.unwrap();
        (x.buf.text(), cur)
    }

    fn ex_err(t: &str, cur: usize, cmd: &str) -> Result<(), String> {
        let (x, _, r) = run_on(t, cur, cmd);
        assert_eq!(x.buf.text(), t, "buffer changed by failing {cmd}");
        r.map(|_| ())
    }

    #[test]
    fn commands() {
        let t = "one\ntwo\nthree\nfour";
        assert_eq!(ex(t, 0, "2d"), ("one\nthree\nfour".into(), 1));
        assert_eq!(ex(t, 0, "$d"), ("one\ntwo\nthree".into(), 2));
        assert_eq!(ex(t, 0, ",s/o/0/g"), ("0ne\ntw0\nthree\nf0ur".into(), 3));
        assert_eq!(
            ex(t, 0, "s/(o)(n)/\\2\\1&/"),
            ("noone\ntwo\nthree\nfour".into(), 0)
        );
        assert_eq!(ex(t, 0, "s/o/\\&/"), ("&ne\ntwo\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "s#o#/#"), ("/ne\ntwo\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "1,2j"), ("onetwo\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "j"), ("onetwo\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "1m$"), ("two\nthree\nfour\none".into(), 3));
        assert_eq!(ex(t, 3, "4m0"), ("four\none\ntwo\nthree".into(), 0));
        assert_eq!(ex(t, 0, "1,2m3"), ("three\none\ntwo\nfour".into(), 2));
        assert_eq!(ex(t, 0, "1t2"), ("one\ntwo\none\nthree\nfour".into(), 2));
        assert_eq!(
            ex(t, 0, "1,2t$"),
            ("one\ntwo\nthree\nfour\none\ntwo".into(), 5)
        );
        assert_eq!(ex(t, 0, "g/o/d"), ("three".into(), 0));
        assert_eq!(ex(t, 0, "v/o/d"), ("one\ntwo\nfour".into(), 2));
        assert_eq!(
            ex(t, 0, "g/o/s/$/!/"),
            ("one!\ntwo!\nthree\nfour!".into(), 3)
        );
        assert_eq!(
            ex(t, 0, "g/o/t$"),
            ("one\ntwo\nthree\nfour\none\ntwo\nfour".into(), 6)
        );
        assert_eq!(ex(t, 0, "3"), (t.into(), 2));
        assert_eq!(
            ex(t, 0, "s/e/x\\ny/"),
            ("onx\ny\ntwo\nthree\nfour".into(), 1)
        );
    }

    #[test]
    fn command_table_abbreviations_and_flags() {
        // Any prefix down to the minimum runs the first command it names.
        assert_eq!(ex("a\nb\nc", 0, "1,2de").0, "c");
        assert_eq!(ex("a\nb\nc", 0, "1,2delete").0, "c");
        assert_eq!(ex("a\nb", 0, "su/a/x/").0, "x\nb");
        assert_eq!(ex("a\nb", 0, "co$").0, "a\nb\na");
        assert_eq!(lookup("b").unwrap().name, "buffer");
        assert_eq!(lookup("bn").unwrap().name, "bnext");
        assert!(lookup("c").is_none() && lookup("exp").is_some() && lookup("ex").is_none());
        // `:g!` is `:v`; a substitute keeps its trailing space.
        assert_eq!(ex("a\nb\na", 0, "g!/a/d").0, "a\na");
        assert_eq!(ex("a", 0, "s/a/x /").0, "x ");
        // What a command doesn't take is an error.
        assert_eq!(ex_err("a", 0, "1,1q"), Err("no range allowed: q".into()));
        assert_eq!(ex_err("a", 0, "pwd!"), Err("no ! allowed: pwd".into()));
        assert_eq!(ex_err("a", 0, "d x"), Err("unexpected argument: x".into()));
        assert!(ex_err("a", 0, "w !ls").is_err());
    }

    #[test]
    fn errors_leave_buffer() {
        let t = "a\nb";
        for c in [
            "100d", "2,1d", "/zz/d", "s/zz/y/", "k", "g//d", "g/a/", "1,2m1", "s/(/x/", "d x",
        ] {
            assert!(ex_err(t, 0, c).is_err(), "{c}");
        }
        assert!(ex_err(t, 1, "j").is_err());
        assert_eq!(ex(t, 1, "$j"), (t.into(), 1));
    }

    #[test]
    fn ex_patterns_are_case_sensitive() {
        assert_eq!(ex("Foo foo", 0, "s/foo/x/g"), ("Foo x".into(), 0));
        assert_eq!(ex("Foo\nfoo", 0, "g/foo/d"), ("Foo".into(), 0));
        assert_eq!(ex("Foo foo", 0, "s/(?i)foo/x/g"), ("x x".into(), 0));
    }

    #[test]
    fn backslash_r_in_replacement_splits_like_vim() {
        assert_eq!(ex("a,b,c", 0, "s/,/\\r/g"), ("a\nb\nc".into(), 2));
    }

    #[test]
    fn g_with_substitute_skips_lines_without_a_match() {
        let t = "foo bar\nfoo baz\nqux";
        assert_eq!(
            ex(t, 0, "g/foo/s/bar/X/"),
            ("foo X\nfoo baz\nqux".into(), 0)
        );
        assert!(
            ex_err(t, 0, "g/foo/s/zzz/X/").is_err(),
            "no substitution at all is still an error"
        );
    }

    #[test]
    fn g_is_one_undo_group() {
        let t = "one\ntwo\nthree\nfour";
        let (mut x, _, r) = run_on(t, 0, "g/o/d");
        r.unwrap();
        x.undo.undo(&mut x.buf);
        assert_eq!(x.buf.text(), t);
    }

    #[test]
    fn delete_all_leaves_one_empty_line() {
        assert_eq!(ex("a\nb", 0, ",d"), ("".into(), 0));
    }

    #[test]
    fn file_effects() {
        let e = |c: &str| run_on("a", 0, c).2.unwrap();
        assert_eq!(
            e("w"),
            ExEffect::Write {
                path: None,
                force: false,
                range: None,
                then_quit: false
            }
        );
        assert_eq!(
            e("w! out.txt"),
            ExEffect::Write {
                path: Some("out.txt".into()),
                force: true,
                range: None,
                then_quit: false
            }
        );
        assert_eq!(
            e("wq"),
            ExEffect::Write {
                path: None,
                force: false,
                range: None,
                then_quit: true
            }
        );
        assert_eq!(e("x"), ExEffect::WriteIfModifiedQuit);
        assert_eq!(e("q"), ExEffect::Quit { force: false });
        assert_eq!(e("q!"), ExEffect::Quit { force: true });
        assert_eq!(
            e("e! f"),
            ExEffect::Edit {
                path: Some("f".into()),
                force: true
            }
        );
        assert_eq!(
            e("e"),
            ExEffect::Edit {
                path: None,
                force: false
            }
        );
        assert_eq!(
            e("e!"),
            ExEffect::Edit {
                path: None,
                force: true
            }
        );
    }

    #[test]
    fn buffer_commands() {
        let b = |cmd, arg: &str, force| {
            Ok(ExEffect::Buffer {
                cmd,
                arg: arg.into(),
                force,
            })
        };
        assert_eq!(run_on("a", 0, "b2").2, b(BufCmd::Go, "2", false));
        assert_eq!(run_on("a", 0, "b #").2, b(BufCmd::Go, "#", false));
        assert_eq!(
            run_on("a", 0, "buffer main").2,
            b(BufCmd::Go, "main", false)
        );
        assert_eq!(run_on("a", 0, "bn").2, b(BufCmd::Next, "", false));
        assert_eq!(run_on("a", 0, "bp").2, b(BufCmd::Prev, "", false));
        assert_eq!(run_on("a", 0, "bd!").2, b(BufCmd::Delete, "", true));
        assert_eq!(run_on("a", 0, "ls").2, b(BufCmd::List, "", false));
    }

    #[test]
    fn filters_lines_through_a_command() {
        assert_eq!(ex("c\nb\na", 0, "%!sort"), ("a\nb\nc".into(), 2));
        assert_eq!(ex("x\nc\nb\ny", 0, "2,3!sort"), ("x\nb\nc\ny".into(), 2));
        // Output can be longer or shorter.
        assert_eq!(ex("a b", 0, ".!tr ' ' '\\n'").0, "a\nb");
        assert_eq!(ex("a\nb\nc", 0, "%!head -1").0, "a");
        // A failing command leaves the text alone and says why.
        assert_eq!(
            ex_err("a\nb", 0, "%!echo nope >&2; exit 1"),
            Err("nope".into())
        );
        assert!(ex_err("a", 0, "%!").is_err());
        // One undo step.
        let (mut x, _, r) = run_on("c\nb\na", 0, "%!sort");
        r.unwrap();
        assert!(x.undo.undo(&mut x.buf).is_some());
        assert_eq!(x.buf.text(), "c\nb\na");
    }

    #[test]
    fn ai_asks_with_the_whole_file() {
        let mut x = T {
            buf: Buffer::from_text("fn a() {}\nfn b() {}\nfn c() {}"),
            undo: Undo::default(),
            marks: HashMap::new(),
            last_pat: None,
        };
        let mut st = ExState::new(&mut x.buf, &mut x.undo, 0, &x.marks, &mut x.last_pat);
        st.file = Some("src/x.rs".into());
        let Ok(ExEffect::Ai {
            range,
            prompt,
            explain: false,
        }) = run(&mut st, "2ai rename")
        else {
            panic!("no ai effect");
        };
        assert_eq!(range, Range { start: 1, end: 1 });
        for w in [
            "src/x.rs",
            "fn c()",
            "Asked: rename",
            "lines=\"2-2\">\nfn b() {}\n",
        ] {
            assert!(prompt.contains(w), "{w} not in {prompt}");
        }
        assert!(run(&mut st, "ai").is_err());
        let Ok(ExEffect::Ai {
            prompt,
            explain: true,
            ..
        }) = run(&mut st, "1,2explain")
        else {
            panic!("no explain effect");
        };
        assert!(
            prompt.contains("Explain what the selected lines do"),
            "{prompt}"
        );
        assert!(
            prompt.contains("lines=\"1-2\">\nfn a() {}\nfn b() {}\n"),
            "{prompt}"
        );
        assert_eq!(unfence("  x\n"), "  x\n");
        assert_eq!(unfence("```rust\n  x\n```\n"), "  x\n");
    }

    #[test]
    fn read_a_file_or_command() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("in.txt");
        std::fs::write(&f, "one\ntwo\n").unwrap();
        let cmd = format!("r {}", f.display());
        assert_eq!(ex("a\nb", 0, &cmd), ("a\none\ntwo\nb".into(), 1));
        assert_eq!(
            ex("a\nb", 0, &format!("$r {}", f.display())).0,
            "a\nb\none\ntwo"
        );
        assert_eq!(ex("a", 0, "r !printf 'x\\ny'").0, "a\nx\ny");
        assert!(
            ex_err("a", 0, "r /no/such/file")
                .unwrap_err()
                .contains("no such file")
        );
        assert!(ex_err("a", 0, "r").is_err());
    }

    #[test]
    fn shell_pwd_and_cd_are_effects() {
        assert_eq!(
            run_on("a", 0, "!ls -l").2,
            Ok(ExEffect::Shell("ls -l".into()))
        );
        assert_eq!(run_on("a", 0, "pwd").2, Ok(ExEffect::Pwd));
        assert_eq!(run_on("a", 0, "cd").2, Ok(ExEffect::Cd(None)));
        assert_eq!(
            run_on("a", 0, "cd ~/x").2,
            Ok(ExEffect::Cd(Some("~/x".into())))
        );
        assert!(run_on("a", 0, "!").2.is_err());
        // `%` needs a file name.
        assert!(run_on("a", 0, "!wc %").2.is_err());
        let mut x = T {
            buf: Buffer::from_text("a"),
            undo: Undo::default(),
            marks: HashMap::new(),
            last_pat: None,
        };
        let mut st = ExState::new(&mut x.buf, &mut x.undo, 0, &x.marks, &mut x.last_pat);
        st.file = Some("src/my file.rs".into());
        assert_eq!(
            run(&mut st, "!wc -l %"),
            Ok(ExEffect::Shell("wc -l 'src/my file.rs'".into()))
        );
    }
}
