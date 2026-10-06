//! "Tools For Backends" of ox.el: affiliated keywords, footnotes,
//! headlines, links, references, src blocks, tables of contents and
//! topology.

use super::{Backend, Cell, Dest, Info, Tx};
use crate::org::element::{self, MapOpts, NodeId, Tree, Type, Value, Visit};
use crate::org::options;
use regex::Regex;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

/// A predicate on nodes (org-export-collect-elements).
pub type Pred<'a> = &'a dyn Fn(&Info, NodeId) -> bool;

/// Memoized data of the helpers (the `*-cache` properties upstream).
#[derive(Default)]
pub struct Caches {
    /// Label to definition (a footnote-definition or an inline
    /// footnote-reference).
    pub(crate) footnote_defs: Option<HashMap<String, NodeId>>,
    /// Footnote references in visiting order over the whole tree, per
    /// BODY-FIRST.
    pub(crate) footnote_order: [Option<Vec<NodeId>>; 2],
    pub(crate) fuzzy_cells: Option<HashMap<Cell, Vec<NodeId>>>,
    pub(crate) fuzzy: HashMap<Vec<Cell>, Option<NodeId>>,
    pub(crate) ids: Option<HashMap<String, NodeId>>,
    pub(crate) table_header: HashMap<NodeId, bool>,
    pub(crate) row_group: HashMap<NodeId, Option<usize>>,
    pub(crate) cell_width: HashMap<NodeId, Vec<Option<Option<usize>>>>,
    pub(crate) cell_align: HashMap<NodeId, Vec<Option<super::table::Align>>>,
    pub(crate) row_number: HashMap<NodeId, Option<usize>>,
    pub(crate) smart_quotes: HashMap<(NodeId, Option<&'static str>), super::quotes::Status>,
    /// ox-ascii's :ascii-table-cell-width-cache.
    pub(crate) ascii_cell_width: HashMap<(NodeId, usize), usize>,
    /// Random state for org-export-new-reference.
    pub(crate) seed: u64,
}

impl Info {
    // ---- mapping ----

    /// org-element-map with INFO (ignored nodes skipped).  TYPES None
    /// maps every node.
    pub fn map<R>(
        &self,
        data: &[NodeId],
        types: Option<&[Type]>,
        no_recursion: &[Type],
        with_affiliated: bool,
        first_match: bool,
        mut f: impl FnMut(&Info, NodeId) -> Option<R>,
    ) -> Vec<R> {
        let objects =
            types.is_none_or(|t| t.iter().any(|x| x.is_object() || *x == Type::PlainText));
        let mut nr = no_recursion.to_vec();
        if !objects {
            nr.extend_from_slice(element::ALL_OBJECTS);
            nr.extend(element::ALL_ELEMENTS.iter().filter(|t| !t.is_greater()));
        }
        let ignore: Vec<NodeId> = self.ignore.clone();
        let opts = MapOpts {
            ignore: &ignore,
            first_match,
            no_recursion: nr,
            with_properties: if with_affiliated && objects {
                vec!["caption"]
            } else {
                vec![]
            },
            no_secondary: !objects,
        };
        let mut out = vec![];
        for &d in data {
            let r = self.tree.ast_map(d, types, &opts, |_, n| match f(self, n) {
                Some(r) => Visit::Value(r),
                None => Visit::Nil,
            });
            let done = first_match && !r.is_empty();
            out.extend(r);
            if done {
                break;
            }
        }
        out
    }

    /// org-element-map on the whole parse tree, first match.
    pub fn find<R>(&self, types: &[Type], f: impl FnMut(&Info, NodeId) -> Option<R>) -> Option<R> {
        let root = self.tree.root;
        self.map(&[root], Some(types), &[], false, true, f)
            .into_iter()
            .next()
    }

    // ---- affiliated keywords ----

    /// org-export-read-attribute: `:key value` pairs of ATTRIBUTE
    /// (`attr_ascii`); empty and "nil" values are None.
    pub fn read_attribute(&self, attribute: &str, id: NodeId) -> Vec<(String, Option<String>)> {
        let v = self.tree.prop(id, attribute);
        let parts: Vec<String> = match v {
            Value::List(l) => l
                .iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect(),
            Value::Str(s) => vec![s.clone()],
            _ => return vec![],
        };
        let s = parts.join(" ");
        static KEY: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?:^|[ \t]+)(:[-a-zA-Z0-9_]+)([ \t]+|$)").unwrap());
        let prepare = |s: &str| -> Option<String> {
            if s.is_empty() || s == "nil" {
                return None;
            }
            if s.len() >= 2
                && s.starts_with('"')
                && s.ends_with('"')
                && s[1..s.len() - 1].chars().all(|c| c == '"')
            {
                return Some(s[1..s.len() - 1].to_owned());
            }
            Some(s.to_owned())
        };
        let mut out = vec![];
        let mut rest = s.as_str();
        let mut key: Option<String> = None;
        while let Some(c) = KEY.captures(rest) {
            let m = c.get(0).unwrap();
            if let Some(k) = key.take() {
                out.push((k, prepare(&rest[..m.start()])));
            }
            key = Some(c[1].to_owned());
            rest = &rest[m.end()..];
            if rest.is_empty() {
                break;
            }
        }
        if let Some(k) = key {
            out.push((k, prepare(rest)));
        }
        out
    }

    /// One property of [`Info::read_attribute`].
    pub fn attribute(&self, attribute: &str, id: NodeId, prop: &str) -> Option<String> {
        self.read_attribute(attribute, id)
            .into_iter()
            .find(|(k, _)| k == prop)
            .and_then(|(_, v)| v)
    }

