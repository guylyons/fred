//! org-lint.el: check an Org buffer for syntax mistakes.
//!
//! Reports go to a `*Org Lint*` buffer (RET jumps to the source line,
//! TAB/C-j show it in the message area since Fred has no windows, `h`
//! hides and `i` ignores a checker, `g` refreshes, `q` quits). Tables that
//! upstream computes from loaded libraries (export options of the default
//! backends, macro templates) were taken from Emacs 30 with the pinned Org.

use super::Prefix;
use super::element::{self, NodeId, Tree, Type, Value};
use super::syntax::Settings;
use crate::editor::Editor;
use regex::Regex;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// (name, trust, categories), in upstream's `org-lint--checkers` order.
pub const CHECKERS: &[(&str, &str, &[&str])] = &[
    ("planning-inactive", "high", &["timestamp"]),
    ("clock-syntax", "low", &["timestamp"]),
    ("timestamp-syntax", "low", &["timestamp"]),
    ("beamer-frame", "low", &["export"]),
    ("LaTeX-$", "low", &["markup"]),
    ("priority", "nil", &["markup"]),
    ("item-number", "nil", &["plain-list"]),
    ("incomplete-citation", "low", &["cite"]),
    ("invalid-cite-export-declaration", "nil", &["cite"]),
    ("missing-print-bibliography", "nil", &["cite"]),
    ("non-existent-bibliography", "nil", &["cite"]),
    ("spurious-colons", "nil", &["tags"]),
    ("percent-encoding-link-escape", "low", &["link", "obsolete"]),
    ("file-application", "nil", &["link", "obsolete"]),
    ("quote-section", "low", &["obsolete"]),
    ("indented-diary-sexp", "low", &[]),
    ("incomplete-drawer", "low", &[]),
    ("misplaced-planning-info", "low", &[]),
    ("mismatched-planning-repeaters", "low", &[]),
    ("invalid-block", "low", &[]),
    ("invalid-image-alignment", "high", &[]),
    ("invalid-keyword-syntax", "low", &[]),
    (
        "extraneous-element-in-footnote-section",
        "nil",
        &["footnote"],
    ),
    ("unreferenced-footnote-definition", "nil", &["footnote"]),
    ("undefined-footnote-reference", "nil", &["footnote"]),
    ("invalid-id-property", "nil", &["properties"]),
    ("invalid-effort-property", "nil", &["properties"]),
    (
        "obsolete-properties-drawer",
        "nil",
        &["obsolete", "properties"],
    ),
    (
        "special-property-in-properties-drawer",
        "nil",
        &["properties"],
    ),
    ("invalid-macro-argument-and-template", "low", &["export"]),
    ("misspelled-export-option", "low", &["export"]),
    ("unknown-options-item", "low", &["export"]),
    ("obsolete-include-markup", "low", &["obsolete", "export"]),
    ("wrong-include-link-parameter", "low", &["export"]),
    ("non-existent-setupfile-parameter", "low", &[]),
    ("link-to-local-file", "low", &["link"]),
    ("unclosed-brackets-in-link-description", "low", &["link"]),
    ("trailing-bracket-after-link", "low", &["link"]),
    ("invalid-id-link", "nil", &["link"]),
    ("invalid-fuzzy-link", "nil", &["link"]),
    ("invalid-custom-id-link", "nil", &["link"]),
    ("invalid-coderef-link", "nil", &["link"]),
    ("deprecated-category-setup", "nil", &["obsolete"]),
    ("empty-header-argument", "low", &["babel"]),
    ("named-result", "high", &["babel"]),
    ("wrong-header-value", "low", &["babel"]),
    ("wrong-header-argument", "nil", &["babel"]),
    ("invalid-babel-call-block", "nil", &["babel"]),
    ("missing-backend-in-export-block", "nil", &["export"]),
    ("suspicious-language-in-src-block", "low", &["babel"]),
    ("missing-language-in-src-block", "nil", &["babel"]),
    ("deprecated-header-syntax", "low", &["obsolete", "babel"]),
    ("deprecated-export-blocks", "low", &["obsolete", "export"]),
    ("obsolete-affiliated-keywords", "nil", &["obsolete"]),
    ("combining-keywords-with-affiliated", "low", &[]),
    ("orphaned-affiliated-keywords", "low", &[]),
    ("duplicate-footnote-definition", "nil", &["footnote"]),
    ("duplicate-target", "nil", &["link"]),
    ("duplicate-name", "nil", &["babel", "'link"]),
    ("duplicate-custom-id", "nil", &["link"]),
    ("misplaced-heading", "low", &[]),
];

/// OPTIONS items of org-export-options-alist and the default backends.
const OPTION_ITEMS: &[&str] = &[
    "H",
    "\\n",
    "num",
    "timestamp",
    "arch",
    "author",
    "expand-links",
    "broken-links",
    "c",
    "creator",
    "date",
    "d",
    "email",
    "*",
    "e",
    ":",
    "f",
    "inline",
    "tex",
    "p",
    "pri",
    "prop",
    "'",
    "-",
    "stat",
    "^",
    "toc",
    "|",
    "tags",
    "tasks",
    "<",
    "title",
    "todo",
    "latex-use-sans",
    "html5-fancy",
    "html-link-use-abs-url",
    "html-postamble",
    "html-preamble",
    "html-style",
    "html-scripts",
    "html-allow-name-attribute-in-anchors",
    "html-checkbox-type",
    "html-extension",
    "html-indent",
    "html-mathjax",
    "html-xml-declaration",
    "html-inline-images",
    "html-head-include-default-style",
    "html-head-include-scripts",
    "ascii-charset",
    "ascii-global-margin",
    "ascii-inner-margin",
    "ascii-text-width",
];

/// Keywords of org-export-options-alist.
const COMMON_OPTIONS: &[&str] = &[
    "CITE_EXPORT",
    "CREATOR",
    "EXCLUDE_TAGS",
    "SELECT_TAGS",
    "OTHER_LANGUAGES",
    "LANGUAGE",
    "EMAIL",
    "AUTHOR",
    "DATE",
    "TITLE",
];

/// Backend option keywords and the backends defining them.
const BACKEND_OPTIONS: &[(&str, &[&str])] = &[
    ("ODT_STYLES_FILE", &["odt"]),
    ("DESCRIPTION", &["html", "latex", "odt"]),
    ("KEYWORDS", &["html", "latex", "odt"]),
    ("SUBTITLE", &["ascii", "html", "latex", "odt"]),
    ("LATEX_HEADER", &["html", "latex", "odt"]),
    ("LATEX_CLASS", &["latex"]),
    ("LATEX_CLASS_OPTIONS", &["latex"]),
    ("LATEX_HEADER_EXTRA", &["latex"]),
    ("LATEX_CLASS_PRE", &["latex"]),
    ("LATEX_DOC_METADATA", &["latex"]),
    ("LATEX_FOOTNOTE_COMMAND", &["latex"]),
    ("LATEX_ENGRAVED_THEME", &["latex"]),
    ("LATEX_COMPILER", &["latex"]),
    ("LATEX_DESCRIPTION", &["latex"]),
    ("ICALENDAR_TIMEZONE", &["icalendar"]),
    ("HTML_DOCTYPE", &["html"]),
    ("HTML_CONTAINER", &["html"]),
    ("HTML_CONTENT_CLASS", &["html"]),
    ("HTML_EQUATION_REFERENCE_FORMAT", &["html"]),
    ("HTML_LINK_HOME", &["html"]),
    ("HTML_LINK_UP", &["html"]),
    ("HTML_MATHJAX", &["html"]),
    ("HTML_HEAD", &["html"]),
    ("HTML_HEAD_EXTRA", &["html"]),
    ("HTML_FORMULA_PATH", &["html"]),
];

/// org-default-properties.
const DEFAULT_PROPERTIES: &[&str] = &[
    "ARCHIVE",
    "CATEGORY",
    "SUMMARY",
    "DESCRIPTION",
    "CUSTOM_ID",
    "LOCATION",
    "LOGGING",
    "COLUMNS",
    "VISIBILITY",
    "TABLE_EXPORT_FORMAT",
    "TABLE_EXPORT_FILE",
    "EXPORT_OPTIONS",
    "EXPORT_TEXT",
    "EXPORT_FILE_NAME",
    "EXPORT_TITLE",
    "EXPORT_AUTHOR",
    "EXPORT_DATE",
    "UNNUMBERED",
    "ORDERED",
    "NOBLOCKING",
    "COOKIE_DATA",
    "LOG_INTO_DRAWER",
    "REPEAT_TO_STATE",
    "CLOCK_MODELINE_TOTAL",
    "STYLE",
    "HTML_CONTAINER_CLASS",
    "ORG-IMAGE-ACTUAL-WIDTH",
];

/// org-special-properties.
const SPECIAL_PROPERTIES: &[&str] = &[
    "ALLTAGS",
    "BLOCKED",
    "CLOCKSUM",
    "CLOCKSUM_T",
    "CLOSED",
    "DEADLINE",
    "FILE",
    "ITEM",
    "PRIORITY",
    "SCHEDULED",
    "TAGS",
    "TIMESTAMP",
    "TIMESTAMP_IA",
    "TODO",
];

/// org-element-affiliated-keywords.
const AFFILIATED: &[&str] = &[
    "CAPTION", "DATA", "HEADER", "HEADERS", "LABEL", "NAME", "PLOT", "RESNAME", "RESULT",
    "RESULTS", "SOURCE", "SRCNAME", "TBLNAME",
];

