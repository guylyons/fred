//! Column view: a port of org-colview.el.
//!
//! Fred adaptation: Emacs draws column view as overlays on the headings.
//! Fred shows it as a generated read-only buffer, `*Org Columns: FILE*`
//! (like a Magit view): a title line, then one row per entry of the
//! scope. Keys of org-columns-map edit the underlying entries through the
//! session and rebuild the view. The capture (formats, summaries, values)
//! is shared with the `columnview` dynamic block and exposed for the
//! agenda's column view ([`entry_values`], [`summarize`]).

use super::clock::{self, Marker};
use super::dblock::{self, Out, Params};
use super::props::{self, Doc, Inherit};
use super::sexp::{self, Sexp};
use super::syntax::{self, Lines, Settings};
use super::table::lisp::{self, L, Num};
use super::time::civil;
use super::{Prefix, fold, line, options};
use crate::editor::Editor;
use crate::session::Session;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

// ---- column specifications ----

/// A compiled column (org-columns--spec).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The property, upper case.
    pub property: String,
    pub title: String,
    pub width: Option<usize>,
    pub operator: Option<String>,
    pub format: Option<String>,
}

/// org-columns-compile-format.
pub fn compile_format(fmt: &str) -> Vec<Spec> {
    let re = crate::org_re!(r"%([0-9]+)?([[:alnum:]_-]+)(?:\(([^)]*)\))?(?:\{([^}]*)\})?\s*");
    re.captures_iter(fmt)
        .map(|c| {
            let prop = c[2].to_owned();
            let title = c
                .get(3)
                .map(|m| m.as_str())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(&prop)
                .to_owned();
            let width = c.get(1).and_then(|m| m.as_str().parse().ok());
            let (operator, format) = match c
                .get(4)
                .map(|m| m.as_str())
                .filter(|o| !o.trim().is_empty())
            {
                None => (None, None),
                Some(op) => {
                    let mut it = op.split(';');
                    let o = it.next().unwrap_or("").to_owned();
                    let f = it.next().map(str::to_owned);
                    let f = if o == "$" { Some("%.2f".to_owned()) } else { f };
                    (Some(o), f)
                }
            };
            Spec {
                property: prop.to_uppercase(),
                title,
                width,
                operator,
                format,
            }
        })
        .collect()
}

/// org-columns-uncompile-format.
pub fn uncompile_format(specs: &[Spec]) -> String {
    specs
        .iter()
        .map(|s| {
            let mut out = format!(
                "%{}{}",
                s.width.map(|w| w.to_string()).unwrap_or_default(),
                s.property
            );
            if s.title != s.property {
                out.push_str(&format!("({})", s.title));
            }
            match (&s.operator, &s.format) {
                (None, _) => {}
                (Some(o), _) if o == "$" => out.push_str("{$}"),
                (Some(o), Some(f)) => out.push_str(&format!("{{{o};{f}}}")),
                (Some(o), None) => out.push_str(&format!("{{{o}}}")),
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- summaries ----

fn lisp_num(n: Num) -> L {
    match n {
        Num::I(i) => L::Int(i),
        Num::F(f) => L::Float(f),
    }
}

/// `(format (or FMT "%s") N)`.
fn fmt_num(fmt: Option<&str>, n: Num) -> String {
    lisp::format(fmt.unwrap_or("%s"), &[lisp_num(n)]).unwrap_or_else(|_| lisp::num_to_string(&n))
}

fn numbers(values: &[String]) -> Vec<Num> {
    values.iter().map(|v| lisp::string_to_number(v)).collect()
}

fn sum(values: &[String]) -> Num {
    numbers(values).iter().fold(Num::I(0), |a, b| a.add(b))
}

/// The time the summaries of ages count from (org-columns--time).
fn age_to_minutes(s: &str) -> Result<f64, String> {
    if crate::org_re!(r"<([0-9]{4}-[0-9]{2}-[0-9]{2}[^\r\n>]*?)>").is_match(s) {
        let t = super::time::stamp::time_string_to_seconds(s)?;
        return Ok((civil::now_epoch() - t) as f64 / 60.0);
    }
    if super::time::duration::is_duration(s) {
        return super::time::duration::to_minutes(s, true);
    }
    Err(format!("Invalid age: {s:?}"))
}

fn format_age(m: f64) -> String {
    use super::time::duration::{Format, Item, from_minutes};
    let f = Format::Units(vec![
        Item::Unit("d".into(), false),
        Item::Unit("h".into(), false),
        Item::Unit("min".into(), false),
    ]);
    from_minutes(m, Some(&f), true).unwrap_or_default()
}

fn apply_times(times: &[String], f: impl Fn(&[f64]) -> f64) -> String {
    let mins: Vec<f64> = times
        .iter()
        .map(|t| super::time::duration::to_minutes(t, false).unwrap_or(0.0))
        .collect();
    let refs: Vec<&str> = times.iter().map(String::as_str).collect();
    let fmt = super::time::duration::hmm_only(&refs);
    super::time::duration::from_minutes(f(&mins), fmt.as_ref(), false).unwrap_or_default()
}

fn ages(values: &[String], f: impl Fn(&[f64]) -> f64) -> Result<String, String> {
    let m = values
        .iter()
        .map(|v| age_to_minutes(v))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format_age(f(&m)))
}

fn fmin(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::INFINITY, f64::min)
}

fn fmax(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// A default summary function (org-columns-summary-types-default) by
/// operator or function name.
fn builtin(name: &str, values: &[String], fmt: Option<&str>) -> Option<Result<String, String>> {
    let pick = |f: fn(f64, f64) -> bool| -> Num {
        let ns = numbers(values);
        let mut best = ns[0];
        for n in &ns[1..] {
            if f(n.f(), best.f()) {
                best = *n;
            }
        }
        best
    };
    Some(Ok(match name {
        "+" | "org-columns--summary-sum" => fmt_num(fmt, sum(values)),
        "$" | "org-columns--summary-currencies" => format!("{:.2}", sum(values).f()),
        "X" | "org-columns--summary-checkbox" => {
            let done = values.iter().filter(|v| *v == "[X]").count();
            if done == values.len() {
                "[X]".into()
            } else if done > 0 {
                "[-]".into()
            } else {
                "[ ]".into()
            }
        }
        "X/" | "org-columns--summary-checkbox-count" => {
            let re = crate::org_re!(r"\[([1-9][0-9]*)/([0-9]+)\]");
            let done = values
                .iter()
                .filter(|v| *v == "[X]" || re.captures(v).is_some_and(|c| c[1] == c[2]))
                .count();
            format!("[{done}/{}]", values.len())
        }
        "X%" | "org-columns--summary-checkbox-percent" => {
            let done = values
                .iter()
                .filter(|v| *v == "[X]" || *v == "[100%]")
                .count();
            super::todo::percent_cookie(done, values.len())
        }
        "min" | "org-columns--summary-min" => fmt_num(fmt, pick(|a, b| a < b)),
        "max" | "org-columns--summary-max" => fmt_num(fmt, pick(|a, b| a > b)),
        "mean" | "org-columns--summary-mean" => {
            fmt_num(fmt, Num::F(sum(values).f() / values.len() as f64))
        }
        ":" | "org-columns--summary-sum-times" => apply_times(values, |v| v.iter().sum()),
        ":min" | "org-columns--summary-min-time" => apply_times(values, fmin),
        ":max" | "org-columns--summary-max-time" => apply_times(values, fmax),
        ":mean" | "org-columns--summary-mean-time" => apply_times(values, mean),
        "@min" | "org-columns--summary-min-age" => return Some(ages(values, fmin)),
        "@max" | "org-columns--summary-max-age" => return Some(ages(values, fmax)),
        "@mean" | "org-columns--summary-mean-age" => return Some(ages(values, mean)),
        "est+" | "org-columns--summary-estimate" => {
            let (mut m, mut var) = (0.0, 0.0);
            for e in values {
                let parts: Vec<f64> = e
                    .split('-')
                    .map(|p| lisp::string_to_number(p).f())
                    .collect();
                match parts.as_slice() {
                    [low, high] => {
                        let mid = (low + high) / 2.0;
                        m += mid;
                        var += (low * low + high * high) / 2.0 - mid * mid;
                    }
                    [v] => m += v,
                    _ => {}
                }
            }
            let sd = var.sqrt();
            format!("{:.0}-{:.0}", m - sd, m + sd)
        }
        _ => return None,
    }))
}

/// The summarize and collect functions of OPERATOR
/// (org-columns--summary-type): from org-columns-summary-types, then the
/// defaults.
fn summary_type(op: &str) -> Result<(Sexp, Option<Sexp>), String> {
    if let Some(v) = sexp::option("org-columns-summary-types") {
        for e in v.list().unwrap_or(&[]) {
            if e.car().and_then(Sexp::str) != Some(op) {
                continue;
            }
            let cdr = e.cdr();
            let items = cdr.list().unwrap_or(&[]);
            if items.first().and_then(Sexp::sym) == Some("lambda") || cdr.sym().is_some() {
                return Ok((cdr, None));
            }
            return match items {
                [f, c] => Ok((f.clone(), Some(c.clone()))),
                _ => Err(format!("Invalid definition for operator {op:?}")),
            };
        }
    }
    if builtin(op, &["0".into()], None).is_some() {
        return Ok((Sexp::Sym(op.into()), None));
    }
    Err(format!("Unknown {op:?} operator"))
}

/// The summary of VALUES (non-empty strings) by OPERATOR, formatted with
/// FMT (org-columns--summarize-function).
pub fn summarize(op: &str, values: &[String], fmt: Option<&str>) -> Result<String, String> {
    let (f, _) = summary_type(op)?;
    if let Some(name) = f.sym()
        && let Some(r) = builtin(name, values, fmt)
    {
        return r;
    }
    // A Lisp summary function: run it in the formula interpreter.
    let mut i = lisp::Interp::new();
    let func = i.eval(&lisp::from_sexp(&f))?;
    let args = vec![
        lisp::list(values.iter().map(|v| lisp::str_(v)).collect()),
        fmt.map_or(L::Nil, lisp::str_),
    ];
    let r = i.apply(&func, args)?;
    Ok(r.as_str().map_or_else(|| lisp::princ(&r), str::to_owned))
}

// ---- values ----

/// org-columns--displayed-value.
pub fn displayed_value(spec: &Spec, value: &str, level: usize, no_star: bool) -> String {
    match spec.property.as_str() {
        "ITEM" => {
            let stars = if no_star {
                String::new()
            } else {
                let c = if options::bool("org-hide-leading-stars", false) {
                    " "
                } else {
                    "*"
                };
                format!("{}* ", c.repeat(level.saturating_sub(1)))
            };
            format!("{stars}{}", super::links::display_format(value))
        }
        "DEADLINE" | "SCHEDULED" | "TIMESTAMP" => {
            crate::org_re!(r"<([0-9]{4}-[0-9]{2}-[0-9]{2}[^\r\n>]*?)>")
                .replace_all(value, "[$1]")
                .into_owned()
        }
        _ => match &spec.format {
            None => value.to_owned(),
            Some(f) => fmt_num(Some(f), lisp::string_to_number(value)),
        },
    }
}

/// One cell: the value, what is shown, and whether it is a summary.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub value: String,
    pub displayed: String,
    pub computed: bool,
}

/// One entry of the view.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub line: usize,
    pub level: usize,
    pub heading: String,
    pub cells: Vec<Cell>,
}

/// org-columns--collect-values for heading `h`, with `summaries`
/// (spec index -> value) of the entry.
pub fn entry_values<L2: Lines + ?Sized>(
    b: &L2,
    st: &Settings,
    file: Option<&std::path::Path>,
    h: usize,
    specs: &[Spec],
    summaries: &HashMap<usize, String>,
    no_star: bool,
) -> Vec<Cell> {
    let doc = Doc::new(b, st, file);
    let level = syntax::level(&b.line_text(h)).unwrap_or(1);
    specs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (value, computed) = match summaries.get(&i) {
                Some(v) => (v.clone(), true),
                None => (
                    doc.get(Some(h), &s.property, Inherit::Selective, true)
                        .unwrap_or_default(),
                    false,
                ),
            };
            let displayed = displayed_value(s, &value, level, no_star);
            Cell {
                value,
                displayed,
                computed,
            }
        })
        .collect()
}

