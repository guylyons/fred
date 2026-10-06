//! org-macro.el: `#+MACRO` templates, the built-in macros (title,
//! author, email, date, keyword, n, property, time, input-file,
//! modification-time) and `(eval ...)` macros, expansion and
//! org-macro-replace-all for the exporter.
//!
//! ```ignore
//! let mut t = macros::initialize_templates(&text, &st, path, &[])?;
//! macros::replace_all(&mut text, &st, &mut t, &["TITLE"])?;
//! ```

use super::element::{self, Type};
use super::props::{Doc, Inherit};
use super::syntax::{self, Settings};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A macro template.
#[derive(Clone, Debug, PartialEq)]
pub enum Template {
    /// Text with `$1`… placeholders.
    Text(String),
    /// `(eval BODY)`: BODY evaluated with `$1`… bound to the arguments.
    Eval(String),
    /// `{{{date}}}` when `#+DATE` holds one timestamp: formatted with the
    /// first argument, else the keyword's value.
    Date {
        value: String,
        ts: [i64; 5],
    },
    Keyword,
    Counter,
    Property,
    Time,
    InputFile(String),
    ModificationTime(PathBuf),
}

/// org-macro-templates and the `n` macro's counters.
#[derive(Clone, Debug, Default)]
pub struct Templates {
    pub list: Vec<(String, Template)>,
    counters: HashMap<String, i64>,
}

/// Where a macro expands (the buffer text and the macro's position).
pub struct Ctx<'a> {
    pub text: &'a str,
    pub st: &'a Settings,
    pub pos: usize,
    pub path: Option<&'a Path>,
}

/// org-macro--find-keyword-value.
pub fn find_keyword_value(text: &str, name: &str, collect: bool) -> Option<String> {
    let re = regex::RegexBuilder::new(&format!(r"(?m)^[ \t]*#\+{}:(.*)$", regex::escape(name)))
        .case_insensitive(true)
        .build()
        .ok()?;
    let mut result: Option<String> = None;
    for c in re.captures_iter(text) {
        let v = c[1].trim().to_owned();
        if !collect {
            return Some(v);
        }
        result = Some(format!("{} {v}", result.unwrap_or_default()));
    }
    result.map(|r| r.trim().to_owned())
}

/// org-macro--find-date.
fn find_date(text: &str, st: &Settings) -> Option<Template> {
    let value = find_keyword_value(text, "DATE", false)?;
    let t = element::parse_secondary_string(&value, element::restriction(Type::Keyword), st);
    let kids = t.contents(t.root);
    if kids.len() == 1 && t.kind(kids[0]) == Type::Timestamp {
        let n = kids[0];
        let g = |k: &str| t.int(n, k).unwrap_or(0);
        return Some(Template::Date {
            value,
            ts: [
                g("year-start"),
                g("month-start"),
                g("day-start"),
                g("hour-start"),
                g("minute-start"),
            ],
        });
    }
    Some(Template::Text(value))
}

/// org-macro--set-templates: (name, value) pairs, None for a missing
/// keyword value.
fn set_templates(templates: Vec<(String, Option<Template>)>) -> Vec<(String, Template)> {
    let mut out: Vec<(String, Template)> = vec![];
    for (name, value) in templates {
        let old = out.iter().position(|(n, _)| *n == name);
        match (value, old) {
            (Some(v), Some(i)) => out[i].1 = v,
            (None, Some(_)) => {}
            (v, None) => out.insert(0, (name, v.unwrap_or(Template::Text(String::new())))),
        }
    }
    out
}

/// A `#+MACRO` definition's template (`(eval ...)` is checked, not run).
fn template_of(name: &str, def: &str) -> Result<Template, String> {
    if crate::org_re!(r"^\(eval\b").is_match(def) {
        let bad = || format!("Invalid definition for macro {name:?}");
        let form = super::sexp::read(def).map_err(|_| bad())?;
        let body = form.list().and_then(|l| l.get(1)).ok_or_else(bad)?;
        // The body as written (the reader normalizes; keep the text).
        let inner = def.trim()[1..def.trim().len() - 1].trim_start();
        let text = inner
            .strip_prefix("eval")
            .unwrap_or(inner)
            .trim()
            .to_owned();
        let _ = body;
        return Ok(Template::Eval(text));
    }
    Ok(Template::Text(def.to_owned()))
}

