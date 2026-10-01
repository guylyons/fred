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
    Edit {
        path: String,
        force: bool,
    },
}

/// What ex commands operate on: borrowed pieces of the editor.
pub struct ExState<'a> {
    pub buf: &'a mut Buffer,
    pub undo: &'a mut Undo,
    /// Current line (0-based), updated by commands.
    pub cur: usize,
    pub marks: &'a HashMap<char, usize>,
    pub last_pat: &'a mut Option<String>,
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
    let r = run_one(st, line, false);
    if r.is_err() {
        // Roll back anything a failing command (e.g. inside `g`) already did.
        st.undo.end(pos);
        if !st.log.is_empty() {
            st.undo.undo(st.buf);
        }
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
    let at_cur = Range {
        start: cur,
        end: cur,
    };
    let mut chars = rest.chars();
    let Some(c) = chars.next() else {
        if let Some(r) = range {
            st.cur = r.end;
        }
        return Ok(ExEffect::None);
    };
    let after = &rest[c.len_utf8()..];
    let delim_follows = after
        .chars()
        .next()
        .is_some_and(|d| !d.is_alphanumeric() && !d.is_whitespace() && d != '!');
    match c {
        's' if delim_follows => substitute(st, range.unwrap_or(at_cur), after),
        'g' | 'v' if delim_follows => {
            if in_global {
                return Err("nested global".into());
            }
            let all = Range {
                start: 0,
                end: st.buf.len_lines() - 1,
            };
            global(st, range.unwrap_or(all), after, c == 'g')
        }
        'd' if after.trim().is_empty() => {
            let r = range.unwrap_or(at_cur);
            st.splice(r.start, r.end - r.start + 1, vec![]);
            st.cur = r.start.min(st.buf.len_lines() - 1);
            Ok(ExEffect::None)
        }
        'j' if after.trim().is_empty() => {
            let r = match range {
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
        'm' | 't' => {
            let r = range.unwrap_or(at_cur);
            let (dest, tail) = {
                let mut ctx = st.addr_ctx();
                parse_addr(after, &mut ctx)?
            };
            if !tail.trim().is_empty() {
                return Err(format!("unexpected: {}", tail.trim()));
            }
            let dest = dest.ok_or("destination expected")? as usize;
            let lines = st.lines(r);
            let count = lines.len();
            if c == 't' {
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
        _ if c.is_ascii_alphabetic() => file_command(rest, range),
        _ => Err(format!("unknown command: {c}")),
    }
}

fn file_command(rest: &str, range: Option<Range>) -> Result<ExEffect, String> {
    let name_end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    let mut tail = &rest[name_end..];
    let force = tail.starts_with('!');
    if force {
        tail = &tail[1..];
    }
    if !tail.is_empty() && !tail.starts_with(char::is_whitespace) {
        return Err(format!("unknown command: {rest}"));
    }
    let arg = tail.trim();
    if arg.starts_with('!') {
        return Err("shell commands are not supported".into());
    }
    let path = (!arg.is_empty()).then(|| arg.to_string());
    let no_arg = |e: ExEffect| {
        if path.is_some() {
            Err(format!("unexpected argument: {arg}"))
        } else {
            Ok(e)
        }
    };
    match name {
        "w" | "write" => Ok(ExEffect::Write {
            path,
            force,
            range,
            then_quit: false,
        }),
        "wq" => Ok(ExEffect::Write {
            path,
            force,
            range,
            then_quit: true,
        }),
        "x" | "xit" => no_arg(ExEffect::WriteIfModifiedQuit),
        "q" | "quit" => no_arg(ExEffect::Quit { force }),
        "e" | "edit" => match path {
            Some(path) => Ok(ExEffect::Edit { path, force }),
            None => Err("file name expected".into()),
        },
        _ => Err(format!("unknown command: {name}")),
    }
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
                Some('n') => out.push('\n'),
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
    let re = search::compile(&pat)?;
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
    let re = search::compile(&pat)?;
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
                path: "f".into(),
                force: true
            }
        );
        assert!(run_on("a", 0, "e").2.is_err());
    }
}
