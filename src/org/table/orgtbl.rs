//! Tables in other formats: orgtbl-to-generic and its specializations
//! (TSV, CSV, LaTeX, HTML, Texinfo, Org, table.el, Unicode), radio tables
//! (`#+ORGTBL: SEND` / `BEGIN RECEIVE ORGTBL`), org-table-export and
//! orgtbl-ascii-plot.
//!
//! Upstream builds these on the export framework; here each backend's
//! table, row and cell transcoders are written out directly, with the
//! generic parameters (`:tstart`, `:lfmt`, `:fmt`, `:efmt`, `:hline`,
//! `:skip`, `:skipcols`, `:splice`, ...) applied the same way. Lisp
//! values in parameters (`(lambda (c) ...)`) run in the formula Lisp.

use super::doc::{Doc, Row, align_field, column_layout, lazy_re, re, width};
use super::lisp::{self, Interp, L};
use regex::Regex;
use std::sync::LazyLock;

/// A parameter plist.
#[derive(Clone, Debug, Default)]
pub struct Params(pub Vec<L>);

impl Params {
    /// Read `:key value ...` text.
    pub fn read(s: &str) -> Result<Params, String> {
        Ok(Params(lisp::read_all(s)?))
    }

    fn find(&self, key: &str) -> Option<&L> {
        self.0.chunks(2).find(|p| p[0].sym_name() == Some(key)).map(|p| p.get(1).unwrap_or(&L::Nil))
    }

    pub fn member(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    pub fn get(&self, key: &str) -> Option<L> {
        self.find(key).filter(|v| v.truthy()).cloned()
    }

    pub fn str(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|v| v.as_str().map(str::to_owned))
    }

    /// org-combine-plists: `self` overridden by `o`.
    pub fn with(&self, o: &Params) -> Params {
        let mut out = self.0.clone();
        for p in o.0.chunks(2) {
            let key = p[0].sym_name().unwrap_or("");
            let v = p.get(1).cloned().unwrap_or(L::Nil);
            match out.chunks(2).position(|q| q[0].sym_name() == Some(key)) {
                Some(i) => out[i * 2 + 1] = v,
                None => {
                    out.push(p[0].clone());
                    out.push(v);
                }
            }
        }
        Params(out)
    }

    pub fn set(mut self, key: &str, v: L) -> Params {
        self = self.with(&Params(vec![lisp::sym(key), v]));
        self
    }
}

fn is_function(v: &L) -> bool {
    match v {
        L::Lambda(_) => true,
        L::Sym(_) => true,
        L::List(x) => x.first().and_then(L::sym_name) == Some("lambda"),
        _ => false,
    }
}

/// org-table--generic-apply for a value applied to `args`.
fn apply(i: &mut Interp, value: &L, name: &str, args: &[L]) -> Result<Option<String>, String> {
    if value.is_nil() {
        return Ok(None);
    }
    if is_function(value) {
        let r = i.apply(value, args.to_vec())?;
        return Ok(Some(match r {
            L::Str(s) => s.to_string(),
            L::Nil => String::new(),
            o => lisp::princ(&o),
        }));
    }
    if let L::Str(s) = value {
        return Ok(Some(match args {
            [] => s.to_string(),
            [L::List(items)] => lisp::format(s, items)?,
            [L::Nil] => lisp::format(s, &[])?,
            a => lisp::format(s, a)?,
        }));
    }
    Err(format!("Wrong {name} value"))
}

