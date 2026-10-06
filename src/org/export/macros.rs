//! The macro seam of the export pipeline.
//!
//! org-macro.el is being ported separately (src/org/macro.rs).  Export
//! calls macro expansion only through [`expand_macros`]
//! (org-macro-initialize-templates + org-macro-replace-all); at merge
//! time its body becomes a call into macro.rs.  Until then it carries
//! what export needs: {{{title}}}, {{{author}}}, {{{date}}}, {{{email}}},
//! {{{input-file}}}, {{{modification-time}}}, {{{n}}}, {{{keyword}}},
//! {{{property}}}, {{{time}}}, `#+MACRO` templates with `$1` arguments
//! and org-export-global-macros.  `(eval ...)` templates are not
//! evaluated (Fred has no Lisp): they expand to nothing.

use crate::org::element::{self, Type};
use crate::org::syntax::Settings;
use std::collections::HashMap;
use std::path::Path;

/// A macro template.
#[derive(Clone, Debug)]
enum Template {
    Text(String),
    /// A built-in macro computed from its arguments.
    Builtin(&'static str),
    /// `{{{date}}}` from a single-timestamp DATE: (timestamp text, raw value).
    Date(String, String),
}

/// org-macro--find-keyword-value: the value of keyword NAME (all of
/// them joined by spaces with COLLECT).
fn keyword_value(text: &str, name: &str, collect: bool) -> Option<String> {
    let mut out: Option<String> = None;
    for l in text.lines() {
        let Some((k, v)) = crate::org::syntax::keyword_line(l) else {
            continue;
        };
        if !k.eq_ignore_ascii_case(name) {
            continue;
        }
        if !collect {
            return Some(v.to_owned());
        }
        out = Some(format!("{} {v}", out.unwrap_or_default()));
    }
    out.map(|s| s.trim().to_owned())
}

/// org-macro-initialize-templates with DEFAULT (global macros).
fn templates(
    text: &str,
    path: Option<&Path>,
    default: &[(String, String)],
) -> Vec<(String, Template)> {
    let mut t: Vec<(String, Template)> = vec![];
    let mut set = |name: &str, v: Template| {
        if !t.iter().any(|(n, _)| n == name) {
            t.push((name.to_owned(), v));
        }
    };
    // Buffer macros override global ones; the first definition of a
    // name in the buffer wins over later ones only through pushing.
    let mut collected: Vec<(String, Template)> = vec![];
    for (k, v) in default {
        collected.push((k.clone(), Template::Text(v.clone())));
    }
    collected.push((
        "author".into(),
        Template::Text(keyword_value(text, "AUTHOR", true).unwrap_or_default()),
    ));
    collected.push((
        "email".into(),
        Template::Text(keyword_value(text, "EMAIL", false).unwrap_or_default()),
    ));
    collected.push((
        "title".into(),
        Template::Text(keyword_value(text, "TITLE", true).unwrap_or_default()),
    ));
    let date = keyword_value(text, "DATE", false).unwrap_or_default();
    let st = Settings::default();
    let parsed = element::parse_secondary_string(&date, element::restriction(Type::Keyword), &st);
    let c = parsed.contents(parsed.root);
    collected.push((
        "date".into(),
        if c.len() == 1 && parsed.kind(c[0]) == Type::Timestamp {
            Template::Date(date.clone(), date.clone())
        } else {
            Template::Text(date)
        },
    ));
    for l in text.lines() {
        if let Some((k, v)) = crate::org::syntax::keyword_line(l)
            && k == "MACRO"
        {
            let (name, def) = match v.split_once([' ', '\t']) {
                Some((n, d)) => (n, d.trim_start()),
                None => (v, ""),
            };
            if !name.is_empty() {
                collected.push((name.to_owned(), Template::Text(def.to_owned())));
            }
        }
    }
    // org-macro--set-templates: later definitions replace earlier ones.
    let mut merged: Vec<(String, Template)> = vec![];
    for (k, v) in collected {
        match merged.iter_mut().find(|(n, _)| *n == k) {
            Some((_, old)) => *old = v,
            None => merged.insert(0, (k, v)),
        }
    }
    for (k, v) in merged {
        set(&k, v);
    }
    if let Some(p) = path.filter(|p| p.exists()) {
        set(
            "input-file",
            Template::Text(
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
        );
        set("modification-time", Template::Builtin("modification-time"));
    }
    set("keyword", Template::Builtin("keyword"));
    set("n", Template::Builtin("n"));
    set("property", Template::Builtin("property"));
    set("time", Template::Builtin("time"));
    t
}

struct Env<'a> {
    text: &'a str,
    path: Option<&'a Path>,
    counters: HashMap<String, i64>,
}

impl Env<'_> {
    /// org-macro-expand: None when there is no template.
    fn expand(
        &mut self,
        templates: &[(String, Template)],
        key: &str,
        args: &[String],
    ) -> Result<Option<String>, String> {
        let Some((_, t)) = templates.iter().find(|(n, _)| n.eq_ignore_ascii_case(key)) else {
            return Ok(None);
        };
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        Ok(Some(match t {
            Template::Text(s) if s.starts_with("(eval") => String::new(),
            Template::Text(s) => {
                let re = regex::Regex::new(r"\$[0-9]+").unwrap();
                re.replace_all(s, |c: &regex::Captures| {
                    let n: usize = c[0][1..].parse().unwrap_or(0);
                    if n == 0 { String::new() } else { arg(n - 1) }
                })
                .into_owned()
            }
            Template::Date(ts, raw) => {
                let fmt = arg(0);
                if crate::org::export::tools::nw(&fmt) {
                    let st = Settings::default();
                    let p = element::parse_secondary_string(ts, &[Type::Timestamp], &st);
                    match p.contents(p.root).first() {
                        Some(&n) => super::pipeline::format_timestamp(&p, n, &fmt, false),
                        None => raw.clone(),
                    }
                } else {
                    raw.clone()
                }
            }
            Template::Builtin("keyword") => {
                keyword_value(self.text, &arg(0), true).unwrap_or_default()
            }
            Template::Builtin("n") => {
                let name = arg(0).trim().to_owned();
                let action = arg(1).trim().to_owned();
                let cur = self.counters.get(&name).copied();
                let v = if action.is_empty() {
                    cur.unwrap_or(0) + 1
                } else if action == "-" {
                    cur.unwrap_or(1)
                } else if action.chars().all(|c| c.is_ascii_digit()) {
                    action.parse().unwrap_or(1)
                } else {
                    1
                };
                self.counters.insert(name, v);
                v.to_string()
            }
            Template::Builtin("property") => {
                property(self.text, &arg(0), &arg(1)).unwrap_or_default()
            }
            Template::Builtin("time") => {
                crate::org::time::civil::format(&arg(0), crate::org::time::civil::now())
            }
            Template::Builtin("modification-time") => {
                let t = self
                    .path
                    .and_then(|p| p.metadata().ok())
                    .and_then(|m| m.modified().ok())
                    .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);
                crate::org::time::civil::format(&arg(0), crate::org::time::civil::Tm::from_epoch(t))
            }
            Template::Builtin(_) => String::new(),
        }))
    }
}

