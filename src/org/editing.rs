//! The remaining editing commands of org.el: RET, self-insert, sorting
//! entries, emphasis, indentation, fixed-width, filling, comments,
//! cloning subtrees, buffer-wide commands and the help/info commands.

use super::props::{self, Inherit};
use super::sexp::Sexp;
use super::syntax;
use super::{Prefix, call, ctx, fold, structure};
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};

/// The plain Fred Insert-mode key (newline, character).
fn vim_insert(ed: &mut Editor, k: Key) {
    crate::vim::insert_key(ed, k);
}

/// Column to indent a new line to in a list item (org-list-item-body-column).
fn item_body_column(ed: &Editor, l: usize) -> Option<usize> {
    let mut i = l;
    loop {
        let t = ed.buf.line(i);
        if syntax::level(&t).is_some() {
            return None;
        }
        if let Some((ind, b)) = ctx::item_bullet(&t) {
            return Some(ind + b.len());
        }
        if t.trim().is_empty() || i == 0 {
            return None;
        }
        if t.len() - t.trim_start().len() == 0 {
            return None;
        }
        i -= 1;
    }
}

/// org-return.
pub fn org_return(ed: &mut Editor, indent: bool) -> Result<(), String> {
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    let b = ed.cur.byte.min(line.len());
    if ctx::at_table(ed, l) && !ctx::in_block(ed, l) {
        if line[b..].trim().is_empty() || line[..b].trim().is_empty() {
            vim_insert(ed, Key::new(KeyCode::Enter));
            return Ok(());
        }
        return call(ed, "org-table-next-row", Prefix::None);
    }
    if super::options::bool("org-return-follows-link", false)
        && (super::links::link_at(ed).is_some_and(|lk| b < lk.range.end)
            || ctx::timestamp_at(ed, l, b).is_some())
    {
        return super::links::open_at_point(ed, Prefix::None);
    }
    let st = super::settings(ed);
    if b > 0
        && let Some(h) = syntax::headline(&line, &st)
    {
        // Split the title, keeping the tags on the heading.
        let in_title = b >= h.title.start && b <= h.title.end;
        let moved = if in_title {
            line[b..h.title.end].to_owned()
        } else {
            String::new()
        };
        let mut head = if in_title {
            format!("{}{}", &line[..b], &line[h.title.end..])
        } else {
            line.clone()
        };
        if in_title && !moved.is_empty() && h.tags_range.is_some() {
            let mut x = super::buf::EBuf::new(&head, 0);
            structure::align_tags_here(&mut x, structure::tags_column());
            head = x.s;
        }
        super::set_line(ed, l, &head);
        fold::show_entry(ed, l);
        super::insert_lines(ed, l + 1, &[moved.trim().to_owned()]);
        ed.set_cursor(l + 1, 0);
        if ed.mode == Mode::Insert {
            ed.cur.byte = 0;
        }
        return Ok(());
    }
    if b < line.len() && super::structure::in_item(ed, l) {
        let trailing = line[b..].to_owned();
        super::set_line(ed, l, &line[..b]);
        let col = if indent || true {
            item_body_column(ed, l).unwrap_or(0)
        } else {
            0
        };
        super::insert_lines(ed, l + 1, &[format!("{}{}", " ".repeat(col), trailing)]);
        ed.set_cursor(l + 1, col);
        return Ok(());
    }
    // A plain newline; inside a list item, indent to the item body (electric indent).
    let col = item_body_column(ed, l).filter(|_| super::structure::in_item(ed, l));
    let before = line[..b].to_owned();
    let after = line[b..].to_owned();
    match col {
        Some(c) if b == line.len() => {
            super::set_line(ed, l, before.trim_end());
            super::insert_lines(ed, l + 1, &[" ".repeat(c)]);
            ed.set_cursor(l + 1, c);
        }
        _ => {
            if indent {
                let c = indentation_for(ed, l + 1, true);
                super::set_line(ed, l, &before);
                super::insert_lines(
                    ed,
                    l + 1,
                    &[format!("{}{}", " ".repeat(c), after.trim_start())],
                );
                ed.set_cursor(l + 1, c);
            } else {
                super::set_line(ed, l, &before);
                super::insert_lines(ed, l + 1, &[after]);
                ed.set_cursor(l + 1, 0);
            }
        }
    }
    Ok(())
}

/// org--get-expected-indentation, simplified to lines: the column a line
/// at `l` should have.
pub fn indentation_for(ed: &Editor, l: usize, new_line: bool) -> usize {
    if l < ed.line_count() && !new_line && ctx::at_heading(ed, l) {
        return 0;
    }
    let adapt = super::sexp::option("org-adapt-indentation").map_or("nil".to_owned(), |v| {
        v.sym().map_or("t".into(), str::to_owned)
    });
    // Inside a list item: its body column.
    if l > 0
        && let Some(c) = item_body_column(ed, l - 1)
        && super::structure::in_item(ed, l - 1)
    {
        return c;
    }
    // Like the first non-blank line above, within the entry.
    let mut i = l;
    while i > 0 {
        i -= 1;
        let t = ed.buf.line(i);
        if syntax::level(&t).is_some() {
            return if adapt == "t" {
                syntax::level(&t).unwrap() + 1
            } else {
                0
            };
        }
        if !t.trim().is_empty() {
            return t.len() - t.trim_start().len();
        }
    }
    0
}

/// org-indent-line.
pub fn indent_line(ed: &mut Editor, l: usize) -> Result<(), String> {
    let line = ed.buf.line(l);
    if syntax::level(&line).is_some() {
        return Ok(());
    }
    if ctx::at_item(ed, l) {
        // Items are not indented (it could break the list structure).
        return Ok(());
    }
    if let Some((b, e)) = ctx::src_block(ed, l)
        && l > b
        && l < e
        && super::options::bool("org-src-tab-acts-natively", true)
    {
        // Keep the code's own indentation, but at least the block's.
        let head = ed.buf.line(b);
        let base = head.len() - head.trim_start().len()
            + if super::options::bool("org-src-preserve-indentation", false) {
                0
            } else {
                super::options::int("org-src-content-indentation", 2) as usize
            };
        let cur = line.len() - line.trim_start().len();
        if cur < base {
            super::set_line(ed, l, &format!("{}{}", " ".repeat(base), line.trim_start()));
        }
        return Ok(());
    }
    let col = indentation_for(ed, l, false);
    let t = line.trim_start();
    let new = format!("{}{t}", " ".repeat(col));
    if new != line {
        let off = ed.cur.byte.saturating_sub(line.len() - t.len());
        super::set_line(ed, l, &new);
        if ed.cur.line == l {
            ed.cur.byte = col + off;
        }
    }
    // Align a node property (org-property-format).
    if ctx::at_property(ed, l)
        && let Some((k, v)) = props::parse_property(&ed.buf.line(l))
    {
        let fmt = super::options::string("org-property-format", "%-10s %s");
        let key = format!(":{k}:");
        let w: usize = fmt
            .trim_start_matches("%-")
            .split('s')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(10);
        let aligned = if v.is_empty() {
            format!("{}{key}", " ".repeat(col))
        } else {
            format!("{}{key:<w$} {v}", " ".repeat(col))
        };
        super::set_line(ed, l, &aligned);
    }
    Ok(())
}

