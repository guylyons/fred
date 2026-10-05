//! Properties (org.el): org-entry-get/put/delete, inheritance, special
//! properties, property drawers and the property commands.
//!
//! An entry is addressed by its heading line (`None`: the file level,
//! before the first heading). Reading works on any [`Lines`].

use super::syntax::{self, Lines, Settings};
use super::{Prefix, ctx};
use crate::editor::Editor;

pub const SPECIAL: &[&str] = &[
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

pub const DEFAULT: &[&str] = &[
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

fn is_planning(t: &str) -> bool {
    ctx::is_planning(t)
}

/// The property drawer of an entry: (`:PROPERTIES:` line, `:END:` line).
pub fn drawer<L: Lines + ?Sized>(b: &L, h: Option<usize>) -> Option<(usize, usize)> {
    let n = b.n_lines();
    let start = match h {
        Some(h) => {
            let mut l = h + 1;
            if l < n && is_planning(&b.line_text(l)) {
                l += 1;
            }
            l
        }
        None => {
            let mut l = 0;
            while l < n {
                let t = b.line_text(l);
                let tt = t.trim_start();
                if tt.starts_with("# ") || tt == "#" {
                    l += 1;
                } else {
                    break;
                }
            }
            l
        }
    };
    if start >= n
        || !b
            .line_text(start)
            .trim()
            .eq_ignore_ascii_case(":PROPERTIES:")
    {
        return None;
    }
    for i in start + 1..n {
        let t = b.line_text(i);
        if syntax::level(&t).is_some() {
            return None;
        }
        if t.trim().eq_ignore_ascii_case(":END:") {
            return Some((start, i));
        }
    }
    None
}

/// `:KEY: value` on a drawer line.
pub fn parse_property(t: &str) -> Option<(String, String)> {
    let t = t.trim_start();
    let rest = t.strip_prefix(':')?;
    let colon = rest.find(':')?;
    let key = &rest[..colon];
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    let value = &rest[colon + 1..];
    if !value.is_empty() && !value.starts_with([' ', '\t']) {
        return None;
    }
    Some((key.to_owned(), value.trim().to_owned()))
}

/// org-not-nil.
fn not_nil(v: Option<String>) -> Option<String> {
    v.filter(|s| s != "nil")
}

/// org--property-local-values: (base value, PROPERTY+ values).
pub fn local_values<L: Lines + ?Sized>(
    b: &L,
    h: Option<usize>,
    prop: &str,
) -> (Option<String>, Vec<String>) {
    let Some((s, e)) = drawer(b, h) else {
        return (None, vec![]);
    };
    let mut base = None;
    let mut extra = vec![];
    let plus = format!("{prop}+");
    for i in s + 1..e {
        if let Some((k, v)) = parse_property(&b.line_text(i)) {
            if k.eq_ignore_ascii_case(prop) {
                if base.is_none() {
                    base = Some(v);
                }
            } else if k.eq_ignore_ascii_case(&plus) {
                extra.push(v);
            }
        }
    }
    (base, extra)
}

/// The separator for accumulated values (org-property-separators).
fn separator(prop: &str) -> String {
    if let Some(v) = super::sexp::option("org-property-separators")
        && let Some(list) = v.list()
    {
        for e in list {
            let sep = e.cdr();
            let Some(sep) = sep.str() else { continue };
            let keys = e.car();
            let hit = match keys {
                Some(k) if k.list().is_some() => k
                    .list()
                    .unwrap()
                    .iter()
                    .any(|x| x.str().is_some_and(|x| x.eq_ignore_ascii_case(prop))),
                Some(k) => k
                    .str()
                    .is_some_and(|re| super::re::compile(re, true).is_ok_and(|r| r.is_match(prop))),
                None => false,
            };
            if hit {
                return sep.to_owned();
            }
        }
    }
    " ".into()
}

/// org-property-inherit-p.
pub fn inherit_p(prop: &str) -> bool {
    match super::sexp::option("org-use-property-inheritance") {
        None | Some(super::sexp::Sexp::Nil) => false,
        Some(super::sexp::Sexp::T) => true,
        Some(super::sexp::Sexp::Str(re)) => {
            super::re::compile(&re, false).is_ok_and(|r| r.is_match(prop))
        }
        Some(v) => v.list().is_some_and(|l| {
            l.iter()
                .any(|x| x.str().is_some_and(|x| x.eq_ignore_ascii_case(prop)))
        }),
    }
}

/// Global and keyword values: #+PROPERTY, org-global-properties.
fn global_value(st: &Settings, prop: &str) -> Option<String> {
    st.properties
        .iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(prop))
        .map(|(_, v)| v.clone())
        .or_else(|| {
            super::sexp::option("org-global-properties").and_then(|v| {
                v.list()?
                    .iter()
                    .find(|e| {
                        e.car()
                            .and_then(|c| c.str())
                            .is_some_and(|k| k.eq_ignore_ascii_case(prop))
                    })
                    .and_then(|e| e.cdr().str().map(str::to_owned))
            })
        })
        .or_else(|| match prop.to_ascii_uppercase().as_str() {
            // org-global-properties-fixed.
            "VISIBILITY_ALL" => Some("folded children content all".into()),
            "CLOCK_MODELINE_TOTAL_ALL" => Some("current today repeat all auto".into()),
            _ => None,
        })
}

/// How `org-entry-get` treats inheritance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inherit {
    No,
    Yes,
    /// Only when org-use-property-inheritance selects the property.
    Selective,
}

