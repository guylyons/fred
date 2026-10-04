//! Port of org-list.el: plain lists. See docs/org-port-notes.md ("Module
//! contracts").
//!
//! The upstream algorithms work on buffer positions; they are ported
//! literally over [`buf::Buf`], a string copy of the buffer with point,
//! markers and folds, which [`store`] writes back as one minimal line
//! splice (one undo step, folds kept).

pub mod buf;
pub mod export;
pub mod structure;
#[cfg(test)]
mod tests;

use super::fold::{self, Spec};
use super::sexp::{self, Sexp};
use super::syntax::{self, Settings};
use super::{Prefix, options};
use crate::editor::Editor;
use buf::{Buf, Caps, Fold};
use regex::Regex;
use std::sync::LazyLock;
use structure::*;

/// Per-buffer list state.
#[derive(Default)]
pub struct State {
    /// org-list-checkbox-radio-mode.
    pub radio_mode: bool,
    /// org-tab-ind-state, valid at the buffer version it was left at.
    tab_ind: Option<(u64, usize, String)>,
    /// org-last-indent-begin/end-marker, valid at a buffer version.
    indent_zone: Option<(u64, usize, usize)>,
}

// ---- the buffer bridge ----

fn line_byte(ed: &Editor, l: usize) -> usize {
    ed.buf
        .rope()
        .line_to_byte(l.min(ed.line_count().saturating_sub(1)))
}

/// The buffer as a [`Buf`]: text (with its final newline), point, folds.
pub fn load(ed: &Editor) -> Buf {
    let mut s = ed.buf.text();
    if ed.buf.final_newline {
        s.push('\n');
    }
    let pt = line_byte(ed, ed.cur.line) + ed.cur.byte;
    let mut b = Buf::new(s, pt);
    if let Some(o) = &ed.org {
        for (spec, f) in [
            (Spec::Outline, &o.specs.outline),
            (Spec::Block, &o.specs.block),
            (Spec::Drawer, &o.specs.drawer),
        ] {
            for &(s, e) in f.ranges() {
                if e < ed.line_count() {
                    let end = line_byte(ed, e) + ed.buf.line_len(e);
                    b.folds.push(Fold {
                        spec,
                        beg: line_byte(ed, s),
                        end,
                    });
                }
            }
        }
    }
    b
}

/// Write `b` back: the changed lines, point, folds and message.
pub fn store(ed: &mut Editor, b: Buf) {
    let mut text = b.s.as_str();
    if ed.buf.final_newline && text.ends_with('\n') {
        text = &text[..text.len() - 1];
    }
    let new: Vec<&str> = text.split('\n').collect();
    let n = ed.line_count();
    let mut p = 0;
    while p < n && p < new.len() && ed.buf.line(p) == new[p] {
        p += 1;
    }
    let mut q = 0;
    while q < n - p && q < new.len() - p && ed.buf.line(n - 1 - q) == new[new.len() - 1 - q] {
        q += 1;
    }
    let had_folds = !ed.folds.ranges().is_empty();
    if p < n || p < new.len() {
        let with: Vec<String> = new[p..new.len() - q]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        if p < n - q || !with.is_empty() {
            super::splice(ed, p, n - p - q, &with);
        }
    }
    if had_folds || !b.folds.is_empty() {
        if let Some(o) = &mut ed.org {
            o.specs = fold::Specs::default();
        }
        for f in &b.folds {
            let (s, e) = (b.line_of(f.beg), b.line_of(f.end));
            fold::region(ed, s, e, true, f.spec);
        }
        fold::region(ed, 0, 0, false, Spec::Outline);
    }
    let pt = b.pt.min(text.len());
    let line = b.line_of(pt);
    let byte = pt - b.bol_at(pt);
    ed.set_cursor(line, byte);
    if line < ed.line_count() && ed.cur.line == line {
        ed.cur.byte = byte.min(ed.buf.line_len(line));
    }
    if let Some(m) = b.msg {
        ed.set_msg(m);
    }
}

/// Run `f` on the buffer; changes stay even when it fails (as in Emacs).
pub fn with_buf<T>(
    ed: &mut Editor,
    f: impl FnOnce(&mut Buf) -> Result<T, String>,
) -> Result<T, String> {
    let mut b = load(ed);
    let r = f(&mut b);
    let lines = std::mem::take(&mut b.cookie_lines);
    store(ed, b);
    align_tags(ed, &lines);
    r
}

/// The region as buffer positions: line beginning of the first line to
/// the end of the last.
fn region_pos(ed: &Editor) -> Option<(usize, usize)> {
    let (lo, hi) = super::region(ed)?;
    let hi = hi.min(ed.line_count() - 1);
    Some((line_byte(ed, lo), line_byte(ed, hi) + ed.buf.line_len(hi)))
}

fn state(ed: &mut Editor) -> Option<&mut State> {
    ed.org.as_mut().map(|o| &mut o.list)
}

fn last_command(ed: &Editor) -> Option<&str> {
    ed.org.as_ref().and_then(|o| o.last_command.as_deref())
}

// ---- predicates ----

static TIMER: LazyLock<Regex> = LazyLock::new(|| re(r"\A([0-9]+:[0-9]+:[0-9]+)[ \t]+::[ \t]+"));
static DESC: LazyLock<Regex> = LazyLock::new(|| re(r"\A(?m:(\S.+)[ \t]+::(?:[ \t]+|$))"));
static CHECKBOX: LazyLock<Regex> = LazyLock::new(|| re(r"\A(\[[- X]\])[ \t]+"));
static COUNTER: LazyLock<Regex> =
    LazyLock::new(|| re(r"\A\[@(?:start:)?([0-9]+|[A-Za-z])\][ \t]*"));

/// org-list-at-regexp-after-bullet-p.
pub fn at_regexp_after_bullet(b: &mut Buf, r: &Regex) -> Option<Caps> {
    if !at_item(b) {
        return None;
    }
    let mut p = b.looking_at_pos(&item_res().0, b.bol())?.end(0)?;
    if let Some(c) = b.looking_at_pos(&COUNTER, p) {
        p = c.end(0)?;
    }
    b.looking_at_pos(r, p)
}

/// org-at-item-timer-p.
pub fn at_item_timer(b: &mut Buf) -> bool {
    at_regexp_after_bullet(b, &TIMER).is_some()
}

/// org-at-item-description-p.
pub fn at_item_description(b: &mut Buf) -> bool {
    at_regexp_after_bullet(b, &DESC).is_some()
}

/// org-at-item-checkbox-p: the checkbox's range.
pub fn at_item_checkbox(b: &mut Buf) -> Option<(usize, usize)> {
    at_regexp_after_bullet(b, &CHECKBOX)?.get(1)
}

/// org-at-item-counter-p.
pub fn at_item_counter(b: &mut Buf) -> Option<String> {
    if !at_item(b) {
        return None;
    }
    let c = b.looking_at_pos(&FULL_ITEM, b.bol())?;
    c.str(b, 2).map(str::to_owned)
}

/// org-at-item-bullet-p.
pub fn at_item_bullet(b: &mut Buf) -> bool {
    at_item(b)
        && !matches!(b.char_after(), Some(b' ' | b'\t'))
        && b.looking_at_pos(&item_res().0, b.bol())
            .and_then(|c| c.end(0))
            .is_some_and(|e| b.pt < e)
}

/// org-apply-on-list: `f` on each item of the list at point, last item
/// first, point at the item; point is restored.
pub fn apply_on_list<T>(b: &mut Buf, init: T, mut f: impl FnMut(&mut Buf, T) -> T) -> T {
    let st = list_struct(b);
    let prevs = st.prevs();
    let item = b.marker(b.bol(), false);
    let mut v = init;
    let all = all_items(b.mpos(item), &prevs);
    let marks: Vec<_> = all.iter().map(|&p| b.marker(p, false)).collect();
    for m in marks.into_iter().rev() {
        b.goto(b.mpos(m));
        b.free(m);
        v = f(b, v);
    }
    b.goto(b.mpos(item));
    b.free(item);
    v
}

/// org-list-item-body-column.
pub fn item_body_column(b: &Buf, item: usize) -> usize {
    let t = b.sub(item, b.eol_at(item));
    let ws = t.len() - t.trim_start_matches([' ', '\t']).len();
    let bullet = t[ws..].split([' ', '\t']).next().unwrap_or("");
    let two = options::string("org-list-two-spaces-after-bullet-regexp", "nil");
    let two = two != "nil" && Regex::new(&emacs_re(&two)).is_ok_and(|r| r.is_match(bullet));
    buf::col_of(&t[..ws + bullet.len()]) + if two { 2 } else { 1 }
}

