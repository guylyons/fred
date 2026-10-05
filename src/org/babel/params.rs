//! Header arguments (ob-core.el): parsing, defaults, merging and the
//! quasi-alist they live in.

use super::value::{self, as_string};
use crate::org::sexp::{self, Sexp};

/// A header-argument quasi-alist: (`:key`, value) with repeated `:var`.
pub type Params = Vec<(String, Sexp)>;

/// `(cdr (assq KEY params))`.
pub fn get<'a>(p: &'a [(String, Sexp)], key: &str) -> Option<&'a Sexp> {
    p.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// `(assq KEY params)` exists.
pub fn has(p: &[(String, Sexp)], key: &str) -> bool {
    p.iter().any(|(k, _)| k == key)
}

/// A value as a string (nil and missing are None).
pub fn get_str(p: &[(String, Sexp)], key: &str) -> Option<String> {
    get(p, key).and_then(as_string)
}

pub fn set(p: &mut Params, key: &str, v: Sexp) {
    p.retain(|(k, _)| k != key);
    p.insert(0, (key.into(), v));
}

/// `:result-params` as strings.
pub fn result_params(p: &[(String, Sexp)]) -> Vec<String> {
    match get(p, ":result-params") {
        Some(v) => value::items(v).unwrap_or(&[]).iter().filter_map(as_string).collect(),
        None => vec![],
    }
}

/// The processed `:var` pairs: (name, value).
pub fn vars(p: &[(String, Sexp)]) -> Vec<(String, Sexp)> {
    p.iter()
        .filter(|(k, _)| k == ":var")
        .filter_map(|(_, v)| value::car_cdr(v))
        .map(|(n, v)| (value::princ(&n), v))
        .collect()
}

/// Allowed values (org-babel-common-header-args-w-values).
pub const COMMON: &[(&str, Option<&[&[&str]]>)] = &[
    ("cache", Some(&[&["no", "yes"]])),
    ("cmdline", None),
    ("colnames", Some(&[&["nil", "no", "yes"]])),
    ("comments", Some(&[&["no", "link", "yes", "org", "both", "noweb"]])),
    ("dir", None),
    ("eval", Some(&[&["yes", "no", "no-export", "strip-export", "never-export", "eval", "never", "query"]])),
    ("exports", Some(&[&["code", "results", "both", "none"]])),
    ("epilogue", None),
    ("file", None),
    ("file-desc", None),
    ("file-ext", None),
    ("file-mode", Some(&[&["#o755", "#o555", "#o444", ":any"]])),
    ("hlines", Some(&[&["no", "yes"]])),
    ("mkdirp", Some(&[&["yes", "no"]])),
    ("no-expand", Some(&[])),
    ("noeval", Some(&[])),
    ("noweb", Some(&[&["yes", "no", "tangle", "strip-tangle", "no-export", "strip-export"]])),
    ("noweb-ref", None),
    ("noweb-sep", None),
    ("noweb-prefix", Some(&[&["no", "yes"]])),
    ("output-dir", None),
    ("padline", Some(&[&["yes", "no"]])),
    ("post", None),
    ("prologue", None),
    (
        "results",
        Some(&[
            &["file", "list", "vector", "table", "scalar", "verbatim"],
            &["raw", "html", "latex", "org", "code", "pp", "drawer", "link", "graphics"],
            &["replace", "silent", "none", "discard", "append", "prepend"],
            &["output", "value"],
        ]),
    ),
    ("rownames", Some(&[&["no", "yes"]])),
    ("sep", None),
    ("session", None),
    ("shebang", None),
    ("tangle", Some(&[&["tangle", "yes", "no", ":any"]])),
    ("tangle-mode", Some(&[&["#o755", "#o555", "#o444", ":any"]])),
    ("var", None),
    ("wrap", None),
];