/// Context for computing special properties of entries in a buffer.
pub struct Doc<'a, L: Lines + ?Sized> {
    pub b: &'a L,
    pub st: &'a Settings,
    pub file: Option<&'a std::path::Path>,
}

impl<'a, L: Lines + ?Sized> Doc<'a, L> {
    pub fn new(b: &'a L, st: &'a Settings, file: Option<&'a std::path::Path>) -> Self {
        Doc { b, st, file }
    }

    fn heading(&self, l: usize) -> Option<usize> {
        syntax::heading_at_or_before(self.b, l)
    }

    /// org-entry-get.
    pub fn get(
        &self,
        h: Option<usize>,
        prop: &str,
        inherit: Inherit,
        literal_nil: bool,
    ) -> Option<String> {
        let up = prop.to_ascii_uppercase();
        if up == "CATEGORY" || SPECIAL.contains(&up.as_str()) {
            return self.special(h, &up);
        }
        if inherit == Inherit::Yes || (inherit == Inherit::Selective && inherit_p(prop)) {
            return self.get_with_inheritance(h, prop, literal_nil);
        }
        let (base, extra) = local_values(self.b, h, prop);
        let parts: Vec<String> = base.into_iter().chain(extra).collect();
        if parts.is_empty() {
            return None;
        }
        let v = parts.join(&separator(prop));
        if literal_nil {
            Some(v)
        } else {
            not_nil(Some(v))
        }
    }

    /// org-entry-get-with-inheritance.
    pub fn get_with_inheritance(
        &self,
        h: Option<usize>,
        prop: &str,
        literal_nil: bool,
    ) -> Option<String> {
        let mut values: Vec<String> = vec![];
        let mut found = false;
        let mut cur = h;
        loop {
            let (base, extra) = local_values(self.b, cur, prop);
            match base {
                None => {
                    let mut e = extra;
                    e.extend(values);
                    values = e;
                }
                Some(v) => {
                    let mut e = vec![v];
                    e.extend(extra);
                    e.extend(values);
                    values = e;
                    found = true;
                    break;
                }
            }
            match cur {
                Some(c) => cur = syntax::parent(self.b, c).or(None),
                None => break,
            }
            if cur.is_none() {
                // The file level (org-data) after the top heading.
                let (base, extra) = local_values(self.b, None, prop);
                if let Some(v) = base {
                    let mut e = vec![v];
                    e.extend(extra);
                    e.extend(values);
                    values = e;
                    found = true;
                } else {
                    let mut e = extra;
                    e.extend(values);
                    values = e;
                }
                break;
            }
        }
        if !found && let Some(g) = global_value(self.st, prop) {
            values.insert(0, g);
        }
        if values.is_empty() {
            return None;
        }
        let v = values.join(&separator(prop));
        if literal_nil {
            Some(v)
        } else {
            not_nil(Some(v))
        }
    }

    /// Local tags of heading `h`.
    pub fn local_tags(&self, h: usize) -> Vec<String> {
        syntax::headline(&self.b.line_text(h), self.st)
            .map(|x| x.tags)
            .unwrap_or_default()
    }

