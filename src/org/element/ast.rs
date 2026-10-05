//! org-element-ast.el: syntax nodes, properties and tree surgery.
//!
//! Nodes live in an arena ([`Tree`]) and are addressed by [`NodeId`].
//! Every node has a [`Type`], a property list (upstream plist, keys
//! without the leading colon), contents and a parent.  Plain text is a
//! node of type [`Type::PlainText`] whose string is [`Node::text`];
//! secondary strings (headline titles, item tags, citation prefixes,
//! parsed affiliated keywords) are [`Value::Secondary`] lists whose
//! members have the owning node as parent, as upstream.
//!
//! Upstream deferred values (`org-element-deferred-*`) are resolved at
//! parse time: Fred keeps the text, so nothing needs to be deferred.

/// Index of a node in its [`Tree`].
pub type NodeId = usize;

macro_rules! types {
    ($($v:ident => $n:literal,)*) => {
        /// Element and object types (org-element-all-elements,
        /// org-element-all-objects), plus `org-data`, `plain-text`,
        /// `anonymous` (a secondary string) and pseudo types.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Type {
            $($v,)*
            /// A pseudo element or object created by a consumer
            /// (exporters), classified like upstream `org-element-class`.
            Pseudo(&'static str),
        }

        impl Type {
            /// The upstream symbol name.
            pub fn name(self) -> &'static str {
                match self {
                    $(Type::$v => $n,)*
                    Type::Pseudo(n) => n,
                }
            }

            /// A type by its upstream symbol name (pseudo types excluded).
            pub fn from_name(s: &str) -> Option<Type> {
                match s {
                    $($n => Some(Type::$v),)*
                    _ => None,
                }
            }
        }
    };
}

types! {
    OrgData => "org-data",
    BabelCall => "babel-call",
    CenterBlock => "center-block",
    Clock => "clock",
    Comment => "comment",
    CommentBlock => "comment-block",
    DiarySexp => "diary-sexp",
    Drawer => "drawer",
    DynamicBlock => "dynamic-block",
    ExampleBlock => "example-block",
    ExportBlock => "export-block",
    FixedWidth => "fixed-width",
    FootnoteDefinition => "footnote-definition",
    Headline => "headline",
    HorizontalRule => "horizontal-rule",
    Inlinetask => "inlinetask",
    Item => "item",
    Keyword => "keyword",
    LatexEnvironment => "latex-environment",
    NodeProperty => "node-property",
    Paragraph => "paragraph",
    PlainList => "plain-list",
    Planning => "planning",
    PropertyDrawer => "property-drawer",
    QuoteBlock => "quote-block",
    Section => "section",
    SpecialBlock => "special-block",
    SrcBlock => "src-block",
    Table => "table",
    TableRow => "table-row",
    VerseBlock => "verse-block",
    Bold => "bold",
    Citation => "citation",
    CitationReference => "citation-reference",
    Code => "code",
    Entity => "entity",
    ExportSnippet => "export-snippet",
    FootnoteReference => "footnote-reference",
    InlineBabelCall => "inline-babel-call",
    InlineSrcBlock => "inline-src-block",
    Italic => "italic",
    LineBreak => "line-break",
    LatexFragment => "latex-fragment",
    Link => "link",
    Macro => "macro",
    RadioTarget => "radio-target",
    StatisticsCookie => "statistics-cookie",
    StrikeThrough => "strike-through",
    Subscript => "subscript",
    Superscript => "superscript",
    TableCell => "table-cell",
    Target => "target",
    Timestamp => "timestamp",
    Underline => "underline",
    Verbatim => "verbatim",
    PlainText => "plain-text",
    Anonymous => "anonymous",
}

use Type as T;