/// Language-specific header arguments (org-babel-header-args:LANG).
pub fn lang_headers(lang: &str) -> Vec<(&'static str, Option<&'static [&'static [&'static str]]>)> {
    let any = |names: &[&'static str]| names.iter().map(|n| (*n, None)).collect::<Vec<_>>();
    match lang {
        "C" => any(&["includes", "defines", "main", "flags", "cmdline", "libs"]),
        "C++" | "cpp" => any(&["namespaces", "includes", "defines", "main", "flags", "cmdline", "libs"]),
        "R" => {
            let mut v = any(&[
                "width", "height", "bg", "units", "pointsize", "antialias", "quality", "compression", "res", "type",
                "family", "title", "fonts", "version", "paper", "encoding", "pagecentre", "colormodel", "useDingbats",
                "horizontal",
            ]);
            v.push(("async", Some(&[&["yes", "no"]])));
            v
        }
        "python" => vec![("return", None), ("python", None), ("async", Some(&[&["yes", "no"]]))],
        "emacs-lisp" | "elisp" => any(&["lexical"]),
        "latex" => {
            let mut v = any(&["border", "fit", "iminoptions", "imoutoptions", "packages", "pdfheight", "pdfpng", "pdfwidth", "headers"]);
            v.push(("imagemagick", Some(&[&["nil", "t"]])));
            v.push(("buffer", Some(&[&["yes", "no"]])));
            v
        }
        "sqlite" => {
            let mut v = any(&["db", "header", "echo", "bail", "csv", "column", "html", "line", "list", "separator", "nullvalue"]);
            v.push(("readonly-p", Some(&[&["yes", "no"]])));
            v
        }
        "sql" => any(&["engine", "out-file", "dbhost", "dbport", "dbuser", "dbpassword", "dbinstance", "database"]),
        "java" => any(&["dir", "classname", "imports", "cmpflag", "cmdline", "cmdarg"]),
        "haskell" => any(&["compile"]),
        "clojure" => vec![("ns", None), ("package", None), ("backend", Some(&[&["inf-clojure", "cider", "slime", "babashka", "nbb"]]))],
        "lisp" => any(&["package"]),
        "scheme" => any(&["host", "port"]),
        "maxima" => vec![("batch", Some(&[&["batchload", "batch", "load"]])), ("graphics-pkg", Some(&[&["plot", "draw"]]))],
        "gnuplot" => any(&["title", "lines", "sets", "x-labels", "y-labels", "timefmt", "time-ind", "missing", "term"]),
        "sed" => any(&[":cmd-line", ":in-file"]),
        "julia" => any(&["width", "horizontal"]),
        l if super::lang::shell_names().iter().any(|s| s == l) || l == "shell" => vec![("async", Some(&[&["yes", "no"]]))],
        _ => vec![],
    }
}

/// org-babel-balanced-split separators.
#[derive(Clone, Copy)]
pub enum Alts<'a> {
    Char(char),
    /// Split where a char of the first set is followed by one of the second.
    Pair(&'a [char], &'a [char]),
}

/// org-babel-balanced-split.
pub fn balanced_split(s: &str, alts: Alts) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut partial: Vec<char> = vec![];
    let mut out = vec![];
    let mut i = 0;
    let split = |past: Option<char>, next: char| match alts {
        Alts::Char(c) => next == c,
        Alts::Pair(a, b) => past.is_some_and(|p| a.contains(&p)) && b.contains(&next),
    };
    while i < chars.len() {
        let c = chars[i];
        let past = if i > 0 { Some(chars[i - 1]) } else { None };
        if split(past, c) {
            if matches!(alts, Alts::Pair(..)) {
                partial.pop();
            }
            if !partial.is_empty() {
                out.push(partial.iter().collect());
                partial.clear();
            }
            i += 1;
        } else if c == '(' || c == '[' {
            let mut stack = vec![c];
            let mut j = i + 1;
            while !stack.is_empty() && j < chars.len() {
                match chars[j] {
                    x @ ('(' | '[') => stack.push(x),
                    ']' if stack.last() == Some(&'[') => {
                        stack.pop();
                    }
                    ')' if stack.last() == Some(&'(') => {
                        stack.pop();
                    }
                    _ => {}
                }
                j += 1;
            }
            if stack.is_empty() {
                partial.extend(&chars[i..j]);
                i = j;
            } else {
                partial.push(c);
                i += 1;
            }
        } else if c == '"' && past != Some('\\') {
            // To the next unescaped double quote.
            let close = (i + 1..chars.len()).find(|&j| chars[j] == '"' && chars[j - 1] != '\\');
            match close {
                Some(j) => {
                    partial.extend(&chars[i..=j]);
                    i = j + 1;
                }
                None => {
                    partial.push('"');
                    i += 1;
                }
            }
        } else {
            partial.push(c);
            i += 1;
        }
    }
    if !partial.is_empty() {
        out.push(partial.iter().collect());
    }
    out
}