    /// org-get-tags: file tags, inherited tags, local tags.
    pub fn tags(&self, h: Option<usize>, local: bool) -> Vec<String> {
        let Some(h) = h else {
            return if local {
                vec![]
            } else {
                self.st.file_tags.clone()
            };
        };
        let ltags = self.local_tags(h);
        if local || !super::tags::use_inheritance() {
            return ltags;
        }
        let mut itags: Vec<String> = vec![];
        let mut p = syntax::parent(self.b, h);
        let mut chain = vec![];
        while let Some(x) = p {
            chain.push(x);
            p = syntax::parent(self.b, x);
        }
        for x in chain.into_iter().rev() {
            itags.extend(self.local_tags(x));
        }
        let mut all: Vec<String> = self
            .st
            .file_tags
            .iter()
            .cloned()
            .chain(itags)
            .filter(|t| super::tags::inherit_p(t))
            .collect();
        all.extend(ltags);
        // delete-dups keeping the most local occurrence.
        let mut out: Vec<String> = vec![];
        for t in all.into_iter().rev() {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        out.reverse();
        out
    }

    /// org-get-category.
    pub fn category(&self, h: Option<usize>) -> String {
        self.get_with_inheritance(h, "CATEGORY", false)
            .or_else(|| self.st.category.clone())
            .or_else(|| {
                super::sexp::option("org-category").and_then(|v| v.str().map(str::to_owned))
            })
            .or_else(|| {
                self.file
                    .and_then(|f| f.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "???".into())
    }

    /// A special property (ITEM, TODO, TAGS ...).
    fn special(&self, h: Option<usize>, prop: &str) -> Option<String> {
        if prop == "CATEGORY" {
            return Some(self.category(h));
        }
        if prop == "FILE" {
            return self.file.map(|f| f.display().to_string());
        }
        let h = h?;
        let line = self.b.line_text(h);
        let hl = syntax::headline(&line, self.st)?;
        match prop {
            "ITEM" => Some(hl.title(&line).replace('\t', " ")),
            "TODO" => hl.todo.clone(),
            "PRIORITY" => Some(priority_string(hl.priority.unwrap_or(self.st.priorities.2))),
            "TAGS" => {
                let t = self.tags(Some(h), true);
                (!t.is_empty()).then(|| super::tags::make_tag_string(&t))
            }
            "ALLTAGS" => {
                let t = self.tags(Some(h), false);
                (!t.is_empty()).then(|| super::tags::make_tag_string(&t))
            }
            "CLOSED" | "DEADLINE" | "SCHEDULED" => {
                let p = h + 1;
                if p >= self.b.n_lines() {
                    return None;
                }
                let t = self.b.line_text(p);
                if !is_planning(&t) {
                    return None;
                }
                let key = format!("{prop}:");
                let i = t.find(&key)? + key.len();
                let rest = t[i..].trim_start();
                let len = super::face::timestamp_len(rest)?;
                Some(rest[..len].to_owned())
            }
            "TIMESTAMP" | "TIMESTAMP_IA" => {
                let active = prop == "TIMESTAMP";
                let end = syntax::entry_end(self.b, h);
                let mut l = h;
                while l < end {
                    let t = self.b.line_text(l);
                    if l == h + 1 && is_planning(&t) {
                        l += 1;
                        continue;
                    }
                    let mut i = 0;
                    while i < t.len() {
                        let r = &t[i..];
                        if (r.starts_with('<') && active || r.starts_with('[') && !active)
                            && let Some(len) = super::face::timestamp_len(r)
                        {
                            return Some(r[..len].to_owned());
                        }
                        i += r.chars().next().map_or(1, char::len_utf8);
                    }
                    l += 1;
                }
                None
            }
            "BLOCKED" => Some(if super::todo::blocked(self, h) {
                "t".into()
            } else {
                String::new()
            }),
            "CLOCKSUM" | "CLOCKSUM_T" => None,
            _ => None,
        }
    }

    /// org-entry-properties (all, or `which` = "special" / "standard").
    pub fn properties(&self, h: Option<usize>, which: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = vec![];
        if which != "standard" {
            for p in SPECIAL {
                if let Some(v) = self.special(h, p) {
                    out.push(((*p).into(), v));
                }
            }
        }
        if which != "special"
            && let Some((s, e)) = drawer(self.b, h)
        {
            for i in s + 1..e {
                let Some((k, v)) = parse_property(&self.b.line_text(i)) else {
                    continue;
                };
                let k = k.to_ascii_uppercase();
                let (base, extend) = match k.strip_suffix('+') {
                    Some(b) => (b.to_owned(), true),
                    None => (k.clone(), false),
                };
                if SPECIAL.contains(&base.as_str()) {
                    continue;
                }
                match out.iter_mut().find(|(n, _)| *n == base) {
                    Some((_, old)) if extend => {
                        old.push(' ');
                        old.push_str(&v);
                    }
                    Some(_) => {}
                    None => out.push((base, v)),
                }
            }
        }
        if !out.iter().any(|(k, _)| k == "CATEGORY") {
            out.push(("CATEGORY".into(), self.category(h)));
        }
        out
    }

    /// org-property-get-allowed-values.
    pub fn allowed_values(&self, h: Option<usize>, prop: &str) -> Vec<String> {
        match prop {
            "TODO" => {
                let mut v: Vec<String> =
                    self.st.todo_names().iter().map(|s| s.to_string()).collect();
                v.push(String::new());
                v
            }
            "PRIORITY" => (self.st.priorities.0..=self.st.priorities.1)
                .map(priority_string)
                .collect(),
            p if SPECIAL.contains(&p) => vec![],
            _ => {
                let Some(vals) = self.get(h, &format!("{prop}_ALL"), Inherit::Yes, false) else {
                    return vec![];
                };
                match super::sexp::read(&format!("({vals})")) {
                    Ok(v) => v
                        .list()
                        .unwrap_or(&[])
                        .iter()
                        .map(|x| match x {
                            super::sexp::Sexp::Str(s) | super::sexp::Sexp::Sym(s) => s.clone(),
                            super::sexp::Sexp::T => "t".into(),
                            other => other.to_string(),
                        })
                        .collect(),
                    Err(_) => vals.split_whitespace().map(str::to_owned).collect(),
                }
            }
        }
    }

    pub fn heading_of(&self, l: usize) -> Option<usize> {
        self.heading(l)
    }
}

/// org-priority-to-string.
pub fn priority_string(p: u32) -> String {
    if p < 65 {
        p.to_string()
    } else {
        char::from_u32(p).map_or_else(|| p.to_string(), |c| c.to_string())
    }
}

/// The entry at line `l` of the editor: its heading, if any.
pub fn entry(ed: &Editor, l: usize) -> Option<usize> {
    syntax::heading_at_or_before(&ed.buf, l)
}

/// org-entry-get on the editor at heading `h`.
pub fn get(ed: &Editor, h: Option<usize>, prop: &str, inherit: Inherit) -> Option<String> {
    let st = super::settings(ed);
    let path = ed.path.clone();
    Doc::new(&ed.buf, &st, path.as_deref()).get(h, prop, inherit, false)
}

/// org-insert-property-drawer: returns the drawer lines.
pub fn insert_drawer(ed: &mut Editor, h: Option<usize>) -> (usize, usize) {
    if let Some(d) = drawer(&ed.buf, h) {
        return d;
    }
    let at = match h {
        Some(h) => {
            let mut l = h + 1;
            if l < ed.line_count() && is_planning(&ed.buf.line(l)) {
                l += 1;
            }
            l
        }
        None => {
            let mut l = 0;
            while l < ed.line_count() && {
                let t = ed.buf.line(l);
                t.trim_start().starts_with("# ") || t.trim() == "#"
            } {
                l += 1;
            }
            l
        }
    };
    let indent = drawer_indent(h.map(|h| syntax::level(&ed.buf.line(h)).unwrap_or(0)));
    let lines = vec![format!("{indent}:PROPERTIES:"), format!("{indent}:END:")];
    if ed.line_count() == 1 && ed.buf.len_bytes() == 0 {
        super::splice(ed, 0, 1, &lines);
    } else {
        super::insert_lines(ed, at, &lines);
    }
    super::fold::region(ed, at + 1, at + 1, true, super::fold::Spec::Drawer);
    (at, at + 1)
}

/// Indentation of planning/drawer lines (org-adapt-indentation t).
fn drawer_indent(level: Option<usize>) -> String {
    let adapt = super::sexp::option("org-adapt-indentation").is_some_and(|v| v.truthy());
    match level {
        Some(l) if adapt => " ".repeat(l + 1),
        _ => String::new(),
    }
}

/// org-entry-put for regular properties (TODO/PRIORITY/SCHEDULED/DEADLINE
/// are routed to their commands).
pub fn put(ed: &mut Editor, h: Option<usize>, prop: &str, value: &str) -> Result<(), String> {
    if prop.is_empty() || prop.contains(char::is_whitespace) {
        return Err(format!("Invalid property name: \"{prop}\""));
    }
    match prop {
        "TODO" => {
            if let Some(h) = h {
                ed.set_cursor(h, 0);
            }
            return super::todo::todo_to(
                ed,
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.to_owned())
                },
            );
        }
        "PRIORITY" => {
            if let Some(h) = h {
                ed.set_cursor(h, 0);
            }
            return super::todo::set_priority(
                ed,
                if value.trim().is_empty() {
                    None
                } else {
                    syntax::priority_value(value)
                },
            );
        }
        "SCHEDULED" | "DEADLINE" => {
            if let Some(h) = h {
                ed.set_cursor(h, 0);
            }
            let cmd = if prop == "SCHEDULED" {
                "org-schedule"
            } else {
                "org-deadline"
            };
            return super::call(ed, cmd, Prefix::None);
        }
        p if SPECIAL.contains(&p) => {
            return Err(format!(
                "The {p} property cannot be set with `org-entry-put'"
            ));
        }
        _ => {}
    }
    let (s, e) = insert_drawer(ed, h);
    let indent = drawer_indent(h.map(|h| syntax::level(&ed.buf.line(h)).unwrap_or(0)));
    let new = if value.is_empty() {
        format!("{indent}:{prop}:")
    } else {
        format!("{indent}:{prop}: {value}")
    };
    let existing = (s + 1..e).find(|&i| {
        parse_property(&ed.buf.line(i)).is_some_and(|(k, _)| k.eq_ignore_ascii_case(prop))
    });
    match existing {
        Some(i) => super::set_line(ed, i, &new),
        None => super::insert_lines(ed, e, &[new]),
    }
    Ok(())
}

/// org-entry-delete: true when something was removed.
pub fn delete(ed: &mut Editor, h: Option<usize>, prop: &str) -> bool {
    let Some((s, e)) = drawer(&ed.buf, h) else {
        return false;
    };
    let plus = format!("{prop}+");
    let hits: Vec<usize> = (s + 1..e)
        .filter(|&i| {
            parse_property(&ed.buf.line(i))
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case(prop) || k.eq_ignore_ascii_case(&plus))
        })
        .collect();
    for &i in hits.iter().rev() {
        super::delete_lines(ed, i, 1);
    }
    let e = e - hits.len();
    if e == s + 1 && !hits.is_empty() {
        super::delete_lines(ed, s, 2);
    }
    !hits.is_empty()
}

