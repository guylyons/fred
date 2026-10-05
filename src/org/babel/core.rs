//! ob-core.el, ob-ref.el, ob-lob.el and ob-eval.el on a [`Doc`]: block
//! information, header-argument collection (defaults, properties with
//! inheritance, `#+HEADER`), variable resolution, noweb expansion,
//! evaluation, caching, and results insertion/removal.

use super::doc::{self, Context, Doc, El, Inline, Kind};
use super::params::{self, Params, get, get_str, has};
use super::value::{self, as_string, echo, hline, is_hline, items, list, prin1, str};
use crate::org::sexp::Sexp;
use crate::org::{options, syntax};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Why evaluation stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fail {
    Error(String),
    /// The user must confirm evaluating a block (org-confirm-babel-evaluate):
    /// ask QUESTION, then retry with KEY approved or denied.
    Confirm {
        question: String,
        key: String,
    },
}

impl From<String> for Fail {
    fn from(s: String) -> Fail {
        Fail::Error(s)
    }
}

impl From<&str> for Fail {
    fn from(s: &str) -> Fail {
        Fail::Error(s.into())
    }
}

pub type R<T> = Result<T, Fail>;

pub fn err<T>(s: impl Into<String>) -> R<T> {
    Err(Fail::Error(s.into()))
}

/// Evaluation state for one command.
#[derive(Default)]
pub struct Cx {
    pub approved: HashSet<String>,
    pub denied: HashSet<String>,
    /// During export (org-babel-exp-reference-buffer is non-nil).
    pub export: bool,
    /// The pristine document references resolve in, during export.
    pub reference: Option<Doc>,
    /// The last message.
    pub msg: Option<String>,
    /// org-babel-current-src-block-location (a marker in the document).
    pub location: Option<usize>,
    pub(super) depth: usize,
    pub(super) noweb: Option<(u64, HashMap<String, Vec<Info>>, bool)>,
    pub(super) settings: Option<(u64, Rc<syntax::Settings>)>,
    /// Error output of evaluations (*Org-Babel Error Output*).
    pub error_output: String,
    /// A buffer to show when the command ends: (name, text).
    pub show: Option<(String, String)>,
}

impl Cx {
    pub fn message(&mut self, m: impl Into<String>) {
        self.msg = Some(m.into());
    }
}

/// A block's information (org-babel-get-src-block-info).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub lang: String,
    pub body: String,
    pub params: Params,
    pub switches: String,
    pub name: Option<String>,
    /// Post-affiliated line (blocks, calls) or object start (inline).
    pub start: Option<(usize, usize)>,
    pub coderef: Option<String>,
    pub preserve_indent: bool,
}

/// The executable thing at a position (org-element-context subset).
pub fn context(doc: &Doc, pos: (usize, usize)) -> Option<Context> {
    let el = doc::element_at(&doc.lines, pos.0)?;
    if doc::holds_objects(&el.kind) && pos.0 < el.stop {
        for o in doc::inlines(pos.0, doc.line(pos.0)) {
            if o.start <= pos.1 && pos.1 < o.end {
                return Some(if o.call {
                    Context::InlineCall(o, el)
                } else {
                    Context::InlineSrc(o, el)
                });
            }
        }
    }
    Some(match el.kind {
        Kind::Src => Context::Src(el),
        Kind::Call => Context::Call(el),
        _ => Context::Other(el),
    })
}

// ---- src block parsing ----

/// `#+begin_src LANG SWITCHES PARAMETERS`.
pub fn parse_begin(line: &str) -> (String, String, String) {
    let t = line.trim_start();
    let rest = &t[t.len().min(11)..];
    let rest_trim = rest.trim_start_matches(' ');
    let lang: String = if rest_trim.len() < rest.len() || rest.is_empty() {
        rest_trim
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect()
    } else {
        String::new()
    };
    let mut s = &rest_trim[lang.len()..];
    let mut switches = String::new();
    loop {
        let t = s.trim_start_matches(' ');
        if t.len() == s.len() && !switches.is_empty() {
            break;
        }
        let taken = if let Some(r) = t.strip_prefix("-l \"") {
            r.find('"').map(|q| 4 + q + 1)
        } else if t.starts_with("-i") || t.starts_with("-k") || t.starts_with("-r") {
            Some(2).filter(|_| t[2..].is_empty() || t[2..].starts_with(' '))
        } else if t.starts_with("-n") || t.starts_with("+n") {
            let r = &t[2..];
            let sp = r.len() - r.trim_start_matches(' ').len();
            let d = r[sp..].bytes().take_while(u8::is_ascii_digit).count();
            Some(if d > 0 { 2 + sp + d } else { 2 })
        } else {
            None
        };
        match taken {
            Some(n) if s.len() > t.len() => {
                switches.push_str(&s[..s.len() - t.len() + n]);
                s = &t[n..];
            }
            _ => break,
        }
    }
    (lang, switches.trim().to_owned(), s.trim().to_owned())
}

