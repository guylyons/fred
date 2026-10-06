//! Smart quotes (org-export-activate-smart-quotes) and translation
//! (org-export-translate with org-export-dictionary).

use super::Info;
use crate::org::element::{NodeId, RECURSIVE_OBJECTS, Type};
use crate::org::sexp::{self, Sexp};
use std::sync::LazyLock;

/// Quote kinds, with None for a quote left as is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quote {
    PrimaryOpening,
    PrimaryClosing,
    SecondaryOpening,
    SecondaryClosing,
    Apostrophe,
}

impl Quote {
    fn name(self) -> &'static str {
        match self {
            Quote::PrimaryOpening => "primary-opening",
            Quote::PrimaryClosing => "primary-closing",
            Quote::SecondaryOpening => "secondary-opening",
            Quote::SecondaryClosing => "secondary-closing",
            Quote::Apostrophe => "apostrophe",
        }
    }
}

/// Quote statuses per plain-text node of a container.
pub type Status = Vec<(NodeId, Vec<Option<Quote>>)>;

static DATA: LazyLock<(Sexp, Sexp)> = LazyLock::new(|| {
    let v = sexp::read(include_str!("dictionary.sexp")).unwrap_or(Sexp::Nil);
    let l = v.list().unwrap_or(&[]);
    (
        l.first().cloned().unwrap_or(Sexp::Nil),
        l.get(1).cloned().unwrap_or(Sexp::Nil),
    )
});

fn assoc<'a>(alist: &'a Sexp, key: &str) -> Option<&'a Sexp> {
    alist
        .list()?
        .iter()
        .find(|e| e.car().and_then(Sexp::str) == Some(key))
}

/// The plist of an alist entry `(KEY . PLIST)`.
fn plist_get(entry: &Sexp, key: &str) -> Option<String> {
    let l = entry.list()?;
    l.iter()
        .skip(1)
        .position(|x| x.sym() == Some(key))
        .and_then(|i| l.get(i + 2))
        .and_then(Sexp::str)
        .map(str::to_owned)
}

/// org-export-translate: S in LANG for ENCODING (`:ascii`, `:html`,
/// `:latin1`, `:utf-8`), else `:default`, else S.
pub fn translate(s: &str, encoding: &str, lang: &str) -> String {
    let dict = super::env::var("org-export-dictionary").unwrap_or_else(|| DATA.0.clone());
    assoc(&dict, s)
        .and_then(|e| {
            let langs = Sexp::List(e.list()?.get(1..)?.to_vec());
            let t = assoc(&langs, lang)?;
            plist_get(t, encoding).or_else(|| plist_get(t, ":default"))
        })
        .unwrap_or_else(|| s.to_owned())
}

/// The translation of a quote in LANG for ENCODING.
fn quote_string(q: Quote, encoding: &str, lang: &str) -> Option<String> {
    let alist = super::env::var("org-export-smart-quotes-alist").unwrap_or_else(|| DATA.1.clone());
    let l = assoc(&alist, lang)?;
    let rest = Sexp::List(l.list()?.get(1..)?.to_vec());
    let e = assoc(&rest, q.name())?;
    plist_get(e, encoding)
}

/// Emacs syntax class of C in an Org buffer.
fn syntax(c: char) -> char {
    match c {
        ' ' | '\t' | '\n' | '\r' | '\u{a0}' | '\u{c}' => ' ',
        '"' => '"',
        '(' | '[' | '{' | '<' => '(',
        ')' | ']' | '}' | '>' => ')',
        '!' | '#' | ',' | '.' | ':' | ';' | '?' | '@' | '^' | '`' => '.',
        '&' | '*' | '+' | '-' | '/' | '=' | '\\' | '_' | '|' | '~' => '_',
        '$' | '%' | '\'' => 'w',
        c if c.is_alphanumeric() => 'w',
        c if c.is_whitespace() => ' ',
        _ => '.',
    }
}

/// A neighbor of a quote: a character, or the space status of the
/// adjacent object.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Char(char),
    Blank,
    NoBlank,
    None,
}