// ---- the capture ----

/// The view of a scope: format, rows, and the scope's lines.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capture {
    pub fmt: String,
    pub specs: Vec<Spec>,
    pub rows: Vec<Row>,
    pub top: usize,
    pub end: usize,
}

/// The first `#+COLUMNS:` keyword.
fn columns_keyword(ed: &Editor) -> Option<String> {
    (0..ed.line_count()).find_map(|i| {
        syntax::keyword_line(&line(ed, i))
            .filter(|(k, _)| k == "COLUMNS")
            .map(|(_, v)| v.trim().to_owned())
    })
}

/// The heading defining COLUMNS for the entry at `h` (itself or an
/// ancestor): org-entry-property-inherited-from.
fn columns_owner(ed: &Editor, h: usize) -> Option<usize> {
    let mut cur = Some(h);
    while let Some(c) = cur {
        if props::get(ed, Some(c), "COLUMNS", Inherit::No).is_some() {
            return Some(c);
        }
        cur = syntax::parent(&ed.buf, c);
    }
    None
}

/// org-columns-get-format.
pub fn get_format(ed: &Editor, l: usize, given: Option<&str>) -> String {
    let nw = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    let h = fold::back_to_heading(ed, l.min(ed.line_count().saturating_sub(1)));
    nw(given.map(str::to_owned))
        .or_else(|| nw(h.and_then(|h| props::get(ed, Some(h), "COLUMNS", Inherit::Yes))))
        .or_else(|| nw(columns_keyword(ed)))
        .unwrap_or_else(|| {
            options::string(
                "org-columns-default-format",
                "%25ITEM %TODO %3PRIORITY %TAGS",
            )
        })
}

/// org-columns--set-scope: the scope's first line and end.
pub fn scope(ed: &Editor, global: bool, l: usize) -> (usize, usize) {
    let n = ed.line_count();
    if global {
        return (0, n);
    }
    let Some(h) = fold::back_to_heading(ed, l.min(n.saturating_sub(1))) else {
        return (0, n);
    };
    let top = columns_owner(ed, h).unwrap_or(h);
    (top, syntax::subtree_end(&ed.buf, top))
}

fn reduced(st: &Settings, l: usize) -> usize {
    if l > 0 && st.opt_bool("org-odd-levels-only", false) {
        1 + l / 2
    } else {
        l
    }
}

/// org-columns-compute-all on `top..end`: the summaries per heading line
/// and spec; existing properties are updated with them (the first spec of
/// each property only).
pub fn compute_all(
    ed: &mut Editor,
    specs: &[Spec],
    top: usize,
    end: usize,
) -> HashMap<usize, HashMap<usize, String>> {
    let mut out: HashMap<usize, HashMap<usize, String>> = HashMap::new();
    let mut seen: Vec<&str> = vec![];
    for (i, s) in specs.iter().enumerate() {
        let update = !seen.contains(&s.property.as_str());
        compute_spec(ed, s, i, update, top, end, &mut out);
        seen.push(&s.property);
    }
    out
}

