//! ex command interpreter.

use super::addr::{AddrCtx, Range, parse_addr, parse_range, read_delimited, use_pattern};
use crate::buffer::{Buffer, Edit};
use crate::options::Level;
use crate::search;
use crate::undo::Undo;
use regex::{Captures, Regex};
use std::collections::HashMap;

/// File-level actions a command asks the caller to perform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExEffect {
    Magit(crate::magit::Action),
    /// Show a message.
    Message(String),
    /// `:draw`: toggle drawing mode.
    Draw,
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
    /// Show a directory in the system's file manager.
    Reveal(std::path::PathBuf),
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
    /// `:[range]occur {regexp}`: list the matching lines (see `occur`).
    Occur {
        pat: String,
        range: Option<(usize, usize)>,
    },
    /// `gr` in an occur buffer: search again.
    OccurRevert,
    /// `:claude`, `Space c` then a key: show the chat and do that.
    Claude(crate::claude::Action),
    /// `:terminal`: a Ghostty terminal in this window; `Space t` (`split`)
    /// in a new one below it (see `term`).
    Terminal {
        split: bool,
    },
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
    /// `:set`, `:setlocal`, `:setglobal` with their arguments.
    Set {
        args: String,
        level: crate::options::Level,
    },
    /// `:b [N|name|#]`, `:bn`, `:bp`, `:bd[!] [N|name]`, `:ls`.
    Buffer {
        cmd: BufCmd,
        arg: String,
        force: bool,
    },
    /// `:close`, `:hide`: close the window, keeping its buffer.
    Close {
        force: bool,
    },
    /// `:only`: close every other window.
    Only,
    /// `:qall`, `:wqall` (`write`), `:xall`.
    QuitAll {
        force: bool,
        write: bool,
    },
    /// `:help`: the commands, in a picker searching for this.
    Help(String),
    /// Zap picked a word in another window: go there.
    ZapTo {
        win: usize,
        line: usize,
        byte: usize,
    },
    /// A `CTRL-W` command (`:wincmd`), with its count.
    Window {
        cmd: char,
        count: Option<usize>,
    },
    /// `:[N]split`, `:vsplit`, `:new`, `:vnew`, `:sbuffer`: `size` text rows
    /// (columns when `vertical`) for the new window.
    Split {
        vertical: bool,
        what: SplitWhat,
        size: Option<usize>,
    },
}

/// What a new window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitWhat {
    /// The current buffer.
    Same,
    /// A file, as `:e`.
    File(String),
    /// A new unnamed buffer.
    Empty,
    /// A buffer, as `:b`.
    Buffer(String),
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
    /// The last `:substitute`, for `:&`, `:~` and `~` in a replacement.
    pub last_sub: Option<&'a mut LastSub>,
    /// Line splices applied by the running command: (at, removed, inserted).
    log: Vec<(usize, usize, usize)>,
}

/// What `:s` did last: its pattern, replacement and flags.
#[derive(Clone, Debug, Default)]
pub struct LastSub {
    pat: Option<String>,
    rep: Option<String>,
    flags: SubFlags,
}

#[derive(Clone, Copy, Debug, Default)]
struct SubFlags {
    global: bool,
    /// `i` (Some(true)) or `I`.
    ignore_case: Option<bool>,
    /// `e`: no error when nothing matches.
    quiet: bool,
    /// `n`: count the matches, change nothing.
    count_only: bool,
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
            last_sub: None,
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
    if let Some((n, rest)) = count_prefix(line) {
        let at = Range {
            start: n - 1,
            end: n - 1,
        };
        return run_named(st, Some(at), rest, st.cur, in_global);
    }
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
    run_named(st, range, rest, cur, in_global)
}

/// `:5split`: the number before a window command is a count, not a line,
/// so it may be past the end. Returns the count (at least 1) and the rest.
fn count_prefix(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let rest = t.trim_start_matches(|c: char| c.is_ascii_digit());
    let n: usize = t[..t.len() - rest.len()].parse().ok()?;
    let name_end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let cmd = lookup(&rest[..name_end])?;
    (cmd.flags & COUNT != 0).then_some((n.max(1), rest))
}

