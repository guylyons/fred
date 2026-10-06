//! org-inlinetask.el: inline tasks, headlines of at least
//! org-inlinetask-min-level stars that do not start a subtree, closed by
//! an optional `*** END` line. Insertion (C-c C-x t, wrapping the
//! region), promotion/demotion (M-left/M-right on a task), TAB folding
//! of the task body, the org-cycle-hook that hides task bodies, and
//! the faces (see face.rs).

use super::buf::EBuf;
use super::sexp::Sexp;
use super::syntax::Settings;
use super::{Prefix, fold};
use crate::editor::Editor;

/// org-inlinetask-min-level (None: inline tasks off).
pub fn min_level(st: &Settings) -> Option<usize> {
    match st.opt("org-inlinetask-min-level") {
        None => Some(15),
        Some(Sexp::Int(n)) if n > 0 => Some(n as usize),
        Some(Sexp::Str(s)) => s.parse().ok(),
        _ => None,
    }
}

/// Stars of the smallest inline task (odd levels count double); usize::MAX
/// when inline tasks are off.
pub fn min_level_stars(st: &Settings) -> usize {
    match min_level(st) {
        Some(n) if st.opt_bool("org-odd-levels-only", false) => 2 * n - 1,
        Some(n) => n,
        None => usize::MAX,
    }
}

/// org-inlinetask-outline-regexp on a line: the stars of a task line
/// (END lines included).
pub fn task_stars(line: &str, st: &Settings) -> Option<usize> {
    let n = line.bytes().take_while(|&b| b == b'*').count();
    (n >= min_level_stars(st) && matches!(line.as_bytes().get(n), Some(b' ' | b'\t'))).then_some(n)
}

/// org-inlinetask-end-p.
pub fn end_p(line: &str, st: &Settings) -> bool {
    task_stars(line, st).is_some_and(|n| {
        let rest = line[n..].trim_start_matches([' ', '\t']);
        rest.len() >= 3
            && rest[..3].eq_ignore_ascii_case("END")
            && rest[3..].trim_matches([' ', '\t']).is_empty()
    })
}

/// org-inlinetask-at-task-p.
pub fn at_task_p(line: &str, st: &Settings) -> bool {
    task_stars(line, st).is_some() && !end_p(line, st)
}

fn lines(ed: &Editor) -> Vec<String> {
    (0..ed.line_count()).map(|l| ed.buf.line(l)).collect()
}

/// org-inlinetask-in-task-p for line `l`.
pub fn in_task_p(ls: &[String], l: usize, st: &Settings) -> bool {
    if task_stars(&ls[l], st).is_some() {
        return true;
    }
    (l..ls.len())
        .find(|&i| super::syntax::level(&ls[i]).is_some() || heading_tab(&ls[i]))
        .is_some_and(|i| end_p(&ls[i], st))
}

/// `^\*+[ \t]+` with a tab.
fn heading_tab(line: &str) -> bool {
    let n = line.bytes().take_while(|&b| b == b'*').count();
    n > 0 && line.as_bytes().get(n) == Some(&b'\t')
}

/// org-inlinetask-goto-beginning: the task line at or before `l`.
pub fn goto_beginning(ls: &[String], l: usize, st: &Settings) -> Option<usize> {
    let b = (0..=l).rev().find(|&i| task_stars(&ls[i], st).is_some())?;
    if end_p(&ls[b], st) {
        return (0..b)
            .rev()
            .find(|&i| task_stars(&ls[i], st).is_some())
            .or(Some(b));
    }
    Some(b)
}

/// org-inlinetask-goto-end: the line after the task at `l`.
pub fn goto_end(ls: &[String], l: usize, st: &Settings) -> usize {
    let n = ls.len();
    let next_task = |from: usize| (from..n).find(|&i| task_stars(&ls[i], st).is_some());
    if end_p(&ls[l], st) {
        return l + 1;
    }
    if task_stars(&ls[l], st).is_some() {
        let k = l + 1;
        if k >= n {
            return n;
        }
        if end_p(&ls[k], st) {
            return k + 1;
        }
        if task_stars(&ls[k], st).is_some() {
            return k;
        }
        if in_task_p(ls, k, st) {
            return next_task(k).map_or(n, |i| i + 1);
        }
        return k;
    }
    next_task(l).map_or(l + 1, |i| i + 1).min(n)
}

/// The task body lines (after the task line, through END): `(start, last)`.
fn body(ls: &[String], l: usize, st: &Settings) -> Option<(usize, usize)> {
    let b = goto_beginning(ls, l, st)?;
    let e = goto_end(ls, b, st);
    (e > b + 1).then_some((b + 1, e - 1))
}

