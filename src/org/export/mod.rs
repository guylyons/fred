//! Port of ox.el, the export framework, with the ox-org, ox-ascii and
//! ox-md backends (and the part of ox-html that ox-md inherits).
//! See docs/org-port-notes.md ("Export").
//!
//! - [`Backend`]: a backend as Rust data (org-export-define-backend /
//!   -derived-backend): transcoders by element type, an options alist,
//!   filters and a dispatcher menu entry.  [`register`] / [`get`].
//! - [`Info`]: the communication channel (the `info` plist): options by
//!   property name (`title`, `with-toc`, ...), the parse tree and the
//!   caches the `org-export-get-*` helpers need.
//! - [`pipeline::export_as`] / [`string_as`]: org-export-as and
//!   org-export-string-as on text; [`Info::data`] is org-export-data.
//! - Macros are expanded through [`macros::expand_macros`], the seam to
//!   org-macro.el's port (src/org/macro.rs) at merge time.

// Work in progress: parts used only by the coming backends are unused.
#![allow(dead_code)]

// The backends (ascii, md, html, org), the pipeline
// (org-export-as), filling and the dispatcher are not written yet.
pub mod env;
pub mod include;
pub mod macros;
pub mod pipeline;
pub mod quotes;
pub mod table;
pub mod tools;

#[cfg(test)]
mod tests;

use crate::org::element::{self, NodeId, Tree, Type, Value};
use crate::org::syntax::Settings;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A transcoder for an element or object: (info, node, contents).
pub type NodeTx = fn(&mut Info, NodeId, Option<String>) -> Option<String>;
/// The plain-text transcoder: (info, the plain-text node if any, text).
pub type TextTx = fn(&mut Info, Option<NodeId>, &str) -> String;
/// `template` and `inner-template`: (info, contents).
pub type TemplateTx = fn(&mut Info, String) -> String;

/// An entry of a translation table.
#[derive(Clone, Copy)]
pub enum Tx {
    Node(NodeTx),
    Text(TextTx),
    Template(TemplateTx),
}

/// A filter (org-export-filters-alist values).
#[derive(Clone, Copy)]
pub enum Filter {
    /// `:filter-TYPE`, `:filter-plain-text`, `:filter-body`,
    /// `:filter-final-output`: the string, or None to skip this filter.
    Text(fn(&mut Info, String) -> Option<String>),
    /// `:filter-parse-tree` and `:filter-options`: work on INFO.
    Info(fn(&mut Info)),
}

/// How repeated keywords combine (BEHAVIOR in org-export-options-alist).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behavior {
    /// Keep the first value.
    First,
    /// Keep the last value.
    Last,
    Space,
    Newline,
    Split,
    Parse,
}

/// A row of an options alist: (PROPERTY KEYWORD OPTION DEFAULT BEHAVIOR).
/// DEFAULT is the variable's value, or `default` (Lisp data) when unset.
#[derive(Clone, Copy, Debug)]
pub struct OptDef {
    pub prop: &'static str,
    pub keyword: Option<&'static str>,
    pub option: Option<&'static str>,
    pub var: Option<&'static str>,
    pub default: &'static str,
    pub behavior: Behavior,
}

/// A dispatcher action: (async, subtree, visible-only, body-only).
pub type Action = fn(&mut crate::editor::Editor, Flags) -> Result<(), String>;

/// Interactive export options of the dispatcher.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub async_: bool,
    pub subtree: bool,
    pub visible: bool,
    pub body: bool,
}

