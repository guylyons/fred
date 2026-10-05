//! org-list.el's structures: org-list-struct, the prevs/parents alists,
//! accessors, methods on structures and their repair and application to
//! the buffer (org-list-write-struct).

use super::buf::{Buf, Caps, Fold};
use crate::org::options;
use crate::org::sexp::{self, Sexp};
use regex::Regex;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

// ---- options ----

/// org-plain-list-ordered-item-terminator: None for both `.` and `)`.
pub fn terminator() -> Option<char> {
    match options::get("org-plain-list-ordered-item-terminator") {
        Some(toml::Value::Integer(41)) => Some(')'),
        Some(toml::Value::Integer(46)) => Some('.'),
        Some(toml::Value::String(s)) if s == ")" || s == "?)" || s == "?\\)" => Some(')'),
        Some(toml::Value::String(s)) if s == "." || s == "?." => Some('.'),
        _ => None,
    }
}

pub fn allow_alpha() -> bool {
    options::bool("org-list-allow-alphabetical", false)
}

pub fn indent_offset() -> usize {
    options::int("org-list-indent-offset", 0).max(0) as usize
}

pub fn circular() -> bool {
    options::bool("org-list-use-circular-motion", false)
}

/// An alist option's value for `key` (symbols and strings alike).
pub fn alist_get(opt: &Sexp, key: &str) -> Option<Sexp> {
    opt.list()?
        .iter()
        .find(|e| e.car().and_then(Sexp::str) == Some(key))
        .map(Sexp::cdr)
}

/// org-list-automatic-rules entry (default t).
pub fn rule(name: &str) -> bool {
    match sexp::option("org-list-automatic-rules") {
        Some(v) => alist_get(&v, name).is_some_and(|x| x.truthy()),
        None => true,
    }
}

/// org-get-alist-option.
pub fn alist_option(name: &str, key: &str, default: Sexp) -> Sexp {
    let v = sexp::option(name).unwrap_or(default);
    if v == Sexp::T || v.is_nil() || v.list().is_none() {
        return v;
    }
    alist_get(&v, key)
        .or_else(|| alist_get(&v, "default"))
        .unwrap_or(Sexp::Nil)
}

/// org-list-demote-modify-bullet as (from . to) pairs.
pub fn demote_bullets() -> Vec<(String, String)> {
    let Some(v) = sexp::option("org-list-demote-modify-bullet") else {
        return vec![];
    };
    v.list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let k = e.car()?.str()?.to_owned();
            let cdr = e.cdr();
            let to = cdr
                .str()
                .map(str::to_owned)
                .or_else(|| cdr.car().and_then(Sexp::str).map(str::to_owned))?;
            Some((k, to))
        })
        .collect()
}

/// org-list-two-spaces-after-bullet-regexp.
fn two_spaces() -> Option<Regex> {
    let s = options::string("org-list-two-spaces-after-bullet-regexp", "nil");
    if s == "nil" || s.is_empty() {
        return None;
    }
    Regex::new(&emacs_re(&s)).ok()
}

/// An Emacs regexp in Rust syntax (the common constructs).
pub fn emacs_re(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some(c @ ('(' | ')' | '|' | '{' | '}')) => {
                    if c == '(' && it.peek() == Some(&'?') {
                        it.next();
                        if it.peek() == Some(&':') {
                            it.next();
                        }
                        out.push_str("(?:");
                    } else {
                        out.push(c);
                    }
                }
                Some('`') => out.push_str(r"\A"),
                Some('\'') => out.push_str(r"\z"),
                Some('<' | '>' | 'b') => out.push_str(r"\b"),
                Some('s') => {
                    it.next();
                    out.push_str(r"\s");
                }
                Some('S') => {
                    it.next();
                    out.push_str(r"\S");
                }
                Some('w') => out.push_str(r"\w"),
                Some('W') => out.push_str(r"\W"),
                Some(c) => {
                    out.push('\\');
                    out.push(c);
                }
                None => out.push_str(r"\\"),
            },
            '(' | ')' | '|' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

// ---- regexps ----

pub fn re(s: &str) -> Regex {
    Regex::new(s).expect("list regexp")
}

macro_rules! lazy_re {
    ($name:ident, $s:expr) => {
        pub static $name: LazyLock<Regex> = LazyLock::new(|| re($s));
    };
}

lazy_re!(
    FULL_ITEM,
    r"\A(?m:[ \t]*((?:[-+*]|(?:[0-9]+|[A-Za-z])[.)])(?:[ \t]+|$))(?:\[@(?:start:)?([0-9]+|[A-Za-z])\][ \t]*)?(?:(\[[ X-]\])(?:[ \t]+|$))?(?:(.*)[ \t]+::(?:[ \t]+|$))?)"
);
lazy_re!(LIST_END, r"\A[ \t]*\n[ \t]*\n");
lazy_re!(LIST_END_ANY, r"(?m)^[ \t]*\n[ \t]*\n");
lazy_re!(BLANK, r"\A(?m:[ \t]*$)");
lazy_re!(BLANK_ANY, r"(?m)^[ \t]*$");
lazy_re!(BLOCK_END, r"\A(?i:[ \t]*#\+end_)");
lazy_re!(BLOCK_BEGIN, r"\A(?i:[ \t]*#\+begin_)");
lazy_re!(BLOCK_END_ANY, r"(?mi)^[ \t]*#\+end_");
lazy_re!(BLOCK_EDGE, r"\A(?i:[ \t]*#\+(begin|end)_)");
lazy_re!(BLOCK_EDGE_ANY, r"(?mi)^[ \t]*#\+(begin|end)_");
lazy_re!(BLOCK_TYPE, r"\A(?i:[ \t]*#\+begin_(\S+))");
lazy_re!(DRAWER, r"\A(?m:[ \t]*:((?:\w|[-_])+):[ \t]*$)");
lazy_re!(DRAWER_END, r"\A(?i:[ \t]*:END:)");
lazy_re!(DRAWER_END_ANY, r"(?mi)^[ \t]*:END:");
lazy_re!(DRAWER_END_LINE_ANY, r"(?mi)^[ \t]*:END:[ \t]*$");
lazy_re!(NONBLANK, r"\A[ \t]*\S");
lazy_re!(LEADING_WS, r"\A[ \t]+");
lazy_re!(BOX_ANYWHERE, r"\A.*?([ \t]*\[[ X-]\])");

type ItemRes = Rc<(Regex, Regex)>;
type ItemReCache = Option<((bool, Option<char>), ItemRes)>;

thread_local! {
    static ITEM_RE: RefCell<ItemReCache> = const { RefCell::new(None) };
}