/// org-element-all-elements.
pub const ALL_ELEMENTS: &[Type] = &[
    T::BabelCall,
    T::CenterBlock,
    T::Clock,
    T::Comment,
    T::CommentBlock,
    T::DiarySexp,
    T::Drawer,
    T::DynamicBlock,
    T::ExampleBlock,
    T::ExportBlock,
    T::FixedWidth,
    T::FootnoteDefinition,
    T::Headline,
    T::HorizontalRule,
    T::Inlinetask,
    T::Item,
    T::Keyword,
    T::LatexEnvironment,
    T::NodeProperty,
    T::Paragraph,
    T::PlainList,
    T::Planning,
    T::PropertyDrawer,
    T::QuoteBlock,
    T::Section,
    T::SpecialBlock,
    T::SrcBlock,
    T::Table,
    T::TableRow,
    T::VerseBlock,
];

/// org-element-greater-elements.
pub const GREATER_ELEMENTS: &[Type] = &[
    T::CenterBlock,
    T::Drawer,
    T::DynamicBlock,
    T::FootnoteDefinition,
    T::Headline,
    T::Inlinetask,
    T::Item,
    T::PlainList,
    T::PropertyDrawer,
    T::QuoteBlock,
    T::Section,
    T::SpecialBlock,
    T::Table,
    T::OrgData,
];

/// org-element-all-objects.
pub const ALL_OBJECTS: &[Type] = &[
    T::Bold,
    T::Citation,
    T::CitationReference,
    T::Code,
    T::Entity,
    T::ExportSnippet,
    T::FootnoteReference,
    T::InlineBabelCall,
    T::InlineSrcBlock,
    T::Italic,
    T::LineBreak,
    T::LatexFragment,
    T::Link,
    T::Macro,
    T::RadioTarget,
    T::StatisticsCookie,
    T::StrikeThrough,
    T::Subscript,
    T::Superscript,
    T::TableCell,
    T::Target,
    T::Timestamp,
    T::Underline,
    T::Verbatim,
];

/// org-element-recursive-objects.
pub const RECURSIVE_OBJECTS: &[Type] = &[
    T::Bold,
    T::Citation,
    T::FootnoteReference,
    T::Italic,
    T::Link,
    T::Subscript,
    T::RadioTarget,
    T::StrikeThrough,
    T::Superscript,
    T::TableCell,
    T::Underline,
];

/// org-element-object-containers.
pub const OBJECT_CONTAINERS: &[Type] = &[
    T::Bold,
    T::Citation,
    T::FootnoteReference,
    T::Italic,
    T::Link,
    T::Subscript,
    T::RadioTarget,
    T::StrikeThrough,
    T::Superscript,
    T::TableCell,
    T::Underline,
    T::Paragraph,
    T::TableRow,
    T::VerseBlock,
];

/// org-element-elements-no-affiliated.
pub const NO_AFFILIATED: &[Type] = &[
    T::OrgData,
    T::Comment,
    T::Clock,
    T::Headline,
    T::Inlinetask,
    T::Item,
    T::NodeProperty,
    T::Planning,
    T::PropertyDrawer,
    T::Section,
    T::TableRow,
];

impl Type {
    pub fn is_element(self) -> bool {
        ALL_ELEMENTS.contains(&self)
    }
    pub fn is_object(self) -> bool {
        ALL_OBJECTS.contains(&self)
    }
    pub fn is_greater(self) -> bool {
        GREATER_ELEMENTS.contains(&self)
    }
    pub fn is_recursive(self) -> bool {
        RECURSIVE_OBJECTS.contains(&self)
    }
    /// org-element-secondary-value-alist: properties holding secondary
    /// strings.
    pub fn secondary(self) -> &'static [&'static str] {
        match self {
            T::Citation | T::CitationReference => &["prefix", "suffix"],
            T::Headline | T::Inlinetask => &["title"],
            T::Item => &["tag"],
            _ => &[],
        }
    }
}