/// What the checkers see.
pub struct Ctx<'a> {
    pub text: &'a str,
    pub t: &'a Tree,
    pub st: &'a Settings,
    /// default-directory of the buffer.
    pub dir: PathBuf,
    /// The buffer's file.
    pub file: Option<PathBuf>,
    /// org-attach-expand.
    pub attach_expand: &'a dyn Fn(&str) -> PathBuf,
    /// org-id-find-id-file.
    pub id_known: &'a dyn Fn(&str) -> bool,
}

/// (byte position, message).
type Reports = Vec<(usize, String)>;

impl Ctx<'_> {
    fn map(&self, types: &[Type], mut f: impl FnMut(NodeId) -> Option<(usize, String)>) -> Reports {
        self.t.map(self.t.root, Some(types), |_, n| f(n))
    }
    fn begin(&self, n: NodeId) -> usize {
        self.t.begin(n).unwrap_or(0)
    }
    fn post_aff(&self, n: NodeId) -> usize {
        self.t.post_affiliated(n).or(self.t.begin(n)).unwrap_or(0)
    }
    fn s(&self, n: NodeId, k: &str) -> Option<&str> {
        self.t.str(n, k)
    }
    fn at(&self, pos: usize) -> Option<Type> {
        element::at_point(self.text, self.t, pos).map(|n| self.t.kind(n))
    }
    fn bol(&self, pos: usize) -> usize {
        self.text[..pos.min(self.text.len())]
            .rfind('\n')
            .map_or(0, |i| i + 1)
    }
    fn file_exists(&self, f: &str) -> bool {
        let p = super::options::expand(&substitute_env(f));
        if p.is_absolute() {
            p.exists()
        } else {
            self.dir.join(p).exists()
        }
    }
}

/// substitute-env-in-file-name: $VAR and ${VAR}.
fn substitute_env(s: &str) -> String {
    crate::org_re!(r"\$(\{([A-Za-z_][A-Za-z0-9_]*)\}|([A-Za-z_][A-Za-z0-9_]*))")
        .replace_all(s, |c: &regex::Captures| {
            let n = c.get(2).or(c.get(3)).unwrap().as_str();
            std::env::var(n).unwrap_or_else(|_| c[0].to_owned())
        })
        .into_owned()
}

fn remote_p(f: &str) -> bool {
    crate::org_re!(r"^/[^/:]+:").is_match(f)
}

fn url_p(f: &str) -> bool {
    crate::org_re!(r"^[a-zA-Z][-a-zA-Z0-9+.]*://").is_match(f)
}

fn strip_quotes(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(s)
}

fn ci(re: &str) -> Regex {
    Regex::new(&format!("(?im){re}")).unwrap()
}

/// org-lint--collect-duplicates.
fn duplicates<K: PartialEq + Clone>(
    c: &Ctx,
    types: &[Type],
    key: impl Fn(NodeId) -> Option<K>,
    pos: impl Fn(NodeId, &K) -> usize,
    msg: impl Fn(&K) -> String,
) -> Reports {
    let mut keys: Vec<(K, usize)> = vec![];
    let mut originals: Vec<(K, usize)> = vec![];
    let mut reports = vec![];
    c.t.map(c.t.root, Some(types), |_, n| -> Option<()> {
        let k = key(n)?;
        match keys.iter().find(|(kk, _)| *kk == k) {
            Some(orig) => {
                if !originals.iter().any(|(kk, _)| *kk == k) {
                    originals.push(orig.clone());
                }
                reports.push((pos(n, &k), msg(&k)));
            }
            None => keys.push((k.clone(), pos(n, &k))),
        }
        None
    });
    for (k, p) in originals {
        reports.push((p, msg(&k)));
    }
    reports
}

// ---- checkers ----

fn misplaced_heading(c: &Ctx) -> Reports {
    let re = crate::org_re!(r"[^*\n\r ,]\*\*+ ");
    re.find_iter(c.text)
        .filter(|m| matches!(c.at(m.end()), Some(Type::Paragraph | Type::Headline)))
        .map(|m| (m.start(), "Possibly misplaced heading line".to_owned()))
        .collect()
}

fn duplicate_custom_id(c: &Ctx) -> Reports {
    duplicates(
        c,
        &[Type::NodeProperty],
        |n| {
            c.s(n, "key")
                .filter(|k| k.eq_ignore_ascii_case("CUSTOM_ID"))
                .and(c.s(n, "value"))
                .map(str::to_owned)
        },
        |n, _| c.begin(n),
        |k| format!("Duplicate CUSTOM_ID property \"{k}\""),
    )
}

fn duplicate_name(c: &Ctx) -> Reports {
    duplicates(
        c,
        element::ALL_ELEMENTS,
        |n| c.s(n, "name").map(str::to_owned),
        |n, name| {
            let b = c.begin(n);
            let re = Regex::new(&format!(
                r"(?im)^[ \t]*#\+[A-Za-z]+:[ \t]*{}[ \t]*$",
                regex::escape(name)
            ))
            .unwrap();
            re.find_at(c.text, b).map_or(b, |m| m.start())
        },
        |k| format!("Duplicate NAME \"{k}\""),
    )
}

fn duplicate_target(c: &Ctx) -> Reports {
    duplicates(
        c,
        &[Type::Target],
        |n| {
            c.s(n, "value")
                .map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
        },
        |n, _| c.begin(n),
        |k| format!("Duplicate target <<{}>>", k.join(" ")),
    )
}

fn duplicate_footnote_definition(c: &Ctx) -> Reports {
    duplicates(
        c,
        &[Type::FootnoteDefinition],
        |n| c.s(n, "label").map(str::to_owned),
        |n, _| c.post_aff(n),
        |k| format!("Duplicate footnote definition \"{k}\""),
    )
}

fn orphaned_affiliated_keywords(c: &Ctx) -> Reports {
    let attr = crate::org_re!(r"(?i)^ATTR_[-_A-Za-z0-9]+$");
    c.map(&[Type::Keyword], |k| {
        let key = c.s(k, "key")?;
        ((attr.is_match(key) || AFFILIATED.contains(&key)) && key != "RESULT" && key != "RESULTS")
            .then(|| {
                (
                    c.post_aff(k),
                    format!("Orphaned affiliated keyword: \"{key}\""),
                )
            })
    })
}

fn regular_keyword_before_affiliated(c: &Ctx) -> Reports {
    c.map(&[Type::Keyword], |k| {
        if c.t.post_blank(k) != 0 {
            return None;
        }
        let end = c.t.end(k)?;
        let next = element::at_point(c.text, c.t, end)?;
        let (nb, np) = (c.begin(next), c.post_aff(next));
        (nb != c.begin(k) && nb < np).then(|| {
            (
                c.begin(k),
                format!(
                    "Independent keyword {} may be confused with affiliated keywords below",
                    c.s(k, "key").unwrap_or_default()
                ),
            )
        })
    })
}

fn obsolete_affiliated_keywords(c: &Ctx) -> Reports {
    let re = ci(r"^[ \t]*#\+(DATA|LABEL|RESNAME|SOURCE|SRCNAME|TBLNAME|RESULT|HEADERS):");
    re.captures_iter(c.text)
        .filter_map(|m| {
            let end = m.get(0).unwrap().end();
            let el = element::at_point(c.text, c.t, end)?;
            if end >= c.post_aff(el) {
                return None;
            }
            let key = m[1].to_uppercase();
            let new = match key.as_str() {
                "HEADERS" => "HEADER",
                "RESULT" => "RESULTS",
                _ => "NAME",
            };
            Some((
                c.bol(end),
                format!("Obsolete affiliated keyword: \"{key}\".  Use \"{new}\" instead"),
            ))
        })
        .collect()
}

fn deprecated_export_blocks(c: &Ctx) -> Reports {
    let dep = [
        "ASCII", "BEAMER", "HTML", "LATEX", "MAN", "MARKDOWN", "MD", "ODT", "ORG", "TEXINFO",
    ];
    c.map(&[Type::SpecialBlock], |b| {
        let ty = c.s(b, "type")?;
        dep.iter().any(|d| d.eq_ignore_ascii_case(ty)).then(|| {
            (
                c.post_aff(b),
                format!("Deprecated syntax for export block.  Use \"BEGIN_EXPORT {ty}\" instead"),
            )
        })
    })
}

fn deprecated_header_syntax(c: &Ctx) -> Reports {
    let names: Vec<String> = super::babel::params::COMMON
        .iter()
        .map(|(n, _)| n.to_ascii_lowercase())
        .filter(|n| n != "dir")
        .collect();
    let alt = names
        .iter()
        .map(|n| regex::escape(n))
        .collect::<Vec<_>>()
        .join("|");
    let re = Regex::new(&format!(r"(?i)^({alt})[ \t]")).unwrap();
    c.map(&[Type::Keyword, Type::NodeProperty], |d| {
        let key = c.s(d, "key")?;
        if c.t.kind(d) == Type::Keyword {
            let v = c.s(d, "value")?;
            let m = (key == "PROPERTY").then(|| re.captures(v)).flatten()?;
            Some((
                c.begin(d),
                format!(
                    "Deprecated syntax for \"{}\".  Use header-args instead",
                    &m[1]
                ),
            ))
        } else {
            names.iter().any(|n| n.eq_ignore_ascii_case(key)).then(|| {
                (
                    c.begin(d),
                    format!("Deprecated syntax for \"{key}\".  Use :header-args: instead"),
                )
            })
        }
    })
}

fn missing_language_in_src_block(c: &Ctx) -> Reports {
    c.map(&[Type::SrcBlock], |b| {
        c.s(b, "language")
            .is_none()
            .then(|| (c.post_aff(b), "Missing language in source block".into()))
    })
}

/// A Babel backend or a mode Fred highlights (org-src-get-lang-mode-if-bound).
fn known_language(lang: &str) -> bool {
    super::babel::lang::executable(lang)
        || super::babel::lang::loaded().iter().any(|l| l == lang)
        || crate::highlight::has_syntax(&super::src::lang_extension(lang))
}

