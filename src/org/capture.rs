//! Capture (org-capture.el) and date trees (org-datetree.el).
//!
//! Adaptation: Emacs edits the inserted entry in an indirect buffer
//! narrowed to it. Fred has no indirect buffers, so the capture happens in
//! the target file's own buffer, narrowed to the inserted text; C-c C-c
//! widens, saves and returns, C-c C-k removes the text again.

use super::sexp::Sexp;
use super::syntax;
use super::{Prefix, ctx, fold, structure};
use crate::editor::Editor;
use std::path::{Path, PathBuf};

thread_local! {
    /// org-capture-last-stored-marker: (file, line).
    static LAST: std::cell::RefCell<Option<(PathBuf, usize)>> = const { std::cell::RefCell::new(None) };
}

/// A template from org-capture-templates.
#[derive(Clone, Debug)]
pub struct Template {
    pub keys: String,
    pub desc: String,
    pub kind: String,
    pub target: Sexp,
    pub template: Sexp,
    pub props: Sexp,
}

impl Template {
    fn prop(&self, k: &str) -> Option<&Sexp> {
        self.props.plist_get(k)
    }
    fn flag(&self, k: &str) -> bool {
        self.prop(k).is_some_and(Sexp::truthy)
    }
    fn num(&self, k: &str) -> Option<i64> {
        self.prop(k).and_then(Sexp::int)
    }
}

/// org-capture-templates (with group entries as (keys, desc) only).
pub fn templates() -> Vec<Result<Template, (String, String)>> {
    let v = super::sexp::option("org-capture-templates");
    let list: Vec<Sexp> = v.and_then(|v| v.list().map(<[Sexp]>::to_vec)).unwrap_or_default();
    let list = if list.is_empty() {
        vec![super::sexp::read("(\"t\" \"Task\" entry (file+headline \"\" \"Tasks\") \"* TODO %?\n  %u\n  %a\")").unwrap()]
    } else {
        list
    };
    list.iter()
        .filter_map(|e| {
            let items = e.list()?;
            let keys = items.first()?.str()?.to_owned();
            let desc = items.get(1).and_then(Sexp::str).unwrap_or("").to_owned();
            if items.len() <= 2 {
                return Some(Err((keys, desc)));
            }
            let kind = items.get(2).and_then(|x| x.str()).unwrap_or("entry").to_owned();
            Some(Ok(Template {
                keys,
                desc,
                kind,
                target: items.get(3).cloned().unwrap_or(Sexp::Nil),
                template: items.get(4).cloned().unwrap_or(Sexp::Nil),
                props: Sexp::List(items.get(5..).map(<[Sexp]>::to_vec).unwrap_or_default()),
            }))
        })
        .collect()
}

/// org-capture-expand-file.
fn expand_file(f: &Sexp) -> Result<PathBuf, String> {
    match f {
        Sexp::Str(s) if s.is_empty() => Ok(super::options::file("org-default-notes-file", "~/.notes")),
        Sexp::Str(s) => {
            let p = super::options::expand(s);
            Ok(if p.is_relative() { super::options::directory().join(p) } else { p })
        }
        Sexp::Sym(s) if s == "org-default-notes-file" => Ok(super::options::file("org-default-notes-file", "~/.notes")),
        Sexp::Sym(s) => match super::options::get(s) {
            Some(toml::Value::String(v)) => Ok(super::options::expand(&v)),
            _ => Err(format!("Invalid file location: {s}")),
        },
        other => Err(format!("Invalid file location: {other}")),
    }
}

// ---- dates ----

/// (year, month, day) of days since the epoch.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// ISO week (year, week) of a civil date.
fn iso_week(y: i64, m: i64, d: i64) -> (i64, i64) {
    let days = super::tags::days_from_civil(y, m, d);
    let wd = (days + 3).rem_euclid(7); // 0 = Monday
    let thursday = days - wd + 3;
    let (ty, _, _) = civil_from_days(thursday);
    let jan1 = super::tags::days_from_civil(ty, 1, 1);
    (ty, (thursday - jan1) / 7 + 1)
}

/// The datetree headline titles for a date and grouping, with the regexp
/// that recognises each level's siblings.
fn datetree_levels(y: i64, m: i64, d: i64, grouping: &[&str]) -> Vec<(String, &'static str)> {
    let days = super::tags::days_from_civil(y, m, d);
    let wd = (days + 4).rem_euclid(7) as usize;
    let (iy, w) = iso_week(y, m, d);
    let has_week = grouping.contains(&"week");
    let ny = if has_week { iy } else { y };
    let nm = if has_week {
        let thursday = days - (days + 3).rem_euclid(7) + 3;
        civil_from_days(thursday).1
    } else {
        m
    };
    let quarter = if has_week && !grouping.contains(&"month") { (1 + (w - 1) / 13).min(4) } else { 1 + (nm - 1) / 3 };
    let mut out = vec![];
    for g in grouping {
        match *g {
            "year" => out.push((ny.to_string(), r"^([12]\d{3})")),
            "quarter" => out.push((format!("{ny}-Q{quarter}"), r"^([12]\d{3}-Q[1-4])")),
            "month" => out.push((format!("{ny}-{nm:02} {}", MONTHS[(nm - 1) as usize]), r"^([12]\d{3}-[01]\d) \w+")),
            "week" => out.push((format!("{iy}-W{w:02}"), r"^([12]\d{3}-W[0-5]\d)")),
            "day" => out.push((format!("{y}-{m:02}-{d:02} {}", DAYS[wd]), r"^([12]\d{3}-[01]\d-[0-3]\d) \w+")),
            _ => {}
        }
    }
    out
}