/// A property value (an upstream plist value).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Nil,
    T,
    Int(i64),
    Str(String),
    /// A symbol such as `ordered`, `on`, `active-range`.
    Sym(String),
    List(Vec<Value>),
    /// A cons cell: dual affiliated keywords `(VALUE . SECONDARY)`,
    /// `:number-lines` `(new . 0)`.
    Cons(Box<Value>, Box<Value>),
    /// A node (e.g. a planning timestamp).
    Node(NodeId),
    /// A secondary string: objects and plain text.
    Secondary(Vec<NodeId>),
}

impl Value {
    /// Nil, or an empty list or secondary string.
    pub fn is_nil(&self) -> bool {
        match self {
            Value::Nil => true,
            Value::List(v) => v.is_empty(),
            Value::Secondary(v) => v.is_empty(),
            _ => false,
        }
    }
    pub fn truthy(&self) -> bool {
        !self.is_nil()
    }
    /// A string or symbol name.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Sym(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_node(&self) -> Option<NodeId> {
        match self {
            Value::Node(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_secondary(&self) -> Option<&[NodeId]> {
        match self {
            Value::Secondary(v) => Some(v),
            _ => None,
        }
    }
    pub fn sym(s: &str) -> Value {
        Value::Sym(s.to_owned())
    }
    pub fn opt_str(s: Option<impl Into<String>>) -> Value {
        s.map_or(Value::Nil, |s| Value::Str(s.into()))
    }
    pub fn opt_int(n: Option<usize>) -> Value {
        n.map_or(Value::Nil, |n| Value::Int(n as i64))
    }
    pub fn cons(a: Value, b: Value) -> Value {
        Value::Cons(Box::new(a), Box::new(b))
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.to_owned())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Value {
        Value::Int(n)
    }
}
impl From<usize> for Value {
    fn from(n: usize) -> Value {
        Value::Int(n as i64)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Value {
        if b { Value::T } else { Value::Nil }
    }
}

static NIL: Value = Value::Nil;

/// A syntax node.
#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Type,
    /// Properties, in insertion order (affiliated keywords are
    /// interpreted in this order).
    pub props: Vec<(String, Value)>,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    /// The string of a plain-text node.
    pub text: String,
}

/// Element or object (org-element-class).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Element,
    Object,
}

/// A syntax tree: an arena of nodes and its root.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: NodeId,
}

fn key(k: &str) -> &str {
    k.strip_prefix(':').unwrap_or(k)
}

/// What a map function asks for (org-element-map's FUN results).
pub enum Visit<T> {
    /// Collect this value and recurse.
    Value(T),
    /// Collect nothing and recurse (FUN returned nil).
    Nil,
    /// Collect nothing and do not recurse (`:org-element-skip`).
    Skip,
}

/// Options of [`Tree::ast_map`] (org-element-ast-map).
#[derive(Default, Clone)]
pub struct MapOpts<'a> {
    pub ignore: &'a [NodeId],
    pub first_match: bool,
    pub no_recursion: Vec<Type>,
    /// Extra properties walked like secondary strings.
    pub with_properties: Vec<&'static str>,
    pub no_secondary: bool,
}