fn suspicious_language_in_src_block(c: &Ctx) -> Reports {
    c.map(&[Type::SrcBlock], |b| {
        let lang = c.s(b, "language")?;
        (!known_language(lang)).then(|| {
            (
                c.post_aff(b),
                format!("Unknown source block language: '{lang}'"),
            )
        })
    })
}

fn missing_backend_in_export_block(c: &Ctx) -> Reports {
    c.map(&[Type::ExportBlock], |b| {
        c.s(b, "type")
            .is_none()
            .then(|| (c.post_aff(b), "Missing backend in export block".into()))
    })
}

fn invalid_babel_call_block(c: &Ctx) -> Reports {
    let br = crate::org_re!(r"^\[.*\]$");
    c.map(&[Type::BabelCall], |b| {
        if c.s(b, "call").is_none() {
            Some((c.post_aff(b), "Invalid syntax in babel call block".into()))
        } else if c.s(b, "end-header").is_some_and(|h| br.is_match(h)) {
            Some((
                c.post_aff(b),
                "Babel call's end header must not be wrapped within brackets".into(),
            ))
        } else {
            None
        }
    })
}

fn deprecated_category_setup(c: &Ctx) -> Reports {
    let mut seen = false;
    c.map(&[Type::Keyword], |k| {
        if c.s(k, "key") != Some("CATEGORY") {
            return None;
        }
        if seen {
            return Some((
                c.post_aff(k),
                "Spurious CATEGORY keyword.  Set :CATEGORY: property instead".into(),
            ));
        }
        seen = true;
        None
    })
}

/// org-export-resolve-coderef succeeds.
fn coderef_known(c: &Ctx, r: &str) -> bool {
    let fmt = super::options::string("org-coderef-label-format", "(ref:%s)");
    c.t.map_first(
        c.t.root,
        Some(&[Type::ExampleBlock, Type::SrcBlock]),
        |_, el| {
            let f = c.s(el, "label-fmt").unwrap_or(&fmt);
            let re = format!(
                r"(?m)([ \t]*({})[ \t]*)$",
                regex::escape(f).replace("%s", &regex::escape(r))
            );
            let v = c.s(el, "value").unwrap_or_default().trim();
            Regex::new(&re).ok().filter(|re| re.is_match(v)).map(|_| ())
        },
    )
    .is_some()
}

fn invalid_coderef_link(c: &Ctx) -> Reports {
    c.map(&[Type::Link], |l| {
        let r = c.s(l, "path")?;
        (c.s(l, "type") == Some("coderef") && !coderef_known(c, r))
            .then(|| (c.begin(l), format!("Unknown coderef \"{r}\"")))
    })
}