/// org-babel-join-splits-near-ch.
fn join_splits_near(ch: char, list: Vec<String>) -> Vec<String> {
    let mut acc: Vec<String> = vec![];
    for el in list {
        match acc.last_mut() {
            Some(head) if head.ends_with(ch) || el.starts_with(ch) => head.push_str(&el),
            _ => acc.push(el),
        }
    }
    acc
}

/// org-babel-parse-header-arguments.
pub fn parse_header_arguments(s: &str, no_eval: bool) -> Result<Params, String> {
    if s.trim().is_empty() {
        return Ok(vec![]);
    }
    let raw = balanced_split(s, Alts::Pair(&[' ', '\t'], &[':']));
    let mut args = vec![];
    for (i, r) in raw.into_iter().enumerate() {
        let arg = if i == 0 { r } else { format!(":{r}") };
        // \([^ \f\t\n\r\v]+\)[ \f\t\n\r\v]+\([^ \f\t\n\r\v]+.*\)
        let ws = |c: char| " \x0c\t\n\r\x0b".contains(c);
        let name_end = arg.find(ws);
        let pair = match name_end {
            Some(e) if e > 0 => {
                let rest = arg[e..].trim_start_matches(ws);
                if rest.is_empty() {
                    None
                } else {
                    Some((arg[..e].to_owned(), rest.to_owned()))
                }
            }
            _ => None,
        };
        match pair {
            Some((k, v)) => {
                let v = value::chomp(&v, value::WS);
                args.push((k, value::read(v, no_eval)?));
            }
            None => args.push((value::chomp(&arg, value::WS).to_owned(), Sexp::Nil)),
        }
    }
    Ok(parse_multiple_vars(args))
}

/// org-babel-parse-multiple-vars.
fn parse_multiple_vars(args: Params) -> Params {
    let mut out = vec![];
    for (k, v) in args {
        if k == ":var" {
            let s = value::as_string(&v).unwrap_or_default();
            let parts = join_splits_near('=', balanced_split(&s, Alts::Char(' ')));
            if parts.is_empty() {
                out.push((k, Sexp::Nil));
            }
            for p in parts {
                out.push((":var".into(), Sexp::Str(value::trim(&p).to_owned())));
            }
        } else {
            out.push((k, v));
        }
    }
    out
}

