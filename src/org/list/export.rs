//! org-list-to-lisp and the list transcoders: org-list-to-generic,
//! -to-org, -to-subtree, -to-latex, -to-html, -to-texinfo.
//!
//! Upstream writes the parsed list back into a temporary Org buffer,
//! parses it with org-element and exports it through an ox backend. Here
//! the parsed list is transcoded directly with the same rules (contents
//! stay in Org syntax; backends transcode the list and item markup).

use super::buf::Buf;
use super::structure::*;
use std::rc::Rc;
use std::sync::LazyLock;

/// A parsed item element: text, or a sub-list.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text(String),
    List(List),
}

/// A list as returned by org-list-to-lisp.
#[derive(Clone, Debug, PartialEq)]
pub struct List {
    pub kind: ListType,
    pub items: Vec<Vec<Node>>,
}

/// org-remove-indentation.
pub fn remove_indentation(code: &str) -> String {
    let n = code
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| super::buf::col_of(&l[..l.len() - l.trim_start_matches([' ', '\t']).len()]))
        .min()
        .unwrap_or(0);
    if n == 0 {
        return code.to_owned();
    }
    let mut out: Vec<String> = vec![];
    for l in code.split('\n') {
        if l.trim().is_empty() {
            out.push(l.to_owned());
            continue;
        }
        let mut col = 0;
        let mut i = 0;
        for (j, c) in l.char_indices() {
            if col >= n || !(c == ' ' || c == '\t') {
                i = j;
                break;
            }
            col = if c == '\t' {
                (col / 8 + 1) * 8
            } else {
                col + 1
            };
            i = j + 1;
        }
        out.push(format!("{}{}", " ".repeat(col.saturating_sub(n)), &l[i..]));
    }
    out.join("\n")
}

/// org-list-to-lisp: the list at point (deleted when `delete`); point is
/// left at the list's top.
pub fn to_lisp(b: &mut Buf, delete: bool) -> List {
    let st = list_struct(b);
    let prevs = st.prevs();
    let parents = st.parents();
    let top = st.top();
    let bottom = st.bottom();
    let trim = |t: &str| remove_indentation(t.strip_suffix('\n').unwrap_or(t));
    fn sublist(
        b: &Buf,
        st: &Struct,
        prevs: &Alist,
        parents: &Alist,
        items: &[usize],
        trim: &dyn Fn(&str) -> String,
    ) -> List {
        List {
            kind: st.list_type(items[0], prevs),
            items: items
                .iter()
                .map(|&e| item(b, st, prevs, parents, e, trim))
                .collect(),
        }
    }
    fn item(
        b: &Buf,
        st: &Struct,
        prevs: &Alist,
        parents: &Alist,
        e: usize,
        trim: &dyn Fn(&str) -> String,
    ) -> Vec<Node> {
        let end = st.end(e);
        let mut kids = children(e, parents);
        let m = b
            .looking_at_pos(&HEAD, e)
            .and_then(|c| c.end(0))
            .unwrap_or(e);
        let first_end = kids.first().copied().unwrap_or(end);
        let width = super::buf::col_of(b.sub(e, m));
        let mut body = vec![Node::Text(trim(&format!(
            "{}{}",
            " ".repeat(width),
            b.sub(m, first_end)
        )))];
        while let Some(&child) = kids.first() {
            let sub = all_items(child, prevs);
            let last = *sub.last().unwrap();
            body.push(Node::List(sublist(b, st, prevs, parents, &sub, trim)));
            kids = kids
                .iter()
                .skip_while(|k| **k != last)
                .skip(1)
                .copied()
                .collect();
            let sub_end = st.end(last);
            let next = kids.first().copied().unwrap_or(end);
            if sub_end != next {
                body.push(Node::Text(trim(b.sub(sub_end, next))));
            }
        }
        body
    }
    let list = sublist(b, &st, &prevs, &parents, &all_items(top, &prevs), &trim);
    b.goto(top);
    if delete {
        b.delete(top, bottom);
    }
    list
}

