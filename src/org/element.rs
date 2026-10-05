//! Port of org-element.el and org-element-ast.el: the Org parser, its
//! syntax tree, interpreter and queries.  See docs/org-port-notes.md
//! ("Module contracts").
//!
//! - [`parse_buffer`] / [`parse_buffer_with`]: org-element-parse-buffer
//!   on a string, positions are byte offsets into it.
//! - [`parse_secondary_string`]: objects of a string.
//! - [`interpret`] / [`interpret_value`]: org-element-interpret-data.
//! - [`Tree`]: org-element-ast (properties, contents, lineage, map,
//!   adopt/extract/insert-before/set/copy).
//! - [`at_point`] / [`context`]: org-element-at-point and
//!   org-element-context.
//!
//! Buffer-local variables come from [`Settings`] (TODO keywords, link
//! abbreviations, `#+STARTUP: odd`) and Fred's options (org-tags-column,
//! org-footnote-section, org-inlinetask-min-level, ...).  The element
//! cache is an Emacs performance device: Fred parses on demand, so the
//! cache commands succeed without doing anything.

mod ast;
mod entities;
mod interpret;
mod objects;
mod parser;
mod query;
#[cfg(test)]
mod tests;

pub use ast::*;
pub use interpret::{
    format_date, interpret, interpret_value, normalize_string, remove_indentation, table_align,
    timestamp_string,
};
pub use objects::{
    escape as link_escape, expand_abbrev, macro_escape_arguments, macro_extract_arguments,
    parse_time_string, unescape as link_unescape,
};
pub use parser::{Ctx, DEFAULT_LINK_TYPES, ListItem, escape_code, unescape_code};
pub use query::{at_point, context};

use super::Prefix;
use crate::editor::Editor;
use crate::org::syntax::Settings;
use std::sync::LazyLock;

/// This module's interactive commands, by upstream name.
pub fn command(_ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    match name {
        // Fred reparses on demand: there is no cache to reset or refresh.
        "org-element-cache-reset" | "org-element-cache-refresh" | "org-element-update-syntax" => {
            Some(Ok(()))
        }
        _ => None,
    }
}

/// Parsing depth (org-element-parse-buffer GRANULARITY).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Granularity {
    Headline,
    GreaterElement,
    Element,
    #[default]
    Object,
}

/// Options of [`parse_buffer_with`].
#[derive(Clone, Debug, Default)]
pub struct Opts {
    pub granularity: Granularity,
    /// The visited file: `:path` of org-data and the default category.
    pub path: Option<std::path::PathBuf>,
    /// Hidden byte ranges skipped by VISIBLE-ONLY parsing.
    pub invisible: Vec<std::ops::Range<usize>>,
}

/// org-element-parse-buffer at object granularity.
pub fn parse_buffer(text: &str, st: &Settings) -> Tree {
    parse_buffer_with(text, st, &Opts::default())
}

/// org-element-parse-buffer.
pub fn parse_buffer_with(text: &str, st: &Settings, opts: &Opts) -> Tree {
    let mut cx = Ctx::new(st);
    cx.category = category(text, opts.path.as_deref());
    if text.contains("<<<") {
        // org-update-radio-target-regexp: the radio targets of the text.
        let t = parse_with(
            text,
            &cx,
            &Opts {
                granularity: Granularity::Object,
                ..opts.clone()
            },
        );
        let targets = t.map(t.root, Some(&[Type::RadioTarget]), |t, n| {
            t.str(n, "value").map(str::to_owned)
        });
        cx.set_radio_targets(targets);
    }
    parse_with(text, &cx, opts)
}

fn parse_with(text: &str, cx: &Ctx, opts: &Opts) -> Tree {
    let mut p = parser::P::new(text, cx);
    p.opts(opts);
    let root = p.org_data();
    if let Some(path) = &opts.path {
        p.t.put(root, "path", path.to_string_lossy().into_owned());
    }
    let cb = p.t.contents_begin(root).unwrap_or(0);
    p.parse_elements(
        cb,
        text.len(),
        parser::Mode::FirstSection,
        None,
        opts.granularity,
        root,
    );
    p.t.root = root;
    p.t
}

/// org-element--get-category: the last `#+CATEGORY` keyword, else
/// org-category, else the file name without extension.
fn category(text: &str, path: Option<&std::path::Path>) -> Option<String> {
    let kw = text.lines().rev().find_map(|l| {
        let (k, v) = crate::org::syntax::keyword_line(l)?;
        (k == "CATEGORY").then(|| v.to_owned())
    });
    kw.or_else(|| {
        let c = crate::org::options::string("org-category", "");
        (!c.is_empty() && c != "nil").then_some(c)
    })
    .or_else(|| {
        path.and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
    })
}

/// org-element-parse-secondary-string: objects of S allowed by
/// RESTRICTION, under an anonymous root.
pub fn parse_secondary_string(s: &str, restriction: &[Type], st: &Settings) -> Tree {
    let cx = Ctx::new(st);
    let mut p = parser::P::new(s, &cx);
    let objs = if s.is_empty() {
        vec![]
    } else {
        p.parse_objects(0, s.len(), restriction, None)
    };
    let root = p.t.anonymous(objs);
    p.t.root = root;
    p.t
}

static DEFAULT_CTX: LazyLock<Ctx> = LazyLock::new(|| Ctx::new(&Settings::default()));

/// Run F with a parser on TEXT in the default environment.
fn with_default_parser<R>(text: &str, f: impl FnOnce(&mut parser::P) -> R) -> R {
    let mut p = parser::P::new(text, &DEFAULT_CTX);
    f(&mut p)
}

/// org-entity-get: (name latex math-p html ascii latin1 utf-8).
pub fn entity(
    name: &str,
) -> Option<&'static (
    &'static str,
    &'static str,
    bool,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    entities::ENTITIES.iter().find(|e| e.0 == name)
}