/// A backend's dispatcher entry (:menu-entry).
#[derive(Clone)]
pub struct Menu {
    pub key: char,
    /// The description, or None for a sub-menu added to another
    /// backend's entry (an ordinal upstream).
    pub label: Option<&'static str>,
    pub ordinal: i64,
    pub entries: Vec<(char, &'static str, Action)>,
}

/// An export backend (org-export-backend).
#[derive(Clone, Default)]
pub struct Backend {
    pub name: Option<&'static str>,
    pub parent: Option<&'static str>,
    pub transcoders: Vec<(&'static str, Tx)>,
    pub options: Vec<OptDef>,
    pub filters: Vec<(&'static str, Filter)>,
    pub menu: Option<Menu>,
}

thread_local! {
    static REGISTRY: RefCell<Vec<Rc<Backend>>> = const { RefCell::new(vec![]) };
}

/// Install the built-in backends (none yet: work in progress).
fn builtins() {}

/// org-export-get-backend.
pub fn get(name: &str) -> Option<Rc<Backend>> {
    builtins();
    REGISTRY.with(|r| r.borrow().iter().find(|b| b.name == Some(name)).cloned())
}

/// org-export-registered-backends, oldest first.
pub fn registered() -> Vec<Rc<Backend>> {
    builtins();
    REGISTRY.with(|r| r.borrow().clone())
}

/// org-export-register-backend: replace a backend of the same name.
pub fn register(b: Backend) -> Result<(), String> {
    builtins();
    let Some(name) = b.name else {
        return Err("Cannot register a unnamed export backend".into());
    };
    if let Some(p) = b.parent
        && get(p).is_none()
    {
        return Err(format!("Cannot use unknown \"{p}\" backend as a parent"));
    }
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        match r.iter().position(|x| x.name == Some(name)) {
            Some(i) => r[i] = Rc::new(b),
            None => r.push(Rc::new(b)),
        }
    });
    Ok(())
}

impl Backend {
    /// The ancestors from this backend up (self first).
    fn chain(self: &Rc<Backend>) -> Vec<Rc<Backend>> {
        let mut out = vec![Rc::clone(self)];
        let mut p = self.parent;
        while let Some(name) = p {
            let Some(b) = get(name) else { break };
            p = b.parent;
            out.push(b);
        }
        out
    }

    /// org-export-derived-backend-p.
    pub fn derived_p(self: &Rc<Backend>, names: &[&str]) -> bool {
        self.chain()
            .iter()
            .any(|b| b.name.is_some_and(|n| names.contains(&n)))
    }

    /// org-export-get-all-transcoders.
    pub fn all_transcoders(self: &Rc<Backend>) -> Vec<(&'static str, Tx)> {
        self.chain()
            .iter()
            .flat_map(|b| b.transcoders.clone())
            .collect()
    }

    /// org-export-get-all-options.
    pub fn all_options(self: &Rc<Backend>) -> Vec<OptDef> {
        self.chain()
            .iter()
            .flat_map(|b| b.options.clone())
            .collect()
    }

    /// org-export-get-all-filters.
    pub fn all_filters(self: &Rc<Backend>) -> Vec<(&'static str, Filter)> {
        self.chain()
            .iter()
            .flat_map(|b| b.filters.clone())
            .collect()
    }

    pub fn name(&self) -> &'static str {
        self.name.unwrap_or("")
    }
}

/// org-export-define-derived-backend as data: a backend inheriting from
/// PARENT.
pub fn derived(name: &'static str, parent: &'static str) -> Backend {
    Backend {
        name: Some(name),
        parent: Some(parent),
        ..Backend::default()
    }
}

/// A resolved link destination: a node of the tree, or a file name
/// (org-export-resolve-id-link on an ID in another file).
#[derive(Clone, Debug, PartialEq)]
pub enum Dest {
    Node(NodeId),
    File(String),
}

/// A search cell (org-export-search-cells).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Cell {
    Headline(Vec<String>),
    Other(Vec<String>),
    CustomId(String),
    Target(Vec<String>),
}

/// The communication channel.
pub struct Info {
    pub tree: Tree,
    pub backend: Rc<Backend>,
    translate: Vec<(&'static str, Tx)>,
    plist: HashMap<String, Value>,
    exported: HashMap<NodeId, String>,
    pub ignore: Vec<NodeId>,
    filters: HashMap<&'static str, Vec<Filter>>,
    pub numbering: HashMap<NodeId, Vec<i64>>,
    pub headline_offset: i64,
    pub id_alist: Vec<(String, String)>,
    /// In-buffer settings of the document (TODO keywords, link abbrevs).
    pub settings: Rc<Settings>,
    /// :internal-references: (reference, node) and (cell, number).
    pub(crate) refs: Vec<(String, NodeId)>,
    pub(crate) ref_cells: Vec<(Cell, u32)>,
    pub(crate) caches: tools::Caches,
    /// org-link-broken signaled by a resolver: the unresolved path.
    pub broken: Option<String>,
    /// An error that aborts the export (user-error upstream).
    pub fatal: Option<String>,
    /// Full-buffer parse for footnote definitions outside a narrowing.
    pub(crate) wide: Option<Rc<(String, Settings)>>,
}

impl Info {
    pub(crate) fn new(backend: Rc<Backend>, settings: Rc<Settings>) -> Info {
        Info {
            tree: Tree::new(),
            translate: backend.all_transcoders(),
            backend,
            plist: HashMap::new(),
            exported: HashMap::new(),
            ignore: vec![],
            filters: HashMap::new(),
            numbering: HashMap::new(),
            headline_offset: 0,
            id_alist: vec![],
            settings,
            refs: vec![],
            ref_cells: vec![],
            caches: tools::Caches::default(),
            broken: None,
            fatal: None,
            wide: None,
        }
    }