/// org-list-search-forward: a match in a valid list context.
pub fn search_forward(b: &mut Buf, r: &Regex, bound: usize, move_on_fail: bool) -> Option<Caps> {
    let origin = b.pt;
    loop {
        match b.re_search_forward(r, bound) {
            None => {
                b.goto(if move_on_fail { bound } else { origin });
                return None;
            }
            Some(c) => {
                if valid_context(b) {
                    return Some(c);
                }
                if c.end(0) == c.beg(0) {
                    b.goto(b.pt + 1);
                }
            }
        }
    }
}

/// org-entry-get (no inheritance) for the entry at `p`.
pub fn entry_get(b: &Buf, p: usize, prop: &str) -> Option<String> {
    let h = heading_before(b, p)?;
    let mut l = b.eol_at(h) + 1;
    if l < b.len() && super::ctx::is_planning(b.sub(l, b.eol_at(l))) {
        l = b.eol_at(l) + 1;
    }
    if l >= b.len()
        || !b
            .sub(l, b.eol_at(l))
            .trim()
            .eq_ignore_ascii_case(":PROPERTIES:")
    {
        return None;
    }
    loop {
        l = b.eol_at(l) + 1;
        if l >= b.len() {
            return None;
        }
        let t = b.sub(l, b.eol_at(l)).trim();
        if t.eq_ignore_ascii_case(":END:") || heading_at(b, l) {
            return None;
        }
        if let Some(rest) = t.strip_prefix(':')
            && let Some((k, v)) = rest.split_once(':')
            && k.eq_ignore_ascii_case(prop)
        {
            return Some(v.trim().to_owned());
        }
    }
}

static PLANNING: LazyLock<Regex> =
    LazyLock::new(|| re(r"\A[ \t]*(?:CLOSED:|DEADLINE:|SCHEDULED:)"));
static PROP_DRAWER: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\A(?mi:[ \t]*:PROPERTIES:[ \t]*\n(?:[ \t]*:\S+:(?:[ \t].*)?[ \t]*\n)*?[ \t]*:END:[ \t]*$)")
});
static CLOCK_OR_BLANK: LazyLock<Regex> = LazyLock::new(|| re(r"\A(?m:[ \t]*$|[ \t]*CLOCK:)"));
static LOGBOOK: LazyLock<Regex> = LazyLock::new(|| re(r"\A(?mi:[ \t]*:LOGBOOK:[ \t]*$)"));

/// org-end-of-meta-data. `full`: None, Some(true) for t, Some(false) for
/// other non-nil values.
pub fn end_of_meta_data(b: &mut Buf, full: Option<bool>) {
    if let Some(h) = heading_before(b, b.pt) {
        b.goto(h);
    }
    b.forward_line(1);
    if b.looking_p(&PLANNING) {
        b.forward_line(1);
    }
    if let Some(c) = b.looking_at(&PROP_DRAWER) {
        b.goto(c.end(0).unwrap_or(b.pt));
        b.forward_line(1);
    }
    let Some(full) = full else { return };
    if heading_at(b, b.pt) && b.pt < b.len() {
        return;
    }
    let end = next_heading(b, b.pt.saturating_sub(1).max(b.bol())).unwrap_or(b.len());
    while !b.eobp() {
        if b.looking_p(&CLOCK_OR_BLANK) {
            b.forward_line(1);
        } else if b.looking_p(&LOGBOOK) || (full && b.looking_p(&DRAWER)) {
            if b.re_search_forward(&DRAWER_END_LINE_ANY, end).is_some() {
                b.forward_line(1);
            } else {
                return;
            }
        } else {
            return;
        }
    }
}

// ---- checkbox statistics ----

static COOKIE: LazyLock<Regex> = LazyLock::new(|| re(r"(\[[0-9]*%\])|(\[[0-9]*/[0-9]*\])"));
static BOX_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?m)^[ \t]*(?:[-+*]|(?:[0-9]+|[A-Za-z])[.)])[ \t]+(?:\[@(?:start:)?(?:[0-9]+|[A-Za-z])\][ \t]*)?(\[[- X]\])",
    )
});
static RECURSIVE: LazyLock<Regex> = LazyLock::new(|| re(r"\brecursive\b"));
static TODO_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"\btodo\b"));

/// Where org-update-checkbox-count looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The current section (no argument).
    Section,
    /// The whole buffer (ALL).
    All,
    /// Between two positions (`narrow`).
    Narrow(usize, usize),
}

/// org-format-percent-cookie.
pub fn percent_cookie(done: usize, total: usize) -> String {
    let p = (100 * done) / total.max(1);
    format!("[{}%]", if p == 0 && done > 0 { 1 } else { p })
}

/// The container of a cookie at `p` (org-element-lineage of the cookie):
/// Some((contents begin, contents end, item)) or None for the section.
/// Err(()) when the cookie is not a statistics cookie (verbatim context).
#[allow(clippy::result_unit_err)]
fn cookie_container(b: &mut Buf, p: usize) -> Result<Option<(usize, usize, Option<usize>)>, ()> {
    let line = b.sub(b.bol_at(p), b.eol_at(p)).to_owned();
    let lt = line.trim_start();
    if lt.starts_with("#+") || lt.starts_with("# ") || lt == "#" {
        return Err(());
    }
    let col = p - b.bol_at(p);
    let mut from = 0;
    while let Some(s) = line[from..].find("[[").map(|i| i + from) {
        let Some(e) = line[s..].find("]]").map(|i| s + i + 2) else {
            break;
        };
        if col >= s && col < e {
            return Err(());
        }
        from = e;
    }
    b.excursion(|b| {
        b.goto(p);
        if heading_at(b, p) {
            return Ok(None);
        }
        let (lo, hi, ty) = context(b);
        if ty == CtxType::Invalid {
            return Err(());
        }
        if let Some(item) = in_item(b)
            && heading_before(b, item) == heading_before(b, p)
        {
            let st = b.excursion(|b| {
                b.goto(item);
                list_struct(b)
            });
            if st.get(item).is_some_and(|it| it.end > p) {
                let cb = b
                    .looking_at_pos(&FULL_ITEM, item)
                    .and_then(|c| c.end(0))
                    .unwrap_or(item);
                return Ok(Some((cb, st.end_before_blank(b, item), Some(item))));
            }
        }
        match ty {
            CtxType::Drawer => {
                let d = b
                    .re_search_backward(&DRAWER, 0)
                    .and_then(|c| c.str(b, 1).map(str::to_owned));
                if d.is_some_and(|d| d.eq_ignore_ascii_case("PROPERTIES")) {
                    return Err(());
                }
                Ok(Some((lo, hi, None)))
            }
            CtxType::Block => Ok(Some((lo, hi, None))),
            _ => Ok(None),
        }
    })
}

/// Number of checked boxes and of all boxes for a cookie (count-boxes).
fn count_boxes(item: Option<usize>, structs: &[Struct], recursive: bool) -> (usize, usize) {
    let (mut on, mut all) = (0, 0);
    for s in structs {
        let prevs = s.prevs();
        let parents = s.parents();
        let items = match (recursive, item) {
            (true, Some(i)) => s.subtree(i),
            (true, None) => s.positions(),
            (false, Some(i)) => children(i, &parents),
            (false, None) => all_items(s.top(), &prevs),
        };
        for i in items {
            if let Some(c) = s.checkbox(i) {
                all += 1;
                on += usize::from(c == "[X]");
            }
        }
    }
    (on, all)
}