/// The plist form `(COL FMT COL FMT)` of :fmt, :hfmt and :efmt.
fn apply_cons(i: &mut Interp, value: &L, name: &str, column: usize, contents: &str, args: &[L]) -> Result<String, String> {
    if let L::List(items) = value
        && !is_function(value)
    {
        let val = items.chunks(2).find(|p| matches!(p[0], L::Int(c) if c as usize == column)).and_then(|p| p.get(1).cloned());
        return match val {
            None => Ok(contents.to_owned()),
            Some(L::Str(s)) => lisp::format(&s, args),
            Some(f) if is_function(&f) => apply(i, &f, name, args).map(Option::unwrap_or_default),
            Some(_) => Err(format!("Wrong {name} value")),
        };
    }
    apply(i, value, name, args).map(|r| r.unwrap_or_else(|| contents.to_owned()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Org,
    Latex,
    Html,
    Texinfo,
    Ascii,
}

lazy_re!(COOKIE, r"^<[lrc]?([0-9]+)?>$");
lazy_re!(EXP, r"^([-+]?[0-9][0-9.]*)[eE]([-+]?[0-9]+)$");

const SPECIAL_MARKS: &[&str] = &["/", "#", "!", "$", "*", "_", "^"];

/// org-export-table-has-special-column-p.
pub fn has_special_column(rows: &[Row]) -> bool {
    let mut special = false;
    for r in rows {
        let Row::Cells(c) = r else { continue };
        match c.first().map(String::as_str) {
            Some(v) if SPECIAL_MARKS.contains(&v) => special = true,
            None | Some("") => {}
            _ => return false,
        }
    }
    special
}

/// org-export-table-row-is-special-p.
fn is_special_row(cells: &[String], special_col: bool) -> bool {
    let first = cells.first().map_or("", String::as_str);
    if first == "/" {
        return true;
    }
    let mut cookie = false;
    let mut all = true;
    for c in cells {
        if c.is_empty() {
            continue;
        }
        if COOKIE.is_match(c) {
            cookie = true;
        } else {
            all = false;
            break;
        }
    }
    if all && cookie {
        return true;
    }
    special_col && ["^", "_", "$", "!"].contains(&first)
}

/// A table prepared for transcoding.
struct Prepared {
    /// Rows (cells exclude the special column).
    rows: Vec<Row>,
    /// The original cells (for column groups and cookies).
    all_rows: Vec<Row>,
    special: bool,
}

fn prepare(table: &[Row], params: &Params, keep_special_rows: bool) -> Result<Prepared, String> {
    let mut rows = table.to_vec();
    if let Some(skip) = params.get(":skip") {
        let L::Int(n) = skip else {
            return Err("Wrong :skip value".into());
        };
        if n < 0 {
            return Err("Wrong :skip value".into());
        }
        rows = rows.into_iter().skip(n as usize).collect();
    }
    let special = has_special_column(&rows);
    let all_rows = rows.clone();
    let skipcols: Vec<usize> = match params.get(":skipcols") {
        Some(L::List(v)) => v.iter().filter_map(|x| if let L::Int(i) = x { Some(*i as usize) } else { None }).collect(),
        Some(_) => return Err("Wrong :skipcols value".into()),
        None => vec![],
    };
    let with_special = if params.member(":with-special-rows") { params.get(":with-special-rows").is_some() } else { keep_special_rows };
    let mut out = vec![];
    for r in rows {
        match r {
            Row::Hline => out.push(Row::Hline),
            Row::Cells(c) => {
                if !with_special && is_special_row(&c, special) {
                    continue;
                }
                let cells: Vec<String> = c
                    .into_iter()
                    .skip(special as usize)
                    .enumerate()
                    .filter(|(i, _)| !skipcols.contains(&(i + 1)))
                    .map(|(_, x)| x)
                    .collect();
                out.push(Row::Cells(cells));
            }
        }
    }
    Ok(Prepared { rows: out, all_rows, special })
}

/// Row-group facts: header rows and the row ending the header.
fn header_info(rows: &[Row]) -> (Vec<bool>, Vec<bool>) {
    // org-export-table-has-header-p.
    let mut group = 1;
    let mut row_flag = false;
    let mut has_header = false;
    for r in rows {
        if group > 1 {
            has_header = true;
            break;
        }
        match r {
            Row::Hline if row_flag => {
                group += 1;
                row_flag = false;
            }
            Row::Cells(_) if !row_flag => row_flag = true,
            _ => {}
        }
    }
    let mut in_header = vec![false; rows.len()];
    let mut ends_header = vec![false; rows.len()];
    if has_header {
        let mut seen_data = false;
        let mut last = None;
        for (i, r) in rows.iter().enumerate() {
            match r {
                Row::Cells(_) => {
                    seen_data = true;
                    in_header[i] = true;
                    last = Some(i);
                }
                Row::Hline if seen_data => break,
                Row::Hline => {}
            }
        }
        if let Some(l) = last {
            ends_header[l] = true;
        }
    }
    (in_header, ends_header)
}

/// Column alignments for the exported table (org-export-table-cell-alignment).
fn alignments(p: &Prepared) -> Vec<char> {
    let rows: Vec<&Vec<String>> = p
        .all_rows
        .iter()
        .filter_map(Row::cells)
        .collect();
    let stripped: Vec<Vec<String>> = rows.iter().map(|r| r.iter().skip(p.special as usize).cloned().collect()).collect();
    let refs: Vec<&Vec<String>> = stripped.iter().collect();
    let (_, aligns) = column_layout(&refs);
    let ncol = p.rows.iter().filter_map(Row::cells).map(Vec::len).max().unwrap_or(0);
    (0..ncol).map(|i| aligns.get(i).copied().unwrap_or('l')).collect()
}

/// Column groups from a `/` row: (starts group, ends group) per column.
fn column_groups(p: &Prepared) -> Vec<(bool, bool)> {
    let ncol = p.rows.iter().filter_map(Row::cells).map(Vec::len).max().unwrap_or(0);
    let mut g = vec![(false, false); ncol];
    for r in &p.all_rows {
        let Row::Cells(c) = r else { continue };
        if c.first().map(String::as_str) != Some("/") {
            continue;
        }
        for (i, cell) in c.iter().skip(p.special as usize).enumerate().take(ncol) {
            match cell.as_str() {
                "<" => g[i].0 = true,
                ">" => g[i].1 = true,
                "<>" => g[i] = (true, true),
                _ => {}
            }
        }
    }
    g
}

/// Cookie widths (`<N>`) per column.
fn cookie_widths(p: &Prepared) -> Vec<Option<usize>> {
    let ncol = p.rows.iter().filter_map(Row::cells).map(Vec::len).max().unwrap_or(0);
    let mut w = vec![None; ncol];
    for r in &p.all_rows {
        let Row::Cells(c) = r else { continue };
        for (i, cell) in c.iter().skip(p.special as usize).enumerate().take(ncol) {
            if let Some(m) = COOKIE.captures(cell)
                && let Some(n) = m.get(1)
                && w[i].is_none()
            {
                w[i] = n.as_str().parse().ok();
            }
        }
    }
    w
}

// ---- inline markup ----

lazy_re!(EMPH, r"(^|[\s\-('{])([*/_=~+])([^\s].*?[^\s]|[^\s])([*/_=~+])($|[\s\-.,:!?;')}])");
lazy_re!(MATH, r"\$([^$\s][^$]*?)\$|\\\((.*?)\\\)");
lazy_re!(LINK, r"\[\[([^\]]+)\](?:\[([^\]]+)\])?\]");

fn latex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\^{}"),
            '\\' => out.push_str("\\textbackslash{}"),
            c => out.push(c),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn texinfo_escape(s: &str) -> String {
    s.replace('@', "@@").replace('{', "@{").replace('}', "@}")
}

/// Transcode a cell's Org markup for `b`.
fn markup(s: &str, b: Backend) -> String {
    if b == Backend::Org || b == Backend::Ascii {
        return s.to_owned();
    }
    // Split into protected pieces (math, links, emphasis) and text.
    let mut out = String::new();
    let mut rest = s;
    loop {
        let m = [MATH.find(rest), LINK.find(rest), EMPH.find(rest).filter(|m| m.as_str().len() > 1)].into_iter().flatten().min_by_key(|m| m.start());
        let Some(m) = m else {
            out.push_str(&escape(rest, b));
            break;
        };
        out.push_str(&escape(&rest[..m.start()], b));
        let piece = m.as_str();
        if let Some(c) = MATH.captures(piece).filter(|c| c.get(0).unwrap().start() == 0) {
            let body = c.get(1).or(c.get(2)).map_or("", |x| x.as_str());
            out.push_str(&match b {
                Backend::Latex | Backend::Html => format!("\\({body}\\)"),
                _ => String::new(),
            });
        } else if let Some(c) = LINK.captures(piece).filter(|c| c.get(0).unwrap().start() == 0) {
            let target = &c[1];
            let desc = c.get(2).map_or(target, |x| x.as_str());
            out.push_str(&match b {
                Backend::Latex => format!("\\href{{{target}}}{{{}}}", latex_escape(desc)),
                Backend::Html => format!("<a href=\"{target}\">{}</a>", html_escape(desc)),
                _ => format!("@uref{{{target},{}}}", texinfo_escape(desc)),
            });
        } else if let Some(c) = EMPH.captures(piece) {
            out.push_str(&escape(&c[1], b));
            let marker = &c[2];
            let body = &c[3];
            let inner = if marker == "=" || marker == "~" { escape(body, b) } else { markup(body, b) };
            out.push_str(&match (b, marker) {
                (Backend::Latex, "*") => format!("\\textbf{{{inner}}}"),
                (Backend::Latex, "/") => format!("\\emph{{{inner}}}"),
                (Backend::Latex, "_") => format!("\\uline{{{inner}}}"),
                (Backend::Latex, "+") => format!("\\sout{{{inner}}}"),
                (Backend::Latex, _) => format!("\\texttt{{{inner}}}"),
                (Backend::Html, "*") => format!("<b>{inner}</b>"),
                (Backend::Html, "/") => format!("<i>{inner}</i>"),
                (Backend::Html, "_") => format!("<span class=\"underline\">{inner}</span>"),
                (Backend::Html, "+") => format!("<del>{inner}</del>"),
                (Backend::Html, _) => format!("<code>{inner}</code>"),
                (_, "*") => format!("@strong{{{inner}}}"),
                (_, "/") => format!("@emph{{{inner}}}"),
                (_, "=") => format!("@samp{{{inner}}}"),
                (_, "~") => format!("@code{{{inner}}}"),
                _ => inner,
            });
            out.push_str(&escape(&c[5], b));
        }
        rest = &rest[m.end()..];
    }
    out
}

fn escape(s: &str, b: Backend) -> String {
    match b {
        Backend::Latex => latex_escape(s),
        Backend::Html => html_escape(s),
        Backend::Texinfo => texinfo_escape(s),
        _ => s.to_owned(),
    }
}

// ---- the generic translator ----

/// orgtbl-to-generic.
pub fn to_generic(table: &[Row], params: &Params) -> Result<String, String> {
    let backend = match params.get(":backend").as_ref().and_then(L::sym_name) {
        None => None,
        Some("org") => Some(Backend::Org),
        Some("latex") => Some(Backend::Latex),
        Some("html") => Some(Backend::Html),
        Some("texinfo") => Some(Backend::Texinfo),
        Some("ascii") => Some(Backend::Ascii),
        Some(other) => return Err(format!("Unknown :backend value: {other}")),
    };
    let raw = params.get(":raw").is_some();
    let b = backend.unwrap_or(Backend::Org);
    let p = prepare(table, params, matches!(b, Backend::Org))?;
    let (in_header, ends_header) = header_info(&p.rows);
    let mut i = Interp::new();
    let g = |k: &str| params.get(k);
    let (sep, hsep) = (g(":sep"), g(":hsep"));
    let (fmt, hfmt, efmt) = (g(":fmt"), g(":hfmt"), g(":efmt"));
    let aligns = alignments(&p);
    let n = p.rows.len();
    // The rows a transcoder sees after skipped ones.
    let mut out_rows: Vec<String> = vec![];
    let ascii = if b == Backend::Ascii { Some(ascii_layout(&p, params)) } else { None };
    for (ri, row) in p.rows.iter().enumerate() {
        let first = ri == 0;
        let last = ri + 1 == n;
        let cells = match row {
            Row::Hline => {
                let r = if params.member(":hline") {
                    match params.get(":hline") {
                        Some(h) => apply(&mut i, &h, ":hline", &[])?,
                        None => None,
                    }
                } else {
                    match b {
                        Backend::Org if backend.is_some() => Some("|-".into()),
                        Backend::Org => None,
                        Backend::Latex => Some(latex_rule(params, first, last)),
                        Backend::Html | Backend::Texinfo => None,
                        Backend::Ascii => (!first && !last).then(|| ascii.as_ref().unwrap().rule('─')),
                    }
                };
                if let Some(r) = r {
                    out_rows.push(r);
                }
                continue;
            }
            Row::Cells(c) => c,
        };
        let headerp = in_header[ri];
        let last_header = ends_header[ri];
        let ncells = cells.len();
        let mut plain: Vec<L> = vec![];
        let mut joined = String::new();
        for (ci, cell) in cells.iter().enumerate() {
            let column = ci + 1;
            let mut contents: Option<String> = if cell.is_empty() {
                None
            } else if raw && backend.is_some() {
                Some(cell.clone())
            } else {
                Some(markup(cell, b))
            };
            if let Some(c) = &contents {
                let mut c = c.clone();
                if let Some(e) = &efmt
                    && let Some(m) = EXP.captures(&c)
                {
                    let args = [lisp::str_(&m[1]), lisp::str_(&m[2])];
                    c = apply_cons(&mut i, e, ":efmt", column, &c, &args)?;
                }
                if let (Some(h), true) = (&hfmt, headerp) {
                    c = apply_cons(&mut i, h, ":hfmt", column, &c, &[lisp::str_(&c)])?;
                } else if let Some(f) = &fmt {
                    c = apply_cons(&mut i, f, ":fmt", column, &c, &[lisp::str_(&c)])?;
                }
                contents = Some(c);
            }
            let body = contents.clone().unwrap_or_default();
            plain.push(lisp::str_(&body));
            let last_cell = ci + 1 == ncells;
            let with_backend = |s: String| -> String {
                match b {
                    Backend::Org if backend.is_some() => format!(" {s} |"),
                    Backend::Latex => {
                        if last_cell {
                            s
                        } else {
                            format!("{s} & ")
                        }
                    }
                    Backend::Html => html_cell(&s, headerp, aligns.get(ci).copied().unwrap_or('l')),
                    Backend::Texinfo => {
                        if ci == 0 {
                            s
                        } else {
                            format!("\n@tab {s}")
                        }
                    }
                    Backend::Ascii => ascii.as_ref().unwrap().cell(ci, &s),
                    Backend::Org => s,
                }
            };
            let piece = if sep.is_some() || hsep.is_some() {
                if (sep.is_none() && !headerp) || last_cell {
                    if backend.is_some() { with_backend(body) } else { body }
                } else {
                    let s = match (&sep, &hsep) {
                        (Some(s), Some(h)) => {
                            if headerp {
                                h
                            } else {
                                s
                            }
                        }
                        (Some(s), None) => s,
                        (None, Some(h)) => h,
                        _ => unreachable!(),
                    };
                    format!("{body}{}", s.as_str().unwrap_or(""))
                }
            } else if backend.is_some() {
                with_backend(body)
            } else {
                body
            };
            joined.push_str(&piece);
        }
        let cells_list = lisp::list(plain);
        let gp = |k: &str| params.get(k);
        let line = if let (Some(f), true) = (gp(":hllfmt"), last_header) {
            apply(&mut i, &f, ":hllfmt", &[cells_list.clone()])?
        } else if let (Some(f), true) = (gp(":hlfmt"), headerp) {
            apply(&mut i, &f, ":hlfmt", &[cells_list.clone()])?
        } else if let (Some(f), true) = (gp(":llfmt"), last) {
            apply(&mut i, &f, ":llfmt", &[cells_list.clone()])?
        } else if let Some(f) = gp(":lfmt") {
            apply(&mut i, &f, ":lfmt", &[cells_list.clone()])?
        } else {
            let wrap = |i: &mut Interp, a: &str, z: &str| -> Result<Option<String>, String> {
                let s = match gp(a) {
                    Some(v) => apply(i, &v, a, &[])?.unwrap_or_default(),
                    None => String::new(),
                };
                let e = match gp(z) {
                    Some(v) => apply(i, &v, z, &[])?.unwrap_or_default(),
                    None => String::new(),
                };
                Ok(Some(format!("{s}{joined}{e}")))
            };
            if (gp(":hllstart").is_some() || gp(":hllend").is_some()) && last_header {
                wrap(&mut i, ":hllstart", ":hllend")?
            } else if (gp(":hlstart").is_some() || gp(":hlend").is_some()) && headerp {
                wrap(&mut i, ":hlstart", ":hlend")?
            } else if (gp(":llstart").is_some() || gp(":llend").is_some()) && last {
                wrap(&mut i, ":llstart", ":llend")?
            } else if gp(":lstart").is_some() || gp(":lend").is_some() {
                wrap(&mut i, ":lstart", ":lend")?
            } else {
                Some(match b {
                    Backend::Org if backend.is_some() => format!("|{joined}"),
                    Backend::Latex => {
                        let booktabs = latex_booktabs(params);
                        format!(
                            "{}{joined}\\\\{}",
                            if booktabs && first { "\\toprule\n" } else { "" },
                            if booktabs && last { "\n\\bottomrule" } else { "" }
                        )
                    }
                    Backend::Html => format!("<tr>\n{joined}</tr>"),
                    Backend::Texinfo => format!("{} {joined}", if headerp { "@headitem" } else { "@item" }),
                    Backend::Ascii => joined.clone(),
                    Backend::Org => joined.clone(),
                })
            }
        };
        if let Some(l) = line {
            out_rows.push(l);
        }
    }
    let splice = params.get(":splice").is_some();
    let (tstart, tend) = (params.get(":tstart"), params.get(":tend"));
    let wrapped_by_backend = backend.is_some() && tstart.is_none() && tend.is_none() && !splice;
    let contents = if b == Backend::Html {
        html_groups(&p.rows, &out_rows, &in_header)
    } else {
        out_rows.iter().map(|r| format!("{r}\n")).collect::<String>()
    };
    let mut s = String::new();
    if let Some(t) = &tstart
        && !splice
    {
        s.push_str(&apply(&mut i, t, ":tstart", &[])?.unwrap_or_default());
        s.push('\n');
    }
    if wrapped_by_backend {
        s.push_str(&match b {
            Backend::Latex => latex_table(&p, params, &aligns, &contents),
            Backend::Html => html_table(&p, params, &aligns, &contents),
            Backend::Texinfo => texinfo_table(&p, &contents),
            Backend::Ascii => ascii.as_ref().unwrap().table(&contents),
            Backend::Org => org_table(&contents),
        });
    } else if b == Backend::Ascii && !splice {
        s.push_str(&ascii.as_ref().unwrap().table(&contents));
    } else {
        s.push_str(&contents);
    }
    if let Some(t) = &tend
        && !splice
    {
        s.push_str(&apply(&mut i, t, ":tend", &[])?.unwrap_or_default());
    }
    if s.ends_with('\n') {
        s.pop();
    }
    Ok(s)
}

fn latex_booktabs(params: &Params) -> bool {
    params.get(":latex-tables-booktabs").is_some() || params.get(":booktabs").is_some()
}

fn latex_rule(params: &Params, first: bool, last: bool) -> String {
    if !latex_booktabs(params) {
        "\\hline".into()
    } else if first {
        "\\toprule".into()
    } else if last {
        "\\bottomrule".into()
    } else {
        "\\midrule".into()
    }
}

fn latex_table(p: &Prepared, params: &Params, aligns: &[char], contents: &str) -> String {
    let groups = column_groups(p);
    let mut spec = String::new();
    for (i, a) in aligns.iter().enumerate() {
        if groups.get(i).is_some_and(|g| g.0) && i > 0 && !spec.ends_with('|') {
            spec.push('|');
        } else if groups.get(i).is_some_and(|g| g.0) && i == 0 {
            spec.push('|');
        }
        spec.push(*a);
        if groups.get(i).is_some_and(|g| g.1) {
            spec.push('|');
        }
    }
    let env = params.str(":environment").or_else(|| params.str(":latex-default-table-environment")).unwrap_or_else(|| "tabular".into());
    format!("\\begin{{{env}}}{{{spec}}}\n{contents}\\end{{{env}}}")
}

fn html_cell(s: &str, header: bool, align: char) -> String {
    let class = match align {
        'r' => "org-right",
        'c' => "org-center",
        _ => "org-left",
    };
    let s = if s.is_empty() { "&nbsp;" } else { s };
    if header {
        format!("<th scope=\"col\" class=\"{class}\">{s}</th>\n")
    } else {
        format!("<td class=\"{class}\">{s}</td>\n")
    }
}

fn html_groups(rows: &[Row], out: &[String], in_header: &[bool]) -> String {
    // Data rows only (rules separate the groups).
    let mut groups: Vec<(bool, Vec<&String>)> = vec![];
    let mut k = 0;
    let mut new_group = true;
    for (i, r) in rows.iter().enumerate() {
        match r {
            Row::Hline => new_group = true,
            Row::Cells(_) => {
                let Some(line) = out.get(k) else { break };
                k += 1;
                if new_group || groups.is_empty() {
                    groups.push((in_header[i], vec![]));
                    new_group = false;
                }
                groups.last_mut().unwrap().1.push(line);
            }
        }
    }
    let mut s = String::new();
    for (header, lines) in groups {
        let tag = if header { "thead" } else { "tbody" };
        s.push_str(&format!("<{tag}>\n{}\n</{tag}>\n", lines.iter().map(|l| l.as_str()).collect::<Vec<_>>().join("\n\n")));
    }
    s
}

fn html_table(p: &Prepared, params: &Params, aligns: &[char], contents: &str) -> String {
    let attrs = if params.member(":attributes") {
        params.get(":attributes")
    } else if params.member(":html-table-attributes") {
        params.get(":html-table-attributes")
    } else {
        lisp::read("(:border \"2\" :cellspacing \"0\" :cellpadding \"6\" :rules \"groups\" :frame \"hsides\")").ok()
    };
    let attr_text: String = match attrs {
        Some(L::List(v)) => v
            .chunks(2)
            .filter_map(|q| {
                let k = q[0].sym_name()?.trim_start_matches(':').to_owned();
                let v = q.get(1).map(|x| x.as_str().map(str::to_owned).unwrap_or_else(|| lisp::princ(x)))?;
                Some(format!(" {k}=\"{v}\""))
            })
            .collect(),
        _ => String::new(),
    };
    let groups = column_groups(p);
    let mut colgroups: Vec<Vec<String>> = vec![vec![]];
    for (i, a) in aligns.iter().enumerate() {
        if groups.get(i).is_some_and(|g| g.0) && !colgroups.last().unwrap().is_empty() {
            colgroups.push(vec![]);
        }
        let class = match a {
            'r' => "org-right",
            'c' => "org-center",
            _ => "org-left",
        };
        colgroups.last_mut().unwrap().push(format!("<col  class=\"{class}\" />"));
        if groups.get(i).is_some_and(|g| g.1) && i + 1 < aligns.len() {
            colgroups.push(vec![]);
        }
    }
    let cg = colgroups.iter().filter(|g| !g.is_empty()).map(|g| format!("<colgroup>\n{}\n</colgroup>", g.join("\n\n"))).collect::<Vec<_>>().join("\n");
    format!("<table{attr_text}>\n\n\n{cg}\n{contents}</table>")
}

fn texinfo_table(p: &Prepared, contents: &str) -> String {
    let ncol = p.rows.iter().filter_map(Row::cells).map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0; ncol];
    for r in p.rows.iter().filter_map(Row::cells) {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let spec = widths.iter().map(|w| format!("{{{}}}", "a".repeat(*w))).collect::<Vec<_>>().join(" ");
    format!("@multitable {spec}\n{contents}@end multitable")
}

fn org_table(contents: &str) -> String {
    let lines: Vec<String> = contents.lines().map(String::from).collect();
    let mut d = Doc { lines, ..Doc::default() };
    if !d.lines.is_empty() {
        d.align();
    }
    d.lines.iter().map(|l| format!("{l}\n")).collect()
}

/// The ASCII (Unicode) table layout.
struct AsciiLayout {
    widths: Vec<usize>,
    aligns: Vec<char>,
    narrow: Vec<Option<usize>>,
}

fn ascii_layout(p: &Prepared, params: &Params) -> AsciiLayout {
    let rows: Vec<&Vec<String>> = p.rows.iter().filter_map(Row::cells).collect();
    let ncol = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let cookies = cookie_widths(p);
    let narrow = params.get(":narrow").is_some() || params.get(":ascii-table-widen-columns").is_none() && params.member(":ascii-table-widen-columns");
    let mut widths = vec![0; ncol];
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(width(c));
        }
    }
    let narrow_w: Vec<Option<usize>> = (0..ncol).map(|i| if narrow { cookies.get(i).copied().flatten() } else { None }).collect();
    for (i, w) in narrow_w.iter().enumerate() {
        if let Some(w) = w {
            widths[i] = *w;
        }
    }
    AsciiLayout { widths, aligns: alignments(p), narrow: narrow_w }
}