/// org-inlinetask-toggle-visibility: `Some(true)` folds, `Some(false)`
/// unfolds, None toggles.
pub fn toggle_visibility(ed: &mut Editor, l: usize, state: Option<bool>) {
    let st = super::settings(ed);
    let ls = lines(ed);
    let Some((s, e)) = body(&ls, l, &st) else {
        return;
    };
    let fold_it = state.unwrap_or_else(|| !ed.folds.hidden(s));
    fold::region(ed, s, e, fold_it, fold::Spec::Outline);
}

/// org-inlinetask-hide-tasks (org-cycle-hook) for `contents` (whole
/// buffer) and `children` (from heading `h`).
pub fn hide_tasks(ed: &mut Editor, state: &str, h: usize) {
    let st = super::settings(ed);
    if min_level(&st).is_none() {
        return;
    }
    let ls = lines(ed);
    let n = ls.len();
    match state {
        "contents" => {
            let mut l = 0;
            while let Some(t) = (l..n).find(|&i| task_stars(&ls[i], &st).is_some()) {
                toggle_visibility(ed, t, Some(true));
                l = goto_end(&ls, t, &st).max(t + 1);
            }
        }
        "children" => {
            let mut l = h;
            loop {
                if l >= n {
                    break;
                }
                if !at_task_p(&ls[l], &st) {
                    match (l + 1..n).find(|&i| super::syntax::level(&ls[i]).is_some()) {
                        Some(i) if at_task_p(&ls[i], &st) => l = i,
                        _ => break,
                    }
                }
                toggle_visibility(ed, l, Some(true));
                l = goto_end(&ls, l, &st).max(l + 1);
            }
        }
        _ => {}
    }
}

/// TAB on an inline task (org-cycle's delegation): true when handled.
pub fn cycle(ed: &mut Editor) -> bool {
    let st = super::settings(ed);
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    if !at_task_p(&line, &st) {
        return false;
    }
    let exc = st.opt_sym("org-cycle-emulate-tab", "t") == "exc-hl-bol";
    if ed.cur.byte != 0 && exc {
        return false;
    }
    toggle_visibility(ed, l, None);
    true
}

/// org-inlinetask-insert-task.
pub fn insert_task(ed: &mut Editor, no_state: bool) -> Result<(), String> {
    let st = super::settings(ed);
    let ls = lines(ed);
    let l = ed.cur.line;
    if in_task_p(&ls, l, &st) && !(at_task_p(&ls[l], &st) && ed.cur.byte == 0) {
        return Err("Cannot nest inline tasks".into());
    }
    let region = super::region(ed);
    let mut b = EBuf::load_emacs(ed);
    if !b.bolp() {
        b.insert("\n");
    }
    let nstars = min_level_stars(&st);
    if nstars == usize::MAX {
        return Err("Inline tasks are off (org-inlinetask-min-level is nil)".into());
    }
    let indent = format!("{} ", "*".repeat(nstars));
    let (rbeg, rend) = match region {
        Some((lo, hi)) => (b.pos_of_line(lo), b.eol_at(b.pos_of_line(hi))),
        None => (b.pt, b.pt),
    };
    let rend_m = b.mark(rend);
    let rbeg_m = b.mark(rbeg);
    b.goto(b.marker(rend_m));
    b.insert(&format!("\n{indent}END\n"));
    b.goto(b.marker(rbeg_m));
    if !b.bolp() {
        b.insert("\n");
    }
    let state = match st.opt("org-inlinetask-default-state") {
        Some(Sexp::Str(s)) if !no_state && s != "nil" => format!("{s} "),
        _ => String::new(),
    };
    let wrap = rend != rbeg;
    b.insert(&format!("{indent}{state}{}", if wrap { "\n" } else { "" }));
    if wrap {
        b.forward_line(-1);
        b.end_of_line();
    }
    b.store_emacs(ed);
    Ok(())
}

/// The task heading at or before `l` and its level.
fn task_level(ls: &[String], l: usize, st: &Settings) -> Option<usize> {
    (0..=l).rev().find_map(|i| task_stars(&ls[i], st))
}