/// org-columns--compute-spec.
fn compute_spec(
    ed: &mut Editor,
    spec: &Spec,
    idx: usize,
    update: bool,
    top: usize,
    end: usize,
    out: &mut HashMap<usize, HashMap<usize, String>>,
) {
    let Some(op) = spec.operator.clone() else {
        return;
    };
    if props::SPECIAL.contains(&spec.property.as_str()) {
        return;
    }
    let collect = summary_type(&op).ok().and_then(|(_, c)| c);
    let st = super::settings(ed);
    let mut stack: Vec<(usize, String)> = vec![];
    let end = end.min(ed.line_count());
    for pos in (top..end).rev() {
        let Some(lvl) = syntax::level(&line(ed, pos)) else {
            continue;
        };
        let level = reduced(&st, lvl);
        let mut children = vec![];
        while stack.last().is_some_and(|(l, _)| level < *l) {
            children.push(stack.pop().unwrap().1);
        }
        let summary = if children.is_empty() {
            None
        } else {
            summarize(&op, &children, spec.format.as_deref()).ok()
        };
        let current = if update || summary.is_none() {
            match &collect {
                // ponytail: Lisp collect functions need org-entry-get in the
                // interpreter; the plain property is used instead.
                Some(_) | None => props::get(ed, Some(pos), &spec.property, Inherit::No),
            }
        } else {
            None
        };
        let value = match summary {
            Some(s) => {
                out.entry(pos).or_default().insert(idx, s.clone());
                if update
                    && let Some(cur) = &current
                    && *cur != s.trim()
                {
                    let _ = props::put(ed, Some(pos), &spec.property, s.trim());
                }
                Some(s)
            }
            None => current,
        };
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            stack.push((level, v));
        }
    }
}

/// An entry org-agenda-skip passes over: in an archived or COMMENT tree.
fn skipped(ed: &Editor, st: &Settings, h: usize) -> bool {
    let mut cur = Some(h);
    while let Some(c) = cur {
        if let Some(x) = syntax::headline(&line(ed, c), st)
            && (x.commented || x.archived())
        {
            return true;
        }
        cur = syntax::parent(&ed.buf, c);
    }
    false
}

/// org-columns--prepare-rows on the scope around line `l`.
pub fn capture(
    ed: &mut Editor,
    global: bool,
    l: usize,
    fmt: Option<&str>,
    inhibit: bool,
) -> Capture {
    let fmt = get_format(ed, l, fmt);
    let specs = compile_format(&fmt);
    let (top, end) = scope(ed, global, l);
    let summaries = if inhibit {
        HashMap::new()
    } else {
        compute_all(ed, &specs, top, end)
    };
    let st = super::settings(ed);
    let lines = super::lines(ed, 0..ed.line_count());
    let file = ed.path.clone();
    let empty = HashMap::new();
    let rows = (top..end.min(ed.line_count()))
        .filter(|&h| syntax::level(&lines[h]).is_some() && !skipped(ed, &st, h))
        .map(|h| Row {
            line: h,
            level: reduced(&st, syntax::level(&lines[h]).unwrap_or(1)),
            heading: lines[h].clone(),
            cells: entry_values(
                &lines,
                &st,
                file.as_deref(),
                h,
                &specs,
                summaries.get(&h).unwrap_or(&empty),
                false,
            ),
        })
        .collect();
    Capture {
        fmt,
        specs,
        rows,
        top,
        end,
    }
}

// ---- display ----

/// org-columns--truncate-below-width.
fn truncate_below(s: &str, w: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if dblock::string_width(&format!("{out}{c}")) > w {
            break;
        }
        out.push(c);
    }
    out
}

/// org-columns-add-ellipses.
pub fn add_ellipses(s: &str, w: usize) -> String {
    let ell = options::string("org-columns-ellipses", "..");
    let ew = dblock::string_width(&ell);
    if dblock::string_width(s) <= w {
        s.to_owned()
    } else if w <= ew {
        truncate_below(&ell, w)
    } else {
        format!("{}{ell}", truncate_below(s, w - ew))
    }
}

/// org-columns--set-widths.
pub fn widths(specs: &[Spec], rows: &[Row]) -> Vec<usize> {
    specs
        .iter()
        .enumerate()
        .map(|(i, s)| match s.width {
            Some(w) => w,
            None => rows
                .iter()
                .filter_map(|r| r.cells.get(i))
                .map(|c| unicode_width::UnicodeWidthStr::width(c.displayed.as_str()))
                .fold(
                    unicode_width::UnicodeWidthStr::width(s.title.as_str()),
                    usize::max,
                ),
        })
        .map(|w| w.max(1))
        .collect()
}

/// org-columns--cell-format-string applied to `text`.
fn cell(text: &str, width: usize, last: bool) -> String {
    let t = add_ellipses(text, width);
    let pad = width.saturating_sub(unicode_width::UnicodeWidthStr::width(t.as_str()));
    format!("{t}{} |{}", " ".repeat(pad), if last { "" } else { " " })
}

/// The view's text lines: titles, then the rows.
fn render(specs: &[Spec], widths: &[usize], rows: &[&Row]) -> Vec<String> {
    let n = specs.len();
    let line = |cells: Vec<&str>| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| cell(c, widths[i], i + 1 == n))
            .collect()
    };
    let mut out = vec![line(specs.iter().map(|s| s.title.as_str()).collect())];
    for r in rows {
        out.push(line(r.cells.iter().map(|c| c.displayed.as_str()).collect()));
    }
    out
}

/// The column index at byte `b` of a view line.
fn column_at(widths: &[usize], b: usize) -> usize {
    let mut start = 0;
    for (i, w) in widths.iter().enumerate() {
        let end = start + w + 3;
        if b < end {
            return i;
        }
        start = end;
    }
    widths.len().saturating_sub(1)
}

/// The byte where column `i` starts.
fn column_start(widths: &[usize], i: usize) -> usize {
    widths[..i.min(widths.len())].iter().map(|w| w + 3).sum()
}

// ---- the view buffer ----

/// Where the source entries live.
#[derive(Clone, Debug, PartialEq)]
enum Src {
    File(PathBuf),
    Buffer(usize),
}

/// A column view buffer's state.
#[derive(Clone, Debug)]
struct View {
    src: Src,
    file: Option<PathBuf>,
    global: bool,
    /// org-columns-begin-marker: where org-columns was called.
    begin: Marker,
    cap: Capture,
    widths: Vec<usize>,
    /// Rows hidden by `o` (org-overview): only levels up to this show.
    max_level: Option<usize>,
}

impl View {
    fn shown(&self) -> Vec<&Row> {
        self.cap
            .rows
            .iter()
            .filter(|r| self.max_level.is_none_or(|m| r.level <= m))
            .collect()
    }
}

thread_local! {
    static VIEWS: RefCell<HashMap<String, View>> = RefCell::new(HashMap::new());
}

fn view_name(ed: &Editor) -> Option<String> {
    ed.org_buffer_name
        .clone()
        .filter(|n| n.starts_with("*Org Columns"))
}

/// A column view buffer.
pub fn is_view(ed: &Editor) -> bool {
    view_name(ed).is_some_and(|n| VIEWS.with(|v| v.borrow().contains_key(&n)))
}

fn get_view(ed: &Editor) -> Result<(String, View), String> {
    let name = view_name(ed).ok_or("Not in column view")?;
    let v = VIEWS
        .with(|v| v.borrow().get(&name).cloned())
        .ok_or("Not in column view")?;
    Ok((name, v))
}

fn put_view(name: &str, v: View) {
    VIEWS.with(|m| m.borrow_mut().insert(name.to_owned(), v));
}

/// Keys Vim keeps in a column view: the leader, `:`, motions.
pub fn vim_keeps(ed: &Editor, k: crate::key::Key) -> bool {
    use crate::key::KeyCode;
    if k.alt || (k.ctrl && k.char() == Some('c')) {
        return false;
    }
    if k.ctrl {
        return true;
    }
    match k.code {
        KeyCode::Char(' ') => ed.vim.pending.is_empty(),
        KeyCode::Char(c) => "jkhlwbG$:0".contains(c) && !(c == '0' && !ed.vim.pending.is_empty()),
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => false,
        _ => true,
    }
}

/// Digit keys in a column view (org-columns-next-allowed-value NTH).
pub fn key(ed: &mut Editor, k: crate::key::Key) -> bool {
    if !is_view(ed) || ed.mode != crate::editor::Mode::Normal || !ed.org_keys.is_empty() {
        return false;
    }
    match k.char() {
        Some(c @ '1'..='9') if !k.ctrl && !k.alt && ed.vim.pending.is_empty() => {
            let r = next_allowed(ed, false, Some(c.to_digit(10).unwrap() as usize));
            if let Err(e) = r {
                ed.set_err(e);
            }
            true
        }
        _ => false,
    }
}

