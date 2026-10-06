//! The environment options of ox.el: org-export-options-alist, OPTIONS
//! lines, in-buffer keywords, subtree EXPORT_* properties, defaults from
//! `[org]` options and `#+BIND`.

use super::{Behavior, Behavior as B, Info, OptDef};
use crate::org::element::{self, NodeId, Tree, Type, Value};
use crate::org::sexp::{self, Sexp};
use crate::org::syntax::Settings;
use std::cell::RefCell;
use std::collections::HashMap;

const fn o(
    prop: &'static str,
    keyword: Option<&'static str>,
    option: Option<&'static str>,
    var: Option<&'static str>,
    default: &'static str,
    behavior: Behavior,
) -> OptDef {
    OptDef {
        prop,
        keyword,
        option,
        var,
        default,
        behavior,
    }
}

/// org-export-options-alist.
pub const CORE: &[OptDef] = &[
    o("title", Some("TITLE"), None, None, "nil", B::Parse),
    o("date", Some("DATE"), None, None, "nil", B::Parse),
    o(
        "author",
        Some("AUTHOR"),
        None,
        Some("user-full-name"),
        "",
        B::Parse,
    ),
    o(
        "email",
        Some("EMAIL"),
        None,
        Some("user-mail-address"),
        "",
        B::Last,
    ),
    o(
        "language",
        Some("LANGUAGE"),
        None,
        Some("org-export-default-language"),
        "\"en\"",
        B::Last,
    ),
    o(
        "other-languages",
        Some("OTHER_LANGUAGES"),
        None,
        Some("org-export-other-languages"),
        "nil",
        B::Split,
    ),
    o(
        "select-tags",
        Some("SELECT_TAGS"),
        None,
        Some("org-export-select-tags"),
        "(\"export\")",
        B::Split,
    ),
    o(
        "exclude-tags",
        Some("EXCLUDE_TAGS"),
        None,
        Some("org-export-exclude-tags"),
        "(\"noexport\")",
        B::Split,
    ),
    o(
        "creator",
        Some("CREATOR"),
        None,
        Some("org-export-creator-string"),
        "",
        B::First,
    ),
    o(
        "headline-levels",
        None,
        Some("H"),
        Some("org-export-headline-levels"),
        "3",
        B::First,
    ),
    o(
        "preserve-breaks",
        None,
        Some("\\n"),
        Some("org-export-preserve-breaks"),
        "nil",
        B::First,
    ),
    o(
        "section-numbers",
        None,
        Some("num"),
        Some("org-export-with-section-numbers"),
        "t",
        B::First,
    ),
    o(
        "time-stamp-file",
        None,
        Some("timestamp"),
        Some("org-export-timestamp-file"),
        "t",
        B::First,
    ),
    o(
        "with-archived-trees",
        None,
        Some("arch"),
        Some("org-export-with-archived-trees"),
        "headline",
        B::First,
    ),
    o(
        "with-author",
        None,
        Some("author"),
        Some("org-export-with-author"),
        "t",
        B::First,
    ),
    o(
        "expand-links",
        None,
        Some("expand-links"),
        Some("org-export-expand-links"),
        "t",
        B::First,
    ),
    o(
        "with-broken-links",
        None,
        Some("broken-links"),
        Some("org-export-with-broken-links"),
        "nil",
        B::First,
    ),
    o(
        "with-clocks",
        None,
        Some("c"),
        Some("org-export-with-clocks"),
        "nil",
        B::First,
    ),
    o(
        "with-creator",
        None,
        Some("creator"),
        Some("org-export-with-creator"),
        "nil",
        B::First,
    ),
    o(
        "with-date",
        None,
        Some("date"),
        Some("org-export-with-date"),
        "t",
        B::First,
    ),
    o(
        "with-drawers",
        None,
        Some("d"),
        Some("org-export-with-drawers"),
        "(not \"LOGBOOK\")",
        B::First,
    ),
    o(
        "with-email",
        None,
        Some("email"),
        Some("org-export-with-email"),
        "nil",
        B::First,
    ),
    o(
        "with-emphasize",
        None,
        Some("*"),
        Some("org-export-with-emphasize"),
        "t",
        B::First,
    ),
    o(
        "with-entities",
        None,
        Some("e"),
        Some("org-export-with-entities"),
        "t",
        B::First,
    ),
    o(
        "with-fixed-width",
        None,
        Some(":"),
        Some("org-export-with-fixed-width"),
        "t",
        B::First,
    ),
    o(
        "with-footnotes",
        None,
        Some("f"),
        Some("org-export-with-footnotes"),
        "t",
        B::First,
    ),
    o(
        "with-inlinetasks",
        None,
        Some("inline"),
        Some("org-export-with-inlinetasks"),
        "t",
        B::First,
    ),
    o(
        "with-latex",
        None,
        Some("tex"),
        Some("org-export-with-latex"),
        "t",
        B::First,
    ),
    o(
        "with-planning",
        None,
        Some("p"),
        Some("org-export-with-planning"),
        "nil",
        B::First,
    ),
    o(
        "with-priority",
        None,
        Some("pri"),
        Some("org-export-with-priority"),
        "nil",
        B::First,
    ),
    o(
        "with-properties",
        None,
        Some("prop"),
        Some("org-export-with-properties"),
        "nil",
        B::First,
    ),
    o(
        "with-smart-quotes",
        None,
        Some("'"),
        Some("org-export-with-smart-quotes"),
        "nil",
        B::First,
    ),
    o(
        "with-special-strings",
        None,
        Some("-"),
        Some("org-export-with-special-strings"),
        "t",
        B::First,
    ),
    o("with-special-rows", None, None, None, "nil", B::First),
    o(
        "with-statistics-cookies",
        None,
        Some("stat"),
        Some("org-export-with-statistics-cookies"),
        "t",
        B::First,
    ),
    o(
        "with-sub-superscript",
        None,
        Some("^"),
        Some("org-export-with-sub-superscripts"),
        "t",
        B::First,
    ),
    o(
        "with-toc",
        None,
        Some("toc"),
        Some("org-export-with-toc"),
        "t",
        B::First,
    ),
    o(
        "with-tables",
        None,
        Some("|"),
        Some("org-export-with-tables"),
        "t",
        B::First,
    ),
    o(
        "with-tags",
        None,
        Some("tags"),
        Some("org-export-with-tags"),
        "t",
        B::First,
    ),
    o(
        "with-tasks",
        None,
        Some("tasks"),
        Some("org-export-with-tasks"),
        "t",
        B::First,
    ),
    o(
        "with-timestamps",
        None,
        Some("<"),
        Some("org-export-with-timestamps"),
        "t",
        B::First,
    ),
    o(
        "with-title",
        None,
        Some("title"),
        Some("org-export-with-title"),
        "t",
        B::First,
    ),
    o(
        "with-todo-keywords",
        None,
        Some("todo"),
        Some("org-export-with-todo-keywords"),
        "t",
        B::First,
    ),
    o(
        "with-cite-processors",
        None,
        None,
        Some("org-export-process-citations"),
        "t",
        B::First,
    ),
    o(
        "cite-export",
        Some("CITE_EXPORT"),
        None,
        Some("org-cite-export-processors"),
        "nil",
        B::First,
    ),
];