/// org-item-re (anchored) and org-item-beginning-re (searching).
pub fn item_res() -> ItemRes {
    let key = (allow_alpha(), terminator());
    ITEM_RE.with(|c| {
        if let Some((k, r)) = &*c.borrow()
            && *k == key
        {
            return Rc::clone(r);
        }
        let term = match key.1 {
            Some(')') => r"\)",
            Some(_) => r"\.",
            None => r"[.)]",
        };
        let alpha = if key.0 { "|[A-Za-z]" } else { "" };
        let body = format!(r"(?:[ \t]*(?:[-+]|(?:(?:[0-9]+{alpha}){term}))|[ \t]+\*)(?:[ \t]+|$)");
        let r = Rc::new((re(&format!(r"\A(?m:{body})")), re(&format!(r"(?m)^{body}"))));
        *c.borrow_mut() = Some((key, Rc::clone(&r)));
        r
    })
}

/// Looking at org-item-re (from a line beginning).
pub fn looking_at_item(b: &Buf) -> Option<Caps> {
    b.looking_at(&item_res().0)
}

// ---- context ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtxType {
    None,
    Drawer,
    Block,
    Invalid,
}

/// org-list-forbidden-blocks.
const FORBIDDEN: &[&str] = &["example", "verse", "src", "export"];

/// The line of an Org heading at `p`.
pub fn heading_at(b: &Buf, p: usize) -> bool {
    crate::org::syntax::level(b.sub(b.bol_at(p), b.eol_at(p))).is_some()
}

/// org-back-to-heading: the heading line at or before `p`.
pub fn heading_before(b: &Buf, p: usize) -> Option<usize> {
    let mut ls = b.bol_at(p);
    loop {
        if heading_at(b, ls) {
            return Some(ls);
        }
        if ls == 0 {
            return None;
        }
        ls = b.bol_at(ls - 1);
    }
}

/// outline-next-heading from `p`: the next heading line after `p`'s line.
pub fn next_heading(b: &Buf, p: usize) -> Option<usize> {
    let mut ls = b.eol_at(p);
    while ls < b.len() {
        ls += 1;
        if heading_at(b, ls) {
            return Some(ls);
        }
        ls = b.eol_at(ls);
    }
    None
}

/// org-list-context at point's line: (MIN MAX TYPE).
pub fn context(b: &mut Buf) -> (usize, usize, CtxType) {
    b.excursion(|b| {
        b.forward_line(0);
        let pos = b.pt;
        let mut lim_up = heading_before(b, pos).unwrap_or(0);
        let mut lim_down = next_heading(b, pos).unwrap_or(b.len());
        let mut ty = CtxType::None;
        // Inside a drawer?
        if !b.looking_p(&DRAWER) && !b.looking_p(&DRAWER_END) {
            let r = b.excursion(|b| {
                b.re_search_backward(&DRAWER, lim_up)?;
                let beg = b.eol() + 1;
                let end = match b.re_search_forward(&DRAWER_END_ANY, lim_down) {
                    Some(c) => c.beg(0)?.saturating_sub(1),
                    None => lim_down,
                };
                (end >= pos).then_some((beg, end))
            });
            if let Some((beg, end)) = r {
                (lim_up, lim_down, ty) = (beg, end, CtxType::Drawer);
            }
        }
        // Strictly inside a block?
        if !b.looking_p(&BLOCK_EDGE) {
            let r = b.excursion(|b| {
                b.re_search_backward(&BLOCK_EDGE, lim_up)?;
                let beg = b.eol() + 1;
                let ty = b.looking_at(&BLOCK_TYPE)?;
                let ty = ty.str(b, 1)?.to_ascii_lowercase();
                b.goto(beg);
                let c = b.re_search_forward(&BLOCK_EDGE_ANY, lim_down)?;
                let end = b.bol().saturating_sub(1);
                (end >= pos && c.str(b, 1)?.eq_ignore_ascii_case("end")).then_some((beg, end, ty))
            });
            if let Some((beg, end, t)) = r {
                lim_up = beg;
                lim_down = end;
                ty = if FORBIDDEN.contains(&t.as_str()) {
                    CtxType::Invalid
                } else {
                    CtxType::Block
                };
            }
        }
        (lim_up, lim_down, ty)
    })
}

/// org-list-in-valid-context-p.
pub fn valid_context(b: &mut Buf) -> bool {
    context(b).2 != CtxType::Invalid
}

/// org-at-item-p.
pub fn at_item(b: &mut Buf) -> bool {
    let bol = b.bol();
    b.looking_at_pos(&item_res().0, bol).is_some() && valid_context(b)
}

/// org-in-regexp: a match of `re` around point within `nlines`.
fn in_regexp(b: &Buf, re: &Regex, nlines: i64) -> Option<(usize, usize)> {
    let pos = b.pt;
    let start = b.lbp(1 - nlines);
    let eol = b.eol_at(b.lbp(nlines + 1));
    let mut from = start;
    while from <= eol {
        let m = re.find_at(&b.s[..eol.min(b.len())], from)?;
        if m.start() > pos {
            return None;
        }
        if m.end() >= pos {
            return Some((m.start(), m.end()));
        }
        from = m.end().max(from + 1);
    }
    None
}

/// org-in-item-p: the beginning of the item point is in.
pub fn in_item(b: &mut Buf) -> Option<usize> {
    b.excursion(|b| {
        b.forward_line(0);
        let (lim_up, _, ty) = context(b);
        let item_re = item_res();
        let mut ind_ref = if b.looking_p(&BLANK) { 10000 } else { b.ind() };
        if ty == CtxType::Invalid {
            return None;
        }
        if b.looking_p(&item_re.0) {
            return Some(b.pt);
        }
        if let Some((s, e)) = in_regexp(b, &LIST_END_ANY, 2)
            && b.pt >= s
            && b.pt < e
        {
            b.goto(s);
            b.forward_line(-1);
        }
        loop {
            let ind = b.ind();
            if b.looking_p(&item_re.0) && ind < ind_ref {
                return Some(b.pt);
            }
            if b.pt <= lim_up || b.looking_p(&LIST_END) {
                return None;
            }
            if b.looking_p(&BLOCK_END) && b.re_search_backward(&BLOCK_BEGIN, lim_up).is_some() {
                continue;
            }
            if b.looking_p(&DRAWER_END) && b.re_search_backward(&DRAWER, lim_up).is_some() {
                b.forward_line(0);
                continue;
            }
            if b.looking_p(&BLANK) {
                b.forward_line(-1);
            } else if ind == 0 {
                return None;
            } else {
                if ind < ind_ref {
                    ind_ref = ind;
                }
                b.forward_line(-1);
            }
        }
    })
}

// ---- the structure ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub pos: usize,
    pub ind: usize,
    /// Bullet with trailing whitespace.
    pub bullet: String,
    pub counter: Option<String>,
    pub checkbox: Option<String>,
    /// Description tag (unordered items only).
    pub tag: Option<String>,
    pub end: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Struct(pub Vec<Item>);