/// org-datetree-find-create-hierarchy within lines `[lo, hi)` at
/// `level`: the heading line of the innermost entry, and whether it existed.
pub fn datetree_find_create(ed: &mut Editor, levels: &[(String, &str)], lo: usize, hi: usize, level: usize, add_ts: Option<String>) -> (usize, bool) {
    let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
    let st = super::settings(ed);
    let (mut lo, mut hi, mut level) = (lo, hi, level);
    let mut found = true;
    let mut at = lo;
    for (title, re) in levels {
        let re = regex::Regex::new(re).unwrap();
        let key = |t: &str| re.captures(t).map(|c| c[1].to_owned());
        let want = key(title).unwrap_or_default();
        let nstars = if odd { 2 * level - 1 } else { level };
        let mut sibling = None;
        for l in lo..hi.min(ed.line_count()) {
            let text = ed.buf.line(l);
            if syntax::level(&text) == Some(nstars)
                && let Some(h) = syntax::headline(&text, &st)
                && let Some(k) = key(h.title(&text))
                && k >= want
            {
                sibling = Some((l, k == want));
                break;
            }
        }
        match sibling {
            Some((l, true)) => {
                at = l;
                found = true;
            }
            other => {
                found = false;
                let ins = other.map_or(hi.min(ed.line_count()), |(l, _)| l);
                // Delete blank lines before the insertion point.
                let mut ins = ins;
                while ins > lo && ed.buf.line(ins - 1).trim().is_empty() && ins >= 1 {
                    super::delete_lines(ed, ins - 1, 1);
                    ins -= 1;
                    hi = hi.saturating_sub(1);
                }
                let mut lines = vec![];
                let blank = super::sexp::option("org-blank-before-new-entry").is_some_and(|v| {
                    v.list().is_some_and(|l| l.iter().any(|e| e.car().and_then(Sexp::sym) == Some("heading") && e.cdr().truthy() && e.cdr().sym() != Some("auto")))
                });
                if blank {
                    lines.push(String::new());
                }
                lines.push(format!("{} {title}", "*".repeat(nstars)));
                let n = lines.len();
                if ins >= ed.line_count() && ed.buf.len_bytes() == 0 {
                    super::splice(ed, 0, 1, &lines);
                } else {
                    super::insert_lines(ed, ins, &lines);
                }
                at = ins + n - 1;
            }
        }
        lo = at + 1;
        hi = syntax::subtree_end(&ed.buf, at);
        level += 1;
    }
    if !found && let Some(ts) = add_ts {
        super::insert_lines(ed, at + 1, &[ts]);
    }
    (at, found)
}

// ---- template filling ----

/// Expand the non-interactive escapes (%a %u %t ... %<fmt> %i %f %F).
pub struct Ctx {
    pub annotation: String,
    pub initial: String,
    pub file: String,
    pub time: i64,
    pub clipboard: String,
    pub clock_heading: String,
    pub clock_link: String,
}

fn strftime(fmt: &str, t: i64) -> String {
    let (y, m, d, hh, mm, wd) = super::localtime(t);
    let mut out = String::new();
    let mut it = fmt.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('Y') => out.push_str(&format!("{y:04}")),
            Some('y') => out.push_str(&format!("{:02}", y % 100)),
            Some('m') => out.push_str(&format!("{m:02}")),
            Some('d') => out.push_str(&format!("{d:02}")),
            Some('e') => out.push_str(&format!("{d:>2}")),
            Some('H') => out.push_str(&format!("{hh:02}")),
            Some('I') => out.push_str(&format!("{:02}", if hh % 12 == 0 { 12 } else { hh % 12 })),
            Some('M') => out.push_str(&format!("{mm:02}")),
            Some('S') => out.push_str("00"),
            Some('p') => out.push_str(if hh < 12 { "AM" } else { "PM" }),
            Some('A') => out.push_str(DAYS[wd as usize]),
            Some('a') => out.push_str(&DAYS[wd as usize][..3]),
            Some('B') => out.push_str(MONTHS[(m - 1) as usize]),
            Some('b') | Some('h') => out.push_str(&MONTHS[(m - 1) as usize][..3]),
            Some('j') => {
                let doy = super::tags::days_from_civil(y, m, d) - super::tags::days_from_civil(y, 1, 1) + 1;
                out.push_str(&format!("{doy:03}"));
            }
            Some('F') => out.push_str(&format!("{y:04}-{m:02}-{d:02}")),
            Some('R') => out.push_str(&format!("{hh:02}:{mm:02}")),
            Some('T') => out.push_str(&format!("{hh:02}:{mm:02}:00")),
            Some('V') => out.push_str(&format!("{:02}", iso_week(y, m, d).1)),
            Some('G') => out.push_str(&iso_week(y, m, d).0.to_string()),
            Some('u') => out.push_str(&(if wd == 0 { 7 } else { wd }).to_string()),
            Some('w') => out.push_str(&wd.to_string()),
            Some('%') => out.push('%'),
            Some(o) => {
                out.push('%');
                out.push(o);
            }
            None => out.push('%'),
        }
    }
    out
}

/// A `%` at byte `i` escaped by a backslash.
fn escaped(s: &str, i: usize) -> bool {
    let bs = s[..i].chars().rev().take_while(|&c| c == '\\').count();
    bs % 2 == 1
}

fn fill_static(t: &str, c: &Ctx) -> String {
    let re = crate::org_re!(r"%(:[-A-Za-z]+|<([^>\n]+)>|[aAcfFikKlLntTuUx])");
    let link_re = crate::org_re!(r"\[\[(.*?)\](\[.*?\])?\]");
    let a = if c.annotation == "[[]]" { String::new() } else { c.annotation.clone() };
    let v_cap_a = link_re.replace(&a, "[[$1][%^{Link description}]]").into_owned();
    let v_l = link_re.replace(&a, "[[$1]]").into_owned();
    let v_cap_l = link_re.replace(&a, "$1").into_owned();
    let user = std::env::var("USER").unwrap_or_default();
    let mut out = String::new();
    let mut last = 0;
    for m in re.captures_iter(t) {
        let all = m.get(0).unwrap();
        if escaped(t, all.start()) {
            continue;
        }
        out.push_str(&t[last..all.start()]);
        last = all.end();
        let v = &m[1];
        let rep = match v.chars().next().unwrap() {
            '<' => strftime(&m[2], c.time),
            ':' => String::new(),
            'i' => {
                // Repeat the line's leading text on every line of %i.
                let line_start = out.rfind('\n').map_or(0, |i| i + 1);
                let lead: String = out[line_start..].chars().take_while(|c| c.is_whitespace()).collect();
                c.initial.replace('\n', &format!("\n{lead}"))
            }
            'a' => a.clone(),
            'A' => v_cap_a.clone(),
            'c' => c.clipboard.clone(),
            'f' => Path::new(&c.file).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            'F' => c.file.clone(),
            'k' => c.clock_heading.clone(),
            'K' => c.clock_link.clone(),
            'l' => v_l.clone(),
            'L' => v_cap_l.clone(),
            'n' => super::options::string("user-full-name", &user),
            't' => super::timestamp(c.time, false, false),
            'T' => super::timestamp(c.time, true, false),
            'u' => super::timestamp(c.time, false, true),
            'U' => super::timestamp(c.time, true, true),
            'x' => crate::clipboard::get().unwrap_or_default(),
            _ => String::new(),
        };
        out.push_str(&rep);
    }
    out.push_str(&t[last..]);
    out
}