/// org-update-checkbox-count. Returns the lines of updated cookies.
pub fn update_checkbox_count(b: &mut Buf, scope: Scope) -> Vec<usize> {
    let children_only = sexp::option("org-checkbox-children-only-statistics")
        .or_else(|| sexp::option("org-checkbox-hierarchical-statistics"))
        .is_none_or(|v| v.truthy());
    b.excursion(|b| {
        let (start, end) = match scope {
            Scope::All => (0, b.len()),
            Scope::Narrow(lo, hi) => (lo, hi),
            Scope::Section => (
                heading_before(b, b.pt).unwrap_or(0),
                next_heading(b, b.pt).unwrap_or(b.len()),
            ),
        };
        b.goto(start);
        let mut cookies: Vec<(usize, usize, bool, usize, usize)> = vec![];
        let mut cache: Vec<(usize, (usize, usize))> = vec![];
        while let Some(m) = b.re_search_forward(&COOKIE, end) {
            let (s, e) = m.get(0).unwrap();
            let percent = m.get(1).is_some();
            let after = b.pt;
            let data = entry_get(b, after, "COOKIE_DATA").unwrap_or_default();
            let recursive = !children_only || RECURSIVE.is_match(&data);
            if TODO_WORD.is_match(&data) {
                continue;
            }
            let Ok(container) = cookie_container(b, s) else {
                b.goto(after);
                continue;
            };
            b.goto(after);
            let (beg, cend, item) = match container {
                Some(c) => c,
                None => {
                    let beg = heading_before(b, after).unwrap_or(0);
                    (beg, next_heading(b, beg).unwrap_or(b.len()), None)
                }
            };
            let count = match cache.iter().find(|c| c.0 == beg) {
                Some(c) => c.1,
                None => {
                    let structs = b.excursion(|b| {
                        b.goto(beg);
                        let mut structs = vec![];
                        while let Some(c) = b.re_search_forward(&BOX_ITEM, cend) {
                            let at = c.beg(0).unwrap();
                            let ok = b.excursion(|b| {
                                b.goto(at);
                                at_item(b)
                            });
                            if ok {
                                let st = b.excursion(|b| {
                                    b.goto(at);
                                    list_struct(b)
                                });
                                let bottom = st.bottom();
                                structs.push(st);
                                b.goto(bottom.min(cend).max(b.pt));
                            }
                        }
                        structs
                    });
                    let n = count_boxes(item, &structs, recursive);
                    cache.push((beg, n));
                    n
                }
            };
            cookies.push((s, e, percent, count.0, count.1));
        }
        let mut lines = vec![];
        for (s, e, percent, on, all) in cookies.into_iter().rev() {
            let new = if percent {
                percent_cookie(on, all)
            } else {
                format!("[{on}/{all}]")
            };
            if b.sub(s, e) != new {
                b.delete(s, e);
                b.insert_at(s, &new);
            }
            lines.push(b.line_of(s));
        }
        b.cookie_lines.extend(&lines);
        lines
    })
}

/// org-update-checkbox-count-maybe.
pub fn update_checkbox_count_maybe(b: &mut Buf, scope: Scope) -> Vec<usize> {
    if rule("checkbox") {
        update_checkbox_count(b, scope)
    } else {
        vec![]
    }
}

/// Re-align tags on headings whose cookies changed (org-auto-align-tags).
fn align_tags(ed: &mut Editor, lines: &[usize]) {
    if lines.is_empty() || !options::bool("org-auto-align-tags", true) {
        return;
    }
    let cur = ed.cur;
    for &l in lines {
        if l < ed.line_count()
            && syntax::level(&ed.buf.line(l)).is_some()
            && ed.buf.line(l).trim_end().ends_with(':')
        {
            ed.cur.line = l;
            ed.cur.byte = 0;
            let _ = super::call(ed, "org-align-tags", Prefix::None);
        }
    }
    ed.cur = cur;
}

/// Update checkbox statistics in the editor (for other modules).
pub fn update_checkbox_count_in(ed: &mut Editor, scope: Scope, maybe: bool) {
    let _ = with_buf(ed, |b| {
        if maybe {
            update_checkbox_count_maybe(b, scope);
        } else {
            update_checkbox_count(b, scope);
        }
        Ok(())
    });
}

// ---- interactive commands ----

/// Is `ORDERED` set on the entry at point.
fn ordered(b: &Buf, not_nil: bool) -> bool {
    entry_get(b, b.pt, "ORDERED").is_some_and(|v| !not_nil || (v != "nil" && !v.is_empty()))
}

fn not_in_item() -> String {
    "Not in an item".into()
}

/// Navigation commands.
fn goto_cmd(ed: &mut Editor, which: &str) -> Result<(), String> {
    let circ = circular();
    with_buf(ed, |b| {
        let item = in_item(b).ok_or_else(not_in_item)?;
        b.goto(item);
        if which == "begin" {
            return Ok(());
        }
        let st = list_struct(b);
        let prevs = st.prevs();
        let to = match which {
            "list-begin" => list_begin(item, &prevs),
            "list-end" => list_end(&st, item, &prevs),
            "end" => st.end(item),
            "prev" => match prev_item(item, &prevs) {
                Some(p) => p,
                None if circ => last_item(item, &prevs),
                None => return Err("On first item".into()),
            },
            _ => match next_item(item, &prevs) {
                Some(p) => p,
                None if circ => list_begin(item, &prevs),
                None => return Err("On last item".into()),
            },
        };
        b.goto(to);
        Ok(())
    })
}

fn move_item(ed: &mut Editor, down: bool) -> Result<(), String> {
    with_buf(ed, |b| {
        if !at_item(b) {
            return Err("Not at an item".into());
        }
        let col = b.column();
        let item = b.bol();
        let mut st = list_struct(b);
        let prevs = st.prevs();
        if down {
            let next = next_item(item, &prevs);
            if next.is_none() && !circular() {
                return Err("Cannot move this item further down".into());
            }
            match next {
                None => st = send_item(b, item, Dest::Begin, &st).0,
                Some(n) => {
                    swap_items(b, item, n, &mut st);
                    if let Some(p) = next_item(item, &st.prevs()) {
                        b.goto(p);
                    }
                }
            }
        } else {
            let prev = prev_item(item, &prevs);
            if prev.is_none() && !circular() {
                return Err("Cannot move this item further up".into());
            }
            match prev {
                None => st = send_item(b, item, Dest::End, &st).0,
                Some(p) => swap_items(b, p, item, &mut st),
            }
        }
        let parents = st.parents();
        write_struct(b, &mut st, &parents, None);
        b.move_to_column(col);
        Ok(())
    })
}

/// org-insert-item: false when not in a (visible) item.
pub fn insert_item_cmd(ed: &mut Editor, checkbox: bool) -> Result<bool, String> {
    let mut timer = false;
    let r = with_buf(ed, |b| {
        let pos = b.pt;
        let Some(itemp) = in_item(b) else {
            return Ok(false);
        };
        if b.invisible(itemp) {
            return Ok(false);
        }
        if b.excursion(|b| {
            b.goto(itemp);
            at_item_timer(b)
        }) {
            timer = true;
            return Ok(true);
        }
        let mut st = b.excursion(|b| {
            b.goto(itemp);
            list_struct(b)
        });
        let prevs = st.prevs();
        let desc = (st.list_type(itemp, &prevs) == ListType::Descriptive).then_some(" :: ");
        insert_item(b, pos, &mut st, &prevs, checkbox, desc.unwrap_or(""), None);
        let parents = st.parents();
        write_struct(b, &mut st, &parents, None);
        if checkbox {
            update_checkbox_count_maybe(b, Scope::Section);
        }
        b.forward_line(0);
        let c = b.looking_at(&FULL_ITEM).unwrap_or_default();
        let to = match c.beg(4) {
            Some(s) if c.str(b, 1).is_some_and(|x| x.contains(['.', ')'])) => s,
            _ => c.end(0).unwrap_or(b.pt),
        };
        b.goto(to);
        if desc.is_some() {
            b.goto(b.pt.saturating_sub(1));
        }
        Ok(true)
    });
    if timer {
        super::call(ed, "org-timer-item", Prefix::None)?;
    }
    r
}

/// org-list-repair.
fn repair(b: &mut Buf) -> Result<(), String> {
    if !at_item(b) {
        return Err("This is not a list".into());
    }
    let mut st = list_struct(b);
    let parents = st.parents();
    write_struct(b, &mut st, &parents, None);
    Ok(())
}

/// The bullet org-cycle-list-bullet moves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Which {
    Next,
    Previous,
    Bullet(String),
    Index(i64),
}