/// Run the named command at the start of `rest`, given its `range`.
fn run_named(
    st: &mut ExState,
    range: Option<Range>,
    rest: &str,
    cur: usize,
    in_global: bool,
) -> Result<ExEffect, String> {
    // `:&`, `:~`, `:<`, `:>`, `:=` are named by one character.
    let name_end = match rest.chars().next() {
        Some('&' | '~' | '<' | '>' | '=' | '#') => 1,
        _ => rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len()),
    };
    let name = &rest[..name_end];
    let cmd = lookup(name).ok_or_else(|| format!("E492: Not an editor command: {rest}"))?;
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
    if arg.is_empty() && cmd.flags & NEEDARG != 0 {
        return Err(format!(":{} needs {}", cmd.name, cmd.usage));
    }
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
/// The argument is required (Tab adds the space before it).
pub const NEEDARG: u8 = 16;
/// The argument is option names (Tab completes them).
pub const OPTION: u8 = 32;
/// A leading number is a count, not a line (`:5split`); it takes RANGE too.
const COUNT: u8 = 64;

/// A named command, as in vim's `ex_cmds.lua`: `min` is how short an
/// abbreviation may be (`:w` for `:write`).
pub struct Command {
    pub name: &'static str,
    pub min: usize,
    pub flags: u8,
    /// What its argument looks like, for `:help` and the error when it's
    /// missing.
    usage: &'static str,
    /// What it does, in a line, for `:help`.
    pub about: &'static str,
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

impl Command {
    const fn about(self, about: &'static str) -> Self {
        Command { about, ..self }
    }

    /// What an optional argument looks like, for `:help`.
    const fn takes(self, usage: &'static str) -> Self {
        Command { usage, ..self }
    }

    /// How to write it, as in `:help`: `:[range]s[ubstitute]/pattern/…`.
    pub fn synopsis(&self) -> String {
        let mut s = String::from(":");
        if self.flags & COUNT != 0 {
            s += "[N]";
        } else if self.flags & RANGE != 0 {
            s += "[range]";
        }
        s += &self.name[..self.min];
        if self.min < self.name.len() {
            s += &format!("[{}]", &self.name[self.min..]);
        }
        if self.flags & BANG != 0 {
            s += "[!]";
        }
        let arg = match self.usage {
            "" if self.flags & FILE != 0 => "[file]",
            "" if self.flags & EXTRA != 0 => "[arg]",
            u => u,
        };
        if !arg.is_empty() && !arg.starts_with('/') {
            s.push(' ');
        }
        s + arg
    }
}

/// The command being typed on the `:` line, after any range: for the help
/// shown beside it.
pub fn typed(line: &str) -> Option<&'static Command> {
    let mut rest = line;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || " .$%,;+-".contains(c));
        // A mark (`:'a,'bd`): the quote and its letter.
        match rest.strip_prefix('\'') {
            Some(r) => rest = r.get(1..).unwrap_or(""),
            None => break,
        }
    }
    let name = &rest[..rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len())];
    lookup(name)
}

