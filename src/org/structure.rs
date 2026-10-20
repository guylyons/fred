//! Structure editing (org.el): headings, promotion, subtree motion,
//! cutting and pasting, outline navigation, and the context dispatchers
//! (M-RET, M-arrows, S-arrows, C-c -, C-c *, C-c RET).

use super::buf::EBuf;
use super::sexp::Sexp;
use super::{Prefix, call, ctx, fold, syntax};
use crate::editor::Editor;
use crate::org_re;

/// Run `f` on an Emacs-like copy of the buffer, then write it back.
pub fn with_buf<T>(
    ed: &mut Editor,
    f: impl FnOnce(&mut EBuf) -> Result<T, String>,
) -> Result<T, String> {
    let mut b = EBuf::load(ed);
    let r = f(&mut b);
    if r.is_ok() {
        b.store(ed);
    }
    r
}

/// The cdr of `key` in an alist option (with `default` entries).
pub fn alist_option(name: &str, key: &str, fallback: Sexp) -> Sexp {
    let Some(v) = super::sexp::option(name) else {
        return fallback;
    };
    let Some(list) = v.list() else {
        // A non-list value applies to every key.
        return v;
    };
    let find = |k: &str| {
        list.iter()
            .find(|e| e.car().and_then(Sexp::str) == Some(k))
            .map(Sexp::cdr)
    };
    find(key).or_else(|| find("default")).unwrap_or(fallback)
}

fn odd_levels(ed: &Editor) -> bool {
    super::settings(ed).opt_bool("org-odd-levels-only", false)
}

/// org-get-valid-level.
pub fn valid_level(odd: bool, level: usize, change: i64) -> usize {
    let level = level as i64;
    if odd {
        let v = if change == 0 {
            1 + 2 * (level / 2)
        } else if change > 0 {
            1 + 2 * ((level - 1 + 2 * change) / 2)
        } else {
            (1 + 2 * ((level + 2 * change) / 2)).max(1)
        };
        v as usize
    } else {
        (level + change).max(1) as usize
    }
}

/// org--blank-before-heading-p.
fn blank_before_heading(b: &EBuf, parent: bool) -> bool {
    match alist_option(
        "org-blank-before-new-entry",
        "heading",
        Sexp::Sym("auto".into()),
    ) {
        Sexp::Sym(s) if s == "auto" => {
            let mut c = EBuf::new(&b.s, b.pt);
            if !c.back_to_heading() && !c.next_heading() {
                return false;
            }
            if parent {
                c.up_heading_safe();
            }
            if !c.bobp() {
                return c.previous_line_empty();
            }
            if c.next_heading() {
                return c.previous_line_empty();
            }
            c.skip_chars_backward(" \t", None);
            c.bolp() && c.previous_line_empty()
        }
        v => v.truthy(),
    }
}

/// org-N-empty-lines-before-current.
fn n_empty_lines_before_current(b: &mut EBuf, n: usize) {
    let col = b.current_column();
    b.beginning_of_line();
    if !b.bobp() {
        let mut c = EBuf::new(&b.s, b.pt);
        c.skip_chars_backward(" \r\t\n", None);
        let start = c.eol();
        let end = b.lep(0);
        if start < end {
            b.delete(start, end);
        }
    }
    b.insert(&"\n".repeat(n));
    b.move_to_column(col);
}

fn may_split_line(key: &str) -> bool {
    alist_option("org-M-RET-may-split-line", key, Sexp::T).truthy()
}

/// org-insert-heading.
pub fn insert_heading(ed: &mut Editor, arg: Prefix, level: Option<usize>) -> Result<(), String> {
    let respect = super::options::bool("org-insert-heading-respect-content", false);
    let invisible = fold::hidden(ed, ed.cur.line);
    let auto_align = super::options::bool("org-auto-align-tags", true);
    let st = super::settings(ed);
    let mut new_line_hidden = None;
    with_buf(ed, |b| {
        let blank = blank_before_heading(b, arg == Prefix::U(2));
        let current = b.current_level();
        let num = level.or(current).unwrap_or(1);
        let stars = "*".repeat(num);
        let add_blank_after = |b: &mut EBuf, blank: bool| {
            let save = b.pt;
            b.end_of_line();
            if !b.eobp() {
                b.pt += 1;
                if blank && b.at_heading() {
                    b.insert("\n");
                }
            }
            b.pt = save.min(b.len());
        };
        if respect || matches!(arg, Prefix::U(1) | Prefix::U(2)) || invisible {
            if current.is_none() {
                b.next_heading();
            } else {
                b.back_to_heading();
                if arg == Prefix::U(2) {
                    b.up_heading_safe();
                }
                b.end_of_subtree(true);
            }
            if !b.bolp() {
                b.insert("\n");
            }
            if blank && {
                let mut c = EBuf::new(&b.s, b.pt.saturating_sub(1));
                !c.back_to_heading()
            } {
                b.insert("\n");
                b.pt -= 1;
            }
            if current.is_none() && !b.eobp() && !b.bobp() {
                if b.at_heading() {
                    b.insert("\n");
                }
                b.pt -= 1;
            }
            if !(blank && b.previous_line_empty()) {
                n_empty_lines_before_current(b, usize::from(blank));
            }
            b.insert(&format!("{stars} \n"));
            b.pt -= 1;
            add_blank_after(b, blank);
            new_line_hidden = Some(b.line_number());
        } else if b.at_heading() {
            if b.bolp() {
                if blank {
                    b.insert_at(b.pt, "\n");
                }
                let p = b.pt;
                b.insert_at(p, &format!("{stars} \n"));
                b.pt = p;
                if !(blank && b.previous_line_empty()) {
                    n_empty_lines_before_current(b, usize::from(blank));
                }
                b.end_of_line();
            } else {
                let line = b.line().to_owned();
                let bol = b.bol();
                let h = syntax::headline(&line, &st).unwrap();
                let in_title = b.pt >= bol + h.title.start && b.pt <= bol + h.title.end;
                if may_split_line("headline") && in_title && !h.title.is_empty() {
                    let split = b.delete(b.pt, bol + h.title.end);
                    if b.s[b.pt..b.eol()].trim().is_empty() {
                        let e = b.eol();
                        b.delete(b.pt, e);
                    } else if auto_align {
                        align_tags_here(b, tags_column());
                    }
                    b.end_of_line();
                    if blank {
                        b.insert("\n");
                    }
                    b.insert(&format!("\n{stars} "));
                    add_blank_after(b, blank);
                    if !split.trim().is_empty() {
                        let p = b.pt;
                        b.insert(&split);
                        b.pt = p;
                    }
                } else {
                    b.end_of_line();
                    if blank {
                        b.insert("\n");
                    }
                    b.insert(&format!("\n{stars} "));
                    add_blank_after(b, blank);
                }
            }
        } else if b.bolp() {
            b.insert(&format!("{stars} "));
            if !(blank && b.previous_line_empty()) {
                n_empty_lines_before_current(b, usize::from(blank));
            }
            add_blank_after(b, blank);
        } else {
            if !may_split_line("headline") {
                b.end_of_line();
            }
            b.insert(&format!("\n{stars} "));
            if !(blank && b.previous_line_empty()) {
                n_empty_lines_before_current(b, usize::from(blank));
            }
            add_blank_after(b, blank);
        }
        Ok(())
    })?;
    let l = ed.cur.line;
    if new_line_hidden.is_some() || fold::hidden(ed, l) {
        fold::region(ed, l, l, false, fold::Spec::Outline);
    }
    Ok(())
}

