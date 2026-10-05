//! Object parsers of org-element.el (org-element-TYPE-parser for
//! objects, org-element--object-lex, org-element--parse-objects).
//!
//! Objects are parsed in a narrowed region [NB, NE) as upstream
//! narrows to the container: the region start counts as a line start.

use super::ast::{NodeId, Type, Value};
use super::parser::{P, TS_BOTH, nw, paired, trim};
use regex::Regex;
use std::sync::LazyLock;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(
    ENTITY,
    r"(?m)\A\\(?:(_ +)|(there4|sup[123]|frac[13][24]|[a-zA-Z]+)($|\{\}|[^\p{Alphabetic}]))"
);
re!(EXPORT_SNIPPET, r"\A@@([-A-Za-z0-9]+):");
re!(FOOTNOTE_REF, r"\A\[fn:(?:([-_\w]+)?(:)|([-_\w]+)\])");
re!(INLINE_CALL, r"\Acall_([^ \t\n\[(]+)[(\[]");
re!(INLINE_SRC, r"\Asrc_([^ \t\n\[{]+)[{\[]");
re!(LATEX_MACRO, r"\A\\[a-zA-Z]+\*?(?:(\[[^\]\[\n{}]*\])|(\{[^{}\n]*\}))*");
re!(LINE_BREAK, r"(?m)\A\\\\[ \t]*$");
re!(
    LINK_BRACKET,
    r"\A\[\[((?:[^\[\]\\]|\\(?:\\\\)*[\[\]]|\\+[^\[\]])+)\](?:\[((?s:.)+?)\])?\]"
);
re!(MACRO, r"\A\{\{\{([a-zA-Z][-a-zA-Z0-9_]*)(\(((?s:.)*?)\))?\}\}\}");
re!(
    RADIO_TARGET,
    r"\A<<<([^<>\n\r \t]|[^<>\n\r \t][^<>\n\r]*[^<>\n\r \t])>>>"
);
re!(TARGET, r"\A<<([^<>\n\r \t]|[^<>\n\r \t][^<>\n\r]*[^<>\n\r \t])>>");
re!(STAT_COOKIE, r"\A\[[0-9]*(%|/[0-9]*)\]");
re!(
    SUBSCRIPT,
    r"\A(\S)([_^])((?:\{([^{}]*?|(?:[^{}]*?\{[^{}]*?\})+[^{}]*?|(?:[^{}]*?\{(?:[^{}]*?\{[^{}]*?\})+[^{}]*?\})+[^{}]*?)\})|(?:\(([^()]*?|(?:[^()]*?\([^()]*?\))+[^()]*?|(?:[^()]*?\((?:[^()]*?\([^()]*?\))+[^()]*?\))+[^()]*?)\))|(?:\*|[+-]?[\p{L}\p{N}.,\\]*[\p{L}\p{N}]))"
);
re!(TABLE_CELL, r"(?m)\A[ \t]*(.*?)[ \t]*(?:\||$)");
re!(CITE_PREFIX, r"\A\[cite(?:/([/_\-\p{L}\p{N}]+))?:[\t\n ]*");
re!(CITE_KEY, r"@([\w\-.:?!`'/*@+|(){}<>&_^$#%~]+)");
re!(
    TIMESTAMP,
    &format!(r"\A(?:{TS_BOTH}|<%%(?:\([^>\n]+\))([^\n>]*)>)")
);
re!(
    TIMESTAMP_RAW,
    &format!(r"\A([<\[](%%)?.*?)[\]>](?:--({TS_BOTH}))?")
);
re!(TIME_RANGE, r"[012]?[0-9]:[0-5][0-9](-([012]?[0-9]):([0-5][0-9]))");
re!(
    REPEATER,
    r"(\+\+|\.\+|\+)([0-9]+)([hdwmy])(?:/([0-9]+)([hdwmy]))?"
);
re!(WARNING, r"(-)?-([0-9]+)([hdwmy])");
re!(
    TS0,
    r"([0-9]{4})-([0-9]{2})-([0-9]{2})( +[^\]+0-9>\r\n -]+)?( +([0-9]{1,2}):([0-9]{2}))?"
);
re!(
    DIARY_TIME,
    r"([012]?[0-9]):([0-5][0-9])(-([012]?[0-9]):([0-5][0-9]))?"
);
re!(NEWLINE_INDENT, r"[ \t]*\n[ \t]*");
re!(FILE_TYPE, r"\Afile(?:\+(.+))?\z");
re!(SEARCH_OPTION, r"::(.*)\z");
re!(FILE_SLASHES, r"\A///*(.:)?/");
re!(ABBREV, r"\A([^:\n]*)(::?(.*))?\z");
re!(MACRO_WS, r"[ \t\r\n]+");

/// Emacs whitespace syntax in Org buffers (`[[:space:]]`).
pub(super) const SPACE: &str = r"\t\n\x0B\x0C\r \u{A0}\u{2000}-\u{200B}\u{202F}\u{205F}\u{3000}";