thread_local! {
    /// A let-binding of org-babel-default-header-args (tangle's TARGET-FILE).
    static DEFAULT_EXTRA: std::cell::RefCell<Params> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Run F with KEY merged into org-babel-default-header-args.
pub fn with_default<T>(key: &str, v: Sexp, f: impl FnOnce() -> T) -> T {
    let old = DEFAULT_EXTRA.with(|d| std::mem::replace(&mut *d.borrow_mut(), vec![(key.to_owned(), v)]));
    let out = f();
    DEFAULT_EXTRA.with(|d| *d.borrow_mut() = old);
    out
}

/// Built-in defaults and the configured option for one header alist
/// variable (org-babel-default-header-args and friends).
pub fn default_args(var: &str) -> Params {
    let mut p = default_args_1(var);
    if var == "org-babel-default-header-args" {
        for (k, v) in DEFAULT_EXTRA.with(|d| d.borrow().clone()) {
            set(&mut p, &k, v);
        }
    }
    p
}

fn default_args_1(var: &str) -> Params {
    if let Some(v) = sexp::option(var) {
        return alist(&v);
    }
    let p = |pairs: &[(&str, &str)]| -> Params { pairs.iter().map(|(k, v)| ((*k).into(), Sexp::Str((*v).into()))).collect() };
    match var {
        "org-babel-default-header-args" => p(&[
            (":session", "none"),
            (":results", "replace"),
            (":exports", "code"),
            (":cache", "no"),
            (":noweb", "no"),
            (":hlines", "no"),
            (":tangle", "no"),
        ]),
        "org-babel-default-inline-header-args" => p(&[(":session", "none"), (":results", "replace"), (":exports", "results"), (":hlines", "yes")]),
        "org-babel-default-lob-header-args" => p(&[(":exports", "results")]),
        "org-babel-default-header-args:emacs-lisp" | "org-babel-default-header-args:elisp" => p(&[(":lexical", "no")]),
        "org-babel-default-header-args:dot" | "org-babel-default-header-args:plantuml" => p(&[(":results", "file"), (":exports", "results")]),
        "org-babel-default-header-args:latex" => p(&[(":results", "latex"), (":exports", "results")]),
        "org-babel-default-header-args:java" => p(&[(":results", "output"), (":dir", ".")]),
        "org-babel-default-header-args:haskell" => p(&[(":padline", "no")]),
        "org-babel-default-header-args:forth" => p(&[(":session", "yes")]),
        "org-babel-default-header-args:processing" => p(&[(":results", "html"), (":exports", "results")]),
        "org-babel-default-header-args:org" => p(&[(":results", "raw silent"), (":exports", "code")]),
        "org-babel-default-header-args:lilypond" => p(&[(":results", "file"), (":exports", "results")]),
        "org-babel-default-header-args:ditaa" => p(&[(":results", "file graphics"), (":exports", "results"), (":file-ext", "png")]),
        "org-babel-default-header-args:gnuplot" => {
            let mut v = p(&[(":results", "file"), (":exports", "results")]);
            v.push((":session".into(), Sexp::Nil));
            v
        }
        "org-babel-default-header-args:screen" => p(&[
            (":results", "silent"),
            (":session", "default"),
            (":cmd", "sh"),
            (":terminal", "xterm"),
            (":screenrc", "/dev/null"),
        ]),
        _ => vec![],
    }
}

/// An alist option value as params: `((:key . "v") ...)`.
pub fn alist(v: &Sexp) -> Params {
    value::items(v)
        .unwrap_or(&[])
        .iter()
        .filter_map(value::car_cdr)
        .map(|(k, v)| {
            let k = value::princ(&k);
            let k = if k.starts_with(':') { k } else { format!(":{k}") };
            (k, v)
        })
        .collect()
}

/// The header arguments of the exclusive groups of a key.
fn groups(key: &str) -> &'static [&'static [&'static str]] {
    COMMON.iter().find(|(k, _)| *k == key).and_then(|(_, v)| *v).unwrap_or(&[])
}

/// org-babel-merge-params.
pub fn merge(alists: &[&[(String, Sexp)]]) -> Result<Params, String> {
    let merge_excl = |out: &mut Vec<String>, groups: &[&[&str]], new: Vec<String>| {
        for p in new {
            for g in groups {
                if g.contains(&p.as_str()) {
                    out.retain(|o| !g.contains(&o.as_str()));
                }
            }
            out.retain(|o| *o != p);
            out.push(p);
        }
    };
    let mut vars: Vec<(String, Sexp)> = vec![];
    let mut clearnames: Vec<String> = vec![];
    let mut variable_index = 0;
    let mut params: Params = vec![];
    let mut results: Vec<String> = vec![];
    let mut exports: Vec<String> = vec![];
    for alist in alists {
        for (k, v) in alist.iter() {
            match k.as_str() {
                ":var" => {
                    let name = match v {
                        Sexp::Str(s) => {
                            let n = s.find('=').map(|i| &s[..i]);
                            n.map(|n| n.trim_end_matches([' ', '\t']))
                                .filter(|n| !n.is_empty() && !n.contains([' ', '\x0c', '\t', '\n', '\r', '\x0b']))
                                .map(str::to_owned)
                        }
                        // Processed (name . value) pairs.
                        v if value::listp(v) && !v.is_nil() => value::car_cdr(v).map(|(n, _)| value::princ(&n)),
                        _ => None,
                    };
                    if let Some(name) = name {
                        if vars.iter().any(|(n, _)| *n == name) {
                            clearnames.push(name.clone());
                            vars.retain(|(n, _)| *n != name);
                        }
                        vars.push((name, v.clone()));
                    } else if variable_index < vars.len() && !v.is_nil() {
                        let name = vars[variable_index].0.clone();
                        clearnames.push(name.clone());
                        vars[variable_index].1 = Sexp::Str(format!("{name}={}", value::as_string(v).unwrap_or_default()));
                        variable_index += 1;
                    } else if v.is_nil() {
                    } else {
                        return Err(format!("Variable \"{}\" must be assigned a default value", value::princ(v)));
                    }
                }
                ":results" => {
                    let s = value::as_string(v).unwrap_or_default();
                    merge_excl(&mut results, groups("results"), s.split_whitespace().map(str::to_owned).collect());
                }
                ":exports" => {
                    let s = value::as_string(v).unwrap_or_default();
                    merge_excl(&mut exports, groups("exports"), s.split_whitespace().map(str::to_owned).collect());
                }
                _ => {
                    params.retain(|(x, _)| x != k);
                    params.insert(0, (k.clone(), v.clone()));
                }
            }
        }
    }
    let mut out: Params = vars.into_iter().map(|(_, v)| (":var".to_owned(), v)).collect();
    out.extend(params);
    for name in &clearnames {
        for key in [":colname-names", ":rowname-names"] {
            if let Some(i) = out.iter().position(|(k, _)| k == key) {
                let kept: Vec<Sexp> = value::items(&out[i].1)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|p| value::car_cdr(p).is_none_or(|(n, _)| value::princ(&n) != *name))
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    out.remove(i);
                } else {
                    out[i].1 = value::list(kept);
                }
            }
        }
    }
    let mut head: Params = vec![
        (":results".into(), Sexp::Str(dedup(results).join(" "))),
        (":exports".into(), Sexp::Str(dedup(exports).join(" "))),
    ];
    head.extend(out);
    Ok(head)
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// org-babel-noweb-p.
pub fn noweb_p(p: &[(String, Sexp)], context: &str) -> bool {
    let allowed: &[&str] = match context {
        ":tangle" => &["yes", "tangle", "no-export", "strip-export", "strip-tangle"],
        ":eval" => &["yes", "no-export", "strip-export", "eval", "strip-tangle"],
        _ => &["yes", "strip-tangle"],
    };
    get_str(p, ":noweb").unwrap_or_default().split_whitespace().any(|v| allowed.contains(&v))
}