use Type as T;

fn without(set: &[Type], drop: &[Type]) -> Vec<Type> {
    set.iter().copied().filter(|t| !drop.contains(t)).collect()
}

const MINIMAL: &[Type] = &[
    T::Bold,
    T::Code,
    T::Entity,
    T::Italic,
    T::LatexFragment,
    T::StrikeThrough,
    T::Subscript,
    T::Superscript,
    T::Underline,
    T::Verbatim,
];
static STANDARD: LazyLock<Vec<Type>> =
    LazyLock::new(|| without(ALL_OBJECTS, &[T::CitationReference, T::TableCell]));
static NO_LINE_BREAK: LazyLock<Vec<Type>> = LazyLock::new(|| without(&STANDARD, &[T::LineBreak]));
static FOR_CITATIONS: LazyLock<Vec<Type>> = LazyLock::new(|| {
    without(
        &NO_LINE_BREAK,
        &[
            T::Citation,
            T::CitationReference,
            T::FootnoteReference,
            T::Link,
        ],
    )
});
static KEYWORD: LazyLock<Vec<Type>> = LazyLock::new(|| without(&STANDARD, &[T::FootnoteReference]));
static LINK: LazyLock<Vec<Type>> = LazyLock::new(|| {
    let mut v = vec![
        T::ExportSnippet,
        T::InlineBabelCall,
        T::InlineSrcBlock,
        T::Macro,
        T::StatisticsCookie,
    ];
    v.extend_from_slice(MINIMAL);
    v
});
static TABLE_CELL: LazyLock<Vec<Type>> = LazyLock::new(|| {
    let mut v = vec![
        T::Citation,
        T::ExportSnippet,
        T::FootnoteReference,
        T::Link,
        T::Macro,
        T::RadioTarget,
        T::Target,
        T::Timestamp,
    ];
    v.extend_from_slice(MINIMAL);
    v
});

/// org-element-restriction: object types allowed in TYPE.
pub fn restriction(t: Type) -> &'static [Type] {
    match t {
        T::Bold
        | T::FootnoteReference
        | T::Italic
        | T::Paragraph
        | T::StrikeThrough
        | T::Subscript
        | T::Superscript
        | T::Underline
        | T::VerseBlock => &STANDARD,
        T::Citation => &[T::CitationReference],
        T::CitationReference => &FOR_CITATIONS,
        T::Headline | T::Inlinetask | T::Item => &NO_LINE_BREAK,
        T::Keyword => &KEYWORD,
        T::Link => &LINK,
        T::RadioTarget => MINIMAL,
        T::TableCell => &TABLE_CELL,
        T::TableRow => &[T::TableCell],
        _ => &[],
    }
}

/// Byte offset of LINE (0-based) and BYTE within it.
pub fn offset(text: &str, line: usize, byte: usize) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    (start + byte).min(text.len())
}

/// org-element-at-point on TEXT: the parse tree and the element.
pub fn element_at(text: &str, st: &Settings, pos: usize) -> (Tree, Option<NodeId>) {
    let t = parse_buffer(text, st);
    let e = at_point(text, &t, pos);
    (t, e)
}

/// org-element-context on TEXT: the parse tree and the element or object.
pub fn context_at(text: &str, st: &Settings, pos: usize) -> (Tree, Option<NodeId>) {
    let t = parse_buffer(text, st);
    let e = context(text, &t, pos);
    (t, e)
}

/// org-element-nested-p.
pub fn nested_p(t: &Tree, a: NodeId, b: NodeId) -> bool {
    let (ba, ea, bb, eb) = (t.begin(a), t.end(a), t.begin(b), t.end(b));
    (ba >= bb && ea <= eb) || (bb >= ba && eb <= ea)
}

/// org-element-swap-A-B on TEXT: swap elements A and B (B after A),
/// without their trailing blank lines.  Returns the new text and the
/// position after the moved A (the end of B's old place).
pub fn swap_a_b(text: &str, t: &Tree, a: NodeId, b: NodeId) -> Result<(String, usize), String> {
    let pos =
        |n: NodeId, k: fn(&Tree, NodeId) -> Option<usize>| k(t, n).ok_or("Cannot swap elements");
    let (beg_a, beg_b) = (pos(a, Tree::begin)?, pos(b, Tree::begin)?);
    let special = beg_a > 0 && text.as_bytes()[beg_a - 1] != b'\n';
    if special && (t.kind(b) != T::Paragraph || t.begin(b) != t.contents_begin(b)) {
        return Err("Cannot swap elements".into());
    }
    let trimmed_end = |n: NodeId| -> Result<usize, String> {
        let e = pos(n, Tree::end)?;
        let q = text[..e].trim_end_matches([' ', '\r', '\t', '\n']).len();
        Ok(text[q..].find('\n').map_or(text.len(), |i| q + i))
    };
    let (end_a, end_b) = (trimmed_end(a)?, trimmed_end(b)?);
    let body_a = &text[beg_a..end_a];
    let mut body_b = text[beg_b..end_b].to_owned();
    let mut ind_b = String::new();
    if special {
        let bol = text[..beg_b].rfind('\n').map_or(0, |i| i + 1);
        ind_b = text[bol..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        body_b = body_b.trim_start_matches([' ', '\t']).to_owned();
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..beg_a]);
    out.push_str(&body_b);
    out.push_str(&text[end_a..beg_b]);
    out.push_str(&ind_b);
    out.push_str(body_a);
    out.push_str(&text[end_b..]);
    // Point goes to B's (stale) end position, as upstream.
    let end = pos(b, Tree::end)?.min(out.len());
    Ok((out, end))
}
