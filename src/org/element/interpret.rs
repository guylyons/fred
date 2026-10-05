//! org-element-interpret-data and the org-element-TYPE-interpreter
//! functions: a tree back to Org syntax.

use super::ast::{Class, NO_AFFILIATED, NodeId, RECURSIVE_OBJECTS, Tree, Type, Value};
use super::objects::macro_escape_arguments;
use super::parser::{escape_code, trim};
use crate::org::options;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;
use unicode_width::UnicodeWidthStr;

/// org-element-interpret-data on a node.
pub fn interpret(t: &Tree, id: NodeId) -> String {
    Interp {
        t,
        over: HashMap::new(),
    }
    .fun(id, None)
}

/// org-element-interpret-data on a property value (secondary string,
/// string, list of them).
pub fn interpret_value(t: &Tree, v: &Value) -> String {
    let i = Interp {
        t,
        over: HashMap::new(),
    };
    i.value(v, None)
}

struct Interp<'a> {
    t: &'a Tree,
    /// Plain text replaced by org-element-normalize-contents.
    over: HashMap<NodeId, String>,
}

/// org-element-normalize-string.
pub fn normalize_string(s: &str, keep_newlines: bool) -> String {
    if s.is_empty() {
        return String::new();
    }
    if keep_newlines {
        return if s.ends_with('\n') {
            s.to_owned()
        } else {
            format!("{s}\n")
        };
    }
    // Strip trailing (newline + blanks)* then add one newline.
    let mut end = s.len();
    loop {
        let b = s[..end].trim_end_matches([' ', '\t']);
        if b.ends_with('\n') {
            end = b.len() - 1;
        } else {
            break;
        }
    }
    format!("{}\n", &s[..end])
}

/// Indentation string for COL columns in a buffer with tabs (indent-to).
fn indent_to(col: usize, tabs: bool) -> String {
    if tabs {
        format!("{}{}", "\t".repeat(col / 8), " ".repeat(col % 8))
    } else {
        " ".repeat(col)
    }
}

fn width_of_blanks(s: &str) -> usize {
    let mut col = 0;
    for c in s.chars() {
        col = if c == '\t' {
            (col / 8 + 1) * 8
        } else {
            col + 1
        };
    }
    col
}

/// org-remove-indentation.
pub fn remove_indentation(code: &str) -> String {
    let lines: Vec<&str> = code.split('\n').collect();
    let mut min = usize::MAX;
    for l in &lines {
        let t = l.trim_start_matches([' ', '\t']);
        if !t.is_empty() {
            let ind = width_of_blanks(&l[..l.len() - t.len()]);
            if ind == 0 {
                return code.to_owned();
            }
            min = min.min(ind);
        }
    }
    // Remove exactly MIN columns (a tab across the boundary becomes
    // spaces, as move-to-column with FORCE).
    let mut out: Vec<String> = vec![];
    for l in &lines {
        let t = l.trim_start_matches([' ', '\t']);
        let ws = &l[..l.len() - t.len()];
        if width_of_blanks(ws) < min {
            if !t.is_empty() {
                return code.to_owned();
            }
            out.push(String::new());
            continue;
        }
        let mut col = 0;
        let mut rest = String::new();
        for (i, c) in ws.char_indices() {
            let next = if c == '\t' {
                (col / 8 + 1) * 8
            } else {
                col + 1
            };
            if col >= min {
                rest.push_str(&ws[i..]);
                break;
            }
            if next > min {
                rest.push_str(&" ".repeat(next - min));
                rest.push_str(&ws[i + 1..]);
                break;
            }
            col = next;
        }
        out.push(format!("{rest}{t}"));
    }
    out.join("\n")
}

static SRC_IND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*\S").unwrap());

/// The value of a src or example block, reindented as upstream.
fn block_value(t: &Tree, id: NodeId) -> String {
    let val = t.str(id, "value").unwrap_or("");
    let preserve =
        t.flag(id, "preserve-indent") || options::bool("org-src-preserve-indentation", false);
    let ind = options::int("org-src-content-indentation", 2).max(0) as usize;
    if preserve {
        val.to_owned()
    } else if ind == 0 {
        remove_indentation(val)
    } else {
        let pad = " ".repeat(ind);
        SRC_IND
            .replace_all(&remove_indentation(val), |c: &regex::Captures| {
                format!("{pad}{}", &c[0])
            })
            .into_owned()
    }
}

/// org-priority-to-string.
fn priority_string(p: i64) -> String {
    if p < 65 {
        p.to_string()
    } else {
        char::from_u32(p as u32).map_or(String::new(), String::from)
    }
}

/// Tag alignment shared by headlines and inline tasks.
fn align_tags(heading: &str, tags: &str) -> String {
    let col = options::int("org-tags-column", -77);
    let hl = heading.chars().count() as i64;
    let tl = tags.chars().count() as i64;
    if col == 0 {
        format!(" {tags}")
    } else if col < 0 {
        format!("{}{tags}", " ".repeat((-(col + hl + tl)).max(1) as usize))
    } else {
        format!("{}{tags}", " ".repeat((col - hl).max(1) as usize))
    }
}