impl AsciiLayout {
    fn total(&self) -> usize {
        self.widths.iter().map(|w| w + 2).sum()
    }

    fn rule(&self, c: char) -> String {
        c.to_string().repeat(self.total())
    }

    fn cell(&self, i: usize, s: &str) -> String {
        let w = self.widths.get(i).copied().unwrap_or(0);
        let mut s = s.to_owned();
        if self.narrow.get(i).copied().flatten().is_some() && width(&s) > w {
            let keep: String = s.chars().take(w.saturating_sub(2)).collect();
            s = format!("{keep}=>");
        }
        align_field(&s, w, self.aligns.get(i).copied().unwrap_or('l'))
    }

    fn table(&self, contents: &str) -> String {
        format!("{}\n{contents}{}\n", self.rule('━'), self.rule('━'))
    }
}

// ---- the specializations ----

/// Run translator `name` (orgtbl-to-*) on `table`.
pub fn translate(name: &str, table: &[Row], params: &Params) -> Result<String, String> {
    let p = |s: &str| Params::read(s).unwrap_or_default();
    match name {
        "orgtbl-to-generic" => to_generic(table, params),
        "orgtbl-to-tsv" => to_generic(table, &p("(:sep \"\\t\")").with(params)),
        "orgtbl-to-csv" => {
            let base = Params(vec![lisp::sym(":sep"), lisp::str_(","), lisp::sym(":fmt"), lisp::sym("org-quote-csv-field")]);
            let mut out = base.with(params);
            // org-quote-csv-field as a lambda.
            if out.get(":fmt").as_ref().and_then(L::sym_name) == Some("org-quote-csv-field") {
                out = out.set(
                    ":fmt",
                    lisp::read("(lambda (s) (if (string-match \"[\\\",]\" s) (concat \"\\\"\" (mapconcat 'identity (split-string s \"\\\"\") \"\\\"\\\"\") \"\\\"\") s))").unwrap(),
                );
            }
            to_generic(table, &out)
        }
        "orgtbl-to-latex" => {
            let mut base = Params(vec![lisp::sym(":backend"), lisp::sym("latex")]);
            if let Some(b) = params.get(":booktabs") {
                base = base.set(":latex-tables-booktabs", b);
            }
            if let Some(e) = params.get(":environment") {
                base = base.set(":latex-default-table-environment", e);
            }
            to_generic(table, &base.with(params))
        }
        "orgtbl-to-html" => to_generic(table, &Params(vec![lisp::sym(":backend"), lisp::sym("html")]).with(params)),
        "orgtbl-to-texinfo" => {
            let out = to_generic(table, &Params(vec![lisp::sym(":backend"), lisp::sym("texinfo")]).with(params))?;
            let columns = params.str(":columns").map(|w| if w.contains('{') || w.starts_with("@columnfractions ") { w } else { format!("@columnfractions {w}") });
            Ok(match columns {
                Some(c) => {
                    static MT: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^@multitable .*$"));
                    MT.replace(&out, format!("@multitable {c}").as_str()).into_owned()
                }
                None => out,
            })
        }
        "orgtbl-to-orgtbl" => to_generic(table, &params.with(&Params(vec![lisp::sym(":backend"), lisp::sym("org")]))),
        "orgtbl-to-table.el" => {
            let org = translate("orgtbl-to-orgtbl", table, params)?;
            Ok(org.replace("-|", "-+").replace("|-", "+-"))
        }
        "orgtbl-to-unicode" => to_generic(table, &Params(vec![lisp::sym(":backend"), lisp::sym("ascii")]).with(params)),
        _ => Err(format!("No such transformation function {name}")),
    }
}