impl Tree {
    pub fn new() -> Tree {
        Tree::default()
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    /// org-element-create: a node of KIND with PROPS and CHILDREN.
    pub fn create(
        &mut self,
        kind: Type,
        props: Vec<(&str, Value)>,
        children: Vec<NodeId>,
    ) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            props: props
                .into_iter()
                .filter(|(_, v)| !matches!(v, Value::Nil))
                .map(|(k, v)| (key(k).to_owned(), v))
                .collect(),
            children: vec![],
            parent: None,
            text: String::new(),
        });
        self.set_contents(id, children);
        id
    }

    /// A plain-text node.
    pub fn text(&mut self, s: impl Into<String>) -> NodeId {
        let id = self.create(T::PlainText, vec![], vec![]);
        self.nodes[id].text = s.into();
        id
    }

    /// org-element-type.
    pub fn kind(&self, id: NodeId) -> Type {
        self.nodes[id].kind
    }

    /// org-element-type-p.
    pub fn is(&self, id: NodeId, types: &[Type]) -> bool {
        types.contains(&self.kind(id))
    }

    /// The string of a plain-text node.
    pub fn plain_text(&self, id: NodeId) -> Option<&str> {
        (self.kind(id) == T::PlainText).then(|| self.nodes[id].text.as_str())
    }

    /// org-element-property: Nil when missing.
    pub fn prop(&self, id: NodeId, k: &str) -> &Value {
        self.get(id, k).unwrap_or(&NIL)
    }

    /// The property when present (even if nil).
    pub fn get(&self, id: NodeId, k: &str) -> Option<&Value> {
        let k = key(k);
        self.nodes[id]
            .props
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
    }

    /// org-element-put-property.
    pub fn put(&mut self, id: NodeId, k: &str, v: impl Into<Value>) {
        let v = v.into();
        let k = key(k);
        let props = &mut self.nodes[id].props;
        match props.iter_mut().find(|(n, _)| n == k) {
            Some((_, old)) => *old = v,
            None => props.push((k.to_owned(), v)),
        }
    }

    /// Properties in order (org-element-properties-map).
    pub fn props(&self, id: NodeId) -> impl Iterator<Item = (&str, &Value)> {
        self.nodes[id].props.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn str(&self, id: NodeId, k: &str) -> Option<&str> {
        self.prop(id, k).as_str()
    }

    pub fn int(&self, id: NodeId, k: &str) -> Option<i64> {
        self.prop(id, k).as_int()
    }

    pub fn flag(&self, id: NodeId, k: &str) -> bool {
        self.prop(id, k).truthy()
    }

    fn pos(&self, id: NodeId, k: &str) -> Option<usize> {
        self.int(id, k).map(|n| n as usize)
    }

    /// org-element-begin.
    pub fn begin(&self, id: NodeId) -> Option<usize> {
        self.pos(id, "begin")
    }
    pub fn end(&self, id: NodeId) -> Option<usize> {
        self.pos(id, "end")
    }
    pub fn contents_begin(&self, id: NodeId) -> Option<usize> {
        self.pos(id, "contents-begin")
    }
    pub fn contents_end(&self, id: NodeId) -> Option<usize> {
        self.pos(id, "contents-end")
    }
    pub fn post_affiliated(&self, id: NodeId) -> Option<usize> {
        self.pos(id, "post-affiliated")
    }
    /// org-element-post-blank (0 when unset).
    pub fn post_blank(&self, id: NodeId) -> usize {
        self.pos(id, "post-blank").unwrap_or(0)
    }

    /// org-element-contents.
    pub fn contents(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id].children
    }

    /// org-element-set-contents (also sets each child's parent).
    pub fn set_contents(&mut self, id: NodeId, children: Vec<NodeId>) {
        for &c in &children {
            self.nodes[c].parent = Some(id);
        }
        self.nodes[id].children = children;
    }

    /// org-element-parent.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id].parent
    }

    pub fn set_parent(&mut self, id: NodeId, parent: Option<NodeId>) {
        self.nodes[id].parent = parent;
    }

    /// org-element-secondary-p: the property of the parent holding ID
    /// in a secondary string.
    pub fn secondary_p(&self, id: NodeId) -> Option<&'static str> {
        let p = self.parent(id)?;
        self.kind(p).secondary().iter().copied().find(|k| {
            self.prop(p, k)
                .as_secondary()
                .is_some_and(|v| v.contains(&id))
        })
    }

    /// org-element-class.
    pub fn class(&self, id: NodeId) -> Class {
        self.class_in(id, self.parent(id))
    }

    /// org-element-class with an explicit PARENT.
    pub fn class_in(&self, id: NodeId, parent: Option<NodeId>) -> Class {
        let t = self.kind(id);
        if t.is_object() || t == T::PlainText || t == T::Anonymous {
            return Class::Object;
        }
        if t.is_element() || t == T::OrgData {
            return Class::Element;
        }
        let Some(p) = parent else {
            return Class::Element;
        };
        let pt = self.kind(p);
        if pt == T::Anonymous || OBJECT_CONTAINERS.contains(&pt) || self.secondary_p(id).is_some() {
            Class::Object
        } else {
            Class::Element
        }
    }

    /// org-element-lineage: ancestors from the closest (with SELF first
    /// when `with_self`).
    pub fn lineage(&self, id: NodeId, with_self: bool) -> Vec<NodeId> {
        let mut up = if with_self { Some(id) } else { self.parent(id) };
        let mut out = vec![];
        while let Some(u) = up {
            out.push(u);
            up = self.parent(u);
        }
        out
    }

    /// org-element-lineage with TYPES: the first ancestor of one of TYPES.
    pub fn lineage_of(&self, id: NodeId, types: &[Type], with_self: bool) -> Option<NodeId> {
        self.lineage(id, with_self)
            .into_iter()
            .find(|&u| self.is(u, types))
    }

    /// org-element-lineage-map.
    pub fn lineage_map<R>(
        &self,
        id: NodeId,
        types: Option<&[Type]>,
        with_self: bool,
        first_match: bool,
        mut f: impl FnMut(&Tree, NodeId) -> Option<R>,
    ) -> Vec<R> {
        let mut out = vec![];
        for u in self.lineage(id, with_self) {
            if types.is_none_or(|t| self.is(u, t))
                && let Some(r) = f(self, u)
            {
                out.push(r);
                if first_match {
                    break;
                }
            }
        }
        out
    }

    /// org-element-parent-element: the first element containing ID.
    pub fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        self.lineage_of(id, ALL_ELEMENTS, false)
    }

    /// org-element-property-inherited.  With `accumulate`, all values
    /// from the outermost ancestor (lists appended); otherwise the first
    /// non-nil value.
    pub fn property_inherited(
        &self,
        keys: &[&str],
        id: NodeId,
        with_self: bool,
        accumulate: bool,
        literal_nil: bool,
        include_nil: bool,
    ) -> Vec<Value> {
        let mut acc: Vec<Value> = vec![];
        for n in self.lineage(id, with_self) {
            let mut local = vec![];
            for k in keys {
                let Some(v) = self.get(n, k) else { continue };
                let mut v = v.clone();
                if literal_nil && v.as_str() == Some("nil") {
                    v = Value::Nil;
                }
                if !accumulate && (v.truthy() || include_nil) {
                    return vec![v];
                }
                if include_nil && v.is_nil() {
                    local.push(Value::Nil);
                } else {
                    match v {
                        Value::List(l) => local.extend(l),
                        Value::Nil => {}
                        v => local.push(v),
                    }
                }
            }
            local.extend(acc);
            acc = local;
        }
        if accumulate { acc } else { vec![] }
    }

    /// org-element-adopt: append CHILDREN to PARENT's contents.
    pub fn adopt(&mut self, parent: NodeId, children: &[NodeId]) -> NodeId {
        let mut c = self.nodes[parent].children.clone();
        c.extend_from_slice(children);
        self.set_contents(parent, c);
        parent
    }

    /// The list (contents or secondary string) holding ID.
    fn siblings_mut(&mut self, id: NodeId) -> Option<&mut Vec<NodeId>> {
        let p = self.parent(id)?;
        match self.secondary_p(id) {
            Some(k) => {
                let k = k.to_owned();
                let props = &mut self.nodes[p].props;
                props
                    .iter_mut()
                    .find(|(n, _)| *n == k)
                    .and_then(|(_, v)| match v {
                        Value::Secondary(s) => Some(s),
                        _ => None,
                    })
            }
            None => Some(&mut self.nodes[p].children),
        }
    }

    /// org-element-extract: remove ID from its parent.
    pub fn extract(&mut self, id: NodeId) -> NodeId {
        if let Some(s) = self.siblings_mut(id) {
            s.retain(|&x| x != id);
        }
        self.nodes[id].parent = None;
        id
    }

    /// org-element-insert-before: insert NODE before LOCATION.
    pub fn insert_before(&mut self, node: NodeId, location: NodeId) {
        let parent = self.parent(location);
        if let Some(s) = self.siblings_mut(location) {
            let i = s.iter().position(|&x| x == location).unwrap_or(s.len());
            s.insert(i, node);
        }
        self.nodes[node].parent = parent;
    }

    /// org-element-set: replace OLD with NEW in place (OLD's id now
    /// holds NEW's data).  KEEP_PROPS are kept from OLD.
    pub fn set(&mut self, old: NodeId, new: NodeId, keep_props: &[&str]) -> NodeId {
        for k in keep_props {
            let v = self.prop(old, k).clone();
            self.put(new, k, v);
        }
        let parent = self.parent(old);
        let mut n = self.nodes[new].clone();
        n.parent = parent;
        for &c in &n.children {
            self.nodes[c].parent = Some(old);
        }
        for k in n.kind.secondary() {
            if let Some(Value::Secondary(s)) = n.props.iter().find(|(x, _)| x == k).map(|(_, v)| v)
            {
                for &c in s {
                    self.nodes[c].parent = Some(old);
                }
            }
        }
        self.nodes[old] = n;
        old
    }

    /// org-element-copy: a parentless copy of ID, with secondary strings
    /// copied (and contents when `keep_contents`, or for anonymous nodes).
    pub fn copy(&mut self, id: NodeId, keep_contents: bool) -> NodeId {
        let mut n = self.nodes[id].clone();
        n.parent = None;
        let new = self.nodes.len();
        self.nodes.push(n);
        let kind = self.kind(id);
        if kind == T::PlainText {
            return new;
        }
        for k in kind.secondary() {
            if let Some(Value::Secondary(s)) = self.get(new, k).cloned() {
                let copies: Vec<NodeId> = s.iter().map(|&c| self.copy(c, true)).collect();
                for &c in &copies {
                    self.nodes[c].parent = Some(new);
                }
                self.put(new, k, Value::Secondary(copies));
            }
        }
        let children = std::mem::take(&mut self.nodes[new].children);
        if keep_contents || kind == T::Anonymous {
            let copies = children.iter().map(|&c| self.copy(c, true)).collect();
            self.set_contents(new, copies);
        }
        new
    }

    /// org-element-ast-map.  TYPES None means all types.
    pub fn ast_map<R>(
        &self,
        data: NodeId,
        types: Option<&[Type]>,
        opts: &MapOpts,
        mut f: impl FnMut(&Tree, NodeId) -> Visit<R>,
    ) -> Vec<R> {
        let mut acc = vec![];
        self.walk(data, types, opts, &mut f, &mut acc);
        acc
    }

    fn walk<R>(
        &self,
        id: NodeId,
        types: Option<&[Type]>,
        opts: &MapOpts,
        f: &mut impl FnMut(&Tree, NodeId) -> Visit<R>,
        acc: &mut Vec<R>,
    ) -> bool {
        if opts.ignore.contains(&id) {
            return false;
        }
        let kind = self.kind(id);
        if kind == T::Anonymous {
            for &c in self.contents(id) {
                if self.walk(c, types, opts, f, acc) {
                    return true;
                }
            }
            return false;
        }
        let mut recurse = true;
        if types.is_none_or(|t| t.contains(&kind)) {
            match f(self, id) {
                Visit::Value(v) => {
                    acc.push(v);
                    if opts.first_match {
                        return true;
                    }
                }
                Visit::Nil => {}
                Visit::Skip => recurse = false,
            }
        }
        if !recurse || opts.no_recursion.contains(&kind) {
            return false;
        }
        let mut props: Vec<&str> = opts.with_properties.clone();
        if !opts.no_secondary {
            props.extend_from_slice(kind.secondary());
        }
        for p in props {
            if let Some(v) = self.get(id, p)
                && self.walk_value(v, types, opts, f, acc)
            {
                return true;
            }
        }
        for &c in self.contents(id) {
            if self.walk(c, types, opts, f, acc) {
                return true;
            }
        }
        false
    }

    fn walk_value<R>(
        &self,
        v: &Value,
        types: Option<&[Type]>,
        opts: &MapOpts,
        f: &mut impl FnMut(&Tree, NodeId) -> Visit<R>,
        acc: &mut Vec<R>,
    ) -> bool {
        match v {
            Value::Node(n) => self.walk(*n, types, opts, f, acc),
            Value::Secondary(s) => s.iter().any(|&n| self.walk(n, types, opts, f, acc)),
            Value::List(l) => l.iter().any(|x| self.walk_value(x, types, opts, f, acc)),
            Value::Cons(a, b) => {
                self.walk_value(a, types, opts, f, acc) || self.walk_value(b, types, opts, f, acc)
            }
            _ => false,
        }
    }

    /// org-element-map: FUN on nodes of TYPES (None: all) under DATA.
    /// Without object types, objects and object containers are not
    /// entered.  `with_affiliated` also maps parsed affiliated keywords.
    pub fn map_with<R>(
        &self,
        data: NodeId,
        types: Option<&[Type]>,
        first_match: bool,
        no_recursion: &[Type],
        with_affiliated: bool,
        f: impl FnMut(&Tree, NodeId) -> Visit<R>,
    ) -> Vec<R> {
        let objects = types.is_none_or(|t| t.iter().any(|x| x.is_object() || *x == T::PlainText));
        let mut nr = no_recursion.to_vec();
        if !objects {
            nr.extend_from_slice(ALL_OBJECTS);
            nr.extend(ALL_ELEMENTS.iter().filter(|t| !t.is_greater()));
        }
        let opts = MapOpts {
            ignore: &[],
            first_match,
            no_recursion: nr,
            with_properties: if with_affiliated && objects {
                vec!["caption"]
            } else {
                vec![]
            },
            no_secondary: !objects,
        };
        self.ast_map(data, types, &opts, f)
    }

    /// org-element-map with defaults: collect FUN's non-None results.
    pub fn map<R>(
        &self,
        data: NodeId,
        types: Option<&[Type]>,
        mut f: impl FnMut(&Tree, NodeId) -> Option<R>,
    ) -> Vec<R> {
        self.map_with(data, types, false, &[], false, |t, n| match f(t, n) {
            Some(r) => Visit::Value(r),
            None => Visit::Nil,
        })
    }

    /// org-element-map with FIRST-MATCH.
    pub fn map_first<R>(
        &self,
        data: NodeId,
        types: Option<&[Type]>,
        mut f: impl FnMut(&Tree, NodeId) -> Option<R>,
    ) -> Option<R> {
        self.map_with(data, types, true, &[], false, |t, n| match f(t, n) {
            Some(r) => Visit::Value(r),
            None => Visit::Nil,
        })
        .into_iter()
        .next()
    }

    /// An anonymous node holding a secondary string (org-element-adopt
    /// with a nil parent).
    pub fn anonymous(&mut self, children: Vec<NodeId>) -> NodeId {
        self.create(T::Anonymous, vec![], children)
    }

    /// Set a secondary string property, parenting its members to ID.
    pub fn put_secondary(&mut self, id: NodeId, k: &str, s: Vec<NodeId>) {
        for &c in &s {
            self.nodes[c].parent = Some(id);
        }
        self.put(id, k, Value::Secondary(s));
    }

    /// The raw text of a secondary string (plain text concatenated,
    /// objects interpreted).
    pub fn secondary(&self, id: NodeId, k: &str) -> &[NodeId] {
        self.prop(id, k).as_secondary().unwrap_or(&[])
    }
}