/// org-cycle-list-bullet.
pub fn cycle_list_bullet(b: &mut Buf, which: &Which) -> Result<(), String> {
    if !at_item(b) {
        return Err("Not at an item".into());
    }
    let origin = b.marker(b.pt, false);
    b.forward_line(0);
    let mut st = list_struct(b);
    let parents = st.parents();
    let prevs = st.prevs();
    let bol = b.pt;
    let list_beg = list_begin(bol, &prevs);
    let o = b.mpos(origin) as i64;
    let off1 = o - (bol + st.ind(bol)) as i64;
    let off2 = o - (bol + st.ind(bol) + st.bullet(bol).len()) as i64;
    let bullet = st.bullet(list_beg).to_owned();
    let alpha = use_alpha(list_beg, &st, &prevs);
    let current = [
        (r"[a-z]\.", "a."),
        (r"[a-z]\)", "a)"),
        (r"[A-Z]\.", "A."),
        (r"[A-Z]\)", "A)"),
        (r"\.", "1."),
        (r"\)", "1)"),
    ]
    .iter()
    .find(|(r, _)| re(r).is_match(&bullet))
    .map_or_else(|| bullet.trim().to_owned(), |(_, k)| (*k).to_owned());
    let term = terminator();
    let desc = at_item_description(b);
    let mut list: Vec<&str> = vec!["-", "+"];
    if !b.looking_p(&re(r"\A\S")) {
        list.push("*");
    }
    if !(term == Some(')') || desc) {
        list.push("1.");
    }
    if !(term == Some('.') || desc) {
        list.push("1)");
    }
    if alpha && !(term == Some(')') || desc) {
        list.extend(["a.", "A."]);
    }
    if alpha && !(term == Some('.') || desc) {
        list.extend(["a)", "A)"]);
    }
    let len = list.len() as i64;
    let idx = list
        .iter()
        .position(|x| *x == current)
        .map_or(len, |i| i as i64);
    let get = |i: i64| list[i.rem_euclid(len) as usize].to_owned();
    let new = match which {
        Which::Bullet(s) if list.contains(&s.as_str()) => s.clone(),
        Which::Index(n) => get(*n),
        Which::Previous => get(idx - 1),
        _ => get(idx + 1),
    };
    let old = st.clone();
    st.item_mut(list_beg).bullet = bullet_string(&new);
    fix_bul(&mut st, &prevs);
    fix_ind(&mut st, &parents, None);
    apply_struct(b, &st, &old);
    b.goto(b.mpos(origin));
    let st = list_struct(b);
    b.forward_line(0);
    let bol = b.pt;
    if off2 >= 0 {
        b.goto(bol + st.ind(bol) + st.bullet(bol).len() + off2 as usize);
    } else if off1 >= 0 {
        b.goto(bol + st.ind(bol) + off1 as usize);
    } else {
        b.goto(b.mpos(origin));
    }
    b.free(origin);
    Ok(())
}

/// org-at-radio-list-p.
pub fn at_radio_list(b: &mut Buf) -> bool {
    if !at_item(b) {
        return false;
    }
    let item = b.bol();
    let st = b.excursion(|b| {
        b.goto(item);
        list_struct(b)
    });
    if st.get(item).is_none() {
        return false;
    }
    let first = list_begin(item, &st.prevs());
    let r = re(r"(?i)^[ \t]*#\+attr_org:.* :radio (\S+)");
    let mut l = first;
    while l > 0 {
        l = b.bol_at(l - 1);
        let t = b.sub(l, b.eol_at(l));
        if !t.trim_start().starts_with("#+") {
            break;
        }
        if let Some(c) = r.captures(t) {
            return &c[1] != "nil";
        }
    }
    false
}

/// org-toggle-radio-button.
pub fn toggle_radio_button(b: &mut Buf, arg: Prefix) -> Result<(), String> {
    if !at_item(b) {
        return Err("Cannot toggle checkbox outside of a list".into());
    }
    let cpos = in_item(b).unwrap_or(b.bol());
    let mut st = list_struct(b);
    let ord = ordered(b, false);
    let parents = st.parents();
    let old = st.clone();
    let cbox = st.checkbox(cpos).map(str::to_owned);
    let prevs = st.prevs();
    let start = list_begin(b.bol(), &prevs);
    let new = if cbox.is_some() && arg.universal() == 1 && start == cpos {
        None
    } else {
        Some("[ ]".to_owned())
    };
    for p in all_items(start, &prevs) {
        st.item_mut(p).checkbox = new.clone();
    }
    if new.is_some() {
        st.item_mut(cpos).checkbox = match arg.universal() {
            1 => cbox.is_none().then(|| "[ ]".into()),
            2 => cbox.is_none().then(|| "[-]".into()),
            _ => Some(
                if cbox.as_deref() == Some("[X]") {
                    "[ ]"
                } else {
                    "[X]"
                }
                .into(),
            ),
        };
    }
    fix_box(&mut st, &parents, &prevs, ord);
    apply_struct(b, &st, &old);
    update_checkbox_count_maybe(b, Scope::Section);
    Ok(())
}

/// org-toggle-checkbox on the buffer; `region` as positions.
pub fn toggle_checkbox(
    b: &mut Buf,
    arg: Prefix,
    region: Option<(usize, usize)>,
) -> Result<(), String> {
    if at_radio_list(b) {
        return toggle_radio_button(b, arg);
    }
    let ord = ordered(b, false);
    let item_begin = item_res().1.clone();
    let r = b.excursion(|b| -> Result<(), String> {
        let mut singlep = false;
        let (lim_up, lim_down) = if let Some((rb, re_)) = region {
            b.goto(rb);
            if search_forward(b, &item_begin, re_, false).is_none() {
                return Err("No item in region".into());
            }
            (b.bol(), b.marker(re_, false))
        } else if heading_at(b, b.pt) {
            let limit = next_heading(b, b.pt).unwrap_or(b.len());
            end_of_meta_data(b, Some(true));
            if search_forward(b, &item_begin, limit, false).is_none() {
                return Err("No item in subtree".into());
            }
            (b.bol(), b.marker(limit, false))
        } else if at_item(b) {
            singlep = true;
            let e = b.eol();
            (b.bol(), b.marker(e, false))
        } else {
            return Err("Not at an item or heading, and no active region".into());
        };
        b.goto(lim_up);
        let cbox = at_item_checkbox(b).map(|(s, e)| b.sub(s, e).to_owned());
        let refbox = match arg.universal() {
            2 => Some("[-]".to_owned()),
            1 => cbox.is_none().then(|| "[ ]".to_owned()),
            _ if cbox.as_deref() == Some("[X]") => Some("[ ]".to_owned()),
            _ => Some("[X]".to_owned()),
        };
        b.goto(lim_up);
        let mut result = Ok(());
        while b.pt < b.mpos(lim_down)
            && search_forward(b, &item_begin, b.mpos(lim_down), true).is_some()
        {
            let mut st = list_struct(b);
            let copy = st.clone();
            let parents = st.parents();
            let prevs = st.prevs();
            let bottom = b.marker(st.bottom(), false);
            let down = b.mpos(lim_down);
            for e in st.positions() {
                if e < lim_up || e > down {
                    continue;
                }
                let cur = st.checkbox(e).map(str::to_owned);
                if cur.is_some() || arg.universal() == 1 {
                    st.item_mut(e).checkbox = refbox.clone();
                }
            }
            let block = fix_box(&mut st, &parents, &prevs, ord);
            if let Some(bi) = block {
                if singlep && lim_up > bi {
                    result = Err(format!(
                        "Checkbox blocked because of unchecked box at line {}",
                        b.line_number(bi)
                    ));
                    break;
                }
                b.msg = Some(format!(
                    "Checkboxes were removed due to unchecked box at line {}",
                    b.line_number(bi)
                ));
            }
            b.goto(b.mpos(bottom));
            b.free(bottom);
            apply_struct(b, &st, &copy);
        }
        b.free(lim_down);
        result
    });
    update_checkbox_count_maybe(b, Scope::Section);
    r
}

/// org-ctrl-c-ctrl-c on an item or at the start of a plain list.
pub fn list_ctrl_c_ctrl_c(b: &mut Buf, arg: Prefix, radio_mode: bool) -> Result<(), String> {
    if !at_item(b) {
        return Err("Not at an item".into());
    }
    if at_radio_list(b) || radio_mode {
        return toggle_radio_button(b, arg);
    }
    let bol = b.bol();
    let mut st = list_struct(b);
    let old = st.clone();
    let parents = st.parents();
    let prevs = st.prevs();
    let ord = ordered(b, true);
    let is_list = b.pt == bol && prev_item(bol, &prevs).is_none();
    if is_list {
        let first_box = st.checkbox(bol).map(str::to_owned);
        let new_box = match arg.universal() {
            2 => Some("[-]".into()),
            1 => first_box.is_none().then(|| "[ ]".into()),
            _ if first_box.as_deref() == Some("[X]") => Some("[ ]".into()),
            _ => Some("[X]".into()),
        };
        if !arg.is_none() {
            for p in all_items(bol, &prevs) {
                st.item_mut(p).checkbox = new_box.clone();
            }
        } else if first_box.is_some() {
            st.item_mut(bol).checkbox = new_box;
        }
        write_struct(b, &mut st, &parents, Some(&old));
        if st == old {
            b.msg = Some("Cannot update this checkbox".into());
        }
        update_checkbox_count_maybe(b, Scope::Section);
        return Ok(());
    }
    let bx = st.checkbox(bol).map(str::to_owned);
    st.item_mut(bol).checkbox = match (arg.universal(), bx.as_deref()) {
        (2, _) => Some("[-]".into()),
        (1, None) => Some("[ ]".into()),
        (1, _) | (_, None) => None,
        (_, Some("[X]")) => Some("[ ]".into()),
        _ => Some("[X]".into()),
    };
    fix_ind(&mut st, &parents, Some(2));
    fix_item_end(&mut st);
    fix_bul(&mut st, &prevs);
    fix_ind(&mut st, &parents, None);
    let block = fix_box(&mut st, &parents, &prevs, ord);
    if bx.is_some() && st == old {
        if arg.universal() == 2 {
            b.msg = Some("Checkboxes already reset".into());
        } else {
            return Err(format!(
                "Cannot toggle this checkbox: {}",
                if bx.as_deref() == Some("[X]") {
                    "all subitems checked"
                } else {
                    "unchecked subitems"
                }
            ));
        }
    } else {
        apply_struct(b, &st, &old);
        update_checkbox_count_maybe(b, Scope::Section);
    }
    if let Some(bi) = block {
        b.msg = Some(format!(
            "Checkboxes were removed due to empty box at line {}",
            b.line_number(bi)
        ));
    }
    Ok(())
}

