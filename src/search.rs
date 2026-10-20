//! Regex search over the buffer.

use crate::buffer::Buffer;
use crate::text::next_grapheme;
use regex::Regex;

/// Compile a pattern exactly as written (ex commands, like ed).
pub fn compile_exact(pat: &str) -> Result<Regex, String> {
    Regex::new(pat).map_err(|e| regex_error(&e))
}

fn regex_error(e: &regex::Error) -> String {
    let msg = e.to_string();
    let last = msg
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    format!("bad pattern: {}", last.trim_start_matches("error: "))
}

/// Compile a pattern; case-insensitive unless it contains an uppercase letter.
pub fn compile(pat: &str) -> Result<Regex, String> {
    let src = if pat.chars().any(char::is_uppercase) {
        pat.to_string()
    } else {
        format!("(?i){pat}")
    };
    Regex::new(&src).map_err(|e| {
        let msg = e.to_string();
        let last = msg
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        format!("bad pattern: {}", last.trim_start_matches("error: "))
    })
}

/// Compile a Vim pattern, as `/`, `:s` and `:g` take it: `\(a\|b\)`,
/// `\<word\>`, `x\{2,}`, `\v`, `\c`; a bare `(`, `|`, `+` or `{` matches
/// itself. With `smart`, case is ignored unless the pattern has a capital.
pub fn compile_vim(pat: &str, smart: bool) -> Result<Regex, String> {
    let (src, ignore) = vim_regex(pat)?;
    let ignore = ignore.unwrap_or(smart && !has_upper(pat));
    let src = if ignore { format!("(?i){src}") } else { src };
    Regex::new(&src).map_err(|e| regex_error(&e))
}

/// A capital letter, escapes like `\S` aside (Vim's 'smartcase').
fn has_upper(pat: &str) -> bool {
    let mut it = pat.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            it.next();
        } else if c.is_uppercase() {
            return true;
        }
    }
    false
}

/// How much of a pattern is special without a backslash: `\v`, `\m`,
/// `\M` and `\V`.
#[derive(Clone, Copy, PartialEq)]
enum Magic {
    Very,
    On,
    Off,
    VeryOff,
}

/// A Vim pattern in Rust regex syntax, and whether `\c` (ignore case) or
/// `\C` said how to treat case.
fn vim_regex(pat: &str) -> Result<(String, Option<bool>), String> {
    let cs: Vec<char> = pat.chars().collect();
    let (mut magic, mut ignore, mut out) = (Magic::On, None, String::new());
    // `^` anchors only at the start of the pattern, a branch or a group.
    let mut start = true;
    let mut i = 0;
    while i < cs.len() {
        let esc = cs[i] == '\\' && i + 1 < cs.len();
        let c = cs[i + usize::from(esc)];
        i += 1 + usize::from(esc);
        let at_start = std::mem::replace(&mut start, false);
        if esc && c.is_ascii_alphanumeric() {
            match c {
                'v' => magic = Magic::Very,
                'm' => magic = Magic::On,
                'M' => magic = Magic::Off,
                'V' => magic = Magic::VeryOff,
                'c' | 'C' => ignore = Some(c == 'c'),
                '1'..='9' => return Err(format!("back-references (\\{c}) are not supported")),
                _ => {
                    out += vim_class(c).ok_or(format!("\\{c} is not supported"))?;
                    continue;
                }
            }
            start = at_start;
            continue;
        }
        let magic_on = matches!(magic, Magic::Very | Magic::On);
        let special = match c {
            '^' | '$' => !esc,
            '.' | '*' | '[' | '~' => esc != magic_on,
            '(' | ')' | '|' | '+' | '?' | '=' | '{' | '@' | '%' | '<' | '>' => {
                esc != (magic == Magic::Very)
            }
            '_' => esc,
            _ => false,
        };
        if !special {
            out += &regex::escape(c.encode_utf8(&mut [0; 4]));
            continue;
        }
        match c {
            '^' if at_start => out.push('^'),
            '$' if branch_end(&cs[i..], magic) => out.push('$'),
            '^' | '$' => {
                out.push('\\');
                out.push(c);
            }
            '*' if at_start => out += r"\*",
            '.' | '*' | '(' | ')' | '|' | '+' => out.push(c),
            '?' | '=' => out.push('?'),
            // shortcut: `~` matches itself, not the last :s replacement;
            // upgrade when :s remembers its replacement.
            '~' => out.push('~'),
            '<' => out += r"\b{start}",
            '>' => out += r"\b{end}",
            '{' => {
                let close = cs[i..].iter().position(|&c| c == '}').ok_or("missing } after \\{")?;
                let body: String = cs[i..i + close].iter().collect();
                i += close + 1;
                let body = body.strip_suffix('\\').unwrap_or(&body);
                let (lazy, body) = match body.strip_prefix('-') {
                    Some(b) => (true, b),
                    None => (false, body),
                };
                if !body.chars().all(|c| c.is_ascii_digit() || c == ',') {
                    return Err(format!("bad \\{{{body}}}"));
                }
                out += &match body.split_once(',') {
                    _ if body.is_empty() => "*".to_string(),
                    None => format!("{{{body}}}"),
                    Some((a, b)) => format!("{{{},{b}}}", if a.is_empty() { "0" } else { a }),
                };
                if lazy {
                    out.push('?');
                }
            }
            '[' => match class_end(&cs[i..]) {
                None => out += r"\[",
                Some(len) => {
                    out += &vim_bracket(&cs[i..i + len]);
                    i += len + 1;
                }
            },
            '%' => {
                let Some(&k) = cs.get(i) else {
                    return Err("bad \\%".into());
                };
                i += 1;
                if k == '(' {
                    out += "(?:";
                    start = true;
                    continue;
                }
                let (radix, max) = match k {
                    'd' => (10, 10),
                    'x' => (16, 2),
                    'u' => (16, 4),
                    'U' => (16, 8),
                    'o' => (8, 4),
                    _ => return Err(format!("\\%{k} is not supported")),
                };
                let n = cs[i..].iter().take(max).take_while(|c| c.is_digit(radix)).count();
                let code: String = cs[i..i + n].iter().collect();
                i += n;
                let ch = u32::from_str_radix(&code, radix)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or(format!("bad \\%{k}{code}"))?;
                out += &regex::escape(ch.encode_utf8(&mut [0; 4]));
            }
            '_' => return Err("multi-line \\_ is not supported".into()),
            '@' => return Err("look-around (\\@) is not supported".into()),
            _ => unreachable!("{c} is not special"),
        }
        if matches!(c, '(' | '|') {
            start = true;
        }
    }
    Ok((out, ignore))
}