fn invalid_custom_id_link(c: &Ctx) -> Reports {
    let ids: Vec<String> =
        c.t.map(c.t.root, Some(&[Type::Headline]), |t, h| {
            Some(
                [t.str(h, "ID"), t.str(h, "CUSTOM_ID")]
                    .into_iter()
                    .flatten()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
        })
        .concat();
    c.map(&[Type::Link], |l| {
        let p = c.s(l, "path")?;
        (c.s(l, "type") == Some("custom-id") && !ids.iter().any(|i| i == p))
            .then(|| (c.begin(l), format!("Unknown custom ID \"{p}\"")))
    })
}

#[derive(PartialEq, Clone)]
enum Cell {
    Headline(Vec<String>),
    CustomId(String),
    Target(Vec<String>),
    Other(Vec<String>),
}

/// org-export-search-cells.
fn search_cells(c: &Ctx, d: NodeId) -> Vec<Cell> {
    let up = |v: &str| {
        v.split_whitespace()
            .map(str::to_uppercase)
            .collect::<Vec<_>>()
    };
    match c.t.kind(d) {
        Type::Headline => {
            let raw = crate::org_re!(r"\[[0-9]*(?:%|/[0-9]*)\]")
                .replace_all(c.s(d, "raw-value").unwrap_or_default(), " ");
            let title = up(&raw);
            vec![Cell::Headline(title.clone()), Cell::Other(title)]
        }
        Type::Target => vec![Cell::Target(up(c.s(d, "value").unwrap_or_default()))],
        _ => {
            let name = c
                .s(d, "name")
                .map(str::to_owned)
                .or_else(|| match c.t.prop(d, "results") {
                    Value::Cons(a, _) => a.as_str().map(str::to_owned),
                    _ => None,
                });
            name.map(|n| {
                vec![Cell::Other(
                    n.split_whitespace().map(str::to_owned).collect(),
                )]
            })
            .unwrap_or_default()
        }
    }
}

/// org-export-string-to-search-cell.
fn string_cells(s: &str) -> Vec<Cell> {
    if let Some(h) = s.strip_prefix('*') {
        return vec![Cell::Headline(
            h.split_whitespace().map(str::to_uppercase).collect(),
        )];
    }
    if let Some(id) = s.strip_prefix('#') {
        return vec![Cell::CustomId(id.to_owned())];
    }
    let w: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
    let u: Vec<String> = w.iter().map(|x| x.to_uppercase()).collect();
    let mut v = vec![Cell::Target(w.clone()), Cell::Other(w)];
    for x in [Cell::Target(u.clone()), Cell::Other(u)] {
        if !v.contains(&x) {
            v.push(x);
        }
    }
    v
}

/// org-export-resolve-fuzzy-link: the destination, if any.
fn resolve_fuzzy(c: &Ctx, path: &str) -> Option<NodeId> {
    let cells = string_cells(path);
    let mut types = vec![Type::Target];
    types.extend_from_slice(element::ALL_ELEMENTS);
    let matches = c.t.map(c.t.root, Some(&types), |_, d| {
        search_cells(c, d)
            .iter()
            .any(|x| cells.contains(x))
            .then_some(d)
    });
    matches
        .iter()
        .copied()
        .find(|&d| c.t.kind(d) != Type::Headline)
        .or(matches.first().copied())
}

fn invalid_fuzzy_link(c: &Ctx) -> Reports {
    c.map(&[Type::Link], |l| {
        let p = c.s(l, "path")?;
        (c.s(l, "type") == Some("fuzzy") && resolve_fuzzy(c, p).is_none()).then(|| {
            (
                c.begin(l),
                format!(
                    "Unknown fuzzy location \"{}\"",
                    p.strip_prefix('*').unwrap_or(p)
                ),
            )
        })
    })
}

fn invalid_id_link(c: &Ctx) -> Reports {
    c.map(&[Type::Link], |l| {
        let id = c.s(l, "path")?;
        (c.s(l, "type") == Some("id") && !(c.id_known)(id))
            .then(|| (c.begin(l), format!("Unknown ID \"{id}\"")))
    })
}

fn confusing_brackets(c: &Ctx) -> Reports {
    c.map(&[Type::Link], |l| {
        let e = c.t.end(l)?;
        (c.text.as_bytes().get(e) == Some(&b']'))
            .then(|| (c.begin(l), "Trailing ']' after link end".into()))
    })
}

fn brackets_inside_description(c: &Ctx) -> Reports {
    c.map(&[Type::Link], |l| {
        let (b, e) = (c.t.contents_begin(l)?, c.t.contents_end(l)?);
        let d = &c.text[b..e];
        let count: i64 = d
            .chars()
            .map(|ch| {
                if ch == '[' {
                    1
                } else if ch == ']' {
                    -1
                } else {
                    0
                }
            })
            .sum();
        (count > 0).then(|| {
            (
                c.begin(l),
                format!("No closing ']' matches '[' in link description: {d}"),
            )
        })
    })
}

fn special_property_in_properties_drawer(c: &Ctx) -> Reports {
    c.map(&[Type::NodeProperty], |p| {
        let key = c.s(p, "key")?;
        SPECIAL_PROPERTIES
            .iter()
            .any(|s| s.eq_ignore_ascii_case(key))
            .then(|| {
                (
                    c.begin(p),
                    format!("Special property \"{key}\" found in a properties drawer"),
                )
            })
    })
}

fn obsolete_properties_drawer(c: &Ctx) -> Reports {
    // Upstream computes the elements before the drawer with `(assq d
    // (reverse contents))`, which never matches a node: the list is always
    // empty, so the message is always the "contents" one.
    c.map(&[Type::Drawer], |d| {
        (c.s(d, "drawer-name") == Some("PROPERTIES")).then(|| {
            (
                c.post_aff(d),
                "Incorrect contents for PROPERTIES drawer".into(),
            )
        })
    })
}

fn invalid_effort_property(c: &Ctx) -> Reports {
    c.map(&[Type::NodeProperty], |p| {
        let v = c.s(p, "value")?;
        (c.s(p, "key") == Some("EFFORT")
            && !v.trim().is_empty()
            && !super::time::duration::is_duration(v))
        .then(|| (c.begin(p), format!("Invalid effort duration format: {v:?}")))
    })
}

fn invalid_id_property(c: &Ctx) -> Reports {
    c.map(&[Type::NodeProperty], |p| {
        let v = c.s(p, "value")?;
        (c.s(p, "key") == Some("ID") && !v.trim().is_empty() && v.contains("::"))
            .then(|| (c.begin(p), format!("IDs should not include \"::\": {v:?}")))
    })
}

fn link_to_local_file(c: &Ctx) -> Reports {
    c.map(&[Type::Link], |l| {
        let ty = c.s(l, "type")?;
        if ty != "file" && ty != "attachment" {
            return None;
        }
        let mut path = c.s(l, "path")?.to_owned();
        if ty == "attachment"
            && let Some(i) = path.find("::")
        {
            path.truncate(i);
        }
        let file = if ty == "file" {
            path
        } else {
            (c.attach_expand)(&path).display().to_string()
        };
        let file = substitute_env(&file);
        (!remote_p(&file) && !c.file_exists(&file)).then(|| {
            let in_desc = c.t.lineage_of(l, &[Type::Link], false).is_some();
            (
                c.begin(l),
                if in_desc {
                    format!("Link to non-existent image file {file:?} in description")
                } else {
                    format!("Link to non-existent local file {file:?}")
                },
            )
        })
    })
}

fn non_existent_setupfile_parameter(c: &Ctx) -> Reports {
    c.map(&[Type::Keyword], |k| {
        if c.s(k, "key") != Some("SETUPFILE") {
            return None;
        }
        let f = strip_quotes(c.s(k, "value").unwrap_or_default());
        (!url_p(f) && !remote_p(f) && !c.file_exists(f))
            .then(|| (c.begin(k), format!("Non-existent setup file {f:?}")))
    })
}

fn wrong_include_link_parameter(c: &Ctx) -> Reports {
    let re = crate::org_re!(r#"^(".+?"|\S+)[ \t]*"#);
    c.map(&[Type::Keyword], |k| {
        if c.s(k, "key") != Some("INCLUDE") {
            return None;
        }
        let v = c.s(k, "value").unwrap_or_default();
        let Some(m) = re.captures(v) else {
            return Some((
                c.post_aff(k),
                "Missing location argument in INCLUDE keyword".into(),
            ));
        };
        let path = strip_quotes(&m[1]);
        let (file, search) = match path.find("::") {
            Some(i) => (
                &path[..i],
                Some(&path[i + 2..]).filter(|s| !s.trim().is_empty()),
            ),
            None => (path, None),
        };
        let file = Some(file).filter(|f| !f.trim().is_empty());
        if file.is_some_and(url_p) {
            return None;
        }
        if let Some(f) = file
            && !remote_p(f)
            && !c.file_exists(f)
        {
            return Some((
                c.post_aff(k),
                "Non-existent file argument in INCLUDE keyword".into(),
            ));
        }
        let search = search?;
        let text = match file {
            Some(f) => {
                let p = super::options::expand(&substitute_env(f));
                std::fs::read_to_string(if p.is_absolute() { p } else { c.dir.join(p) }).ok()?
            }
            None => c.text.to_owned(),
        };
        let mut ed = Editor::new(crate::buffer::Buffer::from_text(&text));
        ed.path = Some("include.org".into());
        super::attach(&mut ed);
        let old = super::options::get("org-link-search-must-match-exact-headline");
        super::options::put(
            "org-link-search-must-match-exact-headline",
            toml::Value::Boolean(true),
        );
        let found = super::links::search(&mut ed, search, None).is_ok();
        match old {
            Some(v) => super::options::put("org-link-search-must-match-exact-headline", v),
            None => super::options::remove("org-link-search-must-match-exact-headline"),
        }
        (!found).then(|| {
            (
                c.post_aff(k),
                format!("Invalid search part \"{search}\" in INCLUDE keyword"),
            )
        })
    })
}

fn obsolete_include_markup(c: &Ctx) -> Reports {
    let re = crate::org_re!(
        r#"(?i)^(?:".+"|\S+)[ \t]+(ASCII|BEAMER|HTML|LATEX|MAN|MARKDOWN|MD|ODT|ORG|TEXINFO)"#
    );
    c.map(&[Type::Keyword], |k| {
        if c.s(k, "key") != Some("INCLUDE") {
            return None;
        }
        let m = re.captures(c.s(k, "value")?)?;
        Some((
            c.post_aff(k),
            format!(
                "Obsolete markup \"{0}\" in INCLUDE keyword.  Use \"export {0}\" instead",
                &m[1]
            ),
        ))
    })
}

fn unknown_options_item(c: &Ctx) -> Reports {
    let re = crate::org_re!(r"(.+?):(\(.*?\)|\S+)?[ \t]*");
    let mut out = vec![];
    c.map(&[Type::Keyword], |k| -> Option<(usize, String)> {
        if c.s(k, "key") != Some("OPTIONS") {
            return None;
        }
        let v = c.s(k, "value").unwrap_or_default();
        let p = c.post_aff(k);
        for m in re.captures_iter(v) {
            let item = &m[1];
            if !OPTION_ITEMS.contains(&item) {
                out.push((p, format!("Unknown OPTIONS item \"{item}\"")));
            }
            if m.get(2).is_none() {
                out.push((p, format!("Missing value for option item {item:?}")));
            }
        }
        None
    });
    out
}

fn export_option_keywords(c: &Ctx) -> Reports {
    c.map(&[Type::NodeProperty], |n| {
        let prop = c.s(n, "key")?;
        let common = COMMON_OPTIONS.contains(&prop);
        let backend = BACKEND_OPTIONS.iter().find(|(o, _)| *o == prop);
        if !(common || backend.is_some()) || DEFAULT_PROPERTIES.contains(&prop) {
            return None;
        }
        let where_ = match backend.filter(|_| !common) {
            Some((_, [one])) => format!(" in {one} export backend"),
            Some((_, many)) => format!(" in ({}) export backends", many.join(" ")),
            None => String::new(),
        };
        Some((
            c.post_aff(n),
            format!(
                "Potentially misspelled {}export option \"{prop}\"{where_}.  Consider \"EXPORT_{prop}\".",
                if common { "global " } else { "" }
            ),
        ))
    })
}

fn placeholders(template: &str) -> Vec<usize> {
    let mut v: Vec<usize> = crate::org_re!(r"\$([1-9][0-9]*)")
        .captures_iter(template)
        .filter_map(|m| m[1].parse().ok())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn invalid_macro_argument_and_template(c: &Ctx) -> Reports {
    let mut reports = vec![];
    let mut defined: Vec<(String, String)> = vec![];
    c.map(&[Type::Keyword], |k| -> Option<(usize, String)> {
        if c.s(k, "key") != Some("MACRO") {
            return None;
        }
        let v = c.s(k, "value").unwrap_or_default();
        let name = v
            .split_whitespace()
            .next()
            .filter(|_| !v.starts_with(char::is_whitespace));
        let p = c.post_aff(k);
        match name {
            None => reports.push((p, "Missing name in MACRO keyword".to_owned())),
            Some(n) => {
                let template = v[n.len()..].trim();
                if template.is_empty() {
                    // Upstream passes the name as a stray `list' argument.
                    reports.push((p, "Missing template in macro \"%s\"".to_owned()));
                } else {
                    let args = placeholders(template);
                    if args != (1..=args.last().copied().unwrap_or(0)).collect::<Vec<_>>() {
                        reports.push((p, format!("Unused placeholders in macro \"{n}\"")));
                    }
                }
            }
        }
        None
    });
    // org-macro-initialize-templates: #+MACRO definitions and built-ins.
    for k in c.t.map(c.t.root, Some(&[Type::Keyword]), |t, k| {
        (t.str(k, "key") == Some("MACRO")).then_some(k)
    }) {
        let v = c.s(k, "value").unwrap_or_default();
        if let Some((n, tpl)) = v.trim().split_once(char::is_whitespace) {
            defined.push((n.to_owned(), tpl.trim().to_owned()));
        } else if !v.trim().is_empty() {
            defined.push((v.trim().to_owned(), String::new()));
        }
    }
    if let Some(g) = super::sexp::option("org-export-global-macros") {
        for e in g.list().unwrap_or(&[]) {
            if let (Some(n), Some(t)) = (e.car().and_then(super::sexp::Sexp::str), e.cdr().str()) {
                defined.push((n.to_owned(), t.to_owned()));
            }
        }
    }
    let mut builtins = vec![
        "date", "title", "email", "author", "keyword", "n", "property", "time",
    ];
    if c.file.is_some() {
        builtins.extend(["input-file", "modification-time"]);
    }
    c.map(&[Type::Macro], |m| -> Option<(usize, String)> {
        let name = c.s(m, "key").unwrap_or_default().to_owned();
        let args: Vec<String> =
            c.t.prop(m, "args")
                .as_list()
                .unwrap_or(&[])
                .iter()
                .map(|v| v.as_str().unwrap_or_default().to_owned())
                .collect();
        let lname = name.to_ascii_lowercase();
        let template: Option<String> =
            if ["author", "date", "email", "title", "results"].contains(&lname.as_str()) {
                Some("$1".into())
            } else {
                defined
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&name))
                    .map(|(_, t)| t.clone())
                    .or_else(|| {
                        builtins
                            .iter()
                            .any(|b| b.eq_ignore_ascii_case(&name))
                            .then(String::new)
                    })
            };
        let pos = c.begin(m);
        let Some(template) = template else {
            reports.push((pos, format!("Undefined macro {name:?}")));
            return None;
        };
        let arity = match lname.as_str() {
            "keyword" | "time" => (1, 1),
            "modification-time" | "property" => (1, 2),
            "n" => (0, 2),
            "input-file" => (0, 0),
            _ => match placeholders(&template).last() {
                None => (0, 0),
                Some(&m) => (m, m),
            },
        };
        let l = args.len();
        if l + 1 < arity.0 {
            reports.push((pos, format!("Missing arguments in macro {name:?}")));
        } else if l < arity.0 {
            reports.push((pos, format!("Missing argument in macro {name:?}")));
        } else if l > arity.1 + 1 {
            let spurious: Vec<&str> = args[arity.1..].iter().map(|a| a.trim()).collect();
            reports.push((
                pos,
                format!(
                    "Spurious arguments in macro {name:?}: {}",
                    spurious.join(", ")
                ),
            ));
        } else if l > arity.1 {
            reports.push((
                pos,
                format!("Spurious argument in macro {name:?}: {}", args[l - 1]),
            ));
        }
        None
    });
    reports
}

fn undefined_footnote_reference(c: &Ctx) -> Reports {
    let defs: Vec<String> = c.t.map(
        c.t.root,
        Some(&[Type::FootnoteDefinition, Type::FootnoteReference]),
        |t, f| {
            (t.kind(f) == Type::FootnoteDefinition || t.str(f, "type") == Some("inline"))
                .then(|| t.str(f, "label").map(str::to_owned))
                .flatten()
        },
    );
    c.map(&[Type::FootnoteReference], |f| {
        let label = c.s(f, "label")?;
        (c.s(f, "type") == Some("standard") && !defs.iter().any(|d| d == label)).then(|| {
            (
                c.begin(f),
                format!("Missing definition for footnote [{label}]"),
            )
        })
    })
}

fn unreferenced_footnote_definition(c: &Ctx) -> Reports {
    let refs: Vec<String> = c.t.map(c.t.root, Some(&[Type::FootnoteReference]), |t, f| {
        t.str(f, "label").map(str::to_owned)
    });
    c.map(&[Type::FootnoteDefinition], |f| {
        let label = c.s(f, "label")?;
        (!refs.iter().any(|r| r == label)).then(|| {
            (
                c.post_aff(f),
                format!("No reference for footnote definition [{label}]"),
            )
        })
    })
}

fn mismatched_planning_repeaters(c: &Ctx) -> Reports {
    c.map(&[Type::Planning], |e| {
        let s = c.t.prop(e, "scheduled").as_node()?;
        let d = c.t.prop(e, "deadline").as_node()?;
        let ty = |n| {
            c.s(n, "repeater-type")
                .filter(|t| *t == "cumulate" || *t == "catch-up")
        };
        let (st, dt) = (ty(s)?, ty(d)?);
        let (sv, dv) = (c.t.int(s, "repeater-value")?, c.t.int(d, "repeater-value")?);
        (sv > 0
            && dv > 0
            && !(st == dt && c.s(s, "repeater-unit") == c.s(d, "repeater-unit") && sv == dv))
            .then(|| {
                (
                    c.begin(e),
                    "Different repeaters in SCHEDULED and DEADLINE timestamps.".into(),
                )
            })
    })
}

fn misplaced_planning_info(c: &Ctx) -> Reports {
    let re = ci(r"^[ \t]*((?:CLOSED|DEADLINE|SCHEDULED):)");
    re.find_iter(c.text)
        .filter(|m| {
            !matches!(
                c.at(m.end()),
                Some(
                    Type::CommentBlock
                        | Type::ExampleBlock
                        | Type::ExportBlock
                        | Type::Planning
                        | Type::SrcBlock
                        | Type::VerseBlock
                )
            )
        })
        .map(|m| (c.bol(m.end()), "Misplaced planning info line".into()))
        .collect()
}

fn incomplete_drawer(c: &Ctx) -> Reports {
    let re = crate::org_re!(r"(?m)^[ \t]*:((?:\w|[-_])+):[ \t]*$");
    let mut out = vec![];
    let mut from = 0;
    while let Some(m) = re.find_at(c.text, from) {
        from = m.end();
        let name = m.as_str().trim();
        let Some(el) = element::at_point(c.text, c.t, m.end()) else {
            continue;
        };
        match c.t.kind(el) {
            Type::Drawer => {
                if let Some(end) = c.t.contents_end(el) {
                    let mut f = m.end();
                    while let Some(n) = re.find_at(c.text, f).filter(|n| n.end() <= end) {
                        out.push((
                            c.bol(n.end()),
                            format!("Possible misleading drawer entry {:?}", n.as_str().trim()),
                        ));
                        f = n.end();
                    }
                }
                from = from.max(c.t.end(el).unwrap_or(from));
            }
            Type::PropertyDrawer => from = from.max(c.t.end(el).unwrap_or(from)),
            Type::CommentBlock
            | Type::ExampleBlock
            | Type::ExportBlock
            | Type::SrcBlock
            | Type::VerseBlock => {}
            _ => out.push((
                c.bol(m.end()),
                format!("Possible incomplete drawer {name:?}"),
            )),
        }
    }
    out
}

fn indented_diary_sexp(c: &Ctx) -> Reports {
    crate::org_re!(r"(?m)^[ \t]+%%\(")
        .find_iter(c.text)
        .filter(|m| {
            !matches!(
                c.at(m.end()),
                Some(
                    Type::CommentBlock
                        | Type::DiarySexp
                        | Type::ExampleBlock
                        | Type::ExportBlock
                        | Type::SrcBlock
                        | Type::VerseBlock
                )
            )
        })
        .map(|m| (c.bol(m.end()), "Possible indented diary-sexp".into()))
        .collect()
}

fn invalid_block(c: &Ctx) -> Reports {
    let re = ci(r"^[ \t]*#\+(BEGIN|END)(?::|_\S*)?[ \t]*");
    re.captures_iter(c.text)
        .filter_map(|m| {
            let end = m.get(0).unwrap().end();
            let bol = c.bol(end);
            let eol = c.text[end..].find('\n').map_or(c.text.len(), |i| end + i);
            let name = c.text[bol..eol].trim();
            if m[1].eq_ignore_ascii_case("END") && end != eol {
                Some((bol, format!("Invalid block closing line \"{name}\"")))
            } else if !matches!(
                c.at(end),
                Some(
                    Type::CenterBlock
                        | Type::CommentBlock
                        | Type::DynamicBlock
                        | Type::ExampleBlock
                        | Type::ExportBlock
                        | Type::QuoteBlock
                        | Type::SpecialBlock
                        | Type::SrcBlock
                        | Type::VerseBlock
                )
            ) {
                Some((bol, format!("Possible incomplete block \"{name}\"")))
            } else {
                None
            }
        })
        .collect()
}

fn invalid_keyword_syntax(c: &Ctx) -> Reports {
    let re = crate::org_re!(r"(?m)^[ \t]*#\+([^\s:]*)(?: |$)");
    let exception = ci(r"^[ \t]*#\+(CAPTION|RESULTS)(\[.*\])?:(?: |$)");
    re.captures_iter(c.text)
        .filter_map(|m| {
            let name = &m[1];
            let start = m.get(0).unwrap().start();
            let low = name.to_ascii_lowercase();
            (!(low.starts_with("begin")
                || low.starts_with("end")
                || exception
                    .find_at(c.text, start)
                    .is_some_and(|e| e.start() == start)))
            .then(|| {
                (
                    start,
                    format!("Possible missing colon in keyword \"{name}\""),
                )
            })
        })
        .collect()
}

fn invalid_image_alignment(c: &Ctx) -> Reports {
    let center = crate::org_re!(r":center\s+(\S+)");
    let align = crate::org_re!(r":align\s+(\S+)");
    let mut out = vec![];
    c.map(&[Type::Paragraph], |p| -> Option<(usize, String)> {
        let s =
            c.t.prop(p, "attr_org")
                .as_list()?
                .first()?
                .as_str()?
                .to_owned();
        let b = c.begin(p);
        let mut r = vec![];
        if let Some(m) = align.captures(&s)
            && !["left", "center", "right"].contains(&&m[1])
        {
            r.push((
                b,
                format!(
                    "\"{}\" not a supported value for #+ATTR_ORG keyword attribute \":align\".",
                    &m[1]
                ),
            ));
        }
        if let Some(m) = center.captures(&s)
            && &m[1] != "t"
        {
            r.push((
                b,
                format!(
                    "\"{}\" not a supported value for #+ATTR_ORG keyword attribute \":center\".",
                    &m[1]
                ),
            ));
        }
        r.reverse();
        out.extend(r);
        None
    });
    out
}

fn extraneous_element_in_footnote_section(c: &Ctx) -> Reports {
    let allowed = [
        Type::Comment,
        Type::CommentBlock,
        Type::FootnoteDefinition,
        Type::PropertyDrawer,
        Type::Section,
    ];
    let types: Vec<Type> = element::ALL_ELEMENTS
        .iter()
        .copied()
        .filter(|t| !allowed.contains(t))
        .collect();
    c.map(&[Type::Headline], |h| {
        if !c.t.flag(h, "footnote-section-p") {
            return None;
        }
        let bad = c.t.contents(h).iter().any(|&child| {
            !c.t.map_with(
                child,
                Some(&types),
                true,
                &[Type::FootnoteDefinition, Type::PropertyDrawer],
                false,
                |t, e| {
                    if t.kind(e) == Type::Headline && t.flag(e, "commentedp") {
                        element::Visit::Nil
                    } else {
                        element::Visit::Value(())
                    }
                },
            )
            .is_empty()
        });
        bad.then(|| {
            (
                c.begin(h),
                "Extraneous elements in footnote section are not exported".into(),
            )
        })
    })
}

fn quote_section(c: &Ctx) -> Reports {
    let comment = super::options::string("org-comment-string", "COMMENT");
    c.map(&[Type::Headline, Type::Inlinetask], |h| {
        let title = c.s(h, "raw-value")?;
        (title.starts_with("QUOTE ") || title.starts_with(&format!("{comment} QUOTE ")))
            .then(|| (c.begin(h), "Deprecated QUOTE section".into()))
    })
}

fn file_application(c: &Ctx) -> Reports {
    c.map(&[Type::Link], |l| {
        let app = c.s(l, "application")?;
        Some((c.begin(l), format!("Deprecated \"file+{app}\" link type")))
    })
}

fn percent_encoding_link_escape(c: &Ctx) -> Reports {
    let re = crate::org_re!(r"%(..)?");
    c.map(&[Type::Link], |l| {
        if c.s(l, "format") != Some("bracket") {
            return None;
        }
        let uri = c.s(l, "path")?;
        let obsolete = re.captures_iter(uri).all(|m| {
            m.get(1)
                .is_some_and(|g| ["25", "5B", "5D", "20"].contains(&g.as_str()))
        }) && uri.contains('%');
        obsolete.then(|| {
            (
                c.begin(l),
                "Link escaped with obsolete percent-encoding syntax".into(),
            )
        })
    })
}

/// Headers of a datum, parsed without evaluation.
fn headers(c: &Ctx, d: NodeId) -> Vec<(String, super::sexp::Sexp)> {
    let parse = |s: Option<&str>| {
        s.and_then(|s| super::babel::params::parse_header_arguments(s, true).ok())
            .unwrap_or_default()
    };
    match c.t.kind(d) {
        Type::BabelCall | Type::InlineBabelCall => {
            [parse(c.s(d, "inside-header")), parse(c.s(d, "end-header"))].concat()
        }
        Type::InlineSrcBlock => parse(c.s(d, "parameters")),
        _ => {
            let mut v = parse(c.s(d, "parameters"));
            for h in c.t.prop(d, "header").as_list().unwrap_or(&[]) {
                v.extend(parse(h.as_str()));
            }
            v
        }
    }
}

fn header_pos(c: &Ctx, d: NodeId) -> usize {
    c.t.post_affiliated(d).or(c.t.begin(d)).unwrap_or(0)
}

fn wrong_header_argument(c: &Ctx) -> Reports {
    let mut out = vec![];
    let common: Vec<&str> = super::babel::params::COMMON
        .iter()
        .map(|(n, _)| *n)
        .collect();
    let allowed = |lang: Option<&str>| -> Vec<String> {
        let mut v: Vec<String> = common.iter().map(|s| (*s).to_owned()).collect();
        let langs = match lang {
            Some(l) => vec![l.to_owned()],
            None => super::babel::lang::loaded(),
        };
        for l in langs {
            v.extend(
                super::babel::params::lang_headers(&l)
                    .into_iter()
                    .map(|(n, _)| n.to_owned()),
            );
        }
        v
    };
    let mut verify = |d: NodeId, lang: Option<&str>, hs: Vec<(String, super::sexp::Sexp)>| {
        let ok = allowed(lang);
        let p = header_pos(c, d);
        for (h, _) in hs {
            match h.strip_prefix(':') {
                None => out.push((p, format!("Missing colon in header argument \"{h}\""))),
                Some(n) if ok.iter().any(|a| a.eq_ignore_ascii_case(n)) => {}
                Some(_) => out.push((p, format!("Unknown header argument \"{h}\""))),
            }
        }
    };
    let parse = |s: &str| super::babel::params::parse_header_arguments(s, true).unwrap_or_default();
    let key_re = crate::org_re!(r"(?i)^HEADER-ARGS(?::(\S+))?\+?");
    let val_re = crate::org_re!(r"^header-args(?::(\S+))?\+? *");
    let datums = c.t.map(
        c.t.root,
        Some(&[
            Type::BabelCall,
            Type::InlineBabelCall,
            Type::InlineSrcBlock,
            Type::Keyword,
            Type::NodeProperty,
            Type::SrcBlock,
        ]),
        |_, d| Some(d),
    );
    for d in datums {
        match c.t.kind(d) {
            Type::BabelCall | Type::InlineBabelCall | Type::InlineSrcBlock | Type::SrcBlock => {
                let lang = c.s(d, "language").map(str::to_owned);
                verify(d, lang.as_deref(), headers(c, d))
            }
            Type::Keyword if c.s(d, "key") == Some("PROPERTY") => {
                let v = c.s(d, "value").unwrap_or_default();
                if let Some(m) = val_re.captures(v) {
                    let lang = m.get(1).map(|x| x.as_str().to_owned());
                    verify(d, lang.as_deref(), parse(&v[m.get(0).unwrap().end()..]));
                }
            }
            Type::NodeProperty => {
                let k = c.s(d, "key").unwrap_or_default();
                if let Some(m) = key_re.captures(k) {
                    let lang = m.get(1).map(|x| x.as_str().to_owned());
                    verify(
                        d,
                        lang.as_deref(),
                        parse(c.s(d, "value").unwrap_or_default()),
                    );
                }
            }
            _ => {}
        }
    }
    out
}

fn empty_header_argument(c: &Ctx) -> Reports {
    let mut out = vec![];
    c.map(
        &[
            Type::BabelCall,
            Type::InlineBabelCall,
            Type::InlineSrcBlock,
            Type::SrcBlock,
        ],
        |d| -> Option<(usize, String)> {
            for (h, v) in headers(c, d) {
                if v.is_nil() {
                    out.push((
                        header_pos(c, d),
                        format!("Empty value in header argument \"{h}\""),
                    ));
                }
            }
            None
        },
    );
    out
}

fn wrong_header_value(c: &Ctx) -> Reports {
    use super::sexp::Sexp;
    let mut out = vec![];
    c.map(
        &[
            Type::BabelCall,
            Type::InlineBabelCall,
            Type::InlineSrcBlock,
            Type::SrcBlock,
        ],
        |d| -> Option<(usize, String)> {
            let lang = c.s(d, "language");
            let mut allowed = lang
                .map(super::babel::params::lang_headers)
                .unwrap_or_default();
            allowed.extend(super::babel::params::COMMON.iter().copied());
            let text = match c.t.kind(d) {
                Type::SrcBlock => {
                    let mut parts = vec![c.s(d, "parameters").unwrap_or_default().to_owned()];
                    parts.extend(
                        c.t.prop(d, "header")
                            .as_list()
                            .unwrap_or(&[])
                            .iter()
                            .filter_map(|h| h.as_str().map(str::to_owned)),
                    );
                    parts.join(" ")
                }
                Type::InlineSrcBlock => c.s(d, "parameters").unwrap_or_default().to_owned(),
                _ => format!(
                    "{} {}",
                    c.s(d, "inside-header").unwrap_or_default(),
                    c.s(d, "end-header").unwrap_or_default()
                ),
            };
            let hs =
                super::babel::params::parse_header_arguments(text.trim(), true).unwrap_or_default();
            let p = header_pos(c, d);
            for (h, value) in hs {
                let name = h.strip_prefix(':').unwrap_or(&h);
                let Some(groups) = allowed
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .and_then(|(_, g)| *g)
                else {
                    continue;
                };
                let values: Vec<String> = match &value {
                    Sexp::Str(s) => s.split_whitespace().map(str::to_owned).collect(),
                    v => vec![v.to_string()],
                };
                let mut seen: Vec<(usize, String)> = vec![];
                'values: for v in values {
                    let mut valid = false;
                    for (gi, group) in groups.iter().enumerate() {
                        if !group.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
                            if group.contains(&":any") {
                                valid = true;
                                seen.push((gi, v.clone()));
                            }
                        } else if let Some((_, prev)) = seen.iter().find(|(g, _)| *g == gi) {
                            out.push((
                                p,
                                format!("Forbidden combination in header \"{h}\": {prev}, {v}"),
                            ));
                            continue 'values;
                        } else {
                            seen.push((gi, v.clone()));
                            valid = true;
                        }
                    }
                    if !valid {
                        out.push((p, format!("Unknown value \"{v}\" for header \"{h}\"")));
                    }
                }
            }
            None
        },
    );
    out
}