/// org-buffer-property-keys.
pub fn buffer_keys(ed: &Editor, specials: bool, defaults: bool, columns: bool) -> Vec<String> {
    let mut keys: Vec<String> = vec![];
    let mut add = |k: String| {
        if !keys.iter().any(|x| x.eq_ignore_ascii_case(&k)) {
            keys.push(k);
        }
    };
    let n = ed.line_count();
    let mut l = 0;
    while l < n {
        let h = if l == 0 && !ctx::at_heading(ed, 0) {
            None
        } else {
            Some(l)
        };
        if (h.is_none() || ctx::at_heading(ed, l))
            && let Some((s, e)) = drawer(&ed.buf, h)
        {
            for i in s + 1..e {
                if let Some((k, _)) = parse_property(&ed.buf.line(i)) {
                    let k = k.strip_suffix('+').map_or(k.clone(), str::to_owned);
                    add(k);
                }
            }
        }
        l += 1;
    }
    let st = super::settings(ed);
    for (k, _) in &st.properties {
        add(k.clone());
    }
    if specials {
        SPECIAL.iter().for_each(|s| add((*s).into()));
    }
    if defaults {
        DEFAULT.iter().for_each(|s| add((*s).into()));
        add(super::options::string("org-effort-property", "Effort"));
    }
    if columns {
        let fmt = st.columns.clone().unwrap_or_else(|| {
            super::options::string(
                "org-columns-default-format",
                "%25ITEM %TODO %3PRIORITY %TAGS",
            )
        });
        for part in fmt.split_whitespace() {
            let name: String = part
                .trim_start_matches('%')
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .chars()
                .take_while(|c| !"({".contains(*c))
                .collect();
            if !name.is_empty() {
                add(name);
            }
        }
    }
    keys.sort_by_key(|k| k.to_ascii_lowercase());
    keys
}

