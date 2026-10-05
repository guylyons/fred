//! The spreadsheet (org-table.el "Formulas"): #+TBLFM parsing and
//! storing, references (`@r$c`, ranges, `@I` hline rows, relative rows,
//! `@<`/`$>`, named fields and columns, parameters, constants, `remote`),
//! column and field formulas, mode strings, durations, and
//! org-table-recalculate / iterate / eval-formula.

use super::calc::{self, FloatFmt, Modes};
use super::doc::{Doc, is_dataline, is_hline, is_tblfm, lazy_re, nw, re, trim};
use super::edit::{justify_field_maybe, line_to_dline, ncol_of, next_row, split_string};
use super::lisp;
use super::opt_bool;
use regex::{Captures, Regex};
use std::collections::HashSet;
use std::sync::LazyLock;

type R = Result<(), String>;

/// org-table-analyze's results.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// (name, column number as a string) from the `!` line.
    pub column_names: Vec<(String, String)>,
    /// From `$` lines and named fields.
    pub local_parameters: Vec<(String, String)>,
    /// (name, line from the table start, column).
    pub named_fields: Vec<(String, usize, usize)>,
    /// Line types from the table start: true = hline; an extra hline ends it.
    pub hline: Vec<bool>,
    /// Data line offsets; index 0 is unused (Emacs vector with nil).
    pub dlines: Vec<usize>,
    pub hlines: Vec<usize>,
    pub ncol: usize,
    pub begin: usize,
}

lazy_re!(COLNAMES, r"^[ \t]*\| *! *(\|.*)");
lazy_re!(PARAMS, r"^[ \t]*\| *\$ *(\|.*)");
lazy_re!(NAMEDROW, r"^[ \t]*\| *([_^]) *(\|.*)");
lazy_re!(FIELDSEP, r" *\| *");
lazy_re!(NAME, r"^[a-zA-Z][_a-zA-Z0-9]*$");
lazy_re!(PARAM_DEF, r"^([a-zA-Z][_a-zA-Z0-9]*|%) *= *(.*)");
lazy_re!(AFTER_FIRST, r"^[ \t]*\|[^|]*(\|.*)");

/// org-table-analyze on the table at point.
pub fn analyze(d: &mut Doc) {
    let beg = d.table_begin();
    let end = d.table_end();
    let mut an = Analysis { begin: beg, ..Analysis::default() };
    if let Some(l) = (beg..end).find(|&l| COLNAMES.is_match(&d.lines[l])) {
        let m = COLNAMES.captures(&d.lines[l]).unwrap();
        let mut c = 1;
        for name in split_string(&m[1], &FIELDSEP) {
            c += 1;
            if NAME.is_match(&name) {
                an.column_names.push((name, c.to_string()));
            }
        }
    }
    for l in beg..end {
        if let Some(m) = PARAMS.captures(&d.lines[l]) {
            for field in split_string(&m[1], &FIELDSEP) {
                if let Some(p) = PARAM_DEF.captures(&field) {
                    an.local_parameters.insert(0, (p[1].to_owned(), p[2].to_owned()));
                }
            }
        }
    }
    for l in beg..end {
        let Some(m) = NAMEDROW.captures(&d.lines[l]) else { continue };
        let fields = split_string(&m[2], &FIELDSEP);
        let other = if &m[1] == "_" { l + 1 } else { l.wrapping_sub(1) };
        if other >= d.n() {
            continue;
        }
        let Some(m1) = AFTER_FIRST.captures(&d.lines[other]) else { continue };
        let fields1 = split_string(&m1[1], &FIELDSEP);
        let mut col = 1;
        for (field, v) in fields.iter().zip(fields1.iter()) {
            col += 1;
            if NAME.is_match(field) {
                an.local_parameters.insert(0, (field.clone(), v.clone()));
                an.named_fields.insert(0, (field.clone(), other - beg, col));
            }
        }
    }
    an.dlines.push(0);
    an.hlines.push(0);
    for (i, l) in (beg..end).enumerate() {
        let h = is_hline(&d.lines[l]);
        an.hline.push(h);
        if h {
            an.hlines.push(i);
        } else {
            an.dlines.push(i);
        }
    }
    an.hline.push(true);
    an.ncol = an.dlines.get(1).map_or(0, |&i| ncol_of(&d.lines[beg + i]));
    d.an = an;
}

// ---- stored formulas ----

lazy_re!(TBLFM_TERM, r"^(@[-+I<>0-9.$@]+|\$([_a-zA-Z0-9]+|[<>]+)) *= *(.*[^ \t])");
lazy_re!(COLREF_LHS, r"^\$([0-9]+|[<>]+)$");
lazy_re!(TERM_SEP, r" *:: *");

/// The #+TBLFM line belonging to the table at point, or at `location`.
fn tblfm_line(d: &Doc, location: Option<usize>) -> Option<usize> {
    let mut l = location.unwrap_or_else(|| d.table_end());
    while l < d.n() && d.lines[l].trim().is_empty() && location.is_none() {
        l += 1;
    }
    (l < d.n() && is_tblfm(&d.lines[l])).then_some(l)
}

/// org-table-get-stored-formulas.
pub fn get_stored_formulas(d: &mut Doc, noerror: bool, location: Option<usize>) -> Result<Vec<(String, String)>, String> {
    let Some(l) = tblfm_line(d, location) else {
        return Ok(vec![]);
    };
    let line = d.lines[l].clone();
    let t = line.trim_start_matches([' ', '\t']);
    let body = t[8..].trim_start_matches(' ');
    let mut out = vec![];
    let mut seen: Vec<String> = vec![];
    for s in split_string(body, &TERM_SEP) {
        let Some(m) = TBLFM_TERM.captures(&s) else { continue };
        let whole = &m[1];
        let lhs = if m.get(2).is_none() || COLREF_LHS.is_match(whole) { whole.to_owned() } else { m[2].to_owned() };
        let rhs = m[3].to_owned();
        if seen.contains(&lhs) {
            let msg = format!("Double definition `{lhs}=' in TBLFM line, please fix by hand");
            if noerror {
                d.message(msg);
            } else {
                return Err(msg);
            }
        } else {
            seen.push(lhs.clone());
        }
        out.push((lhs, rhs));
    }
    Ok(out)
}

lazy_re!(CMP_RC, r"^(@([0-9]+))?(\$?([0-9]+))?(\$?[a-zA-Z0-9]+)?");

/// org-table-formula-make-cmp-string.
fn make_cmp_string(a: &str, an: &Analysis) -> Option<String> {
    let mut a = a.to_owned();
    if a.starts_with("$<") || a.starts_with("$>") {
        let arrow = a.as_bytes()[1];
        a = handle_first_last_rc(&a, an).unwrap_or(a);
        let n: i64 = a[1..].parse().unwrap_or(0);
        a = format!("${}", 10000 + if arrow == b'<' { -1000 } else { 0 } + n);
    }
    let m = CMP_RC.captures(&a)?;
    let mut s = String::new();
    if let Some(r) = m.get(2) {
        s.push_str(&format!("@{:05}", r.as_str().parse::<u64>().unwrap_or(0)));
    }
    if let Some(c) = m.get(4) {
        s.push_str(&format!("${:05}", c.as_str().parse::<u64>().unwrap_or(0)));
    }
    if let Some(x) = m.get(5) {
        s.push_str("@@");
        s.push_str(x.as_str());
    }
    Some(s)
}

/// org-table-formula-less-p, as a sort comparator.
fn formula_cmp(a: &(String, String), b: &(String, String), an: &Analysis) -> std::cmp::Ordering {
    match (make_cmp_string(&a.0, an), make_cmp_string(&b.0, an)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => std::cmp::Ordering::Equal,
    }
}

pub fn sort_formulas(v: &mut [(String, String)], an: &Analysis) {
    v.sort_by(|a, b| formula_cmp(a, b, an));
}

/// org-table-store-formulas.
pub fn store_formulas(d: &mut Doc, alist: &[(String, String)], location: Option<usize>) {
    let mut alist = alist.to_vec();
    let an = d.an.clone();
    sort_formulas(&mut alist, &an);
    let text = alist.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("::");
    match tblfm_line(d, location) {
        Some(l) => {
            let line = &d.lines[l];
            let i = line.to_ascii_lowercase().find("#+tblfm:").unwrap() + 8;
            d.lines[l] = format!("{} {text}", &line[..i]);
        }
        None => {
            let at = location.unwrap_or_else(|| d.table_end());
            let begin = d.begin_of(at.saturating_sub(1));
            let indent: String = d.lines.get(begin).map_or(String::new(), |l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect());
            let new = format!("{indent}#+TBLFM: {text}");
            let (pl, pc) = (d.l, d.c);
            d.lines.insert(at, new);
            d.goto(if pl >= at { pl + 1 } else { pl }, pc);
        }
    }
}

lazy_re!(IN_REMOTE, r"remote\([^)]+?\)");

/// Inside `remote(...)` at byte `pos` of `s` (org-in-regexp).
fn in_remote(s: &str, pos: usize) -> bool {
    IN_REMOTE.find_iter(s).any(|m| m.start() <= pos && pos < m.end())
}