/// {{{property(NAME, LOCATION)}}}: the property of the entry at the
/// search LOCATION (the first entry with that property when empty).
fn property(text: &str, name: &str, location: &str) -> Option<String> {
    let st = Settings::default();
    let tree = element::parse_buffer(text, &st);
    let loc = location.trim();
    let hs = tree.map(tree.root, Some(&[Type::Headline]), |_, h| Some(h));
    let target = if loc.is_empty() {
        None
    } else {
        let want = loc.trim_start_matches('*').trim();
        Some(
            hs.iter()
                .copied()
                .find(|&h| {
                    tree.str(h, "raw-value") == Some(want)
                        || (loc.starts_with('#') && tree.str(h, "CUSTOM_ID") == Some(&loc[1..]))
                })
                .ok_or(())
                .ok()?,
        )
    };
    let key = name.trim().to_uppercase();
    match target {
        Some(h) => tree.str(h, &key).map(str::to_owned),
        None => tree
            .map(tree.root, Some(&[Type::Keyword]), |t, k| {
                (t.str(k, "key") == Some("PROPERTY"))
                    .then(|| t.str(k, "value").unwrap_or("").to_owned())
            })
            .into_iter()
            .find_map(|v| {
                let (k, val) = v.split_once(' ')?;
                k.eq_ignore_ascii_case(&key).then(|| val.trim().to_owned())
            }),
    }
}

