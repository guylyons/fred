//! ed address parsing.

use crate::buffer::Buffer;
use crate::search;
use std::collections::HashMap;

/// Inclusive, 0-based line range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

pub struct AddrCtx<'a> {
    pub buf: &'a Buffer,
    pub cur: usize,
    pub marks: &'a HashMap<char, usize>,
    pub last_pat: &'a mut Option<String>,
}

/// Read a `/re/` style pattern after the opening delimiter. Returns pattern and rest.
pub fn read_delimited(input: &str, delim: char) -> (String, &str) {
    let mut out = String::new();
    let mut chars = input.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == delim {
            return (out, &input[i + c.len_utf8()..]);
        }
        if c == '\\' {
            match chars.next() {
                Some((_, n)) if n == delim => out.push(n),
                Some((_, n)) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    (out, "")
}

/// Resolve an empty pattern to the last one, or remember a new one.
pub fn use_pattern(pat: String, last: &mut Option<String>) -> Result<String, String> {
    if pat.is_empty() {
        last.clone()
            .ok_or_else(|| "no previous pattern".to_string())
    } else {
        *last = Some(pat.clone());
        Ok(pat)
    }
}

fn number(s: &str) -> (Option<i64>, &str) {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return (None, s);
    }
    (s[..end].parse().ok(), &s[end..])
}

/// Parse one address as a 1-based line number (0 allowed). `None` if absent.
pub fn parse_addr<'i>(input: &'i str, ctx: &mut AddrCtx) -> Result<(Option<i64>, &'i str), String> {
    let s = input.trim_start();
    let cur = ctx.cur as i64 + 1;
    let last = ctx.buf.len_lines() as i64;
    let (mut base, mut rest) = match s.chars().next() {
        Some(c) if c.is_ascii_digit() => {
            let (n, r) = number(s);
            (n, r)
        }
        Some('.') => (Some(cur), &s[1..]),
        Some('$') => (Some(last), &s[1..]),
        Some('\'') => {
            let mut it = s[1..].chars();
            let m = it.next().ok_or("mark not set")?;
            let line = ctx.marks.get(&m).ok_or("mark not set")?;
            (Some(*line as i64 + 1), &s[1 + m.len_utf8()..])
        }
        Some(d @ ('/' | '?')) => {
            let (pat, r) = read_delimited(&s[1..], d);
            let pat = use_pattern(pat, ctx.last_pat)?;
            let re = search::compile(&pat)?;
            let line = search::find_line(ctx.buf, &re, ctx.cur, d == '/').ok_or("no match")?;
            (Some(line as i64 + 1), r)
        }
        _ => (None, s),
    };
    // Offsets: +n, -n, bare + / -.
    loop {
        let t = rest.trim_start();
        let sign = match t.chars().next() {
            Some('+') => 1,
            Some('-' | '^') => -1,
            _ => break,
        };
        let (n, r) = number(&t[1..]);
        base = Some(base.unwrap_or(cur) + sign * n.unwrap_or(1));
        rest = r;
    }
    match base {
        Some(n) if n < 0 || n > last => Err("invalid address".into()),
        _ => Ok((base, rest)),
    }
}