/// Run `f` on the view's source buffer (in the session).
fn with_src<R>(s: &mut Session, src: &Src, f: impl FnOnce(&mut Editor) -> R) -> Result<R, String> {
    match src {
        Src::File(p) => s.org_with_file(p, f),
        Src::Buffer(i) if *i < s.org_buffer_count() => Ok(s.org_with_buffer(*i, f)),
        Src::Buffer(_) => Err("The column view's buffer is gone".into()),
    }
}

/// The view's text and cursor after a capture.
fn install(s: &mut Session, name: &str, v: View, cursor: (usize, usize)) {
    let text = render(&v.cap.specs, &v.widths, &v.shown()).join("\n");
    put_view(name, v);
    s.org_temp_buffer(name, &text, false);
    let n = s.ed.line_count();
    s.ed.set_cursor(cursor.0.min(n - 1), cursor.1);
    s.ed.clamp_cursor();
}

/// org-columns-redo: capture again from the source and show it.
fn redo(s: &mut Session, name: &str, inhibit: bool) {
    let Some(mut v) = VIEWS.with(|m| m.borrow().get(name).cloned()) else {
        return;
    };
    let cursor = (s.ed.cur.line, s.ed.cur.byte);
    let (global, fmt, begin) = (v.global, v.cap.fmt.clone(), v.begin.clone());
    let r = with_src(s, &v.src, move |e| {
        let l = clock::locate(e, &begin).unwrap_or(begin.line);
        let cap = capture(e, global, l, Some(&fmt), inhibit);
        (cap, l)
    });
    match r {
        Ok((cap, l)) => {
            v.begin.line = l;
            v.widths = widths(&cap.specs, &cap.rows);
            v.cap = cap;
            install(s, name, v, cursor);
        }
        Err(e) => s.ed.set_err(e),
    }
}

/// org-columns: show column view for the scope at the cursor (the whole
/// buffer with `global`).
pub fn columns(ed: &mut Editor, global: bool, fmt: Option<String>) -> Result<(), String> {
    let l = ed.cur.line;
    let cap = capture(ed, global, l, fmt.as_deref(), false);
    if cap.rows.is_empty() {
        return Ok(());
    }
    let begin = match fold::back_to_heading(ed, l) {
        Some(h) => clock::marker(ed, h),
        None => Marker {
            file: clock::file_of(ed),
            line: l,
            heading: line(ed, l),
            category: String::new(),
            label: String::new(),
        },
    };
    let file = ed.path.clone();
    let name = format!(
        "*Org Columns: {}*",
        file.as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "[No Name]".into(), |f| f.to_string_lossy().into_owned())
    );
    let row = cap
        .rows
        .iter()
        .position(|r| Some(r.line) == fold::back_to_heading(ed, l))
        .map_or(1, |i| i + 1);
    let widths = widths(&cap.specs, &cap.rows);
    let src_file = file
        .as_ref()
        .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.clone()));
    super::effect(ed, move |s| {
        let src = match &src_file {
            Some(p) => Src::File(p.clone()),
            None => Src::Buffer(s.org_current()),
        };
        let v = View {
            src,
            file: src_file,
            global,
            begin,
            cap,
            widths,
            max_level: None,
        };
        install(s, &name, v, (row, 0));
    });
    Ok(())
}

/// The row and column at the cursor of a view.
fn at_point(ed: &Editor, v: &View) -> Result<(Row, usize), String> {
    let shown = v.shown();
    let r = ed
        .cur
        .line
        .checked_sub(1)
        .and_then(|i| shown.get(i))
        .ok_or("No column to edit at point")?;
    Ok(((*r).clone(), column_at(&v.widths, ed.cur.byte)))
}

/// Apply `action` to the row's entry in the source, then rebuild the view
/// (org-columns--execute-and-update).
fn execute(
    ed: &mut Editor,
    name: String,
    src: Src,
    row: Row,
    file: Option<PathBuf>,
    action: impl FnOnce(&mut Editor, usize) -> Result<(), String> + Send + 'static,
) {
    let m = Marker {
        file,
        line: row.line,
        heading: row.heading.clone(),
        category: String::new(),
        label: String::new(),
    };
    super::effect(ed, move |s| {
        let r = with_src(s, &src, |e| {
            let h = clock::locate(e, &m).ok_or("Cannot find the entry")?;
            let pos = (e.cur.line, e.cur.byte);
            e.undo.begin(e.cur.pos());
            e.set_cursor(h, 0);
            let r = action(e, h);
            e.undo.end(e.cur.pos());
            let n = e.line_count();
            e.set_cursor(pos.0.min(n - 1), pos.1);
            e.clamp_cursor();
            let msg = e.msg.take();
            r.map(|_| msg)
        });
        match r {
            Ok(Ok(msg)) => {
                redo(s, &name, false);
                if let Some(m) = msg {
                    s.ed.msg = Some(m);
                }
            }
            Ok(Err(e)) | Err(e) => s.ed.set_err(e),
        }
    });
}

/// org-columns-check-computed.
fn check_computed(v: &View, row: &Row, col: usize) -> Result<(), String> {
    if v.cap.specs[col].operator.is_some() && row.cells[col].computed {
        return Err("This value is computed from the entry's children".into());
    }
    Ok(())
}

/// Compute `f` in the source buffer, then `then` in the view (prompts
/// that need the entry's allowed values).
fn query_src<T: Send + 'static>(
    ed: &mut Editor,
    src: Src,
    f: impl FnOnce(&mut Editor) -> T + Send + 'static,
    then: impl FnOnce(&mut Editor, T) + Send + 'static,
) {
    push_after(ed, move |s| match with_src(s, &src, f) {
        Ok(t) => then(&mut s.ed, t),
        Err(e) => s.ed.set_err(e),
    });
}

/// org-property-get-allowed-values of the entry at row `m` in `e`.
fn allowed_in(e: &Editor, m: &Marker, key: &str) -> Vec<String> {
    let Some(h) = clock::locate(e, m) else {
        return vec![];
    };
    let st = super::settings(e);
    Doc::new(&e.buf, &st, e.path.as_deref()).allowed_values(Some(h), key)
}

fn row_marker(row: &Row, file: &Option<PathBuf>) -> Marker {
    Marker {
        file: file.clone(),
        line: row.line,
        heading: row.heading.clone(),
        category: String::new(),
        label: String::new(),
    }
}