/// org-list-indent-item-generic. `zone` is the reused begin/end markers
/// of a repeated S-M-left/right; returns the zone used.
pub fn indent_item_generic(
    b: &mut Buf,
    arg: i64,
    no_subtree: bool,
    st: &mut Struct,
    region: Option<(usize, usize)>,
    zone: Option<(usize, usize)>,
    odd_levels: bool,
) -> Result<(usize, usize), String> {
    b.excursion(|b| {
        let regionp = region.is_some();
        let top = st.top();
        let parents = st.parents();
        let prevs = st.prevs();
        let bol = b.bol();
        let specialp = !regionp && top == bol && rule("indent");
        if specialp && no_subtree {
            return Err("At first item: use S-M-<left/right> to move the whole list".into());
        }
        let (beg, end) = match (zone, region) {
            (Some(z), _) => z,
            (None, Some(r)) => r,
            (None, None) => (
                bol,
                if specialp {
                    st.bottom()
                } else if no_subtree {
                    bol + 1
                } else {
                    st.get(bol).map_or(bol + 1, |i| i.end)
                },
            ),
        };
        if specialp {
            let skip = if odd_levels { 2 } else { 1 };
            let offset: i64 = if arg < 0 { -skip } else { skip };
            let top_ind = st.ind(beg) as i64;
            let old = st.clone();
            if top_ind + offset < 0 {
                return Err("Cannot outdent beyond margin".into());
            }
            if top_ind + offset == 0 && st.bullet(beg).contains('*') {
                st.item_mut(beg).bullet = bullet_string("-");
            }
            for it in &mut st.0 {
                it.ind = (it.ind as i64 + offset) as usize;
            }
            fix_bul(st, &prevs);
            apply_struct(b, st, &old);
            return Ok((beg, end));
        }
        if arg < 0 {
            let last = st.0.iter().rev().find(|e| e.pos < end).map(|e| e.pos);
            if (no_subtree && !regionp && st.has_child(beg).is_some())
                || last.is_some_and(|l| st.has_child(l).is_some())
            {
                return Err("Cannot outdent an item without its children".into());
            }
        }
        let old = st.clone();
        let new_parents = if arg < 0 {
            struct_outdent(beg, end, &parents)?
        } else {
            struct_indent(beg, end, st, &parents, &prevs)?
        };
        let (mb, me) = (b.marker(beg, false), b.marker(end, false));
        write_struct(b, st, &new_parents, Some(&old));
        update_checkbox_count_maybe(b, Scope::Section);
        let z = (b.mpos(mb), b.mpos(me));
        b.free(mb);
        b.free(me);
        Ok(z)
    })
}

/// org-indent-item, org-outdent-item and their -tree variants.
fn indent_cmd(ed: &mut Editor, arg: i64, no_subtree: bool) -> Result<(), String> {
    let region = region_pos(ed);
    let version = ed.buf.version;
    let repeat = matches!(
        last_command(ed),
        Some("org-shiftmetaright" | "org-shiftmetaleft")
    );
    let zone = ed
        .org
        .as_ref()
        .and_then(|o| o.list.indent_zone)
        .filter(|z| repeat && z.0 == version)
        .map(|z| (z.1, z.2));
    let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
    let r = with_buf(ed, |b| {
        let at = at_item(b)
            || region.is_some_and(|(rb, _)| {
                b.excursion(|b| {
                    b.goto(rb);
                    at_item(b)
                })
            });
        if !at {
            return Err(if region.is_some() {
                "Region not starting at an item"
            } else {
                "Not at an item"
            }
            .into());
        }
        let mut st = match region {
            Some((rb, _)) => b.excursion(|b| {
                b.goto(rb);
                list_struct(b)
            }),
            None => list_struct(b),
        };
        indent_item_generic(b, arg, no_subtree, &mut st, region, zone, odd)
    });
    let v = ed.buf.version;
    if let (Ok(z), Some(s)) = (&r, state(ed)) {
        s.indent_zone = Some((v, z.0, z.1));
    }
    r.map(|_| ())
}

/// org-reset-checkbox-state-subtree.
fn reset_checkbox_state_subtree(ed: &mut Editor) -> Result<(), String> {
    let Some(h) = fold::back_to_heading(ed, ed.cur.line) else {
        return Err("Not inside a tree".into());
    };
    fold::show_subtree(ed, h);
    let end_line = syntax::subtree_end(&ed.buf, h);
    with_buf(ed, |b| {
        b.excursion(|b| {
            let lo = line_start(b, h);
            let hi = line_start(b, end_line);
            b.goto(lo);
            while b.pt < hi && !b.eobp() {
                if let Some((s, e)) = at_item_checkbox(b) {
                    b.delete(s, e);
                    b.insert_at(s, "[ ]");
                }
                b.forward_line(1);
            }
            let hi = line_start(b, end_line);
            update_checkbox_count_maybe(b, Scope::Narrow(lo, hi));
        });
        Ok(())
    })
}

fn line_start(b: &Buf, l: usize) -> usize {
    let mut p = 0;
    for _ in 0..l {
        let e = b.eol_at(p);
        if e >= b.len() {
            return b.len();
        }
        p = e + 1;
    }
    p
}

// ---- org-toggle-item ----

static FN_DEF: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^\[fn:([-_\w]+)\]"));
static FN_END: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?m)^\*+ |^\[fn:[-_\w]+\]|^(?:[ \t]*\n){2,}"));

/// Remove footnote definitions from point to `end` (a marker); return them.
fn extract_footnotes(b: &mut Buf, end: buf::Marker) -> Vec<String> {
    let mut defs = vec![];
    b.excursion(|b| {
        while let Some(c) = b.re_search_forward(&FN_DEF, b.mpos(end)) {
            let beg = c.beg(0).unwrap();
            if !valid_context(b) {
                continue;
            }
            b.goto(c.end(0).unwrap());
            let lim = next_heading(b, beg).unwrap_or(b.len());
            let content_end = match FN_END.find_at(&b.s[..lim], b.pt) {
                Some(m) => m.start(),
                None => lim,
            };
            b.goto(content_end);
            b.skip_forward(" \t\n\r");
            let eend = if b.eobp() { b.pt } else { b.bol() };
            // Blank lines after the contents.
            let mut cend = content_end;
            while cend > beg && matches!(b.s.as_bytes()[cend - 1], b' ' | b'\t' | b'\n') {
                cend -= 1;
            }
            let cend = b.eol_at(cend);
            let post_blank = b.sub(cend, eend).matches('\n').count().saturating_sub(1);
            let mut def = b.sub(beg, eend).to_owned();
            if post_blank < 2 {
                def.push_str(&"\n".repeat(2 - post_blank));
            }
            defs.insert(0, def);
            b.delete(beg, eend);
            b.goto(beg);
        }
    });
    defs
}

/// Shift text from point to `end` so the least indented line is at `ind`.
fn shift_text(b: &mut Buf, ind: usize, end: usize) {
    let end = b.marker(end, false);
    let mut min_i = 1000;
    b.excursion(|b| {
        while b.pt < b.mpos(end) {
            let i = b.ind();
            if b.looking_p(&BLANK) || heading_at(b, b.pt) {
            } else if i == 0 {
                min_i = 0;
                break;
            } else if i < min_i {
                min_i = i;
            }
            let before = b.pt;
            b.forward_line(1);
            if b.pt == before {
                break;
            }
        }
    });
    let delta = ind as i64 - min_i as i64;
    while b.pt < b.mpos(end) {
        if !(b.looking_p(&BLANK) || heading_at(b, b.pt)) {
            let n = (b.ind() as i64 + delta).max(0) as usize;
            b.indent_line_to(n);
        }
        let before = b.pt;
        b.forward_line(1);
        if b.pt == before {
            break;
        }
    }
    b.free(end);
}