    /// org-export-get-caption: the caption lines joined by a space (as
    /// one list of objects; SHORT for the short caption).
    pub fn caption(&mut self, id: NodeId, short: bool) -> Vec<NodeId> {
        let lines: Vec<Value> = match self.tree.prop(id, "caption") {
            Value::List(l) => l.clone(),
            Value::Nil => vec![],
            v => vec![v.clone()],
        };
        let mut out: Vec<NodeId> = vec![];
        for line in lines {
            let (main, dual) = match line {
                Value::Cons(a, b) => (*a, *b),
                v => (v, Value::Nil),
            };
            let part = if short { dual } else { main };
            if let Value::Secondary(s) = part {
                if s.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    let sp = self.tree.text(" ");
                    out.push(sp);
                }
                out.extend(s);
            }
        }
        out
    }

    // ---- export snippets ----

    /// org-export-snippet-backend.
    pub fn snippet_backend(&self, id: NodeId) -> String {
        let b = self.tree.str(id, "back-end").unwrap_or("").to_owned();
        if let Some(crate::org::sexp::Sexp::List(l)) =
            super::env::var("org-export-snippet-translation-alist")
        {
            for e in l {
                if e.car().and_then(|x| x.str()) == Some(b.as_str())
                    && let Some(to) = e.cdr().str()
                {
                    return to.to_owned();
                }
            }
        }
        b
    }

    // ---- footnotes ----

    fn footnote_defs(&mut self) -> &HashMap<String, NodeId> {
        if self.caches.footnote_defs.is_none() {
            let root = self.tree.root;
            let pairs = self.map(
                &[root],
                Some(&[Type::FootnoteDefinition, Type::FootnoteReference]),
                &[],
                false,
                false,
                |i, f| {
                    if i.tree.prop(f, "type").as_str() == Some("standard") {
                        return None;
                    }
                    Some((i.tree.str(f, "label")?.to_owned(), f))
                },
            );
            let mut m = HashMap::new();
            for (k, v) in pairs {
                m.insert(k, v);
            }
            self.caches.footnote_defs = Some(m);
        }
        self.caches.footnote_defs.as_ref().unwrap()
    }

    /// org-export-get-footnote-definition: the definition's contents
    /// (None, with an error recorded, when there is none).
    pub fn footnote_definition(&mut self, r: NodeId) -> Option<Vec<NodeId>> {
        let Some(label) = self.tree.str(r, "label").map(str::to_owned) else {
            return Some(self.tree.contents(r).to_vec());
        };
        match self.footnote_defs().get(&label).copied() {
            Some(d) => Some(self.tree.contents(d).to_vec()),
            None => {
                self.fail(format!("Definition not found for footnote {label}"));
                None
            }
        }
    }

    /// org-export--footnote-reference-map: the footnote references of
    /// DATA in the order FUNCTION would see them.
    pub fn footnote_references(&mut self, data: &[NodeId], body_first: bool) -> Vec<NodeId> {
        let whole = data == [self.tree.root];
        if whole && let Some(v) = &self.caches.footnote_order[body_first as usize] {
            return v.clone();
        }
        let mut out = vec![];
        let mut seen: Vec<String> = vec![];
        let mut delayed: Vec<Vec<NodeId>> = vec![];
        self.search_ref(data, body_first, &mut out, &mut seen, &mut delayed);
        // (nreverse definitions): encounter order, then searched at once.
        let defs: Vec<NodeId> = delayed.into_iter().flatten().collect();
        let mut none = vec![];
        self.search_ref(&defs, false, &mut out, &mut seen, &mut none);
        if whole {
            self.caches.footnote_order[body_first as usize] = Some(out.clone());
        }
        out
    }

    fn search_ref(
        &mut self,
        data: &[NodeId],
        delay: bool,
        out: &mut Vec<NodeId>,
        seen: &mut Vec<String>,
        delayed: &mut Vec<Vec<NodeId>>,
    ) {
        let nr: &[Type] = if delay {
            &[Type::FootnoteDefinition, Type::FootnoteReference]
        } else {
            &[Type::FootnoteDefinition]
        };
        let refs = self.map(
            data,
            Some(&[Type::FootnoteReference]),
            nr,
            false,
            false,
            |_, f| Some(f),
        );
        for f in refs {
            out.push(f);
            let label = self.tree.str(f, "label").map(str::to_owned);
            if let Some(l) = &label {
                if seen.contains(l) {
                    continue;
                }
                seen.push(l.clone());
            }
            if delay {
                if let Some(d) = self.footnote_definition(f) {
                    delayed.push(d);
                }
            } else if self.tree.prop(f, "type").as_str() == Some("inline") {
            } else if let Some(d) = self.footnote_definition(f) {
                self.search_ref(&d, false, out, seen, delayed);
            }
        }
    }

    /// org-export-collect-footnote-definitions: (number, label,
    /// definition contents).
    pub fn collect_footnote_definitions(
        &mut self,
        data: Option<&[NodeId]>,
        body_first: bool,
    ) -> Vec<(usize, Option<String>, Vec<NodeId>)> {
        let root = [self.tree.root];
        let data = data.map(<[NodeId]>::to_vec).unwrap_or(root.to_vec());
        let mut labels: Vec<String> = vec![];
        let mut out = vec![];
        for f in self.footnote_references(&data, body_first) {
            let l = self.tree.str(f, "label").map(str::to_owned);
            if l.as_ref().is_some_and(|l| labels.contains(l)) {
                continue;
            }
            let def = self.footnote_definition(f).unwrap_or_default();
            out.push((out.len() + 1, l.clone(), def));
            if let Some(l) = l {
                labels.push(l);
            }
        }
        out
    }

    /// org-export-footnote-first-reference-p.
    pub fn footnote_first_reference_p(
        &mut self,
        r: NodeId,
        data: Option<&[NodeId]>,
        body_first: bool,
    ) -> bool {
        let Some(label) = self.tree.str(r, "label").map(str::to_owned) else {
            return true;
        };
        let root = [self.tree.root];
        let data = data.map(<[NodeId]>::to_vec).unwrap_or(root.to_vec());
        for f in self.footnote_references(&data, body_first) {
            if self.tree.str(f, "label") == Some(label.as_str()) {
                return f == r;
            }
        }
        false
    }

    /// org-export-get-footnote-number.
    pub fn footnote_number(
        &mut self,
        f: NodeId,
        data: Option<&[NodeId]>,
        body_first: bool,
    ) -> Option<usize> {
        let label = self.tree.str(f, "label").map(str::to_owned);
        let root = [self.tree.root];
        let data = data.map(<[NodeId]>::to_vec).unwrap_or(root.to_vec());
        let mut count = 0;
        let mut seen: Vec<String> = vec![];
        for r in self.footnote_references(&data, body_first) {
            let l = self.tree.str(r, "label").map(str::to_owned);
            match (&l, &label) {
                (None, None) if r == f => return Some(count + 1),
                (Some(a), Some(b)) if a == b => return Some(count + 1),
                (None, _) => count += 1,
                (Some(a), _) => {
                    if !seen.contains(a) {
                        seen.push(a.clone());
                        count += 1;
                    }
                }
            }
        }
        None
    }

    // ---- headlines ----

    /// org-export-get-relative-level.
    pub fn relative_level(&self, h: NodeId) -> i64 {
        self.tree.int(h, "level").unwrap_or(0) + self.headline_offset
    }

    /// org-export-low-level-p.
    pub fn low_level_p(&self, h: NodeId) -> Option<usize> {
        let limit = self.whole("headline-levels")? as i64;
        let level = self.relative_level(h);
        (level > limit).then(|| (level - limit) as usize)
    }

    /// org-export-get-headline-number.
    pub fn headline_number(&self, h: NodeId) -> Option<Vec<i64>> {
        if !self.numbered_headline_p(h) {
            return None;
        }
        self.numbering.get(&h).cloned()
    }

    /// org-export-numbered-headline-p.
    pub fn numbered_headline_p(&self, h: NodeId) -> bool {
        let un = self.node_property("UNNUMBERED", h, true);
        if un.as_str().is_some_and(|s| s != "nil") || matches!(un, Value::T) {
            return false;
        }
        match self.get("section-numbers") {
            Value::Int(n) if *n >= 0 => self.relative_level(h) <= *n,
            v => v.truthy(),
        }
    }

    /// org-export-get-tags.
    pub fn tags(&self, id: NodeId, remove: &[String], inherited: bool) -> Vec<String> {
        let own = |n: NodeId| -> Vec<String> {
            self.tree
                .prop(n, "tags")
                .as_list()
                .unwrap_or(&[])
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        let mut tags = own(id);
        if inherited {
            for p in self.tree.lineage(id, false) {
                if self.tree.is(p, &[Type::Headline, Type::Inlinetask]) {
                    for t in own(p) {
                        if !tags.contains(&t) {
                            tags.insert(0, t);
                        }
                    }
                }
            }
            let mut all: Vec<String> = self
                .get("filetags")
                .as_list()
                .unwrap_or(&[])
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            all.extend(tags);
            let mut uniq: Vec<String> = vec![];
            for t in all {
                if !uniq.contains(&t) {
                    uniq.push(t);
                }
            }
            tags = uniq;
        }
        tags.retain(|t| !remove.contains(t));
        tags
    }

    /// org-export-get-node-property (PROPERTY upcased, `CUSTOM_ID`).
    pub fn node_property(&self, prop: &str, id: NodeId, inherited: bool) -> Value {
        if !inherited {
            return self.tree.prop(id, prop).clone();
        }
        let Some(h) = self.tree.lineage_of(id, &[Type::Headline], true) else {
            return Value::Nil;
        };
        self.tree
            .property_inherited(&[prop], h, true, false, false, true)
            .into_iter()
            .next()
            .unwrap_or(Value::Nil)
    }

    /// org-export-get-category.
    pub fn category(&self, id: NodeId) -> String {
        if let Some(s) = self.node_property("CATEGORY", id, true).as_str() {
            return s.to_owned();
        }
        if let Some(v) = self.find(&[Type::Keyword], |i, k| {
            (i.tree.str(k, "key") == Some("CATEGORY"))
                .then(|| i.tree.str(k, "value").unwrap_or("").to_owned())
        }) {
            return v;
        }
        if let Some(f) = self.str("input-file") {
            return std::path::Path::new(f)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        "???".into()
    }

    /// org-export-get-alt-title.
    pub fn alt_title(&self, h: NodeId) -> Vec<NodeId> {
        match self.tree.prop(h, "ALT_TITLE") {
            Value::Secondary(s) => s.clone(),
            _ => self.tree.secondary(h, "title").to_vec(),
        }
    }

    /// org-export-first-sibling-p.
    pub fn first_sibling_p(&self, id: NodeId) -> bool {
        self.previous(id)
            .is_none_or(|p| self.tree.kind(p) == Type::Section)
    }

    /// org-export-last-sibling-p.
    pub fn last_sibling_p(&self, id: NodeId) -> bool {
        match self.next(id) {
            None => true,
            Some(n) => {
                self.tree.kind(id) == Type::Headline
                    && self.tree.int(id, "level") > self.tree.int(n, "level")
            }
        }
    }

    /// org-export-get-date: the date as objects (FMT applies to a single
    /// timestamp, as a string).
    pub fn date(&mut self, fmt: Option<&str>) -> Value {
        let date = self.get("date").clone();
        let fmt = fmt.map(str::to_owned).or_else(|| {
            match super::env::var("org-export-date-timestamp-format") {
                Some(crate::org::sexp::Sexp::Str(s)) => Some(s),
                _ => None,
            }
        });
        if let (Some(fmt), Value::Secondary(s)) = (&fmt, &date)
            && s.len() == 1
            && self.tree.kind(s[0]) == Type::Timestamp
        {
            return Value::Str(super::pipeline::format_timestamp(
                &self.tree, s[0], fmt, false,
            ));
        }
        date
    }

    // ---- links ----

    /// org-export-custom-protocol-maybe: Fred link types have no export
    /// functions, so links always go through the backend.
    pub fn custom_protocol_maybe(
        &mut self,
        _link: NodeId,
        _desc: Option<&str>,
        _backend: &str,
    ) -> Option<String> {
        None
    }

    /// org-export-inline-image-p (RULES: (type, case-insensitive regexp)).
    pub fn inline_image_p(&self, link: NodeId, rules: Option<&[(&str, &str)]>) -> bool {
        if !self.tree.contents(link).is_empty() {
            return false;
        }
        let rules = rules.unwrap_or(DEFAULT_IMAGE_RULE);
        let ty = self.tree.str(link, "type").unwrap_or("");
        let path = self.tree.str(link, "path").unwrap_or("");
        rules.iter().any(|(t, re)| {
            *t == ty
                && regex::RegexBuilder::new(re)
                    .case_insensitive(true)
                    .build()
                    .is_ok_and(|r| r.is_match(path))
        })
    }

    /// org-export-resolve-coderef: the line number (or the label when
    /// the block keeps labels).  Err: the broken reference.
    pub fn resolve_coderef(&mut self, r: &str) -> Result<String, String> {
        let root = self.tree.root;
        let blocks = self.map(
            &[root],
            Some(&[Type::ExampleBlock, Type::SrcBlock]),
            &[],
            false,
            false,
            |_, b| Some(b),
        );
        for el in blocks {
            let value = self.tree.str(el, "value").unwrap_or("").trim().to_owned();
            let fmt = self
                .tree
                .str(el, "label-fmt")
                .map(str::to_owned)
                .unwrap_or_else(|| options::string("org-coderef-label-format", "(ref:%s)"));
            let re = coderef_regexp(&fmt, Some(r));
            // re-search-backward: the last matching line.
            let lines: Vec<&str> = value.split('\n').collect();
            if let Some(i) = lines.iter().rposition(|l| re.is_match(l)) {
                if self.tree.flag(el, "use-labels") {
                    return Ok(r.to_owned());
                }
                let loc = self.loc(el).unwrap_or(0);
                return Ok((loc + i as i64 + 1).to_string());
            }
        }
        Err(r.to_owned())
    }

    /// org-export-search-cells.
    pub fn search_cells(&self, id: NodeId) -> Vec<Cell> {
        let t = &self.tree;
        match t.kind(id) {
            Type::Headline => {
                static COOKIE: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r"\[[0-9]*(?:%|/[0-9]*)\]").unwrap());
                let raw = t.str(id, "raw-value").unwrap_or("");
                let title: Vec<String> = COOKIE
                    .replace_all(raw, " ")
                    .split_whitespace()
                    .map(str::to_uppercase)
                    .collect();
                let mut v = vec![Cell::Headline(title.clone()), Cell::Other(title)];
                if let Some(c) = t.str(id, "CUSTOM_ID") {
                    v.push(Cell::CustomId(c.to_owned()));
                }
                v
            }
            Type::Target => vec![Cell::Target(
                t.str(id, "value")
                    .unwrap_or("")
                    .split_whitespace()
                    .map(str::to_uppercase)
                    .collect(),
            )],
            _ => {
                let name =
                    t.str(id, "name")
                        .map(str::to_owned)
                        .or_else(|| match t.prop(id, "results") {
                            Value::List(l) => l.first().and_then(|v| v.as_str().map(str::to_owned)),
                            Value::Cons(a, _) => a.as_str().map(str::to_owned),
                            Value::Str(s) => Some(s.clone()),
                            _ => None,
                        });
                match name {
                    Some(n) => vec![Cell::Other(
                        n.split_whitespace().map(str::to_owned).collect(),
                    )],
                    None => vec![],
                }
            }
        }
    }

    /// org-export-match-search-cell-p.
    pub fn match_search_cell_p(&self, id: NodeId, cells: &[Cell]) -> bool {
        let targets = self.search_cells(id);
        cells.iter().any(|c| targets.contains(c))
    }

    /// org-export-resolve-fuzzy-link (PSEUDO types also searched).
    pub fn resolve_fuzzy_link(&mut self, link: NodeId) -> Result<NodeId, String> {
        let path = self.tree.str(link, "path").unwrap_or("").to_owned();
        let cells = string_to_search_cell(&path);
        if self.caches.fuzzy_cells.is_none() {
            let mut types = vec![Type::Target];
            types.extend_from_slice(element::ALL_ELEMENTS);
            let root = self.tree.root;
            let all = self.map(&[root], Some(&types), &[], false, false, |i, d| {
                Some((d, i.search_cells(d)))
            });
            let mut table: HashMap<Cell, Vec<NodeId>> = HashMap::new();
            for (d, cs) in all {
                for c in cs {
                    table.entry(c).or_default().insert(0, d);
                }
            }
            self.caches.fuzzy_cells = Some(table);
        }
        if let Some(c) = self.caches.fuzzy.get(&cells) {
            return c.ok_or(path);
        }
        let table = self.caches.fuzzy_cells.as_ref().unwrap();
        let matches: Vec<NodeId> = cells
            .iter()
            .flat_map(|c| table.get(c).cloned().unwrap_or_default())
            .collect();
        if matches.is_empty() {
            return Err(path);
        }
        let r = matches
            .iter()
            .copied()
            .find(|&d| self.tree.kind(d) != Type::Headline)
            .unwrap_or(matches[0]);
        self.caches.fuzzy.insert(cells, Some(r));
        Ok(r)
    }

    /// org-export-resolve-id-link: a headline of the tree, or the file
    /// holding the ID.
    pub fn resolve_id_link(&mut self, link: NodeId) -> Result<Dest, String> {
        let id = self.tree.str(link, "path").unwrap_or("").to_owned();
        if self.caches.ids.is_none() {
            let root = self.tree.root;
            let hs = self.map(
                &[root],
                Some(&[Type::Headline]),
                &[],
                false,
                false,
                |i, h| {
                    Some((
                        i.tree.str(h, "ID").map(str::to_owned),
                        i.tree.str(h, "CUSTOM_ID").map(str::to_owned),
                        h,
                    ))
                },
            );
            let mut m = HashMap::new();
            for (a, b, h) in hs {
                for k in [a, b].into_iter().flatten() {
                    m.entry(k).or_insert(h);
                }
            }
            self.caches.ids = Some(m);
        }
        if let Some(&h) = self.caches.ids.as_ref().unwrap().get(&id) {
            return Ok(Dest::Node(h));
        }
        if let Some((_, f)) = self.id_alist.iter().find(|(k, _)| *k == id) {
            return Ok(Dest::File(f.clone()));
        }
        Err(id)
    }

    /// org-export-resolve-radio-link.
    pub fn resolve_radio_link(&self, link: NodeId) -> Option<NodeId> {
        let clean = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let path = clean(self.tree.str(link, "path").unwrap_or(""));
        self.find(&[Type::RadioTarget], |i, r| {
            clean(i.tree.str(r, "value").unwrap_or(""))
                .eq_ignore_ascii_case(&path)
                .then_some(r)
        })
    }

    /// org-export-resolve-link on a link string (`*Heading`, `#id`,
    /// `name`, `id:x`).
    pub fn resolve_link_string(&mut self, s: &str) -> Result<Dest, String> {
        let t = element::parse_secondary_string(&format!("[[{s}]]"), &[Type::Link], &self.settings);
        let Some(&l) = t.contents(t.root).first() else {
            return Err(s.to_owned());
        };
        let link = super::graft(&mut self.tree, &t, l);
        self.resolve_link(link)
    }

    /// org-export-resolve-link.
    pub fn resolve_link(&mut self, link: NodeId) -> Result<Dest, String> {
        match self.tree.str(link, "type") {
            Some("custom-id" | "id") => self.resolve_id_link(link),
            Some("fuzzy") => self.resolve_fuzzy_link(link).map(Dest::Node),
            _ => Err(self.tree.str(link, "path").unwrap_or("").to_owned()),
        }
    }

    /// Record a broken link for org-export-data's handler.
    pub fn signal_broken(&mut self, path: String) {
        self.broken = Some(path);
    }

    // ---- references ----

    /// org-export-new-reference: a fresh random number.
    fn new_reference(&mut self) -> u32 {
        loop {
            // xorshift seeded from the clock (Emacs `random`).
            if self.caches.seed == 0 {
                self.caches.seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0x2545_f491, |d| d.as_nanos() as u64)
                    | 1;
            }
            let mut x = self.caches.seed;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.caches.seed = x;
            let n = (x % 0x1000_0000) as u32;
            if !self.ref_cells.iter().any(|(_, v)| *v == n)
                && !self.refs.iter().any(|(r, _)| *r == format_reference(n))
            {
                return n;
            }
        }
    }

    /// org-export-get-reference.
    pub fn reference(&mut self, id: NodeId) -> String {
        if let Some((r, _)) = self.refs.iter().find(|(_, d)| *d == id) {
            return r.clone();
        }
        let cells = self.search_cells(id);
        let new = self.new_reference();
        for c in cells {
            self.ref_cells.push((c, new));
        }
        let r = format_reference(new);
        self.refs.push((r.clone(), id));
        r
    }

    /// org-export-get-ordinal: a headline or item number (as a list), a
    /// footnote number, or the sequence number of ID among nodes of its
    /// type (and TYPES) satisfying PREDICATE.
    pub fn ordinal(
        &mut self,
        id: NodeId,
        types: &[Type],
        pred: Option<fn(&Info, NodeId) -> bool>,
    ) -> Option<Ordinal> {
        let id = if self.tree.kind(id) == Type::Target {
            self.tree.lineage_of(
                id,
                &[
                    Type::FootnoteDefinition,
                    Type::FootnoteReference,
                    Type::Headline,
                    Type::Item,
                    Type::Table,
                ],
                false,
            )?
        } else {
            id
        };
        match self.tree.kind(id) {
            Type::Headline => self.headline_number(id).map(Ordinal::List),
            Type::Item => Some(Ordinal::List(item_number(&self.tree, id))),
            Type::FootnoteDefinition | Type::FootnoteReference => self
                .footnote_number(id, None, false)
                .map(|n| Ordinal::Num(n as i64)),
            k => {
                let mut all = vec![k];
                all.extend_from_slice(types);
                let root = self.tree.root;
                let mut counter = 0;
                self.map(&[root], Some(&all), &[], false, true, |i, el| {
                    if el == id {
                        return pred.is_none_or(|p| p(i, el)).then_some(counter + 1);
                    }
                    if pred.is_none_or(|p| p(i, el)) {
                        counter += 1;
                    }
                    None
                })
                .into_iter()
                .next()
                .map(Ordinal::Num)
            }
        }
    }

    // ---- raw objects ----

    /// org-export-raw-string.
    pub fn raw_string(&mut self, s: &str) -> NodeId {
        let id = self.tree.create(Type::Pseudo("raw"), vec![], vec![]);
        self.tree.node_mut(id).text = s.to_owned();
        id
    }

    // ---- src blocks ----

    /// org-export-get-loc.
    pub fn loc(&self, el: NodeId) -> Option<i64> {
        let (kind, n) = number_lines(&self.tree, el)?;
        if kind == "new" {
            return Some(n);
        }
        let root = self.tree.root;
        let mut loc = 0;
        self.map(
            &[root],
            Some(&[Type::SrcBlock, Type::ExampleBlock]),
            &[],
            false,
            true,
            |i, b| {
                if b == el {
                    return Some(loc + n);
                }
                if let Some((k, m)) = number_lines(&i.tree, b) {
                    let lines = count_lines(i.tree.str(b, "value").unwrap_or("")) as i64;
                    if k == "new" {
                        loc = m + lines;
                    } else {
                        loc += m + lines;
                    }
                }
                None
            },
        )
        .into_iter()
        .next()
    }

    /// org-export-format-code-default.
    pub fn format_code_default(&self, el: NodeId) -> String {
        let (code, refs) = unravel_code(&self.tree, el);
        let lines: Vec<&str> = code.split('\n').collect();
        let refs = if self.tree.flag(el, "retain-labels") {
            refs
        } else {
            vec![]
        };
        let num_start = self.loc(el);
        let num_fmt_width = num_start.map(|s| (lines.len() as i64 + s).to_string().len());
        let max_width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0)
            + match (num_start, num_fmt_width) {
                (Some(s), Some(w)) => format!("{:>w$}  ", s).chars().count(),
                _ => 0,
            };
        format_code(&code, num_start, &refs, |loc, num, r| {
            let number = match (num, num_fmt_width) {
                (Some(n), Some(w)) => format!("{n:>w$}  "),
                _ => String::new(),
            };
            let mut s = format!("{number}{loc}");
            if let Some(r) = r {
                let pad =
                    (6 + max_width).saturating_sub(loc.chars().count() + number.chars().count());
                s.push_str(&" ".repeat(pad));
                s.push_str(&format!("({r})"));
            }
            s
        })
    }

    // ---- tables of contents ----

    /// org-export-collect-headlines.
    pub fn collect_headlines(&self, n: Option<usize>, scope: Option<NodeId>) -> Vec<NodeId> {
        let t = &self.tree;
        let scope = match scope {
            None => t.root,
            Some(s) if t.kind(s) == Type::Headline => s,
            Some(s) => t.lineage_of(s, &[Type::Headline], false).unwrap_or(t.root),
        };
        let limit = self.int("headline-levels").unwrap_or(0);
        let n = match n {
            None => limit,
            Some(n) => (if t.kind(scope) == Type::OrgData {
                n as i64
            } else {
                self.relative_level(scope) + n as i64
            })
            .min(limit),
        };
        let kids = t.contents(scope).to_vec();
        self.map(&kids, Some(&[Type::Headline]), &[], false, false, |i, h| {
            (!i.excluded_from_toc_p(h) && n >= i.relative_level(h)).then_some(h)
        })
    }

    /// org-export-collect-elements: elements of TYPES with a caption.
    pub fn collect_elements(&self, types: &[Type], pred: Option<Pred<'_>>) -> Vec<NodeId> {
        let root = self.tree.root;
        self.map(&[root], Some(types), &[], false, false, |i, e| {
            (i.tree.prop(e, "caption").truthy() && pred.is_none_or(|p| p(i, e))).then_some(e)
        })
    }

    /// org-export-collect-tables.
    pub fn collect_tables(&self) -> Vec<NodeId> {
        self.collect_elements(&[Type::Table], None)
    }

    /// org-export-collect-figures.
    pub fn collect_figures(&self, pred: &dyn Fn(&Info, NodeId) -> bool) -> Vec<NodeId> {
        self.collect_elements(&[Type::Paragraph], Some(pred))
    }

    /// org-export-collect-listings.
    pub fn collect_listings(&self) -> Vec<NodeId> {
        self.collect_elements(&[Type::SrcBlock], None)
    }

    /// org-export-excluded-from-toc-p.
    pub fn excluded_from_toc_p(&self, h: NodeId) -> bool {
        self.tree.flag(h, "footnote-section-p")
            || self.low_level_p(h).is_some()
            || self.node_property("UNNUMBERED", h, true).as_str() == Some("notoc")
            || self
                .whole("with-toc")
                .is_some_and(|d| self.relative_level(h) > d as i64)
    }

    // ---- topology ----

    /// The list (contents or secondary string) holding ID.
    fn siblings(&self, id: NodeId) -> Vec<NodeId> {
        let Some(p) = self.tree.parent(id) else {
            return vec![];
        };
        match self.tree.secondary_p(id) {
            Some(k) => self.tree.secondary(p, k).to_vec(),
            None => self.tree.contents(p).to_vec(),
        }
    }

    /// org-export-get-previous-element.
    pub fn previous(&self, id: NodeId) -> Option<NodeId> {
        let s = self.siblings(id);
        let i = s.iter().position(|&x| x == id)?;
        s[..i]
            .iter()
            .rev()
            .copied()
            .find(|x| !self.ignore.contains(x))
    }

    /// org-export-get-previous-element with N: up to N siblings before
    /// ID (all with None), farthest first.
    pub fn previous_n(&self, id: NodeId, n: Option<usize>) -> Vec<NodeId> {
        let s = self.siblings(id);
        let Some(i) = s.iter().position(|&x| x == id) else {
            return vec![];
        };
        let mut v: Vec<NodeId> = s[..i]
            .iter()
            .rev()
            .copied()
            .filter(|x| !self.ignore.contains(x))
            .collect();
        if let Some(n) = n {
            v.truncate(n);
        }
        v.reverse();
        v
    }

    /// org-export-get-next-element.
    pub fn next(&self, id: NodeId) -> Option<NodeId> {
        let s = self.siblings(id);
        let i = s.iter().position(|&x| x == id)?;
        s[i + 1..]
            .iter()
            .copied()
            .find(|x| !self.ignore.contains(x))
    }

    /// org-export-get-next-element with N, closest first.
    pub fn next_n(&self, id: NodeId, n: Option<usize>) -> Vec<NodeId> {
        let s = self.siblings(id);
        let Some(i) = s.iter().position(|&x| x == id) else {
            return vec![];
        };
        let mut v: Vec<NodeId> = s[i + 1..]
            .iter()
            .copied()
            .filter(|x| !self.ignore.contains(x))
            .collect();
        if let Some(n) = n {
            v.truncate(n);
        }
        v
    }

    /// org-export-get-parent-headline.
    pub fn parent_headline(&self, id: NodeId) -> Option<NodeId> {
        self.tree.lineage_of(id, &[Type::Headline], false)
    }

    /// org-export-get-parent-table.
    pub fn parent_table(&self, id: NodeId) -> Option<NodeId> {
        self.tree.lineage_of(id, &[Type::Table], false)
    }

    /// The previous sibling's kind (None when there is none).
    pub fn previous_kind(&self, id: NodeId) -> Option<Type> {
        self.previous(id).map(|p| self.tree.kind(p))
    }
}