fn unit_str(v: &Value) -> &'static str {
    match v.as_str() {
        Some("hour") => "h",
        Some("day") => "d",
        Some("week") => "w",
        Some("month") => "m",
        Some("year") => "y",
        _ => "",
    }
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// format-time-string of org-time-stamp-format (no brackets) on an
/// encoded time (fields normalized like encode-time).
pub fn format_date(y: i64, mo: i64, d: i64, h: Option<i64>, mi: Option<i64>) -> String {
    let (hh, mm) = (h.unwrap_or(0), mi.unwrap_or(0));
    let total_min = hh * 60 + mm;
    let day_shift = total_min.div_euclid(24 * 60);
    let rem = total_min.rem_euclid(24 * 60);
    let y2 = y + (mo - 1).div_euclid(12);
    let m2 = (mo - 1).rem_euclid(12) + 1;
    let days = days_from_civil(y2, m2, 1) + d - 1 + day_shift;
    let (yy, mmo, dd) = civil_from_days(days);
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    let mut s = format!("{yy:04}-{mmo:02}-{dd:02} {wd}");
    if h.is_some() && mi.is_some() {
        s.push_str(&format!(" {:02}:{:02}", rem / 60, rem % 60));
    }
    s
}

/// org-element-timestamp-interpreter.
pub fn timestamp_string(t: &Tree, id: NodeId) -> String {
    let p = |k: &str| t.prop(id, k);
    let i = |k: &str| t.int(id, k);
    let ty = p("type").as_str().unwrap_or("").to_owned();
    let (ds, ms, ys) = (i("day-start"), i("month-start"), i("year-start"));
    if !(ds.is_some() && ms.is_some() && ys.is_some()) && ty != "diary" {
        return String::new();
    }
    let mut repeat = String::new();
    repeat.push_str(match p("repeater-type").as_str() {
        Some("cumulate") => "+",
        Some("catch-up") => "++",
        Some("restart") => ".+",
        _ => "",
    });
    if let Some(v) = i("repeater-value") {
        repeat.push_str(&v.to_string());
    }
    repeat.push_str(unit_str(p("repeater-unit")));
    if let (Some(v), u) = (i("repeater-deadline-value"), p("repeater-deadline-unit"))
        && u.truthy()
    {
        repeat.push_str(&format!("/{v}{}", unit_str(u)));
    }
    let mut warning = String::new();
    warning.push_str(match p("warning-type").as_str() {
        Some("first") => "--",
        Some("all") => "-",
        _ => "",
    });
    if let Some(v) = i("warning-value") {
        warning.push_str(&v.to_string());
    }
    warning.push_str(unit_str(p("warning-unit")));
    let (hs, mins) = (i("hour-start"), i("minute-start"));
    let (open, close) = if ty == "inactive" || ty == "inactive-range" {
        ("[", "]")
    } else {
        ("<", ">")
    };
    let nw = |s: &str| s.bytes().any(|b| !b" \r\t\n".contains(&b));
    let mut tail = String::new();
    if nw(&repeat) {
        tail.push(' ');
        tail.push_str(&repeat);
    }
    if nw(&warning) {
        tail.push(' ');
        tail.push_str(&warning);
    }
    tail.push_str(close);
    let mut out = open.to_owned();
    if ty == "diary" {
        out.push_str("%%");
        out.push_str(p("diary-sexp").as_str().unwrap_or(""));
        if let (Some(h), Some(m)) = (hs, mins) {
            out.push_str(&format!(" {h:02}:{m:02}"));
        }
    } else {
        let both = hs.is_some() && mins.is_some();
        out.push_str(&format_date(
            ys.unwrap(),
            ms.unwrap(),
            ds.unwrap(),
            if both { hs } else { None },
            if both { mins } else { None },
        ));
    }
    let (he, me) = (i("hour-end"), i("minute-end"));
    let range = p("range-type").as_str().map(str::to_owned);
    match ty.as_str() {
        "active" | "inactive" => {
            if range.is_none()
                && let (Some(hs), Some(he), Some(ms), Some(me)) = (hs, he, mins, me)
                && (hs != he || ms != me)
            {
                out.push_str(&format!("-{he:02}:{me:02}"));
            }
        }
        "active-range" | "inactive-range" | "diary" => {
            if ty == "diary" && range.as_deref() != Some("timerange") {
            } else if range.as_deref() == Some("timerange") {
                if let (Some(h), Some(m)) = (he.or(hs), me.or(mins)) {
                    out.push_str(&format!("-{h:02}:{m:02}"));
                }
            } else {
                out.push_str(&tail);
                out.push_str("--");
                out.push_str(open);
                let both = he.is_some() && me.is_some();
                out.push_str(&format_date(
                    i("year-end").or(ys).unwrap_or(0),
                    i("month-end").or(ms).unwrap_or(1),
                    i("day-end").or(ds).unwrap_or(1),
                    if both { he } else { None },
                    if both { me } else { None },
                ));
            }
        }
        _ => {}
    }
    out.push_str(&tail);
    out
}

static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:[<>]?[-+^.0-9]*[0-9][-+^.0-9eEdDx()%:]*|[<>]?[-+]?0[xX][0-9a-fA-F.]+|[<>]?[-+]?[0-9]+#[0-9a-zA-Z.]+|nan|[-+u]?inf)$").unwrap()
});
static ALIGN_COOKIE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A<([lrc])[0-9]*>\z").unwrap());