/// An alist from item to previous item or parent.
pub type Alist = Vec<(usize, Option<usize>)>;

pub fn assq(a: &Alist, k: usize) -> Option<usize> {
    a.iter().find(|e| e.0 == k).and_then(|e| e.1)
}

fn rassq(a: &Alist, v: usize) -> Option<usize> {
    a.iter().find(|e| e.1 == Some(v)).map(|e| e.0)
}

impl Struct {
    pub fn get(&self, pos: usize) -> Option<&Item> {
        self.0.iter().find(|i| i.pos == pos)
    }
    pub fn item(&self, pos: usize) -> &Item {
        self.get(pos).expect("item in structure")
    }
    pub fn item_mut(&mut self, pos: usize) -> &mut Item {
        self.0
            .iter_mut()
            .find(|i| i.pos == pos)
            .expect("item in structure")
    }
    pub fn ind(&self, pos: usize) -> usize {
        self.item(pos).ind
    }
    pub fn bullet(&self, pos: usize) -> &str {
        &self.item(pos).bullet
    }
    pub fn checkbox(&self, pos: usize) -> Option<&str> {
        self.get(pos).and_then(|i| i.checkbox.as_deref())
    }
    pub fn end(&self, pos: usize) -> usize {
        self.item(pos).end
    }
    pub fn positions(&self) -> Vec<usize> {
        self.0.iter().map(|i| i.pos).collect()
    }
    pub fn sort(&mut self) {
        self.0.sort_by_key(|i| i.pos);
    }

    /// org-list-get-top-point.
    pub fn top(&self) -> usize {
        self.0[0].pos
    }
    /// org-list-get-bottom-point.
    pub fn bottom(&self) -> usize {
        self.0.iter().map(|i| i.end).max().unwrap_or(0)
    }

    /// org-list-prevs-alist.
    pub fn prevs(&self) -> Alist {
        self.0
            .iter()
            .map(|e| (e.pos, self.0.iter().find(|i| i.end == e.pos).map(|i| i.pos)))
            .collect()
    }

    /// org-list-parents-alist.
    pub fn parents(&self) -> Alist {
        let mut ind_to_ori: Vec<(usize, Option<usize>)> = vec![(self.0[0].ind, None)];
        let top = self.top();
        let mut prev_pos = vec![top];
        let mut out = vec![(top, None)];
        for it in &self.0[1..] {
            let (pos, ind) = (it.pos, it.ind);
            let prev_ind = ind_to_ori[0].0;
            prev_pos.insert(0, pos);
            let parent = if prev_ind > ind {
                ind_to_ori = if let Some(i) = ind_to_ori.iter().position(|e| e.0 == ind) {
                    ind_to_ori[i..].to_vec()
                } else if let Some(i) = ind_to_ori.iter().position(|e| e.0 < ind) {
                    ind_to_ori[i..].to_vec()
                } else {
                    vec![(ind, None)]
                };
                ind_to_ori[0].1
            } else if prev_ind < ind {
                let origin = prev_pos[1];
                ind_to_ori.insert(0, (ind, Some(origin)));
                Some(origin)
            } else {
                ind_to_ori[0].1
            };
            out.push((pos, parent));
        }
        out
    }

    /// org-list-has-child-p.
    pub fn has_child(&self, item: usize) -> Option<usize> {
        let i = self.0.iter().position(|e| e.pos == item)?;
        let c = self.0.get(i + 1)?;
        (self.0[i].ind < c.ind).then_some(c.pos)
    }

    /// org-list-get-subtree.
    pub fn subtree(&self, item: usize) -> Vec<usize> {
        let Some(i) = self.0.iter().position(|e| e.pos == item) else {
            return vec![];
        };
        let end = self.0[i].end;
        self.0[i + 1..]
            .iter()
            .take_while(|e| e.pos < end)
            .map(|e| e.pos)
            .collect()
    }

    /// org-list-get-item-end-before-blank.
    pub fn end_before_blank(&self, b: &Buf, item: usize) -> usize {
        let mut p = self.end(item);
        while p > 0 && matches!(b.s.as_bytes()[p - 1], b' ' | b'\t' | b'\n' | b'\r') {
            p -= 1;
        }
        b.eol_at(p)
    }

    /// org-list-get-list-type.
    pub fn list_type(&self, item: usize, prevs: &Alist) -> ListType {
        let first = list_begin(item, prevs);
        let it = self.item(first);
        if it.bullet.chars().any(|c| c.is_alphanumeric()) {
            ListType::Ordered
        } else if it.tag.is_some() {
            ListType::Descriptive
        } else {
            ListType::Unordered
        }
    }