/// org-list--delete-metadata at the heading at point.
fn delete_metadata(b: &mut Buf, st: &Settings) {
    b.excursion(|b| {
        let bol = b.bol();
        let line = b.sub(bol, b.eol()).to_owned();
        if let Some(h) = syntax::headline(&line, st)
            && let Some(r) = h.tags_range
        {
            b.delete(bol + r.start, bol + r.end);
        }
        let from = b.lbp(2);
        b.goto(bol);
        end_of_meta_data(b, None);
        b.skip_forward(" \t\n\r");
        let to = if b.eobp() { b.pt } else { b.bol() };
        if to > from {
            b.delete(from, to);
        }
    });
}

/// org-reduced-level.
fn reduced_level(l: usize, odd: bool) -> usize {
    if l == 0 {
        0
    } else if odd {
        l / 2 + 1
    } else {
        l
    }
}

/// org-toggle-item.
pub fn toggle_item(
    b: &mut Buf,
    arg: bool,
    region: Option<(usize, usize)>,
    st: &Settings,
) -> Result<(), String> {
    let odd = st.opt_bool("org-odd-levels-only", false);
    let (beg, end) = match region {
        Some((rb, re_)) => {
            let beg = b.excursion(|b| {
                b.goto(rb);
                b.skip_forward(" \r\t\n");
                b.bol()
            });
            (beg, b.marker(re_, false))
        }
        None => {
            let e = b.eol();
            (b.bol(), b.marker(e, false))
        }
    };
    b.excursion(|b| {
        b.goto(beg);
        if at_item(b) {
            while b.pt < b.mpos(end) {
                if at_item(b) {
                    let e = b
                        .looking_at_pos(&item_res().0, b.bol())
                        .and_then(|c| c.end(0))
                        .unwrap();
                    b.skip_forward(" \t");
                    b.delete(b.pt, e);
                }
                let before = b.pt;
                b.forward_line(1);
                if b.pt == before {
                    break;
                }
            }
        } else if heading_at(b, b.pt) {
            delete_metadata(b, st);
            let bul = bullet_string("-");
            let bl = bul.len();
            let adapt = options::string("org-adapt-indentation", "nil") != "nil";
            let start_ind = if !adapt {
                0
            } else {
                match b.bol().checked_sub(1).and_then(|p| heading_before(b, p)) {
                    Some(h) => syntax::level(b.sub(h, b.eol_at(h))).unwrap_or(0) + 1,
                    None => 0,
                }
            };
            let mut ref_level =
                reduced_level(syntax::level(b.sub(b.bol(), b.eol())).unwrap_or(1), odd);
            let defs = extract_footnotes(b, end);
            while b.pt < b.mpos(end) {
                let line = b.sub(b.bol(), b.eol()).to_owned();
                let level = reduced_level(syntax::level(&line).unwrap_or(1), odd);
                let delta = level.saturating_sub(ref_level);
                let todo = syntax::headline(&line, st).and_then(|h| h.todo);
                if level < ref_level {
                    ref_level = level;
                }
                delete_metadata(b, st);
                // Remove stars and the TODO keyword.
                let line = b.sub(b.bol(), b.eol()).to_owned();
                let bol = b.bol();
                let mut p = line.bytes().take_while(|c| *c == b'*').count();
                let mut title = None;
                let sp = line[p..].bytes().take_while(|c| *c == b' ').count();
                if sp > 0 {
                    p += sp;
                    title = Some(p);
                    let w = line[p..].find([' ', '\t']).map_or(line.len(), |i| p + i);
                    if w > p && st.todo_names().contains(&&line[p..w]) {
                        let sp2 = line[w..].bytes().take_while(|c| *c == b' ').count();
                        title = if sp2 > 0 { Some(w + sp2) } else { None };
                    }
                }
                let del_to = title.map_or(b.eol(), |t| bol + t);
                b.delete(bol, del_to);
                b.goto(bol);
                b.insert(&bul);
                b.indent_line_to(start_ind + delta * bl);
                if let Some(t) = todo {
                    let mut s = list_struct(b);
                    let old = s.clone();
                    let here = b.bol();
                    s.item_mut(here).checkbox =
                        Some(if st.is_done(&t) { "[X]" } else { "[ ]" }.into());
                    let parents = s.parents();
                    write_struct(b, &mut s, &parents, Some(&old));
                }
                let section_end = next_heading(b, b.pt).unwrap_or(b.len());
                b.forward_line(1);
                let lim = b.mpos(end).min(section_end);
                shift_text(b, start_ind + (delta + 1) * bl, lim);
                if b.pt < lim {
                    b.goto(lim);
                }
                if b.eobp() {
                    break;
                }
            }
            if !defs.is_empty() {
                b.goto(b.mpos(end));
                if !b.bolp() {
                    b.forward_line(1);
                }
                if !b.bolp() {
                    b.insert("\n");
                }
                for d in defs {
                    b.insert(&d);
                }
            }
        } else if arg {
            let bul = bullet_string("-");
            let ref_ind = b.ind();
            let defs = extract_footnotes(b, end);
            b.skip_forward(" \t");
            b.insert(&bul);
            b.forward_line(1);
            while b.pt < b.mpos(end) {
                let lim = b
                    .mpos(end)
                    .min(next_heading(b, b.pt.saturating_sub(1).max(b.bol())).unwrap_or(b.len()));
                let lim = if heading_at(b, b.pt) { b.pt } else { lim };
                shift_text(b, ref_ind + bul.len(), lim);
                let before = b.pt;
                b.forward_line(1);
                if b.pt == before {
                    break;
                }
            }
            if !defs.is_empty() {
                let e = b.mpos(end).saturating_sub(1);
                b.goto(e);
                let lend = if at_item(b) || in_item(b).is_some() {
                    let it = in_item(b).unwrap_or(b.bol());
                    b.goto(it);
                    let s = list_struct(b);
                    let mut p = s.bottom();
                    b.goto(p);
                    b.skip_forward(" \t\n");
                    p = if b.eobp() { b.pt } else { b.bol() };
                    p
                } else {
                    e
                };
                b.goto(lend);
                if !b.bolp() {
                    b.forward_line(1);
                }
                if !b.bolp() {
                    b.insert("\n");
                }
                for d in defs {
                    b.insert(&d);
                }
            }
        } else {
            let r = re(r"\A([ \t]*)(\S)");
            while b.pt < b.mpos(end) {
                if !(heading_at(b, b.pt) || at_item(b))
                    && let Some(c) = b.looking_at(&r)
                {
                    let p = c.end(1).unwrap();
                    b.insert_at(p, &bullet_string("-"));
                }
                let before = b.pt;
                b.forward_line(1);
                if b.pt == before {
                    break;
                }
            }
        }
    });
    b.free(end);
    Ok(())
}

// ---- sorting ----

/// A sort key (org-sort-list's value-to-sort).
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub enum Key {
    Num(f64),
    Str(String),
}

/// A custom key extractor: the item's text from its bullet line on.
pub type GetKey<'a> = &'a dyn Fn(&str) -> Key;
/// A custom comparison (`a < b`).
pub type Compare<'a> = &'a dyn Fn(&Key, &Key) -> bool;

static SORT_ITEM: LazyLock<Regex> =
    LazyLock::new(|| re(r"\A[ \t]*[-+*0-9.)]+([ \t]+\[[- X]\])?[ \t]+"));

/// Seconds for an Org timestamp `<2017-05-08 Mon 10:00>` (UTC).
pub fn timestamp_seconds(ts: &str) -> Option<f64> {
    let c = re(r"(\d{4})-(\d{2})-(\d{2})(?:[^0-9>\]]*?(\d{1,2}):(\d{2}))?").captures(ts)?;
    let n = |i: usize| {
        c.get(i)
            .map_or(0, |m| m.as_str().parse::<i64>().unwrap_or(0))
    };
    let (y, m, d) = (n(1), n(2), n(3));
    // days_from_civil.
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + n(4) * 3600 + n(5) * 60) as f64)
}