static HEAD: LazyLock<regex::Regex> = LazyLock::new(|| re(r"\A[ \t]*\S+[ \t]*"));
static PREFIX: LazyLock<regex::Regex> = LazyLock::new(|| {
    re(r"\A(?m:(?:\[@(?:start:)?([0-9]+|[A-Za-z])\][ \t]*)?(?:(\[[ X-]\])(?:[ \t]+|$))?)")
});
static TAG: LazyLock<regex::Regex> = LazyLock::new(|| re(r"\A(?m:(.*)[ \t]+::(?:[ \t]+|$))"));

/// Arguments to a generic parameter function.
pub struct GenArgs<'a> {
    pub kind: ListType,
    pub depth: usize,
    pub counter: Option<i64>,
    pub contents: &'a str,
}

/// A parameter function.
pub type GenFn = Rc<dyn Fn(&GenArgs) -> Option<String>>;

/// A string parameter, or a function computing it (nil when None).
#[derive(Clone)]
pub enum Gen {
    Str(String),
    Fn(GenFn),
}

impl Gen {
    pub fn s(s: &str) -> Option<Gen> {
        Some(Gen::Str(s.to_owned()))
    }
    pub fn f(f: impl Fn(&GenArgs) -> Option<String> + 'static) -> Option<Gen> {
        Some(Gen::Fn(Rc::new(f)))
    }
    fn eval(&self, a: &GenArgs) -> Option<String> {
        match self {
            Gen::Str(s) => Some(s.clone()),
            Gen::Fn(f) => f(a),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Latex,
    Html,
    Texinfo,
}

/// org-list-to-generic parameters.
#[derive(Clone, Default)]
pub struct Params {
    pub backend: Option<Backend>,
    pub splice: bool,
    pub ustart: Option<Gen>,
    pub uend: Option<Gen>,
    pub ostart: Option<Gen>,
    pub oend: Option<Gen>,
    pub dstart: Option<Gen>,
    pub dend: Option<Gen>,
    pub dtstart: Option<String>,
    pub dtend: Option<String>,
    pub ddstart: Option<String>,
    pub ddend: Option<String>,
    pub istart: Option<Gen>,
    pub iend: Option<Gen>,
    pub icount: Option<Gen>,
    pub isep: Option<Gen>,
    pub ifmt: Option<Gen>,
    pub cbon: Option<String>,
    pub cboff: Option<String>,
    pub cbtrans: Option<String>,
}

impl Params {
    /// org-combine-plists: `over`'s values win.
    pub fn merge(self, over: &Params) -> Params {
        macro_rules! m {
            ($($f:ident),*) => { Params { backend: over.backend.or(self.backend), splice: over.splice || self.splice, $($f: over.$f.clone().or(self.$f)),* } };
        }
        m!(
            ustart, uend, ostart, oend, dstart, dend, dtstart, dtend, ddstart, ddend, istart, iend,
            icount, isep, ifmt, cbon, cboff, cbtrans
        )
    }
}

/// org-element-normalize-string.
fn normalize(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    // Trailing (newline + blanks)* become one newline.
    let mut keep = s.len();
    let bytes = s.as_bytes();
    loop {
        let mut k = keep;
        while k > 0 && matches!(bytes[k - 1], b' ' | b'\t') {
            k -= 1;
        }
        if k > 0 && bytes[k - 1] == b'\n' {
            keep = k - 1;
        } else {
            break;
        }
    }
    format!("{}\n", &s[..keep])
}

/// org-list--trailing-newlines.
fn trailing_newlines(s: &str) -> usize {
    let t = s.trim_end_matches([' ', '\t', '\n']);
    s[t.len()..].matches('\n').count().saturating_sub(1)
}

struct Parsed {
    counter: Option<i64>,
    checkbox: Option<&'static str>,
    tag: Option<String>,
    elems: Vec<Node>,
}

/// An item as org-element sees it once written back as `- ` or `1. `.
fn parse_item(kind: ListType, nodes: &[Node]) -> Parsed {
    let mut p = Parsed {
        counter: None,
        checkbox: None,
        tag: None,
        elems: vec![],
    };
    let mut rest = nodes;
    if let Some(Node::Text(t)) = nodes.first() {
        rest = &nodes[1..];
        let mut s = t.as_str();
        if let Some(c) = PREFIX.captures(s) {
            p.counter = c.get(1).map(|m| {
                let v = m.as_str();
                v.parse()
                    .unwrap_or_else(|_| v.to_ascii_uppercase().as_bytes()[0] as i64 - 64)
            });
            p.checkbox = c.get(2).map(|m| match m.as_str() {
                "[X]" => "on",
                "[-]" => "trans",
                _ => "off",
            });
            s = &s[c.get(0).unwrap().end()..];
        }
        if kind != ListType::Ordered
            && let Some(c) = TAG.captures(s)
        {
            p.tag = Some(c[1].trim_end().to_owned());
            s = &s[c.get(0).unwrap().end()..];
        }
        if !s.trim().is_empty() {
            p.elems.push(Node::Text(s.to_owned()));
        }
    }
    p.elems.extend(rest.iter().cloned());
    p
}

fn list_type_of(l: &List) -> ListType {
    match l.kind {
        ListType::Ordered => ListType::Ordered,
        _ if l
            .items
            .first()
            .is_some_and(|i| parse_item(ListType::Unordered, i).tag.is_some()) =>
        {
            ListType::Descriptive
        }
        _ => ListType::Unordered,
    }
}

/// An element's export: normalized output plus its post-blank newlines.
fn export_data(out: &str, post_blank: usize) -> String {
    format!("{}{}", normalize(out), "\n".repeat(post_blank))
}

fn plain_list(l: &List, depth: usize, p: &Params) -> (String, usize) {
    let kind = list_type_of(l);
    let a = GenArgs {
        kind,
        depth,
        counter: None,
        contents: "",
    };
    let (s, e) = match kind {
        ListType::Ordered => (&p.ostart, &p.oend),
        ListType::Unordered => (&p.ustart, &p.uend),
        ListType::Descriptive => (&p.dstart, &p.dend),
    };
    let start = (!p.splice)
        .then(|| s.as_ref().and_then(|g| g.eval(&a)))
        .flatten();
    let end = (!p.splice)
        .then(|| e.as_ref().and_then(|g| g.eval(&a)))
        .flatten();
    let post_blank = end.as_deref().map_or(0, trailing_newlines);
    let n = l.items.len();
    let contents: String = l
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let (o, pb) = item(kind, it, depth, i + 1 < n, p);
            export_data(&o, pb)
        })
        .collect();
    let body = match p.backend {
        Some(be) if start.is_none() && end.is_none() && !p.splice => {
            backend_list(be, kind, &contents)
        }
        _ => contents,
    };
    (
        format!(
            "{}{body}{}",
            start.map_or(String::new(), |s| s + "\n"),
            end.unwrap_or_default()
        ),
        post_blank,
    )
}