/// The creator string (org-export-creator-string's default).
pub const CREATOR: &str = "Fred (Org mode 10.0-pre)";

thread_local! {
    /// `#+BIND` values in effect during an export.
    static BINDINGS: RefCell<HashMap<String, Sexp>> = RefCell::new(HashMap::new());
}

/// A variable's value during export: a `#+BIND` binding, else the
/// `[org]` option.
pub fn var(name: &str) -> Option<Sexp> {
    BINDINGS
        .with(|b| b.borrow().get(name).cloned())
        .or_else(|| sexp::option(name))
}

/// A string variable with its default.
pub fn var_str(name: &str, default: &str) -> String {
    match var(name) {
        Some(Sexp::Str(s)) | Some(Sexp::Sym(s)) => s,
        Some(Sexp::Nil) => String::new(),
        _ => default.to_owned(),
    }
}

/// Install #+BIND values for the duration of an export (and clear them
/// with an empty list).
pub fn set_bindings(b: Vec<(String, Sexp)>) {
    BINDINGS.with(|m| *m.borrow_mut() = b.into_iter().collect());
}

/// Lisp data as a property value.
pub fn value_of(s: &Sexp) -> Value {
    match s {
        Sexp::Nil => Value::Nil,
        Sexp::T => Value::T,
        Sexp::Int(i) => Value::Int(*i),
        Sexp::Float(f) => Value::Int(*f as i64),
        Sexp::Str(s) => Value::Str(s.clone()),
        Sexp::Sym(s) if s == "nil" => Value::Nil,
        Sexp::Sym(s) if s == "t" => Value::T,
        Sexp::Sym(s) => Value::Sym(s.clone()),
        Sexp::List(l) | Sexp::Vector(l) => Value::List(l.iter().map(value_of).collect()),
        Sexp::Dotted(l, tail) => {
            let mut v = value_of(tail);
            for x in l.iter().rev() {
                v = Value::cons(value_of(x), v);
            }
            v
        }
    }
}