/// org-sort-list on the list at point.
pub fn sort_list(
    b: &mut Buf,
    with_case: bool,
    sorting_type: char,
    getkey: Option<GetKey>,
    compare: Option<Compare>,
) -> Result<(), String> {
    if !at_item(b) && in_item(b).is_none() {
        return Err("Not at an item".into());
    }
    let st = list_struct(b);
    let prevs = st.prevs();
    let here = in_item(b).unwrap_or(b.bol());
    let start = list_begin(here, &prevs);
    let end = list_end(&st, here, &prevs);
    let dcst = sorting_type.to_ascii_lowercase();
    if !"antfx".contains(dcst) {
        return Err(format!("Invalid sorting type `{sorting_type}'"));
    }
    if dcst == 'f' && getkey.is_none() {
        return Err("Missing key extractor".into());
    }
    let ts = re(r"<\d{4}-\d{2}-\d{2}[^>\n]*>");
    let ts_both = re(r"[<\[]\d{4}-\d{2}-\d{2}[^>\]\n]*[>\]]");
    let items = all_items(start, &prevs);
    let mut records = vec![];
    for (i, &it) in items.iter().enumerate() {
        let rend = st.end_before_blank(b, it);
        let next = items.get(i + 1).copied().unwrap_or(end);
        let line = b.sub(it, b.eol_at(it)).to_owned();
        let key = b.looking_at_pos(&SORT_ITEM, it).map(|c| {
            let after = b.sub(c.end(0).unwrap(), b.eol_at(it)).to_owned();
            let fold = |s: String| if with_case { s } else { s.to_lowercase() };
            match dcst {
                'n' => Key::Num(string_to_number(&after)),
                'a' => Key::Str(fold(after)),
                't' => {
                    let mut tb = Buf::new(b.s.clone(), it);
                    let v = if let Some(c) = at_regexp_after_bullet(&mut tb, &TIMER) {
                        hms_to_secs(c.str(&tb, 1).unwrap_or(""))
                    } else if let Some(m) = ts.find(&line).or_else(|| ts_both.find(&line)) {
                        timestamp_seconds(m.as_str()).unwrap_or(0.0)
                    } else {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0.0, |d| d.as_secs_f64())
                    };
                    Key::Num(v)
                }
                'x' => Key::Str(c.str(b, 1).map_or(String::new(), |s| s.trim().to_owned())),
                _ => match getkey.map(|f| f(b.sub(it, rend))) {
                    Some(Key::Str(s)) => Key::Str(fold(s)),
                    Some(k) => k,
                    None => Key::Str(String::new()),
                },
            }
        });
        let body = b.sub(it, rend).to_owned();
        let tail = b.sub(rend, next).to_owned();
        records.push((key, body, tail));
    }
    let reverse = dcst != sorting_type;
    let less = |a: &Option<Key>, c: &Option<Key>| -> bool {
        match (a, c) {
            (Some(x), Some(y)) => match compare {
                Some(f) => f(x, y),
                None => match (x, y) {
                    (Key::Num(p), Key::Num(q)) => p < q,
                    (Key::Str(p), Key::Str(q)) => p < q,
                    _ => false,
                },
            },
            (None, Some(_)) => true,
            _ => false,
        }
    };
    let mut order: Vec<usize> = (0..records.len()).collect();
    if reverse {
        order.reverse();
    }
    order.sort_by(|&i, &j| {
        if less(&records[i].0, &records[j].0) {
            std::cmp::Ordering::Less
        } else if less(&records[j].0, &records[i].0) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    if reverse {
        order.reverse();
    }
    let mut out = String::new();
    for (k, &i) in order.iter().enumerate() {
        out.push_str(&records[i].1);
        out.push_str(&records[k].2);
    }
    b.delete(start, end);
    b.goto(start);
    b.insert(&out);
    b.goto(start);
    repair(b)?;
    b.msg = Some("Sorting items...done".into());
    Ok(())
}

/// string-to-number (leading number, else 0).
fn string_to_number(s: &str) -> f64 {
    let t = s.trim_start();
    let m = re(r"^[-+]?[0-9]*\.?[0-9]+(?:[eE][-+]?[0-9]+)?").find(t);
    m.and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0)
}

/// org-timer-hms-to-secs.
fn hms_to_secs(s: &str) -> f64 {
    let v: Vec<f64> = s.split(':').map(|x| x.parse().unwrap_or(0.0)).collect();
    v.iter().fold(0.0, |a, x| a * 60.0 + x)
}

// ---- visibility (TAB) ----

/// org-list-set-item-visibility.
fn set_item_visibility(ed: &mut Editor, b: &Buf, item: usize, st: &Struct, view: &str) {
    match view {
        "folded" => {
            let l = b.line_of(item);
            let e = b.line_of(st.end_before_blank(b, item));
            fold::region(ed, l + 1, e, true, Spec::Outline);
        }
        "children" => {
            set_item_visibility(ed, b, item, st, "subtree");
            let parents = st.parents();
            for c in children(item, &parents) {
                set_item_visibility(ed, b, c, st, "folded");
            }
        }
        _ => {
            let end = st.end(item);
            let mut e = b.line_of(end);
            if end > item && b.bol_at(end) == end {
                e -= 1;
            }
            fold::region(ed, b.line_of(item), e, false, Spec::Outline);
        }
    }
}

/// org-cycle on a list item (org-cycle-internal-local for items): None
/// when the cursor is not on an item's first line.
pub fn cycle_item(ed: &mut Editor, repeat: bool) -> Option<Result<(), String>> {
    if sexp::option("org-cycle-include-plain-lists").is_some_and(|v| v.is_nil()) {
        return None;
    }
    if options::string("org-cycle-emulate-tab", "t") == "exc-hl-bol" && ed.cur.byte != 0 {
        return None;
    }
    let mut b = load(ed);
    if !at_item(&mut b) {
        return None;
    }
    b.forward_line(0);
    let item = b.pt;
    let st = list_struct(&mut b);
    let eoh = b.eol();
    let eos = st.end_before_blank(&b, item);
    let has_children = st.has_child(item).is_some();
    let l = b.line_of(item);
    let el = b.line_of(eos);
    let set_status = |ed: &mut Editor, s: Option<&'static str>| {
        if let Some(o) = &mut ed.org {
            o.subtree_status = s;
        }
    };
    let reveal_next = |ed: &mut Editor| {
        if let Some(h) =
            (el + 1..ed.line_count()).find(|&i| syntax::level(&ed.buf.line(i)).is_some())
            && ed.folds.hidden(h)
        {
            fold::show_heading(ed, h);
        }
    };
    if eos <= eoh {
        ed.set_msg("EMPTY ENTRY");
        set_status(ed, None);
        reveal_next(ed);
        return Some(Ok(()));
    }
    let total = ed.line_count();
    let vis = ed.folds.down(l, 1, total);
    let blank = |s: String| s.trim().is_empty();
    let rest_blank = vis <= l || vis > el || (vis..=el).all(|i| blank(ed.buf.line(i)));
    let skip = options::bool("org-cycle-skip-children-state-if-no-children", true);
    let status = ed.org.as_ref().and_then(|o| o.subtree_status);
    if rest_blank && (has_children || !skip) {
        set_item_visibility(ed, &b, item, &st, "children");
        ed.set_msg("CHILDREN");
        reveal_next(ed);
        set_status(ed, Some("children"));
    } else if (rest_blank && !has_children && skip) || (repeat && status == Some("children")) {
        fold::region(ed, l + 1, el, false, Spec::Outline);
        let skipped = rest_blank && !has_children && skip;
        ed.set_msg(if skipped {
            "SUBTREE (NO CHILDREN)"
        } else {
            "SUBTREE"
        });
        set_status(ed, Some("subtree"));
    } else {
        fold::region(ed, l + 1, el, true, Spec::Outline);
        ed.set_msg("FOLDED");
        set_status(ed, Some("folded"));
    }
    Some(Ok(()))
}

/// org-cycle-item-indentation: TAB on an empty item cycles its level.
/// True when it acted.
pub fn cycle_item_indentation(ed: &mut Editor) -> bool {
    if sexp::option("org-cycle-level-after-item/entry-creation").is_some_and(|v| v.is_nil()) {
        return false;
    }
    let version = ed.buf.version;
    let continuing = last_command(ed) == Some("org-cycle")
        && ed
            .org
            .as_ref()
            .and_then(|o| o.list.tab_ind.as_ref())
            .is_some_and(|t| t.0 == version);
    let saved = ed.org.as_ref().and_then(|o| o.list.tab_ind.clone());
    let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
    let mut b = load(ed);
    match cycle_indentation(&mut b, continuing, saved.map(|s| (s.1, s.2)), odd) {
        None => false,
        Some(r) => {
            store(ed, b);
            let v = ed.buf.version;
            let new = match r {
                Ok(Some(s)) => Some((v, s.0, s.1)),
                Ok(None) => None,
                Err(e) => {
                    ed.set_err(e);
                    None
                }
            };
            if let Some(s) = state(ed) {
                s.tab_ind = new;
            }
            true
        }
    }
}