/// org-table-align on table text (rows starting with `|`).
pub fn table_align(text: &str) -> String {
    enum Row {
        Hline,
        Cells(Vec<String>),
    }
    let mut rows = vec![];
    let mut indent = "|".to_owned();
    for (n, line) in text.lines().enumerate() {
        let t = line.trim_start_matches([' ', '\t']);
        if !t.starts_with('|') {
            break;
        }
        if n == 0 {
            indent = format!("{}|", &line[..line.len() - t.len()]);
        }
        let rest = &t[1..];
        if rest.starts_with('-') {
            rows.push(Row::Hline);
            continue;
        }
        let mut cells = vec![];
        let mut r = rest;
        loop {
            r = r.trim_start_matches([' ', '\t']);
            if r.is_empty() {
                break;
            }
            let i = r.find('|').unwrap_or(r.len());
            cells.push(r[..i].trim_end_matches([' ', '\t']).to_owned());
            r = if i < r.len() { &r[i + 1..] } else { "" };
        }
        rows.push(Row::Cells(cells));
    }
    let cells: Vec<&Vec<String>> = rows
        .iter()
        .filter_map(|r| match r {
            Row::Cells(c) => Some(c),
            Row::Hline => None,
        })
        .collect();
    let (widths, aligns): (Vec<usize>, Vec<&str>) = if cells.is_empty() {
        let n = text.lines().next().map_or(0, |l| l.matches('+').count()) + 1;
        (vec![1; n], vec!["l"; n])
    } else {
        let ncol = cells.iter().map(|r| r.len()).max().unwrap_or(0);
        (0..ncol)
            .map(|i| {
                let mut w = 1;
                let (mut fixed, mut numbers, mut non_empty) = (None, 0, 0);
                for r in &cells {
                    let c = r.get(i).map_or("", String::as_str);
                    w = w.max(c.width());
                    if fixed.is_some() || c.is_empty() {
                        continue;
                    }
                    if let Some(m) = ALIGN_COOKIE.captures(c) {
                        fixed = Some(match &m[1] {
                            "r" => "r",
                            "c" => "c",
                            _ => "l",
                        });
                    } else {
                        non_empty += 1;
                        if NUMBER.is_match(c) {
                            numbers += 1;
                        }
                    }
                }
                let a = fixed.unwrap_or(if numbers as f64 >= 0.5 * non_empty as f64 {
                    "r"
                } else {
                    "l"
                });
                (w, a)
            })
            .unzip()
    };
    let rule: String = widths
        .iter()
        .map(|w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("+");
    let mut out = String::new();
    for r in &rows {
        out.push_str(&indent);
        match r {
            Row::Hline => out.push_str(&rule),
            Row::Cells(c) => {
                let fields: Vec<String> = (0..widths.len())
                    .map(|i| {
                        let f = c.get(i).map_or("", String::as_str);
                        let spaces = widths[i].saturating_sub(f.width());
                        let pre = match aligns[i] {
                            "r" => spaces,
                            "c" => spaces / 2,
                            _ => 0,
                        };
                        format!(" {}{f}{} ", " ".repeat(pre), " ".repeat(spaces - pre))
                    })
                    .collect();
                out.push_str(&fields.join("|"));
            }
        }
        out.push_str("|\n");
    }
    out
}

/// org-list-bullet-string.
fn bullet_string(b: &str) -> String {
    let t = b.trim_end_matches([' ', '\t']);
    if t.is_empty() {
        b.to_owned()
    } else {
        format!("{t} ")
    }
}

fn inc_bullet(b: &str) -> String {
    if let Some(m) = DIGITS.find(b) {
        let n: u64 = m.as_str().parse().unwrap_or(0);
        format!("{}{}{}", &b[..m.start()], n + 1, &b[m.end()..])
    } else if let Some(i) = b.find(|c: char| c.is_ascii_alphabetic()) {
        let c = (b.as_bytes()[i] + 1) as char;
        format!("{}{c}{}", &b[..i], &b[i + 1..])
    } else {
        b.to_owned()
    }
}

static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new("[0-9]+").unwrap());