/// org-columns-edit-value.
fn edit_value(ed: &mut Editor, key: Option<&str>) -> Result<(), String> {
    let (name, v) = get_view(ed)?;
    let (row, col) = at_point(ed, &v)?;
    check_computed(&v, &row, col)?;
    let key = key.map_or_else(|| v.cap.specs[col].property.clone(), str::to_owned);
    let value = v
        .cap
        .specs
        .iter()
        .position(|s| s.property == key)
        .map_or(String::new(), |i| row.cells[i].value.clone());
    let (src, file) = (v.src.clone(), v.file.clone());
    match key.as_str() {
        "CLOCKSUM" => Err("This special column cannot be edited".into()),
        "DEADLINE" | "SCHEDULED" => {
            let deadline = key == "DEADLINE";
            super::time::read_date(ed, super::time::ReadOpts::default(), move |ed, a| {
                let t = super::time::read::date_string(&a);
                execute(ed, name, src, row, file, move |e, _| {
                    super::time::schedule(e, deadline, Prefix::None, Some(t))
                });
            });
            Ok(())
        }
        "ITEM" => {
            super::read(ed, "Edit: ", &value, move |ed, new| {
                let new = new.trim().to_owned();
                execute(ed, name, src, row, file, move |e, h| {
                    let st = super::settings(e);
                    let t = line(e, h);
                    let x = syntax::headline(&t, &st).ok_or("Not at a heading")?;
                    super::set_line(
                        e,
                        h,
                        &format!("{}{new}{}", &t[..x.title.start], &t[x.title.end..]),
                    );
                    Ok(())
                });
            });
            Ok(())
        }
        "PRIORITY" => {
            let st = super::settings(ed);
            let (lo, hi, _) = st.priorities;
            let mut entries: Vec<(String, String)> = (lo..=hi)
                .map(props::priority_string)
                .map(|p| {
                    let k = p.trim_start_matches("[#").trim_end_matches(']').to_owned();
                    (k.clone(), k)
                })
                .collect();
            entries.push((" ".into(), "remove".into()));
            super::menu(
                ed,
                &format!(
                    "Priority {}-{}, SPC to remove: ",
                    props::priority_string(lo),
                    props::priority_string(hi)
                ),
                entries,
                move |ed, k| {
                    execute(ed, name, src, row, file, move |e, h| {
                        props::put(e, Some(h), "PRIORITY", k.trim())
                    });
                },
            );
            Ok(())
        }
        "TAGS" => {
            let cur = value.trim_matches(':').replace(':', " ");
            super::read(ed, "Tags: ", &cur, move |ed, t| {
                let tags: Vec<String> = t
                    .split([' ', ':', ','])
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned)
                    .collect();
                execute(ed, name, src, row, file, move |e, h| {
                    super::tags::set_tags(e, h, &tags);
                    Ok(())
                });
            });
            Ok(())
        }
        "TODO" => {
            let m = row_marker(&row, &file);
            query_src(
                ed,
                src.clone(),
                move |e| allowed_in(e, &m, "TODO"),
                move |ed, kws| {
                    super::complete(ed, "TODO: ", kws, true, move |ed, k| {
                        execute(ed, name, src, row, file, move |e, h| {
                            props::put(e, Some(h), "TODO", &k)
                        });
                    });
                },
            );
            Ok(())
        }
        _ => {
            let m = row_marker(&row, &file);
            let k2 = key.clone();
            query_src(
                ed,
                src.clone(),
                move |e| allowed_in(e, &m, &k2),
                move |ed, allowed| {
                    let cur = value.clone();
                    let go = move |ed: &mut Editor, nval: String| {
                        let nval = nval.trim().to_owned();
                        if nval == value {
                            return;
                        }
                        execute(ed, name, src, row, file, move |e, h| {
                            props::put(e, Some(h), &key, &nval)
                        });
                    };
                    if allowed.is_empty() {
                        super::read(ed, "Edit: ", &cur, go);
                    } else {
                        let unrestricted = allowed.iter().any(|a| a == ":ETC");
                        let allowed: Vec<String> =
                            allowed.into_iter().filter(|a| a != ":ETC").collect();
                        super::complete(ed, "Value: ", allowed, !unrestricted, go);
                    }
                },
            );
            Ok(())
        }
    }
}

/// org-colview-construct-allowed-dates.
fn allowed_dates(s: &str) -> Vec<String> {
    let re = crate::org_re!(
        r"^[<\[]([0-9]{4})-([0-9]{2})-([0-9]{2})(?: [^\]>\d]*)?(?: ([0-9]{1,2}):([0-9]{2}))?[^\]>]*[\]>]$"
    );
    let Some(c) = re.captures(s) else {
        return vec![];
    };
    let n = |i: usize| c.get(i).map(|m| m.as_str().parse::<i64>().unwrap_or(0));
    let with_time = c.get(4).is_some();
    let t = civil::Tm::at(
        n(1).unwrap(),
        n(2).unwrap(),
        n(3).unwrap(),
        n(4).unwrap_or(0),
        n(5).unwrap_or(0),
    );
    let inactive = s.starts_with('[');
    [-1, 0, 1]
        .iter()
        .map(|d| {
            super::time::stamp::stamp_text(
                civil::Tm {
                    day: t.day + d,
                    ..t
                }
                .norm(),
                with_time,
                inactive,
                "",
            )
        })
        .collect()
}

/// org-columns-next-allowed-value.
fn next_allowed(ed: &mut Editor, previous: bool, nth: Option<usize>) -> Result<(), String> {
    let (name, v) = get_view(ed)?;
    let (row, col) = at_point(ed, &v)?;
    check_computed(&v, &row, col)?;
    let spec = v.cap.specs[col].clone();
    let key = spec.property.clone();
    if key == "ITEM" {
        return Err("Cannot edit item headline from here".into());
    }
    let (src, file) = (v.src.clone(), v.file.clone());
    let m = row_marker(&row, &file);
    let k2 = key.clone();
    query_src(
        ed,
        src.clone(),
        move |e| allowed_in(e, &m, &k2),
        move |ed, allowed| {
            let a = Pick {
                name,
                src,
                file,
                row,
                col,
                spec,
                previous,
                nth,
            };
            if let Err(e) = next_value(ed, a, allowed) {
                ed.set_err(e);
            }
        },
    );
    Ok(())
}

/// What org-columns-next-allowed-value works on.
struct Pick {
    name: String,
    src: Src,
    file: Option<PathBuf>,
    row: Row,
    col: usize,
    spec: Spec,
    previous: bool,
    nth: Option<usize>,
}

fn next_value(ed: &mut Editor, a: Pick, mut allowed: Vec<String>) -> Result<(), String> {
    let Pick {
        name,
        src,
        file,
        row,
        col,
        spec,
        previous,
        nth,
    } = a;
    let key = spec.property.clone();
    let value = row.cells[col].value.clone();
    allowed.retain(|a| a != ":ETC");
    if allowed.is_empty() && matches!(spec.operator.as_deref(), Some("X" | "X/" | "X%")) {
        allowed = options::strings("org-columns-checkbox-allowed-values", &["[ ]", "[X]"]);
    }
    if allowed.is_empty() {
        allowed = allowed_dates(&value);
    }
    if previous {
        allowed.reverse();
    }
    let dated = matches!(key.as_str(), "SCHEDULED" | "DEADLINE" | "CLOCKSUM");
    if allowed.is_empty() && !dated {
        return Err("Allowed values for this property have not been defined".into());
    }
    let l = allowed.len();
    if dated {
        if key == "CLOCKSUM" {
            return Err("The CLOCKSUM property cannot be set with `org-entry-put'".into());
        }
        let n = if previous { -1 } else { 1 };
        let deadline = key == "DEADLINE";
        execute(ed, name, src, row, file, move |e, h| {
            let kind = if deadline {
                super::time::Planning::Deadline
            } else {
                super::time::Planning::Scheduled
            };
            let pl = h + 1;
            let t = if pl < e.line_count() {
                line(e, pl)
            } else {
                String::new()
            };
            let kw = format!("{}:", kind.keyword());
            let start = t
                .find(&kw)
                .and_then(|i| t[i..].find(['<', '[']).map(|j| i + j));
            match start {
                Some(b) if super::time::is_planning_line(&t) => {
                    super::time::change_at(e, pl, b, n, super::time::civil::Unit::Day, false)
                        .map(drop)
                }
                _ => Err(format!("No {} timestamp", kind.keyword())),
            }
        });
        return Ok(());
    }
    let new = match nth {
        Some(n) => {
            if n > l {
                return Err(format!("Only {l} allowed values for property `{key}'"));
            }
            allowed[(n + l - 1) % l].clone()
        }
        None => match allowed.iter().position(|a| *a == value) {
            Some(i) => {
                if l == 1 {
                    return Err("Only one allowed value for this property".into());
                }
                allowed[(i + 1) % l].clone()
            }
            None => allowed[0].clone(),
        },
    };
    execute(ed, name, src, row, file, move |e, h| {
        props::put(e, Some(h), &key, &new)
    });
    Ok(())
}

