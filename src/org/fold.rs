//! Visibility: org-fold.el and org-cycle.el on Fred's line folds.
//!
//! Emacs folds character regions from the end of a line to the end of a
//! later line; here that is the whole lines in between. Each folding spec
//! (outline, block, drawer) is kept apart, as upstream does, and the
//! editor's folds are their union.

use super::syntax::{self, level};
use super::{Prefix, line};
use crate::editor::Editor;
use crate::fold::Folds;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spec {
    Outline,
    Block,
    Drawer,
    /// Narrowing (narrow-to-region): text outside is not part of the buffer.
    Narrow,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Specs {
    pub outline: Folds,
    pub block: Folds,
    pub drawer: Folds,
    pub narrow: Folds,
}

impl Specs {
    fn get(&mut self, spec: Spec) -> &mut Folds {
        match spec {
            Spec::Outline => &mut self.outline,
            Spec::Block => &mut self.block,
            Spec::Drawer => &mut self.drawer,
            Spec::Narrow => &mut self.narrow,
        }
    }

    pub fn line_change(&mut self, at: usize, removed: usize, inserted: usize) {
        for f in [
            &mut self.outline,
            &mut self.block,
            &mut self.drawer,
            &mut self.narrow,
        ] {
            f.line_change(at, removed, inserted);
        }
    }
}

fn specs(ed: &mut Editor) -> Option<&mut Specs> {
    ed.org.as_mut().map(|o| &mut o.specs)
}

/// Make the editor's folds the union of the specs.
fn sync(ed: &mut Editor) {
    let Some(o) = &ed.org else { return };
    let mut all = Folds::default();
    for f in [
        &o.specs.outline,
        &o.specs.block,
        &o.specs.drawer,
        &o.specs.narrow,
    ] {
        for &(s, e) in f.ranges() {
            all.hide(s, e);
        }
    }
    ed.folds = all;
}

/// org-fold-region on lines `s..=e` (nothing when `s > e`).
pub fn region(ed: &mut Editor, s: usize, e: usize, flag: bool, spec: Spec) {
    if s > e {
        return;
    }
    if let Some(sp) = specs(ed) {
        let f = sp.get(spec);
        if flag {
            f.hide(s, e);
        } else {
            f.show(s, e);
        }
    }
    sync(ed);
}

/// Show lines `s..=e` in every spec.
fn show_all_specs(ed: &mut Editor, s: usize, e: usize) {
    for spec in [Spec::Outline, Spec::Block, Spec::Drawer] {
        region(ed, s, e, false, spec);
    }
}

pub fn hidden(ed: &Editor, l: usize) -> bool {
    ed.folds.hidden(l)
}

fn n(ed: &Editor) -> usize {
    ed.line_count()
}

fn is_heading(ed: &Editor, l: usize) -> bool {
    l < n(ed) && level(&line(ed, l)).is_some()
}

/// org-back-to-heading (None before the first heading).
pub fn back_to_heading(ed: &Editor, l: usize) -> Option<usize> {
    syntax::heading_at_or_before(&ed.buf, l)
}

pub fn before_first_heading(ed: &Editor, l: usize) -> bool {
    back_to_heading(ed, l).is_none()
}

/// org-fold-heading nil: show heading line `h`.
pub fn show_heading(ed: &mut Editor, h: usize) {
    region(ed, h, h, false, Spec::Outline);
}

/// org-fold-subtree: hide (or show) the body of the subtree at `h`.
pub fn fold_subtree(ed: &mut Editor, h: usize, flag: bool) {
    let end = syntax::subtree_end(&ed.buf, h);
    region(ed, h + 1, end.saturating_sub(1), flag, Spec::Outline);
}

/// org-fold-show-entry: the heading and its body up to the next heading.
pub fn show_entry(ed: &mut Editor, l: usize) {
    let h = back_to_heading(ed, l).unwrap_or(0);
    let end = syntax::entry_end(&ed.buf, h);
    region(ed, h, end.saturating_sub(1), false, Spec::Outline);
}

/// org-fold-hide-entry.
pub fn hide_entry(ed: &mut Editor, l: usize) {
    let start = match back_to_heading(ed, l) {
        Some(h) => h + 1,
        None => 0,
    };
    if start >= n(ed) || is_heading(ed, start) {
        return;
    }
    let end = (start..n(ed)).find(|&i| is_heading(ed, i)).unwrap_or(n(ed));
    region(ed, start, end - 1, true, Spec::Outline);
}

/// org-fold-show-children: direct children (or LEVEL levels below).
pub fn show_children(ed: &mut Editor, l: usize, levels: Option<usize>) {
    let Some(h) = back_to_heading(ed, l) else {
        return;
    };
    let parent = level(&line(ed, h)).unwrap_or(1);
    let max = parent + levels.unwrap_or(1).max(1);
    let end = syntax::subtree_end(&ed.buf, h);
    show_heading(ed, h);
    let mut min_direct = usize::MAX;
    for i in h + 1..end {
        if let Some(lv) = level(&line(ed, i)) {
            if lv < min_direct {
                min_direct = lv;
            }
            if lv <= max.max(min_direct) {
                show_heading(ed, i);
            }
        }
    }
}

/// org-fold-show-subtree.
pub fn show_subtree(ed: &mut Editor, l: usize) {
    let Some(h) = back_to_heading(ed, l) else {
        return;
    };
    let end = syntax::subtree_end(&ed.buf, h);
    region(ed, h, end - 1, false, Spec::Outline);
}

/// org-fold-show-all.
pub fn show_all(ed: &mut Editor, types: &[Spec]) {
    let last = n(ed) - 1;
    for &t in types {
        region(ed, 0, last, false, t);
    }
}

/// org-cycle-overview.
pub fn overview(ed: &mut Editor) {
    let first = (0..n(ed)).find(|&i| is_heading(ed, i));
    hide_drawers(ed, 0, first.unwrap_or(n(ed)));
    let Some(first) = first else { return };
    let mut last = first;
    let mut lvl = level(&line(ed, first)).unwrap();
    for i in first + 1..n(ed) {
        if let Some(lv) = level(&line(ed, i))
            && lv <= lvl
        {
            region(ed, last + 1, i - 1, true, Spec::Outline);
            show_heading(ed, i);
            last = i;
            lvl = lv;
        }
    }
    region(ed, last + 1, n(ed) - 1, true, Spec::Outline);
}

/// org-cycle-content: all headings (up to level `arg`), no bodies.
pub fn content(ed: &mut Editor, arg: Option<usize>) {
    show_all(ed, &[Spec::Outline]);
    let first = (0..n(ed)).find(|&i| is_heading(ed, i)).unwrap_or(n(ed));
    hide_drawers(ed, 0, first);
    let max = arg.filter(|&a| a > 0).unwrap_or(usize::MAX);
    let mut last = n(ed) - 1;
    for i in (0..n(ed)).rev() {
        if level(&line(ed, i)).is_some_and(|lv| lv <= max) {
            region(ed, i + 1, last, true, Spec::Outline);
            last = i.saturating_sub(1);
            if i == 0 {
                break;
            }
        }
    }
}

/// org-fold-hide-sublevels.
pub fn hide_sublevels(ed: &mut Editor, levels: usize) {
    let Some(beg) = (0..n(ed)).find(|&i| is_heading(ed, i)) else {
        return;
    };
    region(ed, beg, n(ed) - 1, true, Spec::Outline);
    for i in beg..n(ed) {
        if level(&line(ed, i)).is_some_and(|lv| lv <= levels) {
            show_heading(ed, i);
        }
    }
    // org-fold-hide-sublevels hides from the first heading; it stays shown.
    show_heading(ed, beg);
}

/// Block or drawer around line `l`: (opening line, closing line).
pub fn wrapper_at(ed: &Editor, l: usize, drawer: bool) -> Option<(usize, usize)> {
    let text = line(ed, l);
    let t = text.trim();
    if drawer {
        let is_end = t.eq_ignore_ascii_case(":END:");
        let is_start = |s: &str| {
            s.len() > 2
                && s.starts_with(':')
                && s.ends_with(':')
                && !s.eq_ignore_ascii_case(":END:")
                && s[1..s.len() - 1]
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        };
        if is_start(t) {
            let end = (l + 1..n(ed))
                .take_while(|&i| !is_heading(ed, i))
                .find(|&i| line(ed, i).trim().eq_ignore_ascii_case(":END:"))?;
            return Some((l, end));
        }
        if is_end {
            let start = (0..l)
                .rev()
                .take_while(|&i| !is_heading(ed, i))
                .find(|&i| is_start(line(ed, i).trim()))?;
            return Some((start, l));
        }
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("#+begin_") {
        let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
        let close = format!("#+end_{name}");
        let end = (l + 1..n(ed)).find(|&i| line(ed, i).trim().to_ascii_lowercase() == close)?;
        return Some((l, end));
    }
    if lower.starts_with("#+begin:") {
        let end = (l + 1..n(ed)).find(|&i| line(ed, i).trim().eq_ignore_ascii_case("#+end:"))?;
        return Some((l, end));
    }
    if let Some(name) = lower.strip_prefix("#+end_") {
        let open = format!("#+begin_{}", name.trim());
        let start = (0..l).rev().find(|&i| {
            let s = line(ed, i).trim().to_ascii_lowercase();
            s == open || s.starts_with(&format!("{open} "))
        })?;
        return Some((start, l));
    }
    None
}

/// org-fold--hide-wrapper-toggle; `force` Some(false) shows, Some(true) hides.
pub fn toggle_wrapper(ed: &mut Editor, l: usize, drawer: bool, force: Option<bool>) -> bool {
    let Some((s, e)) = wrapper_at(ed, l, drawer) else {
        return false;
    };
    let spec = if drawer { Spec::Drawer } else { Spec::Block };
    let folded = ed.org.as_ref().is_some_and(|o| {
        let f = if drawer {
            &o.specs.drawer
        } else {
            &o.specs.block
        };
        f.hidden(s + 1)
    });
    let flag = force.unwrap_or(!folded);
    region(ed, s + 1, e, flag, spec);
    if hidden(ed, ed.cur.line) {
        ed.set_cursor(s, 0);
    }
    true
}

/// org-fold--hide-drawers between lines `beg..end`.
pub fn hide_drawers(ed: &mut Editor, beg: usize, end: usize) {
    let mut l = beg;
    while l < end.min(n(ed)) {
        if let Some((s, e)) = wrapper_at(ed, l, true).filter(|&(s, _)| s == l) {
            region(ed, s + 1, e, true, Spec::Drawer);
            l = e + 1;
        } else {
            l += 1;
        }
    }
}

/// org-fold-hide-block-all.
pub fn hide_blocks(ed: &mut Editor) {
    let mut l = 0;
    while l < n(ed) {
        if line(ed, l)
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("#+begin")
            && let Some((s, e)) = wrapper_at(ed, l, false)
        {
            region(ed, s + 1, e, true, Spec::Block);
            l = e + 1;
        } else {
            l += 1;
        }
    }
}

/// org-fold-hide-archived-subtrees in lines `beg..end`.
pub fn hide_archived(ed: &mut Editor, beg: usize, end: usize) {
    let st = super::settings(ed);
    let mut l = beg;
    while l < end.min(n(ed)) {
        let text = line(ed, l);
        if let Some(h) = syntax::headline(&text, &st)
            && h.archived()
        {
            fold_subtree(ed, l, true);
            l = syntax::subtree_end(&ed.buf, l);
        } else {
            l += 1;
        }
    }
}

fn blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// org-cycle-show-empty-lines over `beg..end` (lines).
pub fn show_empty_lines(ed: &mut Editor, beg: usize, end: usize) {
    let sep = super::options::int("org-cycle-separator-lines", 2);
    if sep != 0 {
        let k = sep.unsigned_abs() as usize;
        for h in beg.max(1)..end.min(n(ed)) {
            if hidden(ed, h) || !is_heading(ed, h) {
                continue;
            }
            let need = k.max(1);
            if h >= need && (h - need..h).all(|i| blank(&line(ed, i))) {
                if sep > 0 {
                    region(ed, h - 1, h - 1, false, Spec::Outline);
                } else {
                    // Negative: show all blank lines before the heading.
                    let mut b = h;
                    while b > 0 && blank(&line(ed, b - 1)) {
                        b -= 1;
                    }
                    region(ed, b, h - 1, false, Spec::Outline);
                }
            }
        }
    }
    // Never hide empty lines at the end of the file.
    let mut last = n(ed);
    while last > 0 && blank(&line(ed, last - 1)) {
        last -= 1;
    }
    if last < n(ed) {
        region(ed, last, n(ed) - 1, false, Spec::Outline);
    }
}

/// org-cycle-hook after a global (`overview`/`contents`/`all`) or local
/// (`folded`/`children`/`subtree` at heading `h`) change.
fn cycle_hook(ed: &mut Editor, state: &str, h: usize) {
    let open_archived = super::options::bool("org-cycle-open-archived-trees", false);
    let global = matches!(state, "overview" | "contents" | "all");
    if !open_archived && !matches!(state, "overview" | "folded") {
        if global {
            hide_archived(ed, 0, n(ed));
        } else {
            let end = syntax::subtree_end(&ed.buf, h);
            hide_archived(ed, h, end);
            let st = super::settings(ed);
            if syntax::headline(&line(ed, h), &st).is_some_and(|x| x.archived()) {
                ed.set_msg(
                    "Subtree is archived and stays closed.  Use `C-c C-<tab>' to cycle it anyway.",
                );
            }
        }
    }
    if global {
        show_empty_lines(ed, 0, n(ed));
    } else if matches!(state, "children" | "folded") {
        let end = syntax::subtree_end(&ed.buf, h);
        show_empty_lines(ed, h, (end + 1).min(n(ed)));
    } else {
        show_empty_lines(ed, n(ed), n(ed));
    }
}

/// org-cycle-set-startup-visibility.
pub fn startup_visibility(ed: &mut Editor) {
    let st = super::settings(ed);
    let folded = st.opt_sym("org-startup-folded", "showeverything");
    match folded.as_str() {
        "t" | "fold" | "overview" => overview(ed),
        "content" => content(ed, None),
        "show2levels" => content(ed, Some(2)),
        "show3levels" => content(ed, Some(3)),
        "show4levels" => content(ed, Some(4)),
        "show5levels" => content(ed, Some(5)),
        _ => show_all(ed, &[Spec::Outline, Spec::Block, Spec::Drawer]),
    }
    if folded != "showeverything" {
        if st.opt_bool(
            "org-cycle-hide-block-startup",
            st.opt_bool("org-hide-block-startup", false),
        ) {
            hide_blocks(ed);
        }
        visibility_by_property(ed);
        hide_archived(ed, 0, n(ed));
        if st.opt_bool(
            "org-cycle-hide-drawer-startup",
            st.opt_bool("org-hide-drawer-startup", true),
        ) {
            hide_drawers(ed, 0, n(ed));
        }
        show_empty_lines(ed, 0, n(ed));
    }
}

/// Called when an Org buffer opens.
pub fn startup(ed: &mut Editor) {
    startup_visibility(ed);
    if let Some(o) = &mut ed.org {
        o.global_status = None;
    }
}

/// The VISIBILITY property of the entry at heading `h`, if any.
fn visibility_property(ed: &Editor, h: usize) -> Option<String> {
    let end = syntax::entry_end(&ed.buf, h);
    let mut inside = false;
    for i in h + 1..end {
        let t = line(ed, i);
        let t = t.trim();
        if t.eq_ignore_ascii_case(":PROPERTIES:") {
            inside = true;
        } else if t.eq_ignore_ascii_case(":END:") {
            if inside {
                return None;
            }
        } else if inside && let Some(rest) = t.strip_prefix(":VISIBILITY:") {
            return Some(rest.trim().to_owned());
        }
    }
    None
}

/// org-cycle-set-visibility-according-to-property.
pub fn visibility_by_property(ed: &mut Editor) {
    for h in 0..n(ed) {
        if !is_heading(ed, h) {
            continue;
        }
        let Some(state) = visibility_property(ed, h) else {
            continue;
        };
        fold_subtree(ed, h, true);
        match state.as_str() {
            "folded" => {}
            "children" => {
                show_entry(ed, h);
                show_children(ed, h, None);
            }
            "content" => {
                show_heading(ed, h);
                let end = syntax::subtree_end(&ed.buf, h);
                let mut last = end.saturating_sub(1);
                for i in (h..end).rev() {
                    if is_heading(ed, i) {
                        region(ed, i, i, false, Spec::Outline);
                        region(ed, i + 1, last, true, Spec::Outline);
                        last = i.saturating_sub(1);
                    }
                }
            }
            "all" | "showall" => show_subtree(ed, h),
            _ => {}
        }
    }
}

/// The context detail for revealing a location (org-fold-show-context-detail).
fn context_detail(key: &str) -> String {
    match super::sexp::option("org-fold-show-context-detail") {
        Some(v) if v.sym().is_some() => v.sym().unwrap().to_owned(),
        Some(v) => v
            .list()
            .and_then(|l| {
                l.iter()
                    .find(|e| e.car().and_then(|c| c.str()) == Some(key))
                    .or_else(|| {
                        l.iter()
                            .find(|e| e.car().and_then(|c| c.str()) == Some("default"))
                    })
            })
            .and_then(|e| e.cdr().str().map(str::to_owned))
            .unwrap_or_else(|| "ancestors".into()),
        None => match key {
            "agenda" => "local",
            "bookmark-jump" | "isearch" => "lineage",
            _ => "ancestors",
        }
        .into(),
    }
}

/// org-fold-show-context for line `l` (`key` as in org-fold-show-context-detail).
pub fn show_context_for(ed: &mut Editor, l: usize, key: &str) {
    let detail = context_detail(key);
    show_set_visibility(ed, l, &detail);
}

/// The narrowed region (first, last line), if narrowed.
pub fn narrowed(ed: &Editor) -> Option<(usize, usize)> {
    let o = ed.org.as_ref()?;
    let r = o.specs.narrow.ranges();
    if r.is_empty() {
        return None;
    }
    let n = ed.line_count();
    let start = if r[0].0 == 0 { r[0].1 + 1 } else { 0 };
    let end = match r.last() {
        Some(&(s, e)) if e + 1 >= n && s > 0 => s - 1,
        _ => n - 1,
    };
    Some((start.min(n - 1), end.max(start).min(n - 1)))
}

/// narrow-to-region on lines `s..=e`.
pub fn narrow(ed: &mut Editor, s: usize, e: usize) {
    widen(ed);
    let n = ed.line_count();
    if s > 0 {
        region(ed, 0, s - 1, true, Spec::Narrow);
    }
    if e + 1 < n {
        region(ed, e + 1, n - 1, true, Spec::Narrow);
    }
    if ed.cur.line < s || ed.cur.line > e {
        ed.set_cursor(s, 0);
    }
}

/// widen.
pub fn widen(ed: &mut Editor) {
    let n = ed.line_count();
    region(ed, 0, n - 1, false, Spec::Narrow);
}

/// Reveal line `l` after a search or jump (isearch context).
pub fn show_context(ed: &mut Editor, l: usize) {
    if let Some((s, e)) = narrowed(ed)
        && (l < s || l > e)
    {
        // Outside the narrowing: stay inside it.
        let to = if l < s { s } else { e };
        ed.set_cursor(to, 0);
        if hidden(ed, to) {
            show_context(ed, to);
        }
        return;
    }
    show_context_for(ed, l, "isearch");
    if hidden(ed, l) {
        // Still hidden (a fold spec this detail does not open): open it.
        show_all_specs(ed, l, l);
    }
    if ed.cur.line == l && hidden(ed, l) {
        let to = ed.folds.prev_visible(l);
        ed.set_cursor(to, 0);
    }
}

/// org-fold-show-set-visibility.
pub fn show_set_visibility(ed: &mut Editor, l: usize, detail: &str) {
    if is_heading(ed, l) && detail != "local" {
        show_heading(ed, l);
    } else {
        show_entry(ed, l);
        if hidden(ed, l) {
            // Unfold whatever block or drawer holds the line.
            for spec in [Spec::Block, Spec::Drawer, Spec::Outline] {
                let range = ed.org.as_ref().and_then(|o| match spec {
                    Spec::Block => o.specs.block.range_at(l),
                    Spec::Drawer => o.specs.drawer.range_at(l),
                    Spec::Narrow => None,
                    Spec::Outline => o.specs.outline.range_at(l),
                });
                if let Some((s, e)) = range {
                    region(ed, s, e, false, spec);
                }
            }
        }
        if !before_first_heading(ed, l) {
            match detail {
                "tree" | "canonical" | "t" => show_children(ed, l, None),
                "nil" | "minimal" | "ancestors" | "ancestors-full" => {}
                _ => {
                    if let Some(next) = syntax::next_heading(&ed.buf, l, usize::MAX) {
                        show_heading(ed, next);
                    }
                }
            }
        }
    }
    if detail == "ancestors-full" {
        show_subtree(ed, l);
    }
    if detail == "lineage"
        && let Some(h) = back_to_heading(ed, l)
    {
        let mut s = h;
        while let Some(x) = syntax::next_sibling(&ed.buf, s) {
            show_heading(ed, x);
            s = x;
        }
        let mut s = h;
        while let Some(x) = syntax::prev_sibling(&ed.buf, s) {
            show_heading(ed, x);
            s = x;
        }
    }
    if matches!(
        detail,
        "ancestors" | "ancestors-full" | "lineage" | "tree" | "canonical" | "t"
    ) && let Some(h) = back_to_heading(ed, l)
    {
        let mut p = h;
        while let Some(up) = syntax::parent(&ed.buf, p) {
            show_heading(ed, up);
            if matches!(detail, "canonical" | "t") {
                show_entry(ed, up);
            }
            if matches!(detail, "tree" | "canonical" | "t") {
                show_children(ed, up, None);
            }
            p = up;
        }
    }
}

/// org-cycle-internal-global.
fn cycle_global(ed: &mut Editor, repeat: bool) {
    let status = ed.org.as_ref().and_then(|o| o.global_status);
    let new = match (repeat, status) {
        (true, Some("overview")) => {
            content(ed, None);
            ed.set_msg("CONTENTS...done");
            "contents"
        }
        (true, Some("contents")) => {
            show_all(ed, &[Spec::Outline, Spec::Block]);
            ed.set_msg("SHOW ALL");
            "all"
        }
        _ => {
            overview(ed);
            ed.set_msg("OVERVIEW");
            "overview"
        }
    };
    if let Some(o) = &mut ed.org {
        o.global_status = Some(new);
    }
    cycle_hook(ed, new, 0);
}

/// org-cycle-internal-local on heading `h`.
fn cycle_local(ed: &mut Editor, h: usize, repeat: bool) {
    let total = n(ed);
    let end = syntax::subtree_end(&ed.buf, h);
    // eos: the last subtree line (not counting the final newline).
    let eos = end.saturating_sub(1);
    let has_children = syntax::next_heading(&ed.buf, h, usize::MAX).is_some_and(|x| x < end);
    if eos == h {
        ed.set_msg("EMPTY ENTRY");
        if let Some(o) = &mut ed.org {
            o.subtree_status = None;
        }
        if end < total && hidden(ed, end) {
            show_heading(ed, end);
        }
        return;
    }
    // eol: the first visible line after the heading.
    let eol = ed.folds.down(h, 1, total);
    let rest_blank = eol <= h || eol > eos || (eol..=eos).all(|i| blank(&line(ed, i)));
    let skip_children = super::options::bool("org-cycle-skip-children-state-if-no-children", true);
    let children_skipped = !has_children && skip_children;
    let status = ed.org.as_ref().and_then(|o| o.subtree_status);
    let new = if rest_blank && (has_children || !skip_children) {
        show_entry(ed, h);
        show_children(ed, h, None);
        ed.set_msg("CHILDREN");
        if end < total && hidden(ed, end) {
            show_heading(ed, end);
        }
        "children"
    } else if (rest_blank && children_skipped) || (repeat && status == Some("children")) {
        region(ed, h + 1, eos, false, Spec::Outline);
        ed.set_msg(if children_skipped {
            "SUBTREE (NO CHILDREN)"
        } else {
            "SUBTREE"
        });
        "subtree"
    } else {
        region(ed, h + 1, eos, true, Spec::Outline);
        ed.set_msg("FOLDED");
        "folded"
    };
    if let Some(o) = &mut ed.org {
        o.subtree_status = Some(new);
    }
    cycle_hook(ed, new, h);
}

fn repeated(ed: &Editor, names: &[&str]) -> bool {
    ed.org
        .as_ref()
        .and_then(|o| o.last_command.as_deref())
        .is_some_and(|c| names.contains(&c))
}

/// org-cycle.
pub fn cycle(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let repeat = repeated(ed, &["org-cycle", "org-cycle-force-archived"]);
    match arg {
        Prefix::U(2) => {
            startup_visibility(ed);
            ed.set_msg("Startup visibility, plus VISIBILITY properties");
            return Ok(());
        }
        Prefix::U(n) if n >= 3 => {
            show_all(ed, &[Spec::Outline, Spec::Block, Spec::Drawer]);
            ed.set_msg("Entire buffer visible, including drawers");
            return Ok(());
        }
        Prefix::U(1) => {
            cycle_global(
                ed,
                repeated(
                    ed,
                    &[
                        "org-cycle",
                        "org-shifttab",
                        "org-cycle-global",
                        "org-global-cycle",
                    ],
                ),
            );
            return Ok(());
        }
        Prefix::Num(_) | Prefix::Minus => {
            let a = arg.value();
            let Some(h) = back_to_heading(ed, ed.cur.line) else {
                return Err("Before first headline at position".into());
            };
            let lvl = level(&line(ed, h)).unwrap_or(1) as i64;
            let up = if a < 0 { -a } else { lvl - a };
            let mut target = h;
            for _ in 0..up.max(0) {
                match syntax::parent(&ed.buf, target) {
                    Some(p) => target = p,
                    None => break,
                }
            }
            show_subtree(ed, target);
            return Ok(());
        }
        _ => {}
    }
    let l = ed.cur.line;
    let text = line(ed, l);
    if super::options::bool("org-cycle-global-at-bob", false)
        && l == 0
        && ed.cur.byte == 0
        && level(&text).is_none()
    {
        cycle_global(ed, repeated(ed, &["org-cycle"]));
        return Ok(());
    }
    if hooks::cycle_level(ed)? {
        return Ok(());
    }
    if toggle_wrapper(ed, l, false, None) || toggle_wrapper(ed, l, true, None) {
        return Ok(());
    }
    if hooks::table_tab(ed)? {
        return Ok(());
    }
    if level(&text).is_some() {
        cycle_local(ed, l, repeat);
        return Ok(());
    }
    if hooks::item_cycle(ed, repeat)? {
        return Ok(());
    }
    hooks::emulate_tab(ed)
}

/// Contexts other modules provide to TAB, by upstream command name.
pub mod hooks {
    use super::super::{Prefix, call, ctx};
    use crate::editor::Editor;

    /// org-cycle-level: TAB on a new empty heading cycles its level.
    pub fn cycle_level(ed: &mut Editor) -> Result<bool, String> {
        if !super::super::options::bool("org-cycle-level-after-item/entry-creation", true) {
            return Ok(false);
        }
        super::super::structure::cycle_level(ed)
    }
    /// In a table: org-table-next-field.
    pub fn table_tab(ed: &mut Editor) -> Result<bool, String> {
        if ctx::at_table_el(ed, ed.cur.line) && !ctx::at_table(ed, ed.cur.line) {
            ed.set_msg("Use `C-c '' to edit table.el tables");
            return Ok(true);
        }
        if ctx::at_table(ed, ed.cur.line) {
            call(ed, "org-table-next-field", Prefix::None)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub fn item_cycle(ed: &mut Editor, repeat: bool) -> Result<bool, String> {
        if super::super::list::cycle_item_indentation(ed) {
            return Ok(true);
        }
        match super::super::list::cycle_item(ed, repeat) {
            Some(r) => r.map(|()| true),
            None => Ok(false),
        }
    }
    /// org-cycle-emulate-tab: TAB's global binding (indent-for-tab-command).
    pub fn emulate_tab(_ed: &mut Editor) -> Result<(), String> {
        Ok(())
    }
    /// org-table-previous-field in a table.
    pub fn table_back(ed: &mut Editor) -> Option<Result<(), String>> {
        ctx::at_table(ed, ed.cur.line).then(|| call(ed, "org-table-previous-field", Prefix::None))
    }
    /// org-table-toggle-column-width in a table.
    pub fn table_width(ed: &mut Editor) -> Option<Result<(), String>> {
        ctx::at_table(ed, ed.cur.line)
            .then(|| call(ed, "org-table-toggle-column-width", Prefix::None))
    }
    /// A pending log note (org-finish-function), finished or aborted.
    pub fn finish_note(ed: &mut Editor, abort: bool) -> Option<Result<(), String>> {
        ed.org_finish.is_some().then(|| {
            super::super::effect(ed, move |s| s.org_finish(abort));
            Ok(())
        })
    }
}

/// org-fold-reveal.
pub fn reveal(ed: &mut Editor, arg: Prefix) {
    let l = ed.cur.line;
    match arg {
        Prefix::U(1) => show_set_visibility(ed, l, "canonical"),
        Prefix::U(2) => {
            if let Some(h) = back_to_heading(ed, l)
                && let Some(p) = syntax::parent(&ed.buf, h)
            {
                show_subtree(ed, p);
            }
        }
        _ => show_set_visibility(ed, l, "lineage"),
    }
}

/// The visibility commands.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let l = ed.cur.line;
    let count = |a: Prefix| (!a.is_none()).then(|| a.value().max(1) as usize);
    Some(match name {
        "org-cycle" => cycle(ed, arg),
        "org-cycle-force-archived" => {
            super::options::put("org-cycle-open-archived-trees", toml::Value::Boolean(true));
            let r = cycle(ed, arg);
            super::options::put("org-cycle-open-archived-trees", toml::Value::Boolean(false));
            r
        }
        "org-shifttab" => {
            if let Some(r) = hooks::table_back(ed) {
                r
            } else if let Prefix::Num(n) = arg {
                let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
                let lv = if odd { (2 * n - 1).max(1) } else { n.max(1) } as usize;
                ed.set_msg(format!("Content view to level: {n}"));
                content(ed, Some(lv));
                show_empty_lines(ed, 0, n_lines(ed));
                if let Some(o) = &mut ed.org {
                    o.global_status = Some("overview");
                }
                Ok(())
            } else {
                global_cycle(ed, arg)
            }
        }
        "org-cycle-global" | "org-global-cycle" => global_cycle(ed, arg),
        "org-cycle-overview" | "org-overview" => {
            overview(ed);
            Ok(())
        }
        "org-cycle-content" | "org-content" => {
            content(ed, count(arg));
            Ok(())
        }
        "org-fold-show-all" | "org-show-all" => {
            show_all(ed, &[Spec::Outline, Spec::Block, Spec::Drawer]);
            Ok(())
        }
        "org-fold-show-entry" | "org-show-entry" => {
            show_entry(ed, l);
            Ok(())
        }
        "org-fold-hide-entry" | "org-hide-entry" => {
            hide_entry(ed, l);
            Ok(())
        }
        "org-fold-show-children" | "org-show-children" => {
            show_children(ed, l, count(arg));
            Ok(())
        }
        "org-fold-show-subtree" | "org-show-subtree" => {
            show_subtree(ed, l);
            Ok(())
        }
        "org-fold-hide-subtree" | "org-hide-subtree" => match back_to_heading(ed, l) {
            Some(h) => {
                fold_subtree(ed, h, true);
                Ok(())
            }
            None => Err("Before first headline".into()),
        },
        "org-fold-show-branches" | "org-show-branches" => {
            show_children(ed, l, Some(1000));
            Ok(())
        }
        "org-fold-hide-sublevels" | "org-hide-sublevels" => {
            let lv = if !arg.is_none() {
                arg.value()
            } else {
                level(&line(ed, l)).unwrap_or(1) as i64
            };
            if lv < 1 {
                Err("Must keep at least one level of headers".into())
            } else {
                hide_sublevels(ed, lv as usize);
                Ok(())
            }
        }
        "org-fold-reveal" | "org-reveal" => {
            reveal(ed, arg);
            Ok(())
        }
        "org-fold-hide-block-toggle" | "org-hide-block-toggle" => {
            if toggle_wrapper(ed, l, false, None) {
                Ok(())
            } else {
                Err("Not at a block".into())
            }
        }
        "org-fold-hide-drawer-toggle" | "org-hide-drawer-toggle" => {
            if toggle_wrapper(ed, l, true, None) {
                Ok(())
            } else {
                Err("Not at a drawer".into())
            }
        }
        "org-fold-hide-block-all" | "org-hide-block-all" => {
            hide_blocks(ed);
            Ok(())
        }
        "org-fold-hide-drawer-all" | "org-hide-drawer-all" => {
            hide_drawers(ed, 0, n_lines(ed));
            Ok(())
        }
        "org-cycle-set-visibility-according-to-property"
        | "org-set-visibility-according-to-property" => {
            visibility_by_property(ed);
            Ok(())
        }
        "org-set-startup-visibility" | "org-cycle-set-startup-visibility" => {
            startup_visibility(ed);
            Ok(())
        }
        "org-narrow-to-subtree" => match back_to_heading(ed, l) {
            Some(h) => {
                let end = syntax::subtree_end(&ed.buf, h);
                narrow(ed, h, end.saturating_sub(1));
                Ok(())
            }
            None => Err("Not in a subtree".into()),
        },
        "org-narrow-to-block" => match super::ctx::block_at(ed, l)
            .map(|(_, b, e)| (b, e))
            .or_else(|| wrapper_at(ed, l, false))
        {
            Some((b, e)) => {
                narrow(ed, b, e);
                Ok(())
            }
            None => Err("Not in a block".into()),
        },
        "org-narrow-to-element" => {
            // The paragraph, block, drawer or subtree at point.
            if super::ctx::at_heading(ed, l) {
                let end = syntax::subtree_end(&ed.buf, l);
                narrow(ed, l, end - 1);
            } else if let Some((b, e)) =
                wrapper_at(ed, l, false).or_else(|| wrapper_at(ed, l, true))
            {
                narrow(ed, b, e);
            } else {
                let blank = |i: usize| line(ed, i).trim().is_empty();
                let mut s = l;
                while s > 0 && !blank(s - 1) && !super::ctx::at_heading(ed, s - 1) {
                    s -= 1;
                }
                let mut e = l;
                while e + 1 < n_lines(ed) && !blank(e + 1) && !super::ctx::at_heading(ed, e + 1) {
                    e += 1;
                }
                narrow(ed, s, e);
            }
            Ok(())
        }
        "widen" | "org-widen" => {
            widen(ed);
            Ok(())
        }
        "org-toggle-narrow-to-subtree" => {
            if narrowed(ed).is_some() {
                widen(ed);
                Ok(())
            } else {
                command(ed, "org-narrow-to-subtree", arg).unwrap()
            }
        }
        "org-ctrl-c-tab" => {
            if let Some(r) = hooks::table_width(ed) {
                r
            } else if before_first_heading(ed, l) {
                if let Some(first) = (0..n_lines(ed)).find(|&i| is_heading(ed, i))
                    && first > 0
                {
                    region(ed, 0, first - 1, true, Spec::Outline);
                }
                hide_sublevels(ed, count(arg).unwrap_or(1));
                Ok(())
            } else {
                let h = back_to_heading(ed, l).unwrap();
                fold_subtree(ed, h, true);
                show_children(ed, h, count(arg));
                Ok(())
            }
        }
        "org-kill-note-or-show-branches" => {
            if let Some(r) = hooks::finish_note(ed, true) {
                r
            } else if before_first_heading(ed, l) {
                // org-fold-show-branches-buffer.
                if let Some(first) = (0..n_lines(ed)).find(|&i| is_heading(ed, i)) {
                    if first > 0 {
                        region(ed, 0, first - 1, true, Spec::Outline);
                    }
                    hide_sublevels(ed, 1);
                    let mut s = Some(first);
                    while let Some(h) = s {
                        show_children(ed, h, Some(1000));
                        s = syntax::next_sibling(&ed.buf, h);
                    }
                }
                hide_archived(ed, 0, n_lines(ed));
                Ok(())
            } else {
                let h = back_to_heading(ed, l).unwrap();
                let end = syntax::subtree_end(&ed.buf, h);
                fold_subtree(ed, h, true);
                show_children(ed, h, Some(1000));
                hide_archived(ed, h, end);
                Ok(())
            }
        }
        _ => return None,
    })
}

fn n_lines(ed: &Editor) -> usize {
    ed.line_count()
}

fn global_cycle(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    match arg {
        Prefix::Num(n) => {
            content(ed, Some(n.max(1) as usize));
            if let Some(o) = &mut ed.org {
                o.global_status = Some("contents");
            }
            Ok(())
        }
        Prefix::U(1) => {
            startup_visibility(ed);
            ed.set_msg("Startup visibility, plus VISIBILITY properties.");
            Ok(())
        }
        _ => {
            let repeat = repeated(
                ed,
                &[
                    "org-cycle",
                    "org-shifttab",
                    "org-cycle-global",
                    "org-global-cycle",
                ],
            );
            cycle_global(ed, repeat);
            Ok(())
        }
    }
}
