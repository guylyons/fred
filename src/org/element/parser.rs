//! Element parsers of org-element.el (org-element-TYPE-parser,
//! org-element--current-element, org-element--parse-elements).
//!
//! Positions are byte offsets into the parsed text.  Emacs motions
//! (forward-line, skip-chars-*, count-lines, looking-at) are emulated by
//! the helpers at the top of this file.

use super::ast::{NodeId, Tree, Type, Value};
use super::{Granularity, Opts};
use crate::org::syntax::Settings;
use regex::{Captures, Regex};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        pub(super) static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

pub(super) const TS_BOTH: &str = r"[\[<]([0-9]{4}-[0-9]{2}-[0-9]{2}(?: .*?)?)[\]>]";
const TS_INACTIVE_NG: &str = r"\[(?:[0-9]{4}-[0-9]{2}-[0-9]{2}(?: .*?)?)\]";

re!(
    CURRENT,
    r"(?im)\A(?:([ \t]*\\begin\{[A-Za-z0-9*]+\})|([ \t]*:[-_\w]+:[ \t]*$)|([ \t]*:(?: |$))|([ \t]*#\+BEGIN:[ \t]*\S)|([ \t]*#\+)(?:BEGIN_(\S+)|(CALL:)|(\S+:))|(\[fn:[-_\w]+\])|([ \t]*-----+[ \t]*$)|(%%\())"
);
re!(
    AFFILIATED,
    r"(?i)\A[ \t]*#\+(?:(CAPTION|RESULTS)(?:\[(.*)\])?|(DATA|HEADERS?|LABEL|NAME|PLOT|RES(?:NAME|ULT)|(?:S(?:OURC|RCNAM)|TBLNAM)E)|(ATTR_[-_A-Za-z0-9]+)):[ \t]*"
);
re!(
    PROPERTY_DRAWER,
    r"(?im)\A[ \t]*:PROPERTIES:[ \t]*\n(?:[ \t]*:\S+:(?:[ \t].*)?[ \t]*\n)*?[ \t]*:END:[ \t]*$"
);
re!(
    PROPERTY,
    r"(?m)\A([ \t]*)(:(\S+):)(?:($)|[ \t]+(.*?))([ \t]*)$"
);
re!(DRAWER, r"(?m)\A[ \t]*:([-_\w]+):[ \t]*$");
re!(DYN_OPEN, r"(?i)\A[ \t]*#\+BEGIN:[ \t]*(\S+)(?:[ \t]+(.+))?");
re!(FOOTNOTE_DEF, r"\A\[fn:([-_\w]+)\]");
re!(LATEX_BEGIN, r"(?i)\A[ \t]*\\begin\{([A-Za-z0-9*]+)\}");
re!(KEYWORD, r"\A[ \t]*#\+(\S*):");
re!(SPECIAL, r"(?im)\A[ \t]*#\+BEGIN_(\S+)[ \t]*(.*)[ \t]*$");
re!(EXAMPLE, r"(?im)\A[ \t]*#\+BEGIN_EXAMPLE(?: +(.*))?");
re!(
    EXPORT,
    r"(?im)\A[ \t]*#\+BEGIN_EXPORT(?:[ \t]+(\S+))?[ \t]*$"
);
re!(
    SRC,
    r#"(?im)\A[ \t]*#\+BEGIN_SRC(?: +(\S+))?((?: +(?:-(?:l ".+"|[ikr])|[-+]n(?: *[0-9]+)?))+)?(.*)[ \t]*$"#
);
re!(NUMBER_LINES, r"([-+])n(?: *([0-9]+))?\b");
re!(LABEL_FMT, r#"-l +"([^"\n]+)""#);
re!(SW_I, r"-i\b");
re!(SW_R, r"-r\b");
re!(SW_K, r"-k\b");
re!(
    CLOCK_LINE,
    &format!(
        r"(?im)\A[\t ]*CLOCK:(?:[\t ]+(?:{TS_INACTIVE_NG})(?:--(?:{TS_INACTIVE_NG})[\t ]+=>[\t ]+[0-9]+:[0-9][0-9])?|[\t ]+=>[\t ]+[0-9]+:[0-9][0-9])[\t ]*$"
    )
);
re!(
    FOOTNOTE_SEPARATOR,
    r"(?m)^\*+ |^\[fn:[-_\w]+\]|^(?:[ \t]*\n){2,}"
);
re!(
    FULL_ITEM,
    r"(?m)\A[ \t]*((?:[-+*]|(?:[0-9]+|[A-Za-z])[.)])(?:[ \t]+|$))(?:\[@(?:start:)?([0-9]+|[A-Za-z])\][ \t]*)?(?:(\[[ X-]\])(?:[ \t]+|$))?(?:(.*)[ \t]+::(?:[ \t]+|$))?"
);
re!(TAG_GROUP, r"(?m)[ \t]+(:([\p{L}\p{N}_@#%:]+):)[ \t]*$");
re!(
    PRIORITY,
    r"(?i)\A.*?(\[#([A-Z]|[0-9]|[1-5][0-9]|6[0-4])\] ?)"
);
re!(TABLE_EL_RULE, r"(?m)\A[ \t]*\+(?:-+\+)+[ \t]*$");

/// A compiled regex, once per pattern.
// ponytail: unbounded cache, keyed by pattern (radio target sets per
// document add entries); add eviction if documents churn targets.
fn cached(pattern: &str) -> Regex {
    static CACHE: LazyLock<std::sync::Mutex<HashMap<String, Regex>>> =
        LazyLock::new(Default::default);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.entry(pattern.to_owned())
        .or_insert_with(|| Regex::new(pattern).unwrap())
        .clone()
}

/// Parse environment: what upstream reads from buffer-local variables.
pub struct Ctx {
    pub todo: Vec<String>,
    pub done: Vec<String>,
    pub odd_levels: bool,
    /// org-inlinetask-min-level (0: inline tasks off).
    pub inlinetask_min: usize,
    pub footnote_section: Option<String>,
    pub link_abbrevs: Vec<(String, String)>,
    pub link_types: Vec<String>,
    pub category: Option<String>,
    pub(super) item_re: Regex,
    pub(super) paragraph_separate: Regex,
    pub(super) object_re: Regex,
    pub(super) plain_link: Regex,
    pub(super) angle_link: Regex,
    pub(super) link_types_re: Regex,
    /// org-target-link-regexp (radio links), from the document's targets.
    pub(super) radio: Option<Regex>,
}

/// org-link-parameters link types after org-mode loads org-modules.
pub const DEFAULT_LINK_TYPES: &[&str] = &[
    "id",
    "eww",
    "rmail",
    "mhe",
    "irc",
    "info",
    "gnus",
    "docview",
    "bibtex",
    "bbdb",
    "w3m",
    "doi",
    "file+sys",
    "file+emacs",
    "shell",
    "news",
    "mailto",
    "https",
    "http",
    "ftp",
    "shortdoc",
    "help",
    "file",
    "elisp",
    "attachment",
];

fn regexp_opt(words: &[String]) -> String {
    let mut w: Vec<&String> = words.iter().collect();
    // Longest first, as regexp-opt matches the longest alternative.
    w.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    w.iter()
        .map(|s| regex::escape(s))
        .collect::<Vec<_>>()
        .join("|")
}

impl Ctx {
    pub fn new(st: &Settings) -> Ctx {
        let opt = crate::org::options::bool;
        let alpha = opt("org-list-allow-alphabetical", false);
        let term = match crate::org::sexp::option("org-plain-list-ordered-item-terminator") {
            Some(v) if v.int() == Some('.' as i64) => r"\.",
            Some(v) if v.int() == Some(')' as i64) => r"\)",
            _ => "[.)]",
        };
        let alpha_re = if alpha { "|[A-Za-z]" } else { "" };
        let item_re = cached(&format!(
            r"(?m)\A(?:[ \t]*(?:[-+]|(?:[0-9]+{alpha_re}){term})|[ \t]+\*)(?:[ \t]+|$)"
        ));
        let clock = format!(
            r"^[\t ]*CLOCK:(?:[\t ]+(?:{TS_INACTIVE_NG})(?:--(?:{TS_INACTIVE_NG})[\t ]+=>[\t ]+[0-9]+:[0-9][0-9])?|[\t ]+=>[\t ]+[0-9]+:[0-9][0-9])[\t ]*$"
        );
        let paragraph_separate = cached(&format!(
            r"(?im)^(?:\*+ |\[fn:[-_\w]+\]|%%\(|[ \t]*(?:$|\||\+(?:-+\+)+[ \t]*$|#(?: |$|\+(?:BEGIN_\S+|\S+(?:\[.*\])?:[ \t]*))|:(?: |$|[-_\w]+:[ \t]*$)|-{{5,}}[ \t]*$|\\begin\{{[A-Za-z0-9*]+\}}|{clock}|(?:[-+*]|(?:[0-9]+{alpha_re}){term})(?:[ \t]|$)))"
        ));
        let link_types: Vec<String> = DEFAULT_LINK_TYPES.iter().map(|s| s.to_string()).collect();
        let types = regexp_opt(&link_types);
        let space = super::objects::SPACE;
        let object_re = cached(&format!(
            r"(?im)[_^][-{{(*+.,\p{{L}}\p{{N}}]|[*~=+_/][^{space}]|\b(?:{types}):|\[(?:cite[:/]|fn:|(?:[0-9]|(?:%|/[0-9]*)\])|\[)|@@|\{{\{{\{{|<(?:%%|<|[0-9]|(?:{types}))|\$|\\(?:[a-zA-Z\[(]|\\[ \t]*$|_ +)|(?:call|src)_"
        ));
        let nsb = r"[^\]\[ \t\n()<>]";
        let paren = format!(r"[<(\[](?:{nsb}|[<(\[]{nsb}*[\])>])*[\])>]");
        let plain_link = cached(&format!(
            r"(?i)\A({types}):((?:{nsb}|{paren})+(?:[\p{{L}}\p{{N}}\p{{M}}]|-|/|{paren}))"
        ));
        let angle_link = cached(&format!(
            r"(?i)\A<({types}):([^>\n]*(?:\n[ \t]*[^> \t\n][^>\n]*)*)>"
        ));
        let link_types_re = cached(&format!(r"(?i)\A({types}):"));
        let mut link_abbrevs = st.links.clone();
        if let Some(v) = crate::org::sexp::option("org-link-abbrev-alist")
            && let Some(l) = v.list()
        {
            for e in l {
                if let (Some(k), Some(r)) = (
                    e.car().and_then(|x| x.str()),
                    e.cdr().str().map(str::to_owned),
                ) {
                    link_abbrevs.push((k.to_owned(), r));
                }
            }
        }
        let inlinetask_min =
            crate::org::options::int("org-inlinetask-min-level", 15).max(0) as usize;
        let fs = crate::org::options::string("org-footnote-section", "Footnotes");
        Ctx {
            todo: st.todo_names().into_iter().map(str::to_owned).collect(),
            done: st.done_names().into_iter().map(str::to_owned).collect(),
            odd_levels: st.opt_bool("org-odd-levels-only", false),
            inlinetask_min,
            footnote_section: (!fs.is_empty() && fs != "nil").then_some(fs),
            link_abbrevs,
            link_types,
            category: None,
            item_re,
            paragraph_separate,
            object_re,
            plain_link,
            angle_link,
            link_types_re,
            radio: None,
        }
    }

    /// Set the radio targets (org-update-radio-target-regexp).
    pub fn set_radio_targets(&mut self, mut targets: Vec<String>) {
        targets.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
        targets.dedup();
        self.radio = (!targets.is_empty()).then(|| {
            let alts: Vec<String> = targets
                .iter()
                .map(|t| {
                    let parts: Vec<String> = t
                        .split(' ')
                        .filter(|p| !p.is_empty())
                        .map(regex::escape)
                        .collect();
                    parts.join(r"\s+")
                })
                .collect();
            cached(&format!(
                r"(?im)(?:^|[^\p{{L}}\p{{N}}])({})(?:$|[^\p{{L}}\p{{N}}])",
                alts.join("|")
            ))
        });
    }

    /// Stars of the highest level that is still a headline (inline tasks
    /// start at org-inlinetask-min-level).
    pub fn max_headline_stars(&self) -> usize {
        if self.inlinetask_min == 0 {
            return usize::MAX;
        }
        let limit = self.inlinetask_min.saturating_sub(1);
        if self.odd_levels {
            (limit * 2).saturating_sub(1)
        } else {
            limit
        }
    }

    fn inlinetask_stars(&self) -> usize {
        if self.inlinetask_min == 0 {
            usize::MAX
        } else if self.odd_levels {
            (self.inlinetask_min * 2).saturating_sub(1)
        } else {
            self.inlinetask_min
        }
    }

    /// org-reduced-level.
    pub fn reduced_level(&self, l: usize) -> usize {
        if self.odd_levels { 1 + l / 2 } else { l }
    }
}

/// A plain-list structure item (org-list-struct): position,
/// indentation, bullet, counter, checkbox, tag, end.
#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub pos: usize,
    pub ind: usize,
    pub bullet: String,
    pub counter: Option<String>,
    pub checkbox: Option<String>,
    pub tag: Option<String>,
    pub end: usize,
}