/// Whether a `$` followed by `rest` ends the pattern, a branch or a group,
/// where Vim makes it an anchor.
fn branch_end(rest: &[char], magic: Magic) -> bool {
    match (magic, rest) {
        (_, []) => true,
        (Magic::Very, ['|' | ')', ..]) => true,
        (Magic::Very, _) => false,
        (_, ['\\', '|' | ')', ..]) => true,
        _ => false,
    }
}

/// What a backslash and the letter `c` stand for: `\s`, `\a`, `\x`...
fn vim_class(c: char) -> Option<&'static str> {
    Some(match c {
        's' => "[ \t]",
        'S' => "[^ \t]",
        'd' => "[0-9]",
        'D' => "[^0-9]",
        'w' => "[0-9A-Za-z_]",
        'W' => "[^0-9A-Za-z_]",
        'a' => "[A-Za-z]",
        'A' => "[^A-Za-z]",
        'l' => "[a-z]",
        'L' => "[^a-z]",
        'u' => "[A-Z]",
        'U' => "[^A-Z]",
        'x' => "[0-9A-Fa-f]",
        'X' => "[^0-9A-Fa-f]",
        'o' => "[0-7]",
        'O' => "[^0-7]",
        'h' => "[A-Za-z_]",
        'H' => "[^A-Za-z_]",
        'i' | 'k' => r"\w",
        'I' | 'K' => r"[^\W\d]",
        'f' => r"[\w/.\-+,#$%~=]",
        'F' => r"[[\w/.\-+,#$%~=]&&\D]",
        'p' => "[[:print:]]",
        'P' => r"[[:print:]&&\D]",
        'n' => r"\n",
        't' => r"\t",
        'r' => r"\r",
        'e' => r"\x1b",
        'b' => r"\x08",
        _ => return None,
    })
}

