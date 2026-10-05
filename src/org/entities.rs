//! org-entities.el: the entity table, org-entities-help, and the
//! org-pretty-entities display (entities as UTF-8, sub/superscripts).
//!
//! The table is generated from the pinned upstream `org-entities` with
//! Emacs (`prin1` of each field), in upstream order with its headings.

use super::Prefix;
use crate::editor::Editor;
use std::ops::Range;

/// One built-in entity: name, LaTeX, LaTeX math mode, HTML, ASCII,
/// Latin-1 and UTF-8 replacements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Raw {
    pub name: &'static str,
    pub latex: &'static str,
    pub math: bool,
    pub html: &'static str,
    pub ascii: &'static str,
    pub latin1: &'static str,
    pub utf8: &'static str,
}

/// An org-entities element: a section heading (`"* Letters"`) or an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum E {
    Head(&'static str),
    Ent(Raw),
}

/// An entity from org-entities-user or org-entities (org-entity-get).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub name: String,
    pub latex: String,
    pub math: bool,
    pub html: String,
    pub ascii: String,
    pub latin1: String,
    pub utf8: String,
}

impl From<&Raw> for Entity {
    fn from(r: &Raw) -> Entity {
        Entity {
            name: r.name.into(),
            latex: r.latex.into(),
            math: r.math,
            html: r.html.into(),
            ascii: r.ascii.into(),
            latin1: r.latin1.into(),
            utf8: r.utf8.into(),
        }
    }
}

/// org-entities-user: `(("name" "latex" mathp "html" "ascii" "latin1" "utf8") ...)`.
pub fn user() -> Vec<Entity> {
    let Some(v) = super::sexp::option("org-entities-user") else { return vec![] };
    v.list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let f = e.list()?;
            let s = |i: usize| f.get(i).and_then(|x| x.str()).map(str::to_owned);
            Some(Entity {
                name: s(0)?,
                latex: s(1)?,
                math: f.get(2).is_some_and(super::sexp::Sexp::truthy),
                html: s(3)?,
                ascii: s(4)?,
                latin1: s(5)?,
                utf8: s(6)?,
            })
        })
        .collect()
}

/// org-entity-get: the user list first, then the built-in one.
pub fn get(name: &str) -> Option<Entity> {
    if let Some(e) = user().into_iter().find(|e| e.name == name) {
        return Some(e);
    }
    builtin(name).map(Entity::from)
}

/// A built-in entity by name.
pub fn builtin(name: &str) -> Option<&'static Raw> {
    ORG_ENTITIES.iter().find_map(|e| match e {
        E::Ent(r) if r.name == name => Some(r),
        _ => None,
    })
}

/// Every entity name (user ones first), without duplicates: the
/// candidates pcomplete offers after `\`.
pub fn names() -> Vec<String> {
    let mut out: Vec<String> = user().into_iter().map(|e| e.name).collect();
    for e in ORG_ENTITIES {
        if let E::Ent(r) = e
            && !out.iter().any(|n| n == r.name)
        {
            out.push(r.name.to_owned());
        }
    }
    out
}

/// The text of the `*Org Entity Help*` buffer (org-entities-help).
pub fn help_text() -> String {
    let mut out = String::from("Org mode entities\n=================\n\n");
    let head = "\n   Symbol   Org entity        LaTeX code             HTML code\n   -----------------------------------------------------------\n";
    let mut last_was_string = true;
    let row = |out: &mut String, utf8: &str, name: &str, latex: &str, html: &str, last: &mut bool| {
        if *last {
            out.push_str(head);
            *last = false;
        }
        out.push_str(&format!("   {} \\{} {} {}\n", pad(utf8, 8), pad(name, 16), pad(latex, 22), pad(html, 13)));
    };
    out.push_str("* User-defined additions (variable org-entities-user)\n");
    for e in user() {
        row(&mut out, &e.utf8, &e.name, &e.latex, &e.html, &mut last_was_string);
    }
    for e in ORG_ENTITIES {
        match e {
            E::Head(h) => {
                out.push_str(h);
                out.push('\n');
                last_was_string = true;
            }
            E::Ent(r) => row(&mut out, r.utf8, r.name, r.latex, r.html, &mut last_was_string),
        }
    }
    out
}

/// `%-Ns`: pad to N characters.
fn pad(s: &str, n: usize) -> String {
    let w = s.chars().count();
    if w >= n { s.to_owned() } else { format!("{s}{}", " ".repeat(n - w)) }
}

/// org-pretty-entities in this buffer: org-toggle-pretty-entities, else
/// `#+STARTUP: entitiespretty`/`entitiesplain`, else the option.
pub fn pretty(ed: &Editor) -> bool {
    if let Some(v) = ed.org.as_ref().and_then(|o| o.pretty_entities) {
        return v;
    }
    super::settings(ed).opt_bool("org-pretty-entities", false)
}

/// org-toggle-pretty-entities.
pub fn toggle_pretty(ed: &mut Editor) {
    let on = !pretty(ed);
    if let Some(o) = &mut ed.org {
        o.pretty_entities = Some(on);
    }
    ed.set_msg(if on { "Entities are now displayed as UTF8 characters" } else { "Entities are now displayed as plain text" });
}

/// org-at-comment-p for a line: `#` then a space or the end.
fn comment_line(line: &str) -> bool {
    let t = line.trim_start_matches([' ', '\t']);
    t == "#" || t.starts_with("# ") || t.starts_with("#\t")
}

/// org-fontify-entities on one line: the ranges shown as one UTF-8
/// character. `\name` (and a following `{}`) when the entity's UTF-8 form
/// is a single character; not in comments.
pub fn entity_overlays(line: &str) -> Vec<(Range<usize>, String)> {
    let re = crate::org_re!(r"\\(there4|sup[123]|frac[13][24]|[a-zA-Z]+)($|\{\}|[^\p{Alphabetic}\n])");
    let mut out = vec![];
    if comment_line(line) {
        return out;
    }
    let mut from = 0;
    while let Some(c) = re.captures_at(line, from) {
        let m0 = c.get(0).unwrap();
        let name = c.get(1).unwrap();
        let g2 = c.get(2).unwrap();
        // backward-char 1: the next search starts before the terminator.
        from = if g2.is_empty() { m0.end().max(m0.start() + 1) } else { g2.end() - g2.as_str().chars().next_back().map_or(1, char::len_utf8) };
        from = from.max(name.end());
        if let Some(e) = get(name.as_str())
            && e.utf8.chars().count() == 1
        {
            let end = if g2.as_str() == "{}" { g2.end() } else { name.end() };
            out.push((m0.start()..end, e.utf8.clone()));
            from = from.max(end);
        }
    }
    out
}

/// org-create-multibrace-regexp.
fn multibrace(left: &str, right: &str, mut n: usize) -> String {
    let nothing = format!("[^{left}{right}]*?");
    let mut re = nothing.clone();
    let mut next = format!("(?:{nothing}{left}{nothing}{right})+{nothing}");
    while n > 1 {
        n -= 1;
        re = format!("{re}|{next}");
        next = format!("(?:{nothing}{left}{next}{right})+{nothing}");
    }
    format!("{left}({re}){right}")
}

/// org-match-substring-regexp (`braces`: org-match-substring-with-braces-regexp).
pub fn substring_regexp(braces: bool) -> &'static regex::Regex {
    static ANY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static BRACES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let curly = multibrace(r"\{", r"\}", 3);
    if braces {
        BRACES.get_or_init(|| regex::Regex::new(&format!(r"(\S)([_^])({curly})")).unwrap())
    } else {
        ANY.get_or_init(|| {
            let paren = multibrace(r"\(", r"\)", 3);
            regex::Regex::new(&format!(r"(\S)([_^])((?:{curly})|(?:{paren})|(?:\*|[+-]?[\p{{Alphabetic}}\p{{Nd}}.,\\]*[\p{{Alphabetic}}\p{{Nd}}]))")).unwrap()
        })
    }
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' => 'ᵀ',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    })
}

/// org-raise-scripts on one line. A terminal cannot raise text, so a
/// script whose characters all have Unicode super/subscript forms is shown
/// in those forms with its `^`/`_` and braces hidden; other scripts keep
/// their source text. `braces`: org-use-sub-superscripts is `{}`.
/// Not in comments, links or `#+KEYWORD:` lines.
pub fn script_overlays(line: &str, braces: bool) -> Vec<(Range<usize>, String)> {
    let mut out = vec![];
    if comment_line(line) || line.trim_start().starts_with("#+") {
        return out;
    }
    let re = substring_regexp(braces);
    let links: Vec<Range<usize>> = {
        let mut v = vec![];
        let mut from = 0;
        while let Some(s) = line[from..].find("[[").map(|i| i + from) {
            let Some(e) = line[s..].find("]]").map(|i| s + i + 2) else { break };
            v.push(s..e);
            from = e;
        }
        v
    };
    let mut from = 0;
    while let Some(c) = re.captures_at(line, from) {
        let m0 = c.get(0).unwrap();
        let marker = c.get(2).unwrap();
        let mut v = c.get(3).unwrap().range();
        // Handle a_b^c: the next search starts at the marker after the script.
        from = if matches!(line.as_bytes().get(m0.end()), Some(b'_' | b'^')) { m0.end() - line[..m0.end()].chars().next_back().map_or(1, char::len_utf8) } else { m0.end() };
        from = from.max(marker.end());
        if links.iter().any(|r| r.contains(&marker.start())) {
            continue;
        }
        let start = marker.start();
        let body = &line[v.clone()];
        if body.starts_with('{') && body.ends_with('}') && body.len() >= 2 {
            v = v.start + 1..v.end - 1;
        }
        let sup = marker.as_str() == "^";
        let shown: Option<String> = line[v.clone()].chars().map(|ch| if sup { superscript(ch) } else { subscript(ch) }).collect();
        if let Some(s) = shown
            && !s.is_empty()
        {
            out.push((start..m0.end(), s));
        }
    }
    out
}