/// org-table-collapse-header.
pub fn collapse_header(table: &[Row], separator: Option<&str>, max_header_lines: Option<usize>) -> Vec<Row> {
    let sep = separator.unwrap_or(" ");
    let max = max_header_lines.unwrap_or(4);
    let mut t: &[Row] = table;
    while let Some(Row::Hline) = t.first() {
        t = &t[1..];
    }
    let header: Vec<&Vec<String>> = t.iter().take_while(|r| **r != Row::Hline).filter_map(Row::cells).collect();
    let trailer = &t[header.len()..];
    if !trailer.is_empty() && header.len() <= max {
        let n = header.iter().map(|r| r.len()).min().unwrap_or(0);
        let merged: Vec<String> = (0..n).map(|i| header.iter().map(|r| r[i].as_str()).collect::<Vec<_>>().join(sep).trim().to_owned()).collect();
        let mut out = vec![Row::Cells(merged)];
        out.extend_from_slice(trailer);
        out
    } else {
        t.to_vec()
    }
}

// ---- radio tables ----

lazy_re!(SEND, r"(?i)^[ \t]*#\+ORGTBL[: \t][ \t]*SEND[ \t]+([^ \t\r\n]+)[ \t]+([^ \t\r\n]+)([ \t]+.*)?");

/// orgtbl-gather-send-defs: (name, transform, params) above the table.
pub fn gather_send_defs(d: &Doc) -> Vec<(String, String, String)> {
    let mut out = vec![];
    let mut l = d.table_begin();
    while l > 0 {
        l -= 1;
        match SEND.captures(&d.lines[l]) {
            Some(m) => out.insert(0, (m[1].to_owned(), m[2].to_owned(), m.get(3).map_or(String::new(), |x| x.as_str().to_owned()))),
            None => break,
        }
    }
    out
}