fn item(
    kind: ListType,
    nodes: &[Node],
    depth: usize,
    has_next: bool,
    p: &Params,
) -> (String, usize) {
    let it = parse_item(kind, nodes);
    let a = GenArgs {
        kind,
        depth,
        counter: it.counter,
        contents: "",
    };
    let ne = it.elems.len();
    let contents: String = it
        .elems
        .iter()
        .enumerate()
        .map(|(i, e)| match e {
            Node::Text(t) => {
                let pb = if i + 1 < ne {
                    t.len() - t.trim_end_matches('\n').len()
                } else {
                    0
                };
                export_data(t, pb)
            }
            Node::List(l) => {
                let (o, pb) = plain_list(l, depth + 1, p);
                let own = if i + 1 < ne { trailing_blank(l) } else { 0 };
                export_data(&o, if pb > 0 { pb } else { own })
            }
        })
        .collect();
    let separator = if has_next {
        p.isep.as_ref().and_then(|g| g.eval(&a))
    } else {
        None
    };
    let closing = match p.iend.as_ref().and_then(|g| g.eval(&a)) {
        None => "\n".to_owned(),
        Some(s) if s.is_empty() => "\n".to_owned(),
        Some(s) if separator.is_some() => {
            if s.ends_with('\n') {
                s
            } else {
                s + "\n"
            }
        }
        Some(s) => s,
    };
    let last = separator.as_deref().unwrap_or(&closing);
    let post_blank = trailing_newlines(last).saturating_sub(1);
    let start = match (&it.counter, &p.icount) {
        (Some(_), Some(g)) => g.eval(&a),
        _ => p.istart.as_ref().and_then(|g| g.eval(&a)),
    };
    let generic = p.istart.is_some()
        || p.iend.is_some()
        || p.icount.is_some()
        || p.ifmt.is_some()
        || p.cbon.is_some()
        || p.cboff.is_some()
        || p.cbtrans.is_some()
        || p.backend.is_none()
        || (kind == ListType::Descriptive
            && (p.dtstart.is_some()
                || p.dtend.is_some()
                || p.ddstart.is_some()
                || p.ddend.is_some()));
    let body = if let (false, Some(be)) = (generic, p.backend) {
        backend_item(be, kind, &it, &contents)
    } else {
        let mut b = String::new();
        match it.checkbox {
            Some("on") => b.push_str(p.cbon.as_deref().unwrap_or("")),
            Some("off") => b.push_str(p.cboff.as_deref().unwrap_or("")),
            Some(_) => b.push_str(p.cbtrans.as_deref().unwrap_or("")),
            None => {}
        }
        if let Some(t) = &it.tag {
            b.push_str(p.dtstart.as_deref().unwrap_or(""));
            b.push_str(t);
            b.push_str(p.dtend.as_deref().unwrap_or(""));
            b.push_str(p.ddstart.as_deref().unwrap_or(""));
        }
        let c = if contents.is_empty() {
            ""
        } else {
            &contents[..contents.len() - 1]
        };
        match &p.ifmt {
            Some(g) => b.push_str(&g.eval(&GenArgs { contents: c, ..a }).unwrap_or_default()),
            None => b.push_str(c),
        }
        if it.tag.is_some() {
            b.push_str(p.ddend.as_deref().unwrap_or(""));
        }
        b
    };
    let body = if body.is_empty() {
        String::new()
    } else {
        let n = normalize(&body);
        n[..n.len() - 1].to_owned()
    };
    let out = format!(
        "{}{body}{closing}{}",
        start.unwrap_or_default(),
        separator.unwrap_or_default()
    );
    (out, post_blank)
}