fn named_result(c: &Ctx) -> Reports {
    c.map(element::ALL_ELEMENTS, |el| {
        let Value::Cons(result, _) = c.t.prop(el, "results") else { return None };
        let name = c.s(el, "name")?;
        let origin = match result.as_str().filter(|s| !s.trim().is_empty()) {
            Some(r) => resolve_fuzzy(c, r),
            None => {
                let parent = c.t.parent(el)?;
                let sib = c.t.contents(parent);
                let i = sib.iter().position(|&x| x == el)?;
                i.checked_sub(1).map(|j| sib[j])
            }
        }?;
        if c.t.kind(origin) != Type::SrcBlock {
            return None;
        }
        let exports = headers(c, origin)
            .into_iter()
            .rev()
            .find(|(k, _)| k == ":exports")
            .and_then(|(_, v)| v.str().map(str::to_owned))
            .unwrap_or_else(|| "code".into());
        (exports != "results" && exports != "both").then(|| {
            (
                c.begin(el),
                format!(
                    "Links to \"{name}\" will not be valid during export unless the parent source block has :exports results or both"
                ),
            )
        })
    })
}

fn spurious_colons(c: &Ctx) -> Reports {
    c.map(&[Type::Headline, Type::Inlinetask], |h| {
        let tags = c.t.prop(h, "tags").as_list()?;
        tags.iter()
            .any(|t| t.as_str() == Some(""))
            .then(|| (c.begin(h), "Tags contain a spurious colon".into()))
    })
}