/// orgtbl-send-replace-tbl.
pub fn send_replace(d: &mut Doc, name: &str, text: &str) -> Result<(), String> {
    let n = regex::escape(name);
    let begin = re(&format!(r"BEGIN +RECEIVE +ORGTBL +{n}([ \t]|$)"));
    let end = re(&format!(r"END +RECEIVE +ORGTBL +{n}([ \t]|$)"));
    let mut found = false;
    let mut l = 0;
    while l < d.n() {
        if begin.is_match(&d.lines[l]) {
            found = true;
            let Some(e) = (l + 1..d.n()).find(|&i| end.is_match(&d.lines[i])) else {
                return Err(format!("Cannot find end of receiver location at {}", l + 2));
            };
            let new: Vec<String> = text.split('\n').map(String::from).collect();
            let k = new.len();
            let (pl, pc) = (d.l, d.c);
            d.lines.splice(l + 1..e, new);
            if pl >= e {
                d.goto(pl + k - (e - l - 1), pc);
            }
            l += k + 1;
        }
        l += 1;
    }
    if !found {
        return Err("No valid receiver location found in the buffer".into());
    }
    Ok(())
}

/// orgtbl-send-table.
pub fn send_table(d: &mut Doc, maybe: bool) -> Result<usize, String> {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    if d.interactive && !maybe {
        d.align();
    }
    let dests = gather_send_defs(d);
    let table = d.to_lisp();
    if dests.is_empty() {
        return if maybe { Ok(0) } else { Err("Don't know how to transform this table".into()) };
    }
    let mut n = 0;
    for (name, transform, params) in dests {
        let p = Params::read(&params)?;
        let text = translate(&transform, &table, &p)?;
        send_replace(d, &name, &text)?;
        n += 1;
    }
    d.message(format!("Table converted and installed at {n} receiver location{}", if n > 1 { "s" } else { "" }));
    Ok(n)
}