/// org-macro-initialize-templates for TEXT (with `default`, e.g.
/// org-export-global-macros).
pub fn initialize_templates(
    text: &str,
    st: &Settings,
    path: Option<&Path>,
    default: &[(String, String)],
) -> Result<Templates, String> {
    let mut pairs: Vec<(String, Option<Template>)> = vec![];
    for (n, d) in default {
        pairs.push((n.clone(), Some(template_of(n, d)?)));
    }
    // org-macro--collect-macros: #+MACRO keywords (pushed: last first),
    // then the keyword-based macros.
    let mut collected: Vec<(String, Option<Template>)> = vec![];
    let name_re = crate::org_re!(r"^(\S+)[ \t]*");
    for (k, v) in &st.keywords {
        if k == "MACRO"
            && let Some(c) = name_re.captures(v)
        {
            let name = c[1].to_owned();
            let def = &v[c.get(0).unwrap().end()..];
            collected.insert(0, (name.clone(), Some(template_of(&name, def)?)));
        }
    }
    pairs.extend(collected);
    let text_t = |v: Option<String>| v.map(Template::Text);
    pairs.push((
        "author".into(),
        text_t(find_keyword_value(text, "AUTHOR", true)),
    ));
    pairs.push((
        "email".into(),
        text_t(find_keyword_value(text, "EMAIL", false)),
    ));
    pairs.push((
        "title".into(),
        text_t(find_keyword_value(text, "TITLE", true)),
    ));
    pairs.push(("date".into(), find_date(text, st)));
    let mut list = set_templates(pairs);
    if let Some(p) = path.filter(|p| p.exists()) {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        list.push(("input-file".into(), Template::InputFile(name)));
        list.push((
            "modification-time".into(),
            Template::ModificationTime(p.to_path_buf()),
        ));
    }
    list.push(("keyword".into(), Template::Keyword));
    list.push(("n".into(), Template::Counter));
    list.push(("property".into(), Template::Property));
    list.push(("time".into(), Template::Time));
    Ok(Templates {
        list,
        counters: HashMap::new(),
    })
}

/// A string argument for the Lisp reader.
fn lisp_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The number of `$N` placeholders a template uses (org-macro--makeargs).
fn max_arg(s: &str) -> usize {
    crate::org_re!(r"\$([0-9]+)")
        .captures_iter(s)
        .filter_map(|c| c[1].parse().ok())
        .max()
        .unwrap_or(0)
}

/// Evaluate an `(eval ...)` macro body with its arguments.
fn eval(body: &str, args: &[String]) -> Result<String, String> {
    let n = max_arg(body);
    let binds: Vec<String> = (1..=n)
        .map(|i| {
            let v = args.get(i - 1).map_or("nil".to_owned(), |a| lisp_string(a));
            format!("(${i} {v})")
        })
        .collect();
    let form = format!("(quote (let ({}) {body}))", binds.join(" "));
    let out = super::table::lisp::eval_formula(&form)?;
    Ok(if out == "nil" { String::new() } else { out })
}

/// org-macro--counter-increment.
fn counter(counters: &mut HashMap<String, i64>, name: Option<&str>, action: Option<&str>) -> i64 {
    let name = name.map(str::trim).unwrap_or("").to_owned();
    let action = action.map(str::trim).filter(|a| !a.is_empty());
    let v = match action {
        None => counters.get(&name).copied().unwrap_or(0) + 1,
        Some("-") => counters.get(&name).copied().unwrap_or(1),
        Some(a) if a.bytes().all(|b| b.is_ascii_digit()) => a.parse().unwrap_or(1),
        Some(_) => 1,
    };
    counters.insert(name, v);
    v
}