/// org-property-values.
pub fn values(ed: &Editor, key: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in 0..ed.line_count() {
        if let Some((k, v)) = parse_property(&ed.buf.line(l))
            && k.eq_ignore_ascii_case(key)
            && !out.contains(&v)
        {
            out.push(v);
        }
    }
    out
}

/// org-set-property: prompt for the name, then the value.
fn set_property_command(ed: &mut Editor) {
    let mut keys = buffer_keys(ed, false, true, true);
    keys.retain(|k| !SPECIAL.contains(&k.as_str()));
    let last = ed.org.as_ref().and_then(|o| o.last_property.clone());
    let prompt = match &last {
        Some(l) => format!("Property [{l}]: "),
        None => "Property: ".into(),
    };
    super::complete(ed, &prompt, keys, false, move |ed, name| {
        let name = if name.trim().is_empty() {
            last.unwrap_or_default()
        } else {
            name.trim().to_owned()
        };
        if name.is_empty() || name.contains(char::is_whitespace) {
            ed.set_err(format!("Invalid property name: \"{name}\""));
            return;
        }
        read_value(ed, name);
    });
}

/// org-read-property-value, then set it.
fn read_value(ed: &mut Editor, name: String) {
    let h = entry(ed, ed.cur.line);
    let st = super::settings(ed);
    let path = ed.path.clone();
    let doc = Doc::new(&ed.buf, &st, path.as_deref());
    let mut allowed = doc.allowed_values(h, &name);
    let unrestricted = allowed.iter().any(|v| v == ":ETC");
    allowed.retain(|v| v != ":ETC");
    let current = doc.get(h, &name, Inherit::No, false);
    let candidates = if allowed.is_empty() {
        values(ed, &name)
    } else {
        allowed.clone()
    };
    let prompt = match &current {
        Some(c) => format!("{name} value [{c}]: "),
        None => format!("{name} value: "),
    };
    let require = !allowed.is_empty() && !unrestricted;
    super::complete(ed, &prompt, candidates, require, move |ed, v| {
        let v = if v.is_empty() {
            current.unwrap_or_default()
        } else {
            v
        };
        let h = entry(ed, ed.cur.line);
        if let Some(o) = &mut ed.org {
            o.last_property = Some(name.clone());
        }
        let undo = ed.cur.pos();
        ed.undo.begin(undo);
        if get(ed, h, &name, Inherit::No).as_deref() != Some(v.as_str())
            && let Err(e) = put(ed, h, &name, &v)
        {
            ed.set_err(e);
        }
        ed.undo.end(ed.cur.pos());
    });
}