/// orgtbl-radio-table-templates: the template for this file type.
pub fn radio_template(d: &Doc) -> Option<&'static str> {
    let ext = d.path.as_ref().and_then(|p| p.extension()).and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    Some(match ext.as_str() {
        "tex" | "latex" | "sty" | "ltx" => "% BEGIN RECEIVE ORGTBL %n\n% END RECEIVE ORGTBL %n\n\\begin{comment}\n#+ORGTBL: SEND %n orgtbl-to-latex :splice nil :skip 0\n| | |\n\\end{comment}\n",
        "texi" | "texinfo" => "@c BEGIN RECEIVE ORGTBL %n\n@c END RECEIVE ORGTBL %n\n@ignore\n#+ORGTBL: SEND %n orgtbl-to-html :splice nil :skip 0\n| | |\n@end ignore\n",
        "html" | "htm" | "xhtml" => "<!-- BEGIN RECEIVE ORGTBL %n -->\n<!-- END RECEIVE ORGTBL %n -->\n<!--\n#+ORGTBL: SEND %n orgtbl-to-html :splice nil :skip 0\n| | |\n-->\n",
        "org" => "#+ BEGIN RECEIVE ORGTBL %n\n#+ END RECEIVE ORGTBL %n\n\n#+ORGTBL: SEND %n orgtbl-to-orgtbl :splice nil :skip 0\n| | |\n",
        _ if d.org => "#+ BEGIN RECEIVE ORGTBL %n\n#+ END RECEIVE ORGTBL %n\n\n#+ORGTBL: SEND %n orgtbl-to-orgtbl :splice nil :skip 0\n| | |\n",
        _ => return None,
    })
}

