//! Context dispatchers: S-arrows, C-S-arrows and C-c C-c (org.el).

use super::{Prefix, call, ctx, fold};
use crate::editor::Editor;

/// org-support-shift-select.
fn shift_select() -> String {
    super::sexp::option("org-support-shift-select").map_or("nil".into(), |v| v.sym().or(v.str()).unwrap_or("t").to_owned())
}

fn not_always(s: &str) -> bool {
    !matches!(s, "always" | "except-timestamps" | "everywhere")
}

/// What to do when no Org context applies: Fred's plain arrow motion.
fn fallback(ed: &mut Editor, code: crate::key::KeyCode) -> Result<(), String> {
    let ss = shift_select();
    if ss != "nil" {
        crate::vim::normal_key(ed, crate::key::Key::new(code));
        return Ok(());
    }
    Err("To use shift-selection with Org mode, customize `org-support-shift-select'".into())
}

fn shift_vertical(ed: &mut Editor, arg: Prefix, up: bool) -> Result<(), String> {
    let l = ed.cur.line;
    let ss = shift_select();
    let down_later = super::options::bool("org-edit-timestamp-down-means-later", false);
    if ss != "everywhere" && ctx::at_timestamp(ed) {
        let later = up != down_later;
        return call(ed, if later { "org-timestamp-up" } else { "org-timestamp-down" }, arg);
    }
    if not_always(&ss) && super::options::bool("org-priority-enable-commands", true) && ctx::at_heading(ed, l) {
        return call(ed, if up { "org-priority-up" } else { "org-priority-down" }, arg);
    }
    if ss == "nil" && ctx::at_item(ed, l) {
        return call(ed, if up { "org-previous-item" } else { "org-next-item" }, arg);
    }
    if clocktable_line(ed, l) {
        return call(ed, if up { "org-clocktable-shift-up" } else { "org-clocktable-shift-down" }, arg);
    }
    if not_always(&ss) && ctx::at_table(ed, l) {
        return call(ed, if up { "org-table-move-cell-up" } else { "org-table-move-cell-down" }, arg);
    }
    fallback(ed, if up { crate::key::KeyCode::Up } else { crate::key::KeyCode::Down })
}

fn shift_horizontal(ed: &mut Editor, arg: Prefix, right: bool) -> Result<(), String> {
    let l = ed.cur.line;
    let ss = shift_select();
    if ss != "everywhere" && ctx::at_timestamp(ed) {
        return call(ed, if right { "org-timestamp-up-day" } else { "org-timestamp-down-day" }, arg);
    }
    if not_always(&ss) && ctx::at_heading(ed, l) {
        let as_change = super::options::bool("org-treat-S-cursor-todo-selection-as-state-change", true);
        let saved = (super::options::get("org-inhibit-blocking"), super::options::get("org-log-done"));
        if !as_change {
            super::options::put("org-inhibit-blocking", toml::Value::Boolean(true));
        }
        let r = super::todo::shift_todo(ed, right, !as_change);
        if !as_change {
            match saved.0 {
                Some(v) => super::options::put("org-inhibit-blocking", v),
                None => super::options::put("org-inhibit-blocking", toml::Value::Boolean(false)),
            }
        }
        return r;
    }
    let on_bullet = ctx::at_item(ed, l) && {
        let t = ed.buf.line(l);
        let (ind, b) = ctx::item_bullet(&t).unwrap();
        ed.cur.byte >= ind && ed.cur.byte < ind + b.len()
    };
    if (ss != "nil" && not_always(&ss) && on_bullet) || (ss == "nil" && ctx::at_item(ed, l)) {
        return call(ed, "org-cycle-list-bullet", if right { Prefix::None } else { Prefix::Num(-1) }).or_else(|_| {
            call(ed, if right { "org-cycle-list-bullet-next" } else { "org-cycle-list-bullet-previous" }, Prefix::None)
        });
    }
    if not_always(&ss) && ctx::at_property(ed, l) {
        return call(ed, if right { "org-property-next-allowed-value" } else { "org-property-previous-allowed-value" }, arg);
    }
    if clocktable_line(ed, l) {
        return call(ed, if right { "org-clocktable-shift-right" } else { "org-clocktable-shift-left" }, arg);
    }
    if not_always(&ss) && ctx::at_table(ed, l) {
        return call(ed, if right { "org-table-move-cell-right" } else { "org-table-move-cell-left" }, arg);
    }
    fallback(ed, if right { crate::key::KeyCode::Right } else { crate::key::KeyCode::Left })
}