/// org-sort-entries.
pub fn sort_entries(
    ed: &mut Editor,
    with_case: bool,
    kind: char,
    property: Option<String>,
) -> Result<(), String> {
    let st = super::settings(ed);
    let l = ed.cur.line;
    let (start, end, what) = if let Some((lo, hi)) = ed.org_region {
        let s = (lo..=hi)
            .find(|&i| ctx::at_heading(ed, i))
            .or_else(|| syntax::next_heading(&ed.buf, lo, usize::MAX))
            .ok_or("Nothing to sort")?;
        let e = fold::back_to_heading(ed, hi)
            .map_or(ed.line_count(), |h| syntax::subtree_end(&ed.buf, h));
        (s, e, "region")
    } else if let Some(h) = fold::back_to_heading(ed, l) {
        fold::show_subtree(ed, h);
        let first = syntax::next_heading(&ed.buf, h, usize::MAX)
            .filter(|&x| x < syntax::subtree_end(&ed.buf, h))
            .ok_or("Nothing to sort")?;
        (first, syntax::subtree_end(&ed.buf, h), "children")
    } else {
        let s = (0..ed.line_count())
            .find(|&i| ctx::at_heading(ed, i))
            .ok_or("Nothing to sort")?;
        fold::show_all(
            ed,
            &[fold::Spec::Outline, fold::Spec::Drawer, fold::Spec::Block],
        );
        (s, ed.line_count(), "top-level")
    };
    if start >= end {
        return Err("Nothing to sort".into());
    }
    let stars = syntax::level(&ed.buf.line(start)).unwrap();
    if stars > 1 && (start..end).any(|i| syntax::level(&ed.buf.line(i)).is_some_and(|n| n < stars))
    {
        return Err("Region to sort contains a level above the first entry".into());
    }
    let _ = what;
    // Records: each heading of level `stars` and its subtree.
    let mut records: Vec<(usize, usize)> = vec![];
    let mut i = start;
    while i < end {
        if syntax::level(&ed.buf.line(i)) == Some(stars) {
            let e = syntax::subtree_end(&ed.buf, i).min(end);
            records.push((i, e));
            i = e;
        } else {
            i += 1;
        }
    }
    let now = super::now() as f64;
    let dk = kind.to_ascii_lowercase();
    let reverse = kind.is_ascii_uppercase();
    #[derive(PartialEq, PartialOrd)]
    enum K {
        N(f64),
        S(String),
    }
    let key = |ed: &Editor, (b, e): (usize, usize)| -> K {
        let line = ed.buf.line(b);
        let hl = syntax::headline(&line, &st);
        let title = super::links::display_format(hl.as_ref().map_or("", |h| h.title(&line)));
        let entry_end = syntax::entry_end(&ed.buf, b).min(e);
        let find_ts = |re: &regex::Regex| {
            (b..entry_end).find_map(|i| re.find(&ed.buf.line(i)).map(|m| m.as_str().to_owned()))
        };
        let secs = |s: Option<String>| {
            s.and_then(|s| super::time::stamp::time_string_to_seconds(&s).ok())
                .map_or(now, |x| x as f64)
        };
        match dk {
            'n' => K::N(
                title
                    .trim()
                    .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0.0),
            ),
            'a' => K::S(if with_case {
                title
            } else {
                title.to_lowercase()
            }),
            'k' => K::N(super::clock_minutes(ed, b) as f64),
            't' => K::N(secs(
                find_ts(crate::org_re!(r"<\d{4}-\d\d-\d\d[^>\n]*>"))
                    .or_else(|| find_ts(crate::org_re!(r"[\[<]\d{4}-\d\d-\d\d[^>\]\n]*[>\]]"))),
            )),
            'c' => K::N(secs((b..entry_end).find_map(|i| {
                let t = ed.buf.line(i);
                let tt = t.trim_start();
                (tt.starts_with('[') && super::face::timestamp_len(tt).is_some())
                    .then(|| tt[..super::face::timestamp_len(tt).unwrap()].to_owned())
            }))),
            's' => K::N(secs(props::get(ed, Some(b), "SCHEDULED", Inherit::No))),
            'd' => K::N(secs(props::get(ed, Some(b), "DEADLINE", Inherit::No))),
            'p' => K::N(
                hl.as_ref()
                    .and_then(|h| h.priority)
                    .unwrap_or(st.priorities.2) as f64,
            ),
            'r' => K::S(
                property
                    .as_deref()
                    .and_then(|p| props::get(ed, Some(b), p, Inherit::No))
                    .unwrap_or_default(),
            ),
            'o' => {
                let kw = hl.as_ref().and_then(|h| h.todo.clone());
                let all = st.todo_names();
                let n = kw
                    .as_deref()
                    .and_then(|k| all.iter().position(|x| *x == k))
                    .map_or(0, |p| all.len() - p) as f64;
                let done = kw.as_deref().is_some_and(|k| st.is_done(k));
                K::N(if kw.is_none() {
                    99.0
                } else if done {
                    99.0 + n
                } else {
                    99.0 - n
                })
            }
            _ => K::S(String::new()),
        }
    };
    let mut keyed: Vec<(K, Vec<String>)> = records
        .iter()
        .map(|&r| (key(ed, r), super::lines(ed, r.0..r.1)))
        .collect();
    // sort-subr is stable; reverse means reversed order of keys.
    keyed.sort_by(|a, b| {
        let o = a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal);
        if reverse { o.reverse() } else { o }
    });
    // Each record ends with a newline: make sure the last one keeps its blank lines.
    let mut out: Vec<String> = vec![];
    for (_, lines) in keyed {
        out.extend(lines);
    }
    let first = records[0].0;
    let last = records.last().unwrap().1;
    super::splice(ed, first, last - first, &out);
    fold::hide_drawers(ed, 0, ed.line_count());
    ed.set_cursor(
        start.saturating_sub(if what == "children" { 1 } else { 0 }),
        0,
    );
    ed.set_msg("Sorting entries...done");
    Ok(())
}