/// org-tags-column.
pub fn tags_column() -> i64 {
    super::options::int("org-tags-column", -77)
}

/// org--align-tags-here on the line at point.
pub fn align_tags_here(b: &mut EBuf, to_col: i64) {
    let re = org_re!(r"^\*+ (?:.*[ \t])?(:[\w@#%:]+:)[ \t]*$");
    let Some(c) = b.match_line(re) else { return };
    let tags = c.get(1).unwrap();
    let (tags_start, tags_text) = (tags.start(), tags.as_str().to_owned());
    let mut x = EBuf::new(&b.s, tags_start);
    x.skip_chars_backward(" \t", None);
    let blank_start = x.pt;
    let width = unicode_width::UnicodeWidthStr::width(tags_text.as_str()) as i64;
    let min = super::buf::column(&b.s[b.bol()..blank_start]) as i64 + 1;
    let new = (if to_col >= 0 {
        to_col
    } else {
        to_col.abs() - width
    })
    .max(min) as usize;
    let current = super::buf::column(&b.s[b.bol()..tags_start]);
    let origin = b.pt;
    let column = b.current_column();
    let in_blank = origin > blank_start && origin <= tags_start;
    if new != current {
        let pad = " ".repeat(new - super::buf::column(&b.s[b.bol()..blank_start]));
        b.replace(blank_start, tags_start, &pad);
        if in_blank {
            b.move_to_column(column);
        } else if origin > tags_start {
            b.pt = origin + pad.len() - (tags_start - blank_start);
        } else {
            b.pt = origin;
        }
    }
}

/// org-promote / org-demote on the heading at point (`change` = -1/+1).
fn change_heading_level(
    b: &mut EBuf,
    change: i64,
    odd: bool,
    top_ok: bool,
    adapt: bool,
) -> Result<(), String> {
    if !b.back_to_heading() {
        return Err("Before first headline".into());
    }
    let level = b.at_heading_at(b.pt).unwrap();
    let new = valid_level(odd, level, change);
    let bol = b.pt;
    if change < 0 && level == 1 {
        if top_ok {
            b.replace(bol, bol + 2, "# ");
            return Ok(());
        }
        return Err("Cannot promote to level 0.  UNDO to recover if necessary".into());
    }
    b.replace(bol, bol + level, &"*".repeat(new));
    if super::options::bool("org-auto-align-tags", true) {
        let save = b.pt;
        b.pt = b.bol();
        align_tags_here(b, tags_column());
        b.pt = save.min(b.len());
    }
    let diff = new as i64 - level as i64;
    if adapt {
        fixup_indentation(b, diff);
    }
    Ok(())
}

/// org-fixup-indentation (org-adapt-indentation decides what moves).
fn fixup_indentation(b: &mut EBuf, diff: i64) {
    let adapt = super::sexp::option("org-adapt-indentation").map_or("nil".to_owned(), |v| {
        v.sym().map_or("t".into(), str::to_owned)
    });
    let save = b.pt;
    let heading = b.bol();
    let level = b.at_heading_at(heading).unwrap_or(0);
    let mut c = EBuf::new(&b.s, heading);
    c.next_heading();
    let end = c.pt;
    let indent_to = |b: &mut EBuf, at: usize, col: usize| {
        let line_end = b.eol_at(at);
        let text = &b.s[at..line_end];
        let ws = text.len() - text.trim_start().len();
        b.replace(at, at + ws, &" ".repeat(col));
    };
    // Planning and property drawer follow the heading's indentation.
    let target = if adapt == "nil" { 0 } else { level + 1 };
    let mut p = b.eol_at(heading) + 1;
    if p < b.len() && p <= end && ctx::is_planning(b.line_at(p)) {
        if adapt != "nil" || b.line_at(p).starts_with(' ') {
            indent_to(b, p, if adapt == "nil" { 0 } else { target });
        }
        p = b.eol_at(p) + 1;
    }
    if adapt == "t" && diff != 0 {
        // Shift other lines (but not headings or blank lines) by DIFF.
        let mut q = p.min(b.len());
        let mut c = EBuf::new(&b.s, heading);
        c.next_heading();
        let mut end = c.pt;
        // Check that a negative shift is possible.
        if diff < 0 {
            let mut r = q;
            while r < end {
                let t = b.line_at(r);
                if !t.trim().is_empty() && syntax::level(t).is_none() {
                    let ind = super::buf::column(&t[..t.len() - t.trim_start().len()]) as i64;
                    if ind < -diff {
                        b.pt = save;
                        return;
                    }
                }
                r = b.eol_at(r) + 1;
            }
        }
        while q < end && q < b.len() {
            let t = b.line_at(q).to_owned();
            let next_len = t.len();
            if !t.trim().is_empty() && syntax::level(&t).is_none() {
                let ws = t.len() - t.trim_start().len();
                let ind = super::buf::column(&t[..ws]) as i64;
                let new = " ".repeat((ind + diff).max(0) as usize);
                let delta = new.len() as i64 - ws as i64;
                b.replace(q, q + ws, &new);
                end = (end as i64 + delta) as usize;
                q = (q as i64 + next_len as i64 + delta) as usize + 1;
            } else {
                q += next_len + 1;
            }
        }
    }
    b.pt = save.min(b.len());
}