pub(super) fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ' | '\u{A0}' | '\u{2000}'..='\u{200B}' | '\u{202F}' | '\u{205F}' | '\u{3000}')
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// Emacs syntax classes `\s.`, `\s-`, `\s(`, `\s)`, `\s"` and `'` in Org
/// buffers.
fn latex_after_ok(c: Option<char>) -> bool {
    match c {
        None | Some('\n') => true,
        Some(c) => is_space(c) || "!\"#'(),.:;<>?@[]^`{}".contains(c),
    }
}

/// A narrowed region.
#[derive(Clone, Copy)]
pub(super) struct N {
    pub nb: usize,
    pub ne: usize,
}

impl<'a> P<'a> {
    fn n_bolp(&self, p: usize, n: N) -> bool {
        p == n.nb || self.s.as_bytes()[p - 1] == b'\n'
    }
    fn char_before(&self, p: usize, n: N) -> Option<char> {
        if p <= n.nb { None } else { self.s[..p].chars().next_back() }
    }
    fn char_at(&self, p: usize, n: N) -> Option<char> {
        if p >= n.ne { None } else { self.s[p..].chars().next() }
    }
    fn prev_char_pos(&self, p: usize) -> usize {
        p - self.s[..p].chars().next_back().map_or(0, char::len_utf8)
    }
    /// looking-at within the narrowing.
    fn nlook(&self, re: &Regex, p: usize, n: N) -> Option<regex::Captures<'a>> {
        re.captures(&self.s[p..n.ne])
    }
    /// skip-chars-forward " \t" within the narrowing.
    fn skip_blank(&self, p: usize, n: N) -> usize {
        self.skip_fwd(p, b" \t", n.ne)
    }

    /// org-element--parse-objects: objects and text between BEG and END.
    pub fn parse_objects(&mut self, beg: usize, end: usize, restriction: &[Type], parent: Option<NodeId>) -> Vec<NodeId> {
        if beg > end {
            // Upstream narrows to [END, BEG) but collects text up to END:
            // one empty string (the region only holds blanks).
            let e = self.t.text("");
            self.t.set_parent(e, parent);
            return vec![e];
        }
        let n = N { nb: beg, ne: end };
        let mut out = vec![];
        let mut p = beg;
        while p < end {
            let Some(obj) = self.object_lex(p, n, restriction) else { break };
            let ob = self.t.begin(obj).unwrap_or(p);
            if p < ob {
                let txt = self.t.text(&self.s[p..ob]);
                out.push(txt);
            }
            if let (Some(cb), Some(ce)) = (self.t.contents_begin(obj), self.t.contents_end(obj)) {
                let r = super::restriction(self.t.kind(obj));
                let c = self.parse_objects(cb, ce, r, Some(obj));
                self.t.set_contents(obj, c);
            }
            out.push(obj);
            let oe = self.t.end(obj).unwrap_or(p);
            if oe <= p {
                break;
            }
            p = oe;
        }
        if p < end {
            let txt = self.t.text(&self.s[p..end]);
            out.push(txt);
        }
        if let Some(par) = parent {
            for &o in &out {
                self.t.set_parent(o, Some(par));
            }
        }
        out
    }

    /// org-element--object-lex: the next object at or after P.
    fn object_lex(&mut self, p: usize, n: N, r: &[Type]) -> Option<NodeId> {
        if r.contains(&Type::TableCell) {
            return self.table_cell_parser(p, n);
        }
        if r.contains(&Type::CitationReference) {
            return self.citation_reference_parser(p, n);
        }
        // A radio link bounds the search: stop one character into it.
        let mut limit = None;
        if let Some(radio) = &self.cx.radio
            && r.contains(&Type::Link)
        {
            let from = if self.n_bolp(p, n) { p } else { self.prev_char_pos(p) };
            // The narrowed buffer: `^' matches at its start.
            let hay = &self.s[n.nb..n.ne];
            let one = |g: usize| g + self.s[g..].chars().next().map_or(1, char::len_utf8);
            if let Some(c) = radio.captures_at(hay, from - n.nb) {
                let (gs, ge) = (c.get(1).unwrap().start() + n.nb, c.get(1).unwrap().end() + n.nb);
                if p == self.bol(p) + 1 && p == ge {
                    if let Some(c2) = radio.captures_at(hay, c.get(0).unwrap().end()) {
                        limit = Some(one(c2.get(1).unwrap().start() + n.nb));
                    }
                } else {
                    limit = Some(one(gs));
                }
            }
        }
        let lim = limit.unwrap_or(n.ne).min(n.ne);
        let hay = &self.s[n.nb..lim];
        let re = self.cx.object_re.clone();
        let mut q = p;
        while q < lim {
            let Some(m) = re.find_at(hay, q - n.nb) else { break };
            let at = m.start() + n.nb;
            let result = &self.s[at..m.end() + n.nb];
            let found = self.object_at(at, n, r, result);
            if found.is_some() {
                return found;
            }
            if at >= lim {
                break;
            }
            q = at + self.s[at..].chars().next().map_or(1, char::len_utf8);
        }
        let limit = limit?;
        if limit > n.ne {
            return None;
        }
        let at = self.prev_char_pos(limit);
        self.link_parser(at, n)
    }

    /// The object for an object-regexp match RESULT at P.
    fn object_at(&mut self, p: usize, n: N, r: &[Type], result: &str) -> Option<NodeId> {
        let has = |t: Type| r.contains(&t);
        let lower = result.to_ascii_lowercase();
        if lower.starts_with("call_") {
            return if has(Type::InlineBabelCall) { self.inline_babel_call_parser(p, n) } else { None };
        }
        if lower.starts_with("src_") {
            return if has(Type::InlineSrcBlock) { self.inline_src_block_parser(p, n) } else { None };
        }
        let second = result.chars().nth(1);
        match result.chars().next()? {
            '^' => {
                if has(Type::Superscript) {
                    self.script_parser(p, n, Type::Superscript)
                } else {
                    None
                }
            }
            '_' => {
                let u = if has(Type::Underline) { self.emphasis(p, n, '_', Type::Underline) } else { None };
                u.or_else(|| if has(Type::Subscript) { self.script_parser(p, n, Type::Subscript) } else { None })
            }
            '*' => has(Type::Bold).then(|| self.emphasis(p, n, '*', Type::Bold)).flatten(),
            '/' => has(Type::Italic).then(|| self.emphasis(p, n, '/', Type::Italic)).flatten(),
            '~' => has(Type::Code).then(|| self.emphasis(p, n, '~', Type::Code)).flatten(),
            '=' => has(Type::Verbatim).then(|| self.emphasis(p, n, '=', Type::Verbatim)).flatten(),
            '+' => has(Type::StrikeThrough).then(|| self.emphasis(p, n, '+', Type::StrikeThrough)).flatten(),
            '@' => has(Type::ExportSnippet).then(|| self.export_snippet_parser(p, n)).flatten(),
            '{' => has(Type::Macro).then(|| self.macro_parser(p, n)).flatten(),
            '$' => has(Type::LatexFragment).then(|| self.latex_fragment_parser(p, n)).flatten(),
            '<' => {
                if second == Some('<') {
                    let rt = if has(Type::RadioTarget) { self.radio_target_parser(p, n) } else { None };
                    rt.or_else(|| if has(Type::Target) { self.target_parser(p, n) } else { None })
                } else {
                    let ts = if has(Type::Timestamp) { timestamp_parser(self, p, n.ne) } else { None };
                    ts.or_else(|| if has(Type::Link) { self.link_parser(p, n) } else { None })
                }
            }
            '\\' => {
                if second == Some('\\') {
                    has(Type::LineBreak).then(|| self.line_break_parser(p, n)).flatten()
                } else {
                    let e = if has(Type::Entity) { self.entity_parser(p, n) } else { None };
                    e.or_else(|| if has(Type::LatexFragment) { self.latex_fragment_parser(p, n) } else { None })
                }
            }
            '[' => match second {
                Some('[') if has(Type::Link) => self.link_parser(p, n),
                Some('f') if has(Type::FootnoteReference) => self.footnote_reference_parser(p, n),
                Some('c') if has(Type::Citation) => self.citation_parser(p, n),
                Some('%' | '/') if has(Type::StatisticsCookie) => self.statistics_cookie_parser(p, n),
                _ => {
                    let ts = if has(Type::Timestamp) { timestamp_parser(self, p, n.ne) } else { None };
                    ts.or_else(|| {
                        if has(Type::StatisticsCookie) { self.statistics_cookie_parser(p, n) } else { None }
                    })
                }
            },
            _ => has(Type::Link).then(|| self.link_parser(p, n)).flatten(),
        }
    }

    fn obj(&mut self, ty: Type, props: Vec<(&str, Value)>) -> NodeId {
        self.t.create(ty, props, vec![])
    }

    /// org-element--parse-generic-emphasis.
    fn emphasis(&mut self, origin: usize, n: N, mark: char, ty: Type) -> Option<NodeId> {
        if !self.n_bolp(origin, n) {
            let c = self.char_before(origin, n)?;
            if !(is_space(c) || "-('\"{".contains(c)) {
                return None;
            }
        }
        let after = self.char_at(origin + 1, n)?;
        if is_space(after) {
            return None;
        }
        let closing = CLOSING[match mark {
            '*' => 0,
            '/' => 1,
            '_' => 2,
            '+' => 3,
            '=' => 4,
            _ => 5,
        }]
        .captures_at(&self.s[n.nb..n.ne], origin + 1 - n.nb)?;
        let closing = closing.get(1).unwrap().end() + n.nb;
        let end = self.skip_blank(closing, n);
        let (cb, ce) = (origin + 1, closing - 1);
        let mut props = vec![
            ("begin", P::pos(origin)),
            ("end", P::pos(end)),
            ("post-blank", P::pos(end - closing)),
        ];
        if matches!(ty, Type::Code | Type::Verbatim) {
            props.push(("value", Value::Str(self.s[cb..ce].to_owned())));
        } else {
            props.push(("contents-begin", P::pos(cb)));
            props.push(("contents-end", P::pos(ce)));
        }
        Some(self.obj(ty, props))
    }

    fn citation_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&CITE_PREFIX, p, n)?;
        let style = c.get(1).map(|m| m.as_str().to_owned());
        let start = p + c.get(0).unwrap().end();
        let (_, closing) = paired(self.s, p, b'[', n.ne)?;
        let key = CITE_KEY.find_at(&self.s[..closing], start)?;
        let first_key_end = key.end();
        let end = self.skip_blank(closing, n);
        let id = self.obj(
            Type::Citation,
            vec![
                ("style", Value::opt_str(style)),
                ("begin", P::pos(p)),
                ("post-blank", P::pos(end - closing)),
                ("end", P::pos(end)),
            ],
        );
        let restr = super::restriction(Type::CitationReference);
        match self.s[start..first_key_end].rfind(';').map(|i| start + i) {
            None => self.t.put(id, "contents-begin", start),
            Some(semi) => {
                if start < semi {
                    let pre = self.parse_objects(start, semi, restr, Some(id));
                    self.t.put_secondary(id, "prefix", pre);
                }
                self.t.put(id, "contents-begin", semi + 1);
            }
        }
        let e = self.skip_back(closing - 1, b" \r\t\n", 0);
        let semi = self.s[first_key_end..e.max(first_key_end)].rfind(';').map(|i| first_key_end + i);
        match semi {
            Some(s) if CITE_KEY.find(&self.s[s..e]).is_none() => {
                let a = s + 1;
                if a < e {
                    let suf = self.parse_objects(a, e, restr, Some(id));
                    self.t.put_secondary(id, "suffix", suf);
                }
                self.t.put(id, "contents-end", a);
            }
            _ => self.t.put(id, "contents-end", e),
        }
        Some(id)
    }

    fn citation_reference_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let key = CITE_KEY.captures_at(&self.s[..n.ne], p)?;
        let k = key[1].to_owned();
        let (ks, ke) = (key.get(0).unwrap().start(), key.get(0).unwrap().end());
        let sep = self.s[ke..n.ne].find(';').map(|i| ke + i + 1);
        let end = sep.unwrap_or(n.ne);
        let suffix_end = if sep.is_some() { end - 1 } else { end };
        let id = self.obj(
            Type::CitationReference,
            vec![("key", Value::Str(k)), ("begin", P::pos(p)), ("end", P::pos(end)), ("post-blank", P::pos(0))],
        );
        let restr = super::restriction(Type::CitationReference);
        if p < ks {
            let pre = self.parse_objects(p, ks, restr, Some(id));
            self.t.put_secondary(id, "prefix", pre);
        }
        if ke < suffix_end {
            let suf = self.parse_objects(ke, suffix_end, restr, Some(id));
            self.t.put_secondary(id, "suffix", suf);
        }
        Some(id)
    }

    fn entity_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&ENTITY, p, n)?;
        let g1 = c.get(1).or(c.get(2)).unwrap();
        let name = g1.as_str();
        let e = super::entity(name)?;
        let brackets = c.get(3).is_some_and(|m| m.as_str() == "{}");
        let mut q = p + g1.end();
        if brackets {
            q += 2;
        }
        let end = self.skip_blank(q, n);
        Some(self.obj(
            Type::Entity,
            vec![
                ("name", Value::from(e.0)),
                ("latex", Value::from(e.1)),
                ("latex-math-p", Value::from(e.2)),
                ("html", Value::from(e.3)),
                ("ascii", Value::from(e.4)),
                ("latin1", Value::from(e.5)),
                ("utf-8", Value::from(e.6)),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("use-brackets-p", Value::from(brackets)),
                ("post-blank", P::pos(end - q)),
            ],
        ))
    }

    fn export_snippet_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&EXPORT_SNIPPET, p, n)?;
        let cb = p + c.get(0).unwrap().end();
        let ce = cb + self.s[cb..n.ne].find("@@")?;
        let after = ce + 2;
        let end = self.skip_blank(after, n);
        Some(self.obj(
            Type::ExportSnippet,
            vec![
                ("back-end", Value::Str(c[1].to_owned())),
                ("value", Value::Str(self.s[cb..ce].to_owned())),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("post-blank", P::pos(end - after)),
            ],
        ))
    }

    fn footnote_reference_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&FOOTNOTE_REF, p, n)?;
        let (_, closing) = paired(self.s, p, b'[', n.ne)?;
        let label = c.get(1).or(c.get(3)).map(|m| m.as_str().to_owned());
        let inline = c.get(2).is_some();
        let inner_begin = p + c.get(0).unwrap().end();
        let end = self.skip_blank(closing, n);
        Some(self.obj(
            Type::FootnoteReference,
            vec![
                ("label", Value::opt_str(label)),
                ("type", Value::sym(if inline { "inline" } else { "standard" })),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("contents-begin", if inline { P::pos(inner_begin) } else { Value::Nil }),
                ("contents-end", if inline { P::pos(closing - 1) } else { Value::Nil }),
                ("post-blank", P::pos(end - closing)),
            ],
        ))
    }

    fn word_start(&self, p: usize, n: N) -> bool {
        self.char_before(p, n).is_none_or(|c| !is_word(c))
    }

    fn inline_babel_call_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        if !self.word_start(p, n) {
            return None;
        }
        let c = self.nlook(&INLINE_CALL, p, n)?;
        let call = c[1].to_owned();
        let mut q = p + c.get(1).unwrap().end();
        let clean = |s: &str| nw(Some(s)).map(|s| NEWLINE_INDENT.replace_all(trim(s), " ").into_owned());
        let inside = match paired(self.s, q, b'[', n.ne) {
            Some((v, e)) => {
                q = e;
                clean(v)
            }
            None => None,
        };
        let (args, e) = paired(self.s, q, b'(', n.ne)?;
        let args = nw(Some(args)).map(str::to_owned);
        q = e;
        let end_header = match paired(self.s, q, b'[', n.ne) {
            Some((v, e)) => {
                q = e;
                clean(v)
            }
            None => None,
        };
        let value = self.s[p..q].to_owned();
        let end = self.skip_blank(q, n);
        Some(self.obj(
            Type::InlineBabelCall,
            vec![
                ("call", Value::Str(call)),
                ("inside-header", Value::opt_str(inside)),
                ("arguments", Value::opt_str(args)),
                ("end-header", Value::opt_str(end_header)),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("value", Value::Str(value)),
                ("post-blank", P::pos(end - q)),
            ],
        ))
    }

    fn inline_src_block_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        if !self.word_start(p, n) {
            return None;
        }
        let c = self.nlook(&INLINE_SRC, p, n)?;
        let lang = c[1].to_owned();
        let mut q = p + c.get(1).unwrap().end();
        let params = match paired(self.s, q, b'[', n.ne) {
            Some((v, e)) => {
                q = e;
                nw(Some(v)).map(|s| NEWLINE_INDENT.replace_all(trim(s), " ").into_owned())
            }
            None => None,
        };
        let (value, e) = paired(self.s, q, b'{', n.ne)?;
        let value = value.to_owned();
        let end = self.skip_blank(e, n);
        Some(self.obj(
            Type::InlineSrcBlock,
            vec![
                ("language", Value::Str(lang)),
                ("value", Value::Str(value)),
                ("parameters", Value::opt_str(params)),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("post-blank", P::pos(end - e)),
            ],
        ))
    }

    fn latex_fragment_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let s = &self.s[..n.ne];
        let b = s.as_bytes();
        let after = if b[p] != b'$' {
            match b.get(p + 1) {
                Some(b'(') => s[p..].find("\\)").map(|i| p + i + 2),
                Some(b'[') => s[p..].find("\\]").map(|i| p + i + 2),
                _ => self.nlook(&LATEX_MACRO, p, n).map(|c| p + c.get(0).unwrap().end()),
            }
        } else if b.get(p + 1) == Some(&b'$') {
            s[p + 2..].find("$$").map(|i| p + 2 + i + 2)
        } else {
            let prev_ok = self.char_before(p, n) != Some('$');
            let next_ok = !matches!(b.get(p + 1), Some(b' ' | b'\t' | b'\n' | b',' | b'.' | b';') | None);
            if prev_ok && next_ok {
                s[p + 1..].find('$').and_then(|i| {
                    let close = p + 1 + i;
                    let before = s[..close].chars().next_back();
                    let ok_before = !matches!(before, Some(' ' | '\t' | '\n' | ',' | '.'));
                    let next = s[close + 1..].chars().next();
                    (ok_before && latex_after_ok(next)).then_some(close + 1)
                })
            } else {
                None
            }
        }?;
        let end = self.skip_blank(after, n);
        Some(self.obj(
            Type::LatexFragment,
            vec![
                ("value", Value::Str(self.s[p..after].to_owned())),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("post-blank", P::pos(end - after)),
            ],
        ))
    }

    fn line_break_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        self.nlook(&LINE_BREAK, p, n)?;
        if self.char_before(p, n) == Some('\\') {
            return None;
        }
        let end = self.next_line(p).min(n.ne);
        Some(self.obj(Type::LineBreak, vec![("begin", P::pos(p)), ("end", P::pos(end)), ("post-blank", P::pos(0))]))
    }

    pub(super) fn link_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let mut contents = None;
        let mut raw_link = None;
        let mut explicit = false;
        let (format, mut ty, mut path, link_end);
        let radio = self.cx.radio.clone().and_then(|re| {
            let from = if self.n_bolp(p, n) { p } else { self.prev_char_pos(p) };
            let c = re.captures_at(&self.s[n.nb..n.ne], from - n.nb)?;
            (c.get(0).unwrap().start() + n.nb == from).then(|| {
                let g = c.get(1).unwrap();
                (n.nb + g.start(), n.nb + g.end())
            })
        });
        if let Some((a, b)) = radio {
            ty = "radio".to_owned();
            format = "plain";
            link_end = b;
            path = self.s[a..b].to_owned();
            contents = Some((a, b));
        } else if let Some(c) = self.nlook(&LINK_BRACKET, p, n) {
            format = "bracket";
            contents = c.get(2).map(|m| (p + m.start(), p + m.end()));
            link_end = p + c.get(0).unwrap().end();
            let raw = expand_abbrev(&unescape(&NEWLINE_INDENT.replace_all(&c[1], " ")), &self.cx.link_abbrevs);
            if raw.starts_with('/') || raw.starts_with('~') || raw.starts_with("./") || raw.starts_with("../") {
                ty = "file".to_owned();
                path = raw.clone();
            } else if let Some(m) = self.cx.link_types_re.captures(&raw) {
                ty = m[1].to_owned();
                explicit = true;
                path = raw[m.get(0).unwrap().end()..].to_owned();
            } else if raw.starts_with('(') && raw.ends_with(')') && raw.len() >= 2 {
                ty = "coderef".to_owned();
                path = raw[1..raw.len() - 1].to_owned();
            } else if let Some(r) = raw.strip_prefix('#') {
                ty = "custom-id".to_owned();
                path = r.to_owned();
            } else {
                ty = "fuzzy".to_owned();
                path = raw.clone();
            }
            raw_link = Some(raw);
        } else if self.word_start(p, n)
            && let Some(c) = self.nlook(&self.cx.plain_link, p, n)
        {
            format = "plain";
            raw_link = Some(c[0].to_owned());
            ty = c[1].to_owned();
            explicit = true;
            link_end = p + c.get(0).unwrap().end();
            path = c[2].to_owned();
        } else if let Some(c) = self.nlook(&self.cx.angle_link, p, n) {
            format = "angle";
            ty = c[1].to_owned();
            explicit = true;
            link_end = p + c.get(0).unwrap().end();
            raw_link = Some(self.s[p + c.get(1).unwrap().start()..p + c.get(2).unwrap().end()].to_owned());
            path = NEWLINE_INDENT.replace_all(&c[2], "").into_owned();
        } else {
            return None;
        }
        let end = self.skip_blank(link_end, n);
        let mut application = None;
        let mut search = None;
        if let Some(c) = FILE_TYPE.captures(&ty) {
            application = c.get(1).map(|m| m.as_str().to_owned());
            ty = "file".to_owned();
            if let Some(m) = SEARCH_OPTION.captures(&path) {
                search = Some(m[1].to_owned());
                path.truncate(m.get(0).unwrap().start());
            }
            path = FILE_SLASHES.replace(&path, "$1/").into_owned();
        }
        Some(self.obj(
            Type::Link,
            vec![
                ("type", Value::Str(ty)),
                ("type-explicit-p", Value::from(explicit)),
                ("path", Value::Str(path.clone())),
                ("format", Value::sym(format)),
                ("raw-link", Value::Str(raw_link.unwrap_or(path))),
                ("application", Value::opt_str(application)),
                ("search-option", Value::opt_str(search)),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("contents-begin", Value::opt_int(contents.map(|c| c.0))),
                ("contents-end", Value::opt_int(contents.map(|c| c.1))),
                ("post-blank", P::pos(end - link_end)),
            ],
        ))
    }

    fn macro_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&MACRO, p, n)?;
        let key = c[1].to_lowercase();
        let value = c[0].to_owned();
        let me = p + c.get(0).unwrap().end();
        let end = self.skip_blank(me, n);
        let args = c.get(3).map(|a| {
            let a = MACRO_WS.replace_all(trim(a.as_str()), " ").into_owned();
            Value::List(macro_extract_arguments(&a).into_iter().map(Value::Str).collect())
        });
        Some(self.obj(
            Type::Macro,
            vec![
                ("key", Value::Str(key)),
                ("value", Value::Str(value)),
                ("args", args.unwrap_or(Value::Nil)),
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("post-blank", P::pos(end - me)),
            ],
        ))
    }

    fn radio_target_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&RADIO_TARGET, p, n)?;
        let g = c.get(1).unwrap();
        let me = p + c.get(0).unwrap().end();
        let end = self.skip_blank(me, n);
        Some(self.obj(
            Type::RadioTarget,
            vec![
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("contents-begin", P::pos(p + g.start())),
                ("contents-end", P::pos(p + g.end())),
                ("post-blank", P::pos(end - me)),
                ("value", Value::Str(g.as_str().to_owned())),
            ],
        ))
    }

    fn statistics_cookie_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&STAT_COOKIE, p, n)?;
        let me = p + c.get(0).unwrap().end();
        let end = self.skip_blank(me, n);
        Some(self.obj(
            Type::StatisticsCookie,
            vec![
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("value", Value::Str(c[0].to_owned())),
                ("post-blank", P::pos(end - me)),
            ],
        ))
    }

    /// Subscript and superscript parsers.
    fn script_parser(&mut self, p: usize, n: N, ty: Type) -> Option<NodeId> {
        let from = if self.n_bolp(p, n) {
            if ty == Type::Subscript {
                return None;
            }
            p
        } else {
            self.prev_char_pos(p)
        };
        let c = self.nlook(&SUBSCRIPT, from, n)?;
        let brackets = c.get(4).is_some();
        let begin = from + c.get(2).unwrap().start();
        let inner = c.get(4).or(c.get(3)).unwrap();
        let me = from + c.get(0).unwrap().end();
        let end = self.skip_blank(me, n);
        Some(self.obj(
            ty,
            vec![
                ("begin", P::pos(begin)),
                ("end", P::pos(end)),
                ("use-brackets-p", Value::from(brackets)),
                ("contents-begin", P::pos(from + inner.start())),
                ("contents-end", P::pos(from + inner.end())),
                ("post-blank", P::pos(end - me)),
            ],
        ))
    }

    fn table_cell_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&TABLE_CELL, p, n)?;
        let g = c.get(1).unwrap();
        Some(self.obj(
            Type::TableCell,
            vec![
                ("begin", P::pos(p)),
                ("end", P::pos(p + c.get(0).unwrap().end())),
                ("contents-begin", P::pos(p + g.start())),
                ("contents-end", P::pos(p + g.end())),
                ("post-blank", P::pos(0)),
            ],
        ))
    }

    fn target_parser(&mut self, p: usize, n: N) -> Option<NodeId> {
        let c = self.nlook(&TARGET, p, n)?;
        let me = p + c.get(0).unwrap().end();
        let end = self.skip_blank(me, n);
        Some(self.obj(
            Type::Target,
            vec![
                ("begin", P::pos(p)),
                ("end", P::pos(end)),
                ("value", Value::Str(c[1].to_owned())),
                ("post-blank", P::pos(end - me)),
            ],
        ))
    }
}