/// org-sort: entries, items or table lines.
fn sort(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let l = ed.cur.line;
    if ctx::at_table(ed, l) {
        return call(ed, "org-table-sort-lines", arg);
    }
    if super::structure::in_item(ed, l) && !ctx::at_heading(ed, l) {
        return call(ed, "org-sort-list", arg);
    }
    sort_command(ed, !arg.is_none());
    Ok(())
}

fn sort_command(ed: &mut Editor, with_case: bool) {
    let entries = vec![
        (String::new(), "Sort: [a]lpha [n]umeric [p]riority p[r]operty todo[o]rder [f]unc".to_owned()),
        (String::new(), "      [t]ime [s]cheduled [d]eadline [c]reated cloc[k]ing   A/N/P/R/O/F/T/S/D/C/K reverse".to_owned()),
    ]
    .into_iter()
    .chain("anprofktsdcANPROFKTSDC".chars().map(|c| (c.to_string(), String::new())))
    .collect();
    super::menu(ed, "Sort children:", entries, move |ed, k| {
        let c = k.chars().next().unwrap_or('a');
        let go = move |ed: &mut Editor, prop: Option<String>| {
            ed.undo.begin(ed.cur.pos());
            if let Err(e) = sort_entries(ed, with_case, c, prop) {
                ed.set_err(e);
            }
            ed.undo.end(ed.cur.pos());
        };
        if c.eq_ignore_ascii_case(&'f') {
            ed.set_err("Missing key extractor");
        } else if c.eq_ignore_ascii_case(&'r') {
            let keys = props::buffer_keys(ed, true, false, false);
            super::complete(ed, "Property: ", keys, true, move |ed, p| go(ed, Some(p)));
        } else {
            go(ed, None);
        }
    });
}