/// %[file] and %(sexp) expansions.
fn fill_files_and_sexps(t: &str) -> String {
    let re = crate::org_re!(r"%\[(.+?)\]");
    let t = re
        .replace_all(t, |c: &regex::Captures| {
            let f = super::options::expand(&c[1]);
            std::fs::read_to_string(&f).unwrap_or_else(|e| format!("%![could not insert {}: {e}]", f.display()))
        })
        .into_owned();
    // %(sexp): evaluated by Emacs, when available.
    let mut out = String::new();
    let mut rest = t.as_str();
    while let Some(i) = rest.find("%(") {
        if escaped(rest, i) {
            out.push_str(&rest[..i + 2]);
            rest = &rest[i + 2..];
            continue;
        }
        out.push_str(&rest[..i]);
        let body = &rest[i + 1..];
        match super::sexp::read_prefix(body) {
            Ok((_, after)) => {
                let form = &body[..body.len() - after.len()];
                let val = std::process::Command::new("emacs")
                    .args(["--batch", "-Q", "--eval", &format!("(princ (format \"%s\" {form}))")])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default();
                out.push_str(&val);
                rest = after;
            }
            Err(_) => {
                out.push_str("%(");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Interactive escapes, one at a time (`%^{prompt|default|a|b}X`).
fn fill_interactive(ed: &mut Editor, text: String, strings: Vec<String>, all: Vec<String>, done: Box<dyn FnOnce(&mut Editor, String) + Send>) {
    let re = crate::org_re!(r"%\^(?:\{([^}]*)\})?([CgGLptTuU])?");
    let Some(m) = re.captures_iter(&text).find(|m| !escaped(&text, m.get(0).unwrap().start())) else {
        // %\N and %\*N back-references, then cleanup.
        let mut t = text;
        let r1 = crate::org_re!(r"%\\(\*?)([1-9][0-9]*)");
        t = r1
            .replace_all(&t, |c: &regex::Captures| {
                let n: usize = c[2].parse().unwrap_or(1);
                let src = if &c[1] == "*" { &all } else { &strings };
                src.get(n - 1).cloned().unwrap_or_default()
            })
            .into_owned();
        return done(ed, cleanup(&t));
    };
    let whole = m.get(0).unwrap().range();
    let items: Vec<String> = m.get(1).map(|x| x.as_str().split('|').map(str::to_owned).collect()).unwrap_or_default();
    let key = m.get(2).map(|x| x.as_str().to_owned());
    let prompt = items.first().cloned();
    let default = items.get(1).cloned();
    let completions: Vec<String> = items.get(2..).map(<[String]>::to_vec).unwrap_or_default();
    let (text_for_p, whole_p, strings_p, all_p) = (text.clone(), whole.clone(), strings.clone(), all.clone());
    let done_slot = std::sync::Arc::new(std::sync::Mutex::new(Some(done)));
    let done_p: Box<dyn FnOnce(&mut Editor, String) + Send> = {
        let slot = done_slot.clone();
        Box::new(move |ed, s| {
            if let Some(f) = slot.lock().unwrap().take() {
                f(ed, s)
            }
        })
    };
    let done: Box<dyn FnOnce(&mut Editor, String) + Send> = Box::new(move |ed, s| {
        if let Some(f) = done_slot.lock().unwrap().take() {
            f(ed, s)
        }
    });
    let next = move |ed: &mut Editor, value: String, as_string: bool| {
        let mut t = text;
        t.replace_range(whole, &value);
        let mut strings = strings;
        let mut all = all;
        if as_string {
            strings.push(value.clone());
        }
        all.push(value);
        fill_interactive(ed, t, strings, all, done);
    };
    match key.as_deref() {
        None => {
            let p = match (&prompt, &default) {
                (Some(p), Some(d)) => format!("{p} [{d}]: "),
                (Some(p), None) => format!("{p}: "),
                (None, Some(d)) => format!("Enter string [{d}]: "),
                (None, None) => "Enter string: ".into(),
            };
            let cont = move |ed: &mut Editor, v: String| {
                let v = if v.is_empty() { default.clone().unwrap_or_default() } else { v };
                next(ed, v, true);
            };
            if completions.is_empty() {
                super::read(ed, &p, "", cont);
            } else {
                super::complete(ed, &p, completions, false, cont);
            }
        }
        Some("g" | "G") => {
            let tags = super::tags::buffer_tags(ed);
            let p = prompt.map_or("Tags: ".into(), |p| format!("{p}: "));
            super::complete(ed, &p, tags, false, move |ed, v| {
                let v = v.split([':', ' ']).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(":");
                next(ed, if v.is_empty() { v } else { format!(":{v}:") }, false);
            });
        }
        Some("C" | "L") => {
            let link = key.as_deref() == Some("L");
            let clip = crate::clipboard::get().unwrap_or_else(|| ed.reg.text.clone());
            super::read(ed, "Clipboard/kill value: ", &clip, move |ed, v| {
                let v = if link { super::links::make_string(&v, None).unwrap_or(v) } else { v };
                next(ed, v, false);
            });
        }
        Some("p") => {
            let name = prompt.unwrap_or_default();
            let vals = super::props::values(ed, &name);
            let _ = next;
            super::complete(ed, &format!("{name} value: "), vals, false, move |ed, v| {
                // org-set-property on the template's entry: a drawer after
                // its heading line.
                let mut t = text_for_p;
                t.replace_range(whole_p, "");
                let pos = t.find('\n').map_or(t.len(), |i| i + 1);
                let drawer = format!(":PROPERTIES:\n:{name}: {v}\n:END:\n");
                if t[..pos].ends_with('\n') {
                    t.insert_str(pos, &drawer);
                } else {
                    t.push('\n');
                    t.push_str(&drawer);
                }
                let mut all = all_p;
                all.push(v);
                fill_interactive(ed, t, strings_p, all, done_p);
            });
        }
        Some(k) => {
            // t T u U: a date (and time) prompt, via the timestamp module.
            let with_time = k == "T" || k == "U";
            let inactive = k == "u" || k == "U";
            let p = prompt.unwrap_or_else(|| "Date".into());
            super::read(ed, &format!("{p}: "), "", move |ed, v| {
                let ts = super::read_date_timestamp(&v, with_time, inactive).unwrap_or_else(|| super::timestamp(super::now(), with_time, inactive));
                next(ed, ts, false);
            });
        }
    }
}

/// Trim leading blank lines, end with exactly one newline.
fn cleanup(t: &str) -> String {
    let t = t.trim_start_matches(['\n', ' ', '\t']);
    let start = t.len();
    let _ = start;
    let t = t.trim_end();
    if t.is_empty() { String::new() } else { format!("{}\n", t.replace('\t', "        ")) }
}

/// org-capture-fill-template.
pub fn fill_template(ed: &mut Editor, template: &str, c: Ctx, done: impl FnOnce(&mut Editor, String) + Send + 'static) {
    let t = fill_files_and_sexps(template);
    let t = fill_static(&t, &c);
    fill_interactive(ed, t, vec![], vec![], Box::new(done));
}

// ---- placement ----

/// Where and how to insert, decided in the target buffer.
struct Placed {
    /// First and last inserted lines.
    beg: usize,
    end: usize,
    /// Cursor (line, byte) at `%?`.
    cursor: (usize, usize),
}

fn template_text(t: &Template) -> String {
    let kind = t.kind.as_str();
    let text = match &t.template {
        Sexp::Str(s) => s.clone(),
        Sexp::List(v) if v.first().and_then(Sexp::sym) == Some("file") => {
            let f = v.get(1).and_then(Sexp::str).unwrap_or("");
            let p = super::options::expand(f);
            let p = if p.is_relative() { super::options::directory().join(p) } else { p };
            std::fs::read_to_string(&p).unwrap_or_else(|_| format!("* Template file {f:?} not found"))
        }
        Sexp::List(v) if v.first().and_then(Sexp::sym) == Some("function") => "* Template function not supported in Fred".into(),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        match kind {
            "item" => "- %?".into(),
            "checkitem" => "- [ ] %?".into(),
            "table-line" => "| %? |".into(),
            _ => "* %?\n  %a".into(),
        }
    } else {
        text
    }
}

/// Find (or create) the target heading in the target buffer: its line,
/// whether the target is an entry, an exact line, insert-here.
fn locate(ed: &mut Editor, t: &Template, here: Option<usize>) -> Result<(Option<usize>, bool, Option<usize>), String> {
    if let Some(l) = here {
        return Ok((Some(l), false, Some(l)));
    }
    let target = t.target.list().ok_or("Invalid capture target specification")?.to_vec();
    let kind = target.first().and_then(Sexp::sym).unwrap_or("");
    let st = super::settings(ed);
    match kind {
        "file" => Ok((None, false, None)),
        "file+headline" => {
            let headline = target.get(2).and_then(Sexp::str).map(str::to_owned);
            match headline {
                None => Ok((None, false, None)),
                Some(hd) => {
                    let found = (0..ed.line_count()).find(|&l| syntax::headline(&ed.buf.line(l), &st).is_some_and(|h| h.title(&ed.buf.line(l)) == hd));
                    match found {
                        Some(l) => Ok((Some(l), true, None)),
                        None => {
                            let n = ed.line_count();
                            if n == 1 && ed.buf.len_bytes() == 0 {
                                super::splice(ed, 0, 1, &[format!("* {hd}")]);
                                Ok((Some(0), true, None))
                            } else {
                                super::insert_lines(ed, n, &[format!("* {hd}")]);
                                Ok((Some(n), true, None))
                            }
                        }
                    }
                }
            }
        }
        "file+olp" | "file+olp+datetree" => {
            let olp: Vec<String> = target[2..].iter().filter_map(|x| x.str().map(str::to_owned)).collect();
            let mut lo = 0;
            let mut hi = ed.line_count();
            let mut at = None;
            let mut lmin = 1;
            for h in &olp {
                let hits: Vec<usize> = (lo..hi)
                    .filter(|&l| {
                        let text = ed.buf.line(l);
                        syntax::headline(&text, &st).is_some_and(|x| x.title(&text) == h && x.level >= lmin && x.level <= lmin + usize::from(st.opt_bool("org-odd-levels-only", false)))
                    })
                    .collect();
                match hits.as_slice() {
                    [] => return Err(format!("Heading not found on level {lmin}: {h}")),
                    [l] => {
                        at = Some(*l);
                        lmin = syntax::level(&ed.buf.line(*l)).unwrap() + 1;
                        lo = l + 1;
                        hi = syntax::subtree_end(&ed.buf, *l);
                    }
                    _ => return Err(format!("Heading not unique on level {lmin}: {h}")),
                }
            }
            if kind == "file+olp" {
                return Ok((at, at.is_some(), None));
            }
            let tree = t.prop(":tree-type").and_then(Sexp::sym).unwrap_or("day");
            let grouping: Vec<&str> = match tree {
                "week" => vec!["year", "week", "day"],
                "month" => vec!["year", "month"],
                _ => match t.prop(":tree-type").and_then(Sexp::list) {
                    Some(l) => l.iter().filter_map(Sexp::sym).collect(),
                    None => vec!["year", "month", "day"],
                },
            };
            let now = super::now();
            let extend = super::options::int("org-extend-today-until", 0);
            let (mut y, mut m, mut d, hh, _, _) = super::localtime(now);
            if hh < extend {
                let days = super::tags::days_from_civil(y, m, d) - 1;
                (y, m, d) = civil_from_days(days);
            }
            let levels = datetree_levels(y, m, d, &grouping);
            let add_ts = match super::sexp::option("org-datetree-add-timestamp") {
                None | Some(Sexp::Nil) => None,
                Some(v) => {
                    let inactive = v.sym() == Some("inactive");
                    Some(super::timestamp(super::tags::days_from_civil(y, m, d) * 86400 - local_offset(), false, inactive))
                }
            };
            let (lo, hi, level) = match at {
                Some(a) => (a + 1, syntax::subtree_end(&ed.buf, a), syntax::level(&ed.buf.line(a)).unwrap() + 1),
                None => {
                    // The legacy DATE_TREE / WEEK_TREE property.
                    let prop = if grouping.contains(&"week") { "WEEK_TREE" } else { "DATE_TREE" };
                    let p = (0..ed.line_count()).find(|&l| super::props::parse_property(&ed.buf.line(l)).is_some_and(|(k, _)| k.eq_ignore_ascii_case(prop)));
                    match p.and_then(|p| fold::back_to_heading(ed, p)) {
                        Some(h) => (h + 1, syntax::subtree_end(&ed.buf, h), syntax::level(&ed.buf.line(h)).unwrap() + 1),
                        None => (0, ed.line_count(), 1),
                    }
                }
            };
            let (day, _) = datetree_find_create(ed, &levels, lo, hi, level, add_ts);
            Ok((Some(day), true, None))
        }
        "file+regexp" => {
            let re = target.get(2).and_then(Sexp::str).ok_or("Invalid capture target specification")?;
            let r = super::re::compile(re, false)?;
            let n = ed.line_count();
            match (0..n).find(|&l| r.is_match(&ed.buf.line(l))) {
                Some(l) => Ok((Some(l), ctx::at_heading(ed, l), Some(l))),
                None => Err("No match for target regexp in file".into()),
            }
        }
        "id" => Err("id targets are resolved before opening the buffer".into()),
        "clock" => Err("No running clock that could be used as capture target".into()),
        other => Err(format!("Invalid capture target specification: {other}")),
    }
}

fn local_offset() -> i64 {
    let (y, m, d, hh, mm, _) = super::localtime(0);
    let local = super::tags::days_from_civil(y, m, d) * 86400 + hh * 3600 + mm * 60;
    local
}

/// Count the blank lines that should precede/follow (props or default).
fn empty_lines(t: &Template, which: &str) -> usize {
    t.num(which).or_else(|| t.num(":empty-lines")).unwrap_or(0).max(0) as usize
}

/// Insert the filled text into the target buffer.
fn place(ed: &mut Editor, t: &Template, text: &str, heading: Option<usize>, entry: bool, exact: Option<usize>, here: bool) -> Result<Placed, String> {
    let prepend = t.flag(":prepend");
    let mut text = text.to_owned();
    let cursor_marker = "\u{1}";
    text = text.replacen("%?", cursor_marker, 1);
    match t.kind.as_str() {
        "item" | "checkitem" => place_item(ed, t, &text, heading, entry, exact, here, prepend),
        "table-line" => place_table_line(ed, t, &text, heading, entry, exact, here, prepend),
        "plain" => {
            let at = if here {
                exact.unwrap_or(ed.cur.line)
            } else if entry && let Some(h) = heading {
                if prepend { super::todo::end_of_meta_data(ed, h, true) } else { syntax::entry_end(&ed.buf, h) }
            } else if let Some(x) = exact {
                x
            } else if prepend {
                0
            } else {
                ed.line_count()
            };
            insert_block(ed, t, at, &text)
        }
        _ => {
            let mut tmpl = text.clone();
            if syntax::level(tmpl.trim_start_matches('\u{1}')).is_none() && !tmpl.starts_with('*') {
                tmpl = format!("* {tmpl}");
            }
            let stripped = tmpl.replace(cursor_marker, "");
            if !structure::kill_is_subtree(&stripped) {
                return Err("Template is not a valid Org entry or tree".into());
            }
            let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
            let (level, at) = if here {
                let l = exact.unwrap_or(ed.cur.line);
                let lv = fold::back_to_heading(ed, l).map_or(1, |h| syntax::level(&ed.buf.line(h)).unwrap());
                (structure::valid_level(odd, lv, 0), l + 1)
            } else if entry && let Some(h) = heading {
                let lv = structure::valid_level(odd, syntax::level(&ed.buf.line(h)).unwrap_or(1), 1);
                let at = if prepend { syntax::next_heading(&ed.buf, h, usize::MAX).unwrap_or(ed.line_count()) } else { syntax::subtree_end(&ed.buf, h) };
                (lv, at)
            } else if prepend {
                (1, (0..ed.line_count()).find(|&l| ctx::at_heading(ed, l)).unwrap_or(ed.line_count()))
            } else {
                (1, ed.line_count())
            };
            // Re-level the template to `level`.
            let first = tmpl.lines().find_map(|l| syntax::level(l.trim_start_matches('\u{1}'))).unwrap_or(1);
            let shift = level as i64 - first as i64;
            let releveled: String = tmpl
                .split_inclusive('\n')
                .map(|l| match syntax::level(l.trim_start_matches('\u{1}')) {
                    Some(n) if shift != 0 => {
                        let lead = &l[..l.len() - l.trim_start_matches('\u{1}').len()];
                        format!("{lead}{}{}", "*".repeat((n as i64 + shift).max(1) as usize), &l[lead.len() + n..])
                    }
                    _ => l.to_owned(),
                })
                .collect();
            // Blank lines before: :empty-lines-before, else org-blank-before-new-entry.
            let before = t.num(":empty-lines-before").or_else(|| t.num(":empty-lines")).map(|n| n.max(0) as usize).unwrap_or_else(|| {
                let b = super::buf::EBuf::load(ed);
                let mut c = super::buf::EBuf::new(&b.s, b.pos_of_line(at.min(ed.line_count().saturating_sub(1))));
                let auto = c.previous_line_empty();
                let _ = &mut c;
                usize::from(auto && at > 0 && !ed.buf.line(at.saturating_sub(1)).trim().is_empty())
            });
            insert_with_blanks(ed, at, &releveled, before, empty_lines(t, ":empty-lines-after"))
        }
    }
}

/// Insert `text` (ending in \n) at line `at`, managing blank lines.
fn insert_with_blanks(ed: &mut Editor, at: usize, text: &str, before: usize, after: usize) -> Result<Placed, String> {
    let mut at = at.min(ed.line_count());
    let empty_buffer = ed.line_count() == 1 && ed.buf.len_bytes() == 0;
    // org-capture-empty-lines-before: remove blank lines before, add N.
    if !empty_buffer {
        while at > 0 && ed.buf.line(at - 1).trim().is_empty() {
            super::delete_lines(ed, at - 1, 1);
            at -= 1;
        }
    }
    let mut lines: Vec<String> = vec![String::new(); if at == 0 || empty_buffer { 0 } else { before }];
    let blanks = lines.len();
    let body: Vec<String> = text.strip_suffix('\n').unwrap_or(text).split('\n').map(str::to_owned).collect();
    let n = body.len();
    lines.extend(body);
    // org-capture-empty-lines-after: blank lines after the inserted text.
    if !empty_buffer {
        while at < ed.line_count() && ed.buf.line(at).trim().is_empty() && at + 1 < ed.line_count() {
            super::delete_lines(ed, at, 1);
        }
    }
    let at_end = at >= ed.line_count();
    lines.extend(std::iter::repeat_n(String::new(), if at_end { 0 } else { after }));
    if empty_buffer {
        super::splice(ed, 0, 1, &lines);
    } else {
        super::insert_lines(ed, at, &lines);
    }
    let beg = at + blanks;
    let end = beg + n - 1;
    let cursor = find_cursor(ed, beg, end);
    Ok(Placed { beg, end, cursor })
}

/// Remove the `%?` marker, returning where it was (else the start).
fn find_cursor(ed: &mut Editor, beg: usize, end: usize) -> (usize, usize) {
    for l in beg..=end.min(ed.line_count() - 1) {
        let t = ed.buf.line(l);
        if let Some(i) = t.find('\u{1}') {
            super::set_line(ed, l, &t.replacen('\u{1}', "", 1));
            return (l, i);
        }
    }
    (beg, 0)
}

fn insert_block(ed: &mut Editor, t: &Template, at: usize, text: &str) -> Result<Placed, String> {
    insert_with_blanks(ed, at, text, empty_lines(t, ":empty-lines-before"), empty_lines(t, ":empty-lines-after"))
}

#[allow(clippy::too_many_arguments)]
fn place_item(ed: &mut Editor, t: &Template, text: &str, heading: Option<usize>, entry: bool, exact: Option<usize>, here: bool, prepend: bool) -> Result<Placed, String> {
    let mut tmpl: String = text.lines().map(str::trim_start).collect::<Vec<_>>().join("\n");
    if ctx::item_bullet(tmpl.trim_start_matches('\u{1}')).is_none() {
        tmpl = format!("- {}", tmpl.split('\n').collect::<Vec<_>>().join("\n  "));
    }
    let (beg, end) = if let Some(x) = exact {
        (x, if here { x } else { heading.map_or(ed.line_count(), |h| syntax::entry_end(&ed.buf, h)) })
    } else if entry && let Some(h) = heading {
        (h + 1, syntax::entry_end(&ed.buf, h))
    } else {
        (0, ed.line_count())
    };
    // The first plain list in the area.
    let first_item = (beg..end).find(|&l| ctx::at_item(ed, l));
    let (at, indent, in_list) = match first_item {
        Some(l) => {
            let ind = ctx::item_bullet(&ed.buf.line(l)).unwrap().0;
            if prepend {
                (l, ind, true)
            } else {
                // The end of that list: last line before a blank line or
                // a less indented non-item.
                let mut e = l + 1;
                while e < end {
                    let tl = ed.buf.line(e);
                    if tl.trim().is_empty() {
                        break;
                    }
                    let i2 = tl.len() - tl.trim_start().len();
                    if i2 < ind || (i2 == ind && ctx::item_bullet(&tl).is_none()) {
                        break;
                    }
                    e += 1;
                }
                (e, ind, true)
            }
        }
        None => {
            let at = if here {
                beg
            } else if !prepend {
                end
            } else if heading.is_none() {
                beg
            } else {
                super::todo::end_of_meta_data(ed, heading.unwrap(), false).max(beg)
            };
            (at, 0, false)
        }
    };
    let text: String = tmpl.split('\n').map(|l| format!("{}{l}\n", " ".repeat(indent))).collect();
    let (before, after) = if in_list {
        (
            if prepend { 0 } else { t.num(":empty-lines-before").or(t.num(":empty-lines")).unwrap_or(0).clamp(0, 1) as usize },
            if prepend { t.num(":empty-lines-after").or(t.num(":empty-lines")).unwrap_or(0).clamp(0, 1) as usize } else { 0 },
        )
    } else {
        (empty_lines(t, ":empty-lines-before"), empty_lines(t, ":empty-lines-after"))
    };
    if in_list {
        // Inside a list: insert directly, keeping its blank lines.
        let lines: Vec<String> = std::iter::repeat_n(String::new(), before).chain(text.strip_suffix('\n').unwrap().split('\n').map(str::to_owned)).chain(std::iter::repeat_n(String::new(), after)).collect();
        let n = text.lines().count();
        super::insert_lines(ed, at, &lines);
        let b = at + before;
        let cursor = find_cursor(ed, b, b + n - 1);
        let _ = super::call(ed, "org-list-repair", Prefix::None);
        return Ok(Placed { beg: b, end: b + n - 1, cursor });
    }
    insert_with_blanks(ed, at, &text, before, after)
}

#[allow(clippy::too_many_arguments)]
fn place_table_line(ed: &mut Editor, t: &Template, text: &str, heading: Option<usize>, entry: bool, exact: Option<usize>, here: bool, prepend: bool) -> Result<Placed, String> {
    let tmpl = text.trim();
    let line = if tmpl.starts_with('|') { tmpl.to_owned() } else { format!("| {tmpl}") };
    let (beg, end) = if let Some(x) = exact {
        (x, if here { x } else { heading.map_or(ed.line_count(), |h| syntax::entry_end(&ed.buf, h)) })
    } else if !entry {
        (0, ed.line_count())
    } else {
        let h = heading.unwrap();
        (h + 1, syntax::entry_end(&ed.buf, h))
    };
    let first = (beg..end).find(|&l| ctx::at_table(ed, l));
    let (tbeg, tend) = match first {
        Some(l) => {
            let mut e = l;
            while e + 1 < ed.line_count() && ctx::at_table(ed, e + 1) {
                e += 1;
            }
            (l, e)
        }
        None => {
            let at = end.min(ed.line_count());
            super::insert_lines(ed, at, &["|   |".into(), "|---|".into()]);
            (at, at + 1)
        }
    };
    let pos = t.prop(":table-line-pos").and_then(Sexp::str).map(str::to_owned);
    let at = if here {
        exact.unwrap_or(tend + 1)
    } else if let Some(p) = pos {
        let re = regex::Regex::new(r"^(I+)([-+][0-9]+)$").unwrap();
        let c = re.captures(&p).ok_or_else(|| format!("Invalid table line specification {p:?}"))?;
        let nth = c[1].len();
        let delta: i64 = c[2].parse().unwrap();
        let hlines: Vec<usize> = (tbeg..=tend).filter(|&l| ed.buf.line(l).trim_start().starts_with("|-")).collect();
        let hl = *hlines.get(nth - 1).ok_or_else(|| format!("Invalid table line specification {p:?}"))?;
        ((hl as i64 + delta + if delta < 0 { 1 } else { 0 }) as usize).clamp(tbeg, tend + 1)
    } else if prepend {
        let hline = (tbeg..=tend).find(|&l| ed.buf.line(l).trim_start().starts_with("|-"));
        match hline {
            Some(h) => (h + 1..=tend).find(|&l| !ed.buf.line(l).trim_start().starts_with("|-")).unwrap_or(tend + 1),
            None => tend + 1,
        }
    } else {
        tend + 1
    };
    super::insert_lines(ed, at, &[line]);
    let cursor = find_cursor(ed, at, at);
    let _ = super::call(ed, "org-table-align", Prefix::None);
    Ok(Placed { beg: at, end: at, cursor })
}

/// What the capture buffer needs to finish.
#[derive(Clone, Debug)]
struct Pending {
    template: Template,
    file: PathBuf,
    beg: usize,
    origin: usize,
    lines_before: usize,
}

/// Start capturing with template `t`.
fn start(ed: &mut Editor, t: Template, goto: Prefix) {
    let annotation = super::links::store(ed, Prefix::None).ok().map(|(l, d)| super::links::make_string(&l, d.as_deref()).unwrap_or_default()).unwrap_or_default();
    let initial = ed.org_region.map(|(lo, hi)| (lo..=hi).map(|l| ed.buf.line(l)).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    let here = (goto == Prefix::Num(0)).then(|| (ed.path.clone(), ed.cur.line));
    let file = match (&here, t.target.list().and_then(|l| l.get(1)).cloned()) {
        (Some((Some(p), _)), _) => Ok(p.clone()),
        (Some((None, _)), _) => Err("Capture here needs a file buffer".to_owned()),
        (None, Some(f)) if t.target.list().and_then(|l| l.first()).and_then(Sexp::sym) != Some("id") => expand_file(&f),
        (None, _) => {
            // (id "ID") targets and (clock).
            Err("Capture target is not a file".into())
        }
    };
    let file = match file {
        Ok(f) => f,
        Err(e) if t.target.list().and_then(|l| l.first()).and_then(Sexp::sym) == Some("id") => {
            let id = t.target.list().and_then(|l| l.get(1)).and_then(|x| x.str().map(str::to_owned)).unwrap_or_default();
            let tc = t.clone();
            let orig_file = ed.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            let ctx = Ctx { annotation, initial, file: orig_file, time: super::now(), clipboard: ed.reg.text.clone(), clock_heading: String::new(), clock_link: String::new() };
            let text = template_text(&t);
            fill_template(ed, &text, ctx, move |ed, filled| {
                super::effect(ed, move |s| match super::find_id_file(s, &id) {
                    Some((f, line)) => finish_start(s, tc, f, filled, Some(line), None),
                    None => s.ed.set_err(format!("Cannot find target ID \"{id}\" ({e})")),
                });
            });
            return;
        }
        Err(e) => {
            ed.set_err(e);
            return;
        }
    };
    let orig_file = ed.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let ctx = Ctx { annotation, initial, file: orig_file, time: super::now(), clipboard: ed.reg.text.clone(), clock_heading: String::new(), clock_link: String::new() };
    let text = template_text(&t);
    let here_line = here.map(|(_, l)| l);
    fill_template(ed, &text, ctx, move |ed, filled| {
        super::effect(ed, move |s| finish_start(s, t, file, filled, None, here_line));
    });
}

/// Insert the filled template in the target buffer and enter it.
fn finish_start(s: &mut crate::session::Session, t: Template, file: PathBuf, filled: String, id_line: Option<usize>, here: Option<usize>) {
    let origin = s.org_current();
    let i = match s.org_buffer(&file) {
        Ok(i) => i,
        Err(e) => {
            // A new file: create its buffer anyway.
            if !file.exists() {
                let _ = std::fs::write(&file, "");
                match s.org_buffer(&file) {
                    Ok(i) => i,
                    Err(e) => return s.ed.set_err(e),
                }
            } else {
                return s.ed.set_err(e);
            }
        }
    };
    s.org_show(i);
    let ed = &mut s.ed;
    if ed.org.is_none() {
        ed.org = Some(Box::default());
    }
    fold::widen(ed);
    let lines_before = ed.line_count();
    ed.undo.begin(ed.cur.pos());
    let loc = match id_line {
        Some(l) => Ok((Some(l), true, None)),
        None => locate(ed, &t, here),
    };
    let placed = loc.and_then(|(h, entry, exact)| place(ed, &t, &filled, h, entry, exact, here.is_some()));
    ed.undo.end(ed.cur.pos());
    let p = match placed {
        Ok(p) => p,
        Err(e) => {
            s.org_show(origin);
            return s.ed.set_err(format!("Capture template `{}': {e}", t.keys));
        }
    };
    if !t.flag(":unnarrowed") {
        fold::narrow(ed, p.beg, p.end);
    }
    fold::show_all(ed, &[fold::Spec::Outline, fold::Spec::Block]);
    ed.set_cursor(p.cursor.0, p.cursor.1);
    let name = format!("CAPTURE-{}", file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
    ed.org_buffer_name = Some(name);
    ed.org_return = Some(origin);
    let pending = Pending { template: t.clone(), file: file.clone(), beg: p.beg, origin, lines_before };
    ed.org_finish = Some(super::FinishSlot::new(Box::new(move |s, _text, abort| finalize(s, pending, abort, false))));
    ed.mode = crate::editor::Mode::Insert;
    ed.set_msg("Capture: C-c C-c to finish, C-c C-w to refile, C-c C-k to abort");
    if t.flag(":clock-in") {
        let _ = super::call(&mut s.ed, "org-clock-in", Prefix::None);
    }
    if t.flag(":immediate-finish") {
        s.ed.mode = crate::editor::Mode::Normal;
        s.org_finish(false);
    }
}

/// org-capture-finalize (and -kill).
fn finalize(s: &mut crate::session::Session, p: Pending, abort: bool, stay: bool) {
    // org_finish already switched back to the origin; operate on the target.
    let file = p.file.clone();
    let res = s.org_with_file(&file, |ed| {
        ed.org_buffer_name = None;
        ed.org_return = None;
        ed.mode = crate::editor::Mode::Normal;
        let region = fold::narrowed(ed).unwrap_or((p.beg, p.beg + ed.line_count().saturating_sub(p.lines_before)));
        fold::widen(ed);
        if abort {
            let n = ed.line_count();
            let added = n.saturating_sub(p.lines_before);
            ed.undo.begin(ed.cur.pos());
            if added > 0 {
                super::delete_lines(ed, region.0, (region.1 - region.0 + 1).min(added + 1).min(n - region.0));
            } else {
                super::delete_lines(ed, region.0, region.1 - region.0 + 1);
            }
            ed.undo.end(ed.cur.pos());
            ed.set_msg("Capture process aborted and target buffer cleaned up");
            return None;
        }
        ed.set_cursor(region.0, 0);
        if let Some(h) = fold::back_to_heading(ed, region.0) {
            super::todo::update_parent_statistics(ed, h);
        }
        let _ = super::call(ed, "org-update-checkbox-count", Prefix::None);
        Some(region.0)
    });
    let line = match res {
        Ok(Some(l)) => l,
        Ok(None) => return,
        Err(e) => return s.ed.set_err(e),
    };
    LAST.with(|l| *l.borrow_mut() = Some((file.clone(), line)));
    if !p.template.flag(":no-save")
        && let Ok(i) = s.org_buffer(&file)
        && let Err(e) = s.org_save(i)
    {
        s.ed.set_err(e);
    }
    if p.template.flag(":jump-to-captured") || stay {
        let _ = s.org_visit(&file, line);
        s.ed.set_msg("This is the last note stored by a capture process");
    } else if s.org_current() != p.origin && p.origin < s.org_buffer_count() {
        s.org_show(p.origin);
    }
}

/// org-capture-goto-last-stored.
fn goto_last(ed: &mut Editor) -> Result<(), String> {
    let Some((f, l)) = LAST.with(|x| x.borrow().clone()) else { return Err("No last capture".into()) };
    super::effect(ed, move |s| {
        if let Err(e) = s.org_visit(&f, l) {
            s.ed.set_err(e);
        } else {
            s.ed.set_msg("This is the last note stored by a capture process");
        }
    });
    Ok(())
}

/// org-capture template selection (org-mks).
fn select(ed: &mut Editor, then: impl FnOnce(&mut Editor, Template) + Send + 'static) {
    let all = templates();
    let mut entries: Vec<(String, String)> = vec![(String::new(), "Select a capture template".into()), (String::new(), "=========================".into())];
    for e in &all {
        match e {
            Ok(t) => entries.push((t.keys.clone(), t.desc.clone())),
            Err((k, d)) => entries.push((String::new(), format!("[{k}]...  {d}"))),
        }
    }
    entries.push(("C".into(), "Customize org-capture-templates".into()));
    entries.push(("q".into(), "Abort".into()));
    super::menu(ed, "Template key:", entries, move |ed, k| {
        if k == "q" {
            ed.set_err("Abort");
            return;
        }
        if k == "C" {
            ed.pending_effect = Some(crate::ex::ExEffect::Open { path: crate::config::config_path(), line: 0, col: 0, pattern: Some("org-capture-templates".into()) });
            return;
        }
        if let Some(Ok(t)) = all.into_iter().find(|e| matches!(e, Ok(t) if t.keys == k)) {
            then(ed, t);
        }
    });
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-capture" => match arg {
            Prefix::U(1) => {
                select(ed, |ed, t| {
                    let file = match t.target.list().and_then(|l| l.get(1)).map(expand_file) {
                        Some(Ok(f)) => f,
                        _ => return ed.set_err("Invalid capture target"),
                    };
                    super::effect(ed, move |s| {
                        let i = match s.org_buffer(&file) {
                            Ok(i) => i,
                            Err(e) => return s.ed.set_err(e),
                        };
                        s.org_show(i);
                        match locate(&mut s.ed, &t, None) {
                            Ok((h, _, x)) => {
                                let l = x.or(h).unwrap_or(0);
                                s.ed.set_cursor(l, 0);
                            }
                            Err(e) => s.ed.set_err(e),
                        }
                    });
                });
                Ok(())
            }
            Prefix::U(2) => goto_last(ed),
            _ => {
                let key = super::take_call_arg();
                match key {
                    Some(k) => match templates().into_iter().find(|e| matches!(e, Ok(t) if t.keys == k)) {
                        Some(Ok(t)) => {
                            start(ed, t, arg);
                            Ok(())
                        }
                        _ => Err(format!("No capture template referred to by \"{k}\" keys")),
                    },
                    None => {
                        select(ed, move |ed, t| start(ed, t, arg));
                        Ok(())
                    }
                }
            }
        },
        "org-capture-string" => {
            super::read(ed, "Initial text: ", "", move |ed, text| {
                select(ed, move |ed, t| {
                    // Initial text replaces the region.
                    let n = ed.line_count();
                    let _ = n;
                    let tt = t.clone();
                    let mut t2 = tt;
                    t2.template = match &t.template {
                        Sexp::Str(s) => Sexp::Str(s.replace("%i", &text)),
                        o => o.clone(),
                    };
                    start(ed, t2, arg);
                });
            });
            Ok(())
        }
        "org-capture-finalize" => {
            if ed.org_finish.is_some() {
                let stay = !arg.is_none();
                if stay {
                    let path = ed.path.clone();
                    let line = ed.cur.line;
                    super::effect(ed, move |s| {
                        s.org_finish(false);
                        if let Some(p) = path {
                            let _ = s.org_visit(&p, line);
                        }
                    });
                } else {
                    super::effect(ed, |s| s.org_finish(false));
                }
                Ok(())
            } else {
                Err("This does not seem to be a capture buffer for Org mode".into())
            }
        }
        "org-capture-kill" => {
            if ed.org_finish.is_some() {
                super::effect(ed, |s| s.org_finish(true));
                Ok(())
            } else {
                Err("This does not seem to be a capture buffer for Org mode".into())
            }
        }
        "org-capture-refile" => {
            if ed.org_finish.is_none() {
                return Some(Err("This does not seem to be a capture buffer for Org mode".into()));
            }
            let h = fold::back_to_heading(ed, ed.cur.line);
            let path = ed.path.clone();
            super::effect(ed, move |s| {
                s.org_finish(false);
                if let (Some(p), Some(h)) = (path, h)
                    && s.org_visit(&p, h).is_ok()
                {
                    super::run(&mut s.ed, "org-refile", Prefix::None);
                }
            });
            Ok(())
        }
        "org-capture-goto-last-stored" => goto_last(ed),
        "org-capture-goto-target" => command(ed, "org-capture", Prefix::U(1)).unwrap(),
        "org-datetree-find-date-create" | "org-datetree-find-month-create" | "org-datetree-find-iso-week-create" => {
            let grouping: Vec<&str> = match name {
                "org-datetree-find-month-create" => vec!["year", "month"],
                "org-datetree-find-iso-week-create" => vec!["year", "week", "day"],
                _ => vec!["year", "month", "day"],
            };
            let (y, m, d, _, _, _) = super::localtime(super::now());
            let levels = datetree_levels(y, m, d, &grouping);
            let n = ed.line_count();
            let (l, _) = datetree_find_create(ed, &levels, 0, n, 1, None);
            ed.set_cursor(l, 0);
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_trees() {
        assert_eq!(civil_from_days(super::super::tags::days_from_civil(2026, 10, 4)), (2026, 10, 4));
        assert_eq!(iso_week(2026, 1, 1), (2026, 1));
        assert_eq!(iso_week(2027, 1, 1), (2026, 53));
        let l = datetree_levels(2026, 10, 4, &["year", "month", "day"]);
        assert_eq!(l.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["2026", "2026-10 October", "2026-10-04 Sunday"]);
        let mut e = super::super::tests::org("* 2025\n* 2027", "");
        let n = e.line_count();
        let (at, found) = datetree_find_create(&mut e, &l, 0, n, 1, None);
        assert!(!found);
        assert_eq!(e.buf.text(), "* 2025\n* 2026\n** 2026-10 October\n*** 2026-10-04 Sunday\n* 2027");
        assert_eq!(at, 3);
        let n = e.line_count();
        let (at2, found) = datetree_find_create(&mut e, &l, 0, n, 1, None);
        assert!(found);
        assert_eq!(at2, 3);
    }

    #[test]
    fn fills_templates() {
        let c = Ctx { annotation: "[[file:x.org::*H][H]]".into(), initial: "a\nb".into(), file: "/d/x.org".into(), time: 0, clipboard: "clip".into(), clock_heading: String::new(), clock_link: String::new() };
        let out = fill_static("* TICKET %?\nEntered on %u %a %l %L %f\n  - %i\n100\\%a", &c);
        assert!(out.starts_with("* TICKET %?\nEntered on ["));
        assert!(out.contains("[[file:x.org::*H][H]] [[file:x.org::*H]] file:x.org::*H x.org"));
        assert!(out.contains("  - a\n  b"));
        assert!(out.ends_with("100\\%a"));
        assert_eq!(strftime("%Y-%m-%d %A", 86400 * 10 - local_offset_test()), "1970-01-11 Sunday");
    }

    fn local_offset_test() -> i64 {
        local_offset()
    }
}