/// The entry at line `l` of LINES found by a property-macro location
/// (`*Heading`, `#custom-id`, a target or NAME, or a heading title).
fn find_location(lines: &[String], st: &Settings, loc: &str) -> Option<usize> {
    let loc = loc.trim();
    let title = |l: &str| {
        syntax::headline(l, st)
            .map(|h| super::links::normalize(h.title(l), false, false))
            .unwrap_or_default()
    };
    if let Some(id) = loc.strip_prefix('#') {
        let i = lines.iter().position(|l| {
            super::props::parse_property(l)
                .is_some_and(|(k, v)| k.eq_ignore_ascii_case("CUSTOM_ID") && v == id)
        })?;
        return Some(syntax::heading_at_or_before(lines, i).unwrap_or(0));
    }
    if let Some(h) = loc.strip_prefix('*') {
        let h = super::links::normalize(h, false, false);
        return lines
            .iter()
            .position(|l| syntax::level(l).is_some() && title(l) == h);
    }
    let target = format!("<<{loc}>>");
    lines
        .iter()
        .position(|l| l.contains(&target))
        .or_else(|| {
            lines
                .iter()
                .position(|l| syntax::keyword_line(l).is_some_and(|(k, v)| k == "NAME" && v == loc))
        })
        .or_else(|| {
            lines
                .iter()
                .position(|l| syntax::level(l).is_some() && title(l) == loc)
        })
}

/// org-macro--get-property.
fn get_property(cx: &Ctx, property: &str, location: Option<&str>) -> Result<String, String> {
    let lines: Vec<String> = cx.text.lines().map(str::to_owned).collect();
    let mut l = cx.text[..cx.pos.min(cx.text.len())].matches('\n').count();
    if let Some(loc) = location.filter(|s| !s.trim().is_empty()) {
        l = find_location(&lines, cx.st, loc)
            .ok_or_else(|| format!("Macro property failed: cannot find location {loc}"))?;
    }
    let h = syntax::heading_at_or_before(&lines, l.min(lines.len().saturating_sub(1)));
    let doc = Doc::new(&lines, cx.st, cx.path);
    Ok(doc
        .get(h, property, Inherit::Selective, false)
        .unwrap_or_default())
}

impl Templates {
    /// The template for KEY (macro names ignore case).
    pub fn get(&self, key: &str) -> Option<&Template> {
        self.list
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(key))
            .map(|(_, t)| t)
    }

    /// org-macro-expand: None when no template is defined.
    pub fn expand(
        &mut self,
        key: &str,
        args: &[String],
        cx: &Ctx,
    ) -> Result<Option<String>, String> {
        let Some(t) = self.get(key).cloned() else {
            return Ok(None);
        };
        let arg = |i: usize| args.get(i).map(String::as_str);
        let v = match t {
            Template::Text(s) => crate::org_re!(r"\$[0-9]+")
                .replace_all(&s, |c: &regex::Captures| {
                    let i: usize = c[0][1..].parse().unwrap_or(0);
                    if i == 0 {
                        String::new()
                    } else {
                        args.get(i - 1).cloned().unwrap_or_default()
                    }
                })
                .into_owned(),
            Template::Eval(body) => eval(&body, args)?,
            Template::Date { value, ts } => match arg(0).filter(|a| !a.trim().is_empty()) {
                Some(fmt) => {
                    let tm = super::time::civil::Tm::at(ts[0], ts[1], ts[2], ts[3], ts[4]);
                    super::time::civil::format(fmt, tm)
                }
                None => value,
            },
            Template::Keyword => {
                find_keyword_value(cx.text, arg(0).unwrap_or(""), true).unwrap_or_default()
            }
            Template::Counter => counter(&mut self.counters, arg(0), arg(1)).to_string(),
            Template::Property => get_property(cx, arg(0).unwrap_or(""), arg(1))?,
            Template::Time => {
                super::time::civil::format(arg(0).unwrap_or(""), super::time::civil::now())
            }
            Template::InputFile(n) => n,
            Template::ModificationTime(p) => {
                let vc = arg(1).is_some_and(|a| !a.trim().is_empty());
                let secs = vc
                    .then(|| vc_modified_time(&p))
                    .flatten()
                    .or_else(|| {
                        std::fs::metadata(&p)
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64)
                    })
                    .unwrap_or(0);
                super::time::civil::format(
                    arg(0).unwrap_or(""),
                    super::time::civil::Tm::from_epoch(secs),
                )
            }
        };
        Ok(Some(v))
    }
}