/// A `#+BEGIN: clocktable` line.
fn clocktable_line(ed: &Editor, l: usize) -> bool {
    let t = ed.buf.line(l);
    let t = t.trim_start().to_ascii_lowercase();
    t.starts_with("#+begin: clocktable") || t.starts_with("#+begin:clocktable")
}

/// A `#+BEGIN:` dynamic block around line `l`.
fn dynamic_block(ed: &Editor, l: usize) -> bool {
    for i in (0..=l).rev() {
        let t = ed.buf.line(i).trim_start().to_ascii_lowercase();
        if t.starts_with("#+begin:") {
            return true;
        }
        if t.starts_with("#+end:") && i != l || ctx::at_heading(ed, i) {
            return false;
        }
    }
    false
}

/// org-ctrl-c-ctrl-c.
pub fn ctrl_c_ctrl_c(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let l = ed.cur.line;
    if ed.org_finish.is_some() {
        super::effect(ed, |s| s.org_finish(false));
        return Ok(());
    }
    if let Some(r) = super::colview_active(ed) {
        return r;
    }
    if ed.org.as_ref().is_some_and(|o| !o.sparse_hits.is_empty()) {
        if let Some(o) = &mut ed.org {
            o.sparse_hits.clear();
        }
        ed.set_msg("Temporary highlights/overlays removed from current buffer");
        return Ok(());
    }
    let line = ed.buf.line(l);
    let t = line.trim_start();
    let lower = t.to_ascii_lowercase();
    if ctx::src_block(ed, l).is_some() || inline_src_at(ed) {
        if super::options::bool("org-babel-no-eval-on-ctrl-c-ctrl-c", false) {
            return Ok(());
        }
        return call(ed, "org-babel-execute-src-block", arg);
    }
    if t.is_empty() {
        return Err("`C-c C-c' can do nothing useful here".into());
    }
    if lower.starts_with("#+call:") || line.contains("call_") && inline_call_at(ed) {
        return call(ed, "org-babel-execute-src-block", arg);
    }
    if ctx::at_clock_log(ed, l) {
        return if ctx::at_timestamp(ed) { call(ed, "org-timestamp-up-day", Prefix::Num(0)) } else { call(ed, "org-clock-update-time-maybe", arg) };
    }
    if lower.starts_with("#+begin:") || (dynamic_block(ed, l) && lower.starts_with("#+begin:")) {
        return call(ed, "org-update-dblock", arg);
    }
    if ctx::footnote_at(ed, l, ed.cur.byte).is_some() || lower.starts_with("[fn:") {
        return call(ed, "org-footnote-action", arg);
    }
    if ctx::at_heading(ed, l) {
        if ctx::cookie_at(ed, l, ed.cur.byte).is_some() {
            return call(ed, "org-update-statistics-cookies", arg);
        }
        if ctx::at_timestamp(ed) {
            return call(ed, "org-timestamp-up-day", Prefix::Num(0));
        }
        let save = ed.cur;
        let r = call(ed, "org-set-tags-command", arg);
        if ed.mode == crate::editor::Mode::Normal && ed.cur.line == save.line {
            ed.cur = save;
            ed.clamp_cursor();
        }
        return r;
    }
    if ctx::at_item(ed, l) || super::structure::in_item(ed, l) && ctx::item_bullet(&line).is_some() {
        if ctx::cookie_at(ed, l, ed.cur.byte).is_some() {
            return call(ed, "org-update-statistics-cookies", arg);
        }
        return call(ed, "org-list-ctrl-c-ctrl-c", arg).or_else(|e| if e.contains("not yet ported") { call(ed, "org-toggle-checkbox", arg) } else { Err(e) });
    }
    if lower.starts_with("#+tblfm:") || ctx::at_table(ed, l) || ctx::at_table_el(ed, l) {
        return call(ed, "org-table-ctrl-c-ctrl-c", arg);
    }
    if t.starts_with("<<<") {
        return call(ed, "org-update-radio-target-regexp", arg);
    }
    if ctx::at_property(ed, l) || ctx::property_drawer(ed, super::props::entry(ed, l)).is_some_and(|(s, e)| l == s || l == e) {
        return call(ed, "org-property-action", arg);
    }
    if ctx::at_keyword(ed, l) {
        // org-mode-restart: settings are recomputed; refresh visibility.
        if let Some(o) = &mut ed.org {
            o.reset_settings();
        }
        ed.set_msg("Local setup has been refreshed");
        return Ok(());
    }
    if ctx::cookie_at(ed, l, ed.cur.byte).is_some() {
        return call(ed, "org-update-statistics-cookies", arg);
    }
    if ctx::at_timestamp(ed) {
        return call(ed, "org-timestamp-up-day", Prefix::Num(0));
    }
    let _ = fold::hidden(ed, l);
    Err("`C-c C-c' can do nothing useful here".into())
}