/// org-unescape-code-in-string.
pub fn unescape(s: &str) -> String {
    s.split('\n')
        .map(|l| {
            let t = l.trim_start_matches([' ', '\t']);
            let ind = l.len() - t.len();
            let commas = t.bytes().take_while(|&b| b == b',').count();
            if commas > 0 && (t[commas..].starts_with('*') || t[commas..].starts_with("#+")) {
                format!("{}{}", &l[..ind], &t[1..])
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// org-escape-code-in-string.
pub fn escape(s: &str) -> String {
    s.split('\n')
        .map(|l| {
            let t = l.trim_start_matches([' ', '\t']);
            let ind = l.len() - t.len();
            let commas = t.bytes().take_while(|&b| b == b',').count();
            if t[commas..].starts_with('*') || t[commas..].starts_with("#+") {
                format!("{},{}", &l[..ind], t)
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The raw value of a block element: its contents, unescaped, with a
/// final newline.
fn block_value(doc: &Doc, el: &El) -> String {
    let lines: Vec<&str> = (el.post + 1..el.stop - 1).map(|l| doc.line(l)).collect();
    let mut v = unescape(&lines.join("\n"));
    if !lines.is_empty() {
        v.push('\n');
    }
    v
}

fn preserve_indentation(switches: &str) -> bool {
    switches.split_whitespace().any(|s| s == "-i")
        || options::bool("org-src-preserve-indentation", false)
}

/// The coderef format of a block: `-l "fmt"` or org-coderef-label-format.
fn coderef_format(switches: &str) -> String {
    if let Some(i) = switches.find("-l \"") {
        let r = &switches[i + 4..];
        if let Some(q) = r.find('"') {
            return r[..q].to_owned();
        }
    }
    options::string("org-coderef-label-format", "(ref:%s)")
}

/// Remove coderefs (org-src-coderef-regexp group 1) from line ends.
pub fn strip_coderefs(body: &str, fmt: &str) -> String {
    let (pre, post) = fmt.split_once("%s").unwrap_or((fmt, ""));
    body.split('\n')
        .map(|l| {
            let t = l.trim_end_matches([' ', '\t']);
            if let Some(stripped) = t.strip_suffix(post)
                && let Some(i) = stripped.rfind(pre)
            {
                let label = &stripped[i + pre.len()..];
                let ok = !label.is_empty()
                    && label
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
                    && label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_ ".contains(c));
                if ok {
                    return l[..i].trim_end_matches([' ', '\t']).to_owned();
                }
            }
            l.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_body(value: &str, preserve: bool) -> String {
    let body = value.strip_suffix('\n').unwrap_or(value);
    if preserve {
        body.to_owned()
    } else {
        value::remove_indentation(body)
    }
}

// ---- properties ----

fn settings(doc: &Doc, cx: &mut Cx) -> Rc<syntax::Settings> {
    if let Some((t, s)) = &cx.settings
        && *t == doc.tick
    {
        return Rc::clone(s);
    }
    let dir = doc.path.as_deref().and_then(Path::parent);
    let s = Rc::new(syntax::settings(doc.lines.iter().map(String::as_str), dir));
    cx.settings = Some((doc.tick, Rc::clone(&s)));
    s
}

/// Property drawer lines of heading `h` (None: the file's drawer).
fn drawer(doc: &Doc, h: Option<usize>) -> Option<(usize, usize)> {
    let mut l = match h {
        Some(h) => h + 1,
        None => {
            let mut l = 0;
            while l < doc.len()
                && (doc.line(l).trim().is_empty()
                    || doc.line(l).trim_start().starts_with('#')
                        && !doc.line(l).trim_start().starts_with("#+"))
            {
                l += 1;
            }
            l
        }
    };
    if h.is_some() && l < doc.len() && crate::org::ctx::is_planning(doc.line(l)) {
        l += 1;
    }
    if l >= doc.len() || !doc.line(l).trim().eq_ignore_ascii_case(":PROPERTIES:") {
        return None;
    }
    let end = (l + 1..doc.len())
        .take_while(|&i| syntax::level(doc.line(i)).is_none())
        .find(|&i| doc.line(i).trim().eq_ignore_ascii_case(":END:"))?;
    Some((l, end))
}

/// org--property-local-values: (base value present, values in order).
pub fn local_values(doc: &Doc, h: Option<usize>, prop: &str) -> (bool, Vec<String>) {
    let Some((s, e)) = drawer(doc, h) else {
        return (false, vec![]);
    };
    let mut base = false;
    let mut out = vec![];
    for l in s + 1..e {
        let t = doc.line(l).trim_start();
        let Some(rest) = t.strip_prefix(':') else {
            continue;
        };
        let Some(c) = rest.find(':') else { continue };
        let key = &rest[..c];
        let v = rest[c + 1..].trim();
        if key.eq_ignore_ascii_case(prop) {
            base = true;
            out.push(v.to_owned());
        } else if key
            .strip_suffix('+')
            .is_some_and(|k| k.eq_ignore_ascii_case(prop))
        {
            out.push(v.to_owned());
        }
    }
    (base, out)
}

/// org-entry-get at line `l` for PROP; with INHERIT, the inherited and
/// accumulated (`PROP+`) value.
pub fn entry_get(doc: &Doc, cx: &mut Cx, l: usize, prop: &str, inherit: bool) -> Option<String> {
    let mut h = if doc.is_empty() {
        None
    } else {
        syntax::heading_at_or_before(&doc.lines, l.min(doc.len() - 1))
    };
    let mut value: Option<String> = None;
    loop {
        let (base, v) = local_values(doc, h, prop);
        if !v.is_empty() {
            let joined = v.join(" ");
            value = Some(match value {
                Some(old) => format!("{joined} {old}"),
                None => joined,
            });
        }
        if base || !inherit {
            return value;
        }
        match h {
            Some(x) => h = syntax::parent(&doc.lines, x),
            None => break,
        }
        if h.is_none() {
            // The file-level drawer, then keywords.
            let (base, v) = local_values(doc, None, prop);
            if !v.is_empty() {
                let joined = v.join(" ");
                value = Some(match value {
                    Some(old) => format!("{joined} {old}"),
                    None => joined,
                });
            }
            if base {
                return value;
            }
            break;
        }
    }
    let st = settings(doc, cx);
    let global = st
        .properties
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(prop))
        .map(|(_, v)| v.clone())
        .or_else(|| {
            crate::org::sexp::option("org-global-properties").and_then(|v| {
                params::alist(&v)
                    .into_iter()
                    .find(|(k, _)| k.trim_start_matches(':').eq_ignore_ascii_case(prop))
                    .and_then(|(_, v)| as_string(&v))
            })
        });
    match (global, value) {
        (Some(g), Some(v)) => Some(format!("{g} {v}")),
        (g, v) => v.or(g),
    }
}

/// org-babel-params-from-properties.
pub fn params_from_properties(
    doc: &Doc,
    cx: &mut Cx,
    l: usize,
    lang: &str,
    no_eval: bool,
) -> R<Vec<Params>> {
    let a = entry_get(doc, cx, l, "header-args", true).unwrap_or_default();
    let b = if lang.is_empty() {
        String::new()
    } else {
        entry_get(doc, cx, l, &format!("header-args:{lang}"), true).unwrap_or_default()
    };
    Ok(vec![
        params::parse_header_arguments(&a, no_eval)?,
        params::parse_header_arguments(&b, no_eval)?,
    ])
}

fn lang_defaults(lang: &str) -> Params {
    let lang = if lang == "elisp" { "emacs-lisp" } else { lang };
    let name = if super::lang::shell_names().iter().any(|s| s == lang) {
        "shell"
    } else {
        lang
    };
    let mut p = params::default_args(&format!("org-babel-default-header-args:{name}"));
    if name != lang {
        p.extend(params::default_args(&format!(
            "org-babel-default-header-args:{lang}"
        )));
    }
    p
}

/// org-babel-get-src-block-info at POS.
pub fn get_info(doc: &mut Doc, cx: &mut Cx, pos: (usize, usize), no_eval: bool) -> R<Option<Info>> {
    match context(doc, pos) {
        Some(Context::Src(el)) => src_info(doc, cx, &el, no_eval).map(Some),
        Some(Context::InlineSrc(o, _)) => inline_info(doc, cx, &o, no_eval).map(Some),
        _ => Ok(None),
    }
}

/// Info for a src-block element.
pub fn src_info(doc: &mut Doc, cx: &mut Cx, el: &El, no_eval: bool) -> R<Info> {
    let (lang, switches, parameters) = parse_begin(doc.line(el.post));
    let preserve = preserve_indentation(&switches);
    let body = normalize_body(&block_value(doc, el), preserve);
    let mut alists = vec![
        params::default_args("org-babel-default-header-args"),
        lang_defaults(&lang),
    ];
    alists.extend(params_from_properties(doc, cx, el.post, &lang, no_eval)?);
    alists.push(params::parse_header_arguments(&parameters, no_eval)?);
    for h in &el.headers {
        alists.push(params::parse_header_arguments(h, no_eval)?);
    }
    let refs: Vec<&[(String, Sexp)]> = alists.iter().map(Vec::as_slice).collect();
    let mut p = params::merge(&refs)?;
    if !no_eval {
        p = process_params(doc, cx, p)?;
    }
    let p = generate_file_param(doc, el.name.as_deref(), p)?;
    Ok(Info {
        lang,
        body,
        params: p,
        switches,
        name: el.name.clone(),
        start: Some((el.post, 0)),
        coderef: Some(coderef_format(&parse_begin(doc.line(el.post)).1)),
        preserve_indent: preserve,
    })
}

fn inline_info(doc: &mut Doc, cx: &mut Cx, o: &Inline, no_eval: bool) -> R<Info> {
    let body = o
        .body
        .split('\n')
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join(" ");
    let mut alists = vec![
        params::default_args("org-babel-default-inline-header-args"),
        lang_defaults(&o.name),
    ];
    alists.extend(params_from_properties(doc, cx, o.line, &o.name, no_eval)?);
    alists.push(params::parse_header_arguments(
        o.inside.as_deref().unwrap_or(""),
        no_eval,
    )?);
    let refs: Vec<&[(String, Sexp)]> = alists.iter().map(Vec::as_slice).collect();
    let mut p = params::merge(&refs)?;
    if !no_eval {
        p = process_params(doc, cx, p)?;
    }
    let p = generate_file_param(doc, None, p)?;
    Ok(Info {
        lang: o.name.clone(),
        body,
        params: p,
        start: Some((o.line, o.start)),
        ..Info::default()
    })
}

/// org-babel-generate-file-param.
fn generate_file_param(doc: &Doc, name: Option<&str>, mut p: Params) -> R<Params> {
    let dir = get_str(&p, ":output-dir");
    if let Some(d) = &dir {
        let _ = std::fs::create_dir_all(doc.dir().join(d));
    }
    if let Some(i) = p.iter().position(|(k, _)| k == ":file") {
        if let (Some(d), Some(f)) = (&dir, as_string(&p[i].1)) {
            p[i].1 = str(format!("{}/{f}", d.trim_end_matches('/')));
        }
    } else if let (Some(n), Some(ext)) = (name, get_str(&p, ":file-ext")) {
        let f = match &dir {
            Some(d) => format!("{}/{n}.{ext}", d.trim_end_matches('/')),
            None => format!("{n}.{ext}"),
        };
        p.insert(0, (":file".into(), str(f)));
    }
    Ok(p)
}

/// org-babel-graphical-output-file.
pub fn graphical_output_file(p: &[(String, Sexp)]) -> R<Option<String>> {
    match get_str(p, ":file") {
        Some(f) => Ok(params::result_params(p)
            .iter()
            .any(|x| x == "graphics")
            .then_some(f)),
        None if has(p, ":file-ext") => {
            err(":file-ext given but no :file generated; did you forget to name a block?")
        }
        None => err("No :file header argument given; cannot create graphical result"),
    }
}

// ---- babel calls (ob-lob.el) ----

thread_local! {
    /// org-babel-library-of-babel.
    static LIBRARY: std::cell::RefCell<Vec<(String, Info)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn library_get(name: &str) -> Option<Info> {
    LIBRARY.with(|l| {
        l.borrow()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, i)| i.clone())
    })
}

pub fn library_add(name: String, info: Info) {
    LIBRARY.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(n, _)| *n != name);
        l.insert(0, (name, info));
    });
}

/// org-babel-lob-ingest on a document: named blocks into the library.
pub fn lob_ingest(doc: &mut Doc, cx: &mut Cx) -> R<usize> {
    let mut n = 0;
    for el in doc::all_elements(&doc.lines) {
        if el.kind == Kind::Src {
            let mut info = src_info(doc, cx, &el, true)?;
            if let Some(name) = info.name.clone() {
                info.start = None;
                library_add(name, info);
                n += 1;
            }
        }
    }
    cx.message(format!(
        "{n} source block{} added to Library of Babel",
        if n > 1 { "s" } else { "" }
    ));
    Ok(n)
}

/// A call's parts: name, inside header, arguments, end header.
pub fn parse_call(value: &str) -> (String, Option<String>, Option<String>, Option<String>) {
    let v = value.trim();
    let name_end = v.find(['[', ']', '(', ')']).unwrap_or(v.len());
    let name = v[..name_end].to_owned();
    let mut p = name_end;
    let mut inside = None;
    let bracket = |s: &str, o: char, c: char| -> Option<usize> {
        let mut d = 0;
        for (i, ch) in s.char_indices() {
            if ch == o {
                d += 1;
            } else if ch == c {
                d -= 1;
                if d == 0 {
                    return Some(i + 1);
                }
            }
        }
        None
    };
    if v[p..].starts_with('[')
        && let Some(e) = bracket(&v[p..], '[', ']')
    {
        inside = Some(v[p + 1..p + e - 1].to_owned()).filter(|s| !s.trim().is_empty());
        p += e;
    }
    let mut args = None;
    if v[p..].starts_with('(')
        && let Some(e) = bracket(&v[p..], '(', ')')
    {
        args = Some(v[p + 1..p + e - 1].to_owned()).filter(|s| !s.trim().is_empty());
        p += e;
    }
    let end = Some(v[p..].trim().to_owned()).filter(|s| !s.is_empty());
    (name, inside, args, end)
}

/// org-babel-ref-split-args.
pub fn split_args(s: &str) -> Vec<String> {
    params::balanced_split(s, params::Alts::Char(','))
        .iter()
        .map(|a| value::trim(a).to_owned())
        .collect()
}

/// org-babel-lob--src-info: a named block or call, or a library entry.
fn lob_src_info(doc: &mut Doc, cx: &mut Cx, reference: &str, eval: bool) -> R<Option<Info>> {
    if let Some((file, name)) = reference
        .split_once(':')
        .filter(|(f, n)| !f.is_empty() && !n.is_empty())
    {
        let path = doc.dir().join(file);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut other = Doc::new(&text, Some(path));
            return lob_src_info_in(&mut other, cx, name, eval);
        }
    }
    if let Some(mut r) = cx.reference.take() {
        let out = lob_src_info_in(&mut r, cx, reference, eval);
        cx.reference = Some(r);
        if let Ok(Some(i)) = out {
            return Ok(Some(i));
        }
        return out.map(|_| library_get(reference));
    }
    lob_src_info_in(doc, cx, reference, eval)
}

fn lob_src_info_in(doc: &mut Doc, cx: &mut Cx, name: &str, eval: bool) -> R<Option<Info>> {
    for el in doc::all_elements(&doc.lines) {
        if el.name.as_deref() != Some(name) {
            continue;
        }
        return match el.kind {
            Kind::Src => src_info(doc, cx, &el, !eval).map(Some),
            Kind::Call => call_info(doc, cx, &el, !eval),
            _ => Ok(None),
        };
    }
    Ok(library_get(name))
}

/// org-babel-lob-get-info for a #+CALL element.
pub fn call_info(doc: &mut Doc, cx: &mut Cx, el: &El, no_eval: bool) -> R<Option<Info>> {
    let line = doc.line(el.post).to_owned();
    let value = line.split_once(':').map_or("", |(_, v)| v.trim());
    let (name, inside, args, end) = parse_call(value);
    lob_info(
        doc,
        cx,
        &name,
        inside,
        args,
        end,
        el.name.clone(),
        (el.post, 0),
        no_eval,
    )
}

/// org-babel-lob-get-info for an inline call.
pub fn inline_call_info(doc: &mut Doc, cx: &mut Cx, o: &Inline, no_eval: bool) -> R<Option<Info>> {
    let args = Some(o.body.clone()).filter(|s| !s.trim().is_empty());
    lob_info(
        doc,
        cx,
        &o.name,
        o.inside.clone(),
        args,
        o.end_header.clone(),
        None,
        (o.line, o.start),
        no_eval,
    )
}

#[allow(clippy::too_many_arguments)]
fn lob_info(
    doc: &mut Doc,
    cx: &mut Cx,
    reference: &str,
    inside: Option<String>,
    args: Option<String>,
    end: Option<String>,
    name: Option<String>,
    begin: (usize, usize),
    no_eval: bool,
) -> R<Option<Info>> {
    let id = doc.mark(begin);
    let saved = cx.location.replace(id);
    let out = (|| -> R<Option<Info>> {
        let Some(src) = lob_src_info(doc, cx, reference, !no_eval)? else {
            return Ok(None);
        };
        let begin = doc.marker(id);
        let mut alists = vec![
            src.params.clone(),
            params::default_args("org-babel-default-lob-header-args"),
        ];
        alists.extend(params_from_properties(
            doc, cx, begin.0, &src.lang, no_eval,
        )?);
        alists.push(params::parse_header_arguments(
            inside.as_deref().unwrap_or(""),
            no_eval,
        )?);
        alists.push(
            args.as_deref()
                .map(|a| {
                    split_args(a)
                        .into_iter()
                        .map(|r| (":var".to_owned(), Sexp::Str(r)))
                        .collect()
                })
                .unwrap_or_default(),
        );
        alists.push(params::parse_header_arguments(
            end.as_deref().unwrap_or(""),
            no_eval,
        )?);
        let refs: Vec<&[(String, Sexp)]> = alists.iter().map(Vec::as_slice).collect();
        Ok(Some(Info {
            lang: src.lang,
            body: src.body,
            params: params::merge(&refs)?,
            switches: String::new(),
            name,
            start: Some(begin),
            coderef: src.coderef,
            preserve_indent: src.preserve_indent,
        }))
    })();
    cx.location = saved;
    doc.unmark(id);
    out
}

// ---- variables (ob-ref.el) ----

/// org-babel-process-params.
pub fn process_params(doc: &mut Doc, cx: &mut Cx, p: Params) -> R<Params> {
    let mut vars = vec![];
    for (k, v) in &p {
        if k != ":var" {
            continue;
        }
        match v {
            Sexp::Str(s) => {
                if let Some(pair) = ref_parse(doc, cx, s)? {
                    vars.push(pair);
                }
            }
            v if v.is_nil() => {}
            v => vars.push(
                value::car_cdr(v)
                    .map(|(n, v)| (value::princ(&n), v))
                    .ok_or("bad :var")?,
            ),
        }
    }
    let (vars, cnames, rnames) = if has(&p, ":colname-names") && has(&p, ":rowname-names") {
        (vars, None, None)
    } else {
        let (v, c, r) = disassemble_tables(
            vars,
            get_str(&p, ":hlines"),
            get(&p, ":colnames").cloned(),
            get(&p, ":rownames").cloned(),
        );
        (v, Some(c), Some(r))
    };
    let raw = get(&p, ":results")
        .map(|v| as_string(v).unwrap_or_default())
        .unwrap_or_default();
    let mut rp: Vec<String> = vec![];
    for x in raw
        .split_whitespace()
        .map(str::to_owned)
        .chain(params::result_params(&p))
    {
        if !rp.contains(&x) {
            rp.push(x);
        }
    }
    let pairs = |v: Vec<(String, Sexp)>| {
        list(
            v.into_iter()
                .map(|(n, x)| value::cons(Sexp::Sym(n), x))
                .collect(),
        )
    };
    let mut out: Params = vars
        .into_iter()
        .map(|(n, v)| (":var".to_owned(), value::cons(Sexp::Sym(n), v)))
        .collect();
    let keep = |key: &str, new: Option<Vec<(String, Sexp)>>| -> Sexp {
        match get(&p, key) {
            Some(v) if !v.is_nil() => v.clone(),
            _ => new.map(pairs).unwrap_or(Sexp::Nil),
        }
    };
    out.push((":colname-names".into(), keep(":colname-names", cnames)));
    out.push((":rowname-names".into(), keep(":rowname-names", rnames)));
    let rtype = if rp.iter().any(|x| x == "output") {
        "output"
    } else {
        "value"
    };
    out.push((
        ":result-params".into(),
        list(rp.into_iter().map(Sexp::Str).collect()),
    ));
    out.push((":result-type".into(), Sexp::Sym(rtype.into())));
    out.extend(p.into_iter().filter(|(k, _)| {
        !matches!(
            k.as_str(),
            ":colname-names" | ":rowname-names" | ":result-params" | ":result-type" | ":var"
        )
    }));
    Ok(out)
}

type Names = Vec<(String, Sexp)>;

/// org-babel-disassemble-tables.
fn disassemble_tables(
    vars: Vec<(String, Sexp)>,
    hlines: Option<String>,
    colnames: Option<Sexp>,
    rownames: Option<Sexp>,
) -> (Vec<(String, Sexp)>, Names, Names) {
    let cn = colnames.as_ref().and_then(as_string);
    let rn_on = rownames
        .as_ref()
        .is_some_and(|r| !r.is_nil() && as_string(r).as_deref() != Some("no"));
    let mut cnames = vec![];
    let mut rnames = vec![];
    let vars = vars
        .into_iter()
        .map(|(name, mut v)| {
            if let Some(rows) = items(&v).map(<[Sexp]>::to_vec) {
                let auto = cn.is_none() || cn.as_deref() == Some("nil");
                let take = cn.as_deref() != Some("no")
                    && (if auto {
                        rows.len() > 1
                            && !is_hline(&rows[0])
                            && is_hline(&rows[1])
                            && !rows[2..].iter().any(is_hline)
                    } else {
                        true
                    });
                let mut rows = rows;
                if take {
                    while rows.first().is_some_and(is_hline) {
                        rows.remove(0);
                    }
                    let names = rows.first().cloned().unwrap_or(Sexp::Nil);
                    let skip = if rows.get(1).is_some_and(is_hline) {
                        2
                    } else {
                        1
                    };
                    cnames.push((name.clone(), names));
                    rows = rows.into_iter().skip(skip).collect();
                }
                if rn_on {
                    rows.retain(|r| !is_hline(r));
                    let mut names = vec![];
                    for r in &mut rows {
                        if let Some(c) = items(r).map(<[Sexp]>::to_vec) {
                            names.push(c.first().cloned().unwrap_or(Sexp::Nil));
                            *r = list(c.into_iter().skip(1).collect());
                        }
                    }
                    rnames.push((name.clone(), list(names)));
                }
                if hlines.as_deref().is_some_and(|h| h != "yes") {
                    rows.retain(|r| !is_hline(r));
                }
                v = list(rows);
            }
            (name, v)
        })
        .collect();
    (vars, cnames, rnames)
}

/// org-babel-pick-name.
pub fn pick_name(names: Option<&Sexp>, selector: Option<&Sexp>) -> Option<Sexp> {
    let sel = selector.cloned().unwrap_or(Sexp::Nil);
    if value::listp(&sel) {
        return if sel.is_nil() { None } else { Some(sel) };
    }
    let names = names.filter(|n| !n.is_nil())?;
    let all = items(names)?;
    match &sel {
        Sexp::Int(i) => all
            .get((*i as usize).wrapping_sub(1))
            .and_then(|p| value::car_cdr(p))
            .map(|(_, v)| v),
        Sexp::Str(s) if s == "yes" || s == "t" => {
            all.last().and_then(|p| value::car_cdr(p)).map(|(_, v)| v)
        }
        Sexp::T => all.last().and_then(|p| value::car_cdr(p)).map(|(_, v)| v),
        Sexp::Sym(s) | Sexp::Str(s) => all
            .iter()
            .filter_map(value::car_cdr)
            .find(|(n, _)| value::princ(n) == *s)
            .map(|(_, v)| v),
        _ => all.last().and_then(|p| value::car_cdr(p)).map(|(_, v)| v),
    }
}

/// org-babel-reassemble-table.
pub fn reassemble_table(table: Sexp, colnames: Option<Sexp>, rownames: Option<Sexp>) -> Sexp {
    let Some(rows) = items(&table).map(<[Sexp]>::to_vec) else {
        return table;
    };
    let mut rows = rows;
    if let Some(rn) = rownames
        .as_ref()
        .and_then(|r| items(r).map(<[Sexp]>::to_vec))
        && rn.len() == rows.len()
    {
        let mut it = rn.into_iter();
        rows = rows
            .into_iter()
            .map(|r| {
                if value::listp(&r) && !is_hline(&r) {
                    value::cons(it.next().unwrap_or(str("")), r)
                } else {
                    r
                }
            })
            .collect();
    }
    if let Some(cn) = colnames
        .as_ref()
        .and_then(|c| items(c).map(<[Sexp]>::to_vec))
        && let Some(first) = rows.first().and_then(|r| items(r))
        && first.len() == cn.len()
        && !cn.is_empty()
    {
        let mut out = vec![list(cn), hline()];
        out.extend(rows);
        return list(out);
    }
    list(rows)
}

/// org-babel-reassemble-table with the block's names (the last step of
/// most org-babel-execute:LANG).
pub fn reassemble(result: Sexp, p: &[(String, Sexp)]) -> Sexp {
    reassemble_table(
        result,
        pick_name(get(p, ":colname-names"), get(p, ":colnames")),
        pick_name(get(p, ":rowname-names"), get(p, ":rownames")),
    )
}

/// org-babel-ref-parse: `name=value` with the value read or resolved.
pub fn ref_parse(doc: &mut Doc, cx: &mut Cx, assignment: &str) -> R<Option<(String, Sexp)>> {
    let Some(i) = assignment.find('=').filter(|&i| i > 0) else {
        return Ok(None);
    };
    let var = value::trim(&assignment[..i]).to_owned();
    let r = value::trim(&assignment[i + 1..]).to_owned();
    let out = value::read(&r, false)?;
    if out == Sexp::Str(r.clone()) {
        if r.len() >= 2 && r.starts_with('"') && r.ends_with('"') {
            return Ok(Some((
                var,
                crate::org::sexp::read(&r).map_err(|e| e.to_string())?,
            )));
        }
        return Ok(Some((var, ref_resolve(doc, cx, &r)?)));
    }
    Ok(Some((var, out)))
}

/// org-in-commented-heading-p at line `l`.
pub fn in_commented_heading(doc: &Doc, l: usize) -> bool {
    let st = syntax::Settings::default();
    let mut h = if doc.is_empty() {
        None
    } else {
        syntax::heading_at_or_before(&doc.lines, l.min(doc.len() - 1))
    };
    while let Some(x) = h {
        if syntax::headline(doc.line(x), &st).is_some_and(|hl| hl.commented) {
            return true;
        }
        h = syntax::parent(&doc.lines, x);
    }
    false
}

/// org-in-archived-heading-p at line `l`.
pub fn in_archived_heading(doc: &Doc, l: usize) -> bool {
    let st = syntax::Settings::default();
    let mut h = if doc.is_empty() {
        None
    } else {
        syntax::heading_at_or_before(&doc.lines, l.min(doc.len() - 1))
    };
    while let Some(x) = h {
        if syntax::headline(doc.line(x), &st).is_some_and(|hl| hl.archived()) {
            return true;
        }
        h = syntax::parent(&doc.lines, x);
    }
    false
}

/// org-babel-ref-goto-headline-id: a heading with CUSTOM_ID or ID.
pub fn headline_by_id(doc: &Doc, id: &str) -> Option<usize> {
    for prop in ["CUSTOM_ID", "ID"] {
        for h in (0..doc.len()).filter(|&l| syntax::level(doc.line(l)).is_some()) {
            let (_, v) = local_values(doc, Some(h), prop);
            if v.iter().any(|x| x == id) {
                return Some(h);
            }
        }
    }
    None
}

/// org-babel-ref-headline-body: the subtree after its meta data.
pub fn headline_body(doc: &Doc, h: usize) -> String {
    let end = syntax::subtree_end(&doc.lines, h);
    let mut l = h + 1;
    if l < end && crate::org::ctx::is_planning(doc.line(l)) {
        l += 1;
    }
    if let Some((_, e)) = drawer(doc, Some(h)) {
        l = l.max(e + 1);
    }
    let mut s = doc.lines[l.min(end)..end].join("\n");
    if end > l {
        s.push('\n');
    }
    s
}

/// org-babel-ref-resolve.
pub fn ref_resolve(doc: &mut Doc, cx: &mut Cx, reference: &str) -> R<Sexp> {
    if let Some(mut r) = cx.reference.take() {
        cx.export = true;
        let out = ref_resolve_in(&mut r, cx, reference);
        cx.reference = Some(r);
        return out;
    }
    ref_resolve_in(doc, cx, reference)
}

fn ref_resolve_in(doc: &mut Doc, cx: &mut Cx, reference: &str) -> R<Sexp> {
    cx.depth += 1;
    if cx.depth > 100 {
        cx.depth = 0;
        return err(format!("Reference `{reference}' is circular"));
    }
    let out = ref_resolve_1(doc, cx, reference);
    cx.depth = cx.depth.saturating_sub(1);
    out
}

fn ref_resolve_1(doc: &mut Doc, cx: &mut Cx, reference: &str) -> R<Sexp> {
    let mut r = reference.to_owned();
    let mut index = None;
    let mut contents = false;
    // A trailing [index] (nested brackets excluded, parens balanced before).
    if r.ends_with(']')
        && let Some(open) = r.rfind('[')
        && !r[open + 1..r.len() - 1].contains('[')
    {
        let before = &r[..open];
        if before.matches('(').count() == before.matches(')').count() {
            let ix = r[open + 1..r.len() - 1].to_owned();
            if ix.is_empty() {
                contents = true;
            } else {
                index = Some(ix);
            }
            r.truncate(open);
        }
    }
    // name[header](args)
    let mut args: Params = vec![];
    if r.ends_with(')')
        && let Some(open) = find_args_open(&r)
    {
        let head = &r[..open];
        let referent = r[open + 1..r.len() - 1].to_owned();
        let (name, hdr) = match head.find('[') {
            Some(b) if head.ends_with(']') => {
                (head[..b].to_owned(), head[b + 1..head.len() - 1].to_owned())
            }
            _ => (head.to_owned(), String::new()),
        };
        if !name.is_empty() {
            if !referent.is_empty() {
                args = split_args(&referent)
                    .into_iter()
                    .map(|a| (":var".to_owned(), Sexp::Str(a)))
                    .collect();
            }
            if !hdr.is_empty() {
                let mut h = params::parse_header_arguments(&hdr, false)?;
                h.extend(args);
                args = h;
            }
            r = name;
        }
    }
    // file:name
    if let Some((f, n)) = r
        .split_once(':')
        .filter(|(f, n)| !f.is_empty() && !n.is_empty())
    {
        let path = doc.dir().join(f);
        if path.is_file() {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut other = Doc::new(&text, Some(path));
            let n = n.to_owned();
            let reference = format_ref(&n, &args, index.as_deref(), contents);
            return ref_resolve_in(&mut other, cx, &reference);
        }
    }
    let update = options::string("org-babel-update-intermediate", "nil");
    let mut p = args.clone();
    if update == "nil" || update == "cached" {
        p.push((":results".into(), str("none")));
    }
    let result = 'found: {
        for el in doc::all_elements(&doc.lines) {
            if el.name.as_deref() != Some(r.as_str()) || in_commented_heading(doc, el.post) {
                continue;
            }
            match el.kind {
                Kind::Call => {
                    let Some(info) = call_info(doc, cx, &el, false)? else {
                        return err("Reference not found");
                    };
                    break 'found execute_src_block(
                        doc,
                        cx,
                        false,
                        Some(info),
                        &p,
                        ExecKind::Call,
                    )?
                    .unwrap_or(Sexp::Nil);
                }
                Kind::Src if !contents => {
                    let id = doc.mark((el.post, 0));
                    let info = src_info(doc, cx, &el, false)?;
                    let out = execute_src_block(doc, cx, false, Some(info), &p, ExecKind::Src);
                    doc.unmark(id);
                    break 'found out?.unwrap_or(Sexp::Nil);
                }
                _ => match read_element(doc, &el)? {
                    Some(v) => break 'found v,
                    None => return err("Reference not found"),
                },
            }
        }
        if let Some(h) = headline_by_id(doc, &r) {
            break 'found str(headline_body(doc, h));
        }
        if let Some(info) = library_get(&r) {
            let mut lp = args.clone();
            lp.push((":results".into(), str("none")));
            break 'found execute_src_block(doc, cx, false, Some(info), &lp, ExecKind::Src)?
                .unwrap_or(Sexp::Nil);
        }
        return err(format!("Reference `{r}' not found in this buffer"));
    };
    Ok(match (&result, index) {
        (Sexp::Sym(_) | Sexp::T, _) => str(prin1(&result)),
        (Sexp::Nil, _) => Sexp::Nil,
        (v, Some(ix)) if value::listp(v) => index_list(&ix, v),
        _ => result,
    })
}

fn format_ref(name: &str, args: &Params, index: Option<&str>, contents: bool) -> String {
    let mut s = name.to_owned();
    let vars: Vec<String> = args
        .iter()
        .filter(|(k, _)| k == ":var")
        .filter_map(|(_, v)| as_string(v))
        .collect();
    if !vars.is_empty() {
        s.push_str(&format!("({})", vars.join(", ")));
    }
    if let Some(i) = index {
        s.push_str(&format!("[{i}]"));
    } else if contents {
        s.push_str("[]");
    }
    s
}

/// The `(` opening a trailing `(...)` argument list (lazy name match).
fn find_args_open(r: &str) -> Option<usize> {
    // ^\(.+?\)\(\[\(.*\)\]\|\(\)\)(\(.*\))$ : the first `(` after a
    // non-empty name, optionally preceded by a `[...]` header.
    let b = r.as_bytes();
    for i in 1..b.len() {
        if b[i] == b'(' {
            return Some(i);
        }
        if b[i] == b'[' {
            // name[header](...)
            let close = r[i..].find("](")?;
            return Some(i + close + 1);
        }
    }
    None
}

/// org-babel-ref-index-list.
pub fn index_list(index: &str, lis: &Sexp) -> Sexp {
    if index.is_empty() {
        return lis.clone();
    }
    let (portion, remainder) = match index.find(',') {
        Some(i) => (&index[..i], &index[i + 1..]),
        None => (index, ""),
    };
    let all = items(lis).unwrap_or(&[]);
    let len = all.len() as i64;
    let wrap = |n: i64| if n < 0 { len + n } else { n };
    let picked: Vec<Sexp> = if portion.is_empty() || portion == "*" || portion.contains(':') {
        let (a, b) = match portion.split_once(':') {
            Some((a, b)) => (
                wrap(a.trim().parse().unwrap_or(0)),
                wrap(b.trim().parse().unwrap_or(-1)),
            ),
            None => (wrap(0), wrap(-1)),
        };
        (a..=b)
            .map(|n| all.get(n as usize).cloned().unwrap_or(Sexp::Nil))
            .collect()
    } else {
        let n = wrap(portion.trim().parse().unwrap_or(0));
        vec![all.get(n as usize).cloned().unwrap_or(Sexp::Nil)]
    };
    let mapped: Vec<Sexp> = picked
        .into_iter()
        .map(|sub| {
            if value::listp(&sub) && !sub.is_nil() {
                index_list(remainder, &sub)
            } else {
                sub
            }
        })
        .collect();
    if mapped.len() == 1 {
        mapped.into_iter().next().unwrap()
    } else {
        list(mapped)
    }
}

/// org-babel-read-element.
pub fn read_element(doc: &Doc, el: &El) -> R<Option<Sexp>> {
    Ok(match &el.kind {
        Kind::Fixed => {
            let v: Vec<&str> = (el.post..el.stop)
                .map(|l| {
                    let t = doc.line(l).trim_start();
                    t.strip_prefix(": ")
                        .or_else(|| t.strip_prefix(':'))
                        .unwrap_or(t)
                })
                .collect();
            let v = value::trim(&v.join("\n")).to_owned();
            Some(value::string_to_number(&v).unwrap_or(Sexp::Str(v)))
        }
        Kind::Table => Some(read_table(&doc.lines[el.post..el.stop])),
        Kind::List => Some(read_list(&doc.lines[el.post..el.stop])),
        Kind::Src | Kind::Example => {
            let v = block_value(doc, el);
            let switches = parse_begin(doc.line(el.post)).1;
            Some(str(if preserve_indentation(&switches) {
                v
            } else {
                value::remove_indentation(&v)
            }))
        }
        Kind::Export => Some(str(value::remove_indentation(&block_value(doc, el)))),
        Kind::Paragraph => {
            let text = doc.lines[el.post..el.stop].join("\n") + "\n";
            let t = text.trim();
            if t.starts_with("[[")
                && let Some(e) = t.find("]]")
                && e + 2 == t.len()
            {
                Some(str(read_link(doc, &t[2..e])))
            } else {
                Some(str(text))
            }
        }
        Kind::Block(_) => {
            let lines = &doc.lines[el.post + 1..el.stop - 1];
            let mut s = lines.join("\n");
            if !lines.is_empty() {
                s.push('\n');
            }
            Some(str(value::remove_indentation(&s)))
        }
        _ => None,
    })
}

/// org-babel-read-table.
pub fn read_table<S: AsRef<str>>(lines: &[S]) -> Sexp {
    let lines: Vec<&str> = lines
        .iter()
        .map(AsRef::as_ref)
        .filter(|l| doc::is_table_line(l))
        .collect();
    list(
        value::table_to_lisp(&lines)
            .into_iter()
            .map(|r| match r {
                None => hline(),
                Some(c) => list(
                    c.iter()
                        .map(|x| value::read(x, true).unwrap_or_else(|_| str(x.clone())))
                        .collect(),
                ),
            })
            .collect(),
    )
}

/// org-babel-read-list: the top-level items' text.
pub fn read_list(lines: &[String]) -> Sexp {
    let ind = lines.first().map_or(0, |l| value::indentation(l));
    let mut items_out: Vec<String> = vec![];
    let mut sub = false;
    for l in lines {
        if l.trim().is_empty() {
            continue;
        }
        let li = value::indentation(l);
        if li == ind
            && let Some((i, b)) = crate::org::ctx::item_bullet(l)
        {
            let mut text = l[i + b.len()..].to_owned();
            if let Some(rest) = text
                .strip_prefix("[ ] ")
                .or_else(|| text.strip_prefix("[X] "))
                .or_else(|| text.strip_prefix("[-] "))
            {
                text = rest.to_owned();
            }
            items_out.push(text);
            sub = false;
        } else if li > ind && !sub && crate::org::ctx::item_bullet(l).is_none() {
            if let Some(last) = items_out.last_mut() {
                last.push('\n');
                last.push_str(l.trim());
            }
        } else {
            sub = true;
        }
    }
    list(
        items_out
            .iter()
            .map(|t| value::read(t, true).unwrap_or_else(|_| str(t.clone())))
            .collect(),
    )
}

/// org-babel-read-link.
fn read_link(doc: &Doc, inner: &str) -> String {
    let raw = inner.split("][").next().unwrap_or(inner);
    let expand = |p: &str| {
        let p = options::expand(p);
        if p.is_absolute() {
            p.display().to_string()
        } else {
            doc.dir().join(p).display().to_string()
        }
    };
    match raw.split_once(':') {
        Some(("file", rest)) => expand(rest.split("::").next().unwrap_or(rest)),
        Some((t, _))
            if t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !t.is_empty() =>
        {
            raw.to_owned()
        }
        _ => expand(raw),
    }
}

// ---- noweb ----

pub fn noweb_start() -> String {
    options::string("org-babel-noweb-wrap-start", "<<")
}

pub fn noweb_end() -> String {
    options::string("org-babel-noweb-wrap-end", ">>")
}

/// Noweb references on a line: (prefix end, ref start, ref end, id).
fn noweb_refs(line: &str) -> Vec<(usize, usize, String)> {
    let (s, e) = (noweb_start(), noweb_end());
    let mut out = vec![];
    let mut from = 0;
    while let Some(i) = line[from..].find(&s) {
        let a = from + i;
        let inner = a + s.len();
        // [^ \t\n]\(?:.*?[^ \t\n]\)?
        let Some(j) = line[inner..].find(&e) else {
            break;
        };
        let mut close = inner + j;
        // The lazy match ends at the first end delimiter after a non-blank.
        let mut id = &line[inner..close];
        while id.ends_with([' ', '\t']) || id.is_empty() {
            match line[close + 1..].find(&e) {
                Some(k) => {
                    close = close + 1 + k;
                    id = &line[inner..close];
                }
                None => break,
            }
        }
        if id.is_empty() || id.starts_with([' ', '\t']) || id.ends_with([' ', '\t']) {
            from = a + 1;
            continue;
        }
        out.push((a, close + e.len(), id.to_owned()));
        from = close + e.len();
    }
    out
}

/// Remove noweb references (strip-export / strip-tangle).
pub fn strip_noweb(body: &str) -> String {
    body.split('\n')
        .map(|l| {
            let mut out = l.to_owned();
            for (a, b, _) in noweb_refs(l).into_iter().rev() {
                out.replace_range(a..b, "");
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// org-babel-expand-noweb-references.
pub fn expand_noweb(doc: &mut Doc, cx: &mut Cx, info: &Info, context: &str) -> R<String> {
    let comment = get_str(&info.params, ":comments").as_deref() == Some("noweb");
    let prefix_on = match get(&info.params, ":noweb-prefix") {
        None => true,
        Some(v) => !v.is_nil() && as_string(v).as_deref() != Some("no"),
    };
    if cx.noweb.as_ref().is_none_or(|(t, _, _)| *t != doc.tick) {
        cx.noweb = Some((doc.tick, HashMap::new(), false));
    }
    let body = info.body.clone();
    let mut out_lines = vec![];
    for line in body.split('\n') {
        let refs = noweb_refs(line);
        if refs.is_empty() {
            out_lines.push(line.to_owned());
            continue;
        }
        let mut out = String::new();
        let mut last = 0;
        for (a, b, id) in refs {
            let prefix = &line[last..a];
            let expansion = noweb_expansion(doc, cx, info, &id, context)?;
            let mut interposed = if prefix_on {
                expansion
                    .split(['\n', '\r'])
                    .collect::<Vec<_>>()
                    .join(&format!("\n{prefix}"))
            } else {
                expansion
            };
            if comment {
                interposed.push('\n');
            }
            out.push_str(prefix);
            out.push_str(&interposed);
            last = b;
        }
        out.push_str(&line[last..]);
        out_lines.push(out);
    }
    let result = out_lines.join("\n");
    if comment && body.ends_with(&noweb_end()) {
        return Ok(result.strip_suffix('\n').unwrap_or(&result).to_owned());
    }
    Ok(result)
}

fn noweb_expansion(doc: &mut Doc, cx: &mut Cx, info: &Info, id: &str, context: &str) -> R<String> {
    if id.contains('(') && id.contains(')') {
        let raw = ref_resolve(doc, cx, id)?;
        cx.noweb = Some((doc.tick, HashMap::new(), false));
        return Ok(echo(&raw));
    }
    let cached = cx.noweb.as_ref().and_then(|(_, m, _)| m.get(id).cloned());
    if let Some(blocks) = cached {
        return expand_references(doc, cx, info, &blocks, context);
    }
    if let Some(h) = headline_by_id(doc, id) {
        return Ok(headline_body(doc, h));
    }
    // A block named ID.
    for el in doc::all_elements(&doc.lines) {
        if el.kind == Kind::Src
            && el.name.as_deref() == Some(id)
            && !in_commented_heading(doc, el.post)
        {
            let i = src_info(doc, cx, &el, true)?;
            if let Some((_, m, _)) = &mut cx.noweb {
                m.entry(id.to_owned()).or_default().push(i.clone());
            }
            return expand_block(doc, cx, &i, context);
        }
    }
    if let Some(lib) = library_get(id) {
        return Ok(lib.body);
    }
    let processed = cx.noweb.as_ref().is_some_and(|(_, _, p)| *p);
    if !processed {
        let mut map: HashMap<String, Vec<Info>> = HashMap::new();
        for el in doc::all_elements(&doc.lines) {
            if el.kind != Kind::Src || in_commented_heading(doc, el.post) {
                continue;
            }
            let i = src_info(doc, cx, &el, true)?;
            if let Some(r) = get_str(&i.params, ":noweb-ref") {
                map.entry(r).or_default().push(i);
            }
        }
        cx.noweb = Some((doc.tick, map, true));
    }
    let blocks = cx
        .noweb
        .as_ref()
        .and_then(|(_, m, _)| m.get(id).cloned())
        .unwrap_or_default();
    if blocks.is_empty() {
        let langs = crate::org::sexp::option("org-babel-noweb-error-langs")
            .map(|v| {
                items(&v)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(as_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if options::bool("org-babel-noweb-error-all-langs", false) || langs.contains(&info.lang) {
            return err(format!(
                "Cannot resolve {}{id}{} (see `org-babel-noweb-error-langs')",
                noweb_start(),
                noweb_end()
            ));
        }
        return Ok(String::new());
    }
    expand_references(doc, cx, info, &blocks, context)
}

fn expand_references(
    doc: &mut Doc,
    cx: &mut Cx,
    _info: &Info,
    blocks: &[Info],
    context: &str,
) -> R<String> {
    let mut out = String::new();
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str(
                &get_str(&blocks[i - 1].params, ":noweb-sep").unwrap_or_else(|| "\n".into()),
            );
        }
        out.push_str(&expand_block(doc, cx, b, context)?);
    }
    Ok(out)
}

fn expand_block(doc: &mut Doc, cx: &mut Cx, i: &Info, context: &str) -> R<String> {
    let b = if params::noweb_p(&i.params, context) {
        expand_noweb(doc, cx, i, context)?
    } else {
        i.body.clone()
    };
    if get_str(&i.params, ":comments").as_deref() != Some("noweb") {
        return Ok(b);
    }
    let (beg, end) = super::tangle::comment_links(doc, i);
    let wrap = |s: &str| {
        if options::bool("org-babel-tangle-uncomment-comments", false) {
            s.to_owned()
        } else {
            super::lang::comment(&i.lang, s)
        }
    };
    Ok(format!("{}\n{b}\n{}", wrap(&beg), wrap(&end)))
}

/// org-babel--expand-body: noweb (:eval) and coderefs removed.
pub fn expand_body(doc: &mut Doc, cx: &mut Cx, info: &Info) -> R<String> {
    let expand = if params::noweb_p(&info.params, ":eval") {
        expand_noweb(doc, cx, info, ":eval")?
    } else {
        info.body.clone()
    };
    Ok(match &info.coderef {
        Some(f) => strip_coderefs(&expand, f),
        None => expand,
    })
}

/// org-babel-expand-body:generic.
pub fn expand_generic(body: &str, p: &[(String, Sexp)], var_lines: &[String]) -> String {
    let mut parts = vec![];
    if let Some(pro) = get_str(p, ":prologue") {
        parts.push(pro);
    }
    parts.extend(var_lines.iter().cloned());
    parts.push(body.to_owned());
    if let Some(epi) = get_str(p, ":epilogue") {
        parts.push(epi);
    }
    parts.join("\n")
}

/// The expanded body as org-babel-expand-src-block shows it.
pub fn expand_full(env: &mut Env, lang: &str, body: &str, p: &Params) -> R<String> {
    if let Some(e) = super::lang::expand(env, lang, body, p)? {
        return Ok(e);
    }
    let vars = super::lang::assignments(env, lang, p)?.unwrap_or_default();
    Ok(expand_generic(body, p, &vars))
}

// ---- hashes ----

/// org-babel-sha1-hash.
pub fn sha1_hash(doc: &mut Doc, cx: &mut Cx, info: &Info, context: &str) -> R<String> {
    let mut p = info.params.clone();
    p.sort_by(|a, b| a.0.cmp(&b.0));
    const RM: [&str; 6] = ["replace", "silent", "none", "discard", "append", "prepend"];
    let norm = |k: &str, v: &Sexp| -> Option<String> {
        if v.is_nil()
            || matches!(v, Sexp::Str(s) if s.is_empty())
            || matches!(v, Sexp::Vector(x) if x.is_empty())
        {
            return None;
        }
        if k == ":result-params" && value::listp(v) {
            let mut x: Vec<String> = items(v)
                .unwrap_or(&[])
                .iter()
                .filter_map(as_string)
                .filter(|s| !RM.contains(&s.as_str()))
                .collect();
            x.sort();
            return (!x.is_empty()).then(|| prin1(&list(x.into_iter().map(Sexp::Str).collect())));
        }
        if (k == ":results" || k == ":exports")
            && let Sexp::Str(s) = v
        {
            let mut x: Vec<&str> = s.split_whitespace().filter(|w| !RM.contains(w)).collect();
            x.sort();
            return Some(prin1(&Sexp::Str(x.join(" "))));
        }
        Some(prin1(v))
    };
    let parts: Vec<String> = p.iter().filter_map(|(k, v)| norm(k, v)).collect();
    let body = if params::noweb_p(&info.params, context) {
        expand_noweb(doc, cx, info, context)?
    } else {
        info.body.clone()
    };
    let at = info.start.map_or(doc.point.0, |s| s.0);
    let dir = doc.dir();
    let expanded = expand_full(
        &mut Env { doc, cx, at, dir },
        &info.lang,
        &body,
        &info.params,
    )?;
    Ok(super::sha1::hex(
        format!("{}-{expanded}", parts.join(":")).as_bytes(),
    ))
}

/// org-babel-current-result-hash for the block at POS.
pub fn current_result_hash(doc: &mut Doc, cx: &mut Cx, pos: (usize, usize)) -> R<Option<String>> {
    Ok(
        where_is_result(doc, cx, pos, false, None)?.and_then(|l| match l {
            Loc::Line(l) => doc::result_hash(doc.line(l)),
            Loc::Inline(..) => None,
        }),
    )
}

// ---- execution ----

/// What runs a block (for messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecKind {
    Src,
    Inline,
    Call,
    InlineCall,
}

/// The environment a language executes in.
pub struct Env<'a> {
    pub doc: &'a mut Doc,
    pub cx: &'a mut Cx,
    /// The block's line.
    pub at: usize,
    /// default-directory.
    pub dir: PathBuf,
}

impl Env<'_> {
    /// org-babel-ref-resolve from the block.
    pub fn resolve(&mut self, r: &str) -> R<Sexp> {
        ref_resolve(self.doc, self.cx, r)
    }

    /// org-entry-get with inheritance at the block.
    pub fn entry(&mut self, prop: &str) -> Option<String> {
        entry_get(self.doc, self.cx, self.at, prop, true)
    }

    /// org-babel-eval: run COMMAND in a shell with INPUT on stdin.
    pub fn eval(&mut self, command: &str, input: &str) -> R<String> {
        let (out, errs, code) = run_shell(command, &format!("{input}\n"), &self.dir)?;
        if code != Some(0) || !errs.is_empty() {
            self.error_notify(code, &errs);
        }
        Ok(out)
    }

    /// org-babel-eval-error-notify.
    pub fn error_notify(&mut self, code: Option<i32>, stderr: &str) {
        let cx = &mut *self.cx;
        if !cx.error_output.is_empty() && !cx.error_output.ends_with('\n') {
            cx.error_output.push('\n');
        }
        cx.error_output.push_str(stderr);
        cx.error_output.push_str(&match code {
            Some(c) => format!("[ Babel evaluation exited with code {c} ]"),
            None => "[ Babel evaluation exited abnormally ]".into(),
        });
        let first = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| format!(": {}", l.trim()))
            .unwrap_or_default();
        cx.message(match code {
            Some(c) => format!("Babel evaluation exited with code {c}{first}"),
            None => format!("Babel evaluation exited abnormally{first}"),
        });
    }

    pub fn temp_file(&self, prefix: &str, suffix: &str) -> PathBuf {
        temp_file(prefix, suffix)
    }
}

/// shell-file-name.
pub fn shell_file_name() -> String {
    options::get("shell-file-name")
        .and_then(|v| v.as_str().map(str::to_owned))
        .or_else(|| std::env::var("SHELL").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into())
}

/// Run COMMAND with `sh -c` in DIR: (stdout, stderr, exit code).
pub fn run_shell(command: &str, input: &str, dir: &Path) -> R<(String, String, Option<i32>)> {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(if dir.is_dir() {
            dir.to_path_buf()
        } else {
            std::env::temp_dir()
        })
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{command}: {e}"))?;
    let mut stdin = child.stdin.take();
    let data = input.to_owned();
    let writer = std::thread::spawn(move || {
        if let Some(s) = &mut stdin {
            let _ = s.write_all(data.as_bytes());
        }
    });
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = writer.join();
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    ))
}

/// org-babel-temporary-directory.
pub fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("fred-babel-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d
}

/// org-babel-temp-file.
pub fn temp_file(prefix: &str, suffix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let p = temp_dir().join(format!("{prefix}{n}{nanos:x}{suffix}"));
    let _ = std::fs::write(&p, "");
    p
}

/// shell-quote-argument.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=+,:@%".contains(c))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The key a confirmation answer is remembered by.
fn confirm_key(info: &Info, body: &str) -> String {
    super::sha1::hex(
        format!(
            "{}\0{}\0{:?}\0{body}",
            info.lang,
            info.name.clone().unwrap_or_default(),
            info.start
        )
        .as_bytes(),
    )
}

/// org-babel-check-confirm-evaluate: Some(query?) or None (disabled).
fn check_confirm(cx: &Cx, info: &Info) -> Option<bool> {
    let eval = get_str(&info.params, ":eval")
        .or_else(|| has(&info.params, ":noeval").then(|| "no".into()));
    let e = eval.as_deref().unwrap_or("");
    if matches!(e, "no" | "never") || (cx.export && matches!(e, "no-export" | "never-export")) {
        return None;
    }
    let confirm = match crate::org::sexp::option("org-confirm-babel-evaluate") {
        Some(v) => v.truthy(),
        None => true,
    };
    Some(e == "query" || (cx.export && e == "query-export") || confirm)
}

/// org-babel-check-evaluate.
fn check_evaluate(cx: &mut Cx, info: &Info) -> bool {
    let ok = check_confirm(cx, info).is_some();
    if !ok {
        cx.message(format!(
            "Evaluation of this {} code block{}is disabled.",
            info.lang,
            name_string(info)
        ));
    }
    ok
}

fn name_string(info: &Info) -> String {
    match &info.name {
        Some(n) => format!(" ({n}) "),
        None => " ".into(),
    }
}

/// org-babel-confirm-evaluate: Ok(true) to evaluate.
fn confirm_evaluate(doc: &mut Doc, cx: &mut Cx, info: &Info) -> R<bool> {
    match check_confirm(cx, info) {
        None => Ok(false),
        Some(false) => Ok(true),
        Some(true) => {
            let body = expand_body(doc, cx, info)?;
            let key = confirm_key(info, &body);
            if cx.approved.contains(&key) {
                return Ok(true);
            }
            if cx.denied.contains(&key) {
                cx.message(format!(
                    "Evaluation of this {} code block{}is aborted.",
                    info.lang,
                    name_string(info)
                ));
                return Ok(false);
            }
            Err(Fail::Confirm {
                question: format!(
                    "Evaluate this {} code block{}on your system? ",
                    info.lang,
                    name_string(info)
                ),
                key,
            })
        }
    }
}

/// org-babel-execute-src-block. INFO None: the block at point.
pub fn execute_src_block(
    doc: &mut Doc,
    cx: &mut Cx,
    force: bool,
    info: Option<Info>,
    extra: &[(String, Sexp)],
    kind: ExecKind,
) -> R<Option<Sexp>> {
    let info = match info {
        Some(i) => i,
        None => match get_info(doc, cx, doc.point, false)? {
            Some(i) => i,
            None => return Ok(None),
        },
    };
    let start_mark = info.start.map(|s| doc.mark(s));
    let outer = cx.location.is_none();
    if outer {
        cx.location = start_mark;
    }
    let out = execute_inner(doc, cx, force, info, extra, kind, start_mark);
    if outer {
        cx.location = None;
    }
    if let Some(m) = start_mark {
        doc.unmark(m);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn execute_inner(
    doc: &mut Doc,
    cx: &mut Cx,
    force: bool,
    mut info: Info,
    extra: &[(String, Sexp)],
    kind: ExecKind,
    start: Option<usize>,
) -> R<Option<Sexp>> {
    info.params = params::merge(&[&info.params, extra])?;
    if !check_evaluate(cx, &info) {
        return Ok(None);
    }
    info.params = process_params(doc, cx, std::mem::take(&mut info.params))?;
    let p = info.params.clone();
    let pos = |doc: &Doc| start.map(|m| doc.marker(m));
    let cache = !force && get_str(&p, ":cache").as_deref() == Some("yes");
    let new_hash = if cache {
        Some(sha1_hash(doc, cx, &info, ":eval")?)
    } else {
        None
    };
    if let (Some(h), Some(at)) = (&new_hash, pos(doc))
        && current_result_hash(doc, cx, at)?.as_deref() == Some(h.as_str())
        && let Some(Loc::Line(l)) = where_is_result(doc, cx, at, false, None)?
    {
        let r = read_result(doc, l + 1)?;
        cx.message(format!(
            "Cached: {}",
            prin1(&r.clone().unwrap_or(Sexp::Nil))
        ));
        return Ok(r);
    }
    if !confirm_evaluate(doc, cx, &info)? {
        return Ok(None);
    }
    let lang = info.lang.clone();
    let mut rp = params::result_params(&p);
    let body = expand_body(doc, cx, &info)?;
    let dir = match get_str(&p, ":dir") {
        None => doc.dir(),
        Some(d) => {
            let d = options::expand(&d);
            let d = if d.is_absolute() {
                d
            } else {
                doc.dir().join(d)
            };
            let mkdirp = get_str(&p, ":mkdirp");
            if !matches!(mkdirp.as_deref(), None | Some("no") | Some("nil")) {
                std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            d
        }
    };
    if !super::lang::executable(&lang) {
        return err(format!("No org-babel-execute function for {lang}!"));
    }
    let what = match kind {
        ExecKind::Src => "code block",
        ExecKind::Inline => "inline code block",
        ExecKind::Call => "call",
        ExecKind::InlineCall => "inline call",
    };
    let at_line = pos(doc).map_or(doc.point.0, |p| p.0);
    cx.message(format!(
        "Executing {} {what} {}...",
        capitalize(&lang),
        match &info.name {
            Some(n) => format!("({n})"),
            None => format!("at position {}", at_line + 1),
        }
    ));
    let t0 = std::time::Instant::now();
    let mut result = {
        let mut env = Env {
            doc,
            cx,
            at: at_line,
            dir: dir.clone(),
        };
        super::lang::execute(&mut env, &lang, &body, &p)?
    };
    let doc_dir = doc.dir();
    if get(&p, ":result-type").and_then(Sexp::sym) == Some("value")
        && rp.iter().any(|x| x == "vector" || x == "table")
        && result.as_ref().is_some_and(|r| !value::listp(r))
    {
        result = Some(list(vec![list(vec![result.unwrap()])]));
    }
    let file = rp
        .iter()
        .any(|x| x == "file")
        .then(|| get_str(&p, ":file"))
        .flatten();
    if let Some(f) = &file {
        if let Some(r) = &result
            && !r.is_nil()
            && !rp.iter().any(|x| x == "link" || x == "graphics")
        {
            let path = dir.join(f);
            std::fs::write(&path, format_result(r, get_str(&p, ":sep").as_deref()))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if let Some(mode) = get(&p, ":file-mode").and_then(Sexp::int) {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode as u32));
            }
        }
        result = Some(str(f.clone()));
    }
    if let Some(post) = get_str(&p, ":post") {
        let this = if file.is_some() {
            str(result_to_file(
                &doc_dir,
                &dir,
                &value::princ(result.as_ref().unwrap_or(&Sexp::Nil)),
                file_desc(&p, result.as_ref()),
            ))
        } else {
            result.clone().unwrap_or(Sexp::Nil)
        };
        let saved = value::set_this(this);
        let r = ref_resolve(doc, cx, &post);
        value::set_this(saved);
        result = Some(r?).filter(|v| !v.is_nil());
        if file.is_some() {
            rp.retain(|x| x != "file");
        }
    }
    let update = options::string("org-babel-update-intermediate", "nil");
    if !rp.iter().any(|x| x == "none") || (cache && update == "cached") {
        if let Some(at) = pos(doc) {
            let hash = if rp.iter().any(|x| x == "replace") {
                new_hash.clone()
            } else {
                None
            };
            let elapsed = t0.elapsed().as_secs_f64();
            insert_result(
                doc,
                cx,
                result.clone(),
                &rp,
                &info,
                &dir,
                at,
                hash,
                Some(&lang),
                elapsed,
            )?;
        } else if rp.iter().any(|x| x == "silent") {
            cx.message(prin1(result.as_ref().unwrap_or(&Sexp::Nil)));
        }
    }
    Ok(result)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// org-babel-format-result.
pub fn format_result(r: &Sexp, sep: Option<&str>) -> String {
    if value::listp(r) {
        value::orgtbl_to_generic(r, sep.unwrap_or("\t"), &echo, None)
    } else {
        echo(r)
    }
}

/// org-babel--file-desc.
pub fn file_desc(p: &[(String, Sexp)], result: Option<&Sexp>) -> Option<String> {
    match get(p, ":file-desc") {
        None => None,
        Some(Sexp::Nil) => result.map(value::princ),
        Some(v) => as_string(v),
    }
}

/// org-babel-result-to-file: a file link, relative to the document when
/// the evaluation directory differs.
pub fn result_to_file(doc_dir: &Path, dir: &Path, result: &str, desc: Option<String>) -> String {
    let path = if dir != doc_dir {
        let abs = dir.join(result);
        match options::string("org-link-file-path-type", "adaptive").as_str() {
            "adaptive" | "relative" => relative(&abs, doc_dir),
            _ => abs.display().to_string(),
        }
    } else {
        result.to_owned()
    };
    format!(
        "[[file:{path}]{}]",
        desc.map(|d| format!("[{d}]")).unwrap_or_default()
    )
}

/// file-relative-name.
pub fn relative(path: &Path, base: &Path) -> String {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    out.display().to_string()
}

// ---- results location ----

/// Where results are: a `#+RESULTS` line, or a byte position in a line
/// (inline results).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loc {
    Line(usize),
    Inline(usize, usize),
}

/// org-babel-where-is-src-block-head at line `l`.
pub fn where_is_head(doc: &Doc, l: usize) -> Option<usize> {
    let el = doc::element_at(&doc.lines, l)?;
    (el.kind == Kind::Src && l < el.stop).then_some(el.post)
}

/// org-babel-find-named-result.
pub fn find_named_result(doc: &Doc, name: &str) -> Option<usize> {
    let kw = options::string("org-babel-results-keyword", "RESULTS");
    for (i, l) in doc.lines.iter().enumerate() {
        let t = l.trim_start();
        if t.len() < 2 + kw.len()
            || !t.starts_with("#+")
            || !t[2..2 + kw.len()].eq_ignore_ascii_case(&kw)
        {
            continue;
        }
        let Some(c) = t.find(':') else { continue };
        if t[c + 1..].trim() != name {
            continue;
        }
        if let Some(el) = doc::element_at(&doc.lines, i)
            && (el.kind == Kind::Keyword || i < el.post)
        {
            return Some(i);
        }
    }
    None
}

/// org-babel--insert-results-keyword at line `at`: the keyword line.
fn insert_results_keyword(
    doc: &mut Doc,
    at: usize,
    indent: &str,
    name: Option<&str>,
    hash: Option<&str>,
) -> usize {
    let kw = options::string("org-babel-results-keyword", "RESULTS");
    let hash = match hash {
        None => String::new(),
        Some(h) if options::bool("org-babel-hash-show-time", false) => {
            format!("[({}) {h}]", crate::org::babel::lang::now_stamp())
        }
        Some(h) => format!("[{h}]"),
    };
    let line = format!(
        "{indent}#+{kw}{hash}:{}",
        name.map(|n| format!(" {n}")).unwrap_or_default()
    );
    let mut ins = vec![line];
    if at < doc.len() && !doc.line(at).trim().is_empty() {
        ins.push(String::new());
    }
    doc.insert(at, ins);
    at
}

/// org-babel--clear-results-maybe at the keyword line `l`: true when
/// cleared (the keyword is then gone and `l` is where it goes).
fn clear_results_maybe(doc: &mut Doc, l: usize, hash: Option<&str>) -> bool {
    let Some(hash) = hash else { return false };
    if doc::result_hash(doc.line(l)).as_deref() == Some(hash) {
        return false;
    }
    let Some(e) = doc::element_at(&doc.lines, l) else {
        return false;
    };
    let post = e.post;
    doc.splice(post, e.stop - post, vec![]);
    if l != post {
        doc.splice(l, 1, vec![]);
    }
    true
}

fn indent_of(s: &str) -> String {
    s[..s.len() - s.trim_start_matches([' ', '\t']).len()].to_owned()
}

/// org-babel-where-is-src-block-result.
pub fn where_is_result(
    doc: &mut Doc,
    _cx: &mut Cx,
    pos: (usize, usize),
    insert: bool,
    hash: Option<&str>,
) -> R<Option<Loc>> {
    match context(doc, pos) {
        Some(Context::InlineSrc(o, el) | Context::InlineCall(o, el)) => {
            // Results follow the object, maybe on the next paragraph line.
            let mut l = o.line;
            let mut from = o.end;
            loop {
                if let Some((a, b)) = doc::results_macro(doc.line(l), from) {
                    if insert {
                        let line = doc.line(l).to_owned();
                        let a = if l == o.line { o.end } else { a };
                        let mut s = line.clone();
                        s.replace_range(a..b, "");
                        doc.set(l, s);
                        return Ok(Some(Loc::Inline(l, a)));
                    }
                    return Ok(Some(Loc::Inline(l, a)));
                }
                if !doc.line(l)[from..].trim().is_empty() || l + 1 >= el.stop {
                    return Ok(None);
                }
                l += 1;
                from = 0;
            }
        }
        Some(Context::Src(el) | Context::Call(el)) => {
            let indent = indent_of(doc.line(el.post));
            if let Some(name) = &el.name {
                if let Some(r) = find_named_result(doc, name) {
                    if clear_results_maybe(doc, r, hash) {
                        insert_results_keyword(doc, r, &indent, Some(name), hash);
                    }
                    return Ok(Some(Loc::Line(r)));
                }
            } else {
                let (_, sec_end) = doc::section(&doc.lines, el.post);
                let at_end = el.end >= sec_end || el.end >= doc.len();
                if !at_end && let Some(next) = doc::element_at(&doc.lines, el.end) {
                    for j in next.begin..=next.post.min(doc.len() - 1) {
                        let t = doc.line(j);
                        if doc::is_result_line(t) && doc::result_value(t).is_empty() {
                            if clear_results_maybe(doc, j, hash) {
                                insert_results_keyword(doc, j, &indent, None, hash);
                            }
                            return Ok(Some(Loc::Line(j)));
                        }
                    }
                }
            }
            if !insert {
                return Ok(None);
            }
            // A new RESULTS keyword after a blank line.
            let at = el.stop;
            doc.insert(at, vec![String::new()]);
            let kl = insert_results_keyword(doc, at + 1, &indent, el.name.as_deref(), hash);
            Ok(Some(Loc::Line(kl)))
        }
        _ => Ok(None),
    }
}

/// org-babel-result-end from line `l` (the line after the results).
pub fn result_end(doc: &Doc, l: usize) -> usize {
    if l >= doc.len() || doc.line(l).trim().is_empty() {
        return l;
    }
    let t = doc.line(l).trim();
    if t.starts_with("[[") && t.ends_with("]]") && t.matches("[[").count() == 1 {
        return l + 1;
    }
    match doc::element_at(&doc.lines, l) {
        Some(e)
            if matches!(
                e.kind,
                Kind::Drawer
                    | Kind::Example
                    | Kind::Export
                    | Kind::Fixed
                    | Kind::Src
                    | Kind::List
                    | Kind::Table
                    | Kind::Latex
            ) || matches!(&e.kind, Kind::Block(n) if !matches!(n.as_str(), "center" | "quote" | "verse" | "comment")) =>
        {
            e.stop
        }
        _ => l,
    }
}

/// org-babel-read-result at line `l` (the line after the keyword).
pub fn read_result(doc: &Doc, l: usize) -> R<Option<Sexp>> {
    if l >= doc.len() || doc.line(l).trim().is_empty() {
        return Ok(None);
    }
    match doc::element_at(&doc.lines, l) {
        Some(e) => read_element(doc, &e),
        None => Ok(None),
    }
}

/// org-macro-escape-arguments for one argument.
pub fn macro_escape(arg: &str) -> String {
    let trailing = arg.len() - arg.trim_end_matches('\\').len();
    let arg = format!("{arg}{}", "\\".repeat(trailing));
    let mut out = String::new();
    let mut bs = 0;
    for c in arg.chars() {
        if c == '\\' {
            bs += 1;
            continue;
        }
        if c == ',' {
            out.push_str(&"\\".repeat(bs * 2));
            out.push_str("\\,");
        } else {
            out.push_str(&"\\".repeat(bs));
            out.push(c);
        }
        bs = 0;
    }
    out.push_str(&"\\".repeat(bs));
    out
}

/// Whether R can be turned into a table.
fn tabulable(r: &Sexp) -> bool {
    match items(r) {
        Some(x) => x
            .iter()
            .all(|e| !matches!(e, Sexp::Dotted(..)) && (!value::listp(e) || items(e).is_some())),
        None => false,
    }
}

/// org-babel-examplify-region on text that ends in a newline.
fn examplify(text: &str, switches: &str) -> String {
    let size = text.matches('\n').count();
    let min = options::int("org-babel-min-lines-for-block-output", 10) as usize;
    if size == 0 {
        return text.to_owned();
    }
    if size < min {
        return text.lines().map(|l| format!(": {l}\n")).collect();
    }
    let cap = |s: &str| {
        if options::bool("org-babel-uppercase-example-markers", false) {
            s.to_uppercase()
        } else {
            s.to_owned()
        }
    };
    format!(
        "{}{switches}\n{}{}\n",
        cap("#+begin_example"),
        escape(text.strip_suffix('\n').unwrap_or(text)) + "\n",
        cap("#+end_example")
    )
}

/// org-babel-insert-result.
#[allow(clippy::too_many_arguments)]
pub fn insert_result(
    doc: &mut Doc,
    cx: &mut Cx,
    result: Option<Sexp>,
    rp: &[String],
    info: &Info,
    dir: &Path,
    at: (usize, usize),
    hash: Option<String>,
    lang: Option<&str>,
    elapsed: f64,
) -> R<()> {
    let has_rp = |x: &str| rp.iter().any(|r| r == x);
    let mut result = result.filter(|r| !r.is_nil());
    if let Some(Sexp::Str(s)) = &result
        && has_rp("file")
    {
        result = Some(str(result_to_file(
            &doc.dir(),
            dir,
            s,
            file_desc(&info.params, result.as_ref()),
        )));
    } else if let Some(r) = &result
        && !value::listp(r)
        && !matches!(r, Sexp::Str(_))
    {
        result = Some(str(prin1(r)));
    }
    if has_rp("silent") {
        cx.message(prin1(result.as_ref().unwrap_or(&Sexp::Nil)));
        return Ok(());
    }
    let inline = matches!(
        context(doc, at),
        Some(Context::InlineSrc(..) | Context::InlineCall(..))
    );
    if inline {
        let warning = if has_rp("table") {
            Some("`:results table'")
        } else if has_rp("drawer") {
            Some("`:results drawer'")
        } else if result.as_ref().is_some_and(value::listp) {
            Some("list result")
        } else if let Some(Sexp::Str(s)) = &result
            && s.as_bytes()
                .windows(2)
                .any(|w| w[0] == b'\n' && w[1] != b'\n')
        {
            Some("multiline result")
        } else if has_rp("list") {
            Some("`:results list'")
        } else {
            None
        };
        if let Some(w) = warning {
            return err(format!("Inline error: {w} cannot be used"));
        }
    }
    let existing = where_is_result(doc, cx, at, true, hash.as_deref())?;
    let switches = get_str(&info.params, ":results_switches")
        .map(|s| format!(" {s}"))
        .unwrap_or_default();
    let wrap_param = info
        .params
        .iter()
        .find(|(k, _)| k == ":wrap")
        .map(|(_, v)| as_string(v).unwrap_or_else(|| "results".into()));
    let lang = lang.unwrap_or("none");
    if inline {
        let (l, c) = match existing {
            Some(Loc::Inline(l, c)) => (l, c),
            _ => {
                let Some(Context::InlineSrc(o, _) | Context::InlineCall(o, _)) = context(doc, at)
                else {
                    return err("No inline object");
                };
                let mut s = doc.line(o.line).to_owned();
                s.insert(o.end, ' ');
                doc.set(o.line, s);
                (o.line, o.end + 1)
            }
        };
        let text = match &result {
            None => String::new(),
            Some(r) if has_rp("file") => macro_escape(&echo(r)),
            Some(r) if !has_rp("raw") => macro_escape(value::chomp(&echo(r), "\n")),
            Some(r) => echo(r),
        };
        let wrapped = if let Some(full) = &wrap_param {
            let split: Vec<&str> = full.split_whitespace().collect();
            let ty = split.first().copied().unwrap_or("").to_ascii_lowercase();
            match ty.as_str() {
                "nil" | "no" => text,
                "export" => format!(
                    "{{{{{{results(@@{}:{text}@@)}}}}}}",
                    split.get(1).copied().unwrap_or("none")
                ),
                "example" => format!("{{{{{{results(={text}=)}}}}}}"),
                "src" => match split.len() {
                    1 => format!("{{{{{{results(src_none{{{text}}})}}}}}}"),
                    2 => format!("{{{{{{results(src_{}{{{text}}})}}}}}}", split[1]),
                    _ => format!(
                        "{{{{{{results(src_{}[{}]{{{text}}})}}}}}}",
                        split[1],
                        split[2..].join(" ")
                    ),
                },
                _ => format!("{{{{{{results({text})}}}}}}"),
            }
        } else if has_rp("html") {
            format!("{{{{{{results(@@html:{text}@@)}}}}}}")
        } else if has_rp("latex") {
            format!("{{{{{{results(@@latex:{text}@@)}}}}}}")
        } else if has_rp("org") {
            format!("{{{{{{results(src_org{{{text}}})}}}}}}")
        } else if has_rp("code") {
            format!("{{{{{{results(src_{lang}[{switches}]{{{text}}})}}}}}}")
        } else if has_rp("raw") {
            text
        } else if has_rp("drawer") || has_rp("file") {
            format!("{{{{{{results({text})}}}}}}")
        } else if result.is_some() {
            let w = options::string("org-babel-inline-result-wrap", "=%s=");
            format!("{{{{{{results({})}}}}}}", w.replace("%s", &text))
        } else {
            text
        };
        let mut s = doc.line(l).to_owned();
        s.insert_str(c, &wrapped);
        let mut parts: Vec<String> = s.split('\n').map(str::to_owned).collect();
        let first = parts.remove(0);
        doc.set(l, first);
        if !parts.is_empty() {
            doc.insert(l + 1, parts);
        }
        report(cx, &result, rp, elapsed);
        return Ok(());
    }
    let Some(Loc::Line(kl)) = existing else {
        return err("Cannot insert results here");
    };
    let indent = value::indentation(doc.line(kl));
    let mut beg = kl + 1;
    if has_rp("replace") {
        let e = result_end(doc, beg);
        doc.splice(beg, e - beg, vec![]);
    } else if has_rp("append") {
        beg = result_end(doc, beg);
    }
    let mut text = String::new();
    if let Some(r) = &result {
        if let Sexp::Str(s) = r {
            text = s.clone();
            if !s.trim().is_empty() && !s.ends_with('\n') {
                text.push('\n');
            }
        }
        if has_rp("list") {
            let elems: Vec<Sexp> = match items(r) {
                Some(x) => x.to_vec(),
                None => echo(r)
                    .split('\n')
                    .filter(|s| !s.is_empty())
                    .map(|s| str(s))
                    .collect(),
            };
            let lines: Vec<String> = elems
                .iter()
                .map(|e| match items(e) {
                    Some(x) if !x.is_empty() => {
                        format!("- {}", x.iter().map(echo).collect::<Vec<_>>().join("\n  "))
                    }
                    _ => format!("- {}", echo(e)),
                })
                .collect();
            text = value::trim(&lines.join("\n")).to_owned() + "\n";
        } else if tabulable(r) {
            let all_rows = items(r)
                .unwrap_or(&[])
                .iter()
                .all(|e| is_hline(e) || value::listp(e));
            let t = if all_rows {
                r.clone()
            } else {
                list(vec![r.clone()])
            };
            text = value::orgtbl(&t).join("\n") + "\n";
        } else if value::listp(r) {
            text = format!("{}\n", value::princ(r));
        }
    }
    let wrap = |text: &str, start: &str, finish: &str, no_escape: bool| -> String {
        let body = if no_escape {
            text.to_owned()
        } else {
            escape_text(text)
        };
        format!("{start}\n{body}{finish}\n")
    };
    let is_table = result.as_ref().is_some_and(tabulable);
    if let Some(full) = &wrap_param {
        let split: Vec<&str> = full.split_whitespace().collect();
        let ty = split.first().copied().unwrap_or("results");
        let open = format!("#+begin_{full}");
        let close = format!("#+end_{ty}");
        let tl = ty.to_ascii_lowercase();
        if tl != "nil" && tl != "no" {
            let no_escape = !matches!(tl.as_str(), "export" | "example" | "src");
            text = wrap(&text, &open, &close, no_escape);
        }
    } else if has_rp("html") {
        text = wrap(&text, "#+begin_export html", "#+end_export", false);
    } else if has_rp("latex") {
        text = wrap(&text, "#+begin_export latex", "#+end_export", false);
    } else if has_rp("org") {
        text = wrap(&text, "#+begin_src org", "#+end_src", false);
    } else if has_rp("code") {
        text = wrap(
            &text,
            &format!("#+begin_src {lang}{switches}"),
            "#+end_src",
            false,
        );
    } else if has_rp("raw") {
    } else if has_rp("drawer") || has_rp("wrap") {
        text = wrap(&text, ":results:", ":end:", true);
    } else if !is_table && !has_rp("file") {
        text = examplify(&text, &switches);
    }
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    if text.ends_with('\n') || text.is_empty() {
        lines.pop();
    }
    if indent > 0 && !(result.as_ref().is_some_and(value::listp) && has_rp("append")) {
        let pad = " ".repeat(indent);
        for l in &mut lines {
            if !l.is_empty() {
                l.insert_str(0, &pad);
            }
        }
    }
    doc.insert(beg, lines);
    report(cx, &result, rp, elapsed);
    Ok(())
}

fn escape_text(text: &str) -> String {
    match text.strip_suffix('\n') {
        Some(t) => escape(t) + "\n",
        None => escape(text),
    }
}

fn report(cx: &mut Cx, result: &Option<Sexp>, rp: &[String], elapsed: f64) {
    let t = if elapsed > 0.05 {
        format!(" (took {elapsed:.1}s)")
    } else {
        String::new()
    };
    cx.message(match result {
        None if rp.iter().any(|r| r == "value") => format!("Code block returned no value{t}."),
        None => format!("Code block produced no output{t}."),
        Some(_) => format!("Code block evaluation complete{t}."),
    });
}

/// org-babel-remove-result at POS.
pub fn remove_result(doc: &mut Doc, cx: &mut Cx, pos: (usize, usize), keep_keyword: bool) -> R<()> {
    if let Some(Loc::Line(l)) = where_is_result(doc, cx, pos, false, None)?
        && doc::is_result_line(doc.line(l))
    {
        let start = if keep_keyword {
            l + 1
        } else {
            (0..l)
                .rev()
                .find(|&i| !doc.line(i).trim().is_empty())
                .map_or(0, |i| i + 1)
        };
        let end = result_end(doc, l + 1);
        doc.splice(start, end - start, vec![]);
    }
    Ok(())
}

/// org-babel-remove-inline-result for the object at POS.
pub fn remove_inline_result(doc: &mut Doc, pos: (usize, usize)) -> bool {
    let Some(Context::InlineSrc(o, el) | Context::InlineCall(o, el)) = context(doc, pos) else {
        return false;
    };
    let mut l = o.line;
    let mut from = o.end;
    loop {
        if let Some((_, b)) = doc::results_macro(doc.line(l), from) {
            if l == o.line {
                let mut s = doc.line(l).to_owned();
                s.replace_range(o.end..b, "");
                doc.set(l, s);
            } else {
                // Leading white space up to the object is removed too.
                let tail = doc.line(l)[b..].to_owned();
                let mut first = doc.line(o.line)[..o.end].to_owned();
                first.push_str(&tail);
                doc.splice(o.line, l - o.line + 1, vec![first]);
            }
            return true;
        }
        if !doc.line(l)[from..].trim().is_empty() || l + 1 >= el.stop {
            return false;
        }
        l += 1;
        from = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_line_parts() {
        assert_eq!(
            parse_begin("#+begin_src emacs-lisp -n -r -l \"(r:%s)\" :var x=1"),
            (
                "emacs-lisp".into(),
                "-n -r -l \"(r:%s)\"".into(),
                ":var x=1".into()
            )
        );
        assert_eq!(
            parse_begin("  #+BEGIN_SRC sh :results output"),
            ("sh".into(), String::new(), ":results output".into())
        );
        assert_eq!(
            parse_begin("#+begin_src"),
            (String::new(), String::new(), String::new())
        );
        assert_eq!(
            strip_coderefs("a (ref:x)\nb  (ref:y z)", "(ref:%s)"),
            "a\nb"
        );
    }

    #[test]
    fn index_lists() {
        let t = crate::org::sexp::read("((1 2 3) (4 5 6) (7 8 9))").unwrap();
        assert_eq!(prin1(&index_list("1", &t)), "(4 5 6)");
        assert_eq!(prin1(&index_list("0:1,1", &t)), "(2 5)");
        assert_eq!(prin1(&index_list("-1,-1", &t)), "9");
        assert_eq!(prin1(&index_list("*,0", &t)), "(1 4 7)");
        assert_eq!(macro_escape("a,b\\"), "a\\,b\\\\");
    }
}