/// org-emphasize with marker `c` (space removes).
fn emphasize(ed: &mut Editor, c: char) -> Result<(), String> {
    let alist: Vec<String> = super::sexp::option("org-emphasis-alist")
        .and_then(|v| {
            v.list().map(|l| {
                l.iter()
                    .filter_map(|e| e.car().and_then(Sexp::str).map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_else(|| {
            ["*", "/", "_", "=", "~", "+"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    let s = if c == ' ' {
        String::new()
    } else if alist.iter().any(|m| *m == c.to_string()) {
        c.to_string()
    } else {
        return Err(format!("No such emphasis marker: \"{c}\""));
    };
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    let (b0, e0, mut string, mv) = match ed.org_region {
        Some((lo, hi)) if lo == hi => (0, line.len(), line.trim().to_owned(), false),
        Some(_) => {
            return Err("Emphasis over several lines is not supported in Line selections".into());
        }
        None => {
            let b = if ed.mode == Mode::Insert {
                ed.cur.byte
            } else {
                crate::text::next_grapheme(&line, ed.cur.byte).min(line.len())
            };
            (b, b, String::new(), true)
        }
    };
    let (b0, e0) = if ed.org_region.is_some() {
        let lead = line.len() - line.trim_start().len();
        (lead, line.trim_end().len())
    } else {
        (b0, e0)
    };
    while string.chars().count() > 1
        && string.chars().next() == string.chars().last()
        && alist.iter().any(|m| string.starts_with(m.as_str()))
    {
        string = string[1..string.len() - 1].to_owned();
    }
    let wrapped = format!("{s}{string}{s}");
    let mut before = line[..b0].to_owned();
    let mut after = line[e0..].to_owned();
    if !before.is_empty()
        && !before.ends_with(|ch: char| ch.is_whitespace() || "-('\"{".contains(ch))
    {
        before.push(' ');
    }
    let pad_after = !after.is_empty()
        && !after.starts_with(|ch: char| ch.is_whitespace() || "-.,:!?;'\")}\\[".contains(ch));
    if pad_after {
        after.insert(0, ' ');
    }
    let cursor = before.len()
        + if mv && !s.is_empty() {
            s.len() + string.len()
        } else {
            wrapped.len()
        };
    super::set_line(ed, l, &format!("{before}{wrapped}{after}"));
    ed.set_cursor(l, cursor);
    if mv && ed.mode != Mode::Insert && !s.is_empty() {
        ed.mode = Mode::Insert;
        ed.cur.byte = cursor;
    }
    Ok(())
}

/// org-toggle-fixed-width.
fn toggle_fixed_width(ed: &mut Editor) -> Result<(), String> {
    let fixed = |t: &str| {
        let tt = t.trim_start();
        tt.starts_with(": ") || tt == ":"
    };
    let strip = |t: &str| -> String {
        let ind = t.len() - t.trim_start().len();
        let tt = t.trim_start();
        let rest = tt
            .strip_prefix(": ")
            .or_else(|| tt.strip_prefix(':'))
            .unwrap_or(tt);
        format!("{}{rest}", &t[..ind])
    };
    match ed.org_region {
        None => {
            let l = ed.cur.line;
            let t = ed.buf.line(l);
            if fixed(&t) {
                super::set_line(ed, l, &strip(&t));
            } else if t.trim().is_empty() {
                let c = indentation_for(ed, l, false);
                super::set_line(ed, l, &format!("{}: ", " ".repeat(c)));
                ed.set_cursor(l, c + 2);
            } else if ctx::at_item(ed, l) || ctx::at_table(ed, l) || ctx::in_block(ed, l) {
                return Err("Cannot insert a fixed-width line here".into());
            } else {
                let ind = t.len() - t.trim_start().len();
                super::set_line(ed, l, &format!("{}: {}", &t[..ind], t.trim_start()));
            }
        }
        Some((lo, hi)) => {
            let mut hi = hi;
            while hi > lo && ed.buf.line(hi).trim().is_empty() {
                hi -= 1;
            }
            let all = (lo..=hi)
                .filter(|&i| !ed.buf.line(i).trim().is_empty())
                .all(|i| fixed(&ed.buf.line(i)));
            let min_ind = (lo..=hi)
                .map(|i| ed.buf.line(i))
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.len() - t.trim_start().len())
                .min()
                .unwrap_or(0);
            let out: Vec<String> = (lo..=hi)
                .map(|i| {
                    let t = ed.buf.line(i);
                    if all {
                        strip(&t)
                    } else if fixed(&t) {
                        t
                    } else if t.trim().is_empty() {
                        format!("{}:", " ".repeat(min_ind))
                    } else {
                        let pad = format!("{t:min_ind$}");
                        format!("{}: {}", &pad[..min_ind], &pad[min_ind..])
                    }
                })
                .collect();
            super::splice(ed, lo, hi - lo + 1, &out);
        }
    }
    Ok(())
}

/// org-fill-paragraph: refill the paragraph (or item, or comment) at point.
fn fill_paragraph(ed: &mut Editor, justify: bool) -> Result<(), String> {
    let _ = justify;
    let width = super::options::int("fill-column", 70).max(10) as usize;
    let l = ed.cur.line;
    if ctx::at_heading(ed, l)
        || ctx::at_table(ed, l)
        || ctx::in_block(ed, l)
        || ctx::at_keyword(ed, l)
    {
        return Ok(());
    }
    let para_line = |t: &str| {
        !t.trim().is_empty()
            && syntax::level(t).is_none()
            && !t.trim_start().starts_with('|')
            && syntax::keyword_line(t).is_none()
    };
    if !para_line(&ed.buf.line(l)) {
        return Ok(());
    }
    let comment = ed.buf.line(l).trim_start().starts_with("# ");
    let mut s = l;
    while s > 0
        && para_line(&ed.buf.line(s - 1))
        && ed.buf.line(s - 1).trim_start().starts_with("# ") == comment
        && !(ctx::at_item(ed, s) && !comment)
    {
        s -= 1;
    }
    let mut e = l;
    while e + 1 < ed.line_count()
        && para_line(&ed.buf.line(e + 1))
        && ed.buf.line(e + 1).trim_start().starts_with("# ") == comment
        && !ctx::at_item(ed, e + 1)
    {
        e += 1;
    }
    let first = ed.buf.line(s);
    let (prefix_first, prefix_rest) = if comment {
        let ind = first.len() - first.trim_start().len();
        (
            format!("{}# ", &first[..ind]),
            format!("{}# ", &first[..ind]),
        )
    } else if let Some((ind, b)) = ctx::item_bullet(&first) {
        (first[..ind + b.len()].to_owned(), " ".repeat(ind + b.len()))
    } else {
        let ind = first.len() - first.trim_start().len();
        (" ".repeat(ind), " ".repeat(ind))
    };
    let words: Vec<String> = (s..=e)
        .flat_map(|i| {
            let t = ed.buf.line(i);
            let body = if i == s {
                t[prefix_first.len().min(t.len())..].to_owned()
            } else if comment {
                t.trim_start()
                    .trim_start_matches('#')
                    .trim_start()
                    .to_owned()
            } else {
                t.trim_start().to_owned()
            };
            body.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut out: Vec<String> = vec![];
    let mut cur = prefix_first.clone();
    let mut empty = true;
    for w in words {
        if !empty && unicode_width::UnicodeWidthStr::width(format!("{cur} {w}").as_str()) > width {
            out.push(cur);
            cur = prefix_rest.clone();
            empty = true;
        }
        if !empty {
            cur.push(' ');
        }
        cur.push_str(&w);
        empty = false;
    }
    out.push(cur);
    if out != super::lines(ed, s..e + 1) {
        super::splice(ed, s, e - s + 1, &out);
    }
    ed.set_cursor(s, 0);
    Ok(())
}

/// org-comment-dwim: comment or uncomment lines (`# `), or the heading.
fn comment_dwim(ed: &mut Editor) -> Result<(), String> {
    let l = ed.cur.line;
    if ctx::at_heading(ed, l) && ed.org_region.is_none() {
        return call(ed, "org-toggle-comment", Prefix::None);
    }
    let (lo, hi) = ed.org_region.unwrap_or((l, l));
    let lines = super::lines(ed, lo..hi + 1);
    let commented = lines
        .iter()
        .filter(|t| !t.trim().is_empty())
        .all(|t| t.trim_start().starts_with("# ") || t.trim() == "#");
    let min = lines
        .iter()
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.len() - t.trim_start().len())
        .min()
        .unwrap_or(0);
    let out: Vec<String> = lines
        .iter()
        .map(|t| {
            if commented {
                let ind = t.len() - t.trim_start().len();
                let tt = t.trim_start();
                format!(
                    "{}{}",
                    &t[..ind],
                    tt.strip_prefix("# ")
                        .or_else(|| tt.strip_prefix('#'))
                        .unwrap_or(tt)
                )
            } else if t.trim().is_empty() && lines.len() > 1 {
                t.clone()
            } else {
                let pad = format!("{t:min$}");
                format!("{}# {}", &pad[..min], &pad[min..])
            }
        })
        .collect();
    super::splice(ed, lo, hi - lo + 1, &out);
    Ok(())
}

/// org-clone-subtree-with-time-shift.
fn clone_subtree(ed: &mut Editor, n: usize, shift: &str) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("No subtree to clone")?;
    let end = syntax::subtree_end(&ed.buf, h);
    let template = super::lines(ed, h..end);
    let shift = shift.trim();
    let doshift = if shift.is_empty() {
        None
    } else {
        let re = regex::Regex::new(r"^([+-]?[0-9]+)([hdwmy])$").unwrap();
        let c = re
            .captures(shift)
            .ok_or_else(|| format!("Invalid shift specification {shift}"))?;
        Some((c[1].parse::<i64>().unwrap(), c[2].chars().next().unwrap()))
    };
    let repeat = template
        .iter()
        .any(|l| super::time::stamp::repeat_re().is_match(l));
    let n_no_remove = if repeat && doshift.is_some() {
        n.saturating_sub(1)
    } else {
        n
    };
    let _ = n_no_remove;
    let mut out: Vec<String> = vec![];
    let total = if repeat && doshift.is_some() {
        n + 1
    } else {
        n
    };
    for i in 1..=total {
        let mut lines: Vec<String> = template.clone();
        // Remove ID properties and CLOCK lines in clones.
        lines.retain(|l| {
            !(l.trim_start().starts_with(":ID:") || l.trim_start().starts_with("CLOCK:"))
        });
        if repeat && doshift.is_some() && i < total || repeat && doshift.is_none() {
            for l in lines.iter_mut() {
                *l = super::time::stamp::repeat_re()
                    .replace_all(l, |c: &regex::Captures| {
                        c[0].replacen(&c[1], "", 1)
                            .replace("  ", " ")
                            .replace(" >", ">")
                            .replace(" ]", "]")
                    })
                    .into_owned();
            }
        }
        if let Some((k, unit)) = doshift {
            let delta = k * i as i64;
            let mut shifted = vec![];
            for l in lines {
                shifted.push(shift_timestamps(&l, delta, unit));
            }
            lines = shifted;
        }
        if lines
            .iter()
            .any(|l| l.trim_start().starts_with(":PROPERTIES:"))
        {
            // An emptied drawer goes away.
            let mut k = 0;
            while k + 1 < lines.len() {
                if lines[k].trim() == ":PROPERTIES:" && lines[k + 1].trim() == ":END:" {
                    lines.drain(k..k + 2);
                } else {
                    k += 1;
                }
            }
        }
        if i == total && repeat && doshift.is_some() {
            // The original, with repeater, after the clones and shifted past them.
            lines = template
                .iter()
                .map(|l| shift_timestamps(l, doshift.unwrap().0 * total as i64, doshift.unwrap().1))
                .collect();
        }
        out.extend(lines);
    }
    if repeat && doshift.is_some() {
        // Replace the original by clones + shifted original.
        super::splice(ed, h, end - h, &out);
    } else {
        super::insert_lines(ed, end, &out);
    }
    Ok(())
}

/// Shift every timestamp in `line` by `n` units (h d w m y).
fn shift_timestamps(line: &str, n: i64, unit: char) -> String {
    let re = super::time::stamp::ts_both();
    let mut out = String::new();
    let mut last = 0;
    for m in re.find_iter(line) {
        out.push_str(&line[last..m.start()]);
        let ts = m.as_str();
        use super::time::civil::Unit;
        let u = match unit {
            'h' => Unit::Hour,
            'w' => Unit::Week,
            'm' => Unit::Month,
            'y' => Unit::Year,
            _ => Unit::Day,
        };
        let r = super::time::stamp::change(
            ts,
            1,
            super::time::stamp::Part::Day,
            n,
            Some(super::time::stamp::What::Unit(u)),
            Default::default(),
        );
        out.push_str(&r.map(|c| c.text).unwrap_or_else(|_| ts.to_owned()));
        last = m.end();
    }
    out.push_str(&line[last..]);
    out
}

/// org-delete-indentation (join with the previous line; into a heading title).
fn delete_indentation(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let l = if arg.is_none() {
        ed.cur.line
    } else {
        ed.cur.line + 1
    };
    if l == 0 || l >= ed.line_count() {
        return Ok(());
    }
    let prev = ed.buf.line(l - 1);
    let cur = ed.buf.line(l);
    let st = super::settings(ed);
    let joined = match syntax::headline(&prev, &st).and_then(|h| h.tags_range) {
        Some(r) => {
            let tags = prev[r.clone()].trim().to_owned();
            let base = prev[..r.start].trim_end().to_owned();
            let mut x = super::buf::EBuf::new(&format!("{base} {} {tags}", cur.trim()), 0);
            structure::align_tags_here(&mut x, structure::tags_column());
            x.s
        }
        None => {
            if cur.trim().is_empty() {
                prev.trim_end().to_owned()
            } else {
                format!("{} {}", prev.trim_end(), cur.trim_start())
            }
        }
    };
    let at = prev.trim_end().len();
    super::splice(ed, l - 1, 2, &[joined]);
    ed.set_cursor(l - 1, at);
    Ok(())
}

/// org-occur-in-agenda-files: matches across the agenda files in a picker.
fn occur_in_agenda_files(ed: &mut Editor, link: bool) {
    let then = move |ed: &mut Editor, re: String| {
        super::effect(ed, move |s| {
            let Ok(r) = super::re::compile(&re, super::re::smart_fold(&re)) else {
                return s.ed.set_err(format!("Invalid regexp {re}"));
            };
            let mut rows: Vec<String> = vec![];
            let mut locs: Vec<(std::path::PathBuf, usize)> = vec![];
            let mut files = super::agenda_files();
            for x in super::options::strings("org-agenda-text-search-extra-files", &[]) {
                if x != "agenda-archives" {
                    files.push(super::options::expand(&x));
                }
            }
            for f in files {
                let Ok(t) = s.org_text(&f) else { continue };
                for (i, l) in t.lines().enumerate() {
                    if r.is_match(l) {
                        rows.push(format!(
                            "{}:{}: {l}",
                            f.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            i + 1
                        ));
                        locs.push((f.clone(), i));
                    }
                }
            }
            if rows.is_empty() {
                return s.ed.set_msg(format!("No match for {re}"));
            }
            super::complete(
                &mut s.ed,
                &format!("Occur {re}: "),
                rows.clone(),
                true,
                move |ed, choice| {
                    if let Some(i) = rows.iter().position(|x| *x == choice) {
                        let (f, l) = locs[i].clone();
                        super::effect(ed, move |s| {
                            let _ = s.org_visit(&f, l);
                        });
                    }
                },
            );
        });
    };
    if link {
        let lk = super::links::stored_links()
            .first()
            .map(|(l, _)| l.clone())
            .unwrap_or_default();
        then(ed, regex::escape(&lk));
    } else {
        super::read(ed, "Regexp: ", "", then);
    }
}

/// Open an external URL with the system opener.
fn browse(url: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let l = ed.cur.line;
    Some(match name {
        "org-return" => org_return(ed, false),
        "org-return-and-maybe-indent" | "org-return-indent" | "org-newline-and-indent" => {
            org_return(ed, !super::options::bool("electric-indent-mode", true))
        }
        "org-force-self-insert" | "org-self-insert-command" => {
            let c = super::take_call_arg()
                .and_then(|s| s.chars().next())
                .unwrap_or('|');
            if c != '|' || !super::table_self_insert(ed, c) {
                vim_insert(ed, Key::ch(c));
            }
            Ok(())
        }
        "org-delete-char" => {
            vim_insert(ed, Key::new(KeyCode::Delete));
            Ok(())
        }
        "org-delete-backward-char" => {
            vim_insert(ed, Key::new(KeyCode::Backspace));
            Ok(())
        }
        "org-open-line" => {
            if ctx::at_table(ed, l) {
                call(ed, "org-table-insert-row", Prefix::None)
            } else {
                let line = ed.buf.line(l);
                let b = ed.cur.byte.min(line.len());
                super::splice(ed, l, 1, &[line[..b].to_owned(), line[b..].to_owned()]);
                ed.set_cursor(l, b);
                Ok(())
            }
        }
        "org-kill-line" => {
            let line = ed.buf.line(l);
            let b = ed.cur.byte.min(line.len());
            let st = super::settings(ed);
            let hl = syntax::headline(&line, &st);
            let special = super::options::bool("org-special-ctrl-k", false);
            if special && let Some(h) = hl.filter(|h| h.tags_range.is_some() && b < h.title.end) {
                let killed = line[b..h.title.end].to_owned();
                super::set_line(ed, l, &format!("{}{}", &line[..b], &line[h.title.end..]));
                tags_realign(ed, l);
                structure::kill_new(ed, &killed);
            } else if b >= line.len() && l + 1 < ed.line_count() {
                let next = ed.buf.line(l + 1);
                super::splice(ed, l, 2, &[format!("{line}{next}")]);
                structure::kill_new(ed, "\n");
            } else {
                structure::kill_new(ed, &line[b..]);
                super::set_line(ed, l, &line[..b]);
            }
            Ok(())
        }
        "org-yank" => {
            let text = structure::current_kill(ed);
            if structure::kill_is_subtree(&text)
                && super::options::bool("org-yank-adjusted-subtrees", false)
            {
                structure::paste_subtree(ed, Prefix::None, Some(text), true)
            } else {
                crate::vim::ops::put(ed, 1, false);
                Ok(())
            }
        }
        "org-beginning-of-line" => {
            let line = ed.buf.line(l);
            let st = super::settings(ed);
            let special = super::options::bool("org-special-ctrl-a/e", false);
            let b = match syntax::headline(&line, &st) {
                Some(h) if special && ed.cur.byte != h.title.start => h.title.start,
                _ => match ctx::item_bullet(&line) {
                    Some((ind, b)) if special => ind + b.len(),
                    _ => 0,
                },
            };
            ed.set_cursor(l, b);
            Ok(())
        }
        "org-end-of-line" => {
            let line = ed.buf.line(l);
            let st = super::settings(ed);
            let special = super::options::bool("org-special-ctrl-a/e", false);
            let b = match syntax::headline(&line, &st) {
                Some(h) if special && h.tags_range.is_some() && ed.cur.byte < h.title.end => {
                    h.title.end
                }
                _ => line.len(),
            };
            ed.set_cursor(l, b.saturating_sub(usize::from(ed.mode != Mode::Insert)));
            Ok(())
        }
        "org-emphasize" => {
            let marks: Vec<(String, String)> = ["*", "/", "_", "=", "~", "+", " "]
                .iter()
                .map(|m| {
                    (
                        m.to_string(),
                        match *m {
                            "*" => "bold",
                            "/" => "italic",
                            "_" => "underline",
                            "=" => "verbatim",
                            "~" => "code",
                            "+" => "strike-through",
                            _ => "remove",
                        }
                        .to_owned(),
                    )
                })
                .collect();
            let region = ed.org_region;
            super::menu(ed, "Emphasis marker or tag:", marks, move |ed, k| {
                ed.org_region = region;
                ed.undo.begin(ed.cur.pos());
                if let Err(e) = emphasize(ed, k.chars().next().unwrap_or(' ')) {
                    ed.set_err(e);
                }
                ed.org_region = None;
                ed.undo.end(ed.cur.pos());
            });
            Ok(())
        }
        "org-sort" => sort(ed, arg),
        "org-sort-entries" => {
            sort_command(ed, !arg.is_none());
            Ok(())
        }
        "org-edit-headline" => {
            let h = fold::back_to_heading(ed, l).ok_or_else(|| "Before first headline".to_owned());
            h.map(|h| {
                let st = super::settings(ed);
                let line = ed.buf.line(h);
                let title = syntax::headline(&line, &st)
                    .map(|x| x.title(&line).to_owned())
                    .unwrap_or_default();
                super::read(ed, "Edit: ", &title, move |ed, new| {
                    let st = super::settings(ed);
                    let line = ed.buf.line(h);
                    if let Some(x) = syntax::headline(&line, &st) {
                        ed.undo.begin(ed.cur.pos());
                        super::set_line(
                            ed,
                            h,
                            &format!(
                                "{}{}{}",
                                &line[..x.title.start],
                                new.trim(),
                                &line[x.title.end..]
                            ),
                        );
                        tags_realign(ed, h);
                        ed.undo.end(ed.cur.pos());
                    }
                });
            })
        }
        "org-insert-heading-after-current" => structure::insert_heading(ed, Prefix::U(1), None),
        "org-display-outline-path" => {
            let st = super::settings(ed);
            let h = fold::back_to_heading(ed, l);
            let mut path = h
                .map(|h| super::refile::outline_path(&ed.buf, &st, h, !arg.is_none()))
                .unwrap_or_default();
            if (!arg.is_none() || super::options::bool("org-outline-path-include-file", false))
                && let Some(f) = ed.path.as_ref().and_then(|p| p.file_name())
            {
                path.insert(0, f.to_string_lossy().into_owned());
            }
            ed.set_msg(path.join("/"));
            Ok(())
        }
        "org-remove-occur-highlights" => {
            if let Some(o) = &mut ed.org {
                o.sparse_hits.clear();
            }
            Ok(())
        }
        "org-show-priority" => call(ed, "org-priority-show", arg),
        "org-find-entry-with-id" => {
            super::read(ed, "ID: ", "", |ed, id| {
                if let Err(e) = super::links::open_id(ed, &id) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        "org-save-all-org-buffers" => {
            super::effect(ed, |s| {
                let n = s.org_buffer_count();
                for i in 0..n {
                    let org = s.org_with_buffer(i, |e| {
                        e.org.is_some() && e.buf.modified && e.path.is_some()
                    });
                    if org {
                        let _ = s.org_save(i);
                    }
                }
                s.ed.set_msg("Saving all Org buffers... done");
            });
            Ok(())
        }
        "org-revert-all-org-buffers" => {
            super::yes_or_no(ed, "Revert all Org buffers from their files? ", |ed| {
                super::effect(ed, |s| {
                    let n = s.org_buffer_count();
                    let idx: Vec<usize> = (0..n)
                        .filter(|&i| s.org_with_buffer(i, |e| e.org.is_some()))
                        .collect();
                    let paths: Vec<std::path::PathBuf> = idx
                        .into_iter()
                        .filter_map(|i| s.org_buffer_path(i))
                        .collect();
                    for p in paths {
                        if let Ok(text) = std::fs::read_to_string(&p) {
                            let _ = s.org_with_file(&p, |e| {
                                let lines: Vec<String> = text.lines().map(str::to_owned).collect();
                                let n = e.line_count();
                                e.undo.begin(e.cur.pos());
                                super::splice(e, 0, n, &lines);
                                e.undo.end(e.cur.pos());
                                e.mark_saved();
                            });
                        }
                    }
                    s.ed.set_msg("All Org buffers reverted");
                });
            });
            Ok(())
        }
        "org-switchb" | "org-iswitchb" => {
            super::effect(ed, |s| {
                let n = s.org_buffer_count();
                let idx: Vec<usize> = (0..n)
                    .filter(|&i| s.org_with_buffer(i, |e| e.org.is_some()))
                    .collect();
                let list: Vec<(String, std::path::PathBuf)> = idx
                    .into_iter()
                    .filter_map(|i| s.org_buffer_path(i).map(|p| (p.display().to_string(), p)))
                    .collect();
                let names: Vec<String> = list.iter().map(|x| x.0.clone()).collect();
                super::complete(&mut s.ed, "Org buffer: ", names, true, move |ed, n| {
                    if let Some((_, p)) = list.into_iter().find(|x| x.0 == n) {
                        super::effect(ed, move |s| {
                            let _ = s.org_visit(&p, 0);
                        });
                    }
                });
            });
            Ok(())
        }
        "org-edit-agenda-file-list" => {
            ed.pending_effect = Some(crate::ex::ExEffect::Open {
                path: crate::config::config_path(),
                line: 0,
                col: 0,
                pattern: Some("org-agenda-files".into()),
            });
            Ok(())
        }
        "org-customize" | "org-create-customize-menu" => {
            ed.pending_effect = Some(crate::ex::ExEffect::Open {
                path: crate::config::config_path(),
                line: 0,
                col: 0,
                pattern: Some("\\[org\\]".into()),
            });
            Ok(())
        }
        "org-mode-restart" | "org-reload" => {
            if let Some(o) = &mut ed.org {
                o.reset_settings();
            }
            fold::startup(ed);
            ed.set_msg("org-mode restarted");
            Ok(())
        }
        "org-version" => {
            ed.set_msg(format!(
                "Org mode version 9.8-pre (fred port of 3b73b8a0), fred {}",
                env!("CARGO_PKG_VERSION")
            ));
            Ok(())
        }
        "org-info" | "org-info-find-node" => {
            ed.pending_effect = Some(crate::ex::ExEffect::Shell("info org".into()));
            Ok(())
        }
        "org-browse-news" => browse("https://orgmode.org/Changes.html"),
        "org-submit-bug-report" | "org-submit-feature-request" | "org-submit-patch" => {
            browse("https://orgmode.org/manual/Feedback.html")
        }
        "org-transpose-words" => {
            ed.set_msg("transpose-words: use Vim (e.g. dwwP)");
            Ok(())
        }
        "org-increase-number-at-point" | "org-decrease-number-at-point" => {
            let up = name.contains("increase");
            let n = arg.value();
            let line = ed.buf.line(l);
            let b = ed.cur.byte.min(line.len());
            let re = crate::org_re!(r"-?\d+(\.\d+)?");
            match re
                .find_iter(&line)
                .find(|m| m.start() <= b && b <= m.end() || m.start() > b)
            {
                Some(m) => {
                    let v: f64 = m.as_str().parse().unwrap_or(0.0);
                    let nv = v + if up { n as f64 } else { -(n as f64) };
                    let s = if m.as_str().contains('.') {
                        format!("{nv}")
                    } else {
                        format!("{}", nv as i64)
                    };
                    super::set_line(
                        ed,
                        l,
                        &format!("{}{s}{}", &line[..m.start()], &line[m.end()..]),
                    );
                    ed.set_cursor(l, m.start());
                    if ctx::at_table(ed, l) {
                        let _ = call(ed, "org-table-align", Prefix::None);
                    }
                    Ok(())
                }
                None => Err("Not on a number".into()),
            }
        }
        "org-delete-indentation" => delete_indentation(ed, arg),
        "org-indent-line" => indent_line(ed, l),
        "org-indent-region" => {
            let (lo, hi) = ed.org_region.unwrap_or((0, ed.line_count() - 1));
            (lo..=hi).try_for_each(|i| indent_line(ed, i))
        }
        "org-indent-drawer" | "org-indent-block" => {
            let drawer = name == "org-indent-drawer";
            match fold::wrapper_at(ed, l, drawer)
                .or_else(|| ctx::block_at(ed, l).map(|(_, b, e)| (b, e)))
            {
                Some((b, e)) => (b..=e).try_for_each(|i| indent_line(ed, i)),
                None => Err(if drawer {
                    "Not at a drawer".into()
                } else {
                    "Not at a block".into()
                }),
            }
        }
        "org-unindent-buffer" => {
            let n = ed.line_count();
            let lines: Vec<String> = (0..n).map(|i| ed.buf.line(i)).collect();
            if lines.iter().any(|t| ctx::item_bullet(t).is_some()) {
                return Some(Err("Cannot un-indent a buffer with lists".into()));
            }
            let out: Vec<String> = lines
                .iter()
                .map(|t| {
                    if syntax::level(t).is_some() {
                        t.clone()
                    } else {
                        t.trim_start().to_owned()
                    }
                })
                .collect();
            super::splice(ed, 0, n, &out);
            Ok(())
        }
        "org-fill-paragraph" => fill_paragraph(ed, !arg.is_none()),
        "org-toggle-fixed-width" => toggle_fixed_width(ed),
        "org-next-block" | "org-previous-block" => {
            let n = arg.value().max(1) as usize;
            let back = name.ends_with("previous-block");
            let is_begin = |t: &str| t.trim_start().to_ascii_lowercase().starts_with("#+begin");
            let mut cur = l;
            for _ in 0..n {
                let next = if back {
                    (0..cur).rev().find(|&i| is_begin(&ed.buf.line(i)))
                } else {
                    (cur + 1..ed.line_count()).find(|&i| is_begin(&ed.buf.line(i)))
                };
                match next {
                    Some(x) => cur = x,
                    None => return Some(Err("No block found".into())),
                }
            }
            ed.set_cursor(cur, 0);
            if fold::hidden(ed, cur) {
                fold::show_context(ed, cur);
            }
            Ok(())
        }
        "org-comment-dwim" => comment_dwim(ed),
        "org-clone-subtree-with-time-shift" => {
            let no_prompt_shift = arg == Prefix::U(1);
            super::read(ed, "Number of clones to produce: ", "", move |ed, n| {
                let Ok(n) = n.trim().parse::<usize>() else {
                    return ed.set_err(format!("Invalid number of replications {n}"));
                };
                let h = fold::back_to_heading(ed, ed.cur.line);
                let has_ts = h.is_some_and(|h| {
                    (h..syntax::subtree_end(&ed.buf, h))
                        .any(|i| super::time::stamp::ts_both().is_match(&ed.buf.line(i)))
                });
                let go = move |ed: &mut Editor, shift: String| {
                    ed.undo.begin(ed.cur.pos());
                    if let Err(e) = clone_subtree(ed, n, &shift) {
                        ed.set_err(e);
                    }
                    ed.undo.end(ed.cur.pos());
                };
                if has_ts && !no_prompt_shift {
                    super::read(
                        ed,
                        "Date shift per clone (e.g. +1w, empty to copy unchanged): ",
                        "",
                        go,
                    );
                } else {
                    go(ed, String::new());
                }
            });
            Ok(())
        }
        "org-tree-to-indirect-buffer" => {
            // Adaptation: no indirect buffers; narrow this buffer to the subtree.
            fold::command(ed, "org-narrow-to-subtree", arg).unwrap()
        }
        "org-force-cycle-archived" => fold::command(ed, "org-cycle-force-archived", arg).unwrap(),
        "org-advertized-archive-subtree" => call(ed, "org-archive-subtree", arg),
        "org-toggle-custom-properties-visibility" => {
            let custom = super::options::strings("org-custom-properties", &[]);
            if custom.is_empty() {
                return Some(Err("No custom properties to hide".into()));
            }
            let lines: Vec<usize> = (0..ed.line_count())
                .filter(|&i| {
                    props::parse_property(&ed.buf.line(i))
                        .is_some_and(|(k, _)| custom.iter().any(|c| c.eq_ignore_ascii_case(&k)))
                })
                .collect();
            let hide = lines.first().is_some_and(|&i| !fold::hidden(ed, i));
            for i in lines {
                fold::region(ed, i, i, hide, fold::Spec::Drawer);
            }
            Ok(())
        }
        "org-occur-in-agenda-files" => {
            occur_in_agenda_files(ed, false);
            Ok(())
        }
        "org-occur-link-in-agenda-files" => {
            occur_in_agenda_files(ed, true);
            Ok(())
        }
        "org-cdlatex-mode"
        | "org-cdlatex-underscore-caret"
        | "org-cdlatex-math-modify"
        | "org-cdlatex-environment-indent"
        | "org-reftex-citation"
        | "org-create-math-formula" => Err(format!(
            "{name}: needs the Emacs packages cdlatex/reftex, which Fred does not have"
        )),
        "org-setup-comments-handling"
        | "org-require-autoloaded-modules"
        | "org-org-menu"
        | "org-tbl-menu" => Ok(()),
        "org-backward-sentence" | "org-forward-sentence" => {
            crate::vim::normal_key(
                ed,
                Key::ch(if name.contains("forward") { ')' } else { '(' }),
            );
            Ok(())
        }
        _ => return None,
    })
}

fn tags_realign(ed: &mut Editor, h: usize) {
    if super::options::bool("org-auto-align-tags", true) {
        super::tags::align(ed, h);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::org;

    #[test]
    fn return_splits_headings_and_items() {
        let e = org("* Hello world :tag:", "6la<Enter><Esc>");
        assert_eq!(
            e.buf.line(0).split_whitespace().collect::<Vec<_>>(),
            vec!["*", "Hello", ":tag:"]
        );
        assert_eq!(e.buf.line(1), "world");
        let e = org("- item", "A<Enter>x<Esc>");
        assert_eq!(e.buf.text(), "- item\n  x");
        let e = org("text", "A<Enter>y<Esc>");
        assert_eq!(e.buf.text(), "text\ny");
        let e = org("| a |", "A|<Esc>");
        assert_eq!(e.buf.text(), "| a ||");
    }

    #[test]
    fn sorting_entries() {
        let e = org("* P\n** b\n** C\n** a", "<C-c>^a");
        assert_eq!(e.buf.text(), "* P\n** a\n** b\n** C");
        let e = org("* P\n** b\n** C\n** a", "<C-c>^A");
        assert_eq!(e.buf.text(), "* P\n** C\n** b\n** a");
        let e = org("* P\n** TODO [#C] x\n** [#A] y", "<C-c>^p");
        assert_eq!(e.buf.text(), "* P\n** [#A] y\n** TODO [#C] x");
        let e = org("top\n* 10 x\n* 9 y", "<C-c>^n");
        assert_eq!(e.buf.text(), "top\n* 9 y\n* 10 x");
    }

    #[test]
    fn fixed_width_comments_and_emphasis() {
        let e = org("text", "<C-c>:");
        assert_eq!(e.buf.text(), ": text");
        let e = org(": text", "<C-c>:");
        assert_eq!(e.buf.text(), "text");
        let e = org("a", ":org-comment-dwim<Enter>");
        assert_eq!(e.buf.text(), "# a");
        let e = org("word", "V<C-c><C-x><C-f>*");
        assert_eq!(e.buf.text(), "*word*");
    }
}