/// org-fix-position-after-promote.
fn fix_position_after_promote(b: &mut EBuf, st: &syntax::Settings) {
    let line = b.line().to_owned();
    let Some(h) = syntax::headline(&line, st) else {
        return;
    };
    let off = b.pt - b.bol();
    let ends = [h.level, h.todo_range.as_ref().map_or(usize::MAX, |r| r.end)];
    if ends.contains(&off) {
        if b.eobp() || b.eolp() {
            b.insert(" ");
        } else if b.char_after(b.pt) == Some(' ') {
            b.pt += 1;
        }
    }
}

/// Region of headings for the region versions (Visual-line selection).
fn region_lines(ed: &Editor) -> Option<(usize, usize)> {
    ed.org_region
}

/// org-do-promote / org-do-demote (`change` -1/+1).
pub fn do_promote(ed: &mut Editor, change: i64) -> Result<(), String> {
    let odd = odd_levels(ed);
    let st = super::settings(ed);
    let top_ok = super::options::bool("org-allow-promoting-top-level-subtree", false);
    let region = region_lines(ed);
    let adapt = adapt_override(ed);
    with_buf(ed, |b| {
        let save = b.pt;
        match region {
            Some((lo, hi)) => {
                let mut l = lo;
                while l <= hi {
                    let p = b.pos_of_line(l);
                    if b.at_heading_at(p).is_some() {
                        b.pt = p;
                        change_heading_level(b, change, odd, top_ok, adapt)?;
                    }
                    l += 1;
                }
            }
            None => change_heading_level(b, change, odd, top_ok, adapt)?,
        }
        let (sl, sc) = EBuf::new(&b.s, 0).line_col(save.min(b.len()));
        b.pt = b.pos_of_line(sl) + sc.min(b.line_at(b.pos_of_line(sl)).len());
        fix_position_after_promote(b, &st);
        Ok(())
    })
}

/// org-promote-subtree / org-demote-subtree.
pub fn promote_subtree(ed: &mut Editor, change: i64) -> Result<(), String> {
    let odd = odd_levels(ed);
    let st = super::settings(ed);
    let top_ok = super::options::bool("org-allow-promoting-top-level-subtree", false);
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let end = syntax::subtree_end(&ed.buf, h);
    let adapt = adapt_override(ed);
    with_buf(ed, |b| {
        let save_line = b.line_number();
        let save_col = b.pt - b.bol();
        if change < 0 && b.at_heading_at(b.pos_of_line(h)) == Some(1) && !top_ok {
            return Err("Cannot promote to level 0.  UNDO to recover if necessary".into());
        }
        // org-map-tree: the root and every heading below it.
        for l in h..end {
            let p = b.pos_of_line(l);
            if b.at_heading_at(p).is_some() {
                b.pt = p;
                change_heading_level(b, change, odd, top_ok, adapt)?;
            }
        }
        let ls = b.pos_of_line(save_line);
        b.pt = ls + save_col.min(b.line_at(ls).len());
        fix_position_after_promote(b, &st);
        Ok(())
    })
}

/// org-adapt-indentation, unless a caller bound it to nil.
fn adapt_override(ed: &Editor) -> bool {
    !ed.org.as_ref().is_some_and(|o| o.no_adapt)
}

/// org-cycle-level: TAB on an empty heading cycles its level. True if it acted.
pub fn cycle_level(ed: &mut Editor) -> Result<bool, String> {
    let st = super::settings(ed);
    let line = ed.buf.line(ed.cur.line);
    let Some(h) = syntax::headline(&line, &st) else {
        return Ok(false);
    };
    // org-point-at-end-of-empty-headline: rest of line blank, title empty.
    let rest_blank = line[ed.cur.byte.min(line.len())..].trim().is_empty();
    if !rest_blank || !h.title.is_empty() || h.tags_range.is_some() || ed.cur.byte < h.level {
        return Ok(false);
    }
    let cur = h.level;
    let prev = if ed.cur.line == 0 {
        0
    } else {
        syntax::heading_at_or_before(&ed.buf, ed.cur.line - 1)
            .map_or(0, |p| syntax::level(&ed.buf.line(p)).unwrap())
    };
    let inc = if odd_levels(ed) { 2 } else { 1 };
    if let Some(o) = &mut ed.org {
        o.no_adapt = true;
    }
    let step = |ed: &mut Editor, change: i64, times: usize| -> Result<(), String> {
        for _ in 0..times {
            do_promote(ed, change)?;
        }
        Ok(())
    };
    let r = if prev == 0 || prev == 1 && cur != 1 {
        step(ed, -1, (cur - 1) / inc)
    } else if prev == cur {
        step(ed, 1, 1)
    } else if cur == 1 {
        step(ed, 1, (prev - 1) / inc)
    } else if cur < prev {
        step(ed, -1, 1)
    } else {
        step(ed, -1, 1 + (cur - prev) / inc)
    };
    if let Some(o) = &mut ed.org {
        o.no_adapt = false;
    }
    r?;
    // Stay at the end of the (empty) heading.
    let len = ed.buf.line(ed.cur.line).len();
    ed.cur.byte = len;
    Ok(true)
}

/// org-move-subtree-down (negative `n` moves up).
pub fn move_subtree(ed: &mut Editor, n: i64) -> Result<(), String> {
    let col = {
        let l = ed.buf.line(ed.cur.line);
        super::buf::column(&l[..ed.cur.byte.min(l.len())])
    };
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let end = syntax::subtree_end(&ed.buf, h);
    let folded = h + 1 < end && fold::hidden(ed, h + 1);
    // Find the insertion line past |n| siblings.
    let mut target = h;
    for _ in 0..n.unsigned_abs() {
        let s = if n > 0 {
            syntax::next_sibling(&ed.buf, target)
        } else {
            syntax::prev_sibling(&ed.buf, target)
        };
        match s {
            Some(x) => target = x,
            None => return Err("Cannot move past superior level or buffer limit".into()),
        }
    }
    let ins = if n > 0 {
        syntax::subtree_end(&ed.buf, target)
    } else {
        target
    };
    let lines = super::lines(ed, h..end);
    let count = end - h;
    // Delete, then insert where the target now is.
    super::delete_lines(ed, h, count);
    let at = if ins > h { ins - count } else { ins };
    super::insert_lines(ed, at, &lines);
    if folded {
        fold::fold_subtree(ed, at, true);
    } else {
        fold::show_entry(ed, at);
        fold::show_children(ed, at, None);
    }
    fold::show_heading(ed, at);
    // org-clean-visibility-after-subtree-move: drawers stay hidden.
    let around = syntax::subtree_end(&ed.buf, at);
    fold::hide_drawers(ed, at, around);
    ed.set_cursor(at, 0);
    let text = ed.buf.line(at);
    let mut b = EBuf::new(&text, 0);
    b.move_to_column(col);
    ed.set_cursor(at, b.pt);
    Ok(())
}