fn non_existent_bibliography(c: &Ctx) -> Reports {
    c.map(&[Type::Keyword], |k| {
        if c.s(k, "key") != Some("BIBLIOGRAPHY") {
            return None;
        }
        let f = strip_quotes(c.s(k, "value")?);
        (!remote_p(f) && !c.file_exists(f))
            .then(|| (c.begin(k), format!("Non-existent bibliography {f:?}")))
    })
}

fn missing_print_bibliography(c: &Ctx) -> Reports {
    let cites =
        c.t.map_first(c.t.root, Some(&[Type::Citation]), |_, n| Some(n))
            .is_some();
    let printed =
        c.t.map_first(c.t.root, Some(&[Type::Keyword]), |t, k| {
            (t.str(k, "key") == Some("PRINT_BIBLIOGRAPHY")).then_some(())
        })
        .is_some();
    if cites && !printed {
        vec![(
            c.text.len(),
            "Possibly missing \"PRINT_BIBLIOGRAPHY\" keyword".into(),
        )]
    } else {
        vec![]
    }
}

/// Citation processors Org provides (org-cite-try-load-processor).
const CITE_PROCESSORS: &[&str] = &["basic", "biblatex", "bibtex", "csl", "natbib"];

fn invalid_cite_export_declaration(c: &Ctx) -> Reports {
    c.map(&[Type::Keyword], |k| {
        if c.s(k, "key") != Some("CITE_EXPORT") {
            return None;
        }
        let v = c.s(k, "value").unwrap_or_default();
        let src = c.begin(k);
        if v.is_empty() {
            return Some((src, "Missing export processor name".into()));
        }
        // org-cite-read-processor-declaration.
        let parsed = super::sexp::read(&format!("({v})")).ok();
        let items = parsed
            .as_ref()
            .and_then(|p| p.list().map(<[_]>::to_vec))
            .unwrap_or_default();
        let str_or_nil = |x: Option<&super::sexp::Sexp>| {
            x.is_none_or(|x| x.is_nil() || matches!(x, super::sexp::Sexp::Str(_)))
        };
        match items.first() {
            Some(super::sexp::Sexp::Sym(name))
                if items.len() <= 3 && str_or_nil(items.get(1)) && str_or_nil(items.get(2)) =>
            {
                (!CITE_PROCESSORS.contains(&name.as_str()))
                    .then(|| (src, format!("Unknown cite export processor {name}")))
            }
            _ => Some((src, "Invalid cite export processor declaration".into())),
        }
    })
}