/// org-string-distance (Levenshtein).
pub fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != *cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_split_like_upstream() {
        assert_eq!(
            balanced_split(":a 1 :b [2 3] :c (4 :d (5 6))", Alts::Pair(&[' ', '\t'], &[':'])),
            vec![":a 1", "b [2 3]", "c (4 :d (5 6))"]
        );
        assert_eq!(balanced_split(":a 1 :b [2 3] :c (4 :d (5 6))", Alts::Char(':')), vec!["a 1 ", "b [2 3] ", "c (4 :d (5 6))"]);
        assert_eq!(balanced_split("a=\"b c\" d=(e f)", Alts::Char(' ')), vec!["a=\"b c\"", "d=(e f)"]);
    }

    #[test]
    fn parse_headers() {
        let p = parse_header_arguments(":results output :var a=1 b=\"x y\" :tangle", true).unwrap();
        assert_eq!(p[0], (":results".to_owned(), Sexp::Str("output".into())));
        assert_eq!(p[1], (":var".to_owned(), Sexp::Str("a=1".into())));
        assert_eq!(p[2], (":var".to_owned(), Sexp::Str("b=\"x y\"".into())));
        assert_eq!(p[3], (":tangle".to_owned(), Sexp::Nil));
        let p = parse_header_arguments(":var x = 1", true).unwrap();
        assert_eq!(vars_raw(&p), vec!["x=1"]);
    }

    fn vars_raw(p: &Params) -> Vec<String> {
        p.iter().filter(|(k, _)| k == ":var").map(|(_, v)| value::princ(v)).collect()
    }

    #[test]
    fn merge_like_upstream() {
        let a = parse_header_arguments(":results file table :var a=1 :exports both", true).unwrap();
        let b = parse_header_arguments(":results list silent :var a=2 :var b=3", true).unwrap();
        let m = merge(&[&a, &b]).unwrap();
        assert_eq!(get_str(&m, ":results").unwrap(), "list silent");
        assert_eq!(get_str(&m, ":exports").unwrap(), "both");
        assert_eq!(vars_raw(&m), vec!["a=2", "b=3"]);
        let c = parse_header_arguments(":var 5", true).unwrap();
        let m = merge(&[&m, &c]).unwrap();
        assert_eq!(vars_raw(&m), vec!["a=5", "b=3"]);
        assert!(merge(&[&c]).is_err());
        assert_eq!(distance("cahce", "cache"), 2);
    }
}