    /// org-list-get-item-number.
    pub fn item_number(&self, item: usize, prevs: &Alist, parents: &Alist) -> Vec<i64> {
        let rel = |item: usize| {
            let mut seq = 0;
            let mut pos = Some(item);
            let mut counter = None;
            while let Some(p) = pos {
                counter = self.item(p).counter.clone();
                if counter.is_some() {
                    break;
                }
                pos = prev_item(p, prevs);
                if pos.is_some() {
                    seq += 1;
                }
            }
            match counter {
                None => seq + 1,
                Some(c) => {
                    if let Some(ch) = c.chars().find(char::is_ascii_alphabetic) {
                        ch.to_ascii_uppercase() as i64 - 64 + seq
                    } else if let Ok(n) = c.trim().parse::<i64>() {
                        n + seq
                    } else {
                        seq + 1
                    }
                }
            }
        };
        let mut out = vec![rel(item)];
        let mut p = item;
        while let Some(parent) = assq(parents, p) {
            out.insert(0, rel(parent));
            p = parent;
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListType {
    Ordered,
    Unordered,
    Descriptive,
}

pub fn next_item(item: usize, prevs: &Alist) -> Option<usize> {
    rassq(prevs, item)
}

pub fn prev_item(item: usize, prevs: &Alist) -> Option<usize> {
    assq(prevs, item)
}

pub fn parent(item: usize, parents: &Alist) -> Option<usize> {
    assq(parents, item)
}

/// org-list-get-all-items.
pub fn all_items(item: usize, prevs: &Alist) -> Vec<usize> {
    let mut before = vec![];
    let mut p = item;
    while let Some(x) = prev_item(p, prevs) {
        before.insert(0, x);
        p = x;
    }
    before.push(item);
    let mut p = item;
    while let Some(x) = next_item(p, prevs) {
        before.push(x);
        p = x;
    }
    before
}

/// org-list-get-children.
pub fn children(item: usize, parents: &Alist) -> Vec<usize> {
    parents
        .iter()
        .filter(|e| e.1 == Some(item))
        .map(|e| e.0)
        .collect()
}

/// org-list-get-list-begin (org-list-get-first-item).
pub fn list_begin(item: usize, prevs: &Alist) -> usize {
    let mut p = item;
    while let Some(x) = prev_item(p, prevs) {
        p = x;
    }
    p
}

/// org-list-get-last-item.
pub fn last_item(item: usize, prevs: &Alist) -> usize {
    let mut p = item;
    while let Some(x) = next_item(p, prevs) {
        p = x;
    }
    p
}

/// org-list-get-list-end.
pub fn list_end(st: &Struct, item: usize, prevs: &Alist) -> usize {
    st.end(last_item(item, prevs))
}

/// The item record at point's line (assoc-at-point).
fn assoc_at(b: &Buf, ind: usize) -> Item {
    let c = b.looking_at(&FULL_ITEM).unwrap_or_default();
    let bullet = c.str(b, 1).unwrap_or("").to_owned();
    let unordered = bullet.contains(['-', '+', '*']);
    Item {
        pos: b.pt,
        ind,
        tag: if unordered {
            c.str(b, 4).map(str::to_owned)
        } else {
            None
        },
        counter: c.str(b, 2).map(str::to_owned),
        checkbox: c.str(b, 3).map(str::to_owned),
        bullet,
        end: 0,
    }
}

/// org-list-struct: the list at point (point at an item).
pub fn list_struct(b: &mut Buf) -> Struct {
    b.excursion(|b| {
        b.forward_line(0);
        let (lim_up, lim_down, _) = context(b);
        let item_re = item_res();
        let start = b.pt;
        let mut text_min_ind = 10000;
        let mut beg_cell = (start, b.ind());
        let mut up_items: Vec<Item> = vec![];
        let mut up_ends: Vec<(usize, usize)> = vec![];
        let end_before_blank = |b: &mut Buf| {
            b.skip_backward(" \r\t\n");
            (b.eol() + 1).min(lim_down)
        };
        // 1. Upward.
        loop {
            let ind = b.ind();
            if b.pt <= lim_up {
                if b.looking_p(&item_re.0) {
                    beg_cell = (b.pt, ind);
                    up_items.push(assoc_at(b, ind));
                } else {
                    up_items.retain(|i| i.pos >= beg_cell.0);
                }
                break;
            }
            if b.looking_p(&LIST_END) {
                up_items.retain(|i| i.pos >= beg_cell.0);
                break;
            }
            if b.looking_p(&item_re.0) {
                up_items.push(assoc_at(b, ind));
                up_ends.push((ind, b.pt));
                if ind < text_min_ind {
                    beg_cell = (b.pt, ind);
                }
                b.forward_line(-1);
            } else if b.looking_p(&BLOCK_END)
                && b.re_search_backward(&BLOCK_BEGIN, lim_up).is_some()
            {
            } else if b.looking_p(&DRAWER_END) && b.re_search_backward(&DRAWER, lim_up).is_some() {
                b.forward_line(0);
            } else if b.looking_p(&BLANK) {
                b.forward_line(-1);
            } else if ind == 0 {
                up_items.retain(|i| i.pos >= beg_cell.0);
                break;
            } else {
                if ind < text_min_ind {
                    text_min_ind = ind;
                }
                up_ends.push((ind, b.pt));
                b.forward_line(-1);
            }
        }
        up_items.reverse();
        up_ends.reverse();
        // 2. Downward.
        b.goto(start);
        let mut down_items: Vec<Item> = vec![];
        let mut down_ends: Vec<(usize, usize)> = vec![];
        loop {
            let ind = b.ind();
            if b.pt >= lim_down {
                down_ends.push((0, end_before_blank(b)));
                break;
            }
            if b.looking_p(&LIST_END) {
                down_ends.push((0, b.pt));
                break;
            }
            if b.looking_p(&item_re.0) {
                down_items.push(assoc_at(b, ind));
                down_ends.push((ind, b.pt));
                b.forward_line(1);
            } else if b.looking_p(&BLANK) {
                b.forward_line(1);
            } else if ind <= beg_cell.1 {
                down_ends.push((0, end_before_blank(b)));
                break;
            } else {
                if down_items.last().is_some_and(|i| ind <= i.ind) {
                    down_ends.push((ind, b.pt));
                }
                if b.looking_p(&BLOCK_BEGIN) {
                    b.re_search_forward(&BLOCK_END_ANY, lim_down);
                } else if b.looking_p(&DRAWER) {
                    b.re_search_forward(&DRAWER_END_ANY, lim_down);
                }
                b.forward_line(1);
            }
        }
        let mut items = up_items;
        items.extend(down_items.into_iter().skip(1));
        let mut ends = up_ends;
        ends.extend(down_ends.into_iter().skip(1));
        let mut st = Struct(items);
        assoc_end(&mut st, &ends);
        st
    })
}

/// org-list-struct-assoc-end.
fn assoc_end(st: &mut Struct, ends: &[(usize, usize)]) {
    let mut k = 0;
    for it in &mut st.0 {
        while k < ends.len() && ends[k].1 <= it.pos {
            k += 1;
        }
        if let Some(e) = ends[k..].iter().find(|e| e.0 <= it.ind) {
            it.end = e.1;
        }
    }
}

// ---- methods ----

/// org-list-bullet-string.
pub fn bullet_string(bullet: &str) -> String {
    let spaces = if two_spaces().is_some_and(|r| r.is_match(bullet)) {
        "  "
    } else {
        " "
    };
    let t = bullet.trim_start_matches([' ', '\t']);
    let lead = &bullet[..bullet.len() - t.len()];
    let word: &str = t.split([' ', '\t']).next().unwrap_or("");
    if word.is_empty() {
        return bullet.to_owned();
    }
    let rest = &t[word.len()..];
    let rest = rest.trim_start_matches([' ', '\t']);
    format!("{lead}{word}{spaces}{rest}")
}

/// org-list-use-alpha-bul-p.
pub fn use_alpha(first: usize, st: &Struct, prevs: &Alist) -> bool {
    if !allow_alpha() {
        return false;
    }
    let mut ascii = 64u32;
    let mut item = Some(first);
    while let Some(i) = item {
        match st
            .item(i)
            .counter
            .as_deref()
            .and_then(|c| c.chars().find(char::is_ascii_alphabetic))
        {
            Some(c) => ascii = c.to_ascii_uppercase() as u32,
            None => ascii += 1,
        }
        if ascii > 90 {
            return false;
        }
        item = next_item(i, prevs);
    }
    true
}

/// Replace the first match of `re` in `s` with `with`.
fn replace_first(s: &str, re: &Regex, with: &str) -> String {
    match re.find(s) {
        Some(m) => format!("{}{with}{}", &s[..m.start()], &s[m.end()..]),
        None => s.to_owned(),
    }
}

lazy_re!(DIGITS, r"[0-9]+");
lazy_re!(ALPHA, r"[A-Za-z]");
lazy_re!(LOWER, r"[a-z]");
lazy_re!(UPPER, r"[A-Z]");
lazy_re!(NUM_OR_ALPHA, r"[0-9]+|[A-Za-z]");

/// org-list-inc-bullet-maybe.
pub fn inc_bullet(b: &str) -> String {
    if let Some(m) = DIGITS.find(b) {
        let n: u64 = m.as_str().parse().unwrap_or(0);
        return format!("{}{}{}", &b[..m.start()], n + 1, &b[m.end()..]);
    }
    if let Some(m) = ALPHA.find(b) {
        let c = (m.as_str().as_bytes()[0] + 1) as char;
        return format!("{}{c}{}", &b[..m.start()], &b[m.end()..]);
    }
    b.to_owned()
}

/// org-list-struct-fix-bul.
pub fn fix_bul(st: &mut Struct, prevs: &Alist) {
    for i in 0..st.0.len() {
        let item = st.0[i].pos;
        let prev = prev_item(item, prevs);
        let prev_bul = prev.map(|p| st.bullet(p).to_owned());
        let counter = st.0[i].counter.clone();
        let bullet = st.0[i].bullet.clone();
        let alphap = prev.is_none() && use_alpha(item, st, prevs);
        let has = |r: &Regex, s: &Option<String>| s.as_deref().is_some_and(|s| r.is_match(s));
        let new = if prev.is_some() && has(&ALPHA, &counter) && has(&ALPHA, &prev_bul) {
            let pb = prev_bul.unwrap();
            let c = counter.unwrap();
            if LOWER.is_match(&pb) {
                replace_first(&pb, &LOWER, &c.to_ascii_lowercase())
            } else {
                replace_first(&pb, &UPPER, &c.to_ascii_uppercase())
            }
        } else if prev.is_some() && has(&DIGITS, &counter) && has(&DIGITS, &prev_bul) {
            replace_first(
                prev_bul.as_deref().unwrap(),
                &DIGITS,
                counter.as_deref().unwrap(),
            )
        } else if let Some(p) = prev {
            inc_bullet(st.bullet(p))
        } else if let Some(c) = counter
            .as_deref()
            .filter(|c| ALPHA.is_match(c) && ALPHA.is_match(&bullet) && use_alpha(item, st, prevs))
        {
            if LOWER.is_match(&bullet) {
                replace_first(&bullet, &LOWER, &c.to_ascii_lowercase())
            } else {
                replace_first(&bullet, &UPPER, &c.to_ascii_uppercase())
            }
        } else if has(&DIGITS, &counter) && DIGITS.is_match(&bullet) {
            replace_first(&bullet, &DIGITS, counter.as_deref().unwrap())
        } else if alphap && UPPER.is_match(&bullet) {
            replace_first(&bullet, &UPPER, "A")
        } else if alphap && LOWER.is_match(&bullet) {
            replace_first(&bullet, &LOWER, "a")
        } else if NUM_OR_ALPHA.is_match(&bullet) {
            replace_first(&bullet, &NUM_OR_ALPHA, "1")
        } else {
            bullet
        };
        st.0[i].bullet = bullet_string(&new);
    }
}

/// org-list-struct-fix-ind.
pub fn fix_ind(st: &mut Struct, parents: &Alist, bullet_size: Option<usize>) {
    let top_ind = st.0[0].ind;
    let off = indent_offset();
    for i in 1..st.0.len() {
        let item = st.0[i].pos;
        st.0[i].ind = match parent(item, parents) {
            Some(p) => bullet_size.unwrap_or_else(|| st.bullet(p).len()) + st.ind(p) + off,
            None => top_ind,
        };
    }
}

/// org-list-struct-fix-box: the blocking item when ORDERED was broken.
pub fn fix_box(st: &mut Struct, parents: &Alist, _prevs: &Alist, ordered: bool) -> Option<usize> {
    let all = st.positions();
    let mut plist: Vec<usize> = vec![];
    for &e in &all {
        if let Some(p) = parent(e, parents)
            && st.checkbox(p).is_some()
            && !plist.contains(&p)
        {
            plist.insert(0, p);
        }
    }
    plist.sort_by_key(|p| std::cmp::Reverse(st.ind(*p)));
    for p in plist {
        let boxes: Vec<Option<String>> = children(p, parents)
            .iter()
            .map(|c| st.checkbox(*c).map(str::to_owned))
            .collect();
        let has = |s: &str| boxes.iter().any(|b| b.as_deref() == Some(s));
        let new = if has("[ ]") && has("[X]") || has("[-]") {
            Some("[-]".to_owned())
        } else if has("[X]") {
            Some("[X]".to_owned())
        } else if has("[ ]") {
            Some("[ ]".to_owned())
        } else {
            st.item(p).checkbox.clone()
        };
        st.item_mut(p).checkbox = new;
    }
    if ordered {
        let boxes: Vec<Option<String>> = all
            .iter()
            .map(|e| st.checkbox(*e).map(str::to_owned))
            .collect();
        if let Some(first) = boxes.iter().position(|b| b.as_deref() == Some("[ ]"))
            && boxes[first..].iter().any(|b| b.as_deref() == Some("[X]"))
        {
            for &e in &all[first..] {
                if st.checkbox(e).is_some() {
                    st.item_mut(e).checkbox = Some("[ ]".into());
                }
            }
            fix_box(st, parents, _prevs, false);
            return Some(all[first]);
        }
    }
    None
}

/// org-list-struct-fix-item-end.
pub fn fix_item_end(st: &mut Struct) {
    let mut end_list: Vec<(usize, usize)> = vec![];
    let mut acc_end: Vec<(usize, usize)> = vec![];
    for it in &st.0 {
        if st.get(it.end).is_none() {
            let up = acc_end.iter().rev().find(|e| e.0 > it.end).map(|e| e.1);
            end_list.push((up.map_or(0, |u| st.ind(u) + 2), it.end));
        }
        end_list.push((it.ind, it.pos));
        acc_end.push((it.end, it.pos));
    }
    end_list.reverse();
    end_list.sort_by_key(|e| e.1);
    assoc_end(st, &end_list);
}

/// org-list-struct-outdent.
pub fn struct_outdent(start: usize, end: usize, parents: &Alist) -> Result<Alist, String> {
    let mut acc: Vec<(usize, usize)> = vec![];
    let mut out = vec![];
    for &(item, par) in parents {
        out.push(if item < start {
            (item, par)
        } else if item >= end {
            match par.and_then(|p| acc.iter().find(|a| a.0 == p)) {
                Some(c) => (item, Some(c.1)),
                None => (item, par),
            }
        } else {
            let Some(p) = par else {
                return Err("Cannot outdent top-level items".into());
            };
            acc.insert(0, (p, item));
            if p >= start {
                (item, par)
            } else {
                (item, parent(p, parents))
            }
        });
    }
    Ok(out)
}

/// The demotion key of a bullet (org-list-struct-indent).
fn bullet_kind(b: &str) -> String {
    for (r, k) in [
        (r"[A-Z]\.", "A."),
        (r"[A-Z]\)", "A)"),
        (r"[a-z]\.", "a."),
        (r"[a-z]\)", "a)"),
        (r"[0-9]\.", "1."),
        (r"[0-9]\)", "1)"),
    ] {
        if re(r).is_match(b) {
            return k.into();
        }
    }
    b.trim().to_owned()
}

/// org-list-struct-indent.
pub fn struct_indent(
    start: usize,
    end: usize,
    st: &mut Struct,
    parents: &Alist,
    prevs: &Alist,
) -> Result<Alist, String> {
    let mut acc: Vec<(usize, Option<usize>)> = vec![];
    let modify = demote_bullets();
    let mut out = vec![];
    for &(item, par) in parents {
        let cell = if item < start {
            (item, par)
        } else if item >= end {
            match par.and_then(|p| acc.iter().find(|a| a.0 == p)) {
                Some(c) => (item, c.1),
                None => (item, par),
            }
        } else {
            let prev = prev_item(item, prevs);
            let kind = bullet_kind(st.bullet(item));
            if let Some((_, to)) = modify.iter().find(|(k, _)| *k == kind) {
                st.item_mut(item).bullet = bullet_string(to);
            }
            let cell = match prev {
                None if par.is_none_or(|p| p < start) => {
                    return Err("Cannot indent the first item of a list".into());
                }
                None => (item, par),
                Some(pv) if pv < start => (item, Some(pv)),
                Some(pv) => (item, acc.iter().find(|a| a.0 == pv).and_then(|a| a.1)),
            };
            acc.insert(0, cell);
            cell
        };
        out.push(cell);
    }
    Ok(out)
}

// ---- applying ----

/// org-list-struct-apply-struct.
pub fn apply_struct(b: &mut Buf, st: &Struct, old: &Struct) {
    let origin = b.marker(b.pt, false);
    let item_re = item_res();
    let shift_body = |b: &mut Buf, end: usize, beg: usize, delta: i64, ind: Option<usize>| {
        b.goto(end);
        b.skip_backward(" \r\t\n");
        b.forward_line(0);
        while b.pt > beg || (b.pt == beg && !b.looking_p(&item_re.0)) {
            if b.looking_p(&NONBLANK) {
                let floor = ind.map_or(-1, |i| i as i64 + 1);
                let new = (b.ind() as i64 + delta).max(floor).max(0) as usize;
                b.indent_line_to(new);
            }
            if b.pt == 0 || b.bol() == 0 {
                break;
            }
            b.forward_line(-1);
        }
    };
    let modify_item = |b: &mut Buf, item: usize| {
        b.goto(item);
        let it = st.item(item);
        let new_ind = it.ind;
        let old_ind = b.ind();
        let new_bul = bullet_string(&it.bullet);
        let old_bul = &old.item(item).bullet;
        let new_box = it.checkbox.clone();
        let c = b.looking_at(&FULL_ITEM).unwrap_or_default();
        if *old_bul != new_bul
            && let Some((s, e)) = c.get(1)
        {
            let o = b.mpos(origin);
            let keep = if s <= o && o <= e {
                b.looking_at_pos(&LEADING_WS, o)
                    .and_then(|k| k.str(b, 0).map(str::to_owned))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            b.delete(s, e);
            b.goto(s);
            b.insert_before_markers(&new_bul);
            b.insert(&keep);
        }
        b.goto(item);
        let c = b.looking_at(&FULL_ITEM).unwrap_or_default();
        let cur_box = c.str(b, 3).map(str::to_owned);
        match (&cur_box, &new_box) {
            (a, n) if a == n => {}
            (Some(_), Some(n)) => {
                let (s, e) = c.get(3).unwrap();
                b.delete(s, e);
                b.goto(s);
                b.insert(n);
            }
            (Some(_), None) => {
                if let Some(g) = b.looking_at(&BOX_ANYWHERE).and_then(|m| m.get(1)) {
                    b.delete(g.0, g.1);
                }
            }
            (None, Some(n)) => {
                let counterp = c.end(2);
                b.goto(counterp.map_or(c.end(1).unwrap_or(item), |p| p + 1));
                b.insert(&format!("{n}{}", if counterp.is_some() { "" } else { " " }));
            }
            _ => {}
        }
        if new_ind != old_ind {
            b.goto(item);
            let bol = b.bol();
            b.skip_forward(" \t");
            b.delete(bol, b.pt);
            b.goto(bol);
            b.insert(&" ".repeat(new_ind));
        }
    };
    // 1. Item shifts and ending positions.
    let mut end_list: Vec<(usize, Option<usize>)> = vec![];
    let mut acc_end: Vec<(usize, usize)> = vec![];
    let mut itm_shift: Vec<(usize, i64, usize)> = vec![];
    for e in &old.0 {
        let pos = e.pos;
        let ind_pos = st.ind(pos);
        let shift =
            (ind_pos + st.bullet(pos).len()) as i64 - (old.ind(pos) + old.bullet(pos).len()) as i64;
        let end_pos = e.end;
        itm_shift.push((pos, shift, ind_pos));
        if old.get(end_pos).is_none() {
            let up = acc_end.iter().rev().find(|a| a.0 > end_pos).map(|a| a.1);
            end_list.insert(0, (end_pos, up));
        }
        acc_end.push((end_pos, pos));
    }
    // 2. Slices.
    let mut all_ends: Vec<usize> = itm_shift.iter().map(|e| e.0).collect();
    for (e, _) in &end_list {
        if !all_ends.contains(e) {
            all_ends.push(*e);
        }
    }
    all_ends.sort();
    let mut sliced: Vec<(usize, usize, i64, Option<usize>)> = vec![];
    for w in all_ends.windows(2) {
        let (up, down) = (w[0], w[1]);
        let itemp = st.get(up).is_some();
        let delta = if itemp {
            itm_shift.iter().find(|e| e.0 == up).map_or(0, |e| e.1)
        } else {
            let child = acc_end.iter().find(|a| a.0 == up).map_or(up, |a| a.1);
            let ind = st.ind(child) as i64;
            let mut min_ind = i64::MAX;
            b.excursion(|b| {
                b.goto(up);
                while b.pt < down {
                    if !b.looking_p(&BLANK) {
                        min_ind = min_ind.min(b.ind() as i64);
                        if let Some(c) = b.looking_at(&BEGIN_BLOCK_COL0) {
                            let kind = regex::escape(c.str(b, 1).unwrap_or(""));
                            b.re_search_forward(
                                &re(&format!(r"(?mi)^[ \t]*#\+END{kind}[ \t]*$")),
                                down,
                            );
                        } else if b.looking_p(&DRAWER) {
                            b.re_search_forward(&DRAWER_END_LINE_ANY, down);
                        }
                    }
                    let before = b.pt;
                    b.forward_line(1);
                    if b.pt == before {
                        break;
                    }
                }
            });
            if min_ind == i64::MAX {
                0
            } else {
                ind - min_ind
            }
        };
        let ind = itemp.then(|| itm_shift.iter().find(|e| e.0 == up).map_or(0, |e| e.2));
        sliced.insert(0, (down, up, delta, ind));
    }
    // 3. Shift each slice, bottom first.
    for (end, beg, delta, ind) in sliced {
        if delta != 0 {
            shift_body(b, end, beg, delta, ind);
        }
        if let Some(cell) = st.get(beg)
            && Some(cell) != old.get(beg)
        {
            modify_item(b, beg);
        }
    }
    b.goto(b.mpos(origin));
    b.free(origin);
}

lazy_re!(BEGIN_BLOCK_COL0, r"\A(?i:#\+BEGIN(:|_\S+))");

/// org-list-write-struct.
pub fn write_struct(b: &mut Buf, st: &mut Struct, parents: &Alist, old: Option<&Struct>) {
    let old = old.cloned().unwrap_or_else(|| st.clone());
    fix_ind(st, parents, Some(2));
    fix_item_end(st);
    let prevs = st.prevs();
    fix_bul(st, &prevs);
    fix_ind(st, parents, None);
    fix_box(st, parents, &prevs, false);
    apply_struct(b, st, &old);
}

// ---- moving items ----

/// org-list-swap-items: A before B, same sub-list. Visibility is kept.
pub fn swap_items(b: &mut Buf, beg_a: usize, beg_b: usize, st: &mut Struct) {
    let end_a_nb = st.end_before_blank(b, beg_a);
    let end_b_nb = st.end_before_blank(b, beg_b);
    let end_a = st.end(beg_a);
    let end_b = st.end(beg_b);
    let size_a = end_a_nb - beg_a;
    let size_b = end_b_nb - beg_b;
    let body_a = b.sub(beg_a, end_a_nb).to_owned();
    let body_b = b.sub(beg_b, end_b_nb).to_owned();
    let between = b.sub(end_a_nb, beg_b).to_owned();
    let mut sub_a = vec![beg_a];
    sub_a.extend(st.subtree(beg_a));
    let mut sub_b = vec![beg_b];
    sub_b.extend(st.subtree(beg_b));
    let rel = |b: &Buf, from: usize, to: usize| -> Vec<Fold> {
        b.folds
            .iter()
            .filter(|f| f.beg >= from && f.end <= to)
            .map(|f| Fold {
                beg: f.beg - from,
                end: f.end - from,
                ..*f
            })
            .collect()
    };
    let folds_a = rel(b, beg_a, end_a);
    let folds_b = rel(b, beg_b, end_b);
    b.folds.retain(|f| f.end < beg_a || f.beg > end_b_nb);
    b.excursion(|b| {
        b.delete(beg_a, end_b_nb);
        b.goto(beg_a);
        b.insert(&format!("{body_b}{between}{body_a}"));
    });
    let at_a = beg_a + size_b + between.len();
    b.folds.extend(folds_b.iter().map(|f| Fold {
        beg: f.beg + beg_a,
        end: f.end + beg_a,
        ..*f
    }));
    b.folds.extend(folds_a.iter().map(|f| Fold {
        beg: f.beg + at_a,
        end: f.end + at_a,
        ..*f
    }));
    let d = |x: usize, add: i64| (x as i64 + add) as usize;
    for e in &mut st.0 {
        let pos = e.pos;
        if pos < beg_a {
        } else if sub_a.contains(&pos) {
            let end_e = e.end;
            let off = end_b_nb as i64 - end_a_nb as i64;
            e.pos = d(pos, off);
            e.end = if end_e == end_a { end_b } else { d(end_e, off) };
        } else if sub_b.contains(&pos) {
            let end_e = e.end;
            let off = beg_a as i64 - beg_b as i64;
            e.pos = d(pos, off);
            e.end = if end_e == end_b {
                beg_a + size_b + (end_a - end_a_nb)
            } else {
                d(end_e, off)
            };
        } else if pos < beg_b {
            let off = size_b as i64 - size_a as i64;
            e.pos = d(pos, off);
            e.end = d(e.end, off);
        }
    }
    st.sort();
}

/// org-list-separating-blank-lines-number (point at the item).
pub fn blank_lines_number(b: &mut Buf, pos: usize, st: &Struct, prevs: &Alist) -> usize {
    let item = b.pt;
    let count_blanks = |b: &Buf, at: usize| {
        let bol = b.bol_at(at);
        let mut p = bol;
        while p > 0 && matches!(b.s.as_bytes()[p - 1], b' ' | b'\t' | b'\n' | b'\r') {
            p -= 1;
        }
        let after = if b.eol_at(p) >= b.len() {
            b.len()
        } else {
            b.eol_at(p) + 1
        };
        b.sub(after.min(bol), bol).matches('\n').count()
    };
    let opt = alist_option(
        "org-blank-before-new-entry",
        "plain-list-item",
        Sexp::Sym("auto".into()),
    );
    if opt.is_nil() {
        return 0;
    }
    if opt.str() != Some("auto") {
        return 1;
    }
    if let Some(n) = next_item(item, prevs) {
        return count_blanks(b, n);
    }
    if prev_item(item, prevs).is_some() {
        return count_blanks(b, item);
    }
    if pos > st.end_before_blank(b, item) {
        let u = count_blanks(b, pos);
        if u > 0 {
            return u;
        }
    }
    let top = st.top();
    let lim = st.end_before_blank(b, item);
    let found = BLANK_ANY.find_at(&b.s[..lim.min(b.len())], top).is_some();
    usize::from(found)
}

/// Is `p` before the first character after the item's bullet (and tag)?
fn before_bullet_end(b: &Buf, item: usize, pos: usize) -> bool {
    let c = b.looking_at_pos(&FULL_ITEM, item).unwrap_or_default();
    let lim = match c.get(4) {
        None => c.end(0).unwrap_or(item),
        Some((s, _)) if c.str(b, 1).is_some_and(|x| x.contains(['.', ')'])) => s,
        Some((_, e)) => {
            let mut p = e;
            while p < b.len() && matches!(b.s.as_bytes()[p], b' ' | b'\t') {
                p += 1;
            }
            p
        }
    };
    pos <= lim
}

/// org-list-insert-item. `split` overrides org-M-RET-may-split-line.
pub fn insert_item(
    b: &mut Buf,
    pos: usize,
    st: &mut Struct,
    prevs: &Alist,
    checkbox: bool,
    after_bullet: &str,
    split: Option<bool>,
) {
    let mut found = None;
    let mut item = None;
    for it in &st.0 {
        if it.pos > pos {
            found = Some(item);
            break;
        }
        if it.end < pos {
            continue;
        }
        item = Some(it.pos);
    }
    let item = found
        .flatten()
        .or(item)
        .unwrap_or_else(|| st.0.last().unwrap().pos);
    let item_end = st.end(item);
    let item_end_nb = st.end_before_blank(b, item);
    b.goto(item);
    let beforep = before_bullet_end(b, item, pos);
    let split = split.unwrap_or_else(|| {
        alist_option(
            "org-M-RET-may-split-line",
            "item",
            Sexp::List(vec![Sexp::Dotted(
                vec![Sexp::Sym("default".into())],
                Box::new(Sexp::T),
            )]),
        )
        .truthy()
    });
    let blank_nb = blank_lines_number(b, pos, st, prevs);
    let ind = st.ind(item);
    let bullet = bullet_string(st.bullet(item));
    let bx = checkbox.then_some("[ ]");
    let mut pos = pos;
    let text_cut = if !beforep && split {
        b.goto(pos);
        if item_end < pos {
            let e = b.eol();
            b.delete(item_end - 1, e);
        }
        b.skip_backward(" \r\t\n");
        let mut q = b.pt;
        while q < b.len() && matches!(b.s.as_bytes()[q], b' ' | b'\t') {
            q += 1;
        }
        pos = q;
        let pt = b.pt;
        Some(b.delete_extract(pt, item_end_nb.max(pt)))
    } else {
        None
    };
    let cut = text_cut.as_deref().unwrap_or("");
    let body = format!(
        "{bullet}{}{after_bullet}{}",
        bx.map_or(String::new(), |x| format!("{x} ")),
        cut.trim_start_matches([' ', '\t'])
    );
    let sep = "\n".repeat(1 + blank_nb);
    let item_size = ind + body.len() + sep.len();
    let size_offset = item_size as i64 - cut.len() as i64;
    b.goto(item);
    b.insert(&format!("{}{body}{sep}", " ".repeat(ind)));
    let d = |x: usize, add: i64| (x as i64 + add) as usize;
    for e in &mut st.0 {
        let (p, end) = (e.pos, e.end);
        if p < item {
            if end > item {
                e.end = d(end, size_offset);
            }
        } else if p == item && !beforep && split {
            e.pos = p + item_size;
            e.end = d(end, size_offset);
        } else if split && !beforep && p >= pos && p <= item_end_nb {
            let offset = pos as i64
                - item as i64
                - ind as i64
                - bullet.len() as i64
                - after_bullet.len() as i64;
            e.pos = d(p, -offset);
            e.end = d(end, -offset);
        } else {
            e.pos = d(p, size_offset);
            e.end = d(end, size_offset);
        }
    }
    st.0.push(Item {
        pos: item,
        ind,
        bullet: bullet.clone(),
        counter: None,
        checkbox: bx.map(str::to_owned),
        tag: None,
        end: item + item_size,
    });
    st.sort();
    if beforep {
        b.goto(item);
    } else {
        swap_items(b, item, item + item_size, st);
        let prevs = st.prevs();
        if let Some(n) = next_item(item, &prevs) {
            b.goto(n);
        }
    }
}

/// org-list-delete-item.
pub fn delete_item(b: &mut Buf, item: usize, st: &Struct) -> Struct {
    let end = st.end(item);
    let beg = if st.bottom() == end {
        let mut p = item;
        while p > 0 && matches!(b.s.as_bytes()[p - 1], b' ' | b'\t' | b'\n' | b'\r') {
            p -= 1;
        }
        (b.eol_at(p) + 1).min(b.len())
    } else {
        item
    };
    b.delete(beg, end);
    let size = end - beg;
    Struct(
        st.0.iter()
            .filter_map(|e| {
                let mut e = e.clone();
                if e.pos < item {
                    if e.end == item {
                        e.end = beg;
                    } else if e.end > item {
                        e.end -= size;
                    }
                    Some(e)
                } else if e.pos < end {
                    None
                } else {
                    e.pos -= size;
                    e.end -= size;
                    Some(e)
                }
            })
            .collect(),
    )
}

/// Where org-list-send-item sends an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dest {
    Pos(usize),
    Begin,
    End,
    /// "N": the Nth position in the list.
    Nth(usize),
    Kill,
    Delete,
}

/// org-list-send-item: the new structure, and the killed text for Kill.
pub fn send_item(b: &mut Buf, item: usize, dest: Dest, st: &Struct) -> (Struct, Option<String>) {
    let prevs = st.prevs();
    let item_end = st.end(item);
    let bul = st.bullet(item).len();
    let body_beg = b
        .looking_at_pos(&LEADING_WS, item)
        .and_then(|c| c.end(0))
        .unwrap_or(item)
        + bul;
    let body = b.sub(body_beg, item_end).trim().to_owned();
    let last_eol = |b: &Buf| b.eol_at(last_item(item, &prevs));
    let ins = match &dest {
        Dest::Kill => return (delete_item(b, item, st), Some(body)),
        Dest::Delete => return (delete_item(b, item, st), None),
        Dest::Begin => list_begin(item, &prevs),
        Dest::End => last_eol(b),
        Dest::Nth(n) => {
            let all = all_items(item, &prevs);
            let index = n % all.len();
            if index != 0 {
                all[index - 1]
            } else {
                last_eol(b)
            }
        }
        Dest::Pos(p) => *p,
    };
    if item == ins {
        return (st.clone(), None);
    }
    let len = item_end - item;
    let m = b.marker(item, false);
    let mut st = st.clone();
    insert_item(b, ins, &mut st, &prevs, false, &body, Some(false));
    let item = b.mpos(m);
    b.free(m);
    let mut moved = vec![item];
    moved.extend(st.subtree(item));
    let new_item = b.pt;
    let new_end = st.end(new_item);
    let old_end = st.end(item);
    let shift = new_item as i64 - item as i64;
    let off = |x: usize| (x as i64 + shift) as usize;
    st.0.retain(|e| e.pos != new_item);
    let copies: Vec<Item> = moved
        .iter()
        .map(|&p| {
            let c = st.item(p);
            Item {
                pos: off(c.pos),
                end: if c.end == old_end {
                    new_end
                } else {
                    off(c.end)
                },
                ..c.clone()
            }
        })
        .collect();
    st.0.extend(copies);
    st.sort();
    // Keep the moved item's inner visibility.
    let inner: Vec<Fold> = b
        .folds
        .iter()
        .filter(|f| f.beg > item && f.end <= item + len)
        .map(|f| Fold {
            beg: off(f.beg),
            end: off(f.end),
            ..*f
        })
        .collect();
    b.folds.extend(inner);
    (delete_item(b, item, &st), None)
}