/// The pretty-entity display of a line: entities and (with
/// org-pretty-entities-include-sub-superscripts and
/// org-use-sub-superscripts) scripts, sorted, not overlapping.
pub fn overlays(ed: &Editor, line: &str) -> Vec<(Range<usize>, String)> {
    if !pretty(ed) {
        return vec![];
    }
    let st = super::settings(ed);
    let mut out = entity_overlays(line);
    let subsup = st.opt("org-use-sub-superscripts").map_or("t".to_owned(), |v| v.sym().or(v.str()).unwrap_or("t").to_owned());
    if super::options::bool("org-pretty-entities-include-sub-superscripts", true) && subsup != "nil" {
        for (r, s) in script_overlays(line, subsup == "{}") {
            if !out.iter().any(|(o, _)| o.start < r.end && r.start < o.end) {
                out.push((r, s));
            }
        }
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// The entity commands.
pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-toggle-pretty-entities" => {
            toggle_pretty(ed);
            Ok(())
        }
        "org-entities-help" => {
            let text = help_text();
            super::effect(ed, move |s| {
                s.org_temp_buffer("*Org Entity Help*", &text, true);
            });
            Ok(())
        }
        "org-entities-create-table" => {
            // An Org table of every entity at point (for Worg).
            let mut rows = vec!["|Name|LaTeX code|LaTeX|HTML code |HTML|ASCII|Latin1|UTF-8".to_owned(), "|-".to_owned()];
            for e in ORG_ENTITIES {
                if let E::Ent(r) = e {
                    let fix = |s: &str| match s {
                        "|" => "\\vert".to_owned(),
                        "=>" => "= >".to_owned(),
                        s => s.to_owned(),
                    };
                    let latex = if r.math { format!("${}$", r.latex) } else { format!("$\\mbox{{{}}}$", r.latex) };
                    rows.push(format!("|{}|={}=|{}|={}=|{}|{}|{}|{}|", r.name, r.latex, latex, r.html, r.html, fix(r.ascii), fix(r.latin1), fix(r.utf8)));
                }
            }
            let l = ed.cur.line;
            super::insert_lines(ed, l, &rows);
            ed.set_cursor(l, 0);
            let _ = super::call(ed, "org-table-align", Prefix::None);
            Ok(())
        }
        _ => return None,
    })
}