/// The default value of an option (DEFAULT evaluated).
pub fn default_value(d: &OptDef) -> Value {
    match d.var {
        Some("user-full-name") => Value::Str(var_str(
            "user-full-name",
            &std::env::var("USER").unwrap_or_default(),
        )),
        Some("user-mail-address") => match var_str("user-mail-address", "") {
            s if s.is_empty() => Value::Nil,
            s => Value::Str(s),
        },
        Some("org-export-creator-string") => {
            Value::Str(var_str("org-export-creator-string", CREATOR))
        }
        Some(v) => match var(v) {
            Some(s) => value_of(&s),
            None => value_of(&sexp::read(d.default).unwrap_or(Sexp::Nil)),
        },
        None => value_of(&sexp::read(d.default).unwrap_or(Sexp::Nil)),
    }
}

/// The options of BACKEND then the core ones, keeping only the first
/// definition of each property (derived backends take precedence).
pub fn all_options(info: &Info) -> Vec<OptDef> {
    let mut seen: Vec<&str> = vec![];
    let mut out = vec![];
    for d in info
        .backend
        .all_options()
        .into_iter()
        .chain(CORE.iter().copied())
    {
        if !seen.contains(&d.prop) {
            seen.push(d.prop);
            out.push(d);
        }
    }
    out
}

/// Every option row, duplicates included (as `append` upstream).
fn rows(info: &Info) -> Vec<OptDef> {
    info.backend
        .all_options()
        .into_iter()
        .chain(CORE.iter().copied())
        .collect()
}

/// org-export--parse-option-keyword: (property, value) pairs.
pub fn parse_option_keyword(options: &str, info: &Info) -> Vec<(&'static str, Value)> {
    let line = option_items(options);
    if line.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    let mut seen: Vec<&str> = vec![];
    for d in rows(info) {
        let Some(item) = d.option else { continue };
        if seen.contains(&d.prop) {
            continue;
        }
        seen.push(d.prop);
        if let Some((_, v)) = line.iter().find(|(k, _)| k.eq_ignore_ascii_case(item)) {
            out.push((d.prop, value_of(v)));
        }
    }
    out
}

/// The `ITEM:VALUE` pairs of an OPTIONS line, last first (as the alist
/// built by pushing).
pub fn option_items(s: &str) -> Vec<(String, Sexp)> {
    let mut out = vec![];
    let mut p = 0;
    let b = s.as_bytes();
    while p < s.len() {
        // \s-*\(.+?\):
        let mut q = p;
        while q < s.len() && matches!(b[q], b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') {
            q += 1;
        }
        // `.+?` needs one char: a leading `:` belongs to the item.
        let c1 = s[q..].chars().next().map_or(1, char::len_utf8);
        let found = s
            .get(q + c1..)
            .and_then(|r| r.find(':'))
            .map(|i| i + c1)
            .filter(|&i| !s[q..q + i].contains('\n'));
        let Some(rel) = found else {
            // `.+?` cannot cross a newline: retry on the next line.
            match s[q..].find('\n') {
                Some(n) if s[q + n + 1..].contains(':') => {
                    p = q + n + 1;
                    continue;
                }
                _ => break,
            }
        };
        let item = s[q..q + rel].to_owned();
        let after = q + rel + 1;
        p = after;
        if after < s.len() && !s.as_bytes()[after].is_ascii_whitespace() {
            match sexp::read_prefix(&s[after..]) {
                Ok((v, rest)) => {
                    out.insert(0, (item, v));
                    p = s.len() - rest.len();
                }
                Err(_) => break,
            }
        }
    }
    out
}