fn incomplete_citation(c: &Ctx) -> Reports {
    let re = crate::org_re!(r"\[cite(?:/([/_[:alnum:]-]+))?:[ \t\n]*");
    c.map(&[Type::PlainText], |t| {
        let text = c.t.plain_text(t)?;
        if !re.is_match(text) {
            return None;
        }
        let p = c.t.parent(t)?;
        Some((
            c.t.contents_begin(p).unwrap_or(0),
            "Possibly incomplete citation markup".into(),
        ))
    })
}

fn item_number(c: &Ctx) -> Reports {
    use super::list::structure::{Item, Struct};
    c.map(&[Type::Item], |item| {
        if c.t.prop(item, "counter").truthy() {
            return None;
        }
        let bullet = c.s(item, "bullet")?;
        let n = if let Some(ch) = bullet.chars().find(|ch| ch.is_ascii_alphabetic()) {
            ch.to_ascii_uppercase() as i64 - 64
        } else {
            crate::org_re!(r"[0-9]+").find(bullet)?.as_str().parse().ok()?
        };
        let items: Vec<Item> = c
            .t
            .prop(item, "structure")
            .as_list()?
            .iter()
            .filter_map(|e| {
                let l = e.as_list()?;
                let s = |i: usize| l.get(i).and_then(Value::as_str).map(str::to_owned);
                Some(Item {
                    pos: l.first()?.as_int()? as usize,
                    ind: l.get(1)?.as_int()? as usize,
                    bullet: s(2)?,
                    counter: s(3),
                    checkbox: s(4),
                    tag: s(5),
                    end: l.get(6)?.as_int()? as usize,
                })
            })
            .collect();
        let st = Struct(items);
        let true_n = *st.item_number(c.begin(item), &st.prevs(), &st.parents()).last()?;
        (n != true_n).then(|| {
            (
                c.begin(item),
                format!(
                    "Bullet counter \"{bullet}\" is not the same with item position {true_n}.  Consider adding manual [@{n}] counter."
                ),
            )
        })
    })
}

fn priority(c: &Ctx) -> Reports {
    let (hi, lo, _) = c.st.priorities;
    let valid = |p: i64, ignore_bounds: bool| {
        ((0..=64).contains(&p) || (65..=90).contains(&p))
            && (ignore_bounds || (hi as i64 <= p && p <= lo as i64))
    };
    let bad = crate::org_re!(r"^\[#([^\]\[]*)(\]*)");
    c.map(&[Type::Headline], |h| {
        if let Some(p) = c.t.int(h, "priority") {
            (!valid(p, false) && valid(p, true)).then(|| {
                let s = if p <= 64 {
                    p.to_string()
                } else {
                    char::from_u32(p as u32).unwrap_or('?').to_string()
                };
                (c.begin(h), format!("Out-of-bounds priority '{s}'"))
            })
        } else {
            let raw = c.s(h, "raw-value")?;
            let m = bad.captures(raw)?;
            Some((
                c.begin(h),
                if m[2].is_empty() {
                    format!("Malformed priority '{}'", &m[0])
                } else {
                    format!("Invalid priority '{}'", &m[1])
                },
            ))
        }
    })
}

fn latex_dollar_ambiguous(c: &Ctx) -> Reports {
    crate::org_re!(r"\$\.[0-9]")
        .find_iter(c.text)
        .filter(|m| {
            element::context(c.text, c.t, m.end()).is_some_and(|ctx| {
                let k = c.t.kind(ctx);
                k == Type::LatexFragment || element::restriction(k).contains(&Type::LatexFragment)
            })
        })
        .map(|m| (m.end(), "$ symbol potentially matching LaTeX fragment boundary.  Consider using \\dollar entity.".into()))
        .collect()
}

fn timestamp_syntax(c: &Ctx) -> Reports {
    c.map(&[Type::Timestamp], |ts| {
        let expected = element::interpret(c.t, ts);
        let actual = &c.text[c.begin(ts)..c.t.end(ts)?];
        (expected != actual).then(|| {
            (
                c.begin(ts),
                format!("Potentially malformed timestamp {actual}.  Parsed as: {expected}"),
            )
        })
    })
}

fn clock_syntax(c: &Ctx) -> Reports {
    c.map(&[Type::Clock], |cl| {
        let expected = element::interpret(c.t, cl).trim_end().to_owned();
        let actual = c.text[c.begin(cl)..c.t.end(cl)?].trim();
        (expected != actual).then(|| {
            (
                c.begin(cl),
                format!(
                    "Potentially malformed CLOCK: line\n           {actual}\nParsed as: {expected}"
                ),
            )
        })
    })
}

fn inactive_planning(c: &Ctx) -> Reports {
    c.map(&[Type::Planning], |p| {
        let inactive = |k: &str| {
            c.t.prop(p, k)
                .as_node()
                .and_then(|n| c.s(n, "type"))
                .is_some_and(|t| t == "inactive" || t == "inactive-range")
        };
        if inactive("scheduled") {
            Some((
                c.begin(p),
                "Inactive timestamp in SCHEDULED will not appear in agenda.".into(),
            ))
        } else if inactive("deadline") {
            Some((
                c.begin(p),
                "Inactive timestamp in DEADLINE will not appear in agenda.".into(),
            ))
        } else {
            None
        }
    })
}

fn beamer_frame(c: &Ctx) -> Reports {
    let env = super::options::string("org-beamer-frame-environment", "orgframe");
    let re = Regex::new(&format!(r"(?i)\\(begin|end)\{{{}\}}", regex::escape(&env))).unwrap();
    re.find_iter(c.text)
        .map(|m| {
            (
                m.start(),
                "Beamer frame name may cause error when exporting.  Consider customizing `org-beamer-frame-environment'.".into(),
            )
        })
        .collect()
}

fn run_checker(name: &str, c: &Ctx) -> Reports {
    match name {
        "planning-inactive" => inactive_planning(c),
        "clock-syntax" => clock_syntax(c),
        "timestamp-syntax" => timestamp_syntax(c),
        "beamer-frame" => beamer_frame(c),
        "LaTeX-$" => latex_dollar_ambiguous(c),
        "priority" => priority(c),
        "item-number" => item_number(c),
        "incomplete-citation" => incomplete_citation(c),
        "invalid-cite-export-declaration" => invalid_cite_export_declaration(c),
        "missing-print-bibliography" => missing_print_bibliography(c),
        "non-existent-bibliography" => non_existent_bibliography(c),
        "spurious-colons" => spurious_colons(c),
        "percent-encoding-link-escape" => percent_encoding_link_escape(c),
        "file-application" => file_application(c),
        "quote-section" => quote_section(c),
        "indented-diary-sexp" => indented_diary_sexp(c),
        "incomplete-drawer" => incomplete_drawer(c),
        "misplaced-planning-info" => misplaced_planning_info(c),
        "mismatched-planning-repeaters" => mismatched_planning_repeaters(c),
        "invalid-block" => invalid_block(c),
        "invalid-image-alignment" => invalid_image_alignment(c),
        "invalid-keyword-syntax" => invalid_keyword_syntax(c),
        "extraneous-element-in-footnote-section" => extraneous_element_in_footnote_section(c),
        "unreferenced-footnote-definition" => unreferenced_footnote_definition(c),
        "undefined-footnote-reference" => undefined_footnote_reference(c),
        "invalid-id-property" => invalid_id_property(c),
        "invalid-effort-property" => invalid_effort_property(c),
        "obsolete-properties-drawer" => obsolete_properties_drawer(c),
        "special-property-in-properties-drawer" => special_property_in_properties_drawer(c),
        "invalid-macro-argument-and-template" => invalid_macro_argument_and_template(c),
        "misspelled-export-option" => export_option_keywords(c),
        "unknown-options-item" => unknown_options_item(c),
        "obsolete-include-markup" => obsolete_include_markup(c),
        "wrong-include-link-parameter" => wrong_include_link_parameter(c),
        "non-existent-setupfile-parameter" => non_existent_setupfile_parameter(c),
        "link-to-local-file" => link_to_local_file(c),
        "unclosed-brackets-in-link-description" => brackets_inside_description(c),
        "trailing-bracket-after-link" => confusing_brackets(c),
        "invalid-id-link" => invalid_id_link(c),
        "invalid-fuzzy-link" => invalid_fuzzy_link(c),
        "invalid-custom-id-link" => invalid_custom_id_link(c),
        "invalid-coderef-link" => invalid_coderef_link(c),
        "deprecated-category-setup" => deprecated_category_setup(c),
        "empty-header-argument" => empty_header_argument(c),
        "named-result" => named_result(c),
        "wrong-header-value" => wrong_header_value(c),
        "wrong-header-argument" => wrong_header_argument(c),
        "invalid-babel-call-block" => invalid_babel_call_block(c),
        "missing-backend-in-export-block" => missing_backend_in_export_block(c),
        "suspicious-language-in-src-block" => suspicious_language_in_src_block(c),
        "missing-language-in-src-block" => missing_language_in_src_block(c),
        "deprecated-header-syntax" => deprecated_header_syntax(c),
        "deprecated-export-blocks" => deprecated_export_blocks(c),
        "obsolete-affiliated-keywords" => obsolete_affiliated_keywords(c),
        "combining-keywords-with-affiliated" => regular_keyword_before_affiliated(c),
        "orphaned-affiliated-keywords" => orphaned_affiliated_keywords(c),
        "duplicate-footnote-definition" => duplicate_footnote_definition(c),
        "duplicate-target" => duplicate_target(c),
        "duplicate-name" => duplicate_name(c),
        "duplicate-custom-id" => duplicate_custom_id(c),
        "misplaced-heading" => misplaced_heading(c),
        _ => vec![],
    }
}