    /// An empty channel for BACKEND (tests and helpers).
    pub fn empty(backend: &str) -> Info {
        Info::new(get(backend).expect("backend"), Rc::new(Settings::default()))
    }

    // ---- the plist ----

    /// plist-get (nil when missing).
    pub fn get(&self, k: &str) -> &Value {
        static NIL: Value = Value::Nil;
        self.plist.get(k.trim_start_matches(':')).unwrap_or(&NIL)
    }

    pub fn has(&self, k: &str) -> bool {
        self.plist.contains_key(k.trim_start_matches(':'))
    }

    /// plist-put; returns the old value.
    pub fn set(&mut self, k: &str, v: impl Into<Value>) -> Value {
        self.plist
            .insert(k.trim_start_matches(':').to_owned(), v.into())
            .unwrap_or_default()
    }

    pub fn flag(&self, k: &str) -> bool {
        self.get(k).truthy()
    }

    pub fn int(&self, k: &str) -> Option<i64> {
        self.get(k).as_int()
    }

    /// `wholenump`.
    pub fn whole(&self, k: &str) -> Option<usize> {
        self.int(k).filter(|n| *n >= 0).map(|n| n as usize)
    }

    pub fn str(&self, k: &str) -> Option<&str> {
        self.get(k).as_str()
    }

    /// A symbol-valued option is SYM.
    pub fn is(&self, k: &str, sym: &str) -> bool {
        match self.get(k) {
            Value::T => sym == "t",
            Value::Nil => sym == "nil",
            v => v.as_str() == Some(sym),
        }
    }

    /// Run F with K temporarily bound to V (org-combine-plists on a copy
    /// of INFO sharing its caches).
    pub fn with<R>(&mut self, k: &str, v: Value, f: impl FnOnce(&mut Info) -> R) -> R {
        let had = self.plist.remove(k);
        self.set(k, v);
        let r = f(self);
        match had {
            Some(o) => {
                self.set(k, o);
            }
            None => {
                self.plist.remove(k);
            }
        }
        r
    }

    /// :export-options contains OPT (`subtree`, `visible-only`, `body-only`).
    pub fn export_option(&self, opt: &str) -> bool {
        self.get("export-options")
            .as_list()
            .is_some_and(|l| l.iter().any(|v| v.as_str() == Some(opt)))
    }

    pub(crate) fn fail(&mut self, msg: impl Into<String>) {
        if self.fatal.is_none() {
            self.fatal = Some(msg.into());
        }
    }

    // ---- transcoding ----