/// orgtbl-insert-radio-table.
pub fn insert_radio_table(d: &mut Doc, name: &str) -> Result<(), String> {
    let Some(t) = radio_template(d) else {
        return Err("No radio table setup defined for this mode".into());
    };
    let txt = t.replace("%n", name);
    if !d.bolp() {
        d.insert("\n");
    }
    let (l, c) = (d.l, d.c);
    let lines: Vec<String> = txt.strip_suffix('\n').unwrap_or(&txt).split('\n').map(String::from).collect();
    d.insert_lines(l, &lines);
    d.goto(l, c);
    Ok(())
}

/// The comment prefix for this file's type (`comment-start`).
pub fn comment_start(d: &Doc) -> &'static str {
    let ext = d.path.as_ref().and_then(|p| p.extension()).and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "tex" | "latex" | "sty" | "ltx" | "m" => "% ",
        "c" | "h" | "cc" | "cpp" | "hpp" | "rs" | "js" | "ts" | "java" | "go" | "swift" | "kt" | "scala" | "cs" => "// ",
        "el" | "lisp" | "scm" | "clj" => ";; ",
        "texi" | "texinfo" => "@c ",
        "sql" | "lua" | "hs" => "-- ",
        "org" => "# ",
        _ => "# ",
    }
}

lazy_re!(ORGTBL_LINE, r"(?i)^[ \t]*(\||#\+(tblfm|orgtbl|tblname):)");