fn backend_list(be: Backend, kind: ListType, contents: &str) -> String {
    match be {
        Backend::Latex => {
            let env = match kind {
                ListType::Ordered => "enumerate",
                ListType::Unordered => "itemize",
                ListType::Descriptive => "description",
            };
            format!("\\begin{{{env}}}\n{contents}\\end{{{env}}}")
        }
        Backend::Html => {
            let (t, c) = match kind {
                ListType::Ordered => ("ol", "org-ol"),
                ListType::Unordered => ("ul", "org-ul"),
                ListType::Descriptive => ("dl", "org-dl"),
            };
            format!("<{t} class=\"{c}\">\n{contents}</{t}>")
        }
        Backend::Texinfo => match kind {
            ListType::Ordered => format!("@enumerate\n{contents}@end enumerate"),
            ListType::Unordered => format!("@itemize\n{contents}@end itemize"),
            ListType::Descriptive => format!("@table @asis\n{contents}@end table"),
        },
    }
}

fn backend_item(be: Backend, kind: ListType, it: &Parsed, contents: &str) -> String {
    let c = contents.trim();
    match be {
        Backend::Latex => {
            let cb = match it.checkbox {
                Some("on") => "$\\boxtimes$ ",
                Some("off") => "$\\square$ ",
                Some(_) => "$\\boxminus$ ",
                None => "",
            };
            match &it.tag {
                Some(t) => format!("\\item[{{{cb}{t}}}] {c}"),
                None => format!("\\item {cb}{c}"),
            }
        }
        Backend::Html => {
            let cb = match it.checkbox {
                Some("on") => "<code>[X]</code> ",
                Some("off") => "<code>[&#xa0;]</code> ",
                Some(_) => "<code>[-]</code> ",
                None => "",
            };
            match (kind, &it.tag) {
                (ListType::Descriptive, Some(t)) => format!("<dt>{cb}{t}</dt><dd>{c}</dd>"),
                _ => format!("<li>{cb}{c}</li>"),
            }
        }
        Backend::Texinfo => match &it.tag {
            Some(t) => format!("@item {t}\n{c}"),
            None => format!("@item\n{c}"),
        },
    }
}