    /// org-export-transcoder (a complete tree returns its contents).
    fn transcoder(&self, name: &str) -> Option<Tx> {
        if name == "org-data" {
            return Some(Tx::Node(|_, _, c| c));
        }
        self.translate
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, t)| *t)
    }

    pub fn template(&self, name: &str) -> Option<TemplateTx> {
        match self.transcoder(name) {
            Some(Tx::Template(f)) => Some(f),
            _ => None,
        }
    }

    /// A plain string as a parentless plain-text node.
    pub fn text_node(&mut self, s: &str) -> NodeId {
        self.tree.text(s)
    }

    /// org-export-data on a string.
    pub fn data_str(&mut self, s: &str) -> String {
        let id = self.text_node(s);
        self.data(id)
    }

    /// org-export-data on a secondary string (a list of objects).
    pub fn data_seq(&mut self, ids: &[NodeId]) -> String {
        ids.iter().map(|&i| self.data(i)).collect()
    }

    /// org-export-data on a property value (secondary string, string or
    /// nil).
    pub fn data_value(&mut self, v: &Value) -> String {
        match v {
            Value::Secondary(s) => self.data_seq(&s.clone()),
            Value::Str(s) => self.data_str(&s.clone()),
            Value::Node(n) => self.data(*n),
            Value::List(l) => l.clone().iter().map(|x| self.data_value(x)).collect(),
            _ => String::new(),
        }
    }

    /// org-export-data.
    pub fn data(&mut self, id: NodeId) -> String {
        if let Some(s) = self.exported.get(&id) {
            return s.clone();
        }
        let kind = self.tree.kind(id);
        let results = if self.ignore.contains(&id) {
            None
        } else {
            match kind {
                Type::Pseudo("raw") => Some(self.tree.node(id).text.clone()),
                Type::PlainText => {
                    let text = self.tree.node(id).text.clone();
                    let r = match self.transcoder("plain-text") {
                        Some(Tx::Text(f)) => f(self, Some(id), &text),
                        _ => text,
                    };
                    Some(self.apply_filters("filter-plain-text", r))
                }
                Type::Anonymous => {
                    let c = self.tree.contents(id).to_vec();
                    Some(self.data_seq(&c))
                }
                _ if self.tree.contents(id).is_empty()
                    || (kind == Type::Headline
                        && self.is("with-archived-trees", "headline")
                        && self.tree.flag(id, "archivedp")) =>
                {
                    match self.transcoder(kind.name()) {
                        Some(Tx::Node(f)) => {
                            let r = f(self, id, None);
                            self.broken_link(kind, r)
                        }
                        _ => None,
                    }
                }
                _ => match self.transcoder(kind.name()) {
                    Some(Tx::Node(f)) => {
                        let greater = kind.is_greater();
                        let objectp = !greater && kind.is_recursive();
                        if !greater && !objectp {
                            self.normalize_contents(id);
                        }
                        let children = self.tree.contents(id).to_vec();
                        let contents = self.data_seq(&children);
                        let contents = if greater {
                            element::normalize_string(&contents, false)
                        } else {
                            contents
                        };
                        let r = f(self, id, Some(contents));
                        self.broken_link(kind, r)
                    }
                    _ => None,
                },
            }
        };
        let out = match results {
            None => self.keep_spaces(id).unwrap_or_default(),
            Some(r)
                if matches!(
                    kind,
                    Type::Anonymous | Type::OrgData | Type::PlainText | Type::Pseudo("raw")
                ) =>
            {
                r
            }
            Some(r) => {
                let blank = self.tree.post_blank(id);
                let r = if self.tree.class(id) == element::Class::Object {
                    format!("{r}{}", " ".repeat(blank))
                } else {
                    format!(
                        "{}{}",
                        element::normalize_string(&r, false),
                        "\n".repeat(blank)
                    )
                };
                let key = format!("filter-{}", kind.name());
                self.apply_filters(&key, r)
            }
        };
        self.exported.insert(id, out.clone());
        out
    }

    /// The broken-link handler of org-export-data.
    fn broken_link(&mut self, _kind: Type, r: Option<String>) -> Option<String> {
        let Some(path) = self.broken.take() else {
            return r;
        };
        if self.is("with-broken-links", "mark") {
            Some(self.data_str(&format!("[BROKEN LINK: {path}]")))
        } else if self.flag("with-broken-links") {
            None
        } else {
            self.fail(format!(
                "Org export aborted.  Unable to resolve link: {path:?}\nSee `org-export-with-broken-links'"
            ));
            None
        }
    }

    /// org-element-normalize-contents on a paragraph or verse block
    /// (destructive, as upstream).
    fn normalize_contents(&mut self, id: NodeId) {
        let t = &self.tree;
        let kind = t.kind(id);
        let ignore_first = kind == Type::Paragraph
            && t.parent(id).is_some_and(|p| {
                t.is(p, &[Type::FootnoteDefinition, Type::Item])
                    && t.contents(p).first() == Some(&id)
                    && t.int(p, "pre-blank") == Some(0)
            });
        for (n, s) in element::normalize_contents(t, id, ignore_first) {
            self.tree.node_mut(n).text = s;
        }
    }

    /// org-export--keep-spaces.
    fn keep_spaces(&self, id: NodeId) -> Option<String> {
        let blank = self.tree.post_blank(id);
        if blank == 0 || self.tree.class(id) == element::Class::Element {
            return None;
        }
        let prev = self.previous(id)?;
        let has_space = match self.tree.plain_text(prev) {
            Some(s) => s.ends_with([' ', '\t', '\r', '\n']),
            None => self.tree.post_blank(prev) > 0,
        };
        (!has_space).then(|| " ".repeat(blank))
    }

    /// org-export-data-with-backend.
    pub fn data_with_backend(&mut self, id: NodeId, backend: &Rc<Backend>) -> String {
        let saved = (
            std::mem::replace(&mut self.backend, Rc::clone(backend)),
            std::mem::replace(&mut self.translate, backend.all_transcoders()),
            std::mem::take(&mut self.exported),
        );
        let r = self.data(id);
        (self.backend, self.translate, self.exported) = saved;
        r
    }

    /// org-export-data-with-backend on a secondary string.
    pub fn data_seq_with_backend(&mut self, ids: &[NodeId], backend: &Rc<Backend>) -> String {
        let saved = (
            std::mem::replace(&mut self.backend, Rc::clone(backend)),
            std::mem::replace(&mut self.translate, backend.all_transcoders()),
            std::mem::take(&mut self.exported),
        );
        let r = self.data_seq(ids);
        (self.backend, self.translate, self.exported) = saved;
        r
    }

    /// org-export-with-backend: BACKEND's transcoder for ID's type.
    pub fn with_backend(
        &mut self,
        name: &str,
        id: NodeId,
        contents: Option<String>,
    ) -> Option<String> {
        let b = get(name)?;
        let all = b.all_transcoders();
        let kind = self.tree.kind(id);
        let f = all
            .iter()
            .find(|(k, _)| *k == kind.name())
            .map(|(_, t)| *t)?;
        let saved = (
            std::mem::replace(&mut self.backend, Rc::clone(&b)),
            std::mem::replace(&mut self.translate, all),
            std::mem::take(&mut self.exported),
        );
        let r = match f {
            Tx::Node(f) => f(self, id, contents),
            Tx::Text(f) => {
                let text = self.tree.node(id).text.clone();
                Some(f(self, Some(id), &text))
            }
            Tx::Template(_) => None,
        };
        (self.backend, self.translate, self.exported) = saved;
        r
    }

    /// The memoized output of ID, if it was transcoded.
    pub fn exported(&self, id: NodeId) -> Option<&str> {
        self.exported.get(&id).map(String::as_str)
    }

    // ---- filters ----

    /// org-export-install-filters: backend filters first, then the
    /// user's (none: Fred has no Lisp filter functions).
    pub(crate) fn install_filters(&mut self) {
        let mut m: HashMap<&'static str, Vec<Filter>> = HashMap::new();
        for (k, f) in self.backend.all_filters().into_iter().rev() {
            m.entry(k).or_default().insert(0, f);
        }
        self.filters = m;
    }

    /// org-export-filter-apply-functions on a string.
    pub fn apply_filters(&mut self, key: &str, mut value: String) -> String {
        let Some(fs) = self.filters.get(key).cloned() else {
            return value;
        };
        for f in fs {
            if let Filter::Text(f) = f {
                match f(self, value.clone()) {
                    None => {}
                    Some(r) if r.is_empty() => return r,
                    Some(r) => value = r,
                }
            }
        }
        value
    }

    /// Run the `Info` filters registered under KEY (parse tree, options).
    pub(crate) fn apply_info_filters(&mut self, key: &str) {
        for f in self.filters.get(key).cloned().unwrap_or_default() {
            if let Filter::Info(f) = f {
                f(self);
            }
        }
    }
}