/// org-property-next-allowed-value on a property line.
fn next_allowed(ed: &mut Editor, previous: bool) -> Result<(), String> {
    let l = ed.cur.line;
    if !ctx::at_property(ed, l) {
        return Err("Not at a property".into());
    }
    let (key, value) = parse_property(&ed.buf.line(l)).ok_or("Not at a property")?;
    let h = entry(ed, l);
    let st = super::settings(ed);
    let path = ed.path.clone();
    let mut allowed = Doc::new(&ed.buf, &st, path.as_deref()).allowed_values(h, &key);
    allowed.retain(|v| v != ":ETC");
    if allowed.is_empty() && ["[ ]", "[-]", "[X]"].contains(&value.as_str()) {
        allowed = vec!["[ ]".into(), "[X]".into()];
    }
    if allowed.is_empty() {
        return Err("Allowed values for this property have not been defined".into());
    }
    if previous {
        allowed.reverse();
    }
    let nval = allowed
        .iter()
        .position(|v| *v == value)
        .and_then(|i| allowed.get(i + 1))
        .unwrap_or(&allowed[0])
        .clone();
    if nval == value {
        return Err("Only one allowed value for this property".into());
    }
    let line = ed.buf.line(l);
    let indent = &line[..line.len() - line.trim_start().len()];
    super::set_line(ed, l, &format!("{indent}:{key}: {nval}"));
    let b = ed.buf.line(l).len() - ed.buf.line(l).trim_start().len();
    ed.set_cursor(l, b);
    Ok(())
}