/// orgtbl-toggle-comment.
pub fn toggle_comment(d: &mut Doc) -> Result<(), String> {
    let cs = comment_start(d);
    let is_commented = |s: &str| s.strip_prefix(cs).or_else(|| s.strip_prefix(cs.trim_end())).is_some_and(|r| ORGTBL_LINE.is_match(r));
    let commented = if is_commented(d.line()) {
        true
    } else if ORGTBL_LINE.is_match(d.line()) {
        false
    } else {
        return Err("Not at an org table".into());
    };
    let matches = |s: &str| if commented { is_commented(s) } else { ORGTBL_LINE.is_match(s) };
    let mut b = d.l;
    while b > 0 && matches(&d.lines[b - 1]) {
        b -= 1;
    }
    let mut e = d.l;
    while e + 1 < d.n() && matches(&d.lines[e + 1]) {
        e += 1;
    }
    for l in b..=e {
        let s = d.lines[l].clone();
        d.lines[l] = if commented {
            s.strip_prefix(cs).or_else(|| s.strip_prefix(cs.trim_end())).unwrap_or(&s).to_owned()
        } else {
            format!("{cs}{s}")
        };
    }
    Ok(())
}

/// orgtbl-ascii-plot: a new column with bars for the current one.
pub fn ascii_plot(d: &mut Doc, length: i64) -> Result<(), String> {
    let col = d.current_column();
    let mut table = d.to_lisp();
    while table.first() == Some(&Row::Hline) {
        table.remove(0);
    }
    let body: Vec<Row> = match table.iter().position(|r| *r == Row::Hline) {
        Some(i) if i + 1 < table.len() => table[i + 1..].to_vec(),
        Some(_) => vec![],
        None => table.clone(),
    };
    let body = if body.is_empty() { table } else { body };
    static NUM: LazyLock<Regex> = LazyLock::new(|| re(r"^[-+]?([0-9]*[.])?[0-9]*([eE][+-]?[0-9]+)?$"));
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for r in &body {
        let Some(c) = r.cells() else { continue };
        let Some(x) = c.get(col.saturating_sub(1)) else { continue };
        if NUM.is_match(x) {
            let v = lisp::string_to_number(x).f();
            min = min.min(v);
            max = max.max(v);
        }
    }
    super::edit::insert_column(d)?;
    super::edit::move_column(d, false)?;
    super::formula::analyze(d);
    let mut stored = super::formula::get_stored_formulas(d, false, None)?;
    let num = |v: f64| lisp::float_to_string(v).trim_end_matches(".0").to_owned();
    stored.insert(0, (format!("${}", col + 1), format!("'(orgtbl-ascii-draw ${col} {} {} {length})", num(min), num(max))));
    super::formula::store_formulas(d, &stored, None);
    super::formula::recalculate(d, super::formula::Recalc::All, false)
}

/// org-table-export: the file contents for FORMAT ("transform params").
pub fn export_text(table: &[Row], format: &str) -> Result<String, String> {
    static F: LazyLock<Regex> = LazyLock::new(|| re(r"^([^ \t\r\n]+)( +.*)?"));
    let m = F.captures(format).ok_or("TABLE_EXPORT_FORMAT invalid")?;
    let params = match m.get(2) {
        Some(p) => Params::read(p.as_str())?,
        None => Params::default(),
    };
    Ok(translate(&m[1], table, &params)? + "\n")
}

/// The default export format for a file name (org-table-export).
pub fn default_export_format(file: &str) -> String {
    const FORMATS: &[&str] = &[
        "orgtbl-to-tsv",
        "orgtbl-to-csv",
        "orgtbl-to-latex",
        "orgtbl-to-html",
        "orgtbl-to-generic",
        "orgtbl-to-texinfo",
        "orgtbl-to-orgtbl",
        "orgtbl-to-unicode",
    ];
    let ext = std::path::Path::new(file).extension().and_then(|e| e.to_str()).unwrap_or("");
    FORMATS
        .iter()
        .find(|f| !ext.is_empty() && f.ends_with(ext))
        .map(|s| (*s).to_owned())
        .unwrap_or_else(|| super::opt_string("org-table-export-default-format", "orgtbl-to-tsv"))
}

pub const EXPORT_FORMATS: &[&str] = &[
    "orgtbl-to-tsv",
    "orgtbl-to-csv",
    "orgtbl-to-latex",
    "orgtbl-to-html",
    "orgtbl-to-generic",
    "orgtbl-to-texinfo",
    "orgtbl-to-orgtbl",
    "orgtbl-to-unicode",
];