/// org-inlinetask-promote (`change` -1) and org-inlinetask-demote (+1).
pub fn promote(ed: &mut Editor, change: i64) -> Result<(), String> {
    let st = super::settings(ed);
    let ls = lines(ed);
    let l = ed.cur.line;
    if !in_task_p(&ls, l, &st) {
        return Err("Not in an inline task".into());
    }
    let lvl = task_level(&ls, l, &st).ok_or("Not in an inline task")?;
    let odd = st.opt_bool("org-odd-levels-only", false);
    let next = super::structure::valid_level(odd, lvl, change);
    let diff = next as i64 - lvl as i64;
    if change < 0 && next < min_level(&st).unwrap_or(0) {
        return Err("Cannot promote an inline task at minimum level".into());
    }
    let beg = goto_beginning(&ls, l, &st).ok_or("Not in an inline task")?;
    let stars = "*".repeat(next);
    let restar = |t: &str| format!("{stars}{}", t.trim_start_matches('*'));
    let e = goto_end(&ls, beg, &st);
    let last = e.saturating_sub(1);
    let (cl, cb) = ed.cur.pos();
    super::set_line(ed, beg, &restar(&ls[beg]));
    if last != beg && last < ls.len() && task_stars(&ls[last], &st).is_some() {
        super::set_line(ed, last, &restar(&ls[last]));
    }
    let adapt = super::sexp::option("org-adapt-indentation").is_some_and(|v| v == Sexp::T);
    if adapt {
        let mut b = EBuf::load(ed);
        b.goto(b.pos_of_line(beg));
        super::structure::fixup_indentation(&mut b, diff);
        b.store(ed);
    }
    let shift = if cl == beg || cl == last {
        (cb as i64 + diff).max(0) as usize
    } else {
        cb
    };
    ed.set_cursor(cl, shift);
    Ok(())
}

/// The inline task commands.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-inlinetask-insert-task" => insert_task(ed, !arg.is_none()),
        "org-inlinetask-promote" => promote(ed, -1),
        "org-inlinetask-demote" => promote(ed, 1),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::org;
    use super::*;

    fn st() -> Settings {
        super::super::syntax::settings("".lines(), None)
    }

    fn v(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    const T: &str = "***************";

    #[test]
    fn predicates() {
        let st = st();
        let ls = v(&format!("* H\n{T} Task\nbody\n{T} END\ntext"));
        assert!(at_task_p(&ls[1], &st));
        assert!(!at_task_p(&ls[3], &st) && end_p(&ls[3], &st));
        assert!(in_task_p(&ls, 2, &st));
        assert!(!in_task_p(&ls, 4, &st));
        assert!(!in_task_p(&ls, 0, &st));
        assert_eq!(goto_beginning(&ls, 3, &st), Some(1));
        assert_eq!(goto_end(&ls, 1, &st), 4);
        assert_eq!(goto_end(&ls, 2, &st), 4);
        // A one-line task.
        let ls = v(&format!("{T} One\ntext"));
        assert_eq!(goto_end(&ls, 0, &st), 1);
    }

    #[test]
    fn insert_task_and_nesting() {
        let mut e = org("* H\nx", "j");
        e.set_cursor(1, 1);
        insert_task(&mut e, false).unwrap();
        assert_eq!(e.buf.text(), format!("* H\nx\n{T} \n{T} END"));
        assert_eq!(e.cur.line, 2);
        // Inside a task: error.
        let mut e = org(&format!("{T} Task\nbody\n{T} END"), "j");
        assert_eq!(
            insert_task(&mut e, false).unwrap_err(),
            "Cannot nest inline tasks"
        );
        // Wrapping the region.
        let mut e = org("a\nb", "");
        crate::org::run(&mut e, "org-mode", Prefix::None);
        e.org_region = Some((0, 1));
        insert_task(&mut e, true).unwrap();
        e.org_region = None;
        assert_eq!(e.buf.text(), format!("{T} \na\nb\n{T} END"));
        assert_eq!(e.cur.line, 0);
    }

    #[test]
    fn promote_demote() {
        let t16 = format!("{T}*");
        let mut e = org(&format!("{t16} Task\nbody\n{t16} END"), "j");
        promote(&mut e, -1).unwrap();
        assert_eq!(e.buf.text(), format!("{T} Task\nbody\n{T} END"));
        assert_eq!(
            promote(&mut e, -1).unwrap_err(),
            "Cannot promote an inline task at minimum level"
        );
        promote(&mut e, 1).unwrap();
        assert_eq!(e.buf.text(), format!("{t16} Task\nbody\n{t16} END"));
        let mut e = org("text", "");
        assert_eq!(promote(&mut e, 1).unwrap_err(), "Not in an inline task");
        // M-right on a task demotes it.
        let mut e = org(&format!("* H\n{T} Task\n{T} END"), "j<M-Right>");
        assert_eq!(e.buf.line(1), format!("{T}* Task"));
        assert_eq!(e.buf.line(2), format!("{T}* END"));
        e.set_cursor(0, 0);
    }

    #[test]
    fn tab_folds_the_body() {
        let text = format!("* H\n{T} Task\nbody\n{T} END\nafter");
        let mut e = org(&text, "j<Tab>");
        assert!(e.folds.hidden(2) && e.folds.hidden(3));
        assert!(!e.folds.hidden(4));
        e.handle_key(crate::key::Key::new(crate::key::KeyCode::Tab));
        assert!(!e.folds.hidden(2));
        // org-cycle-hook: CONTENTS hides task bodies.
        let e = org(&text, "<S-Tab><S-Tab>");
        assert!(e.folds.hidden(2));
    }
}