/// Copy node ID of SRC (with its secondary strings and node-valued
/// properties) into DST; the copy has no parent.
pub(crate) fn graft(dst: &mut Tree, src: &Tree, id: NodeId) -> NodeId {
    fn value(dst: &mut Tree, src: &Tree, v: &Value, owner: NodeId) -> Value {
        match v {
            Value::Node(n) => {
                let c = graft(dst, src, *n);
                dst.set_parent(c, Some(owner));
                Value::Node(c)
            }
            Value::Secondary(s) => {
                let ids: Vec<NodeId> = s.iter().map(|&n| graft(dst, src, n)).collect();
                for &c in &ids {
                    dst.set_parent(c, Some(owner));
                }
                Value::Secondary(ids)
            }
            Value::List(l) => Value::List(l.iter().map(|x| value(dst, src, x, owner)).collect()),
            Value::Cons(a, b) => Value::cons(value(dst, src, a, owner), value(dst, src, b, owner)),
            v => v.clone(),
        }
    }
    let n = src.node(id);
    let new = dst.create(n.kind, vec![], vec![]);
    dst.node_mut(new).text = n.text.clone();
    let props: Vec<(String, Value)> = n
        .props
        .iter()
        .map(|(k, v)| (k.clone(), value(dst, src, v, new)))
        .collect();
    dst.node_mut(new).props = props;
    let kids: Vec<NodeId> = src
        .contents(id)
        .iter()
        .map(|&c| graft(dst, src, c))
        .collect();
    dst.set_contents(new, kids);
    new
}

/// org-element-parse-secondary-string into TREE: the objects, parented
/// to a fresh anonymous node.
pub(crate) fn parse_secondary(
    tree: &mut Tree,
    s: &str,
    restriction: &[Type],
    st: &Settings,
) -> Vec<NodeId> {
    let t = element::parse_secondary_string(s, restriction, st);
    let ids: Vec<NodeId> = t
        .contents(t.root)
        .iter()
        .map(|&c| graft(tree, &t, c))
        .collect();
    tree.anonymous(ids.clone());
    ids
}

/// This module's interactive commands, by upstream name.
pub fn command(
    ed: &mut crate::editor::Editor,
    name: &str,
    arg: crate::org::Prefix,
) -> Option<Result<(), String>> {
    let _ = (ed, name, arg);
    None
}