/// org-columns-store-format: write the format back (COLUMNS property of
/// the top entry, or the #+COLUMNS keyword).
fn store_format(e: &mut Editor, top: usize, fmt: &str) {
    if top < e.line_count()
        && syntax::level(&line(e, top)).is_some()
        && props::get(e, Some(top), "COLUMNS", Inherit::No).is_some()
    {
        let _ = props::put(e, Some(top), "COLUMNS", fmt);
        return;
    }
    let kw = (0..e.line_count())
        .find(|&i| syntax::keyword_line(&line(e, i)).is_some_and(|(k, _)| k == "COLUMNS"));
    match kw {
        Some(i) => {
            let t = line(e, i);
            let c = t.find(':').map_or(0, |c| c + 1);
            super::set_line(e, i, &format!("{} {fmt}", &t[..c]));
        }
        None => {
            let at = (0..e.line_count())
                .find(|&i| syntax::level(&line(e, i)).is_some())
                .unwrap_or(e.line_count());
            super::insert_lines(e, at, &[format!("#+COLUMNS: {fmt}")]);
        }
    }
}

/// Change the view's specs, store the format and redo.
fn change_specs(
    ed: &mut Editor,
    f: impl FnOnce(&mut Vec<Spec>, usize) -> Result<usize, String>,
) -> Result<(), String> {
    let (name, mut v) = get_view(ed)?;
    let col = column_at(&v.widths, ed.cur.byte);
    let new_col = f(&mut v.cap.specs, col)?;
    let fmt = uncompile_format(&v.cap.specs);
    v.cap.fmt = fmt.clone();
    let top = v.cap.top;
    let line_no = ed.cur.line;
    put_view(&name, v.clone());
    super::effect(ed, move |s| {
        let r = with_src(s, &v.src, |e| {
            e.undo.begin(e.cur.pos());
            store_format(e, top, &fmt);
            e.undo.end(e.cur.pos());
        });
        if let Err(e) = r {
            return s.ed.set_err(e);
        }
        redo(s, &name, true);
        if let Some(v) = VIEWS.with(|m| m.borrow().get(&name).cloned()) {
            s.ed.set_cursor(
                line_no.min(s.ed.line_count() - 1),
                column_start(&v.widths, new_col),
            );
        }
    });
    Ok(())
}

/// org-columns-new and org-columns-edit-attributes.
fn new_column(ed: &mut Editor, edit: bool) -> Result<(), String> {
    let (_, v) = get_view(ed)?;
    let col = column_at(&v.widths, ed.cur.byte);
    let old = edit.then(|| v.cap.specs.get(col).cloned()).flatten();
    let o1 = old.clone();
    query_src(
        ed,
        v.src.clone(),
        |e| props::buffer_keys(e, true, false, true),
        move |ed, keys| {
            super::complete(ed, "Property: ", keys, false, move |ed, prop| {
                let init = o1.as_ref().map(|o| o.title.clone()).unwrap_or_default();
                let o2 = o1.clone();
                super::read(
                    ed,
                    &format!("Column title [{prop}]: "),
                    &init,
                    move |ed, title| {
                        let init = o2
                            .as_ref()
                            .and_then(|o| o.width)
                            .map(|w| w.to_string())
                            .unwrap_or_default();
                        let o3 = o2.clone();
                        super::read(ed, "Column width: ", &init, move |ed, width| {
                            let mut ops: Vec<String> = vec![String::new()];
                            ops.extend(
                                [
                                    "+", "$", "X", "X/", "X%", "max", "mean", "min", ":", ":max",
                                    ":mean", ":min", "@max", "@mean", "@min", "est+",
                                ]
                                .iter()
                                .map(|s| (*s).to_owned()),
                            );
                            let o4 = o3.clone();
                            super::complete(ed, "Summary: ", ops, true, move |ed, op| {
                                let init = o4
                                    .as_ref()
                                    .and_then(|o| o.format.clone())
                                    .unwrap_or_default();
                                super::read(ed, "Format: ", &init, move |ed, fmt| {
                                    let nw = |s: String| (!s.trim().is_empty()).then_some(s);
                                    let operator = nw(op);
                                    let spec = Spec {
                                        property: prop.to_uppercase(),
                                        title: nw(title).unwrap_or_else(|| prop.clone()),
                                        width: width.trim().parse().ok(),
                                        format: if operator.as_deref() == Some("$") {
                                            Some("%.2f".into())
                                        } else {
                                            nw(fmt)
                                        },
                                        operator,
                                    };
                                    let r = change_specs(ed, move |specs, col| {
                                        if edit {
                                            if let Some(s) = specs.get_mut(col) {
                                                *s = spec;
                                            }
                                        } else {
                                            specs.insert(col.min(specs.len()), spec);
                                        }
                                        Ok(col)
                                    });
                                    if let Err(e) = r {
                                        ed.set_err(e);
                                    }
                                });
                            });
                        });
                    },
                );
            });
        },
    );
    Ok(())
}

/// org-columns-quit.
fn quit(ed: &mut Editor) {
    if let Some(n) = view_name(ed) {
        VIEWS.with(|m| m.borrow_mut().remove(&n));
    }
    super::effect(ed, |s| s.org_quit_temp());
}

/// Move the cursor to the next or previous row, keeping the column.
fn move_row(ed: &mut Editor, up: bool) {
    let n = ed.line_count();
    let l = ed.cur.line;
    let to = if up {
        l.saturating_sub(1).max(1)
    } else {
        (l + 1).min(n - 1)
    };
    let b = ed.cur.byte;
    ed.set_cursor(to, b);
    ed.clamp_cursor();
}

const HELP: &str = "Value: e edit  a allowed  n/S-right next  p/S-left previous  C-c C-c toggle checkbox  C-c C-t TODO | \
Column: M-S-right add  M-S-left delete  < narrower  > wider  s attributes | \
Move: M-up/M-down row  M-left/M-right column | View: c contents  o overview  v show value  C-c C-o open link | \
g/r refresh  ? help  q quit";