pub type Struct = Rc<Vec<ListItem>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    None,
    FirstSection,
    Section,
    Planning,
    PropertyDrawer,
    NodeProperty,
    Item,
    TableRow,
    TopComment,
    OrgData,
}

/// org-element--next-mode.
pub(super) fn next_mode(mode: Mode, ty: Type, parent: bool) -> Mode {
    use Mode as M;
    if parent {
        match ty {
            Type::Headline => M::Section,
            Type::Section if mode == M::FirstSection => M::TopComment,
            Type::OrgData if mode == M::OrgData || mode == M::None => M::FirstSection,
            Type::Inlinetask => M::Planning,
            Type::PlainList => M::Item,
            Type::PropertyDrawer => M::NodeProperty,
            Type::Section => M::Planning,
            Type::Table => M::TableRow,
            _ => M::None,
        }
    } else {
        match mode {
            M::Item => M::Item,
            M::NodeProperty => M::NodeProperty,
            M::Planning if ty == Type::Planning => M::PropertyDrawer,
            M::TableRow => M::TableRow,
            M::TopComment if ty == Type::Comment => M::PropertyDrawer,
            _ => M::None,
        }
    }
}

pub(super) const WS: &[u8] = b" \r\t\n";
pub(super) const BLANK: &[u8] = b" \t";

/// Group span helper: (start, end) of group I.
pub(super) fn span(c: &Captures, i: usize, base: usize) -> Option<(usize, usize)> {
    c.get(i).map(|m| (m.start() + base, m.end() + base))
}

/// org-trim.
pub fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r'])
}

/// org-string-nw-p.
pub fn nw(s: Option<&str>) -> Option<&str> {
    s.filter(|s| s.bytes().any(|b| !b" \r\t\n".contains(&b)))
}

/// Parser state.
pub(super) struct P<'a> {
    pub s: &'a str,
    pub t: Tree,
    pub cx: &'a Ctx,
    pub structs: HashMap<NodeId, Struct>,
    pub invisible: Vec<std::ops::Range<usize>>,
}