pub static ORG_ENTITIES: &[E] = &[
    E::Head("* Letters"),
    E::Head("** Latin"),
    E::Ent(Raw { name: "Agrave", latex: "\\`{A}", math: false, html: "&Agrave;", ascii: "A", latin1: "À", utf8: "À" }),
    E::Ent(Raw { name: "agrave", latex: "\\`{a}", math: false, html: "&agrave;", ascii: "a", latin1: "à", utf8: "à" }),
    E::Ent(Raw { name: "Aacute", latex: "\\'{A}", math: false, html: "&Aacute;", ascii: "A", latin1: "Á", utf8: "Á" }),
    E::Ent(Raw { name: "aacute", latex: "\\'{a}", math: false, html: "&aacute;", ascii: "a", latin1: "á", utf8: "á" }),
    E::Ent(Raw { name: "Acirc", latex: "\\^{A}", math: false, html: "&Acirc;", ascii: "A", latin1: "Â", utf8: "Â" }),
    E::Ent(Raw { name: "acirc", latex: "\\^{a}", math: false, html: "&acirc;", ascii: "a", latin1: "â", utf8: "â" }),
    E::Ent(Raw { name: "Amacr", latex: "\\={A}", math: false, html: "&Amacr;", ascii: "A", latin1: "Ã", utf8: "Ã" }),
    E::Ent(Raw { name: "amacr", latex: "\\={a}", math: false, html: "&amacr;", ascii: "a", latin1: "ã", utf8: "ã" }),
    E::Ent(Raw { name: "Atilde", latex: "\\~{A}", math: false, html: "&Atilde;", ascii: "A", latin1: "Ã", utf8: "Ã" }),
    E::Ent(Raw { name: "atilde", latex: "\\~{a}", math: false, html: "&atilde;", ascii: "a", latin1: "ã", utf8: "ã" }),
    E::Ent(Raw { name: "Auml", latex: "\\\"{A}", math: false, html: "&Auml;", ascii: "Ae", latin1: "Ä", utf8: "Ä" }),
    E::Ent(Raw { name: "auml", latex: "\\\"{a}", math: false, html: "&auml;", ascii: "ae", latin1: "ä", utf8: "ä" }),
    E::Ent(Raw { name: "Aring", latex: "\\AA{}", math: false, html: "&Aring;", ascii: "A", latin1: "Å", utf8: "Å" }),
    E::Ent(Raw { name: "AA", latex: "\\AA{}", math: false, html: "&Aring;", ascii: "A", latin1: "Å", utf8: "Å" }),
    E::Ent(Raw { name: "aring", latex: "\\aa{}", math: false, html: "&aring;", ascii: "a", latin1: "å", utf8: "å" }),
    E::Ent(Raw { name: "AElig", latex: "\\AE{}", math: false, html: "&AElig;", ascii: "AE", latin1: "Æ", utf8: "Æ" }),
    E::Ent(Raw { name: "aelig", latex: "\\ae{}", math: false, html: "&aelig;", ascii: "ae", latin1: "æ", utf8: "æ" }),
    E::Ent(Raw { name: "Ccedil", latex: "\\c{C}", math: false, html: "&Ccedil;", ascii: "C", latin1: "Ç", utf8: "Ç" }),
    E::Ent(Raw { name: "ccedil", latex: "\\c{c}", math: false, html: "&ccedil;", ascii: "c", latin1: "ç", utf8: "ç" }),
    E::Ent(Raw { name: "Egrave", latex: "\\`{E}", math: false, html: "&Egrave;", ascii: "E", latin1: "È", utf8: "È" }),
    E::Ent(Raw { name: "egrave", latex: "\\`{e}", math: false, html: "&egrave;", ascii: "e", latin1: "è", utf8: "è" }),
    E::Ent(Raw { name: "Eacute", latex: "\\'{E}", math: false, html: "&Eacute;", ascii: "E", latin1: "É", utf8: "É" }),
    E::Ent(Raw { name: "eacute", latex: "\\'{e}", math: false, html: "&eacute;", ascii: "e", latin1: "é", utf8: "é" }),
    E::Ent(Raw { name: "Ecirc", latex: "\\^{E}", math: false, html: "&Ecirc;", ascii: "E", latin1: "Ê", utf8: "Ê" }),
    E::Ent(Raw { name: "ecirc", latex: "\\^{e}", math: false, html: "&ecirc;", ascii: "e", latin1: "ê", utf8: "ê" }),
    E::Ent(Raw { name: "Euml", latex: "\\\"{E}", math: false, html: "&Euml;", ascii: "E", latin1: "Ë", utf8: "Ë" }),
    E::Ent(Raw { name: "euml", latex: "\\\"{e}", math: false, html: "&euml;", ascii: "e", latin1: "ë", utf8: "ë" }),
    E::Ent(Raw { name: "Igrave", latex: "\\`{I}", math: false, html: "&Igrave;", ascii: "I", latin1: "Ì", utf8: "Ì" }),
    E::Ent(Raw { name: "igrave", latex: "\\`{i}", math: false, html: "&igrave;", ascii: "i", latin1: "ì", utf8: "ì" }),
    E::Ent(Raw { name: "Iacute", latex: "\\'{I}", math: false, html: "&Iacute;", ascii: "I", latin1: "Í", utf8: "Í" }),
    E::Ent(Raw { name: "iacute", latex: "\\'{i}", math: false, html: "&iacute;", ascii: "i", latin1: "í", utf8: "í" }),
    E::Ent(Raw { name: "Idot", latex: "\\.{I}", math: false, html: "&idot;", ascii: "I", latin1: "İ", utf8: "İ" }),
    E::Ent(Raw { name: "inodot", latex: "\\i", math: false, html: "&inodot;", ascii: "i", latin1: "ı", utf8: "ı" }),
    E::Ent(Raw { name: "Icirc", latex: "\\^{I}", math: false, html: "&Icirc;", ascii: "I", latin1: "Î", utf8: "Î" }),
    E::Ent(Raw { name: "icirc", latex: "\\^{i}", math: false, html: "&icirc;", ascii: "i", latin1: "î", utf8: "î" }),
    E::Ent(Raw { name: "Iuml", latex: "\\\"{I}", math: false, html: "&Iuml;", ascii: "I", latin1: "Ï", utf8: "Ï" }),
    E::Ent(Raw { name: "iuml", latex: "\\\"{i}", math: false, html: "&iuml;", ascii: "i", latin1: "ï", utf8: "ï" }),
    E::Ent(Raw { name: "Ntilde", latex: "\\~{N}", math: false, html: "&Ntilde;", ascii: "N", latin1: "Ñ", utf8: "Ñ" }),
    E::Ent(Raw { name: "ntilde", latex: "\\~{n}", math: false, html: "&ntilde;", ascii: "n", latin1: "ñ", utf8: "ñ" }),
    E::Ent(Raw { name: "Ograve", latex: "\\`{O}", math: false, html: "&Ograve;", ascii: "O", latin1: "Ò", utf8: "Ò" }),
    E::Ent(Raw { name: "ograve", latex: "\\`{o}", math: false, html: "&ograve;", ascii: "o", latin1: "ò", utf8: "ò" }),
    E::Ent(Raw { name: "Oacute", latex: "\\'{O}", math: false, html: "&Oacute;", ascii: "O", latin1: "Ó", utf8: "Ó" }),
    E::Ent(Raw { name: "oacute", latex: "\\'{o}", math: false, html: "&oacute;", ascii: "o", latin1: "ó", utf8: "ó" }),
    E::Ent(Raw { name: "Ocirc", latex: "\\^{O}", math: false, html: "&Ocirc;", ascii: "O", latin1: "Ô", utf8: "Ô" }),
    E::Ent(Raw { name: "ocirc", latex: "\\^{o}", math: false, html: "&ocirc;", ascii: "o", latin1: "ô", utf8: "ô" }),
    E::Ent(Raw { name: "Otilde", latex: "\\~{O}", math: false, html: "&Otilde;", ascii: "O", latin1: "Õ", utf8: "Õ" }),
    E::Ent(Raw { name: "otilde", latex: "\\~{o}", math: false, html: "&otilde;", ascii: "o", latin1: "õ", utf8: "õ" }),
    E::Ent(Raw { name: "Ouml", latex: "\\\"{O}", math: false, html: "&Ouml;", ascii: "Oe", latin1: "Ö", utf8: "Ö" }),
    E::Ent(Raw { name: "ouml", latex: "\\\"{o}", math: false, html: "&ouml;", ascii: "oe", latin1: "ö", utf8: "ö" }),
    E::Ent(Raw { name: "Oslash", latex: "\\O", math: false, html: "&Oslash;", ascii: "O", latin1: "Ø", utf8: "Ø" }),
    E::Ent(Raw { name: "oslash", latex: "\\o{}", math: false, html: "&oslash;", ascii: "o", latin1: "ø", utf8: "ø" }),
    E::Ent(Raw { name: "OElig", latex: "\\OE{}", math: false, html: "&OElig;", ascii: "OE", latin1: "OE", utf8: "Œ" }),
    E::Ent(Raw { name: "oelig", latex: "\\oe{}", math: false, html: "&oelig;", ascii: "oe", latin1: "oe", utf8: "œ" }),
    E::Ent(Raw { name: "Scaron", latex: "\\v{S}", math: false, html: "&Scaron;", ascii: "S", latin1: "S", utf8: "Š" }),
    E::Ent(Raw { name: "scaron", latex: "\\v{s}", math: false, html: "&scaron;", ascii: "s", latin1: "s", utf8: "š" }),
    E::Ent(Raw { name: "szlig", latex: "\\ss{}", math: false, html: "&szlig;", ascii: "ss", latin1: "ß", utf8: "ß" }),
    E::Ent(Raw { name: "Ugrave", latex: "\\`{U}", math: false, html: "&Ugrave;", ascii: "U", latin1: "Ù", utf8: "Ù" }),
    E::Ent(Raw { name: "ugrave", latex: "\\`{u}", math: false, html: "&ugrave;", ascii: "u", latin1: "ù", utf8: "ù" }),
    E::Ent(Raw { name: "Uacute", latex: "\\'{U}", math: false, html: "&Uacute;", ascii: "U", latin1: "Ú", utf8: "Ú" }),
    E::Ent(Raw { name: "uacute", latex: "\\'{u}", math: false, html: "&uacute;", ascii: "u", latin1: "ú", utf8: "ú" }),
    E::Ent(Raw { name: "Ucirc", latex: "\\^{U}", math: false, html: "&Ucirc;", ascii: "U", latin1: "Û", utf8: "Û" }),
    E::Ent(Raw { name: "ucirc", latex: "\\^{u}", math: false, html: "&ucirc;", ascii: "u", latin1: "û", utf8: "û" }),
    E::Ent(Raw { name: "Uuml", latex: "\\\"{U}", math: false, html: "&Uuml;", ascii: "Ue", latin1: "Ü", utf8: "Ü" }),
    E::Ent(Raw { name: "uuml", latex: "\\\"{u}", math: false, html: "&uuml;", ascii: "ue", latin1: "ü", utf8: "ü" }),
    E::Ent(Raw { name: "Yacute", latex: "\\'{Y}", math: false, html: "&Yacute;", ascii: "Y", latin1: "Ý", utf8: "Ý" }),
    E::Ent(Raw { name: "yacute", latex: "\\'{y}", math: false, html: "&yacute;", ascii: "y", latin1: "ý", utf8: "ý" }),
    E::Ent(Raw { name: "Yuml", latex: "\\\"{Y}", math: false, html: "&Yuml;", ascii: "Y", latin1: "Y", utf8: "Ÿ" }),
    E::Ent(Raw { name: "yuml", latex: "\\\"{y}", math: false, html: "&yuml;", ascii: "y", latin1: "ÿ", utf8: "ÿ" }),
    E::Head("** Latin (special face)"),
    E::Ent(Raw { name: "fnof", latex: "\\textit{f}", math: false, html: "&fnof;", ascii: "f", latin1: "f", utf8: "ƒ" }),
    E::Ent(Raw { name: "real", latex: "\\Re", math: true, html: "&real;", ascii: "R", latin1: "R", utf8: "ℜ" }),
    E::Ent(Raw { name: "image", latex: "\\Im", math: true, html: "&image;", ascii: "I", latin1: "I", utf8: "ℑ" }),
    E::Ent(Raw { name: "weierp", latex: "\\wp", math: true, html: "&weierp;", ascii: "P", latin1: "P", utf8: "℘" }),
    E::Ent(Raw { name: "ell", latex: "\\ell", math: true, html: "&ell;", ascii: "ell", latin1: "ell", utf8: "ℓ" }),
    E::Ent(Raw { name: "imath", latex: "\\imath", math: true, html: "&imath;", ascii: "[dotless i]", latin1: "dotless i", utf8: "ı" }),
    E::Ent(Raw { name: "jmath", latex: "\\jmath", math: true, html: "&jmath;", ascii: "[dotless j]", latin1: "dotless j", utf8: "ȷ" }),
    E::Head("** Greek"),
    E::Ent(Raw { name: "Alpha", latex: "A", math: false, html: "&Alpha;", ascii: "Alpha", latin1: "Alpha", utf8: "Α" }),
    E::Ent(Raw { name: "alpha", latex: "\\alpha", math: true, html: "&alpha;", ascii: "alpha", latin1: "alpha", utf8: "α" }),
    E::Ent(Raw { name: "Beta", latex: "B", math: false, html: "&Beta;", ascii: "Beta", latin1: "Beta", utf8: "Β" }),
    E::Ent(Raw { name: "beta", latex: "\\beta", math: true, html: "&beta;", ascii: "beta", latin1: "beta", utf8: "β" }),
    E::Ent(Raw { name: "Gamma", latex: "\\Gamma", math: true, html: "&Gamma;", ascii: "Gamma", latin1: "Gamma", utf8: "Γ" }),
    E::Ent(Raw { name: "gamma", latex: "\\gamma", math: true, html: "&gamma;", ascii: "gamma", latin1: "gamma", utf8: "γ" }),
    E::Ent(Raw { name: "Delta", latex: "\\Delta", math: true, html: "&Delta;", ascii: "Delta", latin1: "Delta", utf8: "Δ" }),
    E::Ent(Raw { name: "delta", latex: "\\delta", math: true, html: "&delta;", ascii: "delta", latin1: "delta", utf8: "δ" }),
    E::Ent(Raw { name: "Epsilon", latex: "E", math: false, html: "&Epsilon;", ascii: "Epsilon", latin1: "Epsilon", utf8: "Ε" }),
    E::Ent(Raw { name: "epsilon", latex: "\\epsilon", math: true, html: "&epsilon;", ascii: "epsilon", latin1: "epsilon", utf8: "ε" }),
    E::Ent(Raw { name: "varepsilon", latex: "\\varepsilon", math: true, html: "&epsilon;", ascii: "varepsilon", latin1: "varepsilon", utf8: "ε" }),
    E::Ent(Raw { name: "Zeta", latex: "Z", math: false, html: "&Zeta;", ascii: "Zeta", latin1: "Zeta", utf8: "Ζ" }),
    E::Ent(Raw { name: "zeta", latex: "\\zeta", math: true, html: "&zeta;", ascii: "zeta", latin1: "zeta", utf8: "ζ" }),
    E::Ent(Raw { name: "Eta", latex: "H", math: false, html: "&Eta;", ascii: "Eta", latin1: "Eta", utf8: "Η" }),
    E::Ent(Raw { name: "eta", latex: "\\eta", math: true, html: "&eta;", ascii: "eta", latin1: "eta", utf8: "η" }),
    E::Ent(Raw { name: "Theta", latex: "\\Theta", math: true, html: "&Theta;", ascii: "Theta", latin1: "Theta", utf8: "Θ" }),
    E::Ent(Raw { name: "theta", latex: "\\theta", math: true, html: "&theta;", ascii: "theta", latin1: "theta", utf8: "θ" }),
    E::Ent(Raw { name: "thetasym", latex: "\\vartheta", math: true, html: "&thetasym;", ascii: "theta", latin1: "theta", utf8: "ϑ" }),
    E::Ent(Raw { name: "vartheta", latex: "\\vartheta", math: true, html: "&thetasym;", ascii: "theta", latin1: "theta", utf8: "ϑ" }),
    E::Ent(Raw { name: "Iota", latex: "I", math: false, html: "&Iota;", ascii: "Iota", latin1: "Iota", utf8: "Ι" }),
    E::Ent(Raw { name: "iota", latex: "\\iota", math: true, html: "&iota;", ascii: "iota", latin1: "iota", utf8: "ι" }),
    E::Ent(Raw { name: "Kappa", latex: "K", math: false, html: "&Kappa;", ascii: "Kappa", latin1: "Kappa", utf8: "Κ" }),
    E::Ent(Raw { name: "kappa", latex: "\\kappa", math: true, html: "&kappa;", ascii: "kappa", latin1: "kappa", utf8: "κ" }),
    E::Ent(Raw { name: "Lambda", latex: "\\Lambda", math: true, html: "&Lambda;", ascii: "Lambda", latin1: "Lambda", utf8: "Λ" }),
    E::Ent(Raw { name: "lambda", latex: "\\lambda", math: true, html: "&lambda;", ascii: "lambda", latin1: "lambda", utf8: "λ" }),
    E::Ent(Raw { name: "Mu", latex: "M", math: false, html: "&Mu;", ascii: "Mu", latin1: "Mu", utf8: "Μ" }),
    E::Ent(Raw { name: "mu", latex: "\\mu", math: true, html: "&mu;", ascii: "mu", latin1: "mu", utf8: "μ" }),
    E::Ent(Raw { name: "nu", latex: "\\nu", math: true, html: "&nu;", ascii: "nu", latin1: "nu", utf8: "ν" }),
    E::Ent(Raw { name: "Nu", latex: "N", math: false, html: "&Nu;", ascii: "Nu", latin1: "Nu", utf8: "Ν" }),
    E::Ent(Raw { name: "Xi", latex: "\\Xi", math: true, html: "&Xi;", ascii: "Xi", latin1: "Xi", utf8: "Ξ" }),
    E::Ent(Raw { name: "xi", latex: "\\xi", math: true, html: "&xi;", ascii: "xi", latin1: "xi", utf8: "ξ" }),
    E::Ent(Raw { name: "Omicron", latex: "O", math: false, html: "&Omicron;", ascii: "Omicron", latin1: "Omicron", utf8: "Ο" }),
    E::Ent(Raw { name: "omicron", latex: "\\textit{o}", math: false, html: "&omicron;", ascii: "omicron", latin1: "omicron", utf8: "ο" }),
    E::Ent(Raw { name: "Pi", latex: "\\Pi", math: true, html: "&Pi;", ascii: "Pi", latin1: "Pi", utf8: "Π" }),
    E::Ent(Raw { name: "pi", latex: "\\pi", math: true, html: "&pi;", ascii: "pi", latin1: "pi", utf8: "π" }),
    E::Ent(Raw { name: "Rho", latex: "P", math: false, html: "&Rho;", ascii: "Rho", latin1: "Rho", utf8: "Ρ" }),
    E::Ent(Raw { name: "rho", latex: "\\rho", math: true, html: "&rho;", ascii: "rho", latin1: "rho", utf8: "ρ" }),
    E::Ent(Raw { name: "Sigma", latex: "\\Sigma", math: true, html: "&Sigma;", ascii: "Sigma", latin1: "Sigma", utf8: "Σ" }),
    E::Ent(Raw { name: "sigma", latex: "\\sigma", math: true, html: "&sigma;", ascii: "sigma", latin1: "sigma", utf8: "σ" }),
    E::Ent(Raw { name: "sigmaf", latex: "\\varsigma", math: true, html: "&sigmaf;", ascii: "sigmaf", latin1: "sigmaf", utf8: "ς" }),
    E::Ent(Raw { name: "varsigma", latex: "\\varsigma", math: true, html: "&sigmaf;", ascii: "varsigma", latin1: "varsigma", utf8: "ς" }),
    E::Ent(Raw { name: "Tau", latex: "T", math: false, html: "&Tau;", ascii: "Tau", latin1: "Tau", utf8: "Τ" }),
    E::Ent(Raw { name: "Upsilon", latex: "\\Upsilon", math: true, html: "&Upsilon;", ascii: "Upsilon", latin1: "Upsilon", utf8: "Υ" }),
    E::Ent(Raw { name: "upsih", latex: "\\Upsilon", math: true, html: "&upsih;", ascii: "upsilon", latin1: "upsilon", utf8: "ϒ" }),
    E::Ent(Raw { name: "upsilon", latex: "\\upsilon", math: true, html: "&upsilon;", ascii: "upsilon", latin1: "upsilon", utf8: "υ" }),
    E::Ent(Raw { name: "Phi", latex: "\\Phi", math: true, html: "&Phi;", ascii: "Phi", latin1: "Phi", utf8: "Φ" }),
    E::Ent(Raw { name: "phi", latex: "\\phi", math: true, html: "&phi;", ascii: "phi", latin1: "phi", utf8: "ɸ" }),
    E::Ent(Raw { name: "varphi", latex: "\\varphi", math: true, html: "&varphi;", ascii: "varphi", latin1: "varphi", utf8: "φ" }),
    E::Ent(Raw { name: "Chi", latex: "X", math: false, html: "&Chi;", ascii: "Chi", latin1: "Chi", utf8: "Χ" }),
    E::Ent(Raw { name: "chi", latex: "\\chi", math: true, html: "&chi;", ascii: "chi", latin1: "chi", utf8: "χ" }),
    E::Ent(Raw { name: "acutex", latex: "\\acute x", math: true, html: "&acute;x", ascii: "'x", latin1: "'x", utf8: "𝑥́" }),
    E::Ent(Raw { name: "Psi", latex: "\\Psi", math: true, html: "&Psi;", ascii: "Psi", latin1: "Psi", utf8: "Ψ" }),
    E::Ent(Raw { name: "psi", latex: "\\psi", math: true, html: "&psi;", ascii: "psi", latin1: "psi", utf8: "ψ" }),
    E::Ent(Raw { name: "tau", latex: "\\tau", math: true, html: "&tau;", ascii: "tau", latin1: "tau", utf8: "τ" }),
    E::Ent(Raw { name: "Omega", latex: "\\Omega", math: true, html: "&Omega;", ascii: "Omega", latin1: "Omega", utf8: "Ω" }),
    E::Ent(Raw { name: "omega", latex: "\\omega", math: true, html: "&omega;", ascii: "omega", latin1: "omega", utf8: "ω" }),
    E::Ent(Raw { name: "piv", latex: "\\varpi", math: true, html: "&piv;", ascii: "omega-pi", latin1: "omega-pi", utf8: "ϖ" }),
    E::Ent(Raw { name: "varpi", latex: "\\varpi", math: true, html: "&piv;", ascii: "omega-pi", latin1: "omega-pi", utf8: "ϖ" }),
    E::Ent(Raw { name: "partial", latex: "\\partial", math: true, html: "&part;", ascii: "[partial differential]", latin1: "[partial differential]", utf8: "∂" }),
    E::Head("** Hebrew"),
    E::Ent(Raw { name: "alefsym", latex: "\\aleph", math: true, html: "&alefsym;", ascii: "aleph", latin1: "aleph", utf8: "ℵ" }),
    E::Ent(Raw { name: "aleph", latex: "\\aleph", math: true, html: "&aleph;", ascii: "aleph", latin1: "aleph", utf8: "ℵ" }),
    E::Ent(Raw { name: "gimel", latex: "\\gimel", math: true, html: "&gimel;", ascii: "gimel", latin1: "gimel", utf8: "ℷ" }),
    E::Ent(Raw { name: "beth", latex: "\\beth", math: true, html: "&beth;", ascii: "beth", latin1: "beth", utf8: "ב" }),
    E::Ent(Raw { name: "dalet", latex: "\\daleth", math: true, html: "&daleth;", ascii: "dalet", latin1: "dalet", utf8: "ד" }),
    E::Head("** Icelandic"),
    E::Ent(Raw { name: "ETH", latex: "\\DH{}", math: false, html: "&ETH;", ascii: "D", latin1: "Ð", utf8: "Ð" }),
    E::Ent(Raw { name: "eth", latex: "\\dh{}", math: false, html: "&eth;", ascii: "dh", latin1: "ð", utf8: "ð" }),
    E::Ent(Raw { name: "THORN", latex: "\\TH{}", math: false, html: "&THORN;", ascii: "TH", latin1: "Þ", utf8: "Þ" }),
    E::Ent(Raw { name: "thorn", latex: "\\th{}", math: false, html: "&thorn;", ascii: "th", latin1: "þ", utf8: "þ" }),
    E::Head("* Punctuation"),
    E::Head("** Dots and Marks"),
    E::Ent(Raw { name: "dots", latex: "\\dots{}", math: false, html: "&hellip;", ascii: "...", latin1: "...", utf8: "…" }),
    E::Ent(Raw { name: "cdots", latex: "\\cdots{}", math: true, html: "&ctdot;", ascii: "...", latin1: "...", utf8: "⋯" }),
    E::Ent(Raw { name: "hellip", latex: "\\dots{}", math: false, html: "&hellip;", ascii: "...", latin1: "...", utf8: "…" }),
    E::Ent(Raw { name: "middot", latex: "\\textperiodcentered{}", math: false, html: "&middot;", ascii: ".", latin1: "·", utf8: "·" }),
    E::Ent(Raw { name: "iexcl", latex: "!`", math: false, html: "&iexcl;", ascii: "!", latin1: "¡", utf8: "¡" }),
    E::Ent(Raw { name: "iquest", latex: "?`", math: false, html: "&iquest;", ascii: "?", latin1: "¿", utf8: "¿" }),
    E::Head("** Dash-like"),
    E::Ent(Raw { name: "shy", latex: "\\-", math: false, html: "&shy;", ascii: "", latin1: "", utf8: "" }),
    E::Ent(Raw { name: "ndash", latex: "--", math: false, html: "&ndash;", ascii: "-", latin1: "-", utf8: "–" }),
    E::Ent(Raw { name: "mdash", latex: "---", math: false, html: "&mdash;", ascii: "--", latin1: "--", utf8: "—" }),
    E::Head("** Quotations"),
    E::Ent(Raw { name: "quot", latex: "\\textquotedbl{}", math: false, html: "&quot;", ascii: "\"", latin1: "\"", utf8: "\"" }),
    E::Ent(Raw { name: "acute", latex: "\\textasciiacute{}", math: false, html: "&acute;", ascii: "'", latin1: "´", utf8: "´" }),
    E::Ent(Raw { name: "ldquo", latex: "\\textquotedblleft{}", math: false, html: "&ldquo;", ascii: "\"", latin1: "\"", utf8: "“" }),
    E::Ent(Raw { name: "rdquo", latex: "\\textquotedblright{}", math: false, html: "&rdquo;", ascii: "\"", latin1: "\"", utf8: "”" }),
    E::Ent(Raw { name: "bdquo", latex: "\\quotedblbase{}", math: false, html: "&bdquo;", ascii: "\"", latin1: "\"", utf8: "„" }),
    E::Ent(Raw { name: "lsquo", latex: "\\textquoteleft{}", math: false, html: "&lsquo;", ascii: "`", latin1: "`", utf8: "‘" }),
    E::Ent(Raw { name: "rsquo", latex: "\\textquoteright{}", math: false, html: "&rsquo;", ascii: "'", latin1: "'", utf8: "’" }),
    E::Ent(Raw { name: "sbquo", latex: "\\quotesinglbase{}", math: false, html: "&sbquo;", ascii: ",", latin1: ",", utf8: "‚" }),
    E::Ent(Raw { name: "laquo", latex: "\\guillemotleft{}", math: false, html: "&laquo;", ascii: "<<", latin1: "«", utf8: "«" }),
    E::Ent(Raw { name: "raquo", latex: "\\guillemotright{}", math: false, html: "&raquo;", ascii: ">>", latin1: "»", utf8: "»" }),
    E::Ent(Raw { name: "lsaquo", latex: "\\guilsinglleft{}", math: false, html: "&lsaquo;", ascii: "<", latin1: "<", utf8: "‹" }),
    E::Ent(Raw { name: "rsaquo", latex: "\\guilsinglright{}", math: false, html: "&rsaquo;", ascii: ">", latin1: ">", utf8: "›" }),
    E::Head("* Other"),
    E::Head("** Misc. (often used)"),
    E::Ent(Raw { name: "circ", latex: "\\^{}", math: false, html: "&circ;", ascii: "^", latin1: "^", utf8: "∘" }),
    E::Ent(Raw { name: "vert", latex: "\\vert{}", math: true, html: "&vert;", ascii: "|", latin1: "|", utf8: "|" }),
    E::Ent(Raw { name: "vbar", latex: "|", math: false, html: "|", ascii: "|", latin1: "|", utf8: "|" }),
    E::Ent(Raw { name: "brvbar", latex: "\\textbrokenbar{}", math: false, html: "&brvbar;", ascii: "|", latin1: "¦", utf8: "¦" }),
    E::Ent(Raw { name: "S", latex: "\\S", math: false, html: "&sect;", ascii: "section", latin1: "§", utf8: "§" }),
    E::Ent(Raw { name: "sect", latex: "\\S", math: false, html: "&sect;", ascii: "section", latin1: "§", utf8: "§" }),
    E::Ent(Raw { name: "P", latex: "\\P{}", math: false, html: "&para;", ascii: "paragraph", latin1: "¶", utf8: "¶" }),
    E::Ent(Raw { name: "para", latex: "\\P{}", math: false, html: "&para;", ascii: "paragraph", latin1: "¶", utf8: "¶" }),
    E::Ent(Raw { name: "amp", latex: "\\&", math: false, html: "&amp;", ascii: "&", latin1: "&", utf8: "&" }),
    E::Ent(Raw { name: "lt", latex: "\\textless{}", math: false, html: "&lt;", ascii: "<", latin1: "<", utf8: "<" }),
    E::Ent(Raw { name: "gt", latex: "\\textgreater{}", math: false, html: "&gt;", ascii: ">", latin1: ">", utf8: ">" }),
    E::Ent(Raw { name: "tilde", latex: "\\textasciitilde{}", math: false, html: "~", ascii: "~", latin1: "~", utf8: "~" }),
    E::Ent(Raw { name: "slash", latex: "/", math: false, html: "/", ascii: "/", latin1: "/", utf8: "/" }),
    E::Ent(Raw { name: "plus", latex: "+", math: false, html: "+", ascii: "+", latin1: "+", utf8: "+" }),
    E::Ent(Raw { name: "under", latex: "\\_", math: false, html: "_", ascii: "_", latin1: "_", utf8: "_" }),
    E::Ent(Raw { name: "equal", latex: "=", math: false, html: "=", ascii: "=", latin1: "=", utf8: "=" }),
    E::Ent(Raw { name: "asciicirc", latex: "\\textasciicircum{}", math: false, html: "^", ascii: "^", latin1: "^", utf8: "^" }),
    E::Ent(Raw { name: "dagger", latex: "\\textdagger{}", math: false, html: "&dagger;", ascii: "[dagger]", latin1: "[dagger]", utf8: "†" }),
    E::Ent(Raw { name: "dag", latex: "\\dag{}", math: false, html: "&dagger;", ascii: "[dagger]", latin1: "[dagger]", utf8: "†" }),
    E::Ent(Raw { name: "Dagger", latex: "\\textdaggerdbl{}", math: false, html: "&Dagger;", ascii: "[doubledagger]", latin1: "[doubledagger]", utf8: "‡" }),
    E::Ent(Raw { name: "ddag", latex: "\\ddag{}", math: false, html: "&Dagger;", ascii: "[doubledagger]", latin1: "[doubledagger]", utf8: "‡" }),
    E::Head("** Whitespace"),
    E::Ent(Raw { name: "nbsp", latex: "~", math: false, html: "&nbsp;", ascii: " ", latin1: " ", utf8: " " }),
    E::Ent(Raw { name: "ensp", latex: "\\hspace*{.5em}", math: false, html: "&ensp;", ascii: " ", latin1: " ", utf8: " " }),
    E::Ent(Raw { name: "emsp", latex: "\\hspace*{1em}", math: false, html: "&emsp;", ascii: " ", latin1: " ", utf8: " " }),
    E::Ent(Raw { name: "thinsp", latex: "\\hspace*{.2em}", math: false, html: "&thinsp;", ascii: " ", latin1: " ", utf8: " " }),
    E::Head("** Currency"),
    E::Ent(Raw { name: "curren", latex: "\\textcurrency{}", math: false, html: "&curren;", ascii: "curr.", latin1: "¤", utf8: "¤" }),
    E::Ent(Raw { name: "cent", latex: "\\textcent{}", math: false, html: "&cent;", ascii: "cent", latin1: "¢", utf8: "¢" }),
    E::Ent(Raw { name: "pound", latex: "\\pounds{}", math: false, html: "&pound;", ascii: "pound", latin1: "£", utf8: "£" }),
    E::Ent(Raw { name: "yen", latex: "\\textyen{}", math: false, html: "&yen;", ascii: "yen", latin1: "¥", utf8: "¥" }),
    E::Ent(Raw { name: "euro", latex: "\\texteuro{}", math: false, html: "&euro;", ascii: "EUR", latin1: "EUR", utf8: "€" }),
    E::Ent(Raw { name: "EUR", latex: "\\texteuro{}", math: false, html: "&euro;", ascii: "EUR", latin1: "EUR", utf8: "€" }),
    E::Ent(Raw { name: "dollar", latex: "\\$", math: false, html: "$", ascii: "$", latin1: "$", utf8: "$" }),
    E::Ent(Raw { name: "USD", latex: "\\$", math: false, html: "$", ascii: "$", latin1: "$", utf8: "$" }),
    E::Head("** Property Marks"),
    E::Ent(Raw { name: "copy", latex: "\\textcopyright{}", math: false, html: "&copy;", ascii: "(c)", latin1: "©", utf8: "©" }),
    E::Ent(Raw { name: "reg", latex: "\\textregistered{}", math: false, html: "&reg;", ascii: "(r)", latin1: "®", utf8: "®" }),
    E::Ent(Raw { name: "trade", latex: "\\texttrademark{}", math: false, html: "&trade;", ascii: "TM", latin1: "TM", utf8: "™" }),
    E::Head("** Science et al."),
    E::Ent(Raw { name: "minus", latex: "-", math: true, html: "&minus;", ascii: "-", latin1: "-", utf8: "−" }),
    E::Ent(Raw { name: "pm", latex: "\\textpm{}", math: false, html: "&plusmn;", ascii: "+-", latin1: "±", utf8: "±" }),
    E::Ent(Raw { name: "plusmn", latex: "\\textpm{}", math: false, html: "&plusmn;", ascii: "+-", latin1: "±", utf8: "±" }),
    E::Ent(Raw { name: "times", latex: "\\texttimes{}", math: false, html: "&times;", ascii: "*", latin1: "×", utf8: "×" }),
    E::Ent(Raw { name: "frasl", latex: "/", math: false, html: "&frasl;", ascii: "/", latin1: "/", utf8: "⁄" }),
    E::Ent(Raw { name: "colon", latex: "\\colon", math: true, html: ":", ascii: ":", latin1: ":", utf8: ":" }),
    E::Ent(Raw { name: "div", latex: "\\textdiv{}", math: false, html: "&divide;", ascii: "/", latin1: "÷", utf8: "÷" }),
    E::Ent(Raw { name: "frac12", latex: "\\textonehalf{}", math: false, html: "&frac12;", ascii: "1/2", latin1: "½", utf8: "½" }),
    E::Ent(Raw { name: "frac14", latex: "\\textonequarter{}", math: false, html: "&frac14;", ascii: "1/4", latin1: "¼", utf8: "¼" }),
    E::Ent(Raw { name: "frac34", latex: "\\textthreequarters{}", math: false, html: "&frac34;", ascii: "3/4", latin1: "¾", utf8: "¾" }),
    E::Ent(Raw { name: "permil", latex: "\\textperthousand{}", math: false, html: "&permil;", ascii: "per thousand", latin1: "per thousand", utf8: "‰" }),
    E::Ent(Raw { name: "sup1", latex: "\\textonesuperior{}", math: false, html: "&sup1;", ascii: "^1", latin1: "¹", utf8: "¹" }),
    E::Ent(Raw { name: "sup2", latex: "\\texttwosuperior{}", math: false, html: "&sup2;", ascii: "^2", latin1: "²", utf8: "²" }),
    E::Ent(Raw { name: "sup3", latex: "\\textthreesuperior{}", math: false, html: "&sup3;", ascii: "^3", latin1: "³", utf8: "³" }),
    E::Ent(Raw { name: "radic", latex: "\\sqrt{\\,}", math: true, html: "&radic;", ascii: "[square root]", latin1: "[square root]", utf8: "√" }),
    E::Ent(Raw { name: "sum", latex: "\\sum", math: true, html: "&sum;", ascii: "[sum]", latin1: "[sum]", utf8: "∑" }),
    E::Ent(Raw { name: "prod", latex: "\\prod", math: true, html: "&prod;", ascii: "[product]", latin1: "[n-ary product]", utf8: "∏" }),
    E::Ent(Raw { name: "micro", latex: "\\textmu{}", math: false, html: "&micro;", ascii: "micro", latin1: "µ", utf8: "µ" }),
    E::Ent(Raw { name: "macr", latex: "\\textasciimacron{}", math: false, html: "&macr;", ascii: "[macron]", latin1: "¯", utf8: "¯" }),
    E::Ent(Raw { name: "deg", latex: "\\textdegree{}", math: false, html: "&deg;", ascii: "degree", latin1: "°", utf8: "°" }),
    E::Ent(Raw { name: "prime", latex: "\\prime", math: true, html: "&prime;", ascii: "'", latin1: "'", utf8: "′" }),
    E::Ent(Raw { name: "Prime", latex: "\\prime{}\\prime", math: true, html: "&Prime;", ascii: "''", latin1: "''", utf8: "″" }),
    E::Ent(Raw { name: "infin", latex: "\\infty", math: true, html: "&infin;", ascii: "[infinity]", latin1: "[infinity]", utf8: "∞" }),
    E::Ent(Raw { name: "infty", latex: "\\infty", math: true, html: "&infin;", ascii: "[infinity]", latin1: "[infinity]", utf8: "∞" }),
    E::Ent(Raw { name: "prop", latex: "\\propto", math: true, html: "&prop;", ascii: "[proportional to]", latin1: "[proportional to]", utf8: "∝" }),
    E::Ent(Raw { name: "propto", latex: "\\propto", math: true, html: "&prop;", ascii: "[proportional to]", latin1: "[proportional to]", utf8: "∝" }),
    E::Ent(Raw { name: "not", latex: "\\textlnot{}", math: false, html: "&not;", ascii: "[angled dash]", latin1: "¬", utf8: "¬" }),
    E::Ent(Raw { name: "neg", latex: "\\neg{}", math: true, html: "&not;", ascii: "[angled dash]", latin1: "¬", utf8: "¬" }),
    E::Ent(Raw { name: "land", latex: "\\land", math: true, html: "&and;", ascii: "[logical and]", latin1: "[logical and]", utf8: "∧" }),
    E::Ent(Raw { name: "wedge", latex: "\\wedge", math: true, html: "&and;", ascii: "[logical and]", latin1: "[logical and]", utf8: "∧" }),
    E::Ent(Raw { name: "lor", latex: "\\lor", math: true, html: "&or;", ascii: "[logical or]", latin1: "[logical or]", utf8: "∨" }),
    E::Ent(Raw { name: "vee", latex: "\\vee", math: true, html: "&or;", ascii: "[logical or]", latin1: "[logical or]", utf8: "∨" }),
    E::Ent(Raw { name: "cap", latex: "\\cap", math: true, html: "&cap;", ascii: "[intersection]", latin1: "[intersection]", utf8: "∩" }),
    E::Ent(Raw { name: "cup", latex: "\\cup", math: true, html: "&cup;", ascii: "[union]", latin1: "[union]", utf8: "∪" }),
    E::Ent(Raw { name: "smile", latex: "\\smile", math: true, html: "&smile;", ascii: "[cup product]", latin1: "[cup product]", utf8: "⌣" }),
    E::Ent(Raw { name: "frown", latex: "\\frown", math: true, html: "&frown;", ascii: "[Cap product]", latin1: "[cap product]", utf8: "⌢" }),
    E::Ent(Raw { name: "int", latex: "\\int", math: true, html: "&int;", ascii: "[integral]", latin1: "[integral]", utf8: "∫" }),
    E::Ent(Raw { name: "therefore", latex: "\\therefore", math: true, html: "&there4;", ascii: "[therefore]", latin1: "[therefore]", utf8: "∴" }),
    E::Ent(Raw { name: "there4", latex: "\\therefore", math: true, html: "&there4;", ascii: "[therefore]", latin1: "[therefore]", utf8: "∴" }),
    E::Ent(Raw { name: "because", latex: "\\because", math: true, html: "&because;", ascii: "[because]", latin1: "[because]", utf8: "∵" }),
    E::Ent(Raw { name: "sim", latex: "\\sim", math: true, html: "&sim;", ascii: "~", latin1: "~", utf8: "∼" }),
    E::Ent(Raw { name: "cong", latex: "\\cong", math: true, html: "&cong;", ascii: "[approx. equal to]", latin1: "[approx. equal to]", utf8: "≅" }),
    E::Ent(Raw { name: "simeq", latex: "\\simeq", math: true, html: "&cong;", ascii: "[approx. equal to]", latin1: "[approx. equal to]", utf8: "≅" }),
    E::Ent(Raw { name: "asymp", latex: "\\asymp", math: true, html: "&asymp;", ascii: "[almost equal to]", latin1: "[almost equal to]", utf8: "≈" }),
    E::Ent(Raw { name: "approx", latex: "\\approx", math: true, html: "&asymp;", ascii: "[almost equal to]", latin1: "[almost equal to]", utf8: "≈" }),
    E::Ent(Raw { name: "ne", latex: "\\ne", math: true, html: "&ne;", ascii: "[not equal to]", latin1: "[not equal to]", utf8: "≠" }),
    E::Ent(Raw { name: "neq", latex: "\\neq", math: true, html: "&ne;", ascii: "[not equal to]", latin1: "[not equal to]", utf8: "≠" }),
    E::Ent(Raw { name: "equiv", latex: "\\equiv", math: true, html: "&equiv;", ascii: "[identical to]", latin1: "[identical to]", utf8: "≡" }),
    E::Ent(Raw { name: "triangleq", latex: "\\triangleq", math: true, html: "&triangleq;", ascii: "[defined to]", latin1: "[defined to]", utf8: "≜" }),
    E::Ent(Raw { name: "le", latex: "\\le", math: true, html: "&le;", ascii: "<=", latin1: "<=", utf8: "≤" }),
    E::Ent(Raw { name: "leq", latex: "\\le", math: true, html: "&le;", ascii: "<=", latin1: "<=", utf8: "≤" }),
    E::Ent(Raw { name: "ge", latex: "\\ge", math: true, html: "&ge;", ascii: ">=", latin1: ">=", utf8: "≥" }),
    E::Ent(Raw { name: "geq", latex: "\\ge", math: true, html: "&ge;", ascii: ">=", latin1: ">=", utf8: "≥" }),
    E::Ent(Raw { name: "lessgtr", latex: "\\lessgtr", math: true, html: "&lessgtr;", ascii: "[less than or greater than]", latin1: "[less than or greater than]", utf8: "≶" }),
    E::Ent(Raw { name: "lesseqgtr", latex: "\\lesseqgtr", math: true, html: "&lesseqgtr;", ascii: "[less than or equal or greater than or equal]", latin1: "[less than or equal or greater than or equal]", utf8: "⋚" }),
    E::Ent(Raw { name: "ll", latex: "\\ll", math: true, html: "&Lt;", ascii: "<<", latin1: "<<", utf8: "≪" }),
    E::Ent(Raw { name: "Ll", latex: "\\lll", math: true, html: "&Ll;", ascii: "<<<", latin1: "<<<", utf8: "⋘" }),
    E::Ent(Raw { name: "lll", latex: "\\lll", math: true, html: "&Ll;", ascii: "<<<", latin1: "<<<", utf8: "⋘" }),
    E::Ent(Raw { name: "gg", latex: "\\gg", math: true, html: "&Gt;", ascii: ">>", latin1: ">>", utf8: "≫" }),
    E::Ent(Raw { name: "Gg", latex: "\\ggg", math: true, html: "&Gg;", ascii: ">>>", latin1: ">>>", utf8: "⋙" }),
    E::Ent(Raw { name: "ggg", latex: "\\ggg", math: true, html: "&Gg;", ascii: ">>>", latin1: ">>>", utf8: "⋙" }),
    E::Ent(Raw { name: "prec", latex: "\\prec", math: true, html: "&pr;", ascii: "[precedes]", latin1: "[precedes]", utf8: "≺" }),
    E::Ent(Raw { name: "preceq", latex: "\\preceq", math: true, html: "&prcue;", ascii: "[precedes or equal]", latin1: "[precedes or equal]", utf8: "≼" }),
    E::Ent(Raw { name: "preccurlyeq", latex: "\\preccurlyeq", math: true, html: "&prcue;", ascii: "[precedes or equal]", latin1: "[precedes or equal]", utf8: "≼" }),
    E::Ent(Raw { name: "succ", latex: "\\succ", math: true, html: "&sc;", ascii: "[succeeds]", latin1: "[succeeds]", utf8: "≻" }),
    E::Ent(Raw { name: "succeq", latex: "\\succeq", math: true, html: "&sccue;", ascii: "[succeeds or equal]", latin1: "[succeeds or equal]", utf8: "≽" }),
    E::Ent(Raw { name: "succcurlyeq", latex: "\\succcurlyeq", math: true, html: "&sccue;", ascii: "[succeeds or equal]", latin1: "[succeeds or equal]", utf8: "≽" }),
    E::Ent(Raw { name: "sub", latex: "\\subset", math: true, html: "&sub;", ascii: "[subset of]", latin1: "[subset of]", utf8: "⊂" }),
    E::Ent(Raw { name: "subset", latex: "\\subset", math: true, html: "&sub;", ascii: "[subset of]", latin1: "[subset of]", utf8: "⊂" }),
    E::Ent(Raw { name: "sup", latex: "\\supset", math: true, html: "&sup;", ascii: "[superset of]", latin1: "[superset of]", utf8: "⊃" }),
    E::Ent(Raw { name: "supset", latex: "\\supset", math: true, html: "&sup;", ascii: "[superset of]", latin1: "[superset of]", utf8: "⊃" }),
    E::Ent(Raw { name: "nsub", latex: "\\not\\subset", math: true, html: "&nsub;", ascii: "[not a subset of]", latin1: "[not a subset of", utf8: "⊄" }),
    E::Ent(Raw { name: "sube", latex: "\\subseteq", math: true, html: "&sube;", ascii: "[subset of or equal to]", latin1: "[subset of or equal to]", utf8: "⊆" }),
    E::Ent(Raw { name: "nsup", latex: "\\not\\supset", math: true, html: "&nsup;", ascii: "[not a superset of]", latin1: "[not a superset of]", utf8: "⊅" }),
    E::Ent(Raw { name: "supe", latex: "\\supseteq", math: true, html: "&supe;", ascii: "[superset of or equal to]", latin1: "[superset of or equal to]", utf8: "⊇" }),
    E::Ent(Raw { name: "setminus", latex: "\\setminus", math: true, html: "&setminus;", ascii: "\" ", latin1: "\"", utf8: "⧵" }),
    E::Ent(Raw { name: "forall", latex: "\\forall", math: true, html: "&forall;", ascii: "[for all]", latin1: "[for all]", utf8: "∀" }),
    E::Ent(Raw { name: "exist", latex: "\\exists", math: true, html: "&exist;", ascii: "[there exists]", latin1: "[there exists]", utf8: "∃" }),
    E::Ent(Raw { name: "exists", latex: "\\exists", math: true, html: "&exist;", ascii: "[there exists]", latin1: "[there exists]", utf8: "∃" }),
    E::Ent(Raw { name: "nexist", latex: "\\nexists", math: true, html: "&exist;", ascii: "[there does not exists]", latin1: "[there does not  exists]", utf8: "∄" }),
    E::Ent(Raw { name: "nexists", latex: "\\nexists", math: true, html: "&exist;", ascii: "[there does not exists]", latin1: "[there does not  exists]", utf8: "∄" }),
    E::Ent(Raw { name: "empty", latex: "\\emptyset", math: true, html: "&empty;", ascii: "[empty set]", latin1: "[empty set]", utf8: "∅" }),
    E::Ent(Raw { name: "emptyset", latex: "\\emptyset", math: true, html: "&empty;", ascii: "[empty set]", latin1: "[empty set]", utf8: "∅" }),
    E::Ent(Raw { name: "isin", latex: "\\in", math: true, html: "&isin;", ascii: "[element of]", latin1: "[element of]", utf8: "∈" }),
    E::Ent(Raw { name: "in", latex: "\\in", math: true, html: "&isin;", ascii: "[element of]", latin1: "[element of]", utf8: "∈" }),
    E::Ent(Raw { name: "notin", latex: "\\notin", math: true, html: "&notin;", ascii: "[not an element of]", latin1: "[not an element of]", utf8: "∉" }),
    E::Ent(Raw { name: "ni", latex: "\\ni", math: true, html: "&ni;", ascii: "[contains as member]", latin1: "[contains as member]", utf8: "∋" }),
    E::Ent(Raw { name: "nabla", latex: "\\nabla", math: true, html: "&nabla;", ascii: "[nabla]", latin1: "[nabla]", utf8: "∇" }),
    E::Ent(Raw { name: "ang", latex: "\\angle", math: true, html: "&ang;", ascii: "[angle]", latin1: "[angle]", utf8: "∠" }),
    E::Ent(Raw { name: "angle", latex: "\\angle", math: true, html: "&ang;", ascii: "[angle]", latin1: "[angle]", utf8: "∠" }),
    E::Ent(Raw { name: "perp", latex: "\\perp", math: true, html: "&perp;", ascii: "[up tack]", latin1: "[up tack]", utf8: "⊥" }),
    E::Ent(Raw { name: "parallel", latex: "\\parallel", math: true, html: "&parallel;", ascii: "||", latin1: "||", utf8: "∥" }),
    E::Ent(Raw { name: "sdot", latex: "\\cdot", math: true, html: "&sdot;", ascii: "[dot]", latin1: "[dot]", utf8: "⋅" }),
    E::Ent(Raw { name: "cdot", latex: "\\cdot", math: true, html: "&sdot;", ascii: "[dot]", latin1: "[dot]", utf8: "⋅" }),
    E::Ent(Raw { name: "lceil", latex: "\\lceil", math: true, html: "&lceil;", ascii: "[left ceiling]", latin1: "[left ceiling]", utf8: "⌈" }),
    E::Ent(Raw { name: "rceil", latex: "\\rceil", math: true, html: "&rceil;", ascii: "[right ceiling]", latin1: "[right ceiling]", utf8: "⌉" }),
    E::Ent(Raw { name: "lfloor", latex: "\\lfloor", math: true, html: "&lfloor;", ascii: "[left floor]", latin1: "[left floor]", utf8: "⌊" }),
    E::Ent(Raw { name: "rfloor", latex: "\\rfloor", math: true, html: "&rfloor;", ascii: "[right floor]", latin1: "[right floor]", utf8: "⌋" }),
    E::Ent(Raw { name: "lang", latex: "\\langle", math: true, html: "&lang;", ascii: "<", latin1: "<", utf8: "⟨" }),
    E::Ent(Raw { name: "rang", latex: "\\rangle", math: true, html: "&rang;", ascii: ">", latin1: ">", utf8: "⟩" }),
    E::Ent(Raw { name: "langle", latex: "\\langle", math: true, html: "&lang;", ascii: "<", latin1: "<", utf8: "⟨" }),
    E::Ent(Raw { name: "rangle", latex: "\\rangle", math: true, html: "&rang;", ascii: ">", latin1: ">", utf8: "⟩" }),
    E::Ent(Raw { name: "hbar", latex: "\\hbar", math: true, html: "&hbar;", ascii: "hbar", latin1: "hbar", utf8: "ℏ" }),
    E::Ent(Raw { name: "mho", latex: "\\mho", math: true, html: "&mho;", ascii: "mho", latin1: "mho", utf8: "℧" }),
    E::Head("** Arrows"),
    E::Ent(Raw { name: "larr", latex: "\\leftarrow", math: true, html: "&larr;", ascii: "<-", latin1: "<-", utf8: "←" }),
    E::Ent(Raw { name: "leftarrow", latex: "\\leftarrow", math: true, html: "&larr;", ascii: "<-", latin1: "<-", utf8: "←" }),
    E::Ent(Raw { name: "gets", latex: "\\gets", math: true, html: "&larr;", ascii: "<-", latin1: "<-", utf8: "←" }),
    E::Ent(Raw { name: "lArr", latex: "\\Leftarrow", math: true, html: "&lArr;", ascii: "<=", latin1: "<=", utf8: "⇐" }),
    E::Ent(Raw { name: "Leftarrow", latex: "\\Leftarrow", math: true, html: "&lArr;", ascii: "<=", latin1: "<=", utf8: "⇐" }),
    E::Ent(Raw { name: "uarr", latex: "\\uparrow", math: true, html: "&uarr;", ascii: "[uparrow]", latin1: "[uparrow]", utf8: "↑" }),
    E::Ent(Raw { name: "uparrow", latex: "\\uparrow", math: true, html: "&uarr;", ascii: "[uparrow]", latin1: "[uparrow]", utf8: "↑" }),
    E::Ent(Raw { name: "uArr", latex: "\\Uparrow", math: true, html: "&uArr;", ascii: "[dbluparrow]", latin1: "[dbluparrow]", utf8: "⇑" }),
    E::Ent(Raw { name: "Uparrow", latex: "\\Uparrow", math: true, html: "&uArr;", ascii: "[dbluparrow]", latin1: "[dbluparrow]", utf8: "⇑" }),
    E::Ent(Raw { name: "rarr", latex: "\\rightarrow", math: true, html: "&rarr;", ascii: "->", latin1: "->", utf8: "→" }),
    E::Ent(Raw { name: "to", latex: "\\to", math: true, html: "&rarr;", ascii: "->", latin1: "->", utf8: "→" }),
    E::Ent(Raw { name: "rightarrow", latex: "\\rightarrow", math: true, html: "&rarr;", ascii: "->", latin1: "->", utf8: "→" }),
    E::Ent(Raw { name: "rArr", latex: "\\Rightarrow", math: true, html: "&rArr;", ascii: "=>", latin1: "=>", utf8: "⇒" }),
    E::Ent(Raw { name: "Rightarrow", latex: "\\Rightarrow", math: true, html: "&rArr;", ascii: "=>", latin1: "=>", utf8: "⇒" }),
    E::Ent(Raw { name: "darr", latex: "\\downarrow", math: true, html: "&darr;", ascii: "[downarrow]", latin1: "[downarrow]", utf8: "↓" }),
    E::Ent(Raw { name: "downarrow", latex: "\\downarrow", math: true, html: "&darr;", ascii: "[downarrow]", latin1: "[downarrow]", utf8: "↓" }),
    E::Ent(Raw { name: "dArr", latex: "\\Downarrow", math: true, html: "&dArr;", ascii: "[dbldownarrow]", latin1: "[dbldownarrow]", utf8: "⇓" }),
    E::Ent(Raw { name: "Downarrow", latex: "\\Downarrow", math: true, html: "&dArr;", ascii: "[dbldownarrow]", latin1: "[dbldownarrow]", utf8: "⇓" }),
    E::Ent(Raw { name: "harr", latex: "\\leftrightarrow", math: true, html: "&harr;", ascii: "<->", latin1: "<->", utf8: "↔" }),
    E::Ent(Raw { name: "leftrightarrow", latex: "\\leftrightarrow", math: true, html: "&harr;", ascii: "<->", latin1: "<->", utf8: "↔" }),
    E::Ent(Raw { name: "hArr", latex: "\\Leftrightarrow", math: true, html: "&hArr;", ascii: "<=>", latin1: "<=>", utf8: "⇔" }),
    E::Ent(Raw { name: "Leftrightarrow", latex: "\\Leftrightarrow", math: true, html: "&hArr;", ascii: "<=>", latin1: "<=>", utf8: "⇔" }),
    E::Ent(Raw { name: "crarr", latex: "\\hookleftarrow", math: true, html: "&crarr;", ascii: "<-'", latin1: "<-'", utf8: "↵" }),
    E::Ent(Raw { name: "hookleftarrow", latex: "\\hookleftarrow", math: true, html: "&crarr;", ascii: "<-'", latin1: "<-'", utf8: "↵" }),
    E::Head("** Function names"),
    E::Ent(Raw { name: "arccos", latex: "\\arccos", math: true, html: "arccos", ascii: "arccos", latin1: "arccos", utf8: "arccos" }),
    E::Ent(Raw { name: "arcsin", latex: "\\arcsin", math: true, html: "arcsin", ascii: "arcsin", latin1: "arcsin", utf8: "arcsin" }),
    E::Ent(Raw { name: "arctan", latex: "\\arctan", math: true, html: "arctan", ascii: "arctan", latin1: "arctan", utf8: "arctan" }),
    E::Ent(Raw { name: "arg", latex: "\\arg", math: true, html: "arg", ascii: "arg", latin1: "arg", utf8: "arg" }),
    E::Ent(Raw { name: "cos", latex: "\\cos", math: true, html: "cos", ascii: "cos", latin1: "cos", utf8: "cos" }),
    E::Ent(Raw { name: "cosh", latex: "\\cosh", math: true, html: "cosh", ascii: "cosh", latin1: "cosh", utf8: "cosh" }),
    E::Ent(Raw { name: "cot", latex: "\\cot", math: true, html: "cot", ascii: "cot", latin1: "cot", utf8: "cot" }),
    E::Ent(Raw { name: "coth", latex: "\\coth", math: true, html: "coth", ascii: "coth", latin1: "coth", utf8: "coth" }),
    E::Ent(Raw { name: "csc", latex: "\\csc", math: true, html: "csc", ascii: "csc", latin1: "csc", utf8: "csc" }),
    E::Ent(Raw { name: "deg", latex: "\\deg", math: true, html: "&deg;", ascii: "deg", latin1: "deg", utf8: "deg" }),
    E::Ent(Raw { name: "det", latex: "\\det", math: true, html: "det", ascii: "det", latin1: "det", utf8: "det" }),
    E::Ent(Raw { name: "dim", latex: "\\dim", math: true, html: "dim", ascii: "dim", latin1: "dim", utf8: "dim" }),
    E::Ent(Raw { name: "exp", latex: "\\exp", math: true, html: "exp", ascii: "exp", latin1: "exp", utf8: "exp" }),
    E::Ent(Raw { name: "gcd", latex: "\\gcd", math: true, html: "gcd", ascii: "gcd", latin1: "gcd", utf8: "gcd" }),
    E::Ent(Raw { name: "hom", latex: "\\hom", math: true, html: "hom", ascii: "hom", latin1: "hom", utf8: "hom" }),
    E::Ent(Raw { name: "inf", latex: "\\inf", math: true, html: "inf", ascii: "inf", latin1: "inf", utf8: "inf" }),
    E::Ent(Raw { name: "ker", latex: "\\ker", math: true, html: "ker", ascii: "ker", latin1: "ker", utf8: "ker" }),
    E::Ent(Raw { name: "lg", latex: "\\lg", math: true, html: "lg", ascii: "lg", latin1: "lg", utf8: "lg" }),
    E::Ent(Raw { name: "lim", latex: "\\lim", math: true, html: "lim", ascii: "lim", latin1: "lim", utf8: "lim" }),
    E::Ent(Raw { name: "liminf", latex: "\\liminf", math: true, html: "liminf", ascii: "liminf", latin1: "liminf", utf8: "liminf" }),
    E::Ent(Raw { name: "limsup", latex: "\\limsup", math: true, html: "limsup", ascii: "limsup", latin1: "limsup", utf8: "limsup" }),
    E::Ent(Raw { name: "ln", latex: "\\ln", math: true, html: "ln", ascii: "ln", latin1: "ln", utf8: "ln" }),
    E::Ent(Raw { name: "log", latex: "\\log", math: true, html: "log", ascii: "log", latin1: "log", utf8: "log" }),
    E::Ent(Raw { name: "max", latex: "\\max", math: true, html: "max", ascii: "max", latin1: "max", utf8: "max" }),
    E::Ent(Raw { name: "min", latex: "\\min", math: true, html: "min", ascii: "min", latin1: "min", utf8: "min" }),
    E::Ent(Raw { name: "Pr", latex: "\\Pr", math: true, html: "Pr", ascii: "Pr", latin1: "Pr", utf8: "Pr" }),
    E::Ent(Raw { name: "sec", latex: "\\sec", math: true, html: "sec", ascii: "sec", latin1: "sec", utf8: "sec" }),
    E::Ent(Raw { name: "sin", latex: "\\sin", math: true, html: "sin", ascii: "sin", latin1: "sin", utf8: "sin" }),
    E::Ent(Raw { name: "sinh", latex: "\\sinh", math: true, html: "sinh", ascii: "sinh", latin1: "sinh", utf8: "sinh" }),
    E::Ent(Raw { name: "sup", latex: "\\sup", math: true, html: "&sup;", ascii: "sup", latin1: "sup", utf8: "sup" }),
    E::Ent(Raw { name: "tan", latex: "\\tan", math: true, html: "tan", ascii: "tan", latin1: "tan", utf8: "tan" }),
    E::Ent(Raw { name: "tanh", latex: "\\tanh", math: true, html: "tanh", ascii: "tanh", latin1: "tanh", utf8: "tanh" }),
    E::Head("** Signs & Symbols"),
    E::Ent(Raw { name: "bull", latex: "\\textbullet{}", math: false, html: "&bull;", ascii: "*", latin1: "*", utf8: "•" }),
    E::Ent(Raw { name: "bullet", latex: "\\textbullet{}", math: false, html: "&bull;", ascii: "*", latin1: "*", utf8: "•" }),
    E::Ent(Raw { name: "star", latex: "\\star", math: true, html: "*", ascii: "*", latin1: "*", utf8: "⋆" }),
    E::Ent(Raw { name: "lowast", latex: "\\ast", math: true, html: "&lowast;", ascii: "*", latin1: "*", utf8: "∗" }),
    E::Ent(Raw { name: "ast", latex: "\\ast", math: true, html: "&lowast;", ascii: "*", latin1: "*", utf8: "*" }),
    E::Ent(Raw { name: "odot", latex: "\\odot", math: true, html: "o", ascii: "[circled dot]", latin1: "[circled dot]", utf8: "ʘ" }),
    E::Ent(Raw { name: "oplus", latex: "\\oplus", math: true, html: "&oplus;", ascii: "[circled plus]", latin1: "[circled plus]", utf8: "⊕" }),
    E::Ent(Raw { name: "otimes", latex: "\\otimes", math: true, html: "&otimes;", ascii: "[circled times]", latin1: "[circled times]", utf8: "⊗" }),
    E::Ent(Raw { name: "check", latex: "\\checkmark", math: true, html: "&checkmark;", ascii: "[checkmark]", latin1: "[checkmark]", utf8: "✓" }),
    E::Ent(Raw { name: "checkmark", latex: "\\checkmark", math: true, html: "&check;", ascii: "[checkmark]", latin1: "[checkmark]", utf8: "✓" }),
    E::Head("** Miscellaneous (seldom used)"),
    E::Ent(Raw { name: "ordf", latex: "\\textordfeminine{}", math: false, html: "&ordf;", ascii: "_a_", latin1: "ª", utf8: "ª" }),
    E::Ent(Raw { name: "ordm", latex: "\\textordmasculine{}", math: false, html: "&ordm;", ascii: "_o_", latin1: "º", utf8: "º" }),
    E::Ent(Raw { name: "cedil", latex: "\\c{}", math: false, html: "&cedil;", ascii: "[cedilla]", latin1: "¸", utf8: "¸" }),
    E::Ent(Raw { name: "oline", latex: "\\overline{~}", math: true, html: "&oline;", ascii: "[overline]", latin1: "¯", utf8: "‾" }),
    E::Ent(Raw { name: "uml", latex: "\\textasciidieresis{}", math: false, html: "&uml;", ascii: "[diaeresis]", latin1: "¨", utf8: "¨" }),
    E::Ent(Raw { name: "zwnj", latex: "\\/{}", math: false, html: "&zwnj;", ascii: "", latin1: "", utf8: "‌" }),
    E::Ent(Raw { name: "zwj", latex: "", math: false, html: "&zwj;", ascii: "", latin1: "", utf8: "‍" }),
    E::Ent(Raw { name: "lrm", latex: "", math: false, html: "&lrm;", ascii: "", latin1: "", utf8: "‎" }),
    E::Ent(Raw { name: "rlm", latex: "", math: false, html: "&rlm;", ascii: "", latin1: "", utf8: "‏" }),
    E::Head("** Smilies"),
    E::Ent(Raw { name: "smiley", latex: "\\ddot\\smile", math: true, html: "&#9786;", ascii: ":-)", latin1: ":-)", utf8: "☺" }),
    E::Ent(Raw { name: "blacksmile", latex: "\\ddot\\smile", math: true, html: "&#9787;", ascii: ":-)", latin1: ":-)", utf8: "☻" }),
    E::Ent(Raw { name: "sad", latex: "\\ddot\\frown", math: true, html: "&#9785;", ascii: ":-(", latin1: ":-(", utf8: "☹" }),
    E::Ent(Raw { name: "frowny", latex: "\\ddot\\frown", math: true, html: "&#9785;", ascii: ":-(", latin1: ":-(", utf8: "☹" }),
    E::Head("** Suits"),
    E::Ent(Raw { name: "clubs", latex: "\\clubsuit", math: true, html: "&clubs;", ascii: "[clubs]", latin1: "[clubs]", utf8: "♣" }),
    E::Ent(Raw { name: "clubsuit", latex: "\\clubsuit", math: true, html: "&clubs;", ascii: "[clubs]", latin1: "[clubs]", utf8: "♣" }),
    E::Ent(Raw { name: "spades", latex: "\\spadesuit", math: true, html: "&spades;", ascii: "[spades]", latin1: "[spades]", utf8: "♠" }),
    E::Ent(Raw { name: "spadesuit", latex: "\\spadesuit", math: true, html: "&spades;", ascii: "[spades]", latin1: "[spades]", utf8: "♠" }),
    E::Ent(Raw { name: "hearts", latex: "\\heartsuit", math: true, html: "&hearts;", ascii: "[hearts]", latin1: "[hearts]", utf8: "♥" }),
    E::Ent(Raw { name: "heartsuit", latex: "\\heartsuit", math: true, html: "&heartsuit;", ascii: "[hearts]", latin1: "[hearts]", utf8: "♥" }),
    E::Ent(Raw { name: "diams", latex: "\\diamondsuit", math: true, html: "&diams;", ascii: "[diamonds]", latin1: "[diamonds]", utf8: "◆" }),
    E::Ent(Raw { name: "diamondsuit", latex: "\\diamondsuit", math: true, html: "&diams;", ascii: "[diamonds]", latin1: "[diamonds]", utf8: "◆" }),
    E::Ent(Raw { name: "diamond", latex: "\\diamondsuit", math: true, html: "&diamond;", ascii: "[diamond]", latin1: "[diamond]", utf8: "◆" }),
    E::Ent(Raw { name: "Diamond", latex: "\\diamondsuit", math: true, html: "&diamond;", ascii: "[diamond]", latin1: "[diamond]", utf8: "◆" }),
    E::Ent(Raw { name: "loz", latex: "\\lozenge", math: true, html: "&loz;", ascii: "[lozenge]", latin1: "[lozenge]", utf8: "⧫" }),
    E::Ent(Raw { name: "_ ", latex: "\\hspace*{0.5em}", math: false, html: "&ensp;", ascii: " ", latin1: " ", utf8: " " }),
    E::Ent(Raw { name: "_  ", latex: "\\hspace*{1.0em}", math: false, html: "&ensp;&ensp;", ascii: "  ", latin1: "  ", utf8: "  " }),
    E::Ent(Raw { name: "_   ", latex: "\\hspace*{1.5em}", math: false, html: "&ensp;&ensp;&ensp;", ascii: "   ", latin1: "   ", utf8: "   " }),
    E::Ent(Raw { name: "_    ", latex: "\\hspace*{2.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;", ascii: "    ", latin1: "    ", utf8: "    " }),
    E::Ent(Raw { name: "_     ", latex: "\\hspace*{2.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "     ", latin1: "     ", utf8: "     " }),
    E::Ent(Raw { name: "_      ", latex: "\\hspace*{3.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "      ", latin1: "      ", utf8: "      " }),
    E::Ent(Raw { name: "_       ", latex: "\\hspace*{3.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "       ", latin1: "       ", utf8: "       " }),
    E::Ent(Raw { name: "_        ", latex: "\\hspace*{4.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "        ", latin1: "        ", utf8: "        " }),
    E::Ent(Raw { name: "_         ", latex: "\\hspace*{4.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "         ", latin1: "         ", utf8: "         " }),
    E::Ent(Raw { name: "_          ", latex: "\\hspace*{5.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "          ", latin1: "          ", utf8: "          " }),
    E::Ent(Raw { name: "_           ", latex: "\\hspace*{5.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "           ", latin1: "           ", utf8: "           " }),
    E::Ent(Raw { name: "_            ", latex: "\\hspace*{6.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "            ", latin1: "            ", utf8: "            " }),
    E::Ent(Raw { name: "_             ", latex: "\\hspace*{6.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "             ", latin1: "             ", utf8: "             " }),
    E::Ent(Raw { name: "_              ", latex: "\\hspace*{7.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "              ", latin1: "              ", utf8: "              " }),
    E::Ent(Raw { name: "_               ", latex: "\\hspace*{7.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "               ", latin1: "               ", utf8: "               " }),
    E::Ent(Raw { name: "_                ", latex: "\\hspace*{8.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "                ", latin1: "                ", utf8: "                " }),
    E::Ent(Raw { name: "_                 ", latex: "\\hspace*{8.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "                 ", latin1: "                 ", utf8: "                 " }),
    E::Ent(Raw { name: "_                  ", latex: "\\hspace*{9.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "                  ", latin1: "                  ", utf8: "                  " }),
    E::Ent(Raw { name: "_                   ", latex: "\\hspace*{9.5em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "                   ", latin1: "                   ", utf8: "                   " }),
    E::Ent(Raw { name: "_                    ", latex: "\\hspace*{10.0em}", math: false, html: "&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;&ensp;", ascii: "                    ", latin1: "                    ", utf8: "                    " }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_and_lookup() {
        assert_eq!(ORG_ENTITIES.iter().filter(|e| matches!(e, E::Ent(_))).count(), 414);
        let a = get("alpha").unwrap();
        assert_eq!((a.latex.as_str(), a.math, a.html.as_str(), a.utf8.as_str()), ("\\alpha", true, "&alpha;", "α"));
        assert_eq!(get("Auml").unwrap().ascii, "Ae");
        assert!(get("nosuch").is_none());
        assert!(names().iter().any(|n| n == "rightarrow"));
        assert!(help_text().contains("   α        \\alpha            \\alpha                 &alpha;      \n"));
    }

    #[test]
    fn entity_display() {
        let l = r"a \alpha b \beta{} \pi2 \there4 \notanentity \_ x";
        let o = entity_overlays(l);
        let shown: Vec<(&str, &str)> = o.iter().map(|(r, s)| (&l[r.clone()], s.as_str())).collect();
        assert_eq!(shown, vec![(r"\alpha", "α"), (r"\beta{}", "β"), (r"\pi", "π"), (r"\there4", "∴")]);
        assert!(entity_overlays(r"# \alpha").is_empty());
        // \alphabet is not an entity name.
        assert!(entity_overlays(r"\alphabet").is_empty());
    }

    #[test]
    fn script_display() {
        let l = "x^2 and a_{ij} and e^{i pi} and H_2O x^(n+1)";
        let o = script_overlays(l, false);
        let shown: Vec<(&str, &str)> = o.iter().map(|(r, s)| (&l[r.clone()], s.as_str())).collect();
        assert_eq!(shown, vec![("^2", "²"), ("_{ij}", "ᵢⱼ"), ("^(n+1)", "⁽ⁿ⁺¹⁾")]);
        // `{}`: braces are required.
        let o = script_overlays("x^2 y^{2}", true);
        assert_eq!(o, vec![(5..9, "²".into())]);
        assert!(script_overlays("[[file:a_b]]", false).is_empty());
    }
}