thread_local! {
    /// org-subtree-clip and org-subtree-clip-folded.
    static CLIP: std::cell::RefCell<(String, bool)> = const { std::cell::RefCell::new((String::new(), false)) };
}

/// Put text in the kill ring: Fred's unnamed register (and clipboard).
pub fn kill_new(ed: &mut Editor, text: &str) {
    let linewise = text.ends_with('\n');
    let t = if linewise {
        text.strip_suffix('\n').unwrap()
    } else {
        text
    };
    crate::vim::ops::set_reg(
        ed,
        crate::editor::Register {
            text: t.to_owned(),
            linewise,
        },
    );
}

/// The kill ring's head (the clipboard when it changed since).
pub fn current_kill(ed: &mut Editor) -> String {
    if ed.clipboard()
        && let Some(c) = crate::clipboard::get()
        && c != crate::clipboard::to_clip(&ed.reg)
    {
        ed.reg = crate::clipboard::from_clip(&c);
    }
    if ed.reg.linewise {
        format!("{}\n", ed.reg.text)
    } else {
        ed.reg.text.clone()
    }
}

/// org-copy-subtree (cut when `cut`).
pub fn copy_subtree(ed: &mut Editor, n: usize, cut: bool) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let folded = h + 1 < syntax::subtree_end(&ed.buf, h) && fold::hidden(ed, h + 1);
    let mut last = h;
    for _ in 1..n.max(1) {
        match syntax::next_sibling(&ed.buf, last) {
            Some(x) => last = x,
            None => break,
        }
    }
    let end = syntax::subtree_end(&ed.buf, last);
    let text: String = super::lines(ed, h..end)
        .iter()
        .map(|l| format!("{l}\n"))
        .collect();
    if cut {
        super::delete_lines(ed, h, end - h);
        let l = h.min(ed.line_count() - 1);
        ed.set_cursor(l, 0);
    }
    CLIP.with(|c| *c.borrow_mut() = (text.clone(), folded));
    kill_new(ed, &text);
    ed.set_msg(format!(
        "{}: Subtree(s) with {} characters",
        if cut { "Cut" } else { "Copied" },
        text.chars().count()
    ));
    Ok(())
}

/// org-kill-is-subtree-p.
pub fn kill_is_subtree(txt: &str) -> bool {
    let start = txt.trim_start_matches([' ', '\t', '\n', '\r']);
    // The first non-blank line must be a heading.
    let lead = &txt[..txt.len() - start.len()];
    if !lead.is_empty() && !lead.contains('\n') && !lead.is_empty() {
        return false;
    }
    let Some(start_level) = syntax::level(start.lines().next().unwrap_or("")) else {
        return false;
    };
    start
        .lines()
        .skip(1)
        .filter_map(syntax::level)
        .all(|l| l >= start_level)
}

/// org-paste-subtree.
pub fn paste_subtree(
    ed: &mut Editor,
    level: Prefix,
    tree: Option<String>,
    for_yank: bool,
) -> Result<(), String> {
    let tree = tree.unwrap_or_else(|| current_kill(ed));
    if !kill_is_subtree(&tree) {
        return Err("The kill is not a (set of) tree(s).  Use `p' to yank anyway".into());
    }
    let odd = odd_levels(ed);
    let visible_heading = |ed: &Editor, from: usize, down: bool| -> Option<usize> {
        if down {
            (from + 1..ed.line_count()).find(|&i| !fold::hidden(ed, i) && ctx::at_heading(ed, i))
        } else {
            (0..from)
                .rev()
                .find(|&i| !fold::hidden(ed, i) && ctx::at_heading(ed, i))
        }
    };
    let mut cur = ed.cur.line;
    let cur_line = ed.buf.line(cur);
    let old_level = tree
        .lines()
        .find_map(syntax::level)
        .map_or(-1, |l| l as i64);
    let at_bol_heading = ed.cur.byte == 0 && syntax::level(&cur_line).is_some();
    let star_only = org_re!(r"^\*+[ \t]*$").is_match(&cur_line)
        && !cur_line[ed.cur.byte.min(cur_line.len())..].starts_with('*');
    let force_level: Option<i64> =
        if (level.is_none() || matches!(level, Prefix::U(1) | Prefix::U(2))) && star_only {
            Some(cur_line.trim_end().len() as i64)
        } else if level == Prefix::U(1) {
            syntax::heading_at_or_before(&ed.buf, cur)
                .map(|h| syntax::level(&ed.buf.line(h)).unwrap() as i64)
        } else if level == Prefix::U(2) {
            None
        } else if !level.is_none() {
            Some(level.value())
        } else if at_bol_heading {
            syntax::level(&cur_line).map(|l| l as i64)
        } else {
            None
        };
    let previous = if ctx::at_heading(ed, cur) {
        Some(cur)
    } else {
        visible_heading(ed, cur, false)
    }
    .map_or(1, |p| syntax::level(&ed.buf.line(p)).unwrap() as i64);
    let next = visible_heading(ed, cur, true)
        .map_or(1, |p| syntax::level(&ed.buf.line(p)).unwrap() as i64);
    let new_level = force_level.unwrap_or_else(|| {
        let child = if level == Prefix::U(2) {
            previous + 1
        } else {
            0
        };
        child.max(previous).max(next)
    });
    let shift = if old_level == -1 || new_level == -1 || old_level == new_level {
        0
    } else {
        new_level - old_level
    };
    if star_only {
        super::delete_lines(ed, cur, 1);
        cur = cur.min(ed.line_count().saturating_sub(1));
    }
    let bol_heading = !star_only && at_bol_heading && !matches!(level, Prefix::U(1) | Prefix::U(2));
    let at = if bol_heading
        || (star_only
            && cur < ed.line_count()
            && ctx::at_heading(ed, cur)
            && ed.buf.len_bytes() > 0)
    {
        cur
    } else {
        let from = if level == Prefix::U(1) {
            syntax::heading_at_or_before(&ed.buf, cur)
                .map_or(cur, |h| syntax::subtree_end(&ed.buf, h).saturating_sub(1))
        } else {
            cur
        };
        visible_heading(ed, from, true).unwrap_or(ed.line_count())
    };
    let lines: Vec<String> = tree
        .strip_suffix('\n')
        .unwrap_or(&tree)
        .split('\n')
        .map(|l| match syntax::level(l) {
            Some(n) if shift != 0 => {
                let mut lv = n;
                for _ in 0..shift.abs() {
                    lv = valid_level(odd, lv, shift.signum());
                }
                format!("{}{}", "*".repeat(lv), &l[n..])
            }
            _ => l.to_owned(),
        })
        .collect();
    let n = lines.len();
    super::insert_lines(ed, at, &lines);
    let first = (at..at + n)
        .find(|&i| !ed.buf.line(i).trim().is_empty())
        .unwrap_or(at);
    ed.set_cursor(
        if for_yank {
            (at + n).min(ed.line_count() - 1)
        } else {
            first
        },
        0,
    );
    ed.set_msg(format!("Clipboard pasted as level {new_level} subtree"));
    let (clip, folded) = CLIP.with(|c| c.borrow().clone());
    fold::show_heading(ed, first);
    if !for_yank && clip == tree && folded {
        fold::fold_subtree(ed, first, true);
    }
    Ok(())
}