/// org-list-repair on the text of an interpreted plain list: bullets,
/// indentation and checkboxes.
// ponytail: handles canonical interpreter output (one list, items at
// column 0 or nested by bullet width); arbitrary buffers go through
// org-list.el's port.
pub fn list_repair(text: &str) -> String {
    let st = super::with_default_parser(text, |p| p.list_struct(0, text.len()));
    if st.is_empty() {
        return text.to_owned();
    }
    let n = st.len();
    // Parents and previous siblings.
    let parent: Vec<Option<usize>> = (0..n)
        .map(|i| {
            (0..i)
                .rev()
                .find(|&j| st[j].ind < st[i].ind && st[j].end > st[i].pos)
        })
        .collect();
    let prev: Vec<Option<usize>> = (0..n)
        .map(|i| (0..n).find(|&j| st[j].end == st[i].pos && j != i && st[j].ind == st[i].ind))
        .collect();
    let mut bul: Vec<String> = st.iter().map(|i| i.bullet.clone()).collect();
    for i in 0..n {
        let counter = st[i].counter.as_deref();
        let num_counter = counter.filter(|c| c.bytes().all(|b| b.is_ascii_digit()));
        let new = match (prev[i], num_counter) {
            (Some(p), Some(c)) if DIGITS.is_match(&bul[p]) => {
                DIGITS.replace(&bul[p], c).into_owned()
            }
            (Some(p), _) => inc_bullet(&bul[p]),
            (None, Some(c)) if DIGITS.is_match(&bul[i]) => DIGITS.replace(&bul[i], c).into_owned(),
            (None, _) => match bul[i].find(|c: char| c.is_ascii_alphanumeric()) {
                Some(_) => FIRST_ORD.replace(&bul[i], "1").into_owned(),
                None => bul[i].clone(),
            },
        };
        bul[i] = bullet_string(&new);
    }
    let top = st[0].ind;
    let mut ind = vec![0; n];
    for i in 0..n {
        ind[i] = match parent[i] {
            Some(p) => ind[p] + bul[p].len(),
            None => top,
        };
    }
    let mut boxes: Vec<Option<String>> = st.iter().map(|i| i.checkbox.clone()).collect();
    let mut parents: Vec<usize> = (0..n)
        .filter_map(|i| parent[i])
        .filter(|&p| boxes[p].is_some())
        .collect();
    parents.sort_by_key(|&p| std::cmp::Reverse(ind[p]));
    parents.dedup();
    for p in parents {
        let kids: Vec<&str> = (0..n)
            .filter(|&c| parent[c] == Some(p))
            .filter_map(|c| boxes[c].as_deref())
            .collect();
        let has = |b: &str| kids.contains(&b);
        let nb = if has("[ ]") && has("[X]") || has("[-]") {
            "[-]"
        } else if has("[X]") {
            "[X]"
        } else if has("[ ]") {
            "[ ]"
        } else {
            continue;
        };
        boxes[p] = Some(nb.to_owned());
    }
    // org-list-struct-apply-struct: slice the list at item starts and
    // at item ends that are not items, shift each slice's body.
    let starts: Vec<usize> = st.iter().map(|i| i.pos).collect();
    let mut ends: Vec<usize> = starts.clone();
    ends.extend(st.iter().map(|i| i.end).filter(|e| !starts.contains(e)));
    ends.sort_unstable();
    ends.dedup();
    let lines: Vec<(usize, &str)> = {
        let mut pos = 0;
        text.split_inclusive('\n')
            .map(|l| {
                let s = pos;
                pos += l.len();
                (s, l)
            })
            .collect()
    };
    let blank = |l: &str| l.trim_matches([' ', '\t', '\n', '\r']).is_empty();
    let lead = |l: &str| width_of_blanks(&l[..l.len() - l.trim_start_matches([' ', '\t']).len()]);
    let mut new_ind: Vec<Option<usize>> = vec![None; lines.len()];
    for w in ends.windows(2) {
        let (up, down) = (w[0], w[1]);
        let (delta, min_col) = match st.iter().position(|i| i.pos == up) {
            Some(i) => (
                (ind[i] + bul[i].len()) as i64 - (st[i].ind + st[i].bullet.len()) as i64,
                ind[i] as i64 + 1,
            ),
            None => {
                let Some(child) = st.iter().position(|i| i.end == up) else {
                    continue;
                };
                // The least indented line of the slice aligns with CHILD.
                let mut min = usize::MAX;
                let mut skip_to: Option<String> = None;
                for &(_, l) in lines.iter().filter(|(s, _)| *s >= up && *s < down) {
                    if let Some(end) = &skip_to {
                        if l.trim().eq_ignore_ascii_case(end) {
                            skip_to = None;
                        }
                        continue;
                    }
                    if blank(l) {
                        continue;
                    }
                    min = min.min(lead(l));
                    let up8 = l.get(..8).unwrap_or("").to_ascii_uppercase();
                    if up8.starts_with("#+BEGIN:") || up8.starts_with("#+BEGIN_") {
                        let name: String =
                            l[7..].chars().take_while(|c| !c.is_whitespace()).collect();
                        skip_to = Some(format!("#+END{name}"));
                    } else if is_drawer_line(l) {
                        skip_to = Some(":END:".into());
                    }
                }
                if min == usize::MAX {
                    continue;
                }
                (ind[child] as i64 - min as i64, 0)
            }
        };
        if delta == 0 {
            continue;
        }
        let is_item = starts.contains(&up);
        for (k, &(s, l)) in lines.iter().enumerate() {
            if s < up || s >= down || (s == up && is_item) || blank(l) {
                continue;
            }
            new_ind[k] = Some((lead(l) as i64 + delta).max(min_col).max(0) as usize);
        }
    }
    let mut out = String::new();
    for (k, &(s, line)) in lines.iter().enumerate() {
        let t = line.trim_start_matches([' ', '\t']);
        if let Some(i) = st.iter().position(|it| it.pos == s) {
            let after_bullet = &t[st[i].bullet.len().min(t.len())..];
            let after = match (&st[i].checkbox, &boxes[i]) {
                (Some(old), Some(new)) if old != new => after_bullet.replacen(old.as_str(), new, 1),
                _ => after_bullet.to_owned(),
            };
            out.push_str(&" ".repeat(ind[i]));
            out.push_str(&bul[i]);
            out.push_str(&after);
        } else if let Some(n) = new_ind[k] {
            out.push_str(&" ".repeat(n));
            out.push_str(t);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// org-drawer-regexp: `:NAME:` alone on its line.
fn is_drawer_line(l: &str) -> bool {
    let t = l.trim_matches([' ', '\t', '\n', '\r']);
    t.len() > 2
        && t.starts_with(':')
        && t.ends_with(':')
        && t[1..t.len() - 1]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

static FIRST_ORD: LazyLock<Regex> = LazyLock::new(|| Regex::new("[0-9]+|[A-Za-z]").unwrap());

impl Interp<'_> {
    fn value(&self, v: &Value, parent: Option<NodeId>) -> String {
        match v {
            Value::Nil | Value::T => String::new(),
            Value::Str(s) | Value::Sym(s) => s.clone(),
            Value::Int(n) => n.to_string(),
            Value::Node(n) => self.fun(*n, parent),
            Value::Secondary(s) => s.iter().map(|&n| self.fun(n, parent)).collect(),
            Value::List(l) => l.iter().map(|x| self.value(x, parent)).collect(),
            Value::Cons(a, b) => format!("{}{}", self.value(a, parent), self.value(b, parent)),
        }
    }

    fn fun(&self, id: NodeId, parent: Option<NodeId>) -> String {
        let t = self.t;
        let ty = t.kind(id);
        let results = match ty {
            Type::Anonymous => t
                .contents(id)
                .iter()
                .map(|&c| self.fun(c, parent))
                .collect(),
            Type::PlainText => self
                .over
                .get(&id)
                .cloned()
                .unwrap_or_else(|| t.node(id).text.clone()),
            _ if t.contents(id).is_empty() => self.interpreter(id, None),
            _ => {
                let contents: String = if matches!(ty, Type::Paragraph | Type::VerseBlock) {
                    let ignore_first = ty == Type::Paragraph
                        && parent.is_some_and(|p| {
                            t.is(p, &[Type::FootnoteDefinition, Type::Item])
                                && t.contents(p).first() == Some(&id)
                                && t.int(p, "pre-blank") == Some(0)
                        });
                    let sub = Interp {
                        t,
                        over: normalize_contents(t, id, ignore_first),
                    };
                    t.contents(id)
                        .iter()
                        .map(|&c| sub.fun(c, Some(id)))
                        .collect()
                } else {
                    t.contents(id)
                        .iter()
                        .map(|&c| self.fun(c, Some(id)))
                        .collect()
                };
                self.interpreter(id, Some(contents))
            }
        };
        if matches!(ty, Type::OrgData | Type::Anonymous) {
            return results;
        }
        let blank = t.post_blank(id);
        if t.class_in(id, parent.or(t.parent(id))) == Class::Object {
            format!("{results}{}", " ".repeat(blank))
        } else {
            format!(
                "{}{}{}",
                self.affiliated(id),
                normalize_string(&results, true),
                "\n".repeat(blank)
            )
        }
    }

    /// org-element--interpret-affiliated-keywords.
    fn affiliated(&self, id: NodeId) -> String {
        let t = self.t;
        if NO_AFFILIATED.contains(&t.kind(id)) {
            return String::new();
        }
        let mut out = String::new();
        for (k, v) in t.props(id) {
            if v.is_nil() {
                continue;
            }
            let key = k.to_uppercase();
            let attr = key.starts_with("ATTR_");
            if !(attr || ["CAPTION", "HEADER", "NAME", "PLOT", "RESULTS"].contains(&key.as_str())) {
                continue;
            }
            if attr || key == "CAPTION" || key == "HEADER" {
                match v {
                    Value::List(l) => l.iter().for_each(|x| out.push_str(&self.keyword(&key, x))),
                    v => out.push_str(&self.keyword(&key, v)),
                }
            } else {
                out.push_str(&self.keyword(&key, v));
            }
        }
        out
    }

    fn keyword(&self, key: &str, value: &Value) -> String {
        let (main, dual) = if key == "CAPTION" || key == "RESULTS" {
            match value {
                Value::Cons(a, b) => ((**a).clone(), (**b).clone()),
                v => (v.clone(), Value::Nil),
            }
        } else {
            (value.clone(), Value::Nil)
        };
        let dual = if dual.truthy() {
            format!("[{}]", self.value(&dual, None))
        } else {
            String::new()
        };
        format!(
            "#+{}{dual}: {}\n",
            key.to_lowercase(),
            self.value(&main, None)
        )
    }

    /// org-element-TYPE-interpreter.
    fn interpreter(&self, id: NodeId, contents: Option<String>) -> String {
        let t = self.t;
        let s = |k: &str| t.str(id, k).unwrap_or("").to_owned();
        let c = contents.clone().unwrap_or_default();
        // `format "%s"` on nil contents prints "nil" upstream.
        let cn = contents.clone().unwrap_or_else(|| "nil".into());
        match t.kind(id) {
            Type::CenterBlock => format!("#+begin_center\n{cn}#+end_center"),
            Type::Drawer => format!(
                ":{}:\n{}{c}:END:",
                s("drawer-name"),
                "\n".repeat(t.int(id, "pre-blank").unwrap_or(0).max(0) as usize)
            ),
            Type::DynamicBlock => {
                let args = t
                    .str(id, "arguments")
                    .map_or(String::new(), |a| format!(" {a}"));
                format!("#+begin: {}{args}\n{cn}#+end:", s("block-name"))
            }
            Type::FootnoteDefinition => {
                let pre = t.int(id, "pre-blank").unwrap_or(1).clamp(0, 2) as usize;
                let body = if pre == 0 {
                    format!(" {}", trim(&c))
                } else {
                    format!("{}{c}", "\n".repeat(pre))
                };
                format!("[fn:{}]{body}", s("label"))
            }
            Type::Headline => self.headline(id, &c),
            Type::OrgData => format!(
                "{}{c}",
                "\n".repeat(t.int(id, "pre-blank").unwrap_or(0).max(0) as usize)
            ),
            Type::Inlinetask => self.inlinetask(id, contents),
            Type::Item => self.item(id, contents),
            Type::PlainList => list_repair(&c),
            Type::PropertyDrawer => format!(":PROPERTIES:\n{cn}:END:"),
            Type::QuoteBlock => format!("#+begin_quote\n{cn}#+end_quote"),
            Type::Section | Type::Paragraph => c,
            Type::SpecialBlock => {
                let ty = s("type");
                let params = t
                    .str(id, "parameters")
                    .map_or(String::new(), |p| format!(" {p}"));
                format!("#+begin_{ty}{params}\n{c}#+end_{ty}")
            }
            Type::BabelCall => {
                let ih = t
                    .str(id, "inside-header")
                    .map_or(String::new(), |h| format!("[{h}]"));
                let eh = t
                    .str(id, "end-header")
                    .map_or(String::new(), |h| format!(" {h}"));
                format!("#+call: {}{ih}({}){eh}", s("call"), s("arguments"))
            }
            Type::Clock => {
                let ts = t
                    .prop(id, "value")
                    .as_node()
                    .map_or(String::new(), |v| timestamp_string(t, v));
                let d = t.str(id, "duration").map_or(String::new(), |d| {
                    let parts = super::parser::split_string(d, ':');
                    let a = parts.first().map_or("", String::as_str);
                    let b = parts.get(1).map_or("", String::as_str);
                    format!(" => {a:>2}:{b:>2}")
                });
                format!("CLOCK: {ts}{d}")
            }
            Type::Comment => prefix_lines(&s("value"), "# "),
            Type::CommentBlock => {
                format!(
                    "#+begin_comment\n{}#+end_comment",
                    normalize_string(&remove_indentation(&s("value")), false)
                )
            }
            Type::DiarySexp => s("value"),
            Type::ExampleBlock => {
                let sw = t
                    .str(id, "switches")
                    .map_or(String::new(), |x| format!(" {x}"));
                format!(
                    "#+begin_example{sw}\n{}#+end_example",
                    normalize_string(&escape_code(&block_value(t, id)), false)
                )
            }
            Type::ExportBlock => format!(
                "#+begin_export {}\n{}#+end_export",
                t.str(id, "type").unwrap_or("nil"),
                s("value")
            ),
            Type::FixedWidth => match t.str(id, "value") {
                None => String::new(),
                Some("") => ":\n".to_owned(),
                Some(v) => prefix_lines(v, ": "),
            },
            Type::HorizontalRule => "-----".to_owned(),
            Type::Keyword => format!("#+{}: {}", s("key").to_lowercase(), s("value")),
            Type::LatexEnvironment | Type::LatexFragment | Type::StatisticsCookie => s("value"),
            Type::NodeProperty => {
                let k = format!(":{}:", s("key"));
                format!("{k:<10} {}", s("value"))
            }
            Type::Planning => {
                let mut parts = vec![];
                for (k, kw) in [
                    ("deadline", "DEADLINE:"),
                    ("scheduled", "SCHEDULED:"),
                    ("closed", "CLOSED:"),
                ] {
                    if let Some(n) = t.prop(id, k).as_node() {
                        parts.push(format!("{kw} {}", timestamp_string(t, n)));
                    }
                }
                parts.join(" ")
            }
            Type::SrcBlock => {
                let mut head = String::new();
                for k in ["language", "switches", "parameters"] {
                    if let Some(v) = t.str(id, k) {
                        head.push(' ');
                        head.push_str(v);
                    }
                }
                format!(
                    "#+begin_src{head}\n{}#+end_src",
                    normalize_string(&escape_code(&block_value(t, id)), false)
                )
            }
            Type::Table => {
                if t.str(id, "type") == Some("table.el") {
                    remove_indentation(&s("value"))
                } else {
                    let fm: Vec<String> = t
                        .prop(id, "tblfm")
                        .as_list()
                        .unwrap_or(&[])
                        .iter()
                        .rev()
                        .map(|f| format!("#+TBLFM: {}", f.as_str().unwrap_or("")))
                        .collect();
                    format!("{}{}", table_align(&c), fm.join("\n"))
                }
            }
            Type::TableRow => {
                if t.str(id, "type") == Some("rule") {
                    "|-".to_owned()
                } else {
                    format!("|{c}")
                }
            }
            Type::VerseBlock => format!("#+begin_verse\n{cn}#+end_verse"),
            Type::Bold => format!("*{cn}*"),
            Type::Citation => {
                let pre = t
                    .get(id, "prefix")
                    .map_or(String::new(), |p| format!("{};", self.value(p, Some(id))));
                let style = t
                    .str(id, "style")
                    .map_or(String::new(), |x| format!("/{x}"));
                let body = match t.get(id, "suffix") {
                    Some(suf) if suf.truthy() => format!("{c}{}", self.value(suf, Some(id))),
                    _ => {
                        let mut c = c;
                        c.pop();
                        c
                    }
                };
                format!("[cite{style}:{pre}{body}]")
            }
            Type::CitationReference => format!(
                "{}@{}{};",
                self.value(t.prop(id, "prefix"), Some(id)),
                s("key"),
                self.value(t.prop(id, "suffix"), Some(id))
            ),
            Type::Code => format!("~{}~", s("value")),
            Type::Entity => {
                format!(
                    "\\{}{}",
                    s("name"),
                    if t.flag(id, "use-brackets-p") {
                        "{}"
                    } else {
                        ""
                    }
                )
            }
            Type::ExportSnippet => format!("@@{}:{}@@", s("back-end"), s("value")),
            Type::FootnoteReference => format!(
                "[fn:{}{}]",
                s("label"),
                contents.map_or(String::new(), |c| format!(":{c}"))
            ),
            Type::InlineBabelCall => {
                let ih = t
                    .str(id, "inside-header")
                    .map_or(String::new(), |h| format!("[{h}]"));
                let eh = t
                    .str(id, "end-header")
                    .map_or(String::new(), |h| format!("[{h}]"));
                format!("call_{}{ih}({}){eh}", s("call"), s("arguments"))
            }
            Type::InlineSrcBlock => {
                let p = t
                    .str(id, "parameters")
                    .map_or(String::new(), |p| format!("[{p}]"));
                format!("src_{}{p}{{{}}}", s("language"), s("value"))
            }
            Type::Italic => format!("/{cn}/"),
            Type::LineBreak => "\\\\\n".to_owned(),
            Type::Link => self.link(id, contents),
            Type::Macro => {
                let args = match t.prop(id, "args").as_list() {
                    Some(l) if !l.is_empty() => {
                        let a: Vec<String> = l
                            .iter()
                            .map(|x| x.as_str().unwrap_or("").to_owned())
                            .collect();
                        format!("({})", macro_escape_arguments(&a))
                    }
                    _ => String::new(),
                };
                format!("{{{{{{{}{args}}}}}}}", s("key"))
            }
            Type::RadioTarget => format!("<<<{c}>>>"),
            Type::StrikeThrough => format!("+{cn}+"),
            Type::Subscript => {
                if t.flag(id, "use-brackets-p") {
                    format!("_{{{cn}}}")
                } else {
                    format!("_{cn}")
                }
            }
            Type::Superscript => {
                if t.flag(id, "use-brackets-p") {
                    format!("^{{{cn}}}")
                } else {
                    format!("^{cn}")
                }
            }
            Type::TableCell => format!(" {c} |"),
            Type::Target => format!("<<{}>>", s("value")),
            Type::Timestamp => timestamp_string(t, id),
            Type::Underline => format!("_{cn}_"),
            Type::Verbatim => format!("={}=", s("value")),
            Type::PlainText | Type::Anonymous | Type::Pseudo(_) => c,
        }
    }

    fn headline(&self, id: NodeId, contents: &str) -> String {
        let t = self.t;
        let level = t.int(id, "level").unwrap_or(1).max(1) as usize;
        let odd = options::bool("org-odd-levels-only", false);
        let mut h = "*".repeat(if odd { 2 * level - 1 } else { level });
        if let Some(k) = t.str(id, "todo-keyword") {
            h.push(' ');
            h.push_str(k);
        }
        if let Some(p) = t.int(id, "priority") {
            h.push_str(&format!(" [#{}]", priority_string(p)));
        }
        if t.flag(id, "commentedp") {
            h.push_str(" COMMENT");
        }
        h.push(' ');
        let fs = options::string("org-footnote-section", "Footnotes");
        if t.flag(id, "footnote-section-p") && !fs.is_empty() {
            h.push_str(&fs);
        } else {
            h.push_str(&self.value(t.prop(id, "title"), Some(id)));
        }
        let mut out = h.clone();
        if let Some(tags) = self.tags(id) {
            out.push_str(&align_tags(&h, &tags));
        }
        let pre = t.int(id, "pre-blank").unwrap_or(0).max(0) as usize;
        out.push_str(&"\n".repeat(pre + 1));
        out.push_str(contents);
        out
    }

    fn tags(&self, id: NodeId) -> Option<String> {
        let tags: Vec<&str> = self
            .t
            .prop(id, "tags")
            .as_list()?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        (!tags.is_empty()).then(|| format!(":{}:", tags.join(":")))
    }

    fn inlinetask(&self, id: NodeId, contents: Option<String>) -> String {
        let t = self.t;
        let level = t.int(id, "level").unwrap_or(1).max(1) as usize;
        let mut task = "*".repeat(level);
        if let Some(k) = t.str(id, "todo-keyword") {
            task.push(' ');
            task.push_str(k);
        }
        if let Some(p) = t.int(id, "priority") {
            task.push_str(&format!(" [#{}]", priority_string(p)));
        }
        if t.get(id, "title").is_some() {
            task.push(' ');
            task.push_str(&self.value(t.prop(id, "title"), Some(id)));
        }
        let mut out = task.clone();
        if let Some(tags) = self.tags(id) {
            out.push_str(&align_tags(&task, &tags));
        }
        if let Some(c) = contents {
            out.push('\n');
            out.push_str(&c);
            out.push_str(&"*".repeat(level));
            out.push_str(" end");
        }
        out
    }

    fn item(&self, id: NodeId, contents: Option<String>) -> String {
        let t = self.t;
        let tag = t
            .get(id, "tag")
            .filter(|v| v.truthy())
            .map(|v| format!("{} :: ", self.value(v, Some(id))));
        let b = t.str(id, "bullet").unwrap_or("-");
        let bullet = if !b.chars().any(|c| c.is_ascii_alphanumeric()) {
            "- ".to_owned()
        } else {
            match crate::org::sexp::option("org-plain-list-ordered-item-terminator")
                .and_then(|v| v.int())
            {
                Some(41) => "1) ".to_owned(),
                _ => "1. ".to_owned(),
            }
        };
        let mut out = bullet.clone();
        if let Some(c) = t.int(id, "counter") {
            out.push_str(&format!("[@{c}] "));
        }
        match t.str(id, "checkbox") {
            Some("on") => out.push_str("[X] "),
            Some("off") => out.push_str("[ ] "),
            Some("trans") => out.push_str("[-] "),
            _ => {}
        }
        if let Some(tg) = &tag {
            out.push_str(tg);
        }
        if let Some(c) = contents {
            let ind = " ".repeat(if tag.is_some() { 5 } else { bullet.len() });
            let pre = t.int(id, "pre-blank").unwrap_or(1).clamp(0, 2) as usize;
            let c = ITEM_IND
                .replace_all(&c, |m: &regex::Captures| format!("{ind}{}", &m[1]))
                .into_owned();
            if pre == 0 {
                out.push_str(trim(&c));
            } else {
                out.push_str(&"\n".repeat(pre));
                out.push_str(&c);
            }
        }
        out
    }

    fn link(&self, id: NodeId, contents: Option<String>) -> String {
        let t = self.t;
        let ty = t.str(id, "type").unwrap_or("");
        let path = t.str(id, "path").unwrap_or("");
        if ty == "radio" {
            return path.to_owned();
        }
        let body = match ty {
            "coderef" => format!("({path})"),
            "custom-id" => format!("#{path}"),
            "file" => {
                let explicit = t.flag(id, "type-explicit-p");
                let mut s = String::new();
                if explicit {
                    s.push_str("file");
                    if let Some(app) = t.str(id, "application") {
                        s.push('+');
                        s.push_str(app);
                    }
                    s.push(':');
                }
                s.push_str(path);
                if let Some(o) = t.str(id, "search-option") {
                    s.push_str("::");
                    s.push_str(o);
                }
                s
            }
            "fuzzy" => path.to_owned(),
            _ => format!("{ty}:{path}"),
        };
        let format = t.str(id, "format");
        match contents {
            Some(c) => format!("[[{body}][{c}]]"),
            None if format.is_none()
                || format == Some("bracket")
                || ["coderef", "custom-id", "fuzzy"].contains(&ty) =>
            {
                format!("[[{body}]]")
            }
            None if format == Some("angle") => format!("<{body}>"),
            None => body,
        }
    }
}

static ITEM_IND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^([ \t]*\S)").unwrap());

/// replace-regexp-in-string "^" PREFIX: every line start before the end.
fn prefix_lines(s: &str, prefix: &str) -> String {
    let mut out = String::new();
    for l in s.split_inclusive('\n') {
        out.push_str(prefix);
        out.push_str(l);
    }
    out
}

/// Indentation marks per text node: (start, end, width or blank line).
type Marks = HashMap<NodeId, Vec<(usize, usize, Option<usize>)>>;

/// org-element-normalize-contents: plain text of a paragraph or verse
/// block with the common indentation removed (by node).
fn normalize_contents(t: &Tree, id: NodeId, ignore_first: bool) -> HashMap<NodeId, String> {
    // Indentation marks per text node: (byte range, width or empty).
    let mut marks: HashMap<NodeId, Vec<(usize, usize, Option<usize>)>> = HashMap::new();
    let mut min = usize::MAX;
    fn walk(t: &Tree, id: NodeId, first: &mut bool, min: &mut usize, marks: &mut Marks) -> bool {
        for &d in t.contents(id) {
            let text = t.plain_text(d);
            if *first {
                *first = false;
                let Some(s) = text else { return false };
                let ws = s.len() - s.trim_start_matches([' ', '\t']).len();
                if ws == 0 {
                    return false;
                }
                let next = s.as_bytes().get(ws);
                if next == Some(&b'\n') {
                    marks.entry(d).or_default().push((0, ws, None));
                } else {
                    let w = width_of_blanks(&s[..ws]);
                    marks.entry(d).or_default().push((0, ws, Some(w)));
                    *min = (*min).min(w);
                }
            }
            if let Some(s) = text {
                for (nl, _) in s.match_indices('\n') {
                    let a = nl + 1;
                    let rest = &s[a..];
                    let ws = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                    let next = rest.as_bytes().get(ws);
                    if ws == 0 {
                        if !matches!(next, None | Some(b'\n')) {
                            return false;
                        }
                    } else if next == Some(&b'\n') {
                        marks.entry(d).or_default().push((a, a + ws, None));
                    } else {
                        let w = width_of_blanks(&rest[..ws]);
                        marks.entry(d).or_default().push((a, a + ws, Some(w)));
                        *min = (*min).min(w);
                    }
                }
            } else if t.kind(d) == Type::LineBreak {
                *first = true;
            } else if RECURSIVE_OBJECTS.contains(&t.kind(d)) && !walk(t, d, first, min, marks) {
                return false;
            }
        }
        true
    }
    let mut first = !ignore_first;
    if !walk(t, id, &mut first, &mut min, &mut marks) || min == 0 || min == usize::MAX {
        return HashMap::new();
    }
    marks
        .into_iter()
        .map(|(d, ms)| {
            let s = &t.node(d).text;
            let mut out = String::new();
            let mut last = 0;
            for (a, b, w) in ms {
                out.push_str(&s[last..a]);
                if let Some(w) = w {
                    out.push_str(&indent_to(w - min, true));
                }
                last = b;
            }
            out.push_str(&s[last..]);
            (d, out)
        })
        .collect()
}