/// Parse an optional `addr[,|;addr]` range; returns it and the rest of the input.
pub fn parse_range<'i>(
    input: &'i str,
    ctx: &mut AddrCtx,
) -> Result<(Option<Range>, &'i str), String> {
    let last = ctx.buf.len_lines() as i64;
    // `%` is vim's name for the whole file, like ed's `,`.
    if let Some(rest) = input.trim_start().strip_prefix('%') {
        return Ok((
            Some(Range {
                start: 0,
                end: last as usize - 1,
            }),
            rest,
        ));
    }
    let (a1, rest) = parse_addr(input, ctx)?;
    let t = rest.trim_start();
    let (a, b, rest) = match t.chars().next() {
        Some(sep @ (',' | ';')) => {
            let first = a1.unwrap_or(if sep == ',' { 1 } else { ctx.cur as i64 + 1 });
            if sep == ';' {
                ctx.cur = (first.max(1) - 1) as usize;
            }
            let (a2, rest) = parse_addr(&t[1..], ctx)?;
            let second = a2.unwrap_or(if a1.is_some() { first } else { last });
            (first, second, rest)
        }
        _ => match a1 {
            Some(n) => (n, n, rest),
            None => return Ok((None, rest)),
        },
    };
    if a < 1 || b < 1 {
        return Err("invalid address".into());
    }
    if a > b {
        return Err("invalid range".into());
    }
    Ok((
        Some(Range {
            start: a as usize - 1,
            end: b as usize - 1,
        }),
        rest,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use std::collections::HashMap;

    fn r(buf: &str, input: &str, cur: usize) -> Result<Option<(usize, usize)>, String> {
        let b = Buffer::from_text(buf);
        let marks = HashMap::from([('a', 1)]);
        let mut lp = None;
        let mut ctx = AddrCtx {
            buf: &b,
            cur,
            marks: &marks,
            last_pat: &mut lp,
        };
        parse_range(input, &mut ctx).map(|(r, _)| r.map(|r| (r.start, r.end)))
    }

    #[test]
    fn addresses() {
        let t = "a\nb\nc\nd\ne";
        assert_eq!(r(t, "d", 2), Ok(None));
        assert_eq!(r(t, "2d", 0), Ok(Some((1, 1))));
        assert_eq!(r(t, ".,$d", 2), Ok(Some((2, 4))));
        assert_eq!(r(t, ",d", 2), Ok(Some((0, 4))));
        assert_eq!(r(t, "%d", 2), Ok(Some((0, 4))));
        assert_eq!(r(t, "%s/a/b/", 2), Ok(Some((0, 4))));
        assert_eq!(r(t, ";d", 2), Ok(Some((2, 4))));
        assert_eq!(r(t, ".+1,+2d", 0), Ok(Some((1, 2))));
        assert_eq!(r(t, "$-1d", 0), Ok(Some((3, 3))));
        assert_eq!(r(t, "/d/d", 0), Ok(Some((3, 3))));
        assert_eq!(r(t, "/d/+1d", 0), Ok(Some((4, 4))));
        assert_eq!(r(t, "?b?d", 3), Ok(Some((1, 1))));
        assert_eq!(r(t, "'a,$d", 0), Ok(Some((1, 4))));
        assert_eq!(r(t, "2;+1d", 0), Ok(Some((1, 2))));
        assert_eq!(r(t, "100d", 0), Err("invalid address".into()));
        assert_eq!(r(t, "0d", 0), Err("invalid address".into()));
        assert_eq!(r(t, "4,2d", 0), Err("invalid range".into()));
        assert_eq!(r(t, "/zz/d", 0), Err("no match".into()));
        assert_eq!(r(t, "'z", 0), Err("mark not set".into()));
    }

    #[test]
    fn rest_is_returned() {
        let b = Buffer::from_text("a\nb");
        let marks = HashMap::new();
        let mut lp = None;
        let mut ctx = AddrCtx {
            buf: &b,
            cur: 0,
            marks: &marks,
            last_pat: &mut lp,
        };
        let (_, rest) = parse_range("1,2s/a/b/", &mut ctx).unwrap();
        assert_eq!(rest, "s/a/b/");
    }

    #[test]
    fn empty_pattern_reuses_last() {
        let b = Buffer::from_text("x\ny\nx");
        let marks = HashMap::new();
        let mut lp = Some("x".to_string());
        let mut ctx = AddrCtx {
            buf: &b,
            cur: 0,
            marks: &marks,
            last_pat: &mut lp,
        };
        let (r, _) = parse_range("//d", &mut ctx).unwrap();
        assert_eq!(r, Some(Range { start: 2, end: 2 }));
    }

    #[test]
    fn single_address_allows_zero() {
        let b = Buffer::from_text("a\nb");
        let marks = HashMap::new();
        let mut lp = None;
        let mut ctx = AddrCtx {
            buf: &b,
            cur: 0,
            marks: &marks,
            last_pat: &mut lp,
        };
        assert_eq!(parse_addr("0", &mut ctx).unwrap().0, Some(0));
        assert_eq!(parse_addr("$", &mut ctx).unwrap().0, Some(2));
    }
}