/// org-forward-heading-same-level (negative `n` goes backward).
pub fn forward_heading_same_level(ed: &mut Editor, n: i64, invisible_ok: bool) {
    let l = ed.cur.line;
    let Some(h) = fold::back_to_heading(ed, l) else {
        if n < 0 {
            ed.set_cursor(0, 0);
        } else if let Some(x) = syntax::next_heading(&ed.buf, l, usize::MAX) {
            ed.set_cursor(x, 0);
        }
        return;
    };
    let level = syntax::level(&ed.buf.line(h)).unwrap();
    let mut count = n.unsigned_abs();
    let mut result = h;
    let lines: Box<dyn Iterator<Item = usize>> = if n < 0 {
        Box::new((0..h).rev())
    } else {
        Box::new(h + 1..ed.line_count())
    };
    for i in lines {
        if count == 0 {
            break;
        }
        let Some(lv) = syntax::level(&ed.buf.line(i)) else {
            continue;
        };
        if lv < level {
            break;
        }
        if lv == level && (invisible_ok || !fold::hidden(ed, i)) {
            count -= 1;
            result = i;
        }
    }
    ed.set_cursor(result, 0);
}

/// org-next-visible-heading.
pub fn next_visible_heading(ed: &mut Editor, n: i64) {
    let mut l = ed.cur.line;
    let total = ed.line_count();
    let mut n = n;
    while n > 0 {
        match (l + 1..total).find(|&i| ctx::at_heading(ed, i) && !fold::hidden(ed, i)) {
            Some(x) => l = x,
            None => {
                ed.set_cursor(total - 1, ed.buf.line(total - 1).len());
                return;
            }
        }
        n -= 1;
    }
    while n < 0 {
        match (0..l)
            .rev()
            .find(|&i| ctx::at_heading(ed, i) && !fold::hidden(ed, i))
        {
            Some(x) => l = x,
            None => {
                l = 0;
                break;
            }
        }
        n += 1;
    }
    ed.set_cursor(l, 0);
}

/// outline-up-heading.
pub fn up_heading(ed: &mut Editor, n: i64, invisible_ok: bool) -> Result<(), String> {
    let mut h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first heading")?;
    for _ in 0..n.max(1) {
        let mut p = h;
        loop {
            match syntax::parent(&ed.buf, p) {
                Some(x) if !invisible_ok && fold::hidden(ed, x) => p = x,
                Some(x) => {
                    h = x;
                    break;
                }
                None => return Err("Already at top level of the outline".into()),
            }
        }
    }
    ed.set_cursor(h, 0);
    Ok(())
}

/// org-toggle-heading.
pub fn toggle_heading(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let st = super::settings(ed);
    let region = ed.org_region;
    let l = ed.cur.line;
    let (lo, hi) = match region {
        Some(r) => r,
        None => {
            if !arg.is_none() && ctx::at_item(ed, l) {
                // The whole list at point.
                let mut a = l;
                while a > 0
                    && (ctx::at_item(ed, a - 1)
                        || ed.buf.line(a - 1).starts_with(char::is_whitespace)
                            && !ed.buf.line(a - 1).trim().is_empty())
                {
                    a -= 1;
                }
                let mut z = l;
                while z + 1 < ed.line_count()
                    && (ctx::at_item(ed, z + 1)
                        || ed.buf.line(z + 1).starts_with(char::is_whitespace)
                            && !ed.buf.line(z + 1).trim().is_empty())
                {
                    z += 1;
                }
                (a, z)
            } else {
                (l, l)
            }
        }
    };
    let mut lo = lo;
    while lo < hi
        && (ed.buf.line(lo).trim().is_empty() || ed.buf.line(lo).trim_start().starts_with("# "))
    {
        lo += 1;
    }
    let lines: Vec<String> = (lo..=hi).map(|i| ed.buf.line(i)).collect();
    let first = &lines[0];
    let parent_level = |ed: &Editor| {
        syntax::heading_at_or_before(&ed.buf, lo.saturating_sub(if lo > 0 { 1 } else { 0 }))
            .filter(|&h| h < lo || lo == 0 && false)
            .map_or(0, |h| syntax::level(&ed.buf.line(h)).unwrap())
    };
    let odd = odd_levels(ed);
    let stars_for = |n: Option<i64>, base: usize| -> String {
        match n {
            Some(n) => "*".repeat(n.max(1) as usize),
            None => "*".repeat(valid_level(odd, base, 1)),
        }
    };
    let nstars = match arg {
        Prefix::Num(n) => Some(n),
        _ => None,
    };
    let base = if lo > 0 { parent_level(ed) } else { 0 };
    let out: Vec<String>;
    if syntax::level(first).is_some() {
        out = lines
            .iter()
            .map(|t| match syntax::level(t) {
                Some(n) => t[n + 1..].to_owned(),
                None => t.clone(),
            })
            .collect();
    } else if let Some((_, _)) = ctx::item_bullet(first) {
        let not_done = st.not_done_names().first().map(|s| s.to_string());
        let done = st.done_names().first().map(|s| s.to_string());
        let min_ind = lines
            .iter()
            .filter_map(|t| ctx::item_bullet(t).map(|(i, _)| i))
            .min()
            .unwrap_or(0);
        out = lines
            .iter()
            .map(|t| match ctx::item_bullet(t) {
                Some((ind, bullet)) => {
                    let mut rest = t[ind + bullet.len()..].to_owned();
                    let mut kw = String::new();
                    for (cb, k) in [("[ ] ", &not_done), ("[X] ", &done), ("[-] ", &not_done)] {
                        if let Some(r) = rest.strip_prefix(cb) {
                            if let Some(k) = k {
                                kw = format!("{k} ");
                            }
                            rest = r.to_owned();
                            break;
                        }
                    }
                    let depth = (ind - min_ind) / 2;
                    let stars = stars_for(nstars, base);
                    let extra = if odd { 2 * depth } else { depth };
                    format!("{}{} {kw}{rest}", stars, "*".repeat(extra))
                }
                None => t.clone(),
            })
            .collect();
    } else {
        let toggle_one = !arg.is_none() && region.is_some() && nstars.is_none();
        let mut done_one = false;
        out = lines
            .iter()
            .map(|t| {
                if t.trim().is_empty()
                    || ctx::item_bullet(t).is_some()
                    || syntax::level(t).is_some()
                    || (toggle_one && done_one)
                {
                    return t.clone();
                }
                done_one = true;
                format!("{} {}", stars_for(nstars, base), t.trim_start())
            })
            .collect();
    }
    super::splice(ed, lo, hi - lo + 1, &out);
    Ok(())
}