/// org-table-fix-formulas: after structure edits, renumber `key` (`@`
/// or `$`) references: `replace` maps numbers, those above `limit`
/// shift by `delta`, terms assigning to `remove` are dropped.
pub fn fix_formulas(d: &mut Doc, key: char, replace: &[(String, String)], limit: Option<usize>, delta: i64, remove: Option<usize>) -> R {
    let mut l = d.table_end();
    let mut changed = false;
    while l < d.n() && is_tblfm(&d.lines[l]) {
        let mut s = d.lines[l].clone();
        if let Some(r) = remove {
            let re2 = if key == '$' { re(&format!(r"(@[0-9]+)?\${r}=.*?(::|$)")) } else { re(&format!(r"@{r}\$[0-9]+=.*?(::|$)")) };
            let mut from = 0;
            while let Some(m) = re2.find_at(&s, from).map(|m| (m.start(), m.end(), m.as_str().to_owned())) {
                let m = (m.0..m.1, m.2);
                if in_remote(&s, m.0.start) {
                    from = m.0.end.max(m.0.start + 1);
                    if from > s.len() {
                        break;
                    }
                    continue;
                }
                if m.0.start > 0 && s.as_bytes()[m.0.start - 1] == b'.' {
                    return Err(format!("Change makes TBLFM term {} invalid, use undo to recover", m.1));
                }
                s.replace_range(m.0.clone(), "");
                from = m.0.start;
                if from >= s.len() {
                    break;
                }
            }
        }
        let rk = re(&format!(r"{}([0-9]+)", regex::escape(&key.to_string())));
        let mut out = String::new();
        let mut last = 0;
        for m in rk.captures_iter(&s) {
            let whole = m.get(0).unwrap();
            if in_remote(&s, whole.start()) {
                continue;
            }
            let num = &m[1];
            let n: i64 = num.parse().unwrap_or(0);
            let new = if let Some((_, to)) = replace.iter().find(|(from, _)| from == num) {
                Some(format!("{key}{to}"))
            } else if limit.is_some_and(|lim| n > lim as i64) {
                Some(format!("{key}{}", n + delta))
            } else {
                None
            };
            if let Some(new) = new {
                out.push_str(&s[last..whole.start()]);
                out.push_str(&new);
                last = whole.end();
            }
        }
        out.push_str(&s[last..]);
        if d.lines[l] != out {
            d.lines[l] = out;
        }
        changed = true;
        l += 1;
    }
    if changed {
        d.message("The formulas in #+TBLFM have been updated");
    }
    Ok(())
}

// ---- references ----

lazy_re!(FIRST_LAST, r"([@$])(<+|>+)|(remote\([^)]+\))");

/// org-table-formula-handle-first/last-rc.
pub fn handle_first_last_rc(s: &str, an: &Analysis) -> Result<String, String> {
    let mut s = s.to_owned();
    let mut start = 0;
    while let Some(m) = FIRST_LAST.captures_at(&s, start) {
        if let Some(r) = m.get(3) {
            start = r.end();
            continue;
        }
        let whole = m.get(0).unwrap();
        let kind = &m[1];
        let arrows = &m[2];
        let nmax = if kind == "@" { an.dlines.len() as i64 - 1 } else { an.ncol as i64 };
        let len = arrows.len() as i64;
        let n = if arrows.starts_with('<') { len } else { nmax - len + 1 };
        if n < 1 || n > nmax {
            return Err(format!("Reference \"{}\" in expression \"{}\" points outside table", whole.as_str(), s));
        }
        let r = whole.range();
        start = r.start;
        s.replace_range(r, &format!("{kind}{n}"));
    }
    Ok(s)
}

/// org-table--row-type.
fn row_type(an: &Analysis, hline: bool, n: usize, mut i: i64, backwards: bool, relative: bool, desc: &str) -> Result<i64, String> {
    let l = an.hline.len() as i64;
    let cross = super::opt_string("org-table-relative-ref-may-cross-hline", "t");
    'outer: for _ in 0..n {
        loop {
            i += if backwards { -1 } else { 1 };
            if i < 0 || i >= l || an.hline[i as usize] == hline {
                break;
            }
            // Crossing a hline.
            if !relative || !an.hline[i as usize] || cross == "t" {
                continue;
            }
            if cross == "error" {
                return Err(format!("Row descriptor {desc} crosses hline"));
            }
            i -= if backwards { -1 } else { 1 };
            break 'outer;
        }
    }
    if i < 0 || i >= l {
        return Err(format!("Row descriptor {desc} leads outside table"));
    }
    Ok(if i == l - 1 { i - 1 } else { i })
}

lazy_re!(ROW_DESC, r"^(([-+])?(I+))?(([-+])?([0-9]+))?");

/// org-table--descriptor-line.
fn descriptor_line(an: &Analysis, desc: &str, cline: i64) -> Result<i64, String> {
    if !desc.is_empty() && desc.chars().all(|c| c.is_ascii_digit()) {
        let n: usize = desc.parse().unwrap_or(0);
        return an.dlines.get(n).filter(|_| n > 0).map(|&x| x as i64).ok_or_else(|| format!("Args out of range: row {desc}"));
    }
    let m = ROW_DESC.captures(desc).ok_or_else(|| format!("Invalid row descriptor `{desc}'"))?;
    if (m.get(3).is_none() && m.get(6).is_none()) || (m.get(3).is_some() && m.get(6).is_some() && m.get(5).is_none()) {
        return Err(format!("Invalid row descriptor `{desc}'"));
    }
    let mut hn = m.get(3).map(|x| x.as_str().len());
    let mut hdir = m.get(2).map(|x| x.as_str().to_owned());
    let odir = m.get(5).map(|x| x.as_str());
    let on = m.get(6).map(|x| x.as_str().parse::<usize>().unwrap_or(0));
    let rel = m.get(6).is_some() && ((m.get(1).is_some() && m.get(3).is_none()) || m.get(5).is_some());
    let mut cline = cline;
    if hn.is_some() && hdir.is_none() {
        cline = 0;
        hdir = Some("+".into());
        if an.hline[0] {
            hn = hn.map(|h| h - 1);
        }
    }
    if hn.is_none() && on.is_some() && odir.is_none() {
        return Err("Should never happen".into());
    }
    if let Some(h) = hn {
        cline = row_type(an, true, h, cline, hdir.as_deref() == Some("-"), false, desc)?;
    }
    if let Some(o) = on {
        cline = row_type(an, false, o, cline, odir == Some("-"), rel, desc)?;
    }
    Ok(cline)
}

lazy_re!(RANGE_RE, r"@([-+]?I*[-+]?[0-9]*)(\$[-+]?[0-9]+)?(\.\.@?([-+]?I*[-+]?[0-9]*)(\$[-+]?[0-9]+)?)?");
lazy_re!(SIMPLE_COLRANGE, r"^\$[0-9]+\.\.\$[0-9]+$");

/// A reference's value: one field or a list of fields.
#[derive(Clone, Debug, PartialEq)]
pub enum Refv {
    One(String),
    Many(Vec<String>),
}

/// org-table-get-range: `desc` relative to the current line of the table
/// starting at `tbeg`, current column `col`.
pub fn get_range(d: &mut Doc, desc: &str, col: Option<usize>) -> Result<Refv, String> {
    let desc = if SIMPLE_COLRANGE.is_match(desc) { desc.replace('$', "@0$") } else { desc.to_owned() };
    let col = col.unwrap_or_else(|| d.current_column());
    let tbeg = d.an.begin;
    let thisline = d.l as i64 - tbeg as i64;
    let m = RANGE_RE.captures(&desc).ok_or_else(|| format!("Invalid table range specifier `{desc}'"))?;
    let an = d.an.clone();
    let rangep = m.get(3).is_some();
    let row = |g: Option<regex::Match>| -> Result<i64, String> {
        match g.map(|x| x.as_str()).filter(|s| nw(s)) {
            Some(r) => descriptor_line(&an, r, thisline),
            None => Ok(thisline),
        }
    };
    let colf = |g: Option<regex::Match>| -> i64 {
        match g.map(|x| &x.as_str()[1..]) {
            None => col as i64,
            Some(c) => {
                let n: i64 = c.trim_start_matches('+').parse().unwrap_or(0);
                if n == 0 {
                    col as i64
                } else {
                    n + if c.starts_with(['-', '+']) { col as i64 } else { 0 }
                }
            }
        }
    };
    let r1 = row(m.get(1))?;
    let r2 = row(m.get(4))?;
    let c1 = colf(m.get(2));
    let c2 = colf(m.get(5));
    let (l0, p0) = (d.l, d.c);
    let res = (|| {
        if !rangep || (r1 == r2 && c1 == c2) {
            let mut l = (tbeg as i64 + r1).max(0) as usize;
            while l < d.n() && !is_dataline(&d.lines[l]) {
                l += 1;
            }
            if l >= d.n() {
                return Err("End of buffer".to_string());
            }
            d.goto(l, 0);
            let f = d.get_field(Some(c1.max(0) as usize), None);
            return Ok(Refv::One(trim(&f).to_owned()));
        }
        let (fr, lr) = (r1.min(r2), r1.max(r2));
        let (fc, lc) = (c1.min(c2).max(0) as usize, c1.max(c2).max(0) as usize);
        let mut l = (tbeg as i64 + fr).max(0) as usize;
        while l < d.n() && !is_dataline(&d.lines[l]) {
            l += 1;
        }
        let mut e = ((tbeg as i64 + lr).max(0) as usize).min(d.n() - 1);
        while e > 0 && !is_dataline(&d.lines[e]) {
            e -= 1;
        }
        let mut out = vec![];
        for line in l..=e {
            if !is_dataline(&d.lines[line]) {
                continue;
            }
            d.goto(line, 0);
            for c in fc..=lc {
                let f = d.get_field(Some(c), None);
                out.push(trim(&f).to_owned());
            }
        }
        // copy-region checks both corners are inside data fields.
        d.goto(l.min(d.n() - 1), 0);
        d.goto_column(fc, false, false);
        d.check_inside_data_field(false, true)?;
        Ok(Refv::Many(out))
    })();
    d.goto(l0, p0);
    res
}

