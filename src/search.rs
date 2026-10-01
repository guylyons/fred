//! Regex search over the buffer.

use crate::buffer::Buffer;
use crate::text::next_grapheme;
use regex::Regex;

/// Compile a pattern; case-insensitive unless it contains an uppercase letter.
pub fn compile(pat: &str) -> Result<Regex, String> {
    let src = if pat.chars().any(char::is_uppercase) { pat.to_string() } else { format!("(?i){pat}") };
    Regex::new(&src).map_err(|e| {
        let msg = e.to_string();
        let last = msg.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim();
        format!("bad pattern: {}", last.trim_start_matches("error: "))
    })
}

fn first_from(line: &str, re: &Regex, from: usize) -> Option<usize> {
    if from > line.len() {
        return None;
    }
    re.find_at(line, from).map(|m| m.start())
}

fn last_before(line: &str, re: &Regex, before: usize) -> Option<usize> {
    re.find_iter(line).map(|m| m.start()).take_while(|&s| s < before).last()
}

/// Next match start strictly after (forward) or before (backward) `from`.
pub fn find(buf: &Buffer, re: &Regex, from: (usize, usize), forward: bool, wrap: bool) -> Option<(usize, usize)> {
    let n = buf.len_lines();
    let (l0, b0) = from;
    if forward {
        let line = buf.line(l0);
        let start = if b0 < line.len() { next_grapheme(&line, b0) } else { line.len() + 1 };
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
                if let Some(s) = last_before(&buf.line(l), re, usize::MAX).filter(|&s| l > l0 || s >= b0) {
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
        .map(|i| if forward { (cur + i) % n } else { (cur + n * 2 - i) % n })
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