/// The `parse` behavior: a secondary string in INFO's tree, newlines
/// in plain text turned into spaces.
fn parse_value(info: &mut Info, s: &str) -> Value {
    let st = info.settings.clone();
    let ids = super::parse_secondary(&mut info.tree, s, element::restriction(Type::Keyword), &st);
    for &i in &ids {
        replace_newlines(&mut info.tree, i);
    }
    Value::Secondary(ids)
}

fn replace_newlines(t: &mut Tree, id: NodeId) {
    if t.kind(id) == Type::PlainText {
        let s = t.node(id).text.replace('\n', " ");
        t.node_mut(id).text = s;
        return;
    }
    for c in t.contents(id).to_vec() {
        replace_newlines(t, c);
    }
}

/// org-export--get-global-options.
pub fn global_options(info: &mut Info) -> Vec<(&'static str, Value)> {
    let mut out = vec![];
    for d in all_options(info) {
        let v = default_value(&d);
        let v = match (&v, d.behavior) {
            (Value::Str(s), B::Parse) => parse_value(info, &s.clone()),
            _ => v,
        };
        out.push((d.prop, v));
    }
    out
}

/// org-collect-keywords: values per keyword in KEYS, in document order,
/// from the keyword elements of TREE and SETUPFILEs (read from DIR).
pub fn collect_keywords(
    tree: &Tree,
    keys: &[&str],
    dir: Option<&std::path::Path>,
) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = vec![];
    let mut push = |k: &str, v: &str| {
        if !keys.iter().any(|x| x.eq_ignore_ascii_case(k)) {
            return;
        }
        match out.iter_mut().find(|(x, _)| x == k) {
            Some((_, vs)) => vs.push(v.to_owned()),
            None => out.push((k.to_owned(), vec![v.to_owned()])),
        }
    };
    let kws = tree.map(tree.root, Some(&[Type::Keyword]), |t, n| {
        Some((
            t.str(n, "key").unwrap_or("").to_uppercase(),
            t.str(n, "value").unwrap_or("").to_owned(),
        ))
    });
    for (k, v) in kws {
        if k == "SETUPFILE" {
            let file = v.trim().trim_matches('"');
            let path = match dir {
                Some(d) => d.join(crate::org::options::expand(file)),
                None => crate::org::options::expand(file),
            };
            if let Ok(text) = std::fs::read_to_string(&path) {
                let st = Settings::default();
                let t = element::parse_buffer(&text, &st);
                for (k2, vs) in collect_keywords(&t, keys, path.parent()) {
                    for v2 in vs {
                        push(&k2, &v2);
                    }
                }
            }
            continue;
        }
        push(&k, &v);
    }
    out
}