impl Info {
    /// org-export--smart-quote-status: the statuses of the quotes of
    /// plain-text node S.
    fn smart_quote_status(&mut self, s: NodeId) -> Vec<Option<Quote>> {
        let Some(parent) = self.tree.parent(s) else {
            return vec![];
        };
        let secondary = self.tree.secondary_p(s);
        let key = (parent, secondary);
        if let Some(st) = self.caches.smart_quotes.get(&key) {
            return st
                .iter()
                .find(|(n, _)| *n == s)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
        }
        let container: Vec<NodeId> = match secondary {
            Some(k) => self.tree.secondary(parent, k).to_vec(),
            None => self.tree.contents(parent).to_vec(),
        };
        let texts = self.map(
            &container,
            Some(&[Type::PlainText]),
            RECURSIVE_OBJECTS,
            false,
            false,
            |_, t| Some(t),
        );
        let mut level1_open = false;
        let mut full: Vec<(NodeId, Vec<Option<Quote>>)> = vec![];
        for t in texts {
            let text: Vec<char> = self.tree.node(t).text.chars().collect();
            let mut cur = vec![];
            for (i, &c) in text.iter().enumerate() {
                if c != '\'' && c != '"' {
                    continue;
                }
                let q = if c == '"' {
                    level1_open = !level1_open;
                    if level1_open {
                        Quote::PrimaryOpening
                    } else {
                        Quote::PrimaryClosing
                    }
                } else if !level1_open {
                    Quote::Apostrophe
                } else {
                    let previous = if i > 0 {
                        Side::Char(text[i - 1])
                    } else {
                        match self.previous(t) {
                            None => Side::None,
                            Some(p) => match self.tree.plain_text(p) {
                                Some(s) => s.chars().last().map_or(Side::None, Side::Char),
                                None if self.tree.post_blank(p) == 0 => Side::NoBlank,
                                None => Side::Blank,
                            },
                        }
                    };
                    let next = if i + 1 < text.len() {
                        Side::Char(text[i + 1])
                    } else {
                        match self.next(t) {
                            None => Side::None,
                            Some(n) => match self.tree.plain_text(n) {
                                Some(s) => s.chars().next().map_or(Side::None, Side::Char),
                                None => Side::NoBlank,
                            },
                        }
                    };
                    let open = (match previous {
                        Side::Char(c) => matches!(syntax(c), '"' | ' ' | '('),
                        p => matches!(p, Side::Blank | Side::None),
                    }) && (match next {
                        Side::Char(c) => matches!(syntax(c), 'w' | '.' | '_'),
                        n => n == Side::NoBlank,
                    });
                    let close = (match previous {
                        Side::Char(c) => matches!(syntax(c), 'w' | '.' | '_'),
                        p => p == Side::NoBlank,
                    }) && (match next {
                        Side::Char(c) => matches!(syntax(c), ' ' | ')' | '.' | '"'),
                        n => matches!(n, Side::Blank | Side::None),
                    });
                    if open && !close {
                        Quote::SecondaryOpening
                    } else if close && !open {
                        Quote::SecondaryClosing
                    } else {
                        Quote::Apostrophe
                    }
                };
                cur.push(Some(q));
            }
            if !cur.is_empty() {
                full.push((t, cur));
            }
        }
        // Unbalanced quotes become apostrophes.
        let mut primary: Vec<(usize, usize)> = vec![];
        let mut secondary_open: Vec<(usize, usize)> = vec![];
        for i in 0..full.len() {
            for j in 0..full[i].1.len() {
                match full[i].1[j] {
                    Some(Quote::PrimaryOpening) => primary.push((i, j)),
                    Some(Quote::SecondaryOpening) => secondary_open.push((i, j)),
                    Some(Quote::SecondaryClosing) => {
                        if secondary_open.pop().is_none() {
                            full[i].1[j] = Some(Quote::Apostrophe);
                        }
                    }
                    Some(Quote::PrimaryClosing) => {
                        for (a, b) in secondary_open.drain(..) {
                            full[a].1[b] = Some(Quote::Apostrophe);
                        }
                        primary.pop();
                    }
                    _ => {}
                }
            }
        }
        if let Some(&(pi, pj)) = primary.last() {
            full[pi].1[pj] = None;
            let mut after = false;
            for (i, (_, qs)) in full.iter_mut().enumerate() {
                for (j, q) in qs.iter_mut().enumerate() {
                    if (i, j) == (pi, pj) {
                        after = true;
                    }
                    if after
                        && matches!(*q, Some(Quote::SecondaryOpening | Quote::SecondaryClosing))
                    {
                        *q = Some(Quote::Apostrophe);
                    }
                }
            }
        }
        let r = full
            .iter()
            .find(|(n, _)| *n == s)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        self.caches.smart_quotes.insert(key, full);
        r
    }

    /// org-export-activate-smart-quotes: S (the text of node ORIGINAL)
    /// with its quotes replaced for ENCODING (`:utf-8`, `:html`, ...).
    pub fn activate_smart_quotes(
        &mut self,
        s: &str,
        encoding: &str,
        original: Option<NodeId>,
    ) -> String {
        let status = match original {
            Some(n) => self.smart_quote_status(n),
            None => vec![],
        };
        let lang = self.str("language").unwrap_or("en").to_owned();
        let mut it = status.into_iter();
        let mut out = String::new();
        for c in s.chars() {
            if c == '\'' || c == '"' {
                let q = it.next().flatten();
                match q.and_then(|q| quote_string(q, encoding, &lang)) {
                    Some(r) => out.push_str(&r),
                    None => out.push(c),
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// org-export-translate with INFO's language.
    pub fn translate(&self, s: &str, encoding: &str) -> String {
        translate(s, encoding, self.str("language").unwrap_or("en"))
    }
}