impl<'a> P<'a> {
    pub fn new(s: &'a str, cx: &'a Ctx) -> P<'a> {
        P {
            s,
            t: Tree::new(),
            cx,
            structs: HashMap::new(),
            invisible: vec![],
        }
    }

    // ---- buffer motions ----

    pub fn len(&self) -> usize {
        self.s.len()
    }
    pub fn at_bol(&self, p: usize) -> bool {
        p == 0 || self.s.as_bytes()[p - 1] == b'\n'
    }
    pub fn bol(&self, p: usize) -> usize {
        self.s[..p].rfind('\n').map_or(0, |i| i + 1)
    }
    pub fn eol(&self, p: usize) -> usize {
        self.s[p..].find('\n').map_or(self.len(), |i| p + i)
    }
    /// forward-line (from anywhere: start of the next line, or eob).
    pub fn next_line(&self, p: usize) -> usize {
        self.s[p..].find('\n').map_or(self.len(), |i| p + i + 1)
    }
    /// line-beginning-position 0.
    pub fn prev_bol(&self, p: usize) -> usize {
        let b = self.bol(p);
        if b == 0 { 0 } else { self.bol(b - 1) }
    }
    pub fn skip_fwd(&self, mut p: usize, set: &[u8], limit: usize) -> usize {
        let b = self.s.as_bytes();
        while p < limit && set.contains(&b[p]) {
            p += 1;
        }
        p
    }
    pub fn skip_back(&self, mut p: usize, set: &[u8], limit: usize) -> usize {
        let b = self.s.as_bytes();
        while p > limit && set.contains(&b[p - 1]) {
            p -= 1;
        }
        p
    }
    /// count-lines.
    pub fn count_lines(&self, a: usize, b: usize) -> usize {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let n = self.s[a..b].bytes().filter(|&c| c == b'\n').count();
        if a != b && !self.at_bol(b) { n + 1 } else { n }
    }
    /// The end of an element: after blank lines, at a line start.
    pub fn end_after_blank(&self, p: usize, limit: usize) -> usize {
        let q = self.skip_fwd(p, WS, limit);
        if q == self.len() { q } else { self.bol(q) }
    }
    pub fn looking_at<'r>(&self, re: &'r Regex, p: usize) -> Option<Captures<'a>> {
        re.captures(&self.s[p..])
    }
    pub fn looking_at_p(&self, re: &Regex, p: usize) -> bool {
        re.is_match(&self.s[p..])
    }
    /// The first line at or after P (P's line only if P is at its
    /// start) for which `pred(line)` holds and that ends before LIMIT;
    /// its start.
    pub fn find_line(&self, p: usize, limit: usize, pred: impl Fn(&str) -> bool) -> Option<usize> {
        let mut l = if self.at_bol(p) { p } else { self.next_line(p) };
        while l < limit {
            let e = self.eol(l);
            if e > limit {
                // A partial last line: match against what is before LIMIT.
                return pred(&self.s[l..limit]).then_some(l);
            }
            if pred(&self.s[l..e]) {
                return Some(l);
            }
            if e >= self.len() {
                break;
            }
            l = e + 1;
        }
        None
    }
    /// The column after leading blanks (org-current-text-column).
    pub fn indentation(&self, p: usize) -> usize {
        let mut col = 0;
        for c in self.s[p..].chars() {
            match c {
                ' ' => col += 1,
                '\t' => col = (col / 8 + 1) * 8,
                _ => break,
            }
        }
        col
    }

    pub fn sub(&self, a: usize, b: usize) -> &'a str {
        &self.s[a.min(b)..a.max(b)]
    }

    /// A position property.
    pub fn pos(n: usize) -> Value {
        Value::Int(n as i64)
    }

    // ---- driver ----

    /// org-element--parse-elements.
    #[allow(clippy::too_many_arguments)]
    pub fn parse_elements(
        &mut self,
        beg: usize,
        end: usize,
        mut mode: Mode,
        structure: Option<Struct>,
        gran: Granularity,
        acc: NodeId,
    ) {
        let mut p = beg;
        if gran == Granularity::Headline && !self.at_headline(p) {
            p = self.next_headline(p).unwrap_or(self.len());
        }
        let mut elements = vec![];
        while p < end {
            if let Some(r) = self.invisible.iter().find(|r| r.contains(&p)) {
                p = r.end;
                if p < self.len() && self.s.as_bytes()[p] == b'\n' {
                    p += 1;
                }
                continue;
            }
            let el = self.current_element(p, end, gran, mode, structure.clone());
            let ty = self.t.kind(el);
            let new_p = self.t.end(el).unwrap_or(end);
            p = if new_p > p { new_p } else { self.next_line(p) };
            if let (Some(cb), Some(ce)) = (self.t.contents_begin(el), self.t.contents_end(el)) {
                use Granularity as G;
                if ty.is_greater()
                    && (matches!(gran, G::Element | G::Object)
                        || (gran == G::GreaterElement && ty == Type::Section)
                        || ty == Type::Headline)
                {
                    let st = if matches!(ty, Type::Item | Type::PlainList) {
                        self.structs.get(&el).cloned()
                    } else {
                        None
                    };
                    self.parse_elements(cb, ce, next_mode(mode, ty, true), st, gran, el);
                } else if gran == G::Object {
                    let r = super::restriction(ty);
                    let objs = self.parse_objects(cb, ce, r, Some(el));
                    self.t.set_contents(el, objs);
                }
            }
            elements.push(el);
            mode = next_mode(mode, ty, false);
        }
        self.t.set_contents(acc, elements);
    }

    fn at_headline(&self, p: usize) -> bool {
        self.at_bol(p)
            && self
                .headline_stars(p)
                .is_some_and(|n| n <= self.cx.max_headline_stars())
    }

    /// Stars of a `^\*+ ` line at P.
    pub fn headline_stars(&self, p: usize) -> Option<usize> {
        let n = self.s[p..].bytes().take_while(|&b| b == b'*').count();
        (n > 0 && self.s.as_bytes().get(p + n) == Some(&b' ')).then_some(n)
    }

    /// The next limited-level headline line strictly after P's line
    /// start (re-search-forward of org-get-limited-outline-regexp).
    fn next_headline(&self, p: usize) -> Option<usize> {
        self.next_headline_max(p, self.cx.max_headline_stars())
    }

    /// The first line starting after P whose stars are <= MAX.
    pub fn next_headline_max(&self, p: usize, max: usize) -> Option<usize> {
        let mut l = if self.at_bol(p) { p } else { self.next_line(p) };
        while l < self.len() {
            if self.headline_stars(l).is_some_and(|n| n <= max) {
                return Some(l);
            }
            l = self.next_line(l);
        }
        None
    }

    // ---- org-element--current-element ----

    pub fn current_element(
        &mut self,
        p: usize,
        limit: usize,
        gran: Granularity,
        mode: Mode,
        structure: Option<Struct>,
    ) -> NodeId {
        let raw_secondary = gran != Granularity::Object;
        match mode {
            Mode::Item => {
                if let Some(st) = structure.clone() {
                    return self.item_parser(p, limit, st, raw_secondary);
                }
                let st = self.list_struct(p, limit);
                return self.item_parser(p, limit, st, raw_secondary);
            }
            Mode::TableRow => return self.table_row_parser(p),
            Mode::NodeProperty => return self.node_property_parser(p),
            _ => {}
        }
        let bol = self.at_bol(p);
        let stars = if bol { self.headline_stars(p) } else { None };
        if let Some(n) = stars
            && n < self.cx.inlinetask_stars()
        {
            return self.headline_parser(p, raw_secondary);
        }
        if matches!(mode, Mode::Section | Mode::FirstSection) {
            return self.section_parser(p);
        }
        if bol && self.comment_line(p) {
            return self.comment_parser(p, limit);
        }
        let prev_star = self.s.as_bytes().get(self.prev_bol(p)) == Some(&b'*');
        if mode == Mode::Planning && prev_star && bol && self.planning_line(p) {
            return self.planning_parser(p, limit);
        }
        let pd_ok = match mode {
            Mode::Planning => prev_star,
            Mode::PropertyDrawer | Mode::TopComment => true,
            _ => false,
        };
        if pd_ok && bol && self.looking_at_p(&PROPERTY_DRAWER, p) {
            return self.property_drawer_parser(p, limit);
        }
        if !bol {
            return self.paragraph_parser(p, limit, (p, vec![]));
        }
        if self.looking_at_p(&CLOCK_LINE, p) {
            return self.clock_parser(p, limit);
        }
        if stars.is_some() {
            return self.inlinetask_parser(p, limit, raw_secondary);
        }
        let parse = matches!(gran, Granularity::Object);
        let (aff, q) = self.collect_affiliated(p, limit, parse);
        if !aff.1.is_empty() && q >= limit {
            return self.keyword_parser(aff.0, limit, (aff.0, vec![]));
        }
        if let Some(c) = self.looking_at(&CURRENT, q) {
            let g = |i| c.get(i).is_some();
            if g(1) {
                return self.latex_environment_parser(q, limit, aff);
            }
            if g(2) {
                return self.drawer_parser(q, limit, aff);
            }
            if g(3) {
                return self.fixed_width_parser(q, limit, aff);
            }
            if let Some(m) = c.get(6) {
                return match m.as_str().to_uppercase().as_str() {
                    "CENTER" => {
                        self.greater_block_parser(q, limit, aff, Type::CenterBlock, "CENTER")
                    }
                    "QUOTE" => self.greater_block_parser(q, limit, aff, Type::QuoteBlock, "QUOTE"),
                    "COMMENT" => self.comment_block_parser(q, limit, aff),
                    "EXAMPLE" => self.example_block_parser(q, limit, aff),
                    "EXPORT" => self.export_block_parser(q, limit, aff),
                    "SRC" => self.src_block_parser(q, limit, aff),
                    "VERSE" => self.verse_block_parser(q, limit, aff),
                    _ => self.special_block_parser(q, limit, aff),
                };
            }
            if g(7) {
                return self.babel_call_parser(q, limit, aff);
            }
            if g(4) {
                return self.dynamic_block_parser(q, limit, aff);
            }
            if g(8) {
                return self.keyword_parser(q, limit, aff);
            }
            if g(5) {
                return self.paragraph_parser(q, limit, aff);
            }
            if g(9) {
                return self.footnote_definition_parser(q, limit, aff);
            }
            if g(10) {
                return self.horizontal_rule_parser(q, limit, aff);
            }
            if g(11) {
                return self.diary_sexp_parser(q, limit, aff);
            }
        }
        if self.s[q..].trim_start_matches([' ', '\t']).starts_with('|') || self.table_el_p(q, limit)
        {
            return self.table_parser(q, limit, aff);
        }
        if self.looking_at_p(&self.cx.item_re, q) {
            let st = match structure {
                Some(s) => s,
                None => self.list_struct(q, limit),
            };
            return self.plain_list_parser(q, limit, aff, Some(st));
        }
        self.paragraph_parser(q, limit, aff)
    }

    /// The table.el heuristic of org-element--current-element.
    fn table_el_p(&self, p: usize, limit: usize) -> bool {
        let next = self.next_line(p);
        if !self.looking_at_p(&TABLE_EL_RULE, p) || next >= limit {
            return false;
        }
        // The first line from the end of this one that is not a table.el
        // line (blank or starting with something else than +|).
        let mut l = next;
        let mut found = None;
        while l < limit {
            let line = &self.s[l..self.eol(l).min(limit)];
            let t = line.trim_start_matches([' ', '\t']);
            if t.is_empty() || !(t.starts_with('+') || t.starts_with('|')) {
                found = Some(l);
                break;
            }
            l = self.next_line(l);
        }
        match found {
            None => {
                // At limit: the last line must be a rule.
                let last = if self.at_bol(limit) && limit > 0 {
                    self.bol(limit - 1)
                } else {
                    self.bol(limit)
                };
                self.looking_at_p(&TABLE_EL_RULE, last)
            }
            Some(l) if l == next => false,
            Some(l) => self.looking_at_p(&TABLE_EL_RULE, self.bol(l - 1)),
        }
    }

    /// org-comment-regexp at P (a line start).
    pub fn comment_line(&self, p: usize) -> bool {
        let t = self.s[p..].trim_start_matches([' ', '\t']);
        t.starts_with('#') && matches!(t.as_bytes().get(1), None | Some(b' ') | Some(b'\n'))
    }

    /// org-element-planning-line-re (case-insensitive, as
    /// org-element--current-element binds case-fold-search).
    pub fn planning_line(&self, p: usize) -> bool {
        let t = self.s[p..].trim_start_matches([' ', '\t']);
        ["CLOSED:", "DEADLINE:", "SCHEDULED:"]
            .iter()
            .any(|k| starts_with_ci(t, k))
    }

    // ---- affiliated keywords ----

    /// org-element--collect-affiliated-keywords: ((begin, props), point
    /// after them).
    pub fn collect_affiliated(&mut self, origin: usize, limit: usize, parse: bool) -> (Aff, usize) {
        if !self.at_bol(origin) {
            return ((origin, vec![]), origin);
        }
        let restrict: &[Type] = if parse {
            super::restriction(Type::Keyword)
        } else {
            &[]
        };
        let mut out: Vec<(String, Value)> = vec![];
        let mut p = origin;
        while p < limit {
            let Some(c) = self.looking_at(&AFFILIATED, p) else {
                break;
            };
            let raw = c
                .get(1)
                .or(c.get(3))
                .or(c.get(4))
                .unwrap()
                .as_str()
                .to_uppercase();
            let kwd = match raw.as_str() {
                "DATA" | "LABEL" | "RESNAME" | "SOURCE" | "SRCNAME" | "TBLNAME" => {
                    "NAME".to_owned()
                }
                "RESULT" => "RESULTS".to_owned(),
                "HEADERS" => "HEADER".to_owned(),
                _ => raw,
            };
            let parsed = kwd == "CAPTION";
            let vb = p + c.get(0).unwrap().end();
            let ve = self.skip_back(self.eol(p), BLANK, 0);
            let dual = kwd == "CAPTION" || kwd == "RESULTS";
            let dual_value = match span(&c, 2, p) {
                Some((a, b)) if dual && parsed => {
                    Value::Secondary(self.parse_objects(a, b, restrict, None))
                }
                Some((a, b)) if dual => Value::Str(self.s[a..b].to_owned()),
                _ => Value::Nil,
            };
            let mut value = if parsed {
                Value::Secondary(self.parse_objects(vb, ve, restrict, None))
            } else {
                Value::Str(trim(self.sub(vb, ve)).to_owned())
            };
            if dual {
                value = if value.truthy() || dual_value.truthy() {
                    Value::cons(value, dual_value)
                } else {
                    Value::Nil
                };
            }
            let k = kwd.to_lowercase();
            if kwd == "CAPTION" || kwd == "HEADER" || kwd.starts_with("ATTR_") {
                let mut l = match out.iter().find(|(n, _)| *n == k) {
                    Some((_, Value::List(l))) => l.clone(),
                    _ => vec![],
                };
                l.push(value);
                value = Value::List(l);
            }
            match out.iter_mut().find(|(n, _)| *n == k) {
                Some((_, v)) => *v = value,
                None => out.push((k, value)),
            }
            p = self.next_line(p);
        }
        let t = self.s[p..].trim_start_matches([' ', '\t']);
        let orphan = t.is_empty()
            || t.starts_with('\n')
            || self.comment_line(p)
            || (self.at_bol(p) && self.looking_at_p(&CLOCK_PREFIX, p))
            || (self.at_bol(p) && self.headline_stars(p).is_some());
        if orphan {
            return ((origin, vec![]), origin);
        }
        ((origin, out), p)
    }

    /// Create an element with standard and affiliated properties.
    pub fn el(&mut self, ty: Type, mut props: Vec<(&str, Value)>, aff: Aff) -> NodeId {
        props.retain(|(_, v)| !matches!(v, Value::Nil));
        let id = self.t.create(ty, props, vec![]);
        for (k, v) in aff.1 {
            self.adopt_value(id, &v);
            self.t.node_mut(id).props.push((k, v));
        }
        id
    }

    /// Parent the nodes of a property value to ID.
    pub fn adopt_value(&mut self, id: NodeId, v: &Value) {
        match v {
            Value::Node(n) => self.t.set_parent(*n, Some(id)),
            Value::Secondary(s) => s.iter().for_each(|&n| self.t.set_parent(n, Some(id))),
            Value::List(l) => l.iter().for_each(|x| self.adopt_value(id, x)),
            Value::Cons(a, b) => {
                self.adopt_value(id, a);
                self.adopt_value(id, b);
            }
            _ => {}
        }
    }

    // ---- greater elements ----

    /// center-block and quote-block parsers.
    fn greater_block_parser(
        &mut self,
        p: usize,
        limit: usize,
        aff: Aff,
        ty: Type,
        name: &str,
    ) -> NodeId {
        let end_name = format!("#+END_{name}");
        let Some(block_end) = self.find_line(p, limit, |l| block_end_line(l, &end_name)) else {
            return self.paragraph_parser(p, limit, aff);
        };
        let cb = self.next_line(p);
        let cb = (cb < block_end).then_some(cb);
        let before_blank = self.next_line(block_end);
        let end = self.end_after_blank(before_blank, limit);
        self.el(
            ty,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(cb.map(|_| block_end))),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn drawer_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let from = limit.min(self.eol(p));
        let Some(drawer_end) = self.find_line(from, limit, |l| block_end_line(l, ":END:")) else {
            return self.paragraph_parser(p, limit, aff);
        };
        let name = self
            .looking_at(&DRAWER, p)
            .map(|c| c[1].to_owned())
            .unwrap_or_default();
        let after = self.next_line(p);
        let q = self.skip_fwd(after, WS, self.len());
        let q = self.bol(q);
        let cb = (q < drawer_end).then_some(q);
        let pre_blank = self
            .count_lines(p, cb.unwrap_or(drawer_end))
            .saturating_sub(1);
        let before_blank = self.next_line(drawer_end);
        let end = self.end_after_blank(before_blank, limit);
        self.el(
            Type::Drawer,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("pre-blank", Self::pos(pre_blank)),
                ("drawer-name", Value::Str(name)),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(cb.map(|_| drawer_end))),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn dynamic_block_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let Some(block_end) = self.find_line(p, limit, |l| {
            let t = l.trim_matches([' ', '\t']);
            t.eq_ignore_ascii_case("#+END:") || t.eq_ignore_ascii_case("#+END")
        }) else {
            return self.paragraph_parser(p, limit, aff);
        };
        let c = self.looking_at(&DYN_OPEN, p);
        let name = c.as_ref().map(|c| c[1].to_owned());
        let args = c
            .as_ref()
            .and_then(|c| c.get(2))
            .map(|m| m.as_str().to_owned());
        let cb = self.next_line(p);
        let cb = (cb < block_end).then_some(cb);
        let before_blank = self.next_line(block_end);
        let end = self.end_after_blank(before_blank, limit);
        self.el(
            Type::DynamicBlock,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("block-name", Value::opt_str(name)),
                ("arguments", Value::opt_str(args)),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(cb.map(|_| block_end))),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn footnote_definition_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let label = self
            .looking_at(&FOOTNOTE_DEF, p)
            .map(|c| c[1].to_owned())
            .unwrap_or_default();
        let from = self.eol(p);
        let end = match FOOTNOTE_SEPARATOR.find_at(&self.s[..limit], from) {
            None => limit,
            Some(m) if self.s.as_bytes()[m.start()] == b'[' => {
                // A new definition: end before its affiliated keywords.
                let mut l = self.prev_bol(m.start());
                while l > p && self.looking_at_p(&AFFILIATED, l) {
                    l = self.prev_bol(l);
                }
                self.next_line(l)
            }
            Some(m) if self.s.as_bytes()[m.start()] == b'*' => m.start(),
            Some(m) => {
                let q = self.skip_fwd(m.start(), WS, limit);
                if q == limit { limit } else { self.bol(q) }
            }
        };
        let close = p + self.s[p..].find(']').unwrap() + 1;
        let q = self.skip_fwd(close, WS, end);
        let mut pre_blank = 0;
        let cb = if q == end {
            None
        } else if self.bol(q) == p {
            Some(q)
        } else {
            pre_blank = self.count_lines(self.bol(q), aff.0);
            Some(self.bol(q))
        };
        let ce = self.next_line(self.skip_back(end, WS, 0));
        self.el(
            Type::FootnoteDefinition,
            vec![
                ("label", Value::Str(label)),
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(cb.map(|_| ce))),
                ("pre-blank", Self::pos(pre_blank)),
                ("post-blank", Self::pos(self.count_lines(ce, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    /// org-element--get-node-properties: properties of the property
    /// drawer at P (a line start), as (UPCASED-KEY, value).
    fn node_properties(&self, p: usize) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = vec![];
        let Some(m) = PROPERTY_DRAWER.find(&self.s[p..]) else {
            return out;
        };
        let end = p + m.end();
        let mut l = self.next_line(p);
        while self.eol(l) < end {
            if let Some(c) = self.looking_at(&PROPERTY, l) {
                let name = c[3].to_uppercase();
                let val = c.get(5).map_or("", |m| m.as_str()).to_owned();
                match out.iter_mut().find(|(k, _)| *k == name) {
                    Some((_, old)) if name.ends_with('+') => match old {
                        Value::List(v) => v.push(Value::Str(val)),
                        o => *o = Value::List(vec![o.clone(), Value::Str(val)]),
                    },
                    Some((_, old)) => *old = Value::Str(val),
                    None => out.push((name, Value::Str(val))),
                }
            }
            l = self.next_line(l);
        }
        out
    }

    /// org-element--get-time-properties for the headline at P.
    fn time_properties(&mut self, p: usize) -> Vec<(&'static str, Value)> {
        let l = self.next_line(p);
        let mut out = vec![];
        if l >= self.len() || !self.planning_line(l) {
            return out;
        }
        let end = self.eol(l);
        let mut q = l;
        while let Some((k, after)) = self.next_planning_keyword(q, end) {
            let ts_at = self.skip_fwd(after, BLANK, self.len());
            let ts = self.timestamp_at(ts_at);
            let key = match k {
                "SCHEDULED:" => "scheduled",
                "DEADLINE:" => "deadline",
                _ => "closed",
            };
            if let Some((id, ts_end)) = ts {
                out.retain(|(x, _)| *x != key);
                out.push((key, Value::Node(id)));
                q = ts_end.max(after);
            } else {
                out.retain(|(x, _)| *x != key);
                q = after;
            }
        }
        out
    }

    /// The next planning keyword in [Q, END): (keyword, end).
    fn next_planning_keyword(&self, q: usize, end: usize) -> Option<(&'static str, usize)> {
        let hay = &self.s[q..end];
        ["CLOSED:", "DEADLINE:", "SCHEDULED:"]
            .iter()
            .filter_map(|k| hay.find(k).map(|i| (*k, i)))
            .min_by_key(|(_, i)| *i)
            .map(|(k, i)| (k, q + i + k.len()))
    }

    /// A timestamp object at P (not adopted): (id, end after blanks).
    fn timestamp_at(&mut self, p: usize) -> Option<(NodeId, usize)> {
        let id = super::objects::timestamp_parser(self, p, self.len())?;
        let e = self.t.end(id).unwrap_or(p);
        Some((id, e))
    }

    pub fn headline_parser(&mut self, p: usize, raw_secondary: bool) -> NodeId {
        let true_level = self.headline_stars(p).unwrap_or(0);
        let end = self
            .next_headline_after(p + true_level, true_level)
            .unwrap_or(self.len());
        let l = self.next_line(p);
        let q = self.skip_fwd(l, WS, end);
        let cb = (q != end).then(|| self.bol(q));
        let ce = cb.map(|_| end);
        let id = self.t.create(
            Type::Headline,
            vec![
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                (
                    "pre-blank",
                    Self::pos(cb.map_or(0, |c| self.count_lines(p, c).saturating_sub(1))),
                ),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(ce)),
                ("true-level", Self::pos(true_level)),
            ],
            vec![],
        );
        self.headline_title(id, p, raw_secondary);
        let pb = if ce.is_some() {
            0
        } else {
            self.count_lines(p, end).saturating_sub(1)
        };
        self.t.put(id, "post-blank", pb);
        self.t.put(id, "post-affiliated", p);
        self.headline_deferred(id, p);
        id
    }

    /// The next line after P with 1..=LEVEL stars and a space.
    fn next_headline_after(&self, p: usize, level: usize) -> Option<usize> {
        self.next_headline_max(self.next_line(p), level)
    }

    /// org-element--headline-deferred: planning and node properties.
    fn headline_deferred(&mut self, id: NodeId, p: usize) {
        for (k, v) in self.time_properties(p) {
            if let Value::Node(n) = v {
                self.t.set_parent(n, None);
            }
            self.t.put(id, k, v);
        }
        let mut l = self.next_line(p);
        if l < self.len() && self.planning_line(l) {
            l = self.next_line(l);
        }
        if l < self.len() {
            for (k, v) in self.node_properties(l) {
                self.t.put(id, &k, v);
            }
        }
    }

    /// org-element--headline-parse-title.
    fn headline_title(&mut self, id: NodeId, begin: usize, raw_secondary: bool) {
        let s = self.s;
        let true_level = s[begin..].bytes().take_while(|&b| b == b'*').count();
        let mut p = self.skip_fwd(begin + true_level, BLANK, self.len());
        let eol = self.eol(p);
        let level = self.cx.reduced_level(true_level);
        // TODO keyword: the longest keyword followed by a space or eol.
        let mut todo = None;
        for k in &self.cx.todo {
            let e = p + k.len();
            if s[p..eol].starts_with(k.as_str())
                && (e == eol || s.as_bytes()[e] == b' ')
                && todo.as_ref().is_none_or(|t: &String| t.len() < k.len())
            {
                todo = Some(k.clone());
            }
        }
        if let Some(k) = &todo {
            p += k.len();
            if p < eol {
                p += 1;
            }
            p = self.skip_fwd(p, BLANK, self.len());
        }
        let todo_type = todo.as_ref().map(|k| {
            if self.cx.done.contains(k) {
                "done"
            } else {
                "todo"
            }
        });
        let mut priority = None;
        if let Some(c) = PRIORITY.captures(&s[p..eol]) {
            let v = &c[2];
            priority = Some(if v.as_bytes()[0].is_ascii_digit() {
                v.parse::<i64>().unwrap_or(0)
            } else {
                v.as_bytes()[0] as i64
            });
            p += c.get(0).unwrap().end();
        }
        let mut commented = false;
        if s[p..eol].starts_with("COMMENT")
            && matches!(s.as_bytes().get(p + 7), None | Some(b' ') | Some(b'\n'))
        {
            commented = true;
            p += 7;
            if p < eol {
                p += 1;
            }
            p = self.skip_fwd(p, BLANK, self.len());
        }
        let title_start = p;
        let back = self.skip_back(p, BLANK, 0);
        let mut tags = vec![];
        let title_end = match TAG_GROUP.captures_at(&s[..eol], back) {
            Some(c) => {
                tags = split_string(&c[1], ':');
                c.get(0).unwrap().start()
            }
            None => eol.max(back),
        };
        let raw = trim(self.sub(title_start, title_end)).to_owned();
        let archived = tags.iter().any(|t| t == "ARCHIVE");
        let fsp = self.cx.footnote_section.as_ref().is_some_and(|f| *f == raw);
        let t = &mut self.t;
        t.put(id, "raw-value", raw.clone());
        t.put(id, "level", level);
        if let Some(pr) = priority {
            t.put(id, "priority", pr);
        }
        if !tags.is_empty() {
            t.put(
                id,
                "tags",
                Value::List(tags.into_iter().map(Value::Str).collect()),
            );
        }
        if let Some(k) = todo {
            t.put(id, "todo-keyword", k);
        }
        if let Some(tt) = todo_type {
            t.put(id, "todo-type", Value::sym(tt));
        }
        if fsp {
            t.put(id, "footnote-section-p", Value::T);
        }
        if archived {
            t.put(id, "archivedp", Value::T);
        }
        if commented {
            t.put(id, "commentedp", Value::T);
        }
        if raw_secondary {
            t.put(id, "title", raw);
        } else {
            let a = self.skip_fwd(title_start, BLANK, self.len());
            let b = self.skip_back(title_end, BLANK, 0);
            let objs = self.parse_objects(a, b, super::restriction(Type::Headline), Some(id));
            if !objs.is_empty() {
                self.t.put_secondary(id, "title", objs);
            }
        }
    }

    fn inlinetask_parser(&mut self, p: usize, limit: usize, raw_secondary: bool) -> NodeId {
        let l = self.next_line(p);
        let mut task_end = None;
        let mut q = l;
        while q < limit {
            if self.headline_stars(q).is_some() {
                let stars = self.headline_stars(q).unwrap();
                let rest = &self.s[q + stars..self.eol(q)];
                if rest.trim_matches([' ', '\t']).eq_ignore_ascii_case("END") {
                    task_end = Some(q);
                }
                break;
            }
            q = self.next_line(q);
        }
        let cb = task_end.filter(|&te| p < te).map(|_| {
            let a = self.skip_fwd(l, b" \t\n", self.len());
            self.bol(a)
        });
        let ce = cb.and(task_end);
        let after = self.next_line(task_end.unwrap_or(p));
        let end = self.end_after_blank(after, limit);
        let id = self.t.create(
            Type::Inlinetask,
            vec![
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                (
                    "pre-blank",
                    Self::pos(cb.map_or(0, |c| self.count_lines(p, c).saturating_sub(1))),
                ),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(ce)),
            ],
            vec![],
        );
        self.headline_title(id, p, raw_secondary);
        self.t
            .node_mut(id)
            .props
            .retain(|(k, _)| k != "footnote-section-p");
        self.t.put(
            id,
            "post-blank",
            self.count_lines(task_end.unwrap_or(p), end)
                .saturating_sub(1),
        );
        self.t.put(id, "post-affiliated", p);
        if task_end.is_some() {
            self.headline_deferred(id, p);
        }
        id
    }

    fn item_parser(&mut self, p: usize, limit: usize, st: Struct, raw_secondary: bool) -> NodeId {
        let begin = self.bol(p);
        let c = self.looking_at(&FULL_ITEM, begin).expect("item");
        let bullet = c[1].to_owned();
        let tag_span = span(&c, 4, begin);
        let checkbox = c.get(3).map(|m| match m.as_str() {
            "[ ]" => "off",
            "[X]" => "on",
            _ => "trans",
        });
        let counter = c.get(2).map(|m| {
            let v = m.as_str();
            if v.as_bytes()[0].is_ascii_alphabetic() {
                v.to_ascii_uppercase().as_bytes()[0] as i64 - 64
            } else {
                v.parse::<i64>().unwrap_or(0)
            }
        });
        let item_end = st.iter().find(|i| i.pos == begin).map_or(limit, |i| i.end);
        let end = limit.min(if self.at_bol(item_end) {
            item_end
        } else {
            self.next_line(item_end)
        });
        let start = match tag_span {
            Some((a, _)) if bullet.contains(['.', ')']) => a,
            _ => begin + c.get(0).unwrap().end(),
        };
        let q = self.skip_fwd(start, WS, end);
        let mut pre_blank = 0;
        let cb = if q == end {
            None
        } else if self.bol(q) == begin {
            Some(q)
        } else {
            pre_blank = self.count_lines(self.bol(q), begin);
            Some(self.bol(q))
        };
        let ce = cb.map(|_| self.next_line(self.skip_back(end, WS, 0)));
        let structure = struct_value(&st);
        let id = self.t.create(
            Type::Item,
            vec![
                ("bullet", Value::Str(bullet)),
                ("begin", Self::pos(begin)),
                ("end", Self::pos(end)),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(ce)),
                ("checkbox", checkbox.map_or(Value::Nil, Value::sym)),
                ("counter", counter.map_or(Value::Nil, Value::Int)),
                ("structure", structure),
                ("pre-blank", Self::pos(pre_blank)),
                (
                    "post-blank",
                    Self::pos(self.count_lines(ce.unwrap_or(begin), end)),
                ),
                ("post-affiliated", Self::pos(begin)),
            ],
            vec![],
        );
        let raw_tag = st
            .iter()
            .find(|i| i.pos == begin)
            .and_then(|i| i.tag.clone());
        if let Some(tag) = raw_tag {
            if raw_secondary {
                self.t.put(id, "tag", tag);
            } else if let Some((a, b)) = tag_span {
                let objs = self.parse_objects(a, b, super::restriction(Type::Item), Some(id));
                self.t.put_secondary(id, "tag", objs);
            }
        }
        self.structs.insert(id, st);
        id
    }

    /// org-element--list-struct.
    pub fn list_struct(&self, start: usize, limit: usize) -> Struct {
        let mut items: Vec<ListItem> = vec![];
        let mut done: Vec<ListItem> = vec![];
        let mut p = start;
        let finish = |mut v: Vec<ListItem>| {
            v.sort_by_key(|i| i.pos);
            Rc::new(v)
        };
        loop {
            if p >= limit {
                let end = self.next_line(self.skip_back(p, WS, 0));
                let end = if self.skip_back(p, WS, 0) == self.len() {
                    self.len()
                } else {
                    end
                };
                for mut i in items.drain(..) {
                    i.end = end;
                    done.push(i);
                }
                return finish(done);
            }
            if self.list_end_at(p) {
                for mut i in items.drain(..) {
                    i.end = p;
                    done.push(i);
                }
                return finish(done);
            }
            if self.looking_at_p(&self.cx.item_re, p) {
                let ind = self.indentation(p);
                while items.last().is_some_and(|i| ind <= i.ind) {
                    let mut i = items.pop().unwrap();
                    i.end = p;
                    done.push(i);
                }
                let c = self.looking_at(&FULL_ITEM, p).unwrap();
                let bullet = c[1].to_owned();
                let tag = if bullet.contains(['-', '+', '*']) {
                    c.get(4).map(|m| m.as_str().to_owned())
                } else {
                    None
                };
                items.push(ListItem {
                    pos: p,
                    ind,
                    bullet,
                    counter: c.get(2).map(|m| m.as_str().to_owned()),
                    checkbox: c.get(3).map(|m| m.as_str().to_owned()),
                    tag,
                    end: 0,
                });
                p = self.next_line(p);
                continue;
            }
            if self.s[p..self.eol(p)]
                .bytes()
                .all(|b| b == b' ' || b == b'\t')
            {
                p = self.next_line(p);
                continue;
            }
            let ind = self.indentation(p);
            let end = self.next_line(self.skip_back(p, WS, 0));
            while items.last().is_some_and(|i| ind <= i.ind) {
                let mut i = items.pop().unwrap();
                i.end = end;
                done.push(i);
                if items.is_empty() {
                    return finish(done);
                }
            }
            // Skip blocks and drawers.
            let t = self.s[p..].trim_start_matches([' ', '\t']);
            let lower = t.get(..8).unwrap_or("").to_ascii_uppercase();
            if lower.starts_with("#+BEGIN:") || lower.starts_with("#+BEGIN_") {
                let rest = &t[7..];
                let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
                let target = format!("#+END{name}");
                if let Some(l) =
                    self.find_line(self.next_line(p), limit, |l| block_end_line(l, &target))
                {
                    p = l;
                } else if self.find_line(p, limit, |l| block_end_line(l, &target)) == Some(p) {
                    // The opening line itself cannot close.
                }
            } else if self.looking_at_p(&DRAWER, p)
                && let Some(l) =
                    self.find_line(self.next_line(p), limit, |l| block_end_line(l, ":END:"))
            {
                p = l;
            }
            p = self.next_line(p);
        }
    }

    /// org-list-end-re at P: two blank lines.
    fn list_end_at(&self, p: usize) -> bool {
        if !self.at_bol(p) {
            return false;
        }
        let blank = |l: usize| -> Option<usize> {
            let e = self.eol(l);
            (e < self.len() && self.s[l..e].bytes().all(|b| b == b' ' || b == b'\t'))
                .then_some(e + 1)
        };
        blank(p).and_then(blank).is_some()
    }

    fn plain_list_parser(
        &mut self,
        p: usize,
        limit: usize,
        aff: Aff,
        structure: Option<Struct>,
    ) -> NodeId {
        let st = match structure {
            Some(s) if s.iter().any(|i| i.pos == p) => s,
            _ => self.list_struct(p, limit),
        };
        let first = st.iter().find(|i| i.pos == p).cloned();
        let t = self.s[p..].trim_start_matches([' ', '\t']);
        let ty = if t
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        {
            "ordered"
        } else if first.as_ref().is_some_and(|i| i.tag.is_some()) {
            "descriptive"
        } else {
            "unordered"
        };
        let mut pos = first.as_ref().map_or(limit, |i| i.end);
        if let Some(f) = &first {
            while let Some(i) = st.iter().find(|i| i.pos == pos && i.ind == f.ind) {
                pos = i.end;
            }
        }
        let b = self.skip_back(pos, WS, 0);
        let ce = if self.at_bol(b) { b } else { self.next_line(b) };
        let q = self.skip_fwd(ce, WS, limit);
        let end = if q == limit { limit } else { self.bol(q) };
        let id = self.el(
            Type::PlainList,
            vec![
                ("type", Value::sym(ty)),
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("contents-begin", Self::pos(p)),
                ("contents-end", Self::pos(ce)),
                ("structure", struct_value(&st)),
                ("post-blank", Self::pos(self.count_lines(ce, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        );
        self.structs.insert(id, st);
        id
    }

    fn property_drawer_parser(&mut self, p: usize, limit: usize) -> NodeId {
        let cb = self.next_line(p);
        let end_line = self
            .find_line(p, limit, |l| block_end_line(l, ":END:"))
            .unwrap_or(cb);
        let ce = (end_line > cb).then_some(end_line);
        let before_blank = self.next_line(end_line);
        let end = self.end_after_blank(before_blank, limit);
        self.t.create(
            Type::PropertyDrawer,
            vec![
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                ("contents-begin", Value::opt_int(ce.map(|_| cb))),
                ("contents-end", Value::opt_int(ce)),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            vec![],
        )
    }

    fn section_parser(&mut self, p: usize) -> NodeId {
        let end = self.next_headline(p).unwrap_or(self.len());
        self.t.create(
            Type::Section,
            vec![
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                ("contents-begin", Self::pos(p)),
                ("contents-end", Self::pos(end)),
                ("post-blank", Self::pos(0)),
                ("post-affiliated", Self::pos(p)),
            ],
            vec![],
        )
    }

    fn special_block_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let c = self.looking_at(&SPECIAL, p).unwrap();
        let ty = c[1].to_owned();
        let params = c.get(2).map(|m| m.as_str().to_owned());
        let target = format!("#+END_{ty}");
        let Some(block_end) = self.find_line(p, limit, |l| block_end_line(l, &target)) else {
            return self.paragraph_parser(p, limit, aff);
        };
        let cb = self.next_line(p);
        let cb = (cb < block_end).then_some(cb);
        let before_blank = self.next_line(block_end);
        let end = self.end_after_blank(before_blank, limit);
        self.el(
            Type::SpecialBlock,
            vec![
                ("type", Value::Str(ty)),
                (
                    "parameters",
                    Value::opt_str(nw(params.as_deref()).map(trim)),
                ),
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(cb.map(|_| block_end))),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    // ---- elements ----

    fn babel_call_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let before_blank = self.next_line(p);
        let colon = self.s[p..before_blank]
            .find(':')
            .map(|i| p + i + 1)
            .unwrap_or(p);
        let q = self.skip_fwd(colon, BLANK, self.len());
        let eol = self.eol(q);
        let value = trim(&self.s[q..eol]).to_owned();
        let mut r = q;
        while r < before_blank && !b"[]()\n".contains(&self.s.as_bytes()[r]) {
            r += 1;
        }
        let call = nw(Some(&self.s[q..r])).map(str::to_owned);
        let (inside, r) = match paired(self.s, r, b'[', self.len()) {
            Some((v, e)) => (Some(v.to_owned()), e),
            None => (None, r),
        };
        let (args, r) = match paired(self.s, r, b'(', self.len()) {
            Some((v, e)) => (nw(Some(v)).map(str::to_owned), e),
            None => (None, r),
        };
        let end_header = nw(Some(trim(&self.s[r.min(eol)..eol]))).map(str::to_owned);
        let end = self.end_after_blank(before_blank, limit);
        self.el(
            Type::BabelCall,
            vec![
                ("call", Value::opt_str(call)),
                ("inside-header", Value::opt_str(inside)),
                ("arguments", Value::opt_str(args)),
                ("end-header", Value::opt_str(end_header)),
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("value", Value::Str(value)),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn clock_parser(&mut self, p: usize, limit: usize) -> NodeId {
        let eol = self.eol(p);
        let after = self.s[p..eol]
            .to_ascii_uppercase()
            .find("CLOCK:")
            .map_or(p, |i| p + i + 6);
        let q = self.skip_fwd(after, BLANK, self.len());
        let value = super::objects::timestamp_parser(self, q, self.len());
        let from = value.and_then(|v| self.t.end(v)).unwrap_or(q).min(eol);
        let duration = self.s[from..eol].find("=> ").and_then(|i| {
            let d = self.skip_fwd(from + i + 3, BLANK, eol);
            let t = &self.s[d..eol];
            let word = t.trim_end_matches([' ', '\t']);
            (!word.is_empty() && !word.contains([' ', '\t'])).then(|| word.to_owned())
        });
        let before_blank = self.next_line(p);
        let mut q = self.skip_fwd(before_blank, WS, limit);
        q = self.skip_back(q, BLANK, 0);
        if !self.at_bol(q) {
            q = self.skip_fwd(q, BLANK, self.len());
        }
        let post_blank = self.count_lines(before_blank, q);
        let status = if duration.is_some() {
            "closed"
        } else {
            "running"
        };
        let id = self.t.create(
            Type::Clock,
            vec![
                ("status", Value::sym(status)),
                ("value", value.map_or(Value::Nil, Value::Node)),
                ("duration", Value::opt_str(duration)),
                ("begin", Self::pos(p)),
                ("end", Self::pos(q)),
                ("post-blank", Self::pos(post_blank)),
                ("post-affiliated", Self::pos(p)),
            ],
            vec![],
        );
        if let Some(v) = value {
            self.t.set_parent(v, Some(id));
        }
        id
    }

    fn comment_parser(&mut self, p: usize, limit: usize) -> NodeId {
        let comment_value = |l: usize| -> &str {
            let t = self.s[l..self.eol(l)].trim_start_matches([' ', '\t']);
            let rest = &t[1..];
            rest.strip_prefix(' ').unwrap_or(rest)
        };
        let mut value = comment_value(p).to_owned();
        let mut l = self.next_line(p);
        while l < limit && self.comment_line(l) {
            value.push('\n');
            value.push_str(comment_value(l));
            l = self.next_line(l);
        }
        let end = self.end_after_blank(l, limit);
        self.t.create(
            Type::Comment,
            vec![
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                ("value", Value::Str(value)),
                ("post-blank", Self::pos(self.count_lines(l, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            vec![],
        )
    }

    /// Blocks with a value: (contents-begin, contents-end, before-blank,
    /// end), or None when unterminated.
    fn value_block(
        &self,
        p: usize,
        limit: usize,
        name: &str,
    ) -> Option<(usize, usize, usize, usize)> {
        let target = format!("#+END_{name}");
        let ce = self.find_line(p, limit, |l| block_end_line(l, &target))?;
        let cb = self.next_line(p);
        let before_blank = self.next_line(ce);
        Some((
            cb,
            ce,
            before_blank,
            self.end_after_blank(before_blank, limit),
        ))
    }

    fn comment_block_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let Some((cb, ce, bb, end)) = self.value_block(p, limit, "COMMENT") else {
            return self.paragraph_parser(p, limit, aff);
        };
        let value = self.sub(cb, ce).to_owned();
        self.el(
            Type::CommentBlock,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("value", Value::Str(value)),
                ("post-blank", Self::pos(self.count_lines(bb, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn diary_sexp_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let eol = self.eol(p);
        let value = self.s[p..eol].trim_end_matches([' ', '\t']).to_owned();
        let bb = self.next_line(p);
        let end = self.end_after_blank(bb, limit);
        self.el(
            Type::DiarySexp,
            vec![
                ("value", Value::Str(value)),
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("post-blank", Self::pos(self.count_lines(bb, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn example_block_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let Some((cb, ce, bb, end)) = self.value_block(p, limit, "EXAMPLE") else {
            return self.paragraph_parser(p, limit, aff);
        };
        let switches = self
            .looking_at(&EXAMPLE, p)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_owned()));
        let sw = analyze_switches(switches.as_deref());
        let value = unescape_code(self.sub(cb, ce));
        let mut props = vec![
            ("begin", Self::pos(aff.0)),
            ("end", Self::pos(end)),
            ("value", Value::Str(value)),
            ("switches", Value::opt_str(switches)),
        ];
        props.extend(sw);
        props.push(("post-blank", Self::pos(self.count_lines(bb, end))));
        props.push(("post-affiliated", Self::pos(p)));
        self.el(Type::ExampleBlock, props, aff)
    }

    fn export_block_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let Some((cb, ce, bb, end)) = self.value_block(p, limit, "EXPORT") else {
            return self.paragraph_parser(p, limit, aff);
        };
        let backend = self
            .looking_at(&EXPORT, p)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_uppercase()));
        let value = unescape_code(self.sub(cb, ce));
        self.el(
            Type::ExportBlock,
            vec![
                ("type", Value::opt_str(backend)),
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("value", Value::Str(value)),
                ("post-blank", Self::pos(self.count_lines(bb, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn fixed_width_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let mut l = p;
        while l < limit && {
            let t = self.s[l..].trim_start_matches([' ', '\t']);
            t.starts_with(':') && matches!(t.as_bytes().get(1), None | Some(b' ') | Some(b'\n'))
        } {
            l = self.next_line(l);
        }
        let before_blank = l;
        let end_area = if self.at_bol(l) && l > 0 { l - 1 } else { l };
        let end_area = end_area.max(p);
        let end = self.end_after_blank(before_blank, limit);
        let value: Vec<String> = self
            .sub(p, end_area)
            .split('\n')
            .map(|line| {
                let t = line.trim_start_matches([' ', '\t']);
                let t = t.strip_prefix(':').unwrap_or(t);
                t.strip_prefix(' ').unwrap_or(t).to_owned()
            })
            .collect();
        self.el(
            Type::FixedWidth,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("value", Value::Str(value.join("\n"))),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn horizontal_rule_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let post = self.next_line(p);
        let end = self.end_after_blank(post, limit);
        self.el(
            Type::HorizontalRule,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("post-blank", Self::pos(self.count_lines(post, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn keyword_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let begin = if aff.1.is_empty() && aff.0 != p {
            p
        } else {
            aff.0
        };
        let c = self.looking_at(&KEYWORD, p);
        let key = c.as_ref().map(|c| c[1].to_uppercase()).unwrap_or_default();
        let vstart = c.as_ref().map_or(p, |c| p + c.get(0).unwrap().end());
        let value = trim(&self.s[vstart..self.eol(p).max(vstart)]).to_owned();
        let bb = self.next_line(p);
        let end = self.end_after_blank(bb, limit);
        self.el(
            Type::Keyword,
            vec![
                ("key", Value::Str(key)),
                ("value", Value::Str(value)),
                ("begin", Self::pos(begin)),
                ("end", Self::pos(end)),
                ("post-blank", Self::pos(self.count_lines(bb, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn latex_environment_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let name = self
            .looking_at(&LATEX_BEGIN, p)
            .map(|c| c[1].to_owned())
            .unwrap_or_default();
        let target = format!("\\end{{{name}}}");
        let found = self.find_line(p, limit, |l| {
            ends_with_ci(l.trim_end_matches([' ', '\t']), &target)
        });
        let Some(l) = found else {
            return self.paragraph_parser(p, limit, aff);
        };
        let code_end = self.next_line(l);
        let value = self.s[p..code_end].to_owned();
        let end = self.end_after_blank(code_end, limit);
        self.el(
            Type::LatexEnvironment,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("value", Value::Str(value)),
                ("post-blank", Self::pos(self.count_lines(code_end, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn node_property_parser(&mut self, p: usize) -> NodeId {
        let c = self.looking_at(&PROPERTY, p);
        let key = c.as_ref().map(|c| c[3].to_owned()).unwrap_or_default();
        let value = c
            .as_ref()
            .map(|c| c.get(5).map_or("", |m| m.as_str()).to_owned());
        let mend = c
            .as_ref()
            .map_or(self.eol(p), |c| p + c.get(0).unwrap().end());
        let end = self.len().min(mend + 1);
        self.t.create(
            Type::NodeProperty,
            vec![
                ("key", Value::Str(key)),
                ("value", Value::opt_str(value)),
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                ("post-blank", Self::pos(0)),
                ("post-affiliated", Self::pos(p)),
            ],
            vec![],
        )
    }

    pub fn paragraph_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let cb = p;
        let mut q = self.eol(p);
        let before_blank = loop {
            let found = self
                .cx
                .paragraph_separate
                .find_at(&self.s[..limit], q)
                .map(|m| self.bol(m.start()));
            let Some(l) = found else {
                break limit;
            };
            if self.paragraph_ends_at(l, limit) {
                break l;
            }
            q = self.eol(l);
            if q >= limit {
                break limit;
            }
        };
        let before_blank = if before_blank == limit {
            limit
        } else {
            self.bol(before_blank)
        };
        let ce = self.next_line(self.skip_back(before_blank, WS, cb));
        let ce = ce.min(self.len());
        let end = self.end_after_blank(before_blank, limit);
        self.el(
            Type::Paragraph,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("contents-begin", Self::pos(cb)),
                ("contents-end", Self::pos(ce)),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(cb)),
            ],
            aff,
        )
    }

    /// Whether a paragraph-separate match at line L really ends the
    /// paragraph (drawers, blocks, environments must be closed; dual
    /// keywords).
    fn paragraph_ends_at(&self, l: usize, limit: usize) -> bool {
        let line = &self.s[l..self.eol(l)];
        let t = line.trim_start_matches([' ', '\t']);
        if self.looking_at_p(&DRAWER, l) {
            return self
                .find_line(self.next_line(l), limit, |x| block_end_line(x, ":END:"))
                .is_some();
        }
        if t.len() > 8 && starts_with_ci(t, "#+BEGIN_") {
            let name: String = t[8..].chars().take_while(|c| !c.is_whitespace()).collect();
            if !name.is_empty() {
                let target = format!("#+END_{name}");
                return self
                    .find_line(l, limit, |x| block_end_line(x, &target))
                    .is_some();
            }
        }
        if let Some(c) = self.looking_at(&LATEX_BEGIN, l) {
            let target = format!("\\end{{{}}}", &c[1]);
            return self
                .find_line(l, limit, |x| {
                    ends_with_ci(x.trim_end_matches([' ', '\t']), &target)
                })
                .is_some();
        }
        if let Some(c) = DUAL_KW.captures(t) {
            return ["CAPTION", "RESULTS"]
                .iter()
                .any(|k| k.eq_ignore_ascii_case(&c[1]));
        }
        true
    }

    fn planning_parser(&mut self, p: usize, limit: usize) -> NodeId {
        let before_blank = self.next_line(p);
        let mut q = self.skip_fwd(before_blank, WS, limit);
        q = self.skip_back(q, BLANK, 0);
        if !self.at_bol(q) {
            q = self.skip_fwd(q, BLANK, self.len());
        }
        let end = q;
        let post_blank = self.count_lines(before_blank, end);
        let mut found: Vec<(&str, NodeId)> = vec![];
        let mut r = p;
        let line_end = self.eol(p);
        while let Some((k, after)) = self.next_planning_keyword(r, end.min(line_end.max(end))) {
            let a = self.skip_fwd(after, BLANK, end);
            let key = match k {
                "CLOSED:" => "closed",
                "DEADLINE:" => "deadline",
                _ => "scheduled",
            };
            found.retain(|(x, _)| *x != key);
            match self.timestamp_at(a) {
                Some((id, e)) => {
                    found.push((key, id));
                    r = e.max(after);
                }
                None => r = after,
            }
        }
        let id = self.t.create(
            Type::Planning,
            vec![
                ("closed", Value::Nil),
                ("deadline", Value::Nil),
                ("scheduled", Value::Nil),
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                ("post-blank", Self::pos(post_blank)),
                ("post-affiliated", Self::pos(p)),
            ],
            vec![],
        );
        for (k, ts) in found {
            self.t.put(id, k, Value::Node(ts));
            self.t.set_parent(ts, Some(id));
        }
        id
    }

    fn src_block_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let Some((_, ce, bb, end)) = self.value_block(p, limit, "SRC") else {
            return self.paragraph_parser(p, limit, aff);
        };
        let c = self.looking_at(&SRC, p);
        let g = |i: usize| {
            c.as_ref()
                .and_then(|c| c.get(i))
                .map(|m| m.as_str().to_owned())
        };
        let language = g(1);
        let switches = g(2);
        let params = g(3);
        let sw = analyze_switches(switches.as_deref());
        let value = unescape_code(self.sub(self.next_line(p), ce));
        let mut props = vec![
            ("language", Value::opt_str(language)),
            (
                "switches",
                Value::opt_str(nw(switches.as_deref()).map(trim)),
            ),
            (
                "parameters",
                Value::opt_str(nw(params.as_deref()).map(trim)),
            ),
            ("begin", Self::pos(aff.0)),
            ("end", Self::pos(end)),
        ];
        props.extend(sw);
        props.push(("value", Value::Str(value)));
        props.push(("post-blank", Self::pos(self.count_lines(bb, end))));
        props.push(("post-affiliated", Self::pos(p)));
        self.el(Type::SrcBlock, props, aff)
    }

    fn table_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let org = self.s[p..].trim_start_matches([' ', '\t']).starts_with('|');
        // end-re: a line that is blank or starts with something else
        // than | (and + for table.el).
        let mut l = p;
        let table_end = loop {
            if l >= limit {
                break limit;
            }
            let t = self.s[l..self.eol(l)].trim_start_matches([' ', '\t']);
            let ok = t.starts_with('|') || (!org && t.starts_with('+'));
            if !ok {
                break l;
            }
            let n = self.next_line(l);
            if n == l {
                break limit;
            }
            l = n;
        };
        let table_end = table_end.min(limit);
        let mut tblfm = vec![];
        let mut q = table_end;
        while let Some(c) = TBLFM.captures(&self.s[q..]) {
            tblfm.push(Value::Str(c[1].to_owned()));
            q = self.next_line(q);
            if q >= self.len() {
                break;
            }
        }
        tblfm.reverse();
        let before_blank = q;
        let end = self.end_after_blank(before_blank, limit);
        self.el(
            Type::Table,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("type", Value::sym(if org { "org" } else { "table.el" })),
                ("tblfm", Value::List(tblfm)),
                (
                    "contents-begin",
                    if org { Self::pos(p) } else { Value::Nil },
                ),
                (
                    "contents-end",
                    if org {
                        Self::pos(table_end)
                    } else {
                        Value::Nil
                    },
                ),
                (
                    "value",
                    if org {
                        Value::Nil
                    } else {
                        Value::Str(self.s[p..table_end].to_owned())
                    },
                ),
                ("post-blank", Self::pos(self.count_lines(before_blank, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    fn table_row_parser(&mut self, p: usize) -> NodeId {
        let t = self.s[p..].trim_start_matches([' ', '\t']);
        let rule = t.starts_with("|-");
        let (cb, ce) = if rule {
            (None, None)
        } else {
            let bar = self.s[p..].find('|').map(|i| p + i + 1);
            let e = self.skip_back(self.eol(p), BLANK, 0);
            (bar, bar.map(|b| e.max(b)))
        };
        let end = self.next_line(p);
        self.t.create(
            Type::TableRow,
            vec![
                ("type", Value::sym(if rule { "rule" } else { "standard" })),
                ("begin", Self::pos(p)),
                ("end", Self::pos(end)),
                ("contents-begin", Value::opt_int(cb)),
                ("contents-end", Value::opt_int(ce)),
                ("post-blank", Self::pos(0)),
                ("post-affiliated", Self::pos(p)),
            ],
            vec![],
        )
    }

    fn verse_block_parser(&mut self, p: usize, limit: usize, aff: Aff) -> NodeId {
        let Some((cb, ce, bb, end)) = self.value_block(p, limit, "VERSE") else {
            return self.paragraph_parser(p, limit, aff);
        };
        self.el(
            Type::VerseBlock,
            vec![
                ("begin", Self::pos(aff.0)),
                ("end", Self::pos(end)),
                ("contents-begin", Self::pos(cb)),
                ("contents-end", Self::pos(ce)),
                ("post-blank", Self::pos(self.count_lines(bb, end))),
                ("post-affiliated", Self::pos(p)),
            ],
            aff,
        )
    }

    // ---- org-data ----

    /// org-element-org-data-parser plus its global node properties.
    pub fn org_data(&mut self) -> NodeId {
        let cb = self.bol(self.skip_fwd(0, WS, self.len()));
        let id = self.t.create(
            Type::OrgData,
            vec![
                ("begin", Self::pos(0)),
                ("contents-begin", Self::pos(cb)),
                ("contents-end", Self::pos(self.len())),
                ("end", Self::pos(self.len())),
                ("pre-blank", Self::pos(self.count_lines(0, cb))),
                ("post-blank", Self::pos(0)),
                ("post-affiliated", Self::pos(0)),
            ],
            vec![],
        );
        let mut l = cb;
        while l < self.len() && self.comment_line(l) {
            l = self.next_line(l);
        }
        let mut has_category = false;
        if l < self.len() {
            for (k, v) in self.node_properties(l) {
                has_category |= k == "CATEGORY";
                self.t.put(id, &k, v);
            }
        }
        if !has_category && let Some(c) = &self.cx.category {
            self.t.put(id, "CATEGORY", c.clone());
        }
        id
    }
}

re!(TBLFM, r"(?im)\A[ \t]*#\+TBLFM: +(.*)[ \t]*$");
re!(DUAL_KW, r"\A#\+(\S+)\[.*\]:");
re!(CLOCK_PREFIX, r"(?i)\A[ \t]*CLOCK:");

/// Affiliated keywords: (begin, keyword properties).
pub type Aff = (usize, Vec<(String, Value)>);

fn starts_with_ci(s: &str, k: &str) -> bool {
    s.len() >= k.len() && s.as_bytes()[..k.len()].eq_ignore_ascii_case(k.as_bytes())
}

fn ends_with_ci(s: &str, k: &str) -> bool {
    s.len() >= k.len() && s.as_bytes()[s.len() - k.len()..].eq_ignore_ascii_case(k.as_bytes())
}

/// `^[ \t]*TARGET[ \t]*$`, case-insensitively.
fn block_end_line(l: &str, target: &str) -> bool {
    l.trim_matches([' ', '\t']).eq_ignore_ascii_case(target)
}

/// org-split-string with a one-character separator.
pub fn split_string(s: &str, sep: char) -> Vec<String> {
    let s = s.strip_prefix(sep).unwrap_or(s);
    let s = s.strip_suffix(sep).unwrap_or(s);
    s.split(sep).map(str::to_owned).collect()
}

/// org-element--parse-paired-brackets: the contents between the OPEN
/// bracket at P and its match before LIMIT, and the position after it.
pub fn paired(s: &str, p: usize, open: u8, limit: usize) -> Option<(&str, usize)> {
    let close = match open {
        b'[' => b']',
        b'(' => b')',
        b'{' => b'}',
        _ => return None,
    };
    let b = s.as_bytes();
    if b.get(p) != Some(&open) {
        return None;
    }
    let mut depth = 0;
    for (i, &c) in b[p..limit].iter().enumerate() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some((&s[p + 1..p + i], p + i + 1));
            }
        }
    }
    None
}

/// The switch analysis shared by example and src blocks.
fn analyze_switches(sw: Option<&str>) -> Vec<(&'static str, Value)> {
    let number_lines = sw.and_then(|s| NUMBER_LINES.captures(s)).map(|c| {
        let kind = if &c[1] == "-" { "new" } else { "continued" };
        let n = c
            .get(2)
            .map_or(0, |m| m.as_str().parse::<i64>().unwrap_or(1) - 1);
        Value::cons(Value::sym(kind), Value::Int(n))
    });
    let preserve = sw.is_some_and(|s| SW_I.is_match(s));
    let retain = match sw {
        None => true,
        Some(s) => !SW_R.is_match(s) || (number_lines.is_some() && SW_K.is_match(s)),
    };
    let use_labels = match sw {
        None => true,
        Some(s) => retain && !SW_K.is_match(s),
    };
    let label_fmt = sw
        .and_then(|s| LABEL_FMT.captures(s))
        .map(|c| c[1].to_owned());
    vec![
        ("number-lines", number_lines.unwrap_or(Value::Nil)),
        ("preserve-indent", Value::from(preserve)),
        ("retain-labels", Value::from(retain)),
        ("use-labels", Value::from(use_labels)),
        ("label-fmt", Value::opt_str(label_fmt)),
    ]
}

re!(UNESCAPE, r"(?m)^([ \t]*,*)(,)(\*|#\+)");
re!(ESCAPE, r"(?m)^([ \t]*)(,*(?:\*|#\+))");

/// org-unescape-code-in-string.
pub fn unescape_code(s: &str) -> String {
    UNESCAPE.replace_all(s, "$1$3").into_owned()
}

/// org-escape-code-in-string.
pub fn escape_code(s: &str) -> String {
    ESCAPE.replace_all(s, "$1,$2").into_owned()
}

/// The :structure value of a list.
fn struct_value(st: &[ListItem]) -> Value {
    Value::List(
        st.iter()
            .map(|i| {
                Value::List(vec![
                    Value::Int(i.pos as i64),
                    Value::Int(i.ind as i64),
                    Value::Str(i.bullet.clone()),
                    Value::opt_str(i.counter.clone()),
                    Value::opt_str(i.checkbox.clone()),
                    Value::opt_str(i.tag.clone()),
                    Value::Int(i.end as i64),
                ])
            })
            .collect(),
    )
}

impl<'a> P<'a> {
    pub fn opts(&mut self, o: &Opts) {
        self.invisible = o.invisible.clone();
    }
}