/// org-macro-replace-all on TEXT (with templates from
/// org-macro-initialize-templates and GLOBAL macros).  KEYWORDS are the
/// parsed keywords (TITLE, DATE, ...) where macros are also expanded,
/// with their EXPORT_ node properties.  `results` macros without a
/// template are kept for the second phase.
pub fn expand_macros(
    text: &str,
    path: Option<&Path>,
    global: &[(String, String)],
    keywords: &[&str],
    only: Option<&[(String, String)]>,
) -> Result<String, String> {
    let tpl: Vec<(String, Template)> = match only {
        Some(t) => t
            .iter()
            .map(|(k, v)| (k.clone(), Template::Text(v.clone())))
            .collect(),
        None => templates(text, path, global),
    };
    let mut env = Env {
        text,
        path,
        counters: HashMap::new(),
    };
    let mut s = text.to_owned();
    let mut pos = 0;
    let mut record: Vec<(usize, String, Vec<String>)> = vec![];
    let start = regex::Regex::new(r"\{\{\{[-A-Za-z0-9_]").unwrap();
    let st = crate::org::settings_of(text, path.and_then(Path::parent));
    while let Some(m) = start.find_at(&s, pos) {
        let after = m.end();
        let (tree, ctx) = element::context_at(&s, &st, after - 1);
        if ctx.is_some_and(|n| in_commented_heading(&tree, n)) {
            pos = after;
            continue;
        }
        let macro_node = match ctx {
            Some(n) if tree.kind(n) == Type::Macro => Some((
                tree.begin(n).unwrap_or(m.start()),
                tree.end(n).unwrap_or(after),
                tree.str(n, "key").unwrap_or("").to_owned(),
                macro_args(&tree, n),
            )),
            Some(n)
                if (tree.kind(n) == Type::Keyword
                    && keywords.iter().any(|k| tree.str(n, "key") == Some(k)))
                    || (tree.kind(n) == Type::NodeProperty
                        && is_export_property(tree.str(n, "key").unwrap_or(""), keywords)) =>
            {
                // Force recognition: parse a macro at the match.
                let p = element::parse_secondary_string(&s[m.start()..], &[Type::Macro], &st);
                match p.contents(p.root).first() {
                    Some(&x) if p.kind(x) == Type::Macro && p.begin(x) == Some(0) => Some((
                        m.start(),
                        m.start() + p.end(x).unwrap_or(0),
                        p.str(x, "key").unwrap_or("").to_owned(),
                        macro_args(&p, x),
                    )),
                    _ => None,
                }
            }
            _ => None,
        };
        let Some((begin, end, key, args)) = macro_node else {
            pos = after;
            continue;
        };
        let sig = (begin, key.clone(), args.clone());
        let value = env.expand(&tpl, &key, &args)?;
        match value {
            _ if record.contains(&sig) => return Err(format!("Circular macro expansion: {key}")),
            Some(v) => {
                record.push(sig);
                // Preserve white space after the macro.
                let end = begin + s[begin..end].trim_end_matches([' ', '\t']).len();
                s.replace_range(begin..end, &v);
                pos = begin;
            }
            None if key == "results" => pos = after,
            None => return Err(format!("Undefined Org macro: {key}; aborting")),
        }
    }
    Ok(s)
}

fn macro_args(t: &element::Tree, n: element::NodeId) -> Vec<String> {
    t.prop(n, "args")
        .as_list()
        .unwrap_or(&[])
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_owned())
        .collect()
}

fn is_export_property(key: &str, keywords: &[&str]) -> bool {
    let k = key.strip_suffix('+').unwrap_or(key);
    k.strip_prefix("EXPORT_")
        .is_some_and(|r| keywords.contains(&r))
}

/// org-in-commented-heading-p for node N of TREE.
pub fn in_commented_heading(tree: &element::Tree, n: element::NodeId) -> bool {
    tree.lineage(n, true)
        .into_iter()
        .any(|h| tree.kind(h) == Type::Headline && tree.flag(h, "commentedp"))
}