static CLOSING: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    ["*", "/", "_", "+", "=", "~"]
        .iter()
        .map(|m| {
            Regex::new(&format!(r#"(?m)[^{SPACE}]({})(?:[{SPACE}\-.,;:!?'")}}\\\[]|$)"#, regex::escape(m))).unwrap()
        })
        .collect()
});

fn unit(c: &str) -> &'static str {
    match c {
        "h" => "hour",
        "d" => "day",
        "w" => "week",
        "m" => "month",
        _ => "year",
    }
}

/// org-parse-time-string with NODEFAULT: (year month day hour minute).
pub fn parse_time_string(s: &str) -> Option<[Option<i64>; 5]> {
    let c = TS0.captures(s)?;
    let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<i64>().ok());
    Some([num(1), num(2), num(3), num(6), num(7)])
}

/// org-element-timestamp-parser at P (bounded by LIMIT).
pub(super) fn timestamp_parser(pp: &mut P, p: usize, limit: usize) -> Option<NodeId> {
    let hay = &pp.s[p..limit];
    let ts = TIMESTAMP.captures(hay)?;
    let active = hay.starts_with('<');
    let raw = TIMESTAMP_RAW.captures(hay)?;
    let raw_value = raw[0].to_owned();
    let diary = raw.get(2).is_some();
    let mut diary_sexp = None;
    let date_start = if diary {
        let g = ts.get(2)?;
        diary_sexp = Some(hay[3..g.start()].to_owned());
        g.as_str().to_owned()
    } else {
        raw[1].to_owned()
    };
    let date_end = raw.get(3).map(|m| m.as_str().to_owned());
    let me = p + raw.get(0).unwrap().end();
    let end = pp.skip_fwd(me, b" \t", limit);
    let time_range = TIME_RANGE.captures(&date_start).map(|c| {
        (c[2].parse::<i64>().unwrap_or(0), c[3].parse::<i64>().unwrap_or(0))
    });
    let ranged = date_end.is_some() || time_range.is_some();
    let ty = if diary {
        "diary"
    } else if active && ranged {
        "active-range"
    } else if active {
        "active"
    } else if ranged {
        "inactive-range"
    } else {
        "inactive"
    };
    let range_type = if date_end.is_some() {
        Value::sym("daterange")
    } else if time_range.is_some() {
        Value::sym("timerange")
    } else {
        Value::Nil
    };
    let mut start: [Option<i64>; 5] = [None; 5];
    let mut stop: [Option<i64>; 5] = [None; 5];
    if !diary {
        start = parse_time_string(&date_start).unwrap_or([None; 5]);
        let e = date_end.as_deref().and_then(parse_time_string).unwrap_or([None; 5]);
        stop = [
            e[0].or(start[0]),
            e[1].or(start[1]),
            e[2].or(start[2]),
            e[3].or(time_range.map(|t| t.0)).or(start[3]),
            e[4].or(time_range.map(|t| t.1)).or(start[4]),
        ];
    } else if let Some(c) = DIARY_TIME.captures(&date_start) {
        let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<i64>().ok());
        start[3] = num(1);
        start[4] = num(2);
        stop[3] = num(4);
        stop[4] = num(5);
    }
    let int = |v: Option<i64>| v.map_or(Value::Nil, Value::Int);
    let mut props = vec![
        ("type", Value::sym(ty)),
        ("range-type", range_type),
        ("raw-value", Value::Str(raw_value.clone())),
        ("year-start", int(start[0])),
        ("month-start", int(start[1])),
        ("day-start", int(start[2])),
        ("hour-start", int(start[3])),
        ("minute-start", int(start[4])),
        ("year-end", int(stop[0])),
        ("month-end", int(stop[1])),
        ("day-end", int(stop[2])),
        ("hour-end", int(stop[3])),
        ("minute-end", int(stop[4])),
        ("begin", P::pos(p)),
        ("end", P::pos(end)),
        ("post-blank", P::pos(end - me)),
        ("diary-sexp", Value::opt_str(diary_sexp)),
    ];
    if !diary {
        if let Some(c) = REPEATER.captures(&raw_value) {
            let rt = match &c[1] {
                "++" => "catch-up",
                ".+" => "restart",
                _ => "cumulate",
            };
            props.push(("repeater-type", Value::sym(rt)));
            props.push(("repeater-value", Value::Int(c[2].parse().unwrap_or(0))));
            props.push(("repeater-unit", Value::sym(unit(&c[3]))));
            if let (Some(v), Some(u)) = (c.get(4), c.get(5)) {
                props.push(("repeater-deadline-value", Value::Int(v.as_str().parse().unwrap_or(0))));
                props.push(("repeater-deadline-unit", Value::sym(unit(u.as_str()))));
            }
        }
        if let Some(c) = WARNING.captures(&raw_value) {
            props.push(("warning-type", Value::sym(if c.get(1).is_some() { "first" } else { "all" })));
            props.push(("warning-value", Value::Int(c[2].parse().unwrap_or(0))));
            props.push(("warning-unit", Value::sym(unit(&c[3]))));
        }
    }
    Some(pp.t.create(Type::Timestamp, props, vec![]))
}