/// One report: line (1-based), trust, message, checker.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub pos: usize,
    pub line: usize,
    pub trust: &'static str,
    pub message: String,
    pub checker: &'static str,
}

/// org-lint--generate-reports.
pub fn generate(c: &Ctx, checkers: &[&'static str]) -> Vec<Report> {
    let mut all: Vec<Report> = vec![];
    for &(name, trust, _) in CHECKERS.iter().filter(|(n, _, _)| checkers.contains(n)) {
        for (pos, message) in run_checker(name, c) {
            all.push(Report {
                pos,
                line: 0,
                trust,
                message,
                checker: name,
            });
        }
    }
    all.sort_by_key(|r| r.pos);
    for r in &mut all {
        let p = c.bol(r.pos.min(c.text.len()));
        r.line = c.text[..p].matches('\n').count() + 1;
    }
    all
}

/// Lint the text of `ed`.
pub fn lint_editor(
    ed: &Editor,
    checkers: &[&'static str],
    id_known: &dyn Fn(&str) -> bool,
) -> Vec<Report> {
    let text = ed.buf.text();
    let st = super::settings(ed);
    let t = element::parse_buffer(&text, &st);
    let file = ed.path.as_ref().and_then(|p| std::path::absolute(p).ok());
    let dir = file
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let attach = |p: &str| super::attach::expand(ed, p);
    let c = Ctx {
        text: &text,
        t: &t,
        st: &st,
        dir,
        file,
        attach_expand: &attach,
        id_known,
    };
    generate(&c, checkers)
}

// ---- the report buffer ----

pub const BUFFER: &str = "*Org Lint*";

/// The report buffer's keys (org-lint--report-mode-map, with tabulated
/// list's `g` and `q`).
pub const MAP: &[(&str, &str)] = &[
    ("RET", "org-lint--jump-to-source"),
    ("TAB", "org-lint--show-source"),
    ("C-j", "org-lint--show-source"),
    ("h", "org-lint--hide-checker"),
    ("i", "org-lint--ignore-checker"),
    ("g", "org-lint--refresh-reports"),
    ("q", "quit-window"),
];

/// Keys Vim keeps in the report buffer: everything MAP does not bind.
pub fn vim_keeps(k: crate::key::Key) -> bool {
    use crate::key::KeyCode;
    let bound = match k.code {
        KeyCode::Enter | KeyCode::Tab => !k.ctrl && !k.alt,
        KeyCode::Char('j') => k.ctrl,
        KeyCode::Char('h' | 'i' | 'g' | 'q') => !k.ctrl && !k.alt,
        _ => false,
    };
    !bound
}

struct State {
    source: usize,
    checkers: Vec<&'static str>,
    reports: Vec<Report>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn render(reports: &[Report]) -> String {
    if reports.is_empty() {
        return String::new();
    }
    reports
        .iter()
        .map(|r| {
            format!(
                "{:>6} {:<5} {}",
                r.line,
                r.trust,
                r.message.replace('\n', " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate reports for buffer `source` and show them.
fn display(s: &mut crate::session::Session, source: usize, checkers: Vec<&'static str>) {
    let reports = {
        let mut known_ids: Vec<(String, bool)> = vec![];
        let ids: Vec<String> = s.org_with_buffer(source, |ed| {
            let text = ed.buf.text();
            crate::org_re!(r"\[\[id:([^\]]+)\]")
                .captures_iter(&text)
                .map(|c| c[1].to_owned())
                .collect()
        });
        for id in ids {
            let found = super::find_id_file(s, &id).is_some();
            known_ids.push((id, found));
        }
        let id_known = move |id: &str| known_ids.iter().any(|(i, f)| i == id && *f);
        s.org_with_buffer(source, |ed| lint_editor(ed, &checkers, &id_known))
    };
    let text = render(&reports);
    let n = reports.len();
    STATE.with(|st| {
        *st.borrow_mut() = Some(State {
            source,
            checkers,
            reports,
        })
    });
    s.org_temp_buffer(BUFFER, &text, false);
    s.ed.set_msg(if n == 0 {
        "Org linting process completed".to_owned()
    } else {
        format!("Org linting process completed: {n} reports")
    });
}

fn current(ed: &Editor) -> Option<Report> {
    STATE.with(|st| {
        st.borrow()
            .as_ref()
            .and_then(|s| s.reports.get(ed.cur.line).cloned())
    })
}

fn select_checkers(
    ed: &mut Editor,
    arg: Prefix,
    then: impl FnOnce(&mut Editor, Vec<&'static str>) + Send + 'static,
) {
    let all: Vec<&'static str> = CHECKERS.iter().map(|c| c.0).collect();
    match arg {
        Prefix::U(1) => {
            let mut cats: Vec<String> = CHECKERS
                .iter()
                .filter_map(|c| c.2.first().map(|s| (*s).to_owned()))
                .collect();
            cats.dedup();
            let mut seen = vec![];
            cats.retain(|c| {
                if seen.contains(c) {
                    false
                } else {
                    seen.push(c.clone());
                    true
                }
            });
            super::complete(ed, "Checker category: ", cats, true, move |ed, cat| {
                then(
                    ed,
                    CHECKERS
                        .iter()
                        .filter(|c| c.2.contains(&cat.as_str()))
                        .map(|c| c.0)
                        .collect(),
                )
            });
        }
        Prefix::U(_) => {
            let names = all.iter().map(|s| (*s).to_owned()).collect();
            super::complete(ed, "Checker name: ", names, true, move |ed, n| {
                then(
                    ed,
                    CHECKERS
                        .iter()
                        .filter(|c| c.0 == n)
                        .map(|c| c.0)
                        .take(1)
                        .collect(),
                )
            });
        }
        _ => then(ed, all),
    }
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-lint" => {
            if ed.org.is_none() {
                return Some(Err("Not in an Org buffer".into()));
            }
            ed.set_msg("Org linting process starting...");
            select_checkers(ed, arg, |ed, checkers| {
                super::effect(ed, move |s| {
                    let source = s.org_current();
                    display(s, source, checkers)
                })
            });
            Ok(())
        }
        "org-lint--jump-to-source" | "org-lint--show-source" => {
            let Some(r) = current(ed) else {
                return Some(Ok(()));
            };
            let source = STATE
                .with(|st| st.borrow().as_ref().map(|s| s.source))
                .unwrap_or(0);
            if name == "org-lint--show-source" {
                let target = r.line - 1;
                super::effect(ed, move |s| {
                    let text =
                        s.org_with_buffer(source, |e| e.buf.line(target.min(e.line_count() - 1)));
                    s.ed.set_msg(format!("{}: {text}", r.line));
                });
            } else {
                super::effect(ed, move |s| {
                    s.org_show(source);
                    super::fold::widen(&mut s.ed);
                    let l = (r.line - 1).min(s.ed.line_count() - 1);
                    s.ed.set_cursor(l, 0);
                    super::fold::show_set_visibility(&mut s.ed, l, "local");
                });
            }
            Ok(())
        }
        "org-lint--hide-checker" | "org-lint--ignore-checker" => {
            let Some(r) = current(ed) else {
                return Some(Ok(()));
            };
            let text = STATE.with(|st| {
                let mut st = st.borrow_mut();
                let s = st.as_mut()?;
                s.reports.retain(|x| x.checker != r.checker);
                if name == "org-lint--ignore-checker" {
                    s.checkers.retain(|c| *c != r.checker);
                }
                Some(render(&s.reports))
            });
            if let Some(text) = text {
                set_text(ed, &text);
            }
            Ok(())
        }
        "org-lint--refresh-reports" => {
            let Some((source, checkers)) =
                STATE.with(|st| st.borrow().as_ref().map(|s| (s.source, s.checkers.clone())))
            else {
                return Some(Ok(()));
            };
            let line = ed.cur.line;
            super::effect(ed, move |s| {
                display(s, source, checkers);
                let l = line.min(s.ed.line_count() - 1);
                s.ed.set_cursor(l, 0);
            });
            Ok(())
        }
        _ => return None,
    })
}

/// Replace the report buffer's text.
fn set_text(ed: &mut Editor, text: &str) {
    let line = ed.cur.line;
    ed.readonly = false;
    let n = ed.line_count();
    let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    super::splice(ed, 0, n, &lines);
    ed.readonly = true;
    ed.set_cursor(line.min(ed.line_count() - 1), 0);
}

#[cfg(test)]
mod tests;