/// org-macro--vc-modified-time: the last commit touching FILE (git).
fn vc_modified_time(file: &Path) -> Option<i64> {
    let dir = file.parent()?;
    let out = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct", "--"])
        .arg(file)
        .current_dir(dir)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// org-in-commented-heading-p at byte `pos` of TEXT.
fn in_commented_heading(text: &str, st: &Settings, pos: usize) -> bool {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    if lines.is_empty() {
        return false;
    }
    let l = text[..pos.min(text.len())]
        .matches('\n')
        .count()
        .min(lines.len() - 1);
    let mut h = syntax::heading_at_or_before(&lines, l);
    while let Some(x) = h {
        if syntax::headline(&lines[x], st).is_some_and(|hl| hl.commented) {
            return true;
        }
        h = syntax::parent(&lines, x);
    }
    false
}

/// A macro parsed at `pos` (org-element-macro-parser): (key, args, end).
fn parse_macro_at(text: &str, pos: usize) -> Option<(String, Vec<String>, usize)> {
    let c = crate::org_re!(r"^\{\{\{([a-zA-Z][-a-zA-Z0-9_]*)(\((?s:(.*?))\))?\}\}\}")
        .captures(&text[pos..])?;
    let args = c.get(3).map_or(vec![], |a| {
        let a = crate::org_re!(r"[ \t\r\n]+")
            .replace_all(a.as_str().trim(), " ")
            .into_owned();
        element::macro_extract_arguments(&a)
    });
    Some((c[1].to_lowercase(), args, pos + c.get(0).unwrap().end()))
}

/// org-macro-replace-all on TEXT: expand every macro (also in the
/// KEYWORDS and their EXPORT_ node properties).
pub fn replace_all(
    text: &mut String,
    st: &Settings,
    templates: &mut Templates,
    keywords: &[&str],
    path: Option<&Path>,
) -> Result<(), String> {
    let props_re = regex::Regex::new(&format!(
        r"^EXPORT_(?:{})\+?$",
        keywords
            .iter()
            .map(|k| regex::escape(k))
            .collect::<Vec<_>>()
            .join("|")
    ))
    .ok();
    let start = crate::org_re!(r"\{\{\{[-A-Za-z0-9_]");
    let mut record: Vec<(usize, String, Vec<String>)> = vec![];
    let mut pos = 0;
    let mut tree: Option<(String, element::Tree)> = None;
    while let Some(m) = start.find_at(text, pos) {
        pos = m.end();
        if in_commented_heading(text, st, m.start()) {
            continue;
        }
        if tree.as_ref().is_none_or(|(s, _)| s != text) {
            tree = Some((text.clone(), element::parse_buffer(text, st)));
        }
        let t = &tree.as_ref().unwrap().1;
        let Some(d) = element::context(text, t, m.end()) else {
            continue;
        };
        let macro_ = match t.kind(d) {
            Type::Macro => {
                let key = t.str(d, "key").unwrap_or("").to_owned();
                let args: Vec<String> = t
                    .prop(d, "args")
                    .as_list()
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                t.begin(d).zip(t.end(d)).map(|(b, e)| (b, key, args, e))
            }
            Type::Keyword
                if t.str(d, "key")
                    .is_some_and(|k| keywords.iter().any(|x| x.eq_ignore_ascii_case(k))) =>
            {
                parse_macro_at(text, m.start()).map(|(k, a, e)| (m.start(), k, a, e))
            }
            Type::NodeProperty
                if props_re
                    .as_ref()
                    .zip(t.str(d, "key"))
                    .is_some_and(|(r, k)| r.is_match(k)) =>
            {
                parse_macro_at(text, m.start()).map(|(k, a, e)| (m.start(), k, a, e))
            }
            _ => None,
        };
        let Some((begin, key, args, end)) = macro_ else {
            continue;
        };
        let sig = (begin, text[begin..end].trim_end().to_owned(), args.clone());
        let cx = Ctx {
            text,
            st,
            pos: begin,
            path,
        };
        match templates.expand(&key, &args, &cx)? {
            _ if record.contains(&sig) => {
                return Err(format!("Circular macro expansion: {key}"));
            }
            Some(v) => {
                record.push(sig);
                let mut e = end;
                while e > begin && matches!(text.as_bytes()[e - 1], b' ' | b'\t') {
                    e -= 1;
                }
                text.replace_range(begin..e, &v);
                pos = begin;
            }
            None if key == "results" => {}
            None => return Err(format!("Undefined Org macro: {key}; aborting")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Initialize and replace all in TEXT.
    fn run(text: &str) -> Result<String, String> {
        let st = syntax::settings(text.lines(), None);
        let mut t = initialize_templates(text, &st, None, &[])?;
        let mut s = text.to_owned();
        replace_all(&mut s, &st, &mut t, &[], None)?;
        Ok(s)
    }

    fn last_line(text: &str) -> String {
        run(text).unwrap().lines().last().unwrap_or("").to_owned()
    }

    #[test]
    fn replace_all_spec() {
        // Ported from test-org/macro-replace-all.
        assert_eq!(
            run("#+MACRO: A B\n1 {{{A}}} 3").unwrap(),
            "#+MACRO: A B\n1 B 3"
        );
        assert_eq!(
            run("#+MACRO: macro $1 $2\n{{{macro(some,text)}}}").unwrap(),
            "#+MACRO: macro $1 $2\nsome text"
        );
        assert_eq!(
            last_line(
                "#+MACRO: add (eval (+ (string-to-number $1) (string-to-number $2)))\n{{{add(1,2)}}}"
            ),
            "3"
        );
        assert_eq!(
            run("#+MACRO: in inner\n#+MACRO: out {{{in}}} outer\n{{{out}}}").unwrap(),
            "#+MACRO: in inner\n#+MACRO: out {{{in}}} outer\ninner outer"
        );
        assert!(run("#+MACRO: mac1 {{{mac2}}}\n#+MACRO: mac2 {{{mac1}}}\n{{{mac1}}}").is_err());
        assert!(
            run("#+MACRO: macro expansion\n* COMMENT H\n{{{macro}}}")
                .unwrap()
                .contains("{{{macro}}}")
        );
        assert!(
            run("#+MACRO: macro expansion\n* COMMENT H1\n** H2\n{{{macro}}}")
                .unwrap()
                .contains("{{{macro}}}")
        );
        assert_eq!(
            run("{{{nope}}}").unwrap_err(),
            "Undefined Org macro: nope; aborting"
        );
        assert_eq!(run("{{{results(x)}}}").unwrap(), "{{{results(x)}}}");
        // Loading never evaluates.
        assert!(
            initialize_templates(
                "",
                &syntax::settings(
                    "#+MACRO: title (eval (eval-and-compile (error \"CVE-2024-30202\")))".lines(),
                    None
                ),
                None,
                &[]
            )
            .is_ok()
        );
    }

    #[test]
    fn property_macro() {
        assert_eq!(
            last_line("* H\n:PROPERTIES:\n:A: 1\n:END:\n{{{property(A)}}}"),
            "1"
        );
        assert_eq!(
            last_line("* H\n:PROPERTIES:\n:A: 1\n:END:\n{{{property(A,)}}}"),
            "1"
        );
        assert_eq!(
            last_line("* H1\n:PROPERTIES:\n:A: 1\n:END:\n* H2\n{{{property(A,*H1)}}}"),
            "1"
        );
        assert!(run("* H1\n:PROPERTIES:\n:A: 1\n:END:\n* H2\n{{{property(A,*???)}}}").is_err());
    }

    #[test]
    fn counters() {
        assert_eq!(last_line("{{{n}}} {{{n}}}"), "1 2");
        assert_eq!(last_line("{{{n()}}} {{{n}}}"), "1 2");
        assert_eq!(
            last_line("{{{n}}} {{{n(c1)}}} {{{n(c2)}}} {{{n(c1)}}}"),
            "1 1 1 2"
        );
        assert_eq!(last_line("{{{n(c,9)}}} {{{n(c)}}}"), "9 10");
        assert_eq!(last_line("{{{n(c,9)}}} {{{n(c,reset)}}}"), "9 1");
        assert_eq!(
            last_line("{{{n(c)}}} {{{n(c )}}} {{{n( c)}}} {{{n( c )}}}"),
            "1 2 3 4"
        );
        assert_eq!(
            last_line("{{{n(c,2)}}} {{{n(c, 3)}}} {{{n(c,5 )}}} {{{n(c, 7 )}}}"),
            "2 3 5 7"
        );
        assert_eq!(
            last_line("{{{n(,7)}}} {{{n(, -)}}} {{{n}}} {{{n(,- )}}} {{{n}}} {{{n(, - )}}}"),
            "7 7 8 8 9 9"
        );
        assert_eq!(last_line("{{{n(,9)}}} {{{n}}} {{{n(,reset)}}}"), "9 10 1");
        assert_eq!(last_line("{{{n(c,2)}}} {{{n(c,)}}}"), "2 3");
        assert_eq!(last_line("{{{n(,-)}}}"), "1");
        assert_eq!(last_line("{{{n}}} {{{n(,-)}}} {{{n}}}"), "1 1 2");
        assert_eq!(
            last_line("{{{n}}}.{{{n(c)}}} {{{n(,-)}}}.{{{n(c)}}} {{{n(,-)}}}.{{{n(c)}}}"),
            "1.1 1.2 1.3"
        );
    }

    #[test]
    fn keyword_macros() {
        assert_eq!(
            last_line("#+keyword: value\n{{{keyword(KEYWORD)}}}"),
            "value"
        );
        assert_eq!(last_line("#+author: me\n{{{author}}}"), "me");
        assert_eq!(
            last_line("#+author: author 1\n#+author: author 2\n{{{author}}}"),
            "author 1 author 2"
        );
        assert_eq!(run("{{{author}}}").unwrap(), "");
        assert_eq!(last_line("#+email: me@home\n{{{email}}}"), "me@home");
        assert_eq!(run("{{{email}}}").unwrap(), "");
        assert_eq!(last_line("#+title: Foo!\n{{{title}}}"), "Foo!");
        assert_eq!(
            last_line("#+title: Foo\n#+title: Bar!\n{{{title}}}"),
            "Foo Bar!"
        );
        assert_eq!(
            last_line("#+date: <2026-10-04 Sun>\n{{{date(%Y)}}} {{{date}}}"),
            "2026 <2026-10-04 Sun>"
        );
        assert_eq!(last_line("{{{time(%%)}}}"), "%");
    }

    #[test]
    fn macros_in_keywords() {
        let text = "#+MACRO: v 1.0\n#+TITLE: Version {{{v}}}";
        let st = syntax::settings(text.lines(), None);
        let mut t = initialize_templates(text, &st, None, &[]).unwrap();
        let mut s = text.to_owned();
        replace_all(&mut s, &st, &mut t, &["TITLE"], None).unwrap();
        assert_eq!(s, "#+MACRO: v 1.0\n#+TITLE: Version 1.0");
        // Defaults (org-export-global-macros) yield to the buffer's.
        let mut t = initialize_templates(
            "#+MACRO: x local",
            &syntax::settings("#+MACRO: x local".lines(), None),
            None,
            &[("x".into(), "global".into()), ("y".into(), "Y".into())],
        )
        .unwrap();
        let cx = Ctx {
            text: "",
            st: &st,
            pos: 0,
            path: None,
        };
        assert_eq!(t.expand("x", &[], &cx).unwrap().as_deref(), Some("local"));
        assert_eq!(t.expand("Y", &[], &cx).unwrap().as_deref(), Some("Y"));
    }
}