/// Blank lines a sub-list ends with (its plain-list post-blank).
fn trailing_blank(l: &List) -> usize {
    match l.items.last().and_then(|i| i.last()) {
        Some(Node::Text(t)) => t.len() - t.trim_end_matches('\n').len(),
        Some(Node::List(l)) => trailing_blank(l),
        None => 0,
    }
}

/// org-list-to-generic.
pub fn to_generic(list: &List, params: &Params) -> String {
    let (out, pb) = plain_list(list, 1, params);
    let out = export_data(&out, pb);
    if out.trim().is_empty() {
        String::new()
    } else {
        out[..out.len() - 1].to_owned()
    }
}

/// org-list-to-org.
pub fn to_org(list: &List, params: &Params) -> String {
    let make = Gen::f(|a| {
        Some(format!(
            "{}{}",
            if a.kind == ListType::Ordered {
                "1. "
            } else {
                "- "
            },
            a.counter.map_or(String::new(), |c| format!("[@{c}] "))
        ))
    });
    let defaults = Params {
        istart: make.clone(),
        icount: make,
        ifmt: Gen::f(|a| Some(a.contents.replace('\n', "\n  "))),
        dtend: Some(" :: ".into()),
        cbon: Some("[X] ".into()),
        cboff: Some("[ ] ".into()),
        cbtrans: Some("[-] ".into()),
        ..Params::default()
    };
    to_generic(list, &defaults.merge(params))
}

/// org-list-to-subtree. `blank`: a blank line between headings.
pub fn to_subtree(list: &List, level: usize, blank: bool, odd: bool, params: &Params) -> String {
    let stars = Gen::f(move |a| {
        let l = level + a.depth - 1;
        Some(format!("{} ", "*".repeat(if odd { 2 * l - 1 } else { l })))
    });
    let defaults = Params {
        splice: true,
        istart: stars.clone(),
        icount: stars,
        dtstart: Some(" ".into()),
        dtend: Some(" ".into()),
        isep: Gen::s(if blank { "\n\n" } else { "\n" }),
        cbon: Some("DONE ".into()),
        cboff: Some("TODO ".into()),
        cbtrans: Some("TODO ".into()),
        ..Params::default()
    };
    to_generic(list, &defaults.merge(params))
}

/// org-list-to-latex.
pub fn to_latex(list: &List, params: &Params) -> String {
    to_generic(
        list,
        &Params {
            backend: Some(Backend::Latex),
            ..Params::default()
        }
        .merge(params),
    )
}

/// org-list-to-html.
pub fn to_html(list: &List, params: &Params) -> String {
    to_generic(
        list,
        &Params {
            backend: Some(Backend::Html),
            ..Params::default()
        }
        .merge(params),
    )
}

/// org-list-to-texinfo.
pub fn to_texinfo(list: &List, params: &Params) -> String {
    to_generic(
        list,
        &Params {
            backend: Some(Backend::Texinfo),
            ..Params::default()
        }
        .merge(params),
    )
}