/// org-export--get-inbuffer-options on the keywords KWS (from
/// [`collect_keywords`] over every option keyword, FILETAGS, OPTIONS).
pub fn inbuffer_options(
    info: &mut Info,
    kws: &[(String, Vec<String>)],
) -> Vec<(&'static str, Value)> {
    let rows = rows(info);
    let mut plist: Vec<(&'static str, Value)> = vec![];
    let put = |plist: &mut Vec<(&'static str, Value)>, k: &'static str, v: Value| {
        plist.retain(|(x, _)| *x != k);
        plist.push((k, v));
    };
    let mut to_parse: Vec<&'static str> = vec![];
    for (key, values) in kws {
        match key.as_str() {
            "OPTIONS" => {
                for v in values {
                    for (k, val) in parse_option_keyword(v, info) {
                        put(&mut plist, k, val);
                    }
                }
            }
            "FILETAGS" => {
                let mut tags: Vec<String> = vec![];
                for v in values {
                    for t in v.split(':').map(str::trim).filter(|t| !t.is_empty()) {
                        if !tags.iter().any(|x| x == t) {
                            tags.push(t.to_owned());
                        }
                    }
                }
                put(
                    &mut plist,
                    "filetags",
                    Value::List(tags.into_iter().map(Value::Str).collect()),
                );
            }
            _ => {
                // Properties associated to KEYWORD (first definition of
                // each property only).
                let mut seen: Vec<&str> = vec![];
                let mut props: Vec<OptDef> = vec![];
                for d in &rows {
                    if seen.contains(&d.prop) {
                        continue;
                    }
                    seen.push(d.prop);
                    if d.keyword == Some(key.as_str()) && !props.iter().any(|p| p.prop == d.prop) {
                        props.push(*d);
                    }
                }
                for d in props {
                    let v = match d.behavior {
                        B::Parse => {
                            if !to_parse.contains(&d.prop) {
                                to_parse.push(d.prop);
                            }
                            Value::Str(values.join("\n"))
                        }
                        B::Space => Value::Str(values.join(" ")),
                        B::Newline => Value::Str(values.join("\n")),
                        B::Split => Value::List(
                            values
                                .iter()
                                .flat_map(|v| v.split_whitespace())
                                .map(|s| Value::Str(s.to_owned()))
                                .collect(),
                        ),
                        B::Last => Value::Str(values.last().cloned().unwrap_or_default()),
                        B::First => Value::Str(values[0].clone()),
                    };
                    put(&mut plist, d.prop, v);
                }
            }
        }
    }
    for p in to_parse {
        if let Some(i) = plist.iter().position(|(k, _)| *k == p) {
            let s = plist[i].1.as_str().unwrap_or("").to_owned();
            plist[i].1 = parse_value(info, &s);
        }
    }
    plist
}

/// org-export--get-subtree-options: PROPS are the headline's node
/// properties (upcased keys), TITLE its title without TODO keyword,
/// priority and tags.
pub fn subtree_options(
    info: &mut Info,
    props: &[(String, String)],
    title: &str,
) -> Vec<(&'static str, Value)> {
    let get = |k: &str| {
        props
            .iter()
            .find(|(x, _)| x.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.clone())
    };
    let mut plist: Vec<(&'static str, Value)> = match get("EXPORT_OPTIONS") {
        Some(o) => parse_option_keyword(&o, info),
        None => vec![],
    };
    let mut seen: Vec<&str> = vec![];
    for d in rows(info) {
        let Some(kw) = d.keyword else { continue };
        if seen.contains(&d.prop) {
            continue;
        }
        seen.push(d.prop);
        let v = if kw == "TITLE" {
            get("EXPORT_TITLE").or_else(|| Some(title.to_owned()))
        } else {
            get(&format!("EXPORT_{kw}"))
        };
        let Some(v) = v else { continue };
        let v = match d.behavior {
            B::Parse => {
                let st = info.settings.clone();
                Value::Secondary(super::parse_secondary(
                    &mut info.tree,
                    &v,
                    element::restriction(Type::Keyword),
                    &st,
                ))
            }
            B::Split => Value::List(
                v.split_whitespace()
                    .map(|s| Value::Str(s.to_owned()))
                    .collect(),
            ),
            _ => Value::Str(v),
        };
        plist.retain(|(k, _)| *k != d.prop);
        plist.push((d.prop, v));
    }
    plist
}

/// org-export--list-bound-variables: `#+BIND: VAR VALUE` pairs, when
/// org-export-allow-bind-keywords is set.
pub fn bound_variables(kws: &[(String, Vec<String>)]) -> Vec<(String, Sexp)> {
    if !crate::org::options::bool("org-export-allow-bind-keywords", false) {
        return vec![];
    }
    let mut out = vec![];
    for (k, vs) in kws {
        if k != "BIND" {
            continue;
        }
        for v in vs {
            if let Ok(Sexp::List(l)) = sexp::read(&format!("({v})"))
                && let Some(name) = l.first().and_then(Sexp::sym)
            {
                let val = sexp::unquote(l.get(1).cloned().unwrap_or(Sexp::Nil));
                out.push((name.to_owned(), val));
            }
        }
    }
    out
}

/// The keywords org-export-get-environment reads.
pub fn keyword_names(info: &Info) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = rows(info).iter().filter_map(|d| d.keyword).collect();
    v.extend(["FILETAGS", "OPTIONS", "BIND"]);
    v.dedup();
    v
}