/// Every named command. A prefix runs the first entry it abbreviates, so
/// order decides ties, as in vim (`:s` is `:substitute`).
pub const COMMANDS: &[Command] = &[
    cmd("substitute", 1, RANGE | EXTRA, |st, a| {
        substitute(st, a, Repeat::No)
    })
    .about("replace a pattern in lines")
    .takes("/pattern/replacement/[&cegiInr] [count]"),
    cmd("&", 1, RANGE | EXTRA, |st, a| {
        substitute(st, a, Repeat::Last)
    })
    .about("repeat the last :substitute (&& keeps its flags)")
    .takes("[&][flags] [count]"),
    cmd("~", 1, RANGE | EXTRA, |st, a| {
        substitute(st, a, Repeat::Search)
    })
    .about("repeat the last :substitute with the last search pattern")
    .takes("[&][flags] [count]"),
    needs("global", 1, RANGE | BANG, "/pattern/command", |st, a| {
        global_cmd(st, a, !a.force)
    })
    .about("run a command on every line matching a pattern (! for the rest)"),
    needs("vglobal", 1, RANGE, "/pattern/command", |st, a| {
        global_cmd(st, a, false)
    })
    .about("run a command on every line not matching a pattern"),
    cmd("delete", 1, RANGE, |st, a| {
        st.splice(a.at.start, a.at.end - a.at.start + 1, vec![]);
        st.cur = a.at.start.min(st.buf.len_lines() - 1);
        Ok(ExEffect::None)
    })
    .about("delete lines"),
    cmd("join", 1, RANGE | BANG, join).about("join lines (! without adding spaces)"),
    needs("move", 1, RANGE, "{address}", |st, a| {
        move_copy(st, a, false)
    })
    .about("move lines to below a line"),
    needs("t", 1, RANGE, "{address}", |st, a| move_copy(st, a, true))
        .about("copy lines to below a line, as :copy"),
    needs("copy", 2, RANGE, "{address}", |st, a| {
        move_copy(st, a, true)
    })
    .about("copy lines to below a line"),
    cmd("write", 1, RANGE | BANG | EXTRA | FILE, |_, a| {
        write(a, false)
    })
    .about("save the buffer, or lines, to its file or another"),
    cmd("wq", 2, RANGE | BANG | EXTRA | FILE, |_, a| write(a, true)).about("save, then close"),
    cmd("xit", 1, 0, |_, _| Ok(ExEffect::WriteIfModifiedQuit)).about("save if changed, then close"),
    cmd("quit", 1, BANG, |_, a| {
        Ok(ExEffect::Quit { force: a.force })
    })
    .about("close the window; the last one quits (! drops changes)"),
    cmd("edit", 1, BANG | EXTRA | FILE, |_, a| {
        Ok(ExEffect::Edit {
            path: path_arg(a.arg)?,
            force: a.force,
        })
    })
    .about("open a file; alone, reload this one"),
    needs("read", 1, RANGE | FILE, "{file} or !{command}", |st, a| {
        let at = a.range.map_or(st.cur, |r| r.end);
        read(st, at, a.arg)
    })
    .about("insert a file, or a command's output, below the line"),
    needs("occur", 3, RANGE, "{regexp}", |_, a| {
        Ok(ExEffect::Occur {
            pat: a.arg.to_owned(),
            range: a.range.map(|r| (r.start, r.end)),
        })
    })
    .about("list the matching lines in an editable buffer"),
    cmd("claude", 3, 0, |_, _| {
        Ok(ExEffect::Claude(crate::claude::Action::Chat))
    })
    .about("chat with Claude Code in a buffer"),
    cmd("draw", 3, 0, |_, _| Ok(ExEffect::Draw))
        .about("draw ASCII art with the mouse; again, or Esc, stops"),
    cmd("pwd", 2, 0, |_, _| Ok(ExEffect::Pwd)).about("show the working directory"),
    cmd("terminal", 2, 0, |_, _| {
        Ok(ExEffect::Terminal { split: false })
    })
    .about("your shell in a Ghostty terminal, in this window; jj or Ctrl-\\ Ctrl-n leaves"),
    cmd("set", 2, EXTRA | OPTION, |_, a| set(a, Level::Both))
        .about("show or change options")
        .takes("[option...]"),
    cmd("setlocal", 4, EXTRA | OPTION, |_, a| set(a, Level::Local))
        .about("change an option for this buffer or window only")
        .takes("[option...]"),
    cmd("setglobal", 4, EXTRA | OPTION, |_, a| set(a, Level::Global))
        .about("change an option's global value")
        .takes("[option...]"),
    cmd("cd", 2, EXTRA | FILE, |_, a| {
        Ok(ExEffect::Cd(path_arg(a.arg)?))
    })
    .about("change the working directory")
    .takes("[dir]"),
    cmd("buffer", 1, BANG | EXTRA, |_, a| buffer(a, BufCmd::Go))
        .about("go to a buffer by number or name")
        .takes("[N|name]"),
    cmd("bnext", 2, BANG, |_, a| buffer(a, BufCmd::Next)).about("go to the next buffer"),
    cmd("bprevious", 2, BANG, |_, a| buffer(a, BufCmd::Prev)).about("go to the previous buffer"),
    cmd("bNext", 2, BANG, |_, a| buffer(a, BufCmd::Prev))
        .about("go to the previous buffer, as :bprevious"),
    cmd("bdelete", 2, BANG | EXTRA, |_, a| buffer(a, BufCmd::Delete))
        .about("close a buffer")
        .takes("[N|name]"),
    cmd("ls", 2, BANG, |_, a| buffer(a, BufCmd::List)).about("list the open buffers"),
    cmd("close", 3, BANG, |_, a| {
        Ok(ExEffect::Close { force: a.force })
    })
    .about("close this window"),
    cmd("only", 2, BANG, |_, _| Ok(ExEffect::Only)).about("close every other window"),
    cmd("hide", 3, BANG, |_, a| {
        Ok(ExEffect::Close { force: a.force })
    })
    .about("close this window, as :close"),
    cmd("qall", 2, BANG, |_, a| quit_all(a, false)).about("quit fred (! drops changes)"),
    cmd("quitall", 5, BANG, |_, a| quit_all(a, false)).about("quit fred, as :qall"),
    cmd("wqall", 3, BANG, |_, a| quit_all(a, true)).about("save every changed buffer, then quit"),
    cmd("xall", 2, BANG, |_, a| quit_all(a, true))
        .about("save every changed buffer, then quit, as :wqall"),
    cmd("split", 2, RANGE | COUNT | EXTRA | FILE, |_, a| {
        split(a, false, false)
    })
    .about("split the window, optionally opening a file"),
    cmd("vsplit", 2, RANGE | COUNT | EXTRA | FILE, |_, a| {
        split(a, true, false)
    })
    .about("split the window side by side"),
    cmd("new", 3, RANGE | COUNT | EXTRA | FILE, |_, a| {
        split(a, false, true)
    })
    .about("split, with a new empty buffer"),
    cmd("vnew", 3, RANGE | COUNT | EXTRA | FILE, |_, a| {
        split(a, true, true)
    })
    .about("split side by side, with a new empty buffer"),
    cmd("sbuffer", 2, RANGE | COUNT | EXTRA, |_, a| {
        Ok(ExEffect::Split {
            vertical: false,
            what: SplitWhat::Buffer(a.arg.to_owned()),
            size: a.range.map(|r| r.end + 1),
        })
    })
    .about("split, showing a buffer")
    .takes("[N|name]"),
    needs("wincmd", 4, RANGE | COUNT, "{arg}", |_, a| {
        let mut c = a.arg.chars();
        match (c.next(), c.next()) {
            (Some(cmd), None) => Ok(ExEffect::Window {
                cmd,
                count: a.range.map(|r| r.end + 1),
            }),
            _ => Err("E471: Argument required".into()),
        }
    })
    .about("run a Ctrl-W window command"),
    cmd("buffers", 7, BANG, |_, a| buffer(a, BufCmd::List)).about("list the open buffers, as :ls"),
    cmd("files", 5, BANG, |_, a| buffer(a, BufCmd::List)).about("list the open buffers, as :ls"),
    needs("ai", 2, RANGE, "{instruction}", |st, a| {
        ai(st, a.at, a.arg, false)
    })
    .about("have Claude change the lines"),
    cmd("explain", 3, RANGE | EXTRA, |st, a| {
        ai(st, a.at, a.arg, true)
    })
    .about("have Claude explain the lines")
    .takes("[question]"),
    // :Magit NAME runs an upstream Magit command by name (like M-x);
    // alone, magit-status.
    cmd("Magit", 5, EXTRA, |_, a| {
        let name = if a.arg.is_empty() {
            "magit-status"
        } else {
            a.arg
        };
        crate::magit::commands::by_name(name)
            .map(ExEffect::Magit)
            .ok_or_else(|| format!("unknown Magit command: {}", a.arg))
    })
    .about("run a Magit command by name; alone, magit-status")
    .takes("[command]"),
    cmd("help", 1, EXTRA, |_, a| {
        Ok(ExEffect::Help(a.arg.to_owned()))
    })
    .about("list the commands; with a name, find it")
    .takes("[command]"),
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
        usage: "",
        about: "",
        run,
    }
}