/// The Buf part of org-cycle-item-indentation: None when it does not
/// apply; Some(Ok(state)) with the org-tab-ind-state to keep (None when
/// the cycle starts over).
pub fn cycle_indentation(
    b: &mut Buf,
    continuing: bool,
    saved: Option<(usize, String)>,
    odd: bool,
) -> Option<Result<Option<(usize, String)>, String>> {
    if !at_item(b) {
        return None;
    }
    let mut st = list_struct(b);
    let item = b.bol();
    let ind = st.ind(item);
    let empty = {
        let me = b
            .looking_at_pos(&FULL_ITEM, item)
            .and_then(|c| c.end(0))
            .unwrap_or(item);
        let mut p = st.end(item);
        while p > 0 && matches!(b.s.as_bytes()[p - 1], b' ' | b'\t' | b'\n') {
            p -= 1;
        }
        me >= p
    };
    if !(continuing || empty) {
        return None;
    }
    let prevs = st.prevs();
    let parents = st.parents();
    let allow_outdent = |st: &Struct, prevs: &Alist, parents: &Alist| {
        next_item(item, prevs).is_none()
            && st.has_child(item).is_none()
            && parent(item, parents).is_some()
    };
    if continuing && let Some((old_ind, old_bul)) = saved {
        let state = Some((old_ind, old_bul.clone()));
        if ind > old_ind && prev_item(item, &prevs).is_some() {
            return Some(indent_item_generic(b, 1, true, &mut st, None, None, odd).map(|_| state));
        }
        if ind < old_ind && allow_outdent(&st, &prevs, &parents) {
            return Some(indent_item_generic(b, -1, true, &mut st, None, None, odd).map(|_| state));
        }
        let (bol, eol) = (b.bol(), b.eol());
        b.delete(bol, eol);
        b.goto(bol);
        b.insert(&format!("{}{old_bul} ", " ".repeat(old_ind)));
        let mut st = list_struct(b);
        let parents = st.parents();
        if ind > old_ind && allow_outdent(&st, &st.prevs(), &parents) {
            return Some(indent_item_generic(b, -1, true, &mut st, None, None, odd).map(|_| state));
        }
        write_struct(b, &mut st, &parents, None);
        return Some(Ok(None));
    }
    let line = b.sub(b.bol(), b.eol()).trim().to_owned();
    let state = Some((ind, line));
    if prev_item(item, &prevs).is_some() {
        return Some(indent_item_generic(b, 1, true, &mut st, None, None, odd).map(|_| state));
    }
    if next_item(item, &prevs).is_none() && parent(item, &parents).is_some() {
        return Some(indent_item_generic(b, -1, true, &mut st, None, None, odd).map(|_| state));
    }
    Some(Err("Cannot move item".into()))
}

// ---- org-list-make-subtree ----

/// org-list-make-subtree.
fn make_subtree(b: &mut Buf, st: &Settings) -> Result<(), String> {
    let item = in_item(b).ok_or("Not in a list")?;
    b.goto(item);
    let level = heading_before(b, item)
        .and_then(|h| syntax::level(b.sub(h, b.eol_at(h))))
        .map_or(1, |l| {
            reduced_level(l, st.opt_bool("org-odd-levels-only", false)) + 1
        });
    let blank = subtree_blank(b);
    let list = b.excursion(|b| export::to_lisp(b, true));
    let text = export::to_subtree(
        &list,
        level,
        blank,
        st.opt_bool("org-odd-levels-only", false),
        &export::Params::default(),
    );
    b.insert(&format!("{text}\n"));
    Ok(())
}

/// org-blank-before-new-entry's `heading` value for org-list-to-subtree.
fn subtree_blank(b: &Buf) -> bool {
    let v = alist_option(
        "org-blank-before-new-entry",
        "heading",
        Sexp::Sym("auto".into()),
    );
    if v.str() == Some("auto") {
        let Some(h) = b.pt.checked_sub(1).and_then(|p| heading_before(b, p)) else {
            return false;
        };
        h > 0 && b.sub(b.bol_at(h - 1), h - 1).trim().is_empty()
    } else {
        v.truthy()
    }
}

// ---- the command table ----

/// This module's interactive commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let r = match name {
        "org-beginning-of-item" => goto_cmd(ed, "begin"),
        "org-beginning-of-item-list" => goto_cmd(ed, "list-begin"),
        "org-end-of-item-list" => goto_cmd(ed, "list-end"),
        "org-end-of-item" => goto_cmd(ed, "end"),
        "org-previous-item" => goto_cmd(ed, "prev"),
        "org-next-item" => goto_cmd(ed, "next"),
        "org-move-item-down" => move_item(ed, true),
        "org-move-item-up" => move_item(ed, false),
        "org-insert-item" => insert_item_cmd(ed, !arg.is_none()).map(|_| ()),
        "org-list-repair" => with_buf(ed, repair),
        "org-cycle-list-bullet" | "org-cycle-list-bullet-previous" => {
            let which = match (name, arg) {
                ("org-cycle-list-bullet-previous", _) => Which::Previous,
                (_, Prefix::Num(n)) => Which::Index(n),
                _ => Which::Next,
            };
            with_buf(ed, |b| cycle_list_bullet(b, &which))
        }
        "org-list-checkbox-radio-mode" => {
            let on = match arg {
                Prefix::None => !ed.org.as_ref().is_some_and(|o| o.list.radio_mode),
                a => a.value() > 0,
            };
            if let Some(s) = state(ed) {
                s.radio_mode = on;
            }
            ed.set_msg(format!(
                "Org-List-Checkbox-Radio mode {}",
                if on { "enabled" } else { "disabled" }
            ));
            Ok(())
        }
        "org-toggle-radio-button" => with_buf(ed, |b| toggle_radio_button(b, arg)),
        "org-toggle-checkbox" => {
            let region = region_pos(ed);
            with_buf(ed, |b| toggle_checkbox(b, arg, region))
        }
        "org-list-ctrl-c-ctrl-c" => {
            let radio = ed.org.as_ref().is_some_and(|o| o.list.radio_mode);
            with_buf(ed, |b| list_ctrl_c_ctrl_c(b, arg, radio))
        }
        "org-reset-checkbox-state-subtree" => reset_checkbox_state_subtree(ed),
        "org-update-checkbox-count" => {
            let scope = if arg.is_none() {
                Scope::Section
            } else {
                Scope::All
            };
            update_checkbox_count_in(ed, scope, false);
            Ok(())
        }
        "org-indent-item" => indent_cmd(ed, 1, true),
        "org-outdent-item" => indent_cmd(ed, -1, true),
        "org-indent-item-tree" => indent_cmd(ed, 1, false),
        "org-outdent-item-tree" => indent_cmd(ed, -1, false),
        "org-sort-list" => {
            let with_case = !arg.is_none();
            let entries = ["a", "A", "n", "N", "t", "T", "f", "F", "x", "X"]
                .iter()
                .map(|k| {
                    let label = match k.to_ascii_lowercase().as_str() {
                        "a" => "alpha",
                        "n" => "numeric",
                        "t" => "time",
                        "f" => "func",
                        _ => "checked",
                    };
                    let rev = if k.chars().all(|c| c.is_ascii_uppercase()) {
                        " (reversed)"
                    } else {
                        ""
                    };
                    ((*k).to_owned(), format!("{label}{rev}"))
                })
                .collect();
            super::menu(
                ed,
                "Sort plain list: [a]lpha  [n]umeric  [t]ime  [f]unc  [x]checked  A/N/T/F/X means reversed:",
                entries,
                move |ed, k| {
                    let c = k.chars().next().unwrap_or('a');
                    let before = ed.cur.pos();
                    ed.undo.begin(before);
                    let r = with_buf(ed, |b| sort_list(b, with_case, c, None, None));
                    ed.undo.end(ed.cur.pos());
                    if let Err(e) = r {
                        ed.set_err(e);
                    }
                },
            );
            Ok(())
        }
        "org-toggle-item" => {
            let region = region_pos(ed);
            let st = super::settings(ed);
            with_buf(ed, |b| toggle_item(b, !arg.is_none(), region, &st))
        }
        "org-list-make-subtree" => {
            let st = super::settings(ed);
            with_buf(ed, |b| make_subtree(b, &st))
        }
        _ => return None,
    };
    Some(r)
}