/// org-table-get-range with CORNERS-ONLY: (row1, col1, row2, col2).
fn range_corners(d: &Doc, desc: &str, col: usize) -> Result<(i64, i64, i64, i64), String> {
    let m = RANGE_RE.captures(desc).ok_or_else(|| format!("Invalid table range specifier `{desc}'"))?;
    let thisline = d.l as i64 - d.an.begin as i64;
    let row = |g: Option<regex::Match>| -> Result<i64, String> {
        match g.map(|x| x.as_str()).filter(|s| nw(s)) {
            Some(r) => descriptor_line(&d.an, r, thisline),
            None => Ok(thisline),
        }
    };
    let colf = |g: Option<regex::Match>| -> i64 {
        match g.map(|x| &x.as_str()[1..]) {
            None => col as i64,
            Some(c) => {
                let n: i64 = c.trim_start_matches('+').parse().unwrap_or(0);
                if n == 0 { col as i64 } else { n + if c.starts_with(['-', '+']) { col as i64 } else { 0 } }
            }
        }
    };
    let (r1, r2, c1, c2) = (row(m.get(1))?, row(m.get(4))?, colf(m.get(2)), colf(m.get(5)));
    Ok((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

/// org-table-expand-lhs-ranges.
pub fn expand_lhs_ranges(d: &mut Doc, eqs: Vec<(String, String)>) -> Result<Vec<(String, String, Option<String>)>, String> {
    static FIXED: LazyLock<Regex> = LazyLock::new(|| re(r"^@[-+0-9]+\$-?[0-9]+$"));
    static NAMED: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-zA-Z][_a-zA-Z0-9]*$"));
    static COL: LazyLock<Regex> = LazyLock::new(|| re(r"^\$[0-9]+$"));
    static ROW: LazyLock<Regex> = LazyLock::new(|| re(r"^@[0-9]+$"));
    let mut res = vec![];
    for (lhs, rhs) in eqs {
        if FIXED.is_match(&lhs) || NAMED.is_match(&lhs) || COL.is_match(&lhs) {
            res.push((lhs, rhs, None));
        } else if ROW.is_match(&lhs) {
            for ic in 1..=d.an.ncol {
                res.push((format!("{lhs}${ic}"), rhs.clone(), Some(format!("{lhs}={rhs}"))));
            }
        } else {
            let (l0, c0) = (d.l, d.c);
            d.goto(d.an.begin, 0);
            let r = range_corners(d, &lhs, 1);
            d.goto(l0, c0);
            let (fr, c1, lr, c2) = r?;
            let r1 = line_to_dline(&d.an.dlines, fr.max(0) as usize, false).unwrap_or(1);
            let r2 = line_to_dline(&d.an.dlines, lr.max(0) as usize, true).unwrap_or(0);
            for ir in r1..=r2 {
                for ic in c1..=c2 {
                    res.push((format!("@{ir}${ic}"), rhs.clone(), Some(format!("{lhs}={rhs}"))));
                }
            }
        }
    }
    Ok(res)
}

/// The kind of formula a reference is inserted into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lispp {
    Calc,
    Lisp,
    Literal,
}

/// org-table-make-reference.
pub fn make_reference(elements: &Refv, keep_empty: bool, numbers: bool, lispp: Lispp) -> String {
    let num = |s: &str| lisp::num_to_string(&lisp::string_to_number(s));
    match elements {
        Refv::One(e) => {
            if lispp != Lispp::Calc {
                if lispp == Lispp::Literal {
                    e.clone()
                } else if numbers {
                    num(e)
                } else {
                    lisp::prin1(&lisp::str_(e))
                }
            } else if nw(e) {
                format!("({})", if numbers { num(e) } else { e.clone() })
            } else if !keep_empty || numbers {
                "(0)".into()
            } else {
                "nan".into()
            }
        }
        Refv::Many(v) => {
            let v: Vec<&String> = v.iter().filter(|x| keep_empty || nw(x)).collect();
            if lispp != Lispp::Calc {
                v.iter()
                    .map(|x| {
                        if lispp == Lispp::Literal {
                            (*x).clone()
                        } else if numbers {
                            num(x)
                        } else {
                            lisp::prin1(&lisp::str_(x))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                format!(
                    "[{}]",
                    v.iter()
                        .map(|x| {
                            if nw(x) {
                                if numbers { num(x) } else { (*x).clone() }
                            } else if !keep_empty || numbers {
                                "0".into()
                            } else {
                                "nan".into()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
    }
}

// ---- names and constants ----

/// org-table-get-constant.
pub fn get_constant(d: &Doc, name: &str) -> String {
    if let Some((_, v)) = d.an.local_parameters.iter().find(|(k, _)| k == name) {
        return v.clone();
    }
    if let Some(v) = buffer_constants(&d.lines).into_iter().find(|(k, _)| k == name) {
        return v.1;
    }
    match super::super::options::get("org-table-formula-constants") {
        Some(toml::Value::Table(t)) => {
            if let Some(v) = t.get(name) {
                return match v {
                    toml::Value::String(s) => s.clone(),
                    o => o.to_string(),
                };
            }
        }
        Some(toml::Value::Array(a)) => {
            for e in a {
                if let toml::Value::Array(p) = e
                    && p.first().and_then(|x| x.as_str()) == Some(name)
                {
                    return p.get(1).and_then(|x| x.as_str()).unwrap_or("").to_owned();
                }
            }
        }
        _ => {}
    }
    if let Some(prop) = name.strip_prefix("PROP_")
        && let Some(v) = super::entry_get(&d.lines, d.l, prop, true)
    {
        return v;
    }
    "#UNDEFINED_NAME".into()
}

/// `#+CONSTANTS:` definitions in the buffer.
fn buffer_constants(lines: &[String]) -> Vec<(String, String)> {
    let mut out = vec![];
    for l in lines {
        let t = l.trim_start();
        if t.len() > 12 && t[..12].eq_ignore_ascii_case("#+constants:") {
            for def in t[12..].split_whitespace() {
                if let Some((k, v)) = def.split_once('=') {
                    out.push((k.to_owned(), v.to_owned()));
                }
            }
        }
    }
    out
}

lazy_re!(NAME_OR_REMOTE, r"\$([a-zA-Z][_a-zA-Z0-9]*)|(\bremote\([^)]*\))");
lazy_re!(DURATION_FMT, r";.*[Tt].*$");

/// org-table-formula-substitute-names.
pub fn substitute_names(d: &Doc, f: &str) -> String {
    let pp = !f.starts_with('\'');
    let duration = DURATION_FMT.is_match(f);
    let mut new = f.to_owned();
    if !d.an.column_names.is_empty() {
        let alts = d.an.column_names.iter().map(|(n, _)| regex::escape(n)).collect::<Vec<_>>().join("|");
        let r = re(&format!(r"\$({alts})\b"));
        new = r
            .replace_all(&new, |c: &Captures| {
                let num = &d.an.column_names.iter().find(|(n, _)| n == &c[1]).unwrap().1;
                format!("${num}")
            })
            .into_owned();
    }
    let mut start = 0;
    while let Some(m) = NAME_OR_REMOTE.captures_at(&new, start) {
        if let Some(r) = m.get(2) {
            start = r.end();
            continue;
        }
        let whole = m.get(0).unwrap().range();
        start = whole.start + 1;
        let mut v = get_constant(d, &m[1]);
        if duration && nw(&v) {
            v = time_string_to_seconds(&v);
        }
        let rep = if pp { format!("({v})") } else { v };
        new.replace_range(whole, &rep);
    }
    new
}

// ---- A1 references ----

/// org-letters-to-number.
pub fn letters_to_number(s: &str) -> usize {
    s.to_ascii_uppercase().bytes().fold(0, |n, c| n * 26 + (c - b'A') as usize + 1)
}

/// org-number-to-letters.
pub fn number_to_letters(mut n: usize) -> String {
    let mut s = String::new();
    while n > 0 {
        s.insert(0, (b'A' + ((n - 1) % 26) as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

lazy_re!(A1_OR_SKIP, r"\b([a-zA-Z]+)([0-9]+\b|&)|(;[^\r\n:]+|\bremote\([^,)]*\))");

/// org-table-convert-refs-to-rc: `B3` to `@3$2`, `C&` to `$3`.
pub fn convert_refs_to_rc(s: &str) -> String {
    let mut s = s.to_owned();
    let mut start = 0;
    while let Some(m) = A1_OR_SKIP.captures_at(&s, start) {
        let whole = m.get(0).unwrap();
        if m.get(3).is_some() {
            start = whole.end();
            continue;
        }
        let b = whole.start();
        if b > 0 && s.as_bytes()[b - 1] == b'.' && !(b >= 2 && s.as_bytes()[b - 2] == b'.') {
            start = whole.end();
            continue;
        }
        let letters = m.get(1).unwrap();
        if letters.as_str().len() > 2 {
            start = whole.end();
            continue;
        }
        let rep = if &m[2] == "&" {
            format!("${}", letters_to_number(letters.as_str()))
        } else {
            format!("@{}${}", m[2].parse::<u64>().unwrap_or(0), letters_to_number(letters.as_str()))
        };
        let r = whole.range();
        start = r.start;
        s.replace_range(r, &rep);
    }
    s
}

lazy_re!(RC_ABS, r"@([0-9]+)\$([0-9]+)");
lazy_re!(COL_ABS, r"(^|[^0-9a-zA-Z])\$([1-9][0-9]*)");

/// org-table-convert-refs-to-an: `@7$28` to `AB7`, `$3` to `C&`.
pub fn convert_refs_to_an(s: &str) -> String {
    let mut s = s.to_owned();
    while let Some(m) = RC_ABS.captures(&s) {
        let rep = format!("{}{}", number_to_letters(m[2].parse().unwrap_or(0)), m[1].parse::<u64>().unwrap_or(0));
        let r = m.get(0).unwrap().range();
        s.replace_range(r, &rep);
    }
    while let Some(m) = COL_ABS.captures(&s) {
        let rep = format!("{}{}&", &m[1], number_to_letters(m[2].parse().unwrap_or(0)));
        let r = m.get(0).unwrap().range();
        s.replace_range(r, &rep);
    }
    s
}

/// org-table-formula-to-user.
pub fn to_user(s: &str) -> String {
    if super::opt_string("org-table-use-standard-references", "from") == "t" { convert_refs_to_an(s) } else { s.to_owned() }
}

/// org-table-formula-from-user.
pub fn from_user(s: &str) -> String {
    if super::opt_string("org-table-use-standard-references", "from") != "nil" { convert_refs_to_rc(s) } else { s.to_owned() }
}

// ---- durations ----

lazy_re!(HMS3, r"(-?)([0-9]+):([0-9]+):([0-9]+)");
lazy_re!(HM2, r"(-?)([0-9]+):([0-9]+)");
lazy_re!(TS_BOTH, r"[\[<][0-9]{4}-[0-9]{2}-[0-9]{2} ?[^\]\r\n>]*?[\]>]");

/// org-table-time-string-to-seconds.
pub fn time_string_to_seconds(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let hms = |m: &Captures, sec: bool| -> i64 {
        let v = m[2].parse::<i64>().unwrap_or(0) * 3600 + m[3].parse::<i64>().unwrap_or(0) * 60 + if sec { m[4].parse::<i64>().unwrap_or(0) } else { 0 };
        if m[1].is_empty() { v } else { -v }
    };
    if let Some(m) = HMS3.captures(s) {
        return hms(&m, true).to_string();
    }
    if !TS_BOTH.is_match(s)
        && let Some(m) = HM2.captures(s)
    {
        return hms(&m, false).to_string();
    }
    lisp::num_to_string(&lisp::string_to_number(s))
}

/// org-table-time-seconds-to-string with OUTPUT-FORMAT `fmt` (days,
/// hours, minutes, seconds, hh:mm, or "" for HH:MM:SS).
pub fn time_seconds_to_string(secs: f64, fmt: &str) -> String {
    let secs0 = secs.abs();
    let pad = opt_bool("org-table-duration-hour-zero-padding", true);
    let hms = |s: f64| {
        let s = s.floor() as i64;
        let (h, m, x) = (s / 3600, s % 3600 / 60, s % 60);
        if pad { format!("{h:02}:{m:02}:{x:02}") } else { format!("{h}:{m:02}:{x:02}") }
    };
    let res = match fmt {
        "days" => format!("{:.3}", secs0 / 86400.0),
        "hours" => format!("{:.2}", secs0 / 3600.0),
        "minutes" => format!("{:.1}", secs0 / 60.0),
        "seconds" => format!("{}", secs0.trunc() as i64),
        "hh:mm" => {
            let s = hms(secs0);
            s[..s.len() - 3].to_owned()
        }
        _ => hms(secs0),
    };
    if secs < 0.0 { format!("-{res}") } else { res }
}

// ---- timestamps in Calc formulas ----

lazy_re!(TS_INACTIVE, r"\[([0-9]{4}-[0-9]{2}-[0-9]{2} ?[^\]\r\n>]*?)\]");
lazy_re!(TS_ACTIVE, r"<([0-9]{4}-[0-9]{2}-[0-9]{2} ?[^\]\r\n>]*?)>");
lazy_re!(TS_PARTS, r"^([0-9]{4})-([0-9]{2})-([0-9]{2})(?:[^0-9]*?([0-9]{1,2}):([0-9]{2}))?");

/// Active timestamps re-rendered in the C locale for Calc.
fn localize_timestamps(form: &str) -> String {
    let form = TS_INACTIVE.replace_all(form, "<$1>").into_owned();
    TS_ACTIVE
        .replace_all(&form, |c: &Captures| {
            let Some(p) = TS_PARTS.captures(&c[1]) else {
                return c[0].to_owned();
            };
            let (y, mo, da): (i64, u32, u32) = (p[1].parse().unwrap_or(0), p[2].parse().unwrap_or(1), p[3].parse().unwrap_or(1));
            let days = super::days_from_civil(y, mo, da);
            let wd = super::weekday_name(days);
            match (p.get(4), p.get(5)) {
                (Some(h), Some(m)) => format!("<{y:04}-{mo:02}-{da:02} {wd} {:02}:{}>", h.as_str().parse::<u32>().unwrap_or(0), m.as_str()),
                _ => format!("<{y:04}-{mo:02}-{da:02} {wd}>"),
            }
        })
        .into_owned()
}

// ---- evaluation ----

/// The prefix argument of org-table-eval-formula.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalArg {
    None,
    /// C-u: a field formula.
    Field,
    /// C-u C-u: put the formula back into the field.
    Insert,
    /// A count: apply down this many rows.
    Down(i64),
}

/// org-table-current-field-formula: (key, "=formula" or ":=formula").
pub fn current_field_formula(d: &mut Doc, noerror: bool) -> Result<Option<(String, String)>, String> {
    let line = d.l - d.an.begin;
    if line_to_dline(&d.an.dlines, line, false).is_none() || !is_dataline(d.line()) {
        return if noerror { Ok(None) } else { Err("No formula active for the current field".into()) };
    }
    let col = d.current_column();
    let name = d.an.named_fields.iter().find(|(_, l, c)| *l == line && *c == col).map(|x| x.0.clone());
    let scol = format!("${col}");
    let rf = format!("@{}${col}", d.current_dline());
    let stored = get_stored_formulas(d, noerror, None)?;
    let ass = name
        .as_ref()
        .and_then(|n| stored.iter().find(|(k, _)| k == n))
        .or_else(|| stored.iter().find(|(k, _)| *k == rf))
        .or_else(|| stored.iter().find(|(k, _)| *k == scol));
    Ok(ass.map(|(k, v)| {
        let prefix = if k.chars().all(|c| c.is_ascii_digit()) { "=" } else { ":=" };
        (k.clone(), format!("{prefix}{v}"))
    }))
}

/// The prompt and default for org-table-get-formula.
pub fn formula_prompt(d: &mut Doc, named: bool) -> Result<(String, String), String> {
    let stored = get_stored_formulas(d, false, None)?;
    let line = d.l - d.an.begin;
    let col = d.current_column();
    let name = d.an.named_fields.iter().find(|(_, l, c)| *l == line && *c == col).map(|x| x.0.clone());
    let rf = format!("@{}${}", d.current_dline(), col);
    let scol = if !named { format!("${col}") } else { name.unwrap_or(rf) };
    let stored = stored.iter().find(|(k, _)| *k == scol).map(|x| x.1.clone());
    Ok((
        to_user(&format!("{} formula {}=", if named { "Field" } else { "Column" }, scol)),
        stored.map(|s| to_user(&s)).unwrap_or_default(),
    ))
}

/// org-table-get-formula with the user's EQUATION.
pub fn get_formula(d: &mut Doc, equation: &str, named: bool) -> Result<String, String> {
    let mut stored_list = get_stored_formulas(d, false, None)?;
    let line = d.l - d.an.begin;
    let col = d.current_column();
    let name = d.an.named_fields.iter().find(|(_, l, c)| *l == line && *c == col).map(|x| x.0.clone());
    let rf = format!("@{}${}", d.current_dline(), col);
    let scol = if !named { format!("${col}") } else { name.clone().unwrap_or_else(|| rf.clone()) };
    let name = name.or(Some(rf));
    let saved = d.st.may_need_update;
    d.st.may_need_update = false;
    let stored = stored_list.iter().find(|(k, _)| *k == scol).map(|x| x.1.clone());
    static BLANK_EQ: LazyLock<Regex> = LazyLock::new(|| re(r"^ *=? *$"));
    let mut eq = match &stored {
        Some(s) if BLANK_EQ.is_match(equation) => s.clone(),
        _ => equation.to_owned(),
    };
    d.st.may_need_update = saved;
    if !nw(&eq) {
        stored_list.retain(|(k, _)| *k != scol);
        store_formulas(d, &stored_list, None);
        return Err("Formula removed".into());
    }
    eq = eq.trim_start_matches(' ').trim_start_matches('=').to_owned();
    eq = eq.trim_end_matches(' ').to_owned();
    let mut mustsave = false;
    if let Some(n) = &name
        && !named
        && stored_list.iter().any(|(k, _)| k == n)
    {
        stored_list.retain(|(k, _)| k != n);
        mustsave = true;
    }
    match stored_list.iter_mut().find(|(k, _)| *k == scol) {
        Some(slot) => slot.1 = eq.clone(),
        None => stored_list.insert(0, (scol, eq.clone())),
    }
    if mustsave || stored.as_deref() != Some(eq.as_str()) {
        store_formulas(d, &stored_list, None);
    }
    Ok(eq)
}

lazy_re!(INLINE_FORMULA, r"^:?=(.*[^=])$");

/// org-table-maybe-eval-formula.
pub fn maybe_eval_formula(d: &mut Doc) -> R {
    if !opt_bool("org-table-formula-evaluate-inline", true) || !d.at_table() {
        return Ok(());
    }
    let f = d.get_field(None, None);
    let field = trim(&f).to_owned();
    if let Some(m) = INLINE_FORMULA.captures(&field) {
        let named = field.starts_with(':');
        let eq = from_user(&m[1]);
        eval_formula(d, if named { EvalArg::Field } else { EvalArg::None }, Some(&eq), false, false, false, false)?;
    }
    Ok(())
}

lazy_re!(AUTO_RECALC, r"^[ \t]*\| *# *(\||$)");

/// org-table-maybe-recalculate-line.
pub fn maybe_recalculate_line(d: &mut Doc) -> Result<bool, String> {
    if !opt_bool("org-table-allow-automatic-line-recalculation", true) {
        return Ok(false);
    }
    let just_done = d.last_command.as_ref().is_some_and(|c| d.st.recalc_commands.contains(c)) && d.st.last_recalc_line == Some(d.l);
    if just_done || !d.at_table() || !AUTO_RECALC.is_match(d.line()) {
        return Ok(false);
    }
    recalculate(d, Recalc::Line, false)?;
    Ok(true)
}

lazy_re!(FMT_SPLIT, r"^(.*);(.*)$");
lazy_re!(FMT_NUMFLAG, r"([pnfse])(-?[0-9]+)");
lazy_re!(FMT_FLAG, r"[tTUNLEDRFSu]");
lazy_re!(ROWCOL_HASH, r"[@$]#");
lazy_re!(OLD_ROWREF, r"&[-+0-9I]");
lazy_re!(REMOTE, r"\bremote\([ \t]*([^,)]+)[ \t]*,[ \t]*([^\n)]+)\)");
lazy_re!(SIMPLE_RANGE, r"\$(([-+])?[0-9]+)\.\.\$(([-+])?[0-9]+)");
lazy_re!(FIELD_REF, r"\$(([-+])?[0-9]+)");
lazy_re!(TIME_ONLY, r"^[0-9]+:[0-9]+(?::[0-9]+)?$");
lazy_re!(TS_ACTIVE_ANY, r"<([0-9]{4}-[0-9]{2}-[0-9]{2} ?[^\]\r\n>]*?)>");

/// The parsed mode string of a formula.
struct FormulaModes {
    calc: Modes,
    fmt: Option<String>,
    duration: bool,
    duration_fmt: String,
    numbers: bool,
    literal: bool,
    keep_empty: bool,
}

fn parse_modes(d: &Doc, formula: &str) -> (String, FormulaModes) {
    let mut fm = FormulaModes {
        calc: default_modes(),
        fmt: None,
        duration: false,
        duration_fmt: String::new(),
        numbers: false,
        literal: false,
        keep_empty: false,
    };
    let Some(m) = FMT_SPLIT.captures(formula) else {
        return (formula.to_owned(), fm);
    };
    let pct = d.an.local_parameters.iter().find(|(k, _)| k == "%").map_or("", |x| x.1.as_str());
    let mut fmt = format!("{pct}{}", &m[2]);
    let f = m[1].to_owned();
    while let Some(c) = FMT_NUMFLAG.captures(&fmt) {
        let n: i32 = c[2].parse().unwrap_or(0);
        match &c[1] {
            "p" => fm.calc.prec = n,
            "n" => (fm.calc.float_fmt, fm.calc.figs) = (FloatFmt::Float, n),
            "f" => (fm.calc.float_fmt, fm.calc.figs) = (FloatFmt::Fix, n),
            "s" => (fm.calc.float_fmt, fm.calc.figs) = (FloatFmt::Sci, n),
            _ => (fm.calc.float_fmt, fm.calc.figs) = (FloatFmt::Eng, n),
        }
        let r = c.get(0).unwrap().range();
        fmt.replace_range(r, "");
    }
    while let Some(c) = FMT_FLAG.find(&fmt) {
        match c.as_str() {
            "t" => {
                fm.duration = true;
                fm.numbers = true;
                fm.duration_fmt = super::opt_string("org-table-duration-custom-format", "hours");
            }
            "T" => {
                fm.duration = true;
                fm.numbers = true;
                fm.duration_fmt = String::new();
            }
            "U" => {
                fm.duration = true;
                fm.numbers = true;
                fm.duration_fmt = "hh:mm".into();
            }
            "N" => fm.numbers = true,
            "L" => fm.literal = true,
            "E" => fm.keep_empty = true,
            "D" => fm.calc.deg = true,
            "R" => fm.calc.deg = false,
            "F" => fm.calc.frac = true,
            "S" => fm.calc.symbolic = true,
            _ => fm.calc.units = true,
        }
        let r = c.range();
        fmt.replace_range(r, "");
    }
    if nw(&fmt) {
        fm.fmt = Some(fmt);
    }
    (f, fm)
}

/// org-calc-default-modes, with the user's changes.
fn default_modes() -> Modes {
    let mut m = Modes::default();
    if let Some(v) = super::super::sexp::option("org-calc-default-modes")
        && let Some(items) = v.list()
    {
        for p in items.chunks(2) {
            let (Some(k), Some(val)) = (p[0].sym(), p.get(1)) else { continue };
            match k {
                "calc-internal-prec" => m.prec = val.int().unwrap_or(12) as i32,
                "calc-angle-mode" => m.deg = val.sym() != Some("rad"),
                "calc-prefer-frac" => m.frac = val.truthy(),
                "calc-symbolic-mode" => m.symbolic = val.truthy(),
                "calc-float-format" => {
                    if let Some(l) = val.list()
                        && let (Some(f), Some(n)) = (l.first().and_then(|x| x.sym()), l.get(1).and_then(|x| x.int()))
                    {
                        m.float_fmt = match f {
                            "fix" => FloatFmt::Fix,
                            "sci" => FloatFmt::Sci,
                            "eng" => FloatFmt::Eng,
                            _ => FloatFmt::Float,
                        };
                        m.figs = n as i32;
                    }
                }
                _ => {}
            }
        }
    }
    m
}

/// org-table-eval-formula.
#[allow(clippy::too_many_arguments)]
pub fn eval_formula(d: &mut Doc, arg: EvalArg, equation: Option<&str>, suppress_align: bool, suppress_const: bool, suppress_store: bool, suppress_analysis: bool) -> R {
    if !suppress_analysis {
        d.check_inside_data_field(false, true)?;
        analyze(d);
    }
    if arg == EvalArg::Insert {
        let eq = current_field_formula(d, false)?.map(|x| x.1).ok_or("No formula active for the current field")?;
        d.get_field(None, Some(&eq));
        d.align();
        d.st.may_need_update = true;
        return Ok(());
    }
    let mut ndown = if let EvalArg::Down(n) = arg { n } else { 1 };
    let saved_realign = d.st.no_realign;
    d.st.no_realign = true;
    let down = ndown > 1;
    let formula = match equation {
        Some(e) if suppress_store => e.to_owned(),
        Some(e) => get_formula(d, e, arg == EvalArg::Field)?,
        None => return Err("No formula".into()),
    };
    let n0 = d.current_column();
    let (formula, fm) = parse_modes(d, &formula);
    let mut formula = formula;
    if !suppress_const && opt_bool("org-table-formula-use-constants", true) {
        formula = substitute_names(d, &formula);
    }
    let orig = formula.clone();
    let formula = handle_first_last_rc(&formula, &d.an)?;
    let result = (|| -> R {
        while ndown > 0 {
            static SEP: LazyLock<Regex> = LazyLock::new(|| re(r" *\| *"));
            let mut fields = split_string(trim(d.line()), &SEP);
            if fm.duration {
                fields = fields.iter().map(|f| time_string_to_seconds(f)).collect();
            }
            if fm.numbers {
                fields = fields.iter().map(|x| if nw(x) { lisp::num_to_string(&lisp::string_to_number(x)) } else { x.clone() }).collect();
            }
            ndown -= 1;
            let mut form = formula.clone();
            let lispp = if form.len() > 2 && form.starts_with("'(") {
                if fm.literal { Lispp::Literal } else { Lispp::Lisp }
            } else {
                Lispp::Calc
            };
            while let Some(m) = ROWCOL_HASH.find(&form) {
                let v = if form[m.start()..].starts_with('@') { d.current_dline() } else { d.current_column() };
                form.replace_range(m.range(), &v.to_string());
            }
            if OLD_ROWREF.is_match(&form) {
                return Err("Formula contains old &row reference, please rewrite using @-syntax".into());
            }
            form = remote_reference_indirection(d, &form)?;
            let conv = |r: Refv| -> Refv {
                if !fm.duration {
                    return r;
                }
                match r {
                    Refv::One(s) => Refv::One(time_string_to_seconds(&s)),
                    Refv::Many(v) => Refv::Many(v.iter().map(|x| time_string_to_seconds(x)).collect()),
                }
            };
            while let Some(m) = REMOTE.captures(&form) {
                let r = m.get(0).unwrap().range();
                let rv = get_remote_range(d, m[1].trim(), m[2].trim())?;
                let rep = make_reference(&conv(rv), fm.keep_empty, fm.numbers, lispp);
                form.replace_range(r, &rep);
            }
            loop {
                let Some(m) = RANGE_RE.find(&form) else { break };
                if m.as_str().len() <= 1 {
                    break;
                }
                let desc = m.as_str().to_owned();
                let rg = get_range(d, &desc, Some(n0))?;
                let rep = make_reference(&conv(rg), fm.keep_empty, fm.numbers, lispp);
                if rep.contains(&form) {
                    return Err(format!("Spreadsheet error: invalid reference \"{form}\""));
                }
                form.replace_range(m.range(), &rep);
            }
            while let Some(m) = SIMPLE_RANGE.captures(&form) {
                let a: i64 = m[1].trim_start_matches('+').parse().unwrap_or(0) + if m.get(2).is_some() { n0 as i64 } else { 0 };
                let b: i64 = m[3].trim_start_matches('+').parse().unwrap_or(0) + if m.get(4).is_some() { n0 as i64 } else { 0 };
                let from = (a - 1).max(0) as usize;
                let to = (b.max(0) as usize).min(fields.len());
                let slice = if from < to { fields[from..to].to_vec() } else { vec![] };
                let rep = make_reference(&Refv::Many(slice), fm.keep_empty, fm.numbers, lispp);
                let r = m.get(0).unwrap().range();
                form.replace_range(r, &rep);
            }
            let form0 = form.clone();
            while let Some(m) = FIELD_REF.captures(&form) {
                let n: i64 = m[1].trim_start_matches('+').parse::<i64>().unwrap_or(0) + if m.get(2).is_some() { n0 as i64 } else { 0 };
                let idx = if n == 0 { n0 as i64 } else { n.max(1) } - 1;
                let x = fields.get(idx.max(0) as usize).cloned();
                let rep = x.as_ref().map(|x| make_reference(&Refv::One(conv(Refv::One(x.clone())).one()), fm.keep_empty, fm.numbers, lispp));
                match rep {
                    Some(rep) if !rep.contains(&formula) || formula.is_empty() => {
                        let r = m.get(0).unwrap().range();
                        form.replace_range(r, &rep);
                    }
                    _ => return Err(format!("Invalid field specifier \"{}\"", &m[0])),
                }
            }
            let ev: Result<String, String> = if lispp != Lispp::Calc {
                lisp::eval_formula(&form).map(|v| {
                    if fm.duration {
                        time_seconds_to_string(lisp::string_to_number(&v).f(), &fm.duration_fmt)
                    } else {
                        v
                    }
                })
            } else {
                let cform = localize_timestamps(&form);
                let r = if fm.duration && TIME_ONLY.is_match(&cform) {
                    Ok(cform.clone())
                } else {
                    calc::eval_full(&cform, &fm.calc, !fm.keep_empty && fm.numbers).map(|x| x.0)
                };
                r.map(|ev| {
                    if fm.duration && !ev.is_empty() {
                        let secs = if TIME_ONLY.is_match(&ev) { lisp::string_to_number(&time_string_to_seconds(&ev)).f() } else { lisp::string_to_number(&ev).f() };
                        time_seconds_to_string(secs, &fm.duration_fmt)
                    } else {
                        ev
                    }
                })
            };
            if d.st.formula_debug {
                let entry = format!(
                    "Substitution history of formula\nOrig:   {orig}\n$xyz->  {formula}\n@r$c->  {form0}\n$1->    {form}\n{}",
                    match &ev {
                        Ok(v) => format!(
                            "Result: {v}\nFormat: {}\nFinal:  {}",
                            fm.fmt.as_deref().unwrap_or("NONE"),
                            match &fm.fmt {
                                Some(f) => lisp::format_number(f, v).unwrap_or_else(|e| e),
                                None => v.clone(),
                            }
                        ),
                        Err(e) => format!("Error:  {e}"),
                    }
                );
                d.st.debug_log.push(entry.clone());
                d.message(entry.replace('\n', " | "));
            }
            let (ev, fmt) = match ev {
                Ok(v) => (v, fm.fmt.clone()),
                Err(_) => ("#ERROR".to_owned(), None),
            };
            let shown = match &fmt {
                Some(f) => lisp::format_number(f, &ev).unwrap_or_else(|_| "#ERROR".into()),
                None => TS_ACTIVE_ANY.replace_all(&ev, "[$1]").into_owned(),
            };
            let field_fmt = super::opt_string("org-table-formula-field-format", "%s");
            let shown = lisp::format(&field_fmt, &[lisp::str_(&shown)]).unwrap_or(shown);
            justify_field_maybe(d, Some(&shown));
            if down && ndown > 0 && d.l + 1 < d.n() && is_dataline(&d.lines[d.l + 1]) {
                next_row(d)?;
            } else {
                ndown = 0;
            }
        }
        Ok(())
    })();
    d.st.no_realign = saved_realign;
    result?;
    if down {
        maybe_recalculate_line(d)?;
    }
    if !suppress_align && d.st.may_need_update {
        d.align();
    }
    Ok(())
}

impl Refv {
    fn one(self) -> String {
        match self {
            Refv::One(s) => s,
            Refv::Many(v) => v.join(" "),
        }
    }
}

lazy_re!(REMOTE_INDIRECT, r"\bremote\([ \t]*([@$][^ \t,]+)[ \t]*,[ \t]*([^\n)]+)\)");

/// org-table-remote-reference-indirection.
fn remote_reference_indirection(d: &mut Doc, form: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut last = 0;
    for m in REMOTE_INDIRECT.captures_iter(form) {
        let g = m.get(1).unwrap();
        let eq = handle_first_last_rc(g.as_str(), &d.an)?;
        static COL: LazyLock<Regex> = LazyLock::new(|| re(r"^\$[0-9]+$"));
        let desc = if COL.is_match(&eq) { format!("@0{eq}") } else { eq };
        let v = match get_range(d, &desc, None)? {
            Refv::One(s) => s,
            Refv::Many(v) => v.join(" "),
        };
        out.push_str(&form[last..g.start()]);
        out.push_str(&v);
        last = g.end();
    }
    out.push_str(&form[last..]);
    Ok(out)
}

/// org-table-get-remote-range: a field or range of the table named
/// NAME-OR-ID (`#+NAME:`) or under the entry with that ID.
pub fn get_remote_range(d: &mut Doc, name_or_id: &str, form: &str) -> Result<Refv, String> {
    let form = convert_refs_to_rc(form);
    let name_re = re(&format!(r"(?i)^[ \t]*#\+(tbl)?name:[ \t]*{}[ \t]*$", regex::escape(name_or_id)));
    let loc = d.lines.iter().position(|l| name_re.is_match(l)).or_else(|| {
        let id_re = re(&format!(r"(?i)^[ \t]*:ID:[ \t]+{}[ \t]*$", regex::escape(name_or_id)));
        d.lines.iter().position(|l| id_re.is_match(l))
    });
    let Some(loc) = loc else {
        return Err(format!("Can't find remote table \"{name_or_id}\""));
    };
    let mut tl = None;
    for l in loc + 1..d.n() {
        if super::super::syntax::level(&d.lines[l]).is_some() {
            break;
        }
        if super::doc::is_table_line(&d.lines[l]) {
            tl = Some(l);
            break;
        }
    }
    let Some(tl) = tl else {
        return Err(format!("Cannot find a table at NAME or ID {name_or_id}"));
    };
    let (l0, c0) = (d.l, d.c);
    let saved = std::mem::take(&mut d.an);
    d.goto(tl, 0);
    analyze(d);
    let r = (|| {
        let f = handle_first_last_rc(&form, &d.an)?;
        let f = substitute_names(d, &f);
        match RANGE_RE.find(&f) {
            Some(m) if m.as_str().len() > 1 => {
                let desc = m.as_str().to_owned();
                get_range(d, &desc, Some(1))
            }
            _ => Ok(Refv::One(f)),
        }
    })();
    d.an = saved;
    d.goto(l0, c0);
    r
}

/// How much of the table org-table-recalculate covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recalc {
    Line,
    All,
    Iterate,
}

lazy_re!(CALC_MARK, r"^[ \t]*\| *[!$^_#*] *(\||$)");
lazy_re!(RECALC_MARK, r"^[ \t]*\| *[#*] *(\||$)");
lazy_re!(SPECIAL_FIRST, r"^ *[_^!$/] *$");

/// org-table-goto-field.
fn goto_field(d: &mut Doc, reference: &str, create: Option<&dyn Fn(usize) -> Result<bool, String>>) -> R {
    let (line, column) = if let Some((_, l, c)) = d.an.named_fields.iter().find(|(n, ..)| n == reference) {
        (*l, *c)
    } else if let Some(m) = super::edit::field_spec_re().captures(reference) {
        let r: usize = m[1].parse().unwrap_or(0);
        let l = *d.an.dlines.get(r).filter(|_| r > 0).ok_or_else(|| format!("Invalid row number in {reference}"))?;
        (l, m[2].parse().unwrap_or(1))
    } else {
        return Err(format!("Unknown field: {reference}"));
    };
    let force = match create {
        Some(f) => f(column)?,
        None => false,
    };
    d.goto(d.an.begin + line, 0);
    d.goto_column(column, false, force);
    Ok(())
}

/// org-table-recalculate.
pub fn recalculate(d: &mut Doc, all: Recalc, noalign: bool) -> R {
    let cmd = d.this_command.clone();
    if !d.st.recalc_commands.contains(&cmd) {
        d.st.recalc_commands.push(cmd);
    }
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    if all == Recalc::Iterate {
        return iterate(d, None);
    }
    analyze(d);
    let mut eqlist = get_stored_formulas(d, false, None)?;
    eqlist.sort_by(|a, b| a.0.cmp(&b.0));
    let table_beg = d.an.begin;
    let table_end = d.table_end();
    let mut cnt = 0;
    if !eqlist.is_empty() {
        let (save_l, save_col) = (d.l, d.current_column());
        let an = d.an.clone();
        let mut eqlcol = vec![];
        let mut eqlfield = vec![];
        static HLREL: LazyLock<Regex> = LazyLock::new(|| re(r"^@-?I+"));
        static COLFIRST: LazyLock<Regex> = LazyLock::new(|| re(r"^\$[<>]"));
        static COLNUM: LazyLock<Regex> = LazyLock::new(|| re(r"^\$[0-9]+$"));
        for (old_lhs, rhs) in &eqlist {
            let rhs = substitute_names(d, &handle_first_last_rc(rhs, &an)?);
            let lhs = if HLREL.is_match(old_lhs) {
                return Err("Can't assign to hline relative reference".into());
            } else if COLFIRST.is_match(old_lhs) {
                let new = handle_first_last_rc(old_lhs, &an)?;
                if eqlist.iter().any(|(k, _)| *k == new) {
                    return Err(format!("\"{old_lhs}=\" formula tries to overwrite existing formula for column {new}"));
                }
                new
            } else {
                handle_first_last_rc(old_lhs, &an)?
            };
            if COLNUM.is_match(&lhs) {
                eqlcol.push((lhs, rhs));
            } else {
                eqlfield.push((lhs, rhs));
            }
        }
        let eqlfield = expand_lhs_ranges(d, eqlfield)?;
        let (beg, end, line_re): (usize, usize, &Regex) = if all == Recalc::All {
            if (table_beg..table_end).any(|l| CALC_MARK.is_match(&d.lines[l])) {
                (table_beg, table_end, &RECALC_MARK)
            } else {
                static DL: LazyLock<Regex> = LazyLock::new(|| re(r"^[ \t]*\|[^-]|^[ \t]*\|$"));
                let first = (table_beg..table_end).find(|&l| is_dataline(&d.lines[l]));
                let h = first.and_then(|f| (f..table_end).find(|&l| is_hline(&d.lines[l])));
                let b = h.and_then(|h| (h..table_end).find(|&l| is_dataline(&d.lines[l])));
                (b.unwrap_or(table_beg), table_end, &DL)
            }
        } else {
            static DL: LazyLock<Regex> = LazyLock::new(|| re(r"^[ \t]*\|[^-]|^[ \t]*\|$"));
            (d.l, d.l + 1, &DL)
        };
        let current_line = d.l - table_beg;
        let mut seen: Vec<String> = vec![];
        let mut untouchable: HashSet<(usize, usize)> = HashSet::new();
        for (name, _, _) in &eqlfield {
            let location = d.an.named_fields.iter().find(|(n, ..)| n == name).cloned();
            let eq_line = match &location {
                Some((_, l, _)) => Some(*l),
                None => {
                    static RNUM: LazyLock<Regex> = LazyLock::new(|| re(r"^@([0-9]+)"));
                    RNUM.captures(name).and_then(|m| d.an.dlines.get(m[1].parse::<usize>().unwrap_or(0)).copied())
                }
            };
            let reference = match &location {
                Some((_, l, c)) => format!("@{}${}", line_to_dline(&d.an.dlines, *l, false).unwrap_or(0), c),
                None => name.clone(),
            };
            if seen.contains(&reference) {
                return Err(format!("Several field/range formulas try to set {reference}"));
            }
            seen.push(reference);
            if all != Recalc::Line || eq_line == Some(current_line) {
                goto_field(d, name, None)?;
                untouchable.insert((d.l, d.current_column()));
            }
        }
        // Column formulas, skipping fields covered by field formulas.
        let mut l = beg;
        while l < end.min(d.n()) {
            if line_re.is_match(&d.lines[l]) {
                d.goto(l, 0);
                let first = d.get_field(Some(1), None);
                if !SPECIAL_FIRST.is_match(&first) {
                    cnt += 1;
                    d.st.last_recalc_line = Some(l);
                    for (lhs, rhs) in &eqlcol {
                        d.goto(l, 0);
                        let c: usize = lhs[1..].parse().unwrap_or(1);
                        d.goto_column(c, false, true);
                        if !untouchable.contains(&(l, d.current_column())) {
                            eval_formula(d, EvalArg::None, Some(rhs), true, true, true, true)?;
                        }
                    }
                }
            }
            l += 1;
        }
        // Field formulas.
        let create_opt = super::opt_string("org-table-formula-create-columns", "nil");
        for (reference, formula, _) in &eqlfield {
            d.eol();
            let column_count = d.current_column().saturating_sub(1);
            let pred = |column: usize| -> Result<bool, String> {
                if column > 1000 {
                    return Err("Formula column target too large".into());
                }
                if column <= column_count {
                    return Ok(false);
                }
                match create_opt.as_str() {
                    "t" | "warn" | "prompt" => Ok(true),
                    _ => Err("Missing columns in the table.  Aborting".into()),
                }
            };
            goto_field(d, reference, Some(&pred))?;
            eval_formula(d, EvalArg::None, Some(formula), true, true, true, true)?;
        }
        if create_opt == "warn" {
            d.message("Out-of-bounds formula added columns");
        }
        d.goto(save_l, 0);
        d.goto_column(save_col, false, false);
    }
    if !noalign {
        if d.st.may_need_update {
            d.align();
        }
        if all == Recalc::All {
            d.message(format!("Re-applying formulas to {cnt} lines... done"));
        }
    }
    if d.msg.is_none() {
        d.message("Re-applying formulas... done");
    }
    Ok(())
}

/// org-table-iterate.
pub fn iterate(d: &mut Doc, imax: Option<i64>) -> R {
    let imax = imax.unwrap_or(10);
    let snapshot = |d: &Doc| d.lines[d.table_begin()..d.table_end()].to_vec();
    let mut last = snapshot(d);
    let mut i = 0;
    while i < imax {
        i += 1;
        recalculate(d, Recalc::All, false)?;
        let this = snapshot(d);
        if this != last {
            last = this;
        } else {
            d.message(if i > 1 { format!("Convergence after {i} iterations") } else { "Table was already stable".into() });
            return Ok(());
        }
    }
    Err(format!("No convergence after {i} iterations"))
}

/// The first lines of the Org tables in the buffer (org-table-map-tables).
pub fn table_starts(d: &Doc) -> Vec<usize> {
    let mut out = vec![];
    let mut l = 0;
    while l < d.n() {
        if d.table_line_at(l) && (l == 0 || !super::doc::is_table_line(&d.lines[l - 1])) {
            out.push(l);
            l = d.end_of(l);
        } else {
            l += 1;
        }
    }
    out
}

/// org-table-recalculate-buffer-tables.
pub fn recalculate_buffer_tables(d: &mut Doc) -> R {
    let (l0, c0) = (d.l, d.c);
    for t in table_starts(d) {
        d.goto(t, 0);
        recalculate(d, Recalc::All, true)?;
        d.align();
    }
    d.goto(l0, c0);
    Ok(())
}

/// org-table-iterate-buffer-tables.
pub fn iterate_buffer_tables(d: &mut Doc) -> R {
    let (l0, c0) = (d.l, d.c);
    let imax = 10;
    let mut checksum = d.lines.clone();
    for i in 1..=imax {
        for t in table_starts(d) {
            d.goto(t, 0);
            recalculate(d, Recalc::All, true)?;
        }
        if d.lines == checksum {
            for t in table_starts(d) {
                d.goto(t, 0);
                d.align();
            }
            d.goto(l0, c0);
            d.message(format!("Convergence after {i} iterations"));
            return Ok(());
        }
        checksum = d.lines.clone();
    }
    for t in table_starts(d) {
        d.goto(t, 0);
        d.align();
    }
    d.goto(l0, c0);
    Err(format!("No convergence after {imax} iterations"))
}

/// org-table-TBLFM-begin: the first #+TBLFM line of the block at point.
pub fn tblfm_begin(d: &Doc) -> Option<usize> {
    // Search backward from the end of the next line for a table line
    // followed by a TBLFM line.
    let start = (d.l + 1).min(d.n() - 1);
    (1..=start).rev().find(|&l| is_tblfm(&d.lines[l]) && super::doc::is_table_line(&d.lines[l - 1]))
}

/// org-table-calc-current-TBLFM.
pub fn calc_current_tblfm(d: &mut Doc, all: Recalc) -> R {
    if !d.at_tblfm() {
        return Err("Not at a #+TBLFM line".into());
    }
    let formula = d.line().to_owned();
    let (l0, c0) = (d.l, d.c);
    let at = tblfm_begin(d).ok_or("Not at a #+TBLFM line")?;
    d.lines.insert(at, formula);
    d.goto(at - 1, 0);
    let r = recalculate(d, all, false);
    d.lines.remove(at);
    d.goto(l0, c0);
    r
}

/// org-table-field-info's message.
pub fn field_info(d: &mut Doc) -> R {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    analyze(d);
    let col = d.current_column();
    let cname = d.an.column_names.iter().find(|(_, c)| *c == col.to_string()).map(|x| x.0.clone());
    let line = d.l - d.an.begin;
    let name = d.an.named_fields.iter().find(|(_, l, c)| *l == line && *c == col).map(|x| x.0.clone());
    let stored = get_stored_formulas(d, true, None)?;
    let an = d.an.clone();
    let eqs: Vec<(String, String)> = stored.into_iter().map(|(k, v)| (handle_first_last_rc(&k, &an).unwrap_or(k), v)).collect();
    let eql = expand_lhs_ranges(d, eqs)?;
    let dline = d.current_dline();
    let rf = format!("@{dline}${col}");
    let ref1 = convert_refs_to_an(&rf);
    let fequation = name.as_ref().and_then(|n| eql.iter().find(|(k, ..)| k == n)).or_else(|| eql.iter().find(|(k, ..)| *k == rf));
    let cequation = eql.iter().find(|(k, ..)| *k == format!("${col}"));
    let eqn = fequation.or(cequation).map(|(k, v, orig)| match orig {
        Some(o) => {
            let (k, v) = o.split_once('=').unwrap();
            (k.to_owned(), v.to_owned())
        }
        None => (k.clone(), v.clone()),
    });
    let formula = eqn.map_or(String::new(), |(k, v)| {
        let pre = if k.starts_with('$') || k.starts_with('@') { "" } else { "$" };
        format!(", formula: {}", to_user(&format!("{pre}{k}={v}")))
    });
    d.message(format!(
        "line @{dline}, col ${col}{}, ref @{dline}${col} or {ref1}{}{formula}",
        cname.map_or(String::new(), |c| format!(" or ${c}")),
        name.map_or(String::new(), |n| format!(" or ${n}"))
    ));
    Ok(())
}

/// org-table-show-reference: describe the reference in `formula` at
/// byte `pos` (Fred has no rectangle highlighting; the referenced cells
/// are named in the echo area).
pub fn show_reference(d: &mut Doc, formula: &str, pos: usize) -> R {
    analyze(d);
    static RANGE2: LazyLock<Regex> =
        LazyLock::new(|| re(r"(@[-0-9I$&]+|[a-zA-Z]{1,2}([0-9]+|&)|\$[a-zA-Z0-9]+)\.\.(@?[-0-9I$&]+|[a-zA-Z]{1,2}([0-9]+|&)|\$[a-zA-Z0-9]+)"));
    static TRANSLATE: LazyLock<Regex> = LazyLock::new(|| re(r"(@[-0-9I$]+|[a-zA-Z]{1,2}([0-9]+|&))"));
    static NAMEREF: LazyLock<Regex> = LazyLock::new(|| re(r"\$[a-zA-Z][a-zA-Z0-9]*"));
    static COLREF: LazyLock<Regex> = LazyLock::new(|| re(r"\$[0-9]+"));
    let at = |r: &Regex| r.find_iter(formula).find(|m| m.start() <= pos && pos <= m.end()).map(|m| m.as_str().to_owned());
    if let Some(m) = at(&RANGE2).or_else(|| at(&TRANSLATE)).or_else(|| at(&RANGE_RE)) {
        let mut desc = convert_refs_to_rc(&m);
        if !desc.starts_with('@') {
            desc = format!("@{desc}");
        }
        let desc = substitute_names(d, &desc);
        let v = get_range(d, &desc, None)?;
        d.message(format!(
            "{m}: {}",
            match v {
                Refv::One(s) => s,
                Refv::Many(v) => format!("[{}]", v.join(", ")),
            }
        ));
        return Ok(());
    }
    if let Some(m) = at(&NAMEREF) {
        let var = &m[1..];
        if let Some((_, l, c)) = d.an.named_fields.iter().find(|(n, ..)| n == var) {
            d.message(format!("Named field, column {c} of line {l}"));
        } else if let Some((_, c)) = d.an.column_names.iter().find(|(n, _)| n == var) {
            d.message(format!("Named column (column {c})"));
        } else if d.an.local_parameters.iter().any(|(n, _)| n == var) {
            d.message("Local parameter.");
        } else if let Some((_, v)) = buffer_constants(&d.lines).into_iter().find(|(k, _)| k == var) {
            d.message(format!("Local Constant: ${var}={v} in #+CONSTANTS line."));
        } else {
            let v = get_constant(d, var);
            if v == "#UNDEFINED_NAME" {
                return Err(format!("Undefined name ${var}"));
            }
            d.message(format!("Constant: ${var}={v} in `org-table-formula-constants'."));
        }
        return Ok(());
    }
    if let Some(m) = at(&COLREF) {
        d.message(format!("Column {}", &m[1..]));
        return Ok(());
    }
    Err("No reference at point".into())
}

// ---- formula editor support ----

/// org-table-shift-refpart.
pub fn shift_refpart(r: &str, decr: bool, hline: bool) -> Result<String, String> {
    let (sign, rest) = match r.chars().next() {
        Some(c @ ('-' | '+')) => (Some(c), &r[1..]),
        _ => (None, r),
    };
    if hline && rest.starts_with('I') {
        let mut n: i64 = rest.chars().take_while(|c| *c == 'I').count() as i64 * if sign == Some('-') { -1 } else { 1 };
        n += if decr { -1 } else { 1 };
        if n == 0 {
            n += if decr { -1 } else { 1 };
        }
        return Ok(match sign {
            Some(_) => format!("{}{}", if n < 0 { '-' } else { '+' }, "I".repeat(n.unsigned_abs() as usize)),
            None => "I".repeat(n.max(1) as usize),
        });
    }
    if rest.starts_with(|c: char| c.is_ascii_digit()) {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let mut n: i64 = digits.parse().unwrap_or(0) * if sign == Some('-') { -1 } else { 1 };
        n += if decr { -1 } else { 1 };
        return Ok(match sign {
            Some(_) => format!("{}{}", if n < 0 { '-' } else { '+' }, n.abs()),
            None => n.max(1).to_string(),
        });
    }
    if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        let letters: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
        let n = letters_to_number(&letters) as i64 + if decr { -1 } else { 1 };
        return Ok(number_to_letters(n.max(1) as usize));
    }
    Err("Cannot shift reference".into())
}

/// org-table-fedit-shift-reference on `s` at byte `pos`: the new text.
pub fn shift_reference(s: &str, pos: usize, dir: char) -> Result<String, String> {
    static AMP: LazyLock<Regex> = LazyLock::new(|| re(r"\b([a-zA-Z])&"));
    static A1: LazyLock<Regex> = LazyLock::new(|| re(r"\b([a-zA-Z]{1,2})([0-9]+)"));
    static INTERNAL: LazyLock<Regex> = LazyLock::new(|| re(r"(@|\.\.)([-+]?(I+\b|[0-9]+))(\$([-+]?[0-9]+))?"));
    let find = |r: &Regex| r.captures_iter(s).find(|c| {
        let m = c.get(0).unwrap();
        m.start() <= pos && pos <= m.end()
    });
    let vertical = matches!(dir, 'u' | 'd');
    let decr = matches!(dir, 'u' | 'l');
    let replace = |c: &Captures, g: usize, hline: bool| -> Result<String, String> {
        let m = c.get(g).ok_or("Cannot shift reference in this direction")?;
        let new = shift_refpart(m.as_str(), decr, hline)?;
        Ok(format!("{}{}{}", &s[..m.start()], new, &s[m.end()..]))
    };
    if let Some(c) = find(&AMP) {
        if vertical {
            return Err("Cannot shift reference in this direction".into());
        }
        return replace(&c, 1, false);
    }
    if let Some(c) = find(&A1) {
        return replace(&c, if vertical { 2 } else { 1 }, false);
    }
    if let Some(c) = find(&INTERNAL) {
        return if vertical { replace(&c, 2, c.get(3).is_some_and(|m| m.as_str().starts_with('I'))) } else { replace(&c, 5, false) };
    }
    Err("No reference at point".into())
}

/// The formula editor's text for the stored formulas: one `lhs = rhs`
/// per line, under section titles (org-table-edit-formulas).
pub fn fedit_lines(formulas: &[(String, String)]) -> Vec<String> {
    static COL: LazyLock<Regex> = LazyLock::new(|| re(r"^\$([0-9]+|[<>]+)$"));
    let mut out = vec![];
    let mut titles = vec![("column", "# Column Formulas"), ("field", "# Field and Range Formulas"), ("named", "# Named Field Formulas")];
    for (k, v) in formulas {
        let ty = if COL.is_match(k) {
            "column"
        } else if k.starts_with('@') {
            "field"
        } else {
            "named"
        };
        if let Some(i) = titles.iter().position(|(t, _)| *t == ty) {
            if !out.is_empty() {
                out.push(String::new());
            }
            out.push(titles[i].1.to_owned());
            titles.remove(i);
        }
        let pre = if k.starts_with('@') || k.starts_with('$') { "" } else { "$" };
        out.push(format!("{pre}{k} = {v}"));
    }
    out
}

/// org-table-fedit-finish's parse of the editor text.
pub fn fedit_parse(text: &str) -> Result<Vec<(String, String)>, String> {
    static DEF: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^(@[-+I<>0-9.$@]+|@?[0-9]+|\$([a-zA-Z0-9]+|[<>]+)) *= *(.*(\n[ \t]+.*)*)"));
    let mut eql: Vec<(String, String)> = vec![];
    for c in DEF.captures_iter(text) {
        let var = c[1].to_owned();
        let form = trim(&c[3]).to_owned();
        if form.is_empty() {
            continue;
        }
        static WS: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]*\n[ \t]*"));
        let form = WS.replace_all(&form, " ").into_owned();
        if eql.iter().any(|(k, _)| *k == var) {
            return Err(format!("Double formulas for {var}"));
        }
        eql.insert(0, (var, form));
    }
    Ok(eql)
}