/// The value of org-export-get-ordinal.
#[derive(Clone, Debug, PartialEq)]
pub enum Ordinal {
    Num(i64),
    List(Vec<i64>),
}

impl Ordinal {
    /// `number-to-string` or the numbers joined by dots.
    pub fn dotted(&self) -> String {
        match self {
            Ordinal::Num(n) => n.to_string(),
            Ordinal::List(l) => l.iter().map(i64::to_string).collect::<Vec<_>>().join("."),
        }
    }
}

/// org-export-default-inline-image-rule.
pub const DEFAULT_IMAGE_RULE: &[(&str, &str)] = &[(
    "file",
    r"\.(gif|jp(?:e?g)|p(?:bm|gm|ng|pm)|tiff?|x(?:bm|pm))$",
)];

/// org-export-format-reference.
pub fn format_reference(n: u32) -> String {
    format!("org{n:07x}")
}

/// org-export-number-to-roman.
pub fn number_to_roman(mut n: i64) -> String {
    if n <= 0 {
        return n.to_string();
    }
    const R: &[(i64, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for &(v, r) in R {
        while n >= v {
            n -= v;
            s.push_str(r);
        }
    }
    s
}

/// org-export-get-coderef-format.
pub fn coderef_format(path: &str, desc: Option<&str>) -> String {
    match desc {
        None => "%s".into(),
        Some(d) => {
            let needle = format!("({path})");
            match d.find(&needle) {
                Some(i) => format!("{}%s{}", &d[..i], &d[i + needle.len()..]),
                None => d.to_owned(),
            }
        }
    }
}

/// org-src-coderef-regexp: group 1 the coderef with blanks, 2 without,
/// 3 the label.
pub fn coderef_regexp(fmt: &str, label: Option<&str>) -> Regex {
    let label_re = match label {
        Some(l) => format!("({})", regex::escape(l)),
        None => "([-a-zA-Z0-9_][-a-zA-Z0-9_ ]*)".into(),
    };
    let body = regex::escape(fmt).replace("%s", &label_re);
    Regex::new(&format!(r"([ \t]*({body})[ \t]*)$")).unwrap()
}

/// org-export-string-to-search-cell.
pub fn string_to_search_cell(s: &str) -> Vec<Cell> {
    if let Some(r) = s.strip_prefix('*') {
        return vec![Cell::Headline(
            r.split_whitespace().map(str::to_uppercase).collect(),
        )];
    }
    if let Some(r) = s.strip_prefix('#') {
        return vec![Cell::CustomId(r.to_owned())];
    }
    let search: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
    let up: Vec<String> = search.iter().map(|w| w.to_uppercase()).collect();
    let mut v = vec![
        Cell::Target(search.clone()),
        Cell::Other(search),
        Cell::Target(up.clone()),
        Cell::Other(up),
    ];
    let mut out: Vec<Cell> = vec![];
    for c in v.drain(..) {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// org-export-file-uri.
pub fn file_uri(f: &str) -> String {
    if f.starts_with("//") {
        return format!("file:{f}");
    }
    if !(f.starts_with('/') || f.starts_with('~')) {
        return f.to_owned();
    }
    let full = options::expand(f).to_string_lossy().into_owned();
    if full.starts_with('/') {
        format!("file://{full}")
    } else {
        format!("file:///{full}")
    }
}

/// org-export-link-remote-p.
pub fn link_remote_p(t: &Tree, link: NodeId) -> bool {
    let ty = t.str(link, "type").unwrap_or("");
    matches!(ty, "http" | "https" | "ftp")
        || (ty == "file"
            && t.str(link, "path")
                .is_some_and(|p| p.starts_with("/ssh:") || p.contains("://")))
}

/// org-export-file-relative-name-maybe.
pub fn file_relative_name_maybe(f: &str, base: Option<&str>) -> String {
    match base {
        Some(b) if f.starts_with('/') => {
            let b = b.trim_end_matches('/');
            match f.strip_prefix(&format!("{b}/")) {
                Some(r) => r.to_owned(),
                None => f.to_owned(),
            }
        }
        _ => f.to_owned(),
    }
}

/// `:number-lines` as (kind, start): `(new . 0)`.
fn number_lines(t: &Tree, el: NodeId) -> Option<(String, i64)> {
    match t.prop(el, "number-lines") {
        Value::Cons(a, b) => Some((a.as_str()?.to_owned(), b.as_int().unwrap_or(0))),
        _ => None,
    }
}

/// org-count-lines: lines of S, a final line without newline counting.
pub fn count_lines(s: &str) -> usize {
    let n = s.matches('\n').count();
    if s.is_empty() || s.ends_with('\n') {
        n
    } else {
        n + 1
    }
}

/// org-src-preserve-indentation-p.
pub fn preserve_indentation_p(t: &Tree, el: NodeId) -> bool {
    options::bool("org-src-preserve-indentation", false) || t.flag(el, "preserve-indent")
}

/// org-export-unravel-code: the code without coderefs and global
/// indentation, and (line, label) pairs.
pub fn unravel_code(t: &Tree, el: NodeId) -> (String, Vec<(usize, String)>) {
    let value = t.str(el, "value").unwrap_or("");
    let code = if preserve_indentation_p(t, el) {
        value.to_owned()
    } else {
        element::remove_indentation(value)
    };
    let code = code.strip_suffix('\n').unwrap_or(&code).to_owned();
    let fmt = t
        .str(el, "label-fmt")
        .map(str::to_owned)
        .unwrap_or_else(|| options::string("org-coderef-label-format", "(ref:%s)"));
    let re = coderef_regexp(&fmt, None);
    let mut refs = vec![];
    let lines: Vec<String> = code
        .split('\n')
        .enumerate()
        .map(|(i, loc)| match re.captures(loc) {
            None => loc.to_owned(),
            Some(c) => {
                refs.insert(0, (i + 1, c[3].to_owned()));
                let g = c.get(1).unwrap();
                format!("{}{}", &loc[..g.start()], &loc[g.end()..])
            }
        })
        .collect();
    (lines.join("\n"), refs)
}

/// org-export-format-code: FUN on each line with its number (from
/// NUM_LINES) and coderef.
pub fn format_code(
    code: &str,
    num_lines: Option<i64>,
    refs: &[(usize, String)],
    mut fun: impl FnMut(&str, Option<i64>, Option<&str>) -> String,
) -> String {
    let mut out: Vec<String> = vec![];
    for (i, loc) in code.split('\n').enumerate() {
        let line = i + 1;
        let r = refs
            .iter()
            .find(|(n, _)| *n == line)
            .map(|(_, l)| l.as_str());
        out.push(fun(loc, num_lines.map(|n| n + line as i64), r));
    }
    format!("{}\n", out.join("\n"))
}

/// org-list-get-item-number for an item node (from its :structure).
pub fn item_number(t: &Tree, item: NodeId) -> Vec<i64> {
    use crate::org::list::structure::{Item, Struct};
    let items: Vec<Item> = t
        .prop(item, "structure")
        .as_list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| {
            let l = v.as_list()?;
            Some(Item {
                pos: l.first()?.as_int()? as usize,
                ind: l.get(1)?.as_int()? as usize,
                bullet: l.get(2)?.as_str()?.to_owned(),
                counter: l.get(3).and_then(|v| v.as_str().map(str::to_owned)),
                checkbox: l.get(4).and_then(|v| v.as_str().map(str::to_owned)),
                tag: l.get(5).and_then(|v| v.as_str().map(str::to_owned)),
                end: l.get(6)?.as_int()? as usize,
            })
        })
        .collect();
    let st = Struct(items);
    let Some(pos) = t.begin(item) else {
        return vec![1];
    };
    if st.get(pos).is_none() {
        return vec![1];
    }
    st.item_number(pos, &st.prevs(), &st.parents())
}

/// org-export-toc-entry-backend: PARENT without footnote references and
/// targets, links and radio targets as plain text.
pub fn toc_entry_backend(parent: &'static str, extra: Vec<(&'static str, Tx)>) -> Rc<Backend> {
    let mut transcoders = extra;
    transcoders.extend([
        ("footnote-reference", Tx::Node(|_, _, _| None)),
        ("link", Tx::Node(toc_default_link_transcoder)),
        ("radio-target", Tx::Node(|_, _, c| c)),
        ("target", Tx::Node(|_, _, _| None)),
    ]);
    Rc::new(Backend {
        name: None,
        parent: Some(parent),
        transcoders,
        ..Backend::default()
    })
}

/// org-export-toc-default-link-transcoder.
pub fn toc_default_link_transcoder(
    info: &mut Info,
    link: NodeId,
    contents: Option<String>,
) -> Option<String> {
    contents.or_else(|| {
        let raw = info.tree.str(link, "raw-link").unwrap_or("").to_owned();
        Some(info.data_str(&raw))
    })
}

/// org-trim.
pub fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r'])
}

/// org-string-nw-p.
pub fn nw(s: &str) -> bool {
    s.chars().any(|c| !matches!(c, ' ' | '\t' | '\n' | '\r'))
}

/// org-remove-blank-lines.
pub fn remove_blank_lines(s: &str) -> String {
    static BLANK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n[ \t]*\n").unwrap());
    let mut s = s.to_owned();
    while BLANK.is_match(&s) {
        s = BLANK.replace_all(&s, "\n").into_owned();
    }
    s
}