/// org-mark-subtree: select it in Visual-line mode.
pub fn mark_subtree(ed: &mut Editor, up: i64) -> Result<(), String> {
    let mut h = fold::back_to_heading(ed, ed.cur.line).ok_or("Not in a subtree")?;
    for _ in 0..up {
        match syntax::parent(&ed.buf, h) {
            Some(p) => h = p,
            None => break,
        }
    }
    let end = syntax::subtree_end(&ed.buf, h);
    ed.mode = crate::editor::Mode::VisualLine { anchor: h };
    ed.set_cursor(end - 1, 0);
    Ok(())
}

/// org-convert-to-odd-levels / org-convert-to-oddeven-levels.
fn convert_levels(ed: &mut Editor, to_odd: bool) -> Result<(), String> {
    if !to_odd
        && (0..ed.line_count()).any(|l| syntax::level(&ed.buf.line(l)).is_some_and(|n| n % 2 == 0))
    {
        return Err("Not all levels are odd in this file.  Conversion not possible".into());
    }
    let q = if to_odd {
        "Are you sure you want to globally change levels to odd? "
    } else {
        "Are you sure you want to globally change levels to odd-even? "
    };
    super::yes_or_no(ed, q, move |ed| {
        let lines: Vec<String> = (0..ed.line_count())
            .map(|l| {
                let t = ed.buf.line(l);
                match syntax::level(&t) {
                    Some(n) if n > 1 => {
                        let new = if to_odd { 2 * n - 1 } else { n.div_ceil(2) };
                        format!("{}{}", "*".repeat(new), &t[n..])
                    }
                    _ => t,
                }
            })
            .collect();
        let n = ed.line_count();
        ed.undo.begin(ed.cur.pos());
        super::splice(ed, 0, n, &lines);
        ed.undo.end(ed.cur.pos());
    });
    Ok(())
}

/// org-copy-visible on the region (or the whole buffer).
fn copy_visible(ed: &mut Editor) {
    let (lo, hi) = ed.org_region.unwrap_or((0, ed.line_count() - 1));
    let text: String = (lo..=hi)
        .filter(|&l| !fold::hidden(ed, l))
        .map(|l| format!("{}\n", ed.buf.line(l)))
        .collect();
    kill_new(ed, &text);
    ed.set_msg("Visible strings have been copied to the kill ring.");
}

/// The commands of this module.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let l = ed.cur.line;
    Some(match name {
        "org-insert-heading" => insert_heading(ed, arg, None),
        "org-insert-heading-respect-content" => insert_heading(ed, Prefix::U(1), None),
        "org-insert-todo-heading" => insert_todo_heading(ed, arg, false),
        "org-insert-todo-heading-respect-content" => insert_todo_heading(ed, arg, true),
        "org-insert-subheading" => {
            let line = ed.buf.line(l);
            if ed.cur.byte == 0 && !line.is_empty() {
                ed.cur.byte = 1;
            }
            insert_heading(ed, arg, None).and_then(|()| {
                if ctx::at_heading(ed, ed.cur.line) {
                    do_promote(ed, 1)
                } else if ctx::at_item(ed, ed.cur.line) {
                    call(ed, "org-indent-item", Prefix::None)
                } else {
                    Ok(())
                }
            })
        }
        "org-insert-todo-subheading" => insert_todo_heading(ed, arg, false).and_then(|()| {
            if ctx::at_heading(ed, ed.cur.line) {
                do_promote(ed, 1)
            } else if ctx::at_item(ed, ed.cur.line) {
                call(ed, "org-indent-item", Prefix::None)
            } else {
                Ok(())
            }
        }),
        "org-meta-return" => {
            if !arg.is_none() {
                insert_heading(ed, arg, None)
            } else if ctx::at_table(ed, l) {
                call(ed, "org-table-wrap-region", arg)
            } else if in_item(ed, l) {
                call(ed, "org-insert-item", arg)
            } else {
                insert_heading(ed, arg, None)
            }
        }
        "org-do-promote" => do_promote(ed, -1),
        "org-do-demote" => do_promote(ed, 1),
        "org-promote-subtree" => promote_subtree(ed, -1),
        "org-demote-subtree" => promote_subtree(ed, 1),
        "org-cycle-level" => cycle_level(ed).map(|_| ()),
        "org-move-subtree-down" => move_subtree(ed, arg.value()),
        "org-move-subtree-up" => move_subtree(ed, -arg.value()),
        "org-cut-subtree" => copy_subtree(ed, arg.value().max(1) as usize, true),
        "org-copy-subtree" => copy_subtree(ed, arg.value().max(1) as usize, false),
        "org-paste-subtree" => paste_subtree(ed, arg, None, false),
        "org-cut-special" => {
            if ctx::at_table(ed, l) {
                call(ed, "org-table-cut-region", arg)
            } else {
                copy_subtree(ed, arg.value().max(1) as usize, true)
            }
        }
        "org-copy-special" => {
            if ctx::at_table(ed, l) {
                call(ed, "org-table-copy-region", arg)
            } else {
                copy_subtree(ed, arg.value().max(1) as usize, false)
            }
        }
        "org-paste-special" => {
            if ctx::at_table(ed, l) {
                call(ed, "org-table-paste-rectangle", arg)
            } else {
                paste_subtree(ed, arg, None, false)
            }
        }
        "org-forward-heading-same-level" | "outline-forward-same-level" => {
            forward_heading_same_level(ed, arg.value(), false);
            Ok(())
        }
        "org-backward-heading-same-level" | "outline-backward-same-level" => {
            forward_heading_same_level(ed, -arg.value(), false);
            Ok(())
        }
        "org-next-visible-heading" | "outline-next-visible-heading" => {
            next_visible_heading(ed, arg.value());
            Ok(())
        }
        "org-previous-visible-heading" | "outline-previous-visible-heading" => {
            next_visible_heading(ed, -arg.value());
            Ok(())
        }
        "outline-up-heading" | "org-up-heading" => up_heading(ed, arg.value(), false),
        "org-up-heading-all" => up_heading(ed, arg.value(), true),
        "org-toggle-heading" => toggle_heading(ed, arg),
        "org-ctrl-c-star" => {
            if ctx::at_table(ed, l) {
                call(ed, "org-table-recalculate", arg)
            } else {
                toggle_heading(ed, arg)
            }
        }
        "org-ctrl-c-minus" => {
            if ctx::at_table(ed, l) {
                call(ed, "org-table-insert-hline", arg)
            } else if ed.org_region.is_some() {
                call(ed, "org-toggle-item", arg)
            } else if in_item(ed, l) {
                call(ed, "org-cycle-list-bullet", arg)
            } else {
                call(ed, "org-toggle-item", arg)
            }
        }
        "org-ctrl-c-ret" => {
            if ctx::at_table(ed, l) {
                call(ed, "org-table-hline-and-move", arg)
            } else {
                insert_heading(ed, arg, None)
            }
        }
        "org-mark-subtree" => mark_subtree(ed, if arg.is_none() { 0 } else { arg.value() }),
        "org-convert-to-odd-levels" => convert_levels(ed, true),
        "org-convert-to-oddeven-levels" => convert_levels(ed, false),
        "org-copy-visible" => {
            copy_visible(ed);
            Ok(())
        }
        "org-metaup" => meta_vertical(ed, arg, -1),
        "org-metadown" => meta_vertical(ed, arg, 1),
        "org-metaleft" => meta_horizontal(ed, arg, -1),
        "org-metaright" => meta_horizontal(ed, arg, 1),
        "org-shiftmetaleft" => shiftmeta_horizontal(ed, arg, -1),
        "org-shiftmetaright" => shiftmeta_horizontal(ed, arg, 1),
        "org-shiftmetaup" => shiftmeta_vertical(ed, arg, -1),
        "org-shiftmetadown" => shiftmeta_vertical(ed, arg, 1),
        _ => return None,
    })
}

