//! Context predicates (org-at-*-p) on the cursor line.

use super::line;
use super::syntax;
use crate::editor::Editor;

/// org-at-heading-p.
pub fn at_heading(ed: &Editor, l: usize) -> bool {
    syntax::level(&line(ed, l)).is_some()
}

/// org-at-table-p: an Org table line (`|...`).
pub fn at_table(ed: &Editor, l: usize) -> bool {
    line(ed, l).trim_start().starts_with('|') && !in_block(ed, l)
}

/// A table.el table line (`+---+`).
pub fn at_table_el(ed: &Editor, l: usize) -> bool {
    let t = line(ed, l);
    let t = t.trim();
    t.starts_with("+-") && t.ends_with('+') && !in_block(ed, l)
}

/// org-at-table.el-p or org table: any table.
pub fn at_any_table(ed: &Editor, l: usize) -> bool {
    at_table(ed, l) || at_table_el(ed, l)
}

/// The bullet of a list item line: (indent, bullet text incl. trailing space).
pub fn item_bullet(text: &str) -> Option<(usize, &str)> {
    let t = text.trim_start();
    let ind = text.len() - t.len();
    // `*` bullets only when indented (else a heading).
    let unordered =
        (t.starts_with("- ") || t.starts_with("+ ") || (ind > 0 && t.starts_with("* ")))
            || matches!(t, "-" | "+")
            || (ind > 0 && t == "*");
    if unordered {
        return Some((ind, &t[..t.len().min(2)]));
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    let alpha = super::options::bool("org-list-allow-alphabetical", false);
    let n = if digits > 0 {
        digits
    } else if alpha && t.len() > 1 && t.as_bytes()[0].is_ascii_alphabetic() {
        1
    } else {
        return None;
    };
    let term = t.as_bytes().get(n)?;
    if !matches!(term, b'.' | b')') {
        return None;
    }
    match t.as_bytes().get(n + 1) {
        Some(b' ') => Some((ind, &t[..n + 2])),
        None => Some((ind, &t[..n + 1])),
        _ => None,
    }
}

/// org-at-item-p.
pub fn at_item(ed: &Editor, l: usize) -> bool {
    item_bullet(&line(ed, l)).is_some() && !in_block(ed, l)
}

/// org-at-item-checkbox-p.
pub fn at_checkbox(ed: &Editor, l: usize) -> bool {
    let text = line(ed, l);
    item_bullet(&text).is_some_and(|(ind, b)| {
        let rest = &text[ind + b.len()..];
        let rest = rest
            .strip_prefix("[@")
            .map_or(rest, |r| r.find("] ").map_or(rest, |i| &r[i + 2..]));
        rest.starts_with("[ ]") || rest.starts_with("[X]") || rest.starts_with("[-]")
    })
}

/// Inside a `#+begin_…`/`#+end_…` block (not on its delimiters): its kind.
pub fn block_at(ed: &Editor, l: usize) -> Option<(String, usize, usize)> {
    for b in (0..l).rev() {
        let t = line(ed, b).trim_start().to_ascii_lowercase();
        if syntax::level(&line(ed, b)).is_some() {
            return None;
        }
        if t.starts_with("#+end_") {
            return None;
        }
        if let Some(rest) = t.strip_prefix("#+begin_") {
            let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            let close = format!("#+end_{name}");
            let end =
                (l..ed.line_count()).find(|&i| line(ed, i).trim().to_ascii_lowercase() == close)?;
            return Some((name, b, end));
        }
    }
    None
}

pub fn in_block(ed: &Editor, l: usize) -> bool {
    block_at(ed, l).is_some()
}

/// On a `#+begin_src` line, its contents or `#+end_src`: (begin, end) lines.
pub fn src_block(ed: &Editor, l: usize) -> Option<(usize, usize)> {
    let t = line(ed, l).trim_start().to_ascii_lowercase();
    if t.starts_with("#+begin_src") {
        let end = (l + 1..ed.line_count())
            .find(|&i| line(ed, i).trim().eq_ignore_ascii_case("#+end_src"))?;
        return Some((l, end));
    }
    if t.trim_end() == "#+end_src" {
        let b = (0..l).rev().find(|&i| {
            line(ed, i)
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("#+begin_src")
        })?;
        return Some((b, l));
    }
    block_at(ed, l)
        .filter(|(name, ..)| name == "src")
        .map(|(_, b, e)| (b, e))
}

/// org-at-keyword-p: `#+KEY: value`.
pub fn at_keyword(ed: &Editor, l: usize) -> bool {
    syntax::keyword_line(&line(ed, l)).is_some()
}

/// org-before-first-heading-p.
pub fn before_first_heading(ed: &Editor, l: usize) -> bool {
    syntax::heading_at_or_before(&ed.buf, l).is_none()
}

/// The planning line of the entry at heading `h`.
pub fn planning_line(ed: &Editor, h: usize) -> Option<usize> {
    let l = h + 1;
    (l < ed.line_count() && is_planning(&line(ed, l))).then_some(l)
}

pub fn is_planning(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("SCHEDULED:") || t.starts_with("DEADLINE:") || t.starts_with("CLOSED:")
}

/// org-at-planning-p.
pub fn at_planning(ed: &Editor, l: usize) -> bool {
    is_planning(&line(ed, l)) && l > 0 && (at_heading(ed, l - 1))
}

/// Property drawer of the entry at heading `h` (or the file's, before the
/// first heading): (`:PROPERTIES:` line, `:END:` line).
pub fn property_drawer(ed: &Editor, h: Option<usize>) -> Option<(usize, usize)> {
    let start = match h {
        Some(h) => {
            let mut l = h + 1;
            if l < ed.line_count() && is_planning(&line(ed, l)) {
                l += 1;
            }
            l
        }
        None => {
            // Before the first heading: after comments and blank lines.
            let mut l = 0;
            while l < ed.line_count() {
                let t = line(ed, l);
                let tt = t.trim();
                if tt.is_empty() || tt.starts_with("# ") || tt == "#" {
                    l += 1;
                } else {
                    break;
                }
            }
            l
        }
    };
    if start >= ed.line_count() || !line(ed, start).trim().eq_ignore_ascii_case(":PROPERTIES:") {
        return None;
    }
    let end = (start + 1..ed.line_count())
        .take_while(|&i| !at_heading(ed, i))
        .find(|&i| line(ed, i).trim().eq_ignore_ascii_case(":END:"))?;
    Some((start, end))
}

/// org-at-property-p: inside a property drawer, on a property line.
pub fn at_property(ed: &Editor, l: usize) -> bool {
    let h = syntax::heading_at_or_before(&ed.buf, l);
    property_drawer(ed, h).is_some_and(|(s, e)| l > s && l < e)
}

/// org-at-clock-log-p: a `CLOCK:` line.
pub fn at_clock_log(ed: &Editor, l: usize) -> bool {
    line(ed, l).trim_start().starts_with("CLOCK:")
}

/// A timestamp around byte `b` of line `l`: its byte range.
pub fn timestamp_at(ed: &Editor, l: usize, b: usize) -> Option<std::ops::Range<usize>> {
    let text = line(ed, l);
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if (rest.starts_with('<') || rest.starts_with('['))
            && let Some(len) = super::face::timestamp_len(rest)
        {
            if b >= i && b < i + len {
                return Some(i..i + len);
            }
            i += len;
            continue;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// org-at-timestamp-p.
pub fn at_timestamp(ed: &Editor) -> bool {
    timestamp_at(ed, ed.cur.line, ed.cur.byte).is_some()
}

/// A bracket link around byte `b`: its range.
pub fn link_at(ed: &Editor, l: usize, b: usize) -> Option<std::ops::Range<usize>> {
    let text = line(ed, l);
    let mut from = 0;
    while let Some(s) = text[from..].find("[[").map(|i| i + from) {
        let e = text[s..].find("]]").map(|i| s + i + 2)?;
        if b >= s && b < e {
            return Some(s..e);
        }
        from = e;
    }
    None
}

/// A footnote reference or definition label around byte `b`.
pub fn footnote_at(ed: &Editor, l: usize, b: usize) -> Option<std::ops::Range<usize>> {
    let text = line(ed, l);
    let mut from = 0;
    while let Some(s) = text[from..].find("[fn:").map(|i| i + from) {
        // Inline definitions nest brackets.
        let mut depth = 0;
        let mut end = None;
        for (j, c) in text[s..].char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(s + j + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let e = end?;
        if b >= s && b < e {
            return Some(s..e);
        }
        from = e;
    }
    None
}

/// A statistics cookie `[1/3]` or `[33%]` around byte `b`.
pub fn cookie_at(ed: &Editor, l: usize, b: usize) -> Option<std::ops::Range<usize>> {
    let text = line(ed, l);
    let s = text[..=b.min(text.len().saturating_sub(1))].rfind('[')?;
    let e = text[s..].find(']').map(|i| s + i + 1)?;
    let inner = &text[s + 1..e - 1];
    (b < e
        && !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_ascii_digit() || c == '/' || c == '%')
        && (inner.contains('/') || inner.ends_with('%')))
    .then_some(s..e)
}