/// A command that must be given an argument shaped like `usage`.
const fn needs(
    name: &'static str,
    min: usize,
    flags: u8,
    usage: &'static str,
    run: fn(&mut ExState, Args) -> Result<ExEffect, String>,
) -> Command {
    Command {
        name,
        min,
        flags: flags | EXTRA | NEEDARG,
        usage,
        about: "",
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

fn quit_all(a: Args, write: bool) -> Result<ExEffect, String> {
    Ok(ExEffect::QuitAll {
        force: a.force,
        write,
    })
}

/// `:[N]split [file]` and friends: `empty` for `:new` without a file.
fn split(a: Args, vertical: bool, empty: bool) -> Result<ExEffect, String> {
    let what = match path_arg(a.arg)? {
        Some(p) => SplitWhat::File(p),
        None if empty => SplitWhat::Empty,
        None => SplitWhat::Same,
    };
    Ok(ExEffect::Split {
        vertical,
        what,
        // `:5split`: the count is the line number the range parsed.
        size: a.range.map(|r| r.end + 1),
    })
}

fn write(a: Args, then_quit: bool) -> Result<ExEffect, String> {
    Ok(ExEffect::Write {
        path: path_arg(a.arg)?,
        force: a.force,
        range: a.range,
        then_quit,
    })
}

fn set(a: Args, level: Level) -> Result<ExEffect, String> {
    Ok(ExEffect::Set {
        args: a.raw.trim().to_string(),
        level,
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
        // `:j!` keeps white space as it is.
        let lines = st.lines(r).into_iter();
        let joined = match a.force {
            true => lines.collect::<String>(),
            false => lines
                .reduce(|a, b| crate::text::join_lines(&a, &b))
                .unwrap_or_default(),
        };
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

/// How `:s` gets its pattern and replacement.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Repeat {
    /// `:s/pat/rep/`, or `:s [flags]` repeating the last.
    No,
    /// `:&`: the last pattern and replacement.
    Last,
    /// `:~`: the last search pattern with the last replacement.
    Search,
}

/// A replacement as Vim expands it: `&` and `\0` the match, `\1`-`\9`
/// groups, `\u \l` the next character's case, `\U \L` until `\E`/`\e`,
/// `\r` (and `\n`, as ed) a line break, `\t` a tab.
fn expand(rep: &str, caps: &Captures) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Case {
        None,
        Upper,
        Lower,
    }
    let mut out = String::new();
    let mut one = Case::None;
    let mut all = Case::None;
    let push = |out: &mut String, s: &str, one: &mut Case, all: Case| {
        for c in s.chars() {
            let case = match *one {
                Case::None => all,
                o => {
                    *one = Case::None;
                    o
                }
            };
            match case {
                Case::Upper => out.extend(c.to_uppercase()),
                Case::Lower => out.extend(c.to_lowercase()),
                Case::None => out.push(c),
            }
        }
    };
    let mut it = rep.chars();
    while let Some(c) = it.next() {
        match c {
            '&' => push(&mut out, &caps[0], &mut one, all),
            '\r' => out.push('\n'),
            '\\' => match it.next() {
                Some(d @ '0'..='9') => {
                    let i = d as usize - '0' as usize;
                    push(
                        &mut out,
                        caps.get(i).map_or("", |m| m.as_str()),
                        &mut one,
                        all,
                    );
                }
                // `\r` is vim's way to split a line; `\n` is ed's.
                Some('n' | 'r' | '\r') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => one = Case::Upper,
                Some('l') => one = Case::Lower,
                Some('U') => all = Case::Upper,
                Some('L') => all = Case::Lower,
                Some('e' | 'E') => {
                    all = Case::None;
                    one = Case::None;
                }
                Some(o) => push(&mut out, &o.to_string(), &mut one, all),
                None => out.push('\\'),
            },
            _ => push(&mut out, &c.to_string(), &mut one, all),
        }
    }
    out
}

/// `~` in a new replacement is the previous one (`\~` a tilde).
fn regtilde(rep: &str, prev: Option<&str>) -> String {
    let mut out = String::new();
    let mut it = rep.chars();
    while let Some(c) = it.next() {
        match c {
            '~' => out.push_str(prev.unwrap_or("")),
            '\\' => {
                out.push('\\');
                if let Some(n) = it.next() {
                    out.push(n);
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// `:s`, `:&` and `:~`: flags `& c e g i I n r` and a count after them.
fn substitute(st: &mut ExState, a: Args, repeat: Repeat) -> Result<ExEffect, String> {
    let mut last = st.last_sub.as_deref().cloned().unwrap_or_default();
    let spec = a.raw.trim_start();
    let delim = spec
        .chars()
        .next()
        .filter(|c| repeat == Repeat::No && !c.is_alphanumeric() && !"\\\"| &".contains(*c));
    let (mut pat, rest, mut flags) = match delim {
        Some(d) => {
            let (pat, rest) = read_delimited(&spec[d.len_utf8()..], d);
            let (rep, rest) = read_delimited(rest, d);
            last.rep = Some(regtilde(&rep, last.rep.as_deref()));
            (Some(pat), rest, SubFlags::default())
        }
        None => {
            let pat = match repeat {
                Repeat::Search => st.last_pat.clone(),
                _ => last.pat.clone(),
            };
            (
                Some(pat.ok_or("E35: No previous regular expression")?),
                spec,
                SubFlags::default(),
            )
        }
    };
    // `&` first keeps the flags of the last one.
    let mut rest = rest;
    if let Some(r) = rest.strip_prefix('&') {
        flags = last.flags;
        rest = r;
    }
    let mut chars = rest.char_indices().peekable();
    let mut end = rest.len();
    while let Some(&(i, f)) = chars.peek() {
        match f {
            'g' => flags.global = !flags.global,
            'i' => flags.ignore_case = Some(true),
            'I' => flags.ignore_case = Some(false),
            'e' => flags.quiet = true,
            'n' => flags.count_only = true,
            // The last search pattern instead of the last :s one.
            'r' if delim.is_none() => pat = st.last_pat.clone(),
            'c' => return Err("E1: confirm (c) is not supported".into()),
            'p' | '#' | 'l' => {}
            _ => {
                end = i;
                break;
            }
        }
        chars.next();
    }
    let tail = rest[end..].trim();
    let count: Option<usize> = match tail {
        "" => None,
        t if t.chars().all(|c| c.is_ascii_digit()) => {
            Some(t.parse().map_err(|_| "E939: Positive count required")?)
        }
        t => return Err(format!("E488: Trailing characters: {t}")),
    };
    let pat = pat.unwrap_or_default();
    let pat = use_pattern(pat, st.last_pat)?;
    last.pat = Some(pat.clone());
    last.flags = flags;
    let rep = last.rep.clone().unwrap_or_default();
    if let Some(slot) = st.last_sub.as_deref_mut() {
        *slot = last;
    }
    let mut r = a.at;
    if let Some(n) = count {
        if n == 0 {
            return Err("E939: Positive count required".into());
        }
        r = Range {
            start: r.end,
            end: (r.end + n - 1).min(st.buf.len_lines() - 1),
        };
    }
    let case = match flags.ignore_case {
        Some(true) => "\\c",
        Some(false) => "\\C",
        None => "",
    };
    let re = search::compile_vim(&format!("{case}{pat}"), false)?;
    let mut last_line = None;
    let (mut subs, mut lines) = (0, 0);
    for l in (r.start..=r.end).rev() {
        let line = st.buf.line(l);
        let Some((new, n)) = subst_line(&re, &line, &rep, flags.global) else {
            continue;
        };
        subs += n;
        lines += 1;
        if flags.count_only {
            continue;
        }
        let parts: Vec<String> = new.split('\n').map(String::from).collect();
        let added = parts.len() - 1;
        st.splice(l, 1, parts);
        if last_line.is_none() {
            last_line = Some(l + added);
        }
    }
    if subs == 0 {
        return if flags.quiet || a.in_global {
            Err(NO_MATCH.into())
        } else {
            Err(format!("E486: Pattern not found: {pat}"))
        };
    }
    if let Some(l) = last_line {
        st.cur = l;
    }
    let s = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    if flags.count_only {
        return Ok(ExEffect::Message(format!(
            "{} on {}",
            s(subs, "match", "matches"),
            s(lines, "line", "lines")
        )));
    }
    if subs > 2 {
        return Ok(ExEffect::Message(format!(
            "{} on {}",
            s(subs, "substitution", "substitutions"),
            s(lines, "line", "lines")
        )));
    }
    Ok(ExEffect::None)
}

/// A substitute inside `:g` that found nothing on its line (not an error
/// unless it found nothing on any), or one with the `e` flag.
const NO_MATCH: &str = "no match";

fn subst_line(re: &Regex, line: &str, rep: &str, global: bool) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut prev = 0;
    let mut n = 0;
    for caps in re.captures_iter(line) {
        let m = caps.get(0).expect("group 0");
        out.push_str(&line[prev..m.start()]);
        out.push_str(&expand(rep, &caps));
        prev = m.end();
        n += 1;
        if !global {
            break;
        }
    }
    (n > 0).then(|| (out + &line[prev..], n))
}

fn global(st: &mut ExState, r: Range, spec: &str, want: bool) -> Result<ExEffect, String> {
    let delim = spec.chars().next().ok_or("bad global")?;
    let (pat, cmd) = read_delimited(&spec[delim.len_utf8()..], delim);
    if cmd.trim().is_empty() {
        return Err("command expected".into());
    }
    let pat = use_pattern(pat, st.last_pat)?;
    let re = search::compile_vim(&pat, false)?;
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
            Err(e) if e == NO_MATCH || e.starts_with("E486") => no_match = true,
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
            ex(t, 0, "s/\\(o\\)\\(n\\)/\\2\\1&/"),
            ("noone\ntwo\nthree\nfour".into(), 0)
        );
        assert_eq!(ex(t, 0, "s/o/\\&/"), ("&ne\ntwo\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "s#o#/#"), ("/ne\ntwo\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "1,2j"), ("one two\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "j"), ("one two\nthree\nfour".into(), 0));
        assert_eq!(ex(t, 0, "j!"), ("onetwo\nthree\nfour".into(), 0));
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
        assert_eq!(ex_err("a", 0, "m"), Err(":move needs {address}".into()));
        // Every command runs bare or says what it needs.
        for c in COMMANDS {
            let (_, _, r) = run_on("a\nb", 0, c.name);
            if let Err(e) = r {
                // Or, repeating the last of something, that there is none.
                assert!(
                    e.starts_with(&format!(":{} needs", c.name)) || e.starts_with("E35"),
                    "{e}"
                );
            }
        }
    }

    /// What `:` completes and `:help` lists is what runs: each name, and its
    /// shortest abbreviation, runs that command, and each says what it does.
    #[test]
    fn every_command_is_reachable_and_described() {
        for c in COMMANDS {
            assert_eq!(lookup(c.name).map(|l| l.name), Some(c.name));
            assert_eq!(
                lookup(&c.name[..c.min]).map(|l| l.name),
                Some(c.name),
                "{}",
                c.name
            );
            if c.min > 1 {
                let short = lookup(&c.name[..c.min - 1]).map(|l| l.name);
                assert_ne!(short, Some(c.name), "{} runs shorter than its min", c.name);
            }
            assert!(!c.about.is_empty(), "{} has no help", c.name);
        }
    }

    #[test]
    fn synopsis_and_typed() {
        let syn = |n: &str| lookup(n).unwrap().synopsis();
        assert_eq!(
            syn("s"),
            ":[range]s[ubstitute]/pattern/replacement/[&cegiInr] [count]"
        );
        assert_eq!(syn("q"), ":q[uit][!]");
        assert_eq!(syn("sp"), ":[N]sp[lit] [file]");
        assert_eq!(syn("b"), ":b[uffer][!] [N|name]");
        assert_eq!(syn("ai"), ":[range]ai {instruction}");
        assert_eq!(syn("t"), ":[range]t {address}");
        let name = |l: &str| typed(l).map(|c| c.name);
        assert_eq!(name("sp"), Some("split"));
        assert_eq!(name("1,$s/a/b/"), Some("substitute"));
        assert_eq!(name("'a,'bd"), Some("delete"));
        assert_eq!(name(".,+3 j"), Some("join"));
        assert_eq!(name("%"), None);
        assert_eq!(name("zz"), None);
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
        assert_eq!(ex("Foo foo", 0, "s/\\cfoo/x/g"), ("x x".into(), 0));
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