/// org-in-item-p: inside a plain list item (its first line or body).
pub fn in_item(ed: &Editor, l: usize) -> bool {
    if ctx::in_block(ed, l) {
        return false;
    }
    let mut i = l;
    let mut min_ind = usize::MAX;
    loop {
        let t = ed.buf.line(i);
        if syntax::level(&t).is_some() {
            return false;
        }
        if let Some((ind, _)) = ctx::item_bullet(&t)
            && ind < min_ind
        {
            return true;
        }
        if !t.trim().is_empty() {
            let ind = t.len() - t.trim_start().len();
            if ind == 0 && i != l || ind == 0 && ctx::item_bullet(&t).is_none() {
                return false;
            }
            min_ind = min_ind.min(ind);
        }
        if i == 0 {
            return false;
        }
        i -= 1;
    }
}

/// org-insert-todo-heading.
fn insert_todo_heading(ed: &mut Editor, arg: Prefix, respect: bool) -> Result<(), String> {
    let l = ed.cur.line;
    if !respect && super::list::insert_item_cmd(ed, true)? {
        return Ok(());
    }
    let st = super::settings(ed);
    let prev_kw = fold::back_to_heading(ed, l)
        .and_then(|h| syntax::headline(&ed.buf.line(h), &st))
        .and_then(|h| h.todo);
    insert_heading(
        ed,
        if arg == Prefix::U(2) {
            Prefix::U(2)
        } else if respect {
            Prefix::U(1)
        } else {
            Prefix::None
        },
        None,
    )?;
    let first = st
        .todo_names()
        .first()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "TODO".into());
    let mark = match (&prev_kw, arg) {
        (_, Prefix::U(1)) => first,
        (Some(k), _) if !st.is_done(k) => k.clone(),
        _ => first,
    };
    let as_state_change =
        super::options::bool("org-treat-insert-todo-heading-as-state-change", false);
    if as_state_change {
        return call(ed, "org-todo", Prefix::None);
    }
    let nl = ed.cur.line;
    let text = ed.buf.line(nl);
    if let Some(n) = syntax::level(&text) {
        let new = format!("{} {mark} {}", &text[..n], &text[n + 1..]);
        super::set_line(ed, nl, &new);
        ed.cur.byte = n + 1 + mark.len() + 1;
    }
    if super::options::bool("org-provide-todo-statistics", true) {
        let _ = call(ed, "org-update-parent-todo-statistics", Prefix::None);
    }
    Ok(())
}

/// org-metaup/down: table rows, items, subtrees, or drag elements.
fn meta_vertical(ed: &mut Editor, arg: Prefix, dir: i64) -> Result<(), String> {
    let l = ed.cur.line;
    if ed.org_region.is_some() && !ctx::at_heading(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-drag-element-backward"
            } else {
                "org-drag-element-forward"
            },
            arg,
        );
    }
    if ctx::at_table(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-table-move-row-up"
            } else {
                "org-table-move-row-down"
            },
            arg,
        );
    }
    if ctx::at_heading(ed, l) {
        return move_subtree(ed, dir * arg.value());
    }
    if ctx::at_item(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-move-item-up"
            } else {
                "org-move-item-down"
            },
            arg,
        );
    }
    call(
        ed,
        if dir < 0 {
            "org-drag-element-backward"
        } else {
            "org-drag-element-forward"
        },
        arg,
    )
}

/// org-metaleft/right: promote/demote, outdent/indent items, table columns.
fn meta_horizontal(ed: &mut Editor, arg: Prefix, dir: i64) -> Result<(), String> {
    let l = ed.cur.line;
    if ctx::at_table(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-table-move-column-left"
            } else {
                "org-table-move-column-right"
            },
            arg,
        );
    }
    if ed.org_region.is_some_and(|(lo, _)| ctx::at_heading(ed, lo)) || ctx::at_heading(ed, l) {
        return do_promote(ed, dir);
    }
    if ed.org_region.is_some_and(|(lo, _)| ctx::at_item(ed, lo)) || ctx::at_item(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-outdent-item"
            } else {
                "org-indent-item"
            },
            arg,
        );
    }
    // Fred has no backward-word/forward-word fallback here: move by word.
    let key = if dir < 0 { 'b' } else { 'w' };
    crate::vim::normal_key(ed, crate::key::Key::ch(key));
    Ok(())
}