/// This module's interactive commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let in_view = is_view(ed);
    Some(match name {
        "org-columns" => {
            if ed.org.is_none() {
                return Some(Err("org-columns needs an Org buffer".into()));
            }
            let fmt = super::take_call_arg();
            columns(ed, !arg.is_none(), fmt)
        }
        "org-columns-quit" | "org-columns-remove-overlays" if in_view => {
            quit(ed);
            Ok(())
        }
        "org-columns-quit" | "org-columns-remove-overlays" => Ok(()),
        "org-columns-insert-dblock" | "org-insert-columns-dblock" => insert_dblock(ed),
        "org-columns-get-format-and-top-level" | "org-columns-get-format" => {
            let f = get_format(ed, ed.cur.line, None);
            ed.set_msg(f);
            Ok(())
        }
        "org-columns-compute" => {
            let prop = super::take_call_arg().unwrap_or_default().to_uppercase();
            let f = get_format(ed, ed.cur.line, None);
            let specs = compile_format(&f);
            let (top, end) = scope(ed, false, ed.cur.line);
            let mut out = HashMap::new();
            let mut update = true;
            for (i, s) in specs.iter().enumerate() {
                if s.property == prop {
                    compute_spec(ed, s, i, update, top, end, &mut out);
                    update = false;
                }
            }
            Ok(())
        }
        "org-agenda-columns" => {
            Err("org-agenda-columns: the agenda calls colview::entry_values".into())
        }
        _ if !in_view && name.starts_with("org-columns-") => Err("Not in column view".into()),
        "org-columns-redo" => {
            let n = view_name(ed).unwrap_or_default();
            super::effect(ed, move |s| redo(s, &n, false));
            Ok(())
        }
        "org-columns-edit-value" => edit_value(ed, None),
        "org-columns-todo" => edit_value(ed, Some("TODO")),
        "org-columns-next-allowed-value" => next_allowed(ed, false, None),
        "org-columns-previous-allowed-value" => next_allowed(ed, true, None),
        "org-columns-toggle-or-columns-quit" => {
            let (_, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let toggles = at_point(ed, &v)
                .is_ok_and(|(r, c)| crate::org_re!(r"^\[[ xX-]\]$").is_match(&r.cells[c].value));
            if toggles {
                next_allowed(ed, false, None)
            } else {
                quit(ed);
                Ok(())
            }
        }
        "org-columns-show-value" => {
            let (_, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let value = at_point(ed, &v)
                .map(|(r, c)| r.cells[c].value.clone())
                .unwrap_or_default();
            ed.set_msg(format!("Value is: {value}"));
            Ok(())
        }
        "org-columns-open-link" => {
            let (_, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let value = at_point(ed, &v)
                .map(|(r, c)| r.cells[c].value.clone())
                .unwrap_or_default();
            let st = super::settings(ed);
            match super::links::links_in(&value, &st).into_iter().next() {
                Some(lk) => super::links::open(ed, &lk, arg),
                None => Err("No link found".into()),
            }
        }
        "org-columns-edit-allowed" => {
            let (name, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let (row, col) = match at_point(ed, &v) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let key = format!("{}_ALL", v.cap.specs[col].property);
            let (src, file, top) = (v.src.clone(), v.file.clone(), v.cap.top);
            let m = row_marker(&row, &file);
            let k2 = key.clone();
            let q = move |e: &mut Editor| {
                clock::locate(e, &m)
                    .and_then(|h| props::get(e, Some(h), &k2, Inherit::Yes))
                    .unwrap_or_default()
            };
            query_src(ed, src.clone(), q, move |ed, cur| {
                super::read(ed, "Allowed: ", &cur, move |ed, new| {
                    execute(ed, name, src, row, file, move |e, h| {
                        let owner = {
                            let mut c = Some(h);
                            let mut found = None;
                            while let Some(x) = c {
                                if props::get(e, Some(x), &key, Inherit::No).is_some() {
                                    found = Some(x);
                                    break;
                                }
                                c = syntax::parent(&e.buf, x);
                            }
                            found
                        };
                        let at = owner
                            .or_else(|| syntax::level(&line(e, top)).map(|_| top))
                            .unwrap_or(h);
                        props::put(e, Some(at), &key, &new)
                    });
                });
            });
            Ok(())
        }
        "org-columns-new" => new_column(ed, false),
        "org-columns-edit-attributes" => new_column(ed, true),
        "org-columns-delete" => {
            let (_, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let col = column_at(&v.widths, ed.cur.byte);
            let title = v
                .cap
                .specs
                .get(col)
                .map(|s| s.title.clone())
                .unwrap_or_default();
            super::yes_or_no(
                ed,
                &format!("Are you sure you want to remove column {title:?}? "),
                move |ed| {
                    let r = change_specs(ed, |specs, col| {
                        if col < specs.len() {
                            specs.remove(col);
                        }
                        Ok(col.min(specs.len().saturating_sub(1)))
                    });
                    if let Err(e) = r {
                        ed.set_err(e);
                    }
                },
            );
            Ok(())
        }
        "org-columns-widen" | "org-columns-narrow" => {
            let (_, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let n = if name == "org-columns-widen" {
                arg.value()
            } else {
                -arg.value()
            };
            let col = column_at(&v.widths, ed.cur.byte);
            let w = v.widths.get(col).copied().unwrap_or(1) as i64;
            change_specs(ed, move |specs, col| {
                if let Some(s) = specs.get_mut(col) {
                    s.width = Some((w + n).max(1) as usize);
                }
                Ok(col)
            })
        }
        "org-columns-move-right" | "org-columns-move-left" => {
            let right = name == "org-columns-move-right";
            change_specs(ed, move |specs, col| {
                if right {
                    if col + 1 >= specs.len() {
                        return Err("Cannot shift this column further to the right".into());
                    }
                    specs.swap(col, col + 1);
                    Ok(col + 1)
                } else {
                    if col == 0 {
                        return Err("Cannot shift this column further to the left".into());
                    }
                    specs.swap(col, col - 1);
                    Ok(col - 1)
                }
            })
        }
        "org-columns-move-row-up" | "org-columns-move-row-down" => {
            let (name, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let (row, _) = match at_point(ed, &v) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let up = name.ends_with("up");
            let n = if up { -1 } else { 1 };
            let target = (ed.cur.line as i64 + n).max(1) as usize;
            let b = ed.cur.byte;
            let (src, file) = (v.src.clone(), v.file.clone());
            execute(ed, name.clone(), src, row, file, move |e, _| {
                super::structure::move_subtree(e, n)
            });
            let nm = name;
            push_after(ed, move |s| {
                let _ = &nm;
                s.ed.set_cursor(target.min(s.ed.line_count() - 1), b);
                s.ed.clamp_cursor();
            });
            Ok(())
        }
        "org-columns-move-up" => {
            move_row(ed, true);
            Ok(())
        }
        "org-columns-move-down" => {
            move_row(ed, false);
            Ok(())
        }
        "org-columns-content" | "org-overview" => {
            let (n, mut v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            v.max_level = (name == "org-overview").then_some(1);
            let cursor = (ed.cur.line, ed.cur.byte);
            super::effect(ed, move |s| install(s, &n, v, cursor));
            Ok(())
        }
        "org-columns-help-quick-toggle" => {
            ed.set_msg(HELP);
            Ok(())
        }
        "forward-char" | "backward-char" if in_view => {
            let (_, v) = match get_view(ed) {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            let col = column_at(&v.widths, ed.cur.byte);
            let to = if name == "forward-char" {
                (col + 1).min(v.widths.len() - 1)
            } else {
                col.saturating_sub(1)
            };
            let b = column_start(&v.widths, to);
            ed.cur.byte = b.min(line(ed, ed.cur.line).len());
            Ok(())
        }
        _ => return None,
    })
}

/// Queue `f` after the pending effect.
fn push_after(ed: &mut Editor, f: impl FnOnce(&mut Session) + Send + 'static) {
    let prev = ed.pending_effect.take();
    super::effect(ed, move |s| {
        if let Some(p) = prev {
            s.perform(p);
        }
        f(s);
    });
}

// ---- the columnview dynamic block ----

/// org-columns--clean-item: drop objects a table cell must not hold.
fn clean_item(item: &str) -> String {
    let mut s = item.to_owned();
    for re in [
        crate::org_re!(r"\[fn:[^\]]*\][ \t]*"),
        crate::org_re!(r"call_[^\s(\[]+(?:\[[^\]]*\])?\([^)]*\)(?:\[[^\]]*\])?[ \t]*"),
        crate::org_re!(r"src_[^\s{\[]+(?:\[[^\]]*\])?\{[^}]*\}[ \t]*"),
        crate::org_re!(r"<<<?[^<>\n]+>>>?[ \t]*"),
        crate::org_re!(r"\[[0-9]*(?:%|/[0-9]*)\][ \t]*"),
    ] {
        s = re.replace_all(&s, "").into_owned();
    }
    s.trim_end().replace('|', "\\vert")
}

/// A captured table row: the entry's level and cells.
type TRow = (usize, Vec<String>);

/// org-columns--capture-view.
fn capture_view(
    ed: &mut Editor,
    l: usize,
    local: bool,
    params: &Params,
) -> (Vec<Spec>, Vec<String>, Vec<TRow>) {
    let fmt = params.str(":format");
    let cap = capture(ed, !local, l, fmt.as_deref(), false);
    let maxlevel = params.int(":maxlevel");
    let skip_empty = params.on(":skip-empty-rows");
    let exclude: Vec<String> = params
        .get(":exclude-tags")
        .and_then(|v| {
            v.list().map(|l| {
                l.iter()
                    .filter_map(|x| x.str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default();
    let st = super::settings(ed);
    let m = match (params.str(":match"), maxlevel) {
        (Some(m), Some(n)) => Some(format!("{m}+LEVEL<={n}")),
        (Some(m), None) => Some(m),
        (None, Some(n)) => Some(format!("LEVEL<={n}")),
        (None, None) => None,
    };
    let matcher = m.map(|m| super::tags::Matcher::new(&m, &st, civil::now_epoch()));
    let lines = super::lines(ed, 0..ed.line_count());
    let doc = Doc::new(&lines, &st, ed.path.as_deref());
    let has_item = cap.specs.iter().any(|s| s.property == "ITEM");
    let mut table = vec![];
    for r in &cap.rows {
        let h = r.line;
        if let Some(m) = &matcher {
            let Some(x) = syntax::headline(&lines[h], &st) else {
                continue;
            };
            let tags = doc.tags(Some(h), false);
            let prop = |p: &str| doc.get(Some(h), p, Inherit::Selective, false);
            let not_done = |t: &str| !st.is_done(t);
            let ok = m.matches(&super::tags::EntryInfo {
                todo: x.todo.as_deref(),
                tags: &tags,
                level: x.level,
                prop: &prop,
                not_done: &not_done,
            });
            if !ok {
                continue;
            }
        }
        let row: Vec<String> = cap
            .specs
            .iter()
            .zip(&r.cells)
            .map(|(s, c)| {
                if s.property == "ITEM" {
                    c.value.clone()
                } else {
                    c.displayed.clone()
                }
            })
            .collect();
        if skip_empty {
            let mut r2: Vec<&String> = row.iter().filter(|x| !x.is_empty()).collect();
            r2.dedup();
            if r2.is_empty() || (has_item && r2.len() == 1) {
                continue;
            }
        }
        if !exclude.is_empty() && doc.tags(Some(h), false).iter().any(|t| exclude.contains(t)) {
            continue;
        }
        table.push((r.level, row));
    }
    let titles = cap.specs.iter().map(|s| s.title.clone()).collect();
    (cap.specs, titles, table)
}

/// The entry with ID in `ed` (org-find-entry-with-id).
fn find_id(ed: &Editor, id: &str) -> Option<usize> {
    (0..ed.line_count()).find_map(|i| {
        props::parse_property(&line(ed, i))
            .filter(|(k, v)| k.eq_ignore_ascii_case("ID") && v == id)
            .and_then(|_| fold::back_to_heading(ed, i))
    })
}

/// org-dblock-write:columnview.
pub fn dblock_write(ed: &mut Editor, params: &Params, out: &mut Out) -> Result<(), String> {
    if let Some(f) = params.get(":formatter").filter(|f| f.truthy())
        && f.sym() != Some("org-columns-dblock-write-default")
    {
        return Err(format!(
            "Column view formatter {f} cannot be called in fred"
        ));
    }
    let point = out.line;
    let id = params.get(":id").cloned().unwrap_or(Sexp::Nil);
    let id_s = id.str().or(id.sym()).map(str::to_owned);
    let (specs, titles, table) = match id_s.as_deref() {
        Some("global") => capture_view(ed, 0, false, params),
        None | Some("nil") | Some("local") => capture_view(ed, point, true, params),
        Some(f) if f.starts_with("file:") => {
            let p = options::expand(&f[5..]);
            let text = std::fs::read_to_string(&p).map_err(|_| format!("No such file: {f:?}"))?;
            let mut e = Editor::new(crate::buffer::Buffer::from_text(&text));
            e.path = Some(p);
            e.org = Some(Box::default());
            capture_view(&mut e, 0, false, params)
        }
        Some(id) => match find_id(ed, id) {
            Some(h) => capture_view(ed, h, true, params),
            None => {
                let mut found = None;
                for f in super::agenda_files() {
                    let Ok(text) = std::fs::read_to_string(&f) else {
                        continue;
                    };
                    let mut e = Editor::new(crate::buffer::Buffer::from_text(&text));
                    e.path = Some(f.clone());
                    e.org = Some(Box::default());
                    if let Some(h) = find_id(&e, id) {
                        found = Some(capture_view(&mut e, h, true, params));
                        break;
                    }
                }
                found.ok_or_else(|| format!("Cannot find entry with :ID: {id}"))?
            }
        },
    };
    write_default(ed, &specs, titles, table, params, out);
    Ok(())
}

/// org-columns-dblock-write-default.
fn write_default(
    ed: &Editor,
    specs: &[Spec],
    titles: Vec<String>,
    table: Vec<TRow>,
    params: &Params,
    out: &mut Out,
) {
    let link = params.on(":link");
    let hlines = params.get(":hlines").cloned().unwrap_or(Sexp::Nil);
    let indent = params.on(":indent");
    let item = specs.iter().position(|s| s.property == "ITEM");
    let mut rows: Vec<Option<Vec<String>>> = vec![Some(titles), None];
    for (level, mut cells) in table {
        let rule = match &hlines {
            Sexp::Nil => false,
            Sexp::Int(n) => level as i64 <= *n,
            _ => true,
        };
        if rule && rows.last().is_some_and(Option::is_some) {
            rows.push(None);
        }
        if let Some(i) = item {
            let raw = cells[i].clone();
            let cleaned = clean_item(&raw);
            let it = if link {
                let search = format!("*{}", super::links::normalize(&raw, false, false));
                let target = match &ed.path {
                    Some(p) => format!(
                        "file:{}::{search}",
                        std::path::absolute(p)
                            .unwrap_or_else(|_| p.clone())
                            .display()
                    ),
                    None => search,
                };
                super::links::make_string(&target, Some(&cleaned)).unwrap_or(cleaned)
            } else {
                cleaned
            };
            cells[i] = if indent && level > 1 {
                format!("\\_{}{it}", " ".repeat(2 * (level - 1)))
            } else {
                it
            };
        }
        rows.push(Some(cells));
    }
    if params.on(":vlines") {
        for r in rows.iter_mut().flatten() {
            r.insert(0, String::new());
        }
        let mut last = vec!["/".to_owned()];
        last.extend(std::iter::repeat_n("<>".to_owned(), specs.len()));
        rows.push(Some(last));
    }
    if specs.iter().any(|s| s.width.is_some()) {
        rows.insert(
            0,
            Some(
                specs
                    .iter()
                    .map(|s| s.width.map(|w| format!("<{w}>")).unwrap_or_default())
                    .collect(),
            ),
        );
    }
    let content = params.str(":content").unwrap_or_default();
    let mut content_lines: Vec<&str> = content.lines().collect();
    let mut text = String::new();
    while content_lines
        .first()
        .is_some_and(|l| l.trim_start().starts_with("#+"))
    {
        let l = content_lines.remove(0);
        if l.trim_start().to_ascii_lowercase().starts_with("#+tblfm:") {
            content_lines.insert(0, l);
            break;
        }
        text.push_str(l.trim_start());
        text.push('\n');
    }
    let mut table_lines: Vec<String> = rows
        .iter()
        .map(|r| match r {
            None => "|-|".to_owned(),
            Some(c) => format!("|{}|", c.join("|")),
        })
        .collect();
    let tblfm: Vec<String> = content_lines
        .iter()
        .filter(|l| l.trim_start().to_ascii_lowercase().starts_with("#+tblfm:"))
        .map(|l| l.trim_start().to_owned())
        .collect();
    let mut all = dblock::align(&table_lines);
    if !tblfm.is_empty() {
        let mut d = super::table::doc::Doc::new(&[all.clone(), tblfm.clone()].concat().join("\n"));
        if super::table::formula::recalculate(&mut d, super::table::formula::Recalc::All, true)
            .is_ok()
        {
            d.align();
            all = d.lines[..all.len()].to_vec();
        }
        all.extend(tblfm);
    }
    table_lines = all;
    text.push_str(&table_lines.join("\n"));
    out.insert(&text);
}

/// org-columns-insert-dblock.
fn insert_dblock(ed: &mut Editor) -> Result<(), String> {
    let mut ids = vec!["global".to_owned(), "local".to_owned()];
    ids.extend(props::values(ed, "ID"));
    let go = |ed: &mut Editor, id: String| {
        let id = match id.as_str() {
            "global" => Sexp::Sym("global".into()),
            "" | "local" => Sexp::Sym("local".into()),
            _ => Sexp::Str(id),
        };
        let mut p = Params::default();
        p.set(":name", Sexp::Str("columnview".into()));
        p.set(":hlines", Sexp::Int(1));
        p.set(":id", id);
        ed.undo.begin(ed.cur.pos());
        let b = dblock::create(ed, "columnview", &p);
        if let Err(e) = dblock::update(ed, b) {
            ed.set_err(e);
        }
        ed.undo.end(ed.cur.pos());
    };
    match super::take_call_arg() {
        Some(id) => go(ed, id),
        None => super::complete(
            ed,
            "Capture columns (local, global, entry with :ID: property) [local]: ",
            ids,
            false,
            go,
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