/// org-insert-drawer.
fn insert_named_drawer(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if !arg.is_none() {
        let h = entry(ed, ed.cur.line);
        let (s, _) = insert_drawer(ed, h);
        super::fold::region(ed, s + 1, s + 1, false, super::fold::Spec::Drawer);
        super::insert_lines(ed, s + 1, &[String::new()]);
        ed.set_cursor(s + 1, 0);
        return Ok(());
    }
    let region = ed.org_region;
    super::read(ed, "Drawer: ", "", move |ed, name| {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            ed.set_err("Invalid drawer name");
            return;
        }
        ed.undo.begin(ed.cur.pos());
        match region {
            None => {
                let l = ed.cur.line;
                let at = if ed.buf.line(l).is_empty() { l } else { l + 1 };
                let lines = vec![format!(":{name}:"), String::new(), ":END:".into()];
                if ed.buf.line(l).is_empty() {
                    super::splice(ed, l, 1, &lines);
                } else {
                    super::insert_lines(ed, at, &lines);
                }
                ed.set_cursor(at + 1, 0);
            }
            Some((lo, hi)) => {
                if (lo..=hi).any(|i| ctx::at_heading(ed, i)) {
                    ed.set_err("Drawers cannot contain headlines");
                } else {
                    let mut lo = lo;
                    while lo < hi && ed.buf.line(lo).trim().is_empty() {
                        lo += 1;
                    }
                    let mut hi = hi;
                    while hi > lo && ed.buf.line(hi).trim().is_empty() {
                        hi -= 1;
                    }
                    super::insert_lines(ed, hi + 1, &[":END:".into()]);
                    super::insert_lines(ed, lo, &[format!(":{name}:")]);
                    ed.set_cursor(hi + 1, 0);
                }
            }
        }
        ed.undo.end(ed.cur.pos());
    });
    Ok(())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let l = ed.cur.line;
    Some(match name {
        "org-set-property" => {
            set_property_command(ed);
            Ok(())
        }
        "org-set-property-and-value" => {
            let last = ed.org.as_ref().and_then(|o| o.last_property_value.clone());
            let go = |ed: &mut Editor, pv: String| {
                let Some((p, v)) = pv.split_once(':') else {
                    return;
                };
                let (p, v) = (p.trim().to_owned(), v.trim().to_owned());
                if let Some(o) = &mut ed.org {
                    o.last_property_value = Some(format!("{p}: {v}"));
                }
                let h = entry(ed, ed.cur.line);
                ed.undo.begin(ed.cur.pos());
                if let Err(e) = put(ed, h, &p, &v) {
                    ed.set_err(e);
                }
                ed.undo.end(ed.cur.pos());
            };
            match (arg.is_none(), last) {
                (false, Some(pv)) => {
                    go(ed, pv);
                    Ok(())
                }
                (_, last) => {
                    super::read(
                        ed,
                        "Enter a \"[Property]: [value]\" pair: ",
                        &last.unwrap_or_default(),
                        go,
                    );
                    Ok(())
                }
            }
        }
        "org-delete-property" => {
            let h = entry(ed, l);
            let st = super::settings(ed);
            let path = ed.path.clone();
            let doc = Doc::new(&ed.buf, &st, path.as_deref());
            let mut props: Vec<String> = doc
                .properties(h, "standard")
                .into_iter()
                .map(|(k, _)| k)
                .collect();
            if doc.get(h, "CATEGORY", Inherit::No, false).is_some()
                && local_values(&ed.buf, h, "CATEGORY").0.is_none()
            {
                props.retain(|k| k != "CATEGORY");
            }
            if !props.iter().any(|_| true)
                || local_values(&ed.buf, h, &props[0]).0.is_none()
                    && props.len() == 1
                    && props[0] == "CATEGORY"
            {
                props.retain(|k| local_values(&ed.buf, h, k).0.is_some());
            }
            let del = |ed: &mut Editor, p: String| {
                let h = entry(ed, ed.cur.line);
                ed.undo.begin(ed.cur.pos());
                delete(ed, h, &p);
                ed.undo.end(ed.cur.pos());
                ed.set_msg(format!("Property \"{p}\" deleted"));
            };
            match props.len() {
                0 => {
                    ed.set_msg("No property to delete in this entry");
                    Ok(())
                }
                1 => {
                    del(ed, props.remove(0));
                    Ok(())
                }
                _ => {
                    super::complete(ed, "Property: ", props, true, del);
                    Ok(())
                }
            }
        }
        "org-delete-property-globally" => {
            let keys = buffer_keys(ed, false, false, false);
            super::complete(ed, "Globally remove property: ", keys, false, |ed, p| {
                ed.undo.begin(ed.cur.pos());
                let mut count = 0;
                let mut l = 0;
                while l < ed.line_count() {
                    let h = if ctx::at_heading(ed, l) {
                        Some(l)
                    } else if l == 0 {
                        None
                    } else {
                        l += 1;
                        continue;
                    };
                    if delete(ed, h, &p) {
                        count += 1;
                    }
                    l += 1;
                }
                ed.undo.end(ed.cur.pos());
                ed.set_msg(format!("Property \"{p}\" removed from {count} entries"));
            });
            Ok(())
        }
        "org-property-action" => {
            super::menu(
                ed,
                "Property Action:",
                vec![
                    ("s".into(), "set".into()),
                    ("d".into(), "delete".into()),
                    ("D".into(), "delete globally".into()),
                    ("c".into(), "compute".into()),
                ],
                |ed, k| {
                    let cmd = match k.as_str() {
                        "s" => "org-set-property",
                        "d" => "org-delete-property",
                        "D" => "org-delete-property-globally",
                        _ => "org-compute-property-at-point",
                    };
                    super::run(ed, cmd, Prefix::None);
                },
            );
            Ok(())
        }
        "org-property-next-allowed-value" => next_allowed(ed, false),
        "org-property-previous-allowed-value" => next_allowed(ed, true),
        "org-insert-drawer" => insert_named_drawer(ed, arg),
        "org-insert-property-drawer" => {
            let h = entry(ed, l);
            insert_drawer(ed, h);
            Ok(())
        }
        "org-set-effort" | "org-inc-effort" => {
            set_effort(ed, name == "org-inc-effort" || !arg.is_none())
        }
        _ => return None,
    })
}