/// org-shiftmetaleft/right.
fn shiftmeta_horizontal(ed: &mut Editor, arg: Prefix, dir: i64) -> Result<(), String> {
    let l = ed.cur.line;
    if ctx::at_table(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-table-delete-column"
            } else {
                "org-table-insert-column"
            },
            arg,
        );
    }
    if ctx::at_heading(ed, l) {
        return promote_subtree(ed, dir);
    }
    if ctx::at_item(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-outdent-item-tree"
            } else {
                "org-indent-item-tree"
            },
            arg,
        );
    }
    Err("No command for this context".into())
}

/// org-shiftmetaup/down.
fn shiftmeta_vertical(ed: &mut Editor, arg: Prefix, dir: i64) -> Result<(), String> {
    let l = ed.cur.line;
    if ctx::at_table(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-table-kill-row"
            } else {
                "org-table-insert-row"
            },
            arg,
        );
    }
    if ctx::at_clock_log(ed, l) {
        return call(
            ed,
            if dir < 0 {
                "org-clock-timestamps-up"
            } else {
                "org-clock-timestamps-down"
            },
            arg,
        );
    }
    call(
        ed,
        if dir < 0 {
            "org-drag-line-backward"
        } else {
            "org-drag-line-forward"
        },
        arg,
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::{org, shown};
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(valid_level(true, 1, 1), 3);
        assert_eq!(valid_level(true, 3, -1), 1);
        assert_eq!(valid_level(false, 1, -1), 1);
        assert!(kill_is_subtree("** a\n*** b\n** c\n"));
        assert!(!kill_is_subtree("** a\n* b\n"));
        assert!(!kill_is_subtree("text\n* b\n"));
    }

    #[test]
    fn meta_return_inserts_headings() {
        let e = org("* A\n* B", "A<M-Enter>x<Esc>");
        assert_eq!(e.buf.text(), "* A\n* x\n* B");
        // Splitting the title at point.
        let e = org("* Hello world", "7l<M-Enter>");
        assert_eq!(e.buf.text(), "* Hello\n*  world");
        // Blank line before when the previous heading has one (auto).
        let e = org("* A\n\n* B", "GA<M-Enter>x<Esc>");
        assert_eq!(e.buf.text(), "* A\n\n* B\n\n* x");
        // At the beginning of a heading: insert above.
        let e = org("* A\n** B", "j0<M-Enter>ax<Esc>");
        assert_eq!(e.buf.text(), "* A\n** x\n** B");
        // C-RET: after the subtree.
        let e = org("* A\nbody\n** sub\n* B", " u<M-Enter>ax<Esc>");
        assert_eq!(e.buf.text(), "* A\nbody\n** sub\n* x\n* B");
        // A plain line becomes a heading at bol.
        let e = org("text", "0<M-Enter>");
        assert_eq!(e.buf.text(), "* text");
    }

    #[test]
    fn promote_demote_and_subtrees() {
        let e = org("* A :tag:\n** B", "<M-Right>");
        assert_eq!(e.buf.line(0), format!("** A{}:tag:", " ".repeat(68)));
        let e = org("* A\n** B\ntext\n* C", "<M-S-Right>");
        assert_eq!(e.buf.text(), "** A\n*** B\ntext\n* C");
        let e = org("* A\n** B\n* C", "<M-S-Left>");
        assert!(
            e.msg
                .as_ref()
                .unwrap()
                .0
                .ends_with("Cannot promote to level 0.  UNDO to recover if necessary")
        );
        let e = org("** A\n*** B\n** C", "<M-S-Left>");
        assert_eq!(e.buf.text(), "* A\n** B\n** C");
    }

    #[test]
    fn move_cut_paste_subtrees() {
        let e = org("* A\na\n* B\nb\n* C", "<M-Down>");
        assert_eq!(e.buf.text(), "* B\nb\n* A\na\n* C");
        assert_eq!(e.cur.line, 2);
        let e = org("* A\na\n* B\nb", "Gk<M-Up>");
        assert_eq!(e.buf.text(), "* B\nb\n* A\na");
        let e = org("* A\n* B", "<M-Up>");
        assert!(
            e.msg
                .as_ref()
                .unwrap()
                .0
                .ends_with("Cannot move past superior level or buffer limit")
        );
        let mut e = org("* A\na\n* B", "<C-c><C-x><C-w>");
        assert_eq!(e.buf.text(), "* B");
        assert_eq!(e.reg.text, "* A\na");
        for k in crate::key::parse_keys("A<Esc><C-c><C-x><C-y>") {
            e.handle_key(k);
        }
        assert_eq!(e.buf.text(), "* B\n* A\na");
        // Pasting under a deeper heading re-levels the tree.
        let e = org("* A\n** B\nb", "jj<C-c><C-x><M-w>");
        assert_eq!(e.reg.text, "** B\nb");
        let mut e = org("* X\n*** Y", "");
        e.reg = crate::editor::Register {
            text: "* T\n** U".into(),
            linewise: true,
        };
        for k in crate::key::parse_keys("A<Esc><C-c><C-x><C-y>") {
            e.handle_key(k);
        }
        assert_eq!(e.buf.text(), "* X\n*** T\n**** U\n*** Y");
        let _ = shown(&e);
    }

    #[test]
    fn navigation_and_toggle_heading() {
        let e = org("* A\n** a1\n** a2\n* B", "j<C-c><C-f>");
        assert_eq!(e.cur.line, 2);
        let e = org("* A\n** a1\n** a2\n* B", "jj<C-c><C-u>");
        assert_eq!(e.cur.line, 0);
        let e = org("* A\n** a1\n** a2\n* B", "<C-c><C-n><C-c><C-n>");
        assert_eq!(e.cur.line, 2);
        let e = org("* A\ntext", "j<C-c>*");
        assert_eq!(e.buf.text(), "* A\n** text");
        let e = org("* A", "<C-c>*");
        assert_eq!(e.buf.text(), "A");
        let e = org("- [ ] a\n- [X] b", " u<C-c>*");
        assert_eq!(e.buf.text(), "* TODO a\n* DONE b");
    }

    #[test]
    fn tab_on_empty_heading_cycles_level() {
        let e = org("* A\n** B\n", "GA<M-Enter><Tab>");
        assert_eq!(e.buf.text(), "* A\n** B\n*** ");
        let e = org("* A\n** B\n", "GA<M-Enter><Tab><Tab>");
        assert_eq!(e.buf.text(), "* A\n** B\n* ");
    }
}