fn inline_src_at(ed: &Editor) -> bool {
    let line = ed.buf.line(ed.cur.line);
    let b = ed.cur.byte;
    let mut from = 0;
    while let Some(i) = line[from..].find("src_").map(|i| i + from) {
        let end = line[i..].find('}').map_or(line.len(), |e| i + e + 1);
        if b >= i && b < end {
            return true;
        }
        from = end.max(i + 4);
    }
    false
}

fn inline_call_at(ed: &Editor) -> bool {
    let line = ed.buf.line(ed.cur.line);
    let b = ed.cur.byte;
    let mut from = 0;
    while let Some(i) = line[from..].find("call_").map(|i| i + from) {
        let end = line[i..].find(')').map_or(line.len(), |e| i + e + 1);
        if b >= i && b < end {
            return true;
        }
        from = end.max(i + 5);
    }
    false
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-shiftup" => shift_vertical(ed, arg, true),
        "org-shiftdown" => shift_vertical(ed, arg, false),
        "org-shiftright" => shift_horizontal(ed, arg, true),
        "org-shiftleft" => shift_horizontal(ed, arg, false),
        "org-shiftcontrolright" | "org-shiftcontrolleft" => {
            let right = name.ends_with("right");
            let ss = shift_select();
            if not_always(&ss) && ctx::at_heading(ed, ed.cur.line) {
                super::todo::shift_set(ed, right)
            } else if ss != "nil" {
                crate::vim::normal_key(ed, crate::key::Key::ch(if right { 'w' } else { 'b' }));
                Ok(())
            } else {
                Err("To use shift-selection with Org mode, customize `org-support-shift-select'".into())
            }
        }
        "org-shiftcontrolup" | "org-shiftcontroldown" => {
            if ctx::at_clock_log(ed, ed.cur.line) && ctx::at_timestamp(ed) {
                call(ed, if name.ends_with("up") { "org-clock-timestamps-up" } else { "org-clock-timestamps-down" }, arg)
            } else {
                Err("Not at a clock log".into())
            }
        }
        "org-ctrl-c-ctrl-c" => ctrl_c_ctrl_c(ed, arg),
        "org-ctrl-c-ctrl-k" | "org-capture-kill" | "org-edit-src-abort" if ed.org_finish.is_some() => {
            super::effect(ed, |s| s.org_finish(true));
            Ok(())
        }
        _ => return None,
    })
}