/// org-link-unescape.
pub fn unescape(link: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = link.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\\' {
            let start = i;
            while i < b.len() && b[i] == '\\' {
                i += 1;
            }
            let n = i - start;
            if i == b.len() || b[i] == '[' || b[i] == ']' {
                out.extend(std::iter::repeat_n('\\', n / 2));
            } else {
                out.extend(std::iter::repeat_n('\\', n));
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// org-link-escape.
pub fn escape(link: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = link.chars().collect();
    let mut i = 0;
    loop {
        let start = i;
        while i < b.len() && b[i] == '\\' {
            i += 1;
        }
        let n = i - start;
        let at_end = i == b.len();
        let bracket = !at_end && (b[i] == '[' || b[i] == ']');
        if at_end || bracket {
            out.extend(std::iter::repeat_n('\\', 2 * n));
            if bracket {
                out.push('\\');
                out.push(b[i]);
                i += 1;
            }
        } else {
            out.extend(std::iter::repeat_n('\\', n));
            out.push(b[i]);
            i += 1;
        }
        if at_end {
            break;
        }
    }
    out
}

/// org-link-expand-abbrev (string replacements; function abbrevs are
/// not supported).
pub fn expand_abbrev(link: &str, abbrevs: &[(String, String)]) -> String {
    let Some(c) = ABBREV.captures(link) else { return link.to_owned() };
    let key = &c[1];
    let Some((_, rpl)) = abbrevs.iter().find(|(k, _)| k == key) else {
        return link.to_owned();
    };
    let tag = c.get(2).map(|_| c.get(3).map_or("", |m| m.as_str()));
    if let Some(i) = rpl.find("%s") {
        format!("{}{}{}", &rpl[..i], tag.unwrap_or(""), &rpl[i + 2..])
    } else if let Some(i) = rpl.find("%h") {
        format!("{}{}{}", &rpl[..i], url_hexify(tag.unwrap_or("")), &rpl[i + 2..])
    } else {
        format!("{rpl}{}", tag.unwrap_or(""))
    }
}

/// url-hexify-string.
fn url_hexify(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// org-macro-extract-arguments.
pub fn macro_extract_arguments(s: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\\' {
            let start = i;
            while i < b.len() && b[i] == '\\' {
                i += 1;
            }
            let n = i - start;
            if i < b.len() && b[i] == ',' {
                out.last_mut().unwrap().extend(std::iter::repeat_n('\\', n / 2));
                if n % 2 == 0 {
                    out.push(String::new());
                } else {
                    out.last_mut().unwrap().push(',');
                }
                i += 1;
            } else {
                out.last_mut().unwrap().extend(std::iter::repeat_n('\\', n));
            }
        } else if b[i] == ',' {
            out.push(String::new());
            i += 1;
        } else {
            out.last_mut().unwrap().push(b[i]);
            i += 1;
        }
    }
    out
}

/// org-macro-escape-arguments.
pub fn macro_escape_arguments(args: &[String]) -> String {
    let mut s = String::new();
    for arg in args.iter().rev() {
        let mut a = arg.clone();
        if !s.is_empty() {
            let trailing = a.len() - a.trim_end_matches('\\').len();
            a.push_str(&"\\".repeat(trailing));
        }
        let mut esc = String::new();
        let chars: Vec<char> = a.chars().collect();
        let mut j = 0;
        while j < chars.len() {
            if chars[j] == '\\' || chars[j] == ',' {
                let start = j;
                while j < chars.len() && chars[j] == '\\' {
                    j += 1;
                }
                let n = j - start;
                if j < chars.len() && chars[j] == ',' {
                    esc.push_str(&"\\".repeat(2 * n + 1));
                    esc.push(',');
                    j += 1;
                } else {
                    esc.push_str(&"\\".repeat(n));
                }
            } else {
                esc.push(chars[j]);
                j += 1;
            }
        }
        s = format!(",{esc}{s}");
    }
    s.strip_prefix(',').unwrap_or(&s).to_owned()
}