/// Where the `]` closing a `[` class is, in what follows the `[`.
fn class_end(cs: &[char]) -> Option<usize> {
    let mut j = usize::from(cs.first() == Some(&'^'));
    j += usize::from(cs.get(j) == Some(&']'));
    while j < cs.len() {
        match cs[j] {
            ']' => return Some(j),
            '\\' => j += 1,
            '[' if cs.get(j + 1) == Some(&':') => {
                j += cs[j..].windows(2).position(|w| w == [':', ']'])? + 1;
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// The inside of a Vim `[...]` class as a Rust one, brackets included.
fn vim_bracket(cs: &[char]) -> String {
    let mut out = String::from("[");
    let mut j = 0;
    if cs.first() == Some(&'^') {
        out.push('^');
        j = 1;
    }
    if cs.get(j) == Some(&']') {
        out += r"\]";
        j += 1;
    }
    while j < cs.len() {
        let c = cs[j];
        j += 1;
        match c {
            '\\' => match cs.get(j) {
                Some(&e @ ('e' | 'b' | 't' | 'r' | 'n')) => {
                    out += vim_class(e).unwrap();
                    j += 1;
                }
                Some(&e @ ('\\' | ']' | '^' | '-')) => {
                    out.push('\\');
                    out.push(e);
                    j += 1;
                }
                // Any other backslash is itself.
                _ => out += r"\\",
            },
            '[' if cs.get(j) == Some(&':') => {
                let end = j + cs[j..].windows(2).position(|w| w == [':', ']']).unwrap() + 2;
                out.push('[');
                out.extend(&cs[j..end]);
                j = end;
            }
            '[' | '&' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out + "]"
}

fn first_from(line: &str, re: &Regex, from: usize) -> Option<usize> {
    if from > line.len() {
        return None;
    }
    re.find_at(line, from).map(|m| m.start())
}

fn last_before(line: &str, re: &Regex, before: usize) -> Option<usize> {
    re.find_iter(line)
        .map(|m| m.start())
        .take_while(|&s| s < before)
        .last()
}

/// Next match start strictly after (forward) or before (backward) `from`.
pub fn find(
    buf: &Buffer,
    re: &Regex,
    from: (usize, usize),
    forward: bool,
    wrap: bool,
) -> Option<(usize, usize)> {
    let n = buf.len_lines();
    let (l0, b0) = from;
    if forward {
        let line = buf.line(l0);
        let start = if b0 < line.len() {
            next_grapheme(&line, b0)
        } else {
            line.len() + 1
        };
        if let Some(s) = first_from(&line, re, start) {
            return Some((l0, s));
        }
        for l in l0 + 1..n {
            if let Some(s) = first_from(&buf.line(l), re, 0) {
                return Some((l, s));
            }
        }
        if wrap {
            for l in 0..=l0 {
                let line = buf.line(l);
                if let Some(s) = first_from(&line, re, 0).filter(|&s| l < l0 || s <= b0) {
                    return Some((l, s));
                }
            }
        }
    } else {
        if let Some(s) = last_before(&buf.line(l0), re, b0) {
            return Some((l0, s));
        }
        for l in (0..l0).rev() {
            if let Some(s) = last_before(&buf.line(l), re, usize::MAX) {
                return Some((l, s));
            }
        }
        if wrap {
            for l in (l0..n).rev() {
                if let Some(s) =
                    last_before(&buf.line(l), re, usize::MAX).filter(|&s| l > l0 || s >= b0)
                {
                    return Some((l, s));
                }
            }
        }
    }
    None
}

/// Next line matching after (forward) or before `cur`, wrapping around to `cur` itself.
pub fn find_line(buf: &Buffer, re: &Regex, cur: usize, forward: bool) -> Option<usize> {
    let n = buf.len_lines();
    (1..=n)
        .map(|i| {
            if forward {
                (cur + i) % n
            } else {
                (cur + n * 2 - i) % n
            }
        })
        .find(|&l| re.is_match(&buf.line(l)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    #[test]
    fn smart_case() {
        assert!(compile("abc").unwrap().is_match("ABC"));
        assert!(!compile("Abc").unwrap().is_match("abc"));
        assert!(compile("(").is_err());
    }

    #[test]
    fn vim_patterns() {
        let m = |p: &str, s: &str| compile_vim(p, false).unwrap().is_match(s);
        // Bare ( | + { match themselves; with a backslash they group and count.
        assert!(m("a.or(|| b)", "x.a.or(|| b)"));
        assert!(m(r"^\(foo\|bar\)\+$", "barfoo") && !m(r"^\(foo\|bar\)\+$", "bar!"));
        assert!(m(r"x\{2,}", "xx") && !m(r"x\{2,}", "x") && m(r"^x\{-1,}$", "xxx"));
        assert!(m(r"\<is\>", "this is") && !m(r"\<is\>", "this"));
        assert!(m(r"\v^(a|b){2}$", "ab") && m(r"\Va.b", "a.b") && !m(r"\Va.b", "axb"));
        assert!(m("a^b$c", "a^b$c") && m("[[:digit:]x]", "7") && m(r"\%x41", "A"));
        assert!(m(r"\s\d\w", " 1a") && m("[]x]", "]") && m("a[", "a["));
        assert!(m(r"\cABC", "abc") && !compile_vim(r"\Cabc", true).unwrap().is_match("ABC"));
        assert!(compile_vim("abc", true).unwrap().is_match("ABC"));
        assert!(compile_vim(r"\Sbc", true).unwrap().is_match("aBC"));
        assert!(compile_vim(r"\(a\)\1", false).is_err());
    }

    #[test]
    fn find_forward_backward_wrap() {
        let b = Buffer::from_text("foo x\nbar\nfoo y");
        let re = compile("foo").unwrap();
        assert_eq!(find(&b, &re, (0, 0), true, true), Some((2, 0)));
        assert_eq!(find(&b, &re, (2, 0), true, true), Some((0, 0)));
        assert_eq!(find(&b, &re, (2, 0), true, false), None);
        assert_eq!(find(&b, &re, (2, 0), false, true), Some((0, 0)));
        assert_eq!(find(&b, &re, (0, 0), false, true), Some((2, 0)));
        assert_eq!(find(&b, &re, (0, 0), false, false), None);
    }

    #[test]
    fn find_line_wraps() {
        let b = Buffer::from_text("a\nb\na");
        let re = compile("a").unwrap();
        assert_eq!(find_line(&b, &re, 0, true), Some(2));
        assert_eq!(find_line(&b, &re, 2, true), Some(0));
        assert_eq!(find_line(&b, &re, 0, false), Some(2));
        assert_eq!(find_line(&b, &compile("b").unwrap(), 1, true), Some(1));
    }
}