/// org-set-effort.
fn set_effort(ed: &mut Editor, increment: bool) -> Result<(), String> {
    let prop = super::options::string("org-effort-property", "Effort");
    let h = entry(ed, ed.cur.line);
    let st = super::settings(ed);
    let path = ed.path.clone();
    let doc = Doc::new(&ed.buf, &st, path.as_deref());
    let mut allowed = doc.allowed_values(h, &prop);
    let unrestricted = allowed.iter().any(|v| v == ":ETC");
    allowed.retain(|v| v != ":ETC");
    let current = doc.get(h, &prop, Inherit::No, false);
    let finish = move |ed: &mut Editor, value: String| {
        let p = super::options::string("org-effort-property", "Effort");
        let h = entry(ed, ed.cur.line);
        ed.undo.begin(ed.cur.pos());
        if get(ed, h, &p, Inherit::No).as_deref() != Some(value.as_str())
            && let Err(e) = put(ed, h, &p, &value)
        {
            ed.set_err(e);
        }
        ed.undo.end(ed.cur.pos());
        ed.set_msg(format!("{p} is now {value}"));
    };
    if increment {
        if allowed.is_empty() {
            return Err("Allowed effort values are not set".into());
        }
        let next = allowed
            .iter()
            .position(|v| Some(v) == current.as_ref())
            .and_then(|i| allowed.get(i + 1))
            .ok_or_else(|| {
                format!(
                    "Unknown value {:?} among allowed values",
                    current.clone().unwrap_or_default()
                )
            })?
            .clone();
        finish(ed, next);
        return Ok(());
    }
    let require = !allowed.is_empty() && !unrestricted;
    super::complete(ed, "Effort: ", allowed, require, finish);
    Ok(())
}
