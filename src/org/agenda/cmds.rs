//! Agenda buffer commands (org-agenda-mode-map): motion, view changes,
//! filters, bulk marks and the commands acting on source entries.
//!
//! Remote commands run the Org command by name (or the module function)
//! in the source buffer through [`super::remote`]; their prompts move to
//! the agenda buffer, and the agenda lines of the entry are updated
//! afterwards (org-agenda-change-all-lines).

use super::collect::Src;
use super::views::{self, redo};
use super::{
    Face, Kind, Line, LineKind, Loc, Remote, Span, Spec, View, cur_line, item, opt, opt_bool,
    opt_int, opt_string, view, view_mut,
};
use crate::editor::Editor;
use crate::org::sexp::Sexp;
use crate::org::time::civil::{self, Tm};
use crate::org::time::{self, stamp};
use crate::org::{Prefix, call, effect};
use crate::session::Session;
use std::sync::{Arc, Mutex};

// ---- motion ----

fn goto_view_line(ed: &mut Editor, i: usize) {
    let Ok(v) = view(ed) else { return };
    let b = v.buffer_line(i);
    ed.set_cursor(b.min(ed.line_count().saturating_sub(1)), 0);
}

/// Lines of the buffer, as view lines.
fn visible(ed: &Editor) -> Vec<usize> {
    view(ed).map(|v| v.map.clone()).unwrap_or_default()
}

fn is_item(v: &View, i: usize) -> bool {
    v.lines[i]
        .item
        .as_ref()
        .is_some_and(|it| it.marker.is_some() || it.hd.is_some())
        && v.lines[i].kind == LineKind::Text
}

fn move_lines(ed: &mut Editor, n: i64) {
    let last = ed.line_count().saturating_sub(1) as i64;
    let l = (ed.cur.line as i64 + n).clamp(0, last) as usize;
    let col = ed.cur.byte;
    ed.set_cursor(l, 0);
    ed.cur.byte = col.min(ed.buf.line(l).len());
    ed.clamp_cursor();
}

/// org-agenda-do-context-action: follow mode shows the entry.
fn context_action(ed: &mut Editor) {
    let follow = view(ed).is_ok_and(|v| v.modes.follow);
    if follow && item(ed).is_ok() {
        show_entry(ed, false);
    }
    if opt_bool("org-agenda-show-outline-path", true)
        && let Ok(it) = item(ed)
        && let Some(hd) = it.hd.clone()
    {
        let _ = hd;
        if !it.breadcrumbs.is_empty() {
            ed.set_msg(it.breadcrumbs.trim_end_matches("->").to_owned());
        }
    }
}

fn next_item(ed: &mut Editor, n: i64) {
    let Ok(v) = view(ed) else { return };
    let vis = visible(ed);
    let mut b = ed.cur.line;
    for _ in 0..n.unsigned_abs() {
        let found = if n > 0 {
            (b + 1..vis.len()).find(|&x| is_item(v, vis[x]))
        } else {
            (0..b).rev().find(|&x| is_item(v, vis[x]))
        };
        match found {
            Some(x) => b = x,
            None => break,
        }
    }
    let col = ed.cur.byte;
    ed.set_cursor(b, 0);
    ed.cur.byte = col.min(ed.buf.line(b).len());
    ed.clamp_cursor();
}

fn date_line(ed: &mut Editor, n: i64) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let lines: Vec<usize> = (0..ed.line_count())
        .filter(|&l| {
            ed.buf
                .line(l)
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace())
        })
        .collect();
    let cur = ed.cur.line;
    let target = if n > 0 {
        lines.iter().filter(|&&l| l > cur).nth(n as usize - 1)
    } else {
        lines
            .iter()
            .rev()
            .filter(|&&l| l < cur)
            .nth((-n) as usize - 1)
    };
    match target {
        Some(&l) => {
            ed.set_cursor(l, 0);
            Ok(())
        }
        None if n > 0 => Err("No next date after this line in this buffer".into()),
        None => Err("No previous date before this line in this buffer".into()),
    }
}

fn forward_block(ed: &mut Editor, backward: bool) -> Result<(), String> {
    let v = view(ed)?;
    let vis = visible(ed);
    let b = ed.cur.line;
    let found = if backward {
        (0..b).rev().find(|&x| v.lines[vis[x]].structural)
    } else {
        (b + 1..vis.len()).find(|&x| v.lines[vis[x]].structural)
    };
    match found {
        Some(x) => ed.set_cursor(x, 0),
        None => {
            let to = if backward {
                0
            } else {
                vis.len().saturating_sub(1)
            };
            ed.set_cursor(to, 0);
            ed.set_msg(format!(
                "No {} block",
                if backward { "previous" } else { "further" }
            ));
        }
    }
    Ok(())
}

// ---- dates and spans ----

/// The `day` of the cursor line.
fn cursor_day(ed: &Editor) -> Option<i64> {
    let v = view(ed).ok()?;
    let i = cur_line(ed)?;
    v.lines[i]
        .day
        .or_else(|| v.lines[..i].iter().rev().find_map(|l| l.day))
}

/// The List spec of the cursor's block: (arg, start, span).
fn list_args(ed: &Editor) -> Option<(Option<i64>, Option<i64>, Option<Span>)> {
    let v = view(ed).ok()?;
    let i = cur_line(ed).unwrap_or(0);
    let b = v.lines.get(i).map_or(0, |l| l.block);
    let spec = v
        .blocks
        .get(b)
        .filter(|x| matches!(x.spec, Spec::List { .. }))
        .or_else(|| {
            v.blocks
                .iter()
                .find(|x| matches!(x.spec, Spec::List { .. }))
        })?;
    match &spec.spec {
        Spec::List {
            arg, start, span, ..
        } => Some((*arg, *start, *span)),
        _ => None,
    }
}

fn cur_span(ed: &Editor) -> Span {
    list_args(ed)
        .and_then(|(arg, _, span)| arg.filter(|&n| n > 0).map(Span::from_days).or(span))
        .unwrap_or_else(views::default_span)
}

/// The first day shown by the cursor's agenda block.
fn starting_day(ed: &Editor) -> Option<i64> {
    let v = view(ed).ok()?;
    v.lines.iter().find_map(|l| l.day.filter(|_| l.date_header))
}

/// org-agenda-later (negative `n`: org-agenda-earlier).
fn later(ed: &mut Editor, n: i64) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let span = cur_span(ed);
    let sd = list_args(ed)
        .and_then(|a| a.1)
        .or_else(|| cursor_day(ed))
        .or_else(|| starting_day(ed))
        .unwrap_or_else(time::today);
    let sd = list_args(ed)
        .and_then(|a| a.1)
        .or_else(|| starting_day(ed))
        .unwrap_or(sd);
    let t = Tm::from_absolute(sd);
    let new = match span {
        Span::Day => sd + n,
        Span::Week => sd + 7 * n,
        Span::Fortnight => sd + 14 * n,
        Span::Month => Tm::date(t.year, t.month + n, t.day).norm().absolute(),
        Span::Year => Tm::date(t.year + n, t.month, t.day).norm().absolute(),
        Span::Days(d) => sd + d * n,
    };
    let cnt = view(ed)
        .ok()
        .and_then(|v| cur_line(ed).and_then(|i| v.lines[i].day_cnt));
    views::redo_block(
        ed,
        move |s| {
            if let Spec::List {
                arg,
                start,
                span: sp,
                ..
            } = s
            {
                *start = Some(new);
                *sp = Some(span);
                *arg = None;
            }
        },
        cnt,
    )
}

/// org-agenda-change-time-span.
fn change_span(ed: &mut Editor, span: Span, n: Option<i64>) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    if n.is_none() && list_args(ed).and_then(|a| a.2) == Some(span) {
        return Err(format!("Viewing span is already \"{}\"", span.symbol()));
    }
    let sd = cursor_day(ed)
        .or_else(|| list_args(ed).and_then(|a| a.1))
        .or_else(|| starting_day(ed))
        .unwrap_or_else(time::today);
    let sd = views::starting_span(sd, span, n);
    views::redo_block(
        ed,
        move |s| {
            if let Spec::List {
                arg,
                start,
                span: sp,
                ..
            } = s
            {
                *start = Some(sd);
                *sp = Some(span);
                *arg = None;
            }
        },
        None,
    )?;
    ed.set_msg(format!("Switched to {} view", span.symbol()));
    Ok(())
}

fn goto_today(ed: &mut Editor) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let v = view(ed)?;
    if let Some(i) = v.lines.iter().position(|l| l.today && l.hidden.is_empty()) {
        goto_view_line(ed, i);
        return Ok(());
    }
    let span = cur_span(ed);
    let sd = views::starting_span(time::today(), span, None);
    views::redo_block(
        ed,
        move |s| {
            if let Spec::List {
                start, span: sp, ..
            } = s
            {
                *start = Some(sd);
                *sp = Some(span);
            }
        },
        None,
    )
}

fn goto_date(ed: &mut Editor) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let opts = time::ReadOpts {
        prompt: Some("Date".into()),
        ..time::ReadOpts::default()
    };
    time::read_date(ed, opts, move |ed, a| {
        let day = a.tm.absolute();
        let span = cur_span(ed);
        let r = views::redo_block(
            ed,
            move |s| {
                if let Spec::List {
                    start, span: sp, ..
                } = s
                {
                    *start = Some(day);
                    *sp = Some(span);
                }
            },
            None,
        );
        if let Err(e) = r {
            ed.set_err(e);
        }
        if let Some(v) = ed.org_view.as_deref_mut() {
            v.goto_day = Some(day);
        }
    });
    Ok(())
}

fn view_dispatch(ed: &mut Editor) {
    let entries: Vec<(String, String)> = [
        ("d", "day"),
        ("w", "week"),
        ("t", "fortnight"),
        ("m", "month"),
        ("y", "year"),
        (" ", "reset"),
        ("G", "time grid"),
        ("[", "inactive timestamps"),
        ("f", "follow"),
        ("l", "log"),
        ("L", "log all"),
        ("c", "clockcheck"),
        ("a", "archive trees"),
        ("A", "archive files"),
        ("R", "clock report"),
        ("D", "include diary"),
        ("E", "entry text"),
        ("!", "deadlines"),
    ]
    .iter()
    .map(|(k, d)| ((*k).to_owned(), (*d).to_owned()))
    .collect();
    crate::org::menu(ed, "View:", entries, |ed, k| {
        let r = match k.as_str() {
            " " => change_span(ed, views::default_span(), None),
            "d" => change_span(ed, Span::Day, None),
            "w" => change_span(ed, Span::Week, None),
            "t" => change_span(ed, Span::Fortnight, None),
            "m" => change_span(ed, Span::Month, None),
            "y" => change_span(ed, Span::Year, None),
            "l" => log_mode(ed, Prefix::None),
            "L" => log_mode(ed, Prefix::U(1)),
            "c" => log_clockcheck(ed),
            "f" | "F" => follow_mode(ed),
            "a" => archives_mode(ed, false),
            "A" => archives_mode(ed, true),
            "R" | "r" => clockreport(ed),
            "E" | "e" => entry_text_mode(ed, Prefix::None),
            "G" => toggle_opt(ed, "org-agenda-use-time-grid", true, "Time-grid"),
            "D" => toggle_opt(ed, "org-agenda-include-diary", false, "Diary inclusion"),
            "!" => toggle_opt(
                ed,
                "org-agenda-include-deadlines",
                true,
                "Deadlines inclusion",
            ),
            "[" => inactive(ed),
            _ => Ok(()),
        };
        if let Err(e) = r {
            ed.set_err(e);
        }
    });
}

fn inactive(ed: &mut Editor) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    view_mut(ed)?.modes.inactive = true;
    redo(ed, false);
    view_mut(ed)?.modes.inactive = false;
    ed.set_msg("Display now includes inactive timestamps as well");
    Ok(())
}

fn toggle_opt(ed: &mut Editor, name: &str, default: bool, what: &str) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let v = !opt_bool(name, default);
    crate::org::options::put(name, toml::Value::Boolean(v));
    redo(ed, false);
    ed.set_msg(format!("{what} turned {}", if v { "on" } else { "off" }));
    Ok(())
}

fn log_mode(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let v = view_mut(ed)?;
    v.modes.show_log = match arg {
        Prefix::U(2) => Some(Sexp::Sym("only".into())),
        Prefix::None => {
            if v.modes.show_log.is_some() {
                None
            } else {
                Some(Sexp::T)
            }
        }
        _ => Some(crate::org::sexp::read("(closed clock state)").unwrap()),
    };
    let on = v.modes.show_log.is_some();
    redo(ed, false);
    ed.set_msg(format!("Log mode is {}", if on { "on" } else { "off" }));
    Ok(())
}

fn log_clockcheck(ed: &mut Editor) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let v = view_mut(ed)?;
    v.modes.show_log = if v.modes.show_log.as_ref().and_then(Sexp::sym) == Some("clockcheck") {
        None
    } else {
        Some(Sexp::Sym("clockcheck".into()))
    };
    let on = v.modes.show_log.is_some();
    redo(ed, false);
    ed.set_msg(format!("Log mode is {}", if on { "on" } else { "off" }));
    Ok(())
}

fn archives_mode(ed: &mut Editor, files: bool) -> Result<(), String> {
    let v = view_mut(ed)?;
    v.modes.archives = match (files, v.modes.archives) {
        (true, Some("t")) => None,
        (true, _) => Some("t"),
        (false, Some(_)) => None,
        (false, None) => Some("trees"),
    };
    let a = v.modes.archives;
    redo(ed, false);
    let tag = opt_string("org-archive-tag", "ARCHIVE");
    ed.set_msg(match a {
        None => "No archives are included".to_owned(),
        Some("trees") => format!("Trees with :{tag}: tag are included"),
        _ => format!("Trees with :{tag}: tag and all active archive files are included"),
    });
    Ok(())
}

fn clockreport(ed: &mut Editor) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let v = view_mut(ed)?;
    v.modes.clockreport = !v.modes.clockreport;
    let on = v.modes.clockreport;
    redo(ed, false);
    ed.set_msg(format!(
        "Clocktable mode is {}",
        if on { "on" } else { "off" }
    ));
    Ok(())
}

fn follow_mode(ed: &mut Editor) -> Result<(), String> {
    let v = view_mut(ed)?;
    v.modes.follow = !v.modes.follow;
    let on = v.modes.follow;
    v.mode_name = views::mode_name(v);
    if !on {
        hide_shown(ed);
    } else {
        context_action(ed);
    }
    ed.set_msg(format!("Follow mode is {}", if on { "on" } else { "off" }));
    Ok(())
}

fn entry_text_mode(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let v = view_mut(ed)?;
    if !v.filters.tag.is_empty()
        || !v.filters.category.is_empty()
        || !v.filters.regexp.is_empty()
        || v.filters.top.is_some()
    {
        return Err("Can't show entry text in filtered views".into());
    }
    let n = match arg {
        Prefix::Num(n) => Some(n),
        _ => {
            if v.modes.entry_text.is_some() {
                None
            } else {
                Some(opt_int("org-agenda-entry-text-maxlines", 5))
            }
        }
    };
    v.modes.entry_text = n;
    redo(ed, false);
    ed.set_msg(match n {
        Some(n) => format!("Entry text mode is on (maximum number of lines is {n})"),
        None => "Entry text mode is off".to_owned(),
    });
    Ok(())
}

// ---- entry text and showing entries ----

/// org-agenda-get-some-entry-text from a source's lines.
pub fn some_entry_text(lines: &[String], h: usize, n: i64, indent: &str) -> String {
    let end = (h + 1..lines.len())
        .find(|&l| crate::org::syntax::level(&lines[l]).is_some())
        .unwrap_or(lines.len());
    let mut body: Vec<String> = vec![];
    let mut drawer = false;
    for l in &lines[(h + 1).min(end)..end] {
        let t = l.trim();
        if drawer {
            if t.eq_ignore_ascii_case(":END:") {
                drawer = false;
            }
            continue;
        }
        if t.len() > 2
            && t.starts_with(':')
            && t.ends_with(':')
            && !t[1..t.len() - 1].contains([' ', ':'])
            && !t.eq_ignore_ascii_case(":END:")
        {
            drawer = true;
            continue;
        }
        if time::is_planning_line(l) {
            continue;
        }
        let mut s = l.replace('\t', "        ");
        for re in opt("org-agenda-entry-text-exclude-regexps")
            .and_then(|v| v.list().map(<[Sexp]>::to_vec))
            .unwrap_or_default()
        {
            if let Some(r) = re
                .str()
                .and_then(|r| crate::org::re::compile(r, false).ok())
            {
                s = r.replace_all(&s, "").into_owned();
            }
        }
        body.push(s);
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    while body.first().is_some_and(|l| l.trim().is_empty()) {
        body.remove(0);
    }
    let ind = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    body.iter()
        .take(n.max(0) as usize)
        .map(|l| {
            let l = if l.trim().is_empty() { "" } else { &l[ind..] };
            format!("{indent}{l}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// org-agenda-entry-text-show for every item line of `v`.
pub fn entry_text_show(v: &mut View, srcs: &[Src], n: i64) {
    let leaders = opt_string("org-agenda-entry-text-leaders", "    > ");
    let mut out = Vec::with_capacity(v.lines.len());
    for l in std::mem::take(&mut v.lines) {
        let hd = l.item.as_ref().and_then(|it| it.hd.clone());
        let block = l.block;
        out.push(l);
        let Some(hd) = hd else { continue };
        let Some(src) = srcs.iter().find(|s| s.path == hd.path) else {
            continue;
        };
        let Some(h) = hd.find(&src.lines) else {
            continue;
        };
        let txt = some_entry_text(&src.lines, h, n, &leaders);
        if txt.trim().is_empty() {
            continue;
        }
        for t in txt.lines() {
            out.push(Line {
                text: t.to_owned(),
                kind: LineKind::EntryText,
                block,
                ..Line::default()
            });
        }
    }
    v.lines = out;
}

/// Remove the entry text shown by org-agenda-show (follow mode).
fn hide_shown(ed: &mut Editor) {
    let at = cur_line(ed);
    let Some(v) = ed.org_view.as_deref_mut() else {
        return;
    };
    let before = v.lines.len();
    let keep_entry_text = v.modes.entry_text.is_some();
    v.lines
        .retain(|l| l.kind != LineKind::EntryText || keep_entry_text && !l.shown);
    if v.lines.len() != before {
        let at = at.map(|i| i.min(v.lines.len().saturating_sub(1)));
        super::redisplay(ed);
        if let Some(i) = at {
            goto_view_line(ed, i);
        }
    }
}

/// org-agenda-show (and follow mode): Fred has one window, so the
/// entry's text is shown under its agenda line until the next show.
fn show_entry(ed: &mut Editor, full: bool) {
    let Ok(it) = item(ed) else { return };
    let Some(at) = it.hd.clone().or(it.marker.clone()) else {
        return;
    };
    let i = cur_line(ed).unwrap_or(0);
    effect(ed, move |s| {
        let Some((h, lines)) = views::find_line(s, &at) else {
            s.ed.set_err("Cannot find the entry in its file (it was changed or removed)");
            return;
        };
        let n = if full {
            i64::MAX
        } else {
            opt_int("org-agenda-entry-text-maxlines", 5).max(10)
        };
        let mut shown = vec![lines[h].clone()];
        let txt = some_entry_text(&lines, h, n, "  ");
        shown.extend(txt.lines().map(str::to_owned));
        let Some(v) = s.ed.org_view.as_deref_mut() else {
            return;
        };
        v.lines.retain(|l| !l.shown);
        let at = i.min(v.lines.len());
        let block = v.lines.get(at).map_or(0, |l| l.block);
        for (k, t) in shown.into_iter().enumerate() {
            v.lines.insert(
                at + 1 + k,
                Line {
                    text: format!("  │ {t}"),
                    kind: LineKind::EntryText,
                    face: Face::Dimmed,
                    shown: true,
                    block,
                    ..Line::default()
                },
            );
        }
        super::redisplay(&mut s.ed);
        goto_view_line(&mut s.ed, at);
    });
}

/// org-agenda-goto / org-agenda-switch-to: visit the source entry.
fn visit(ed: &mut Editor, hd_first: bool) -> Result<(), String> {
    let it = item(ed)?;
    let at = if hd_first {
        it.hd.clone().or(it.marker.clone())
    } else {
        it.marker.clone().or(it.hd.clone())
    }
    .ok_or("Command not allowed in this line")?;
    effect(ed, move |s| {
        let Some((l, _)) = views::find_line(s, &at) else {
            s.ed.set_err("Cannot find the entry in its file (it was changed or removed)");
            return;
        };
        if let Err(e) = s.org_visit(&at.path, l) {
            s.ed.set_err(e);
            return;
        }
        if s.ed.org.is_some() {
            let l = s.ed.cur.line;
            crate::org::fold::show_context_for(&mut s.ed, l, "agenda");
            // Point at the heading text (org-complex-heading-regexp group 4).
            if let Some(h) = crate::org::syntax::heading_at_or_before(&s.ed.buf, s.ed.cur.line)
                && h == s.ed.cur.line
            {
                let st = crate::org::settings(&s.ed);
                if let Some(hl) = crate::org::syntax::headline(&s.ed.buf.line(h), &st) {
                    s.ed.cur.byte = hl.title.start;
                }
            }
        }
    });
    Ok(())
}

// ---- remote editing ----

/// The entry markers of the cursor line: (org-marker, org-hd-marker).
fn markers(ed: &Editor) -> Result<(Loc, Loc), String> {
    let it = item(ed)?;
    if it.diary_link {
        return Err("Command not allowed in this line".into());
    }
    let m = it
        .marker
        .clone()
        .or(it.hd.clone())
        .ok_or("Command not allowed in this line")?;
    let hd = it.hd.clone().unwrap_or_else(|| m.clone());
    Ok((m, hd))
}

/// Run `f` at `at` in its source buffer, then update the agenda lines
/// of the entry (org-agenda-change-all-lines) when `update`.
pub fn remote_then(
    ed: &mut Editor,
    at: Loc,
    hd: Loc,
    f: impl FnOnce(&mut Editor) -> Result<(), String> + Send + 'static,
    then: impl FnOnce(&mut Session, &Remote, &str) + Send + 'static,
) {
    let name = view(ed).map(|v| v.name.clone()).unwrap_or_default();
    super::remote(ed, at, f, move |s, r| {
        let agenda = s.org_agenda_find(&name);
        if let Some(i) = agenda {
            s.org_with_buffer(i, |e| shift_markers(e, &r));
        }
        then(s, &r, &name);
        if let Some(i) = s.org_agenda_find(&name) {
            let hd2 = hd.clone();
            let r2 = r.clone();
            let src = s.org_text(&r.path).ok();
            s.org_with_buffer(i, |e| {
                if let Some(text) = src {
                    change_all_lines(e, &hd2, &r2, &text);
                }
            });
        }
    });
}

/// Adjust the markers of the changed file after a change of `r.delta` lines.
fn shift_markers(ed: &mut Editor, r: &Remote) {
    if r.delta == 0 {
        return;
    }
    let Some(v) = ed.org_view.as_deref_mut() else {
        return;
    };
    let old = r.old.line;
    let fix = |l: &mut Loc| {
        if l.path == r.path && l.line > old {
            l.line = (l.line as i64 + r.delta).max(0) as usize;
        }
    };
    for line in &mut v.lines {
        if let Some(it) = &mut line.item {
            if let Some(m) = &mut it.marker {
                fix(m);
            }
            if let Some(h) = &mut it.hd {
                fix(h);
            }
        }
    }
    for m in &mut v.marks {
        fix(m);
    }
}

/// org-agenda-change-all-lines: reformat every line of entry `hd` from
/// the source `text` (removing them when the entry is gone).
pub fn change_all_lines(ed: &mut Editor, hd: &Loc, r: &Remote, text: &str) {
    let src = Src::new(r.path.clone(), text, None);
    let Some(v) = ed.org_view.as_deref_mut() else {
        return;
    };
    // The entry's heading now: the heading the command left the cursor
    // under (its text may have changed), else the old text found again.
    let new_h = r
        .head
        .as_ref()
        .filter(|_| r.path == hd.path)
        .map(|(l, _)| *l)
        .or_else(|| hd.find(&src.lines));
    let new_loc = new_h.map(|h| src.loc(h));
    for line in &mut v.lines {
        let Some(it) = &mut line.item else { continue };
        let same = it
            .hd
            .as_ref()
            .is_some_and(|x| x.path == hd.path && (x == hd || x.line == hd.line));
        if !same {
            continue;
        }
        let Some(h) = new_h else { continue };
        let old_hd = it.hd.clone();
        let p = super::format::compile(if it.fmt.is_empty() { "agenda" } else { it.fmt });
        let inherited =
            it.tags.iter().any(|t| t.1) || opt_bool("org-agenda-use-tag-inheritance", true);
        let mut new = super::format::item(
            &p,
            super::format::In {
                extra: Some(it.extra.clone()),
                txt: src.head_text(h),
                level: Some(it.level.clone()),
                category: Some(it.category.clone()),
                tags: src.tags(h, inherited),
                dotime: it.dotime.clone(),
                effort: it.effort.clone(),
                ..super::format::In::default()
            },
        );
        new.heading = new.heading.min(new.line.len());
        it.line = new.line;
        it.heading = new.heading;
        it.txt = new.txt;
        it.tags = new.tags;
        it.head = src.head_text(h);
        it.todo_state = src.todo(h);
        it.priority = src.priority(&it.line);
        if let Some(n) = &new_loc {
            if it.marker.as_ref() == old_hd.as_ref() {
                it.marker = Some(n.clone());
            }
            it.hd = Some(n.clone());
        }
        let done = it.todo_state.as_ref().is_some_and(|t| src.st.is_done(t));
        if let (Some(u), Some(d)) = (it.undone_face, it.done_face) {
            it.face = if done { d } else { u };
        } else if done {
            it.face = Face::Done;
        }
        super::format::highlight_todo(it);
        line.text = super::format::align_tags(&it.line, v.width);
        line.face = it.face;
        if let Some(hb) = &it.habit {
            let (t, spans) = super::habit::insert_graph(&line.text, hb, time::today());
            line.text = t;
            line.spans = spans;
        }
    }
    if new_h.is_none() {
        v.lines.retain(|l| {
            !l.item
                .as_ref()
                .and_then(|it| it.hd.as_ref())
                .is_some_and(|x| x.path == hd.path && x.line == hd.line)
        });
    }
    super::redisplay(ed);
}

/// Remove the agenda lines of a subtree (org-remove-subtree-entries-from-agenda).
fn remove_subtree_lines(ed: &mut Editor, path: &std::path::Path, beg: usize, end: usize) {
    let Some(v) = ed.org_view.as_deref_mut() else {
        return;
    };
    v.lines.retain(|l| {
        !l.item
            .as_ref()
            .and_then(|it| it.marker.as_ref())
            .is_some_and(|m| m.path == path && m.line >= beg && m.line < end)
    });
    super::redisplay(ed);
}

fn todo(ed: &mut Editor, arg: Prefix, how: &'static str) -> Result<(), String> {
    let (m, hd) = markers(ed)?;
    let col = ed.cur.byte;
    remote_then(
        ed,
        m,
        hd,
        move |src| match how {
            "nextset" => crate::org::todo::shift_set(src, true),
            "previousset" => crate::org::todo::shift_set(src, false),
            s if !s.is_empty() => crate::org::todo::todo_to(src, Some(s.to_owned())),
            _ => call(src, "org-todo", arg),
        },
        move |s, _, _| s.ed.cur.byte = col,
    );
    Ok(())
}

/// A remote command by name at the headline, updating the lines.
fn at_heading(ed: &mut Editor, name: &'static str, arg: Prefix) -> Result<(), String> {
    let (_, hd) = markers(ed)?;
    remote_then(
        ed,
        hd.clone(),
        hd,
        move |src| call(src, name, arg),
        |_, _, _| {},
    );
    Ok(())
}

/// A remote command by name at the marker (no line update).
fn at_marker(ed: &mut Editor, name: &'static str, arg: Prefix) -> Result<(), String> {
    let (m, _) = markers(ed)?;
    super::remote(ed, m, move |src| call(src, name, arg), |_, _| {});
    Ok(())
}

fn set_tags(ed: &mut Editor, tag: Option<(String, bool)>) -> Result<(), String> {
    let (_, hd) = markers(ed)?;
    remote_then(
        ed,
        hd.clone(),
        hd,
        move |src| match tag {
            Some((t, on)) => {
                let h = src.cur.line;
                crate::org::tags::toggle_tag(src, h, &t, Some(on));
                Ok(())
            }
            None => call(src, "org-set-tags-command", Prefix::None),
        },
        |_, _, _| {},
    );
    Ok(())
}

/// org-agenda-show-new-time: draw " S => <ts>" on the item's lines.
fn show_new_time(s: &mut Session, name: &str, marker: &Loc, stamp: &str, prefix: &str) {
    let Some(i) = s.org_agenda_find(name) else {
        return;
    };
    let note = format!("{prefix} => {stamp} ");
    let m = marker.clone();
    s.org_with_buffer(i, |e| {
        let at = cur_line(e);
        if let Some(v) = e.org_view.as_deref_mut() {
            for l in &mut v.lines {
                if l.item
                    .as_ref()
                    .and_then(|it| it.marker.as_ref())
                    .is_some_and(|x| x.path == m.path && (x.line == m.line || x.text == m.text))
                {
                    l.new_time = Some(note.clone());
                }
            }
        }
        super::redisplay(e);
        if let Some(a) = at {
            goto_view_line(e, a);
        }
    });
}

/// org-agenda-schedule / org-agenda-deadline.
fn schedule(
    ed: &mut Editor,
    deadline: bool,
    arg: Prefix,
    time: Option<String>,
) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda, Kind::Todo, Kind::Tags, Kind::Search])?;
    let (m, hd) = markers(ed)?;
    let name = view(ed)?.name.clone();
    let mm = m.clone();
    super::remote(
        ed,
        m,
        move |src| time::schedule(src, deadline, arg, time),
        move |s, r| {
            let ts = s
                .org_text(&r.path)
                .ok()
                .and_then(|t| {
                    let lines: Vec<String> = t.lines().map(str::to_owned).collect();
                    let h = r.head.as_ref()?.0;
                    let kind = if deadline {
                        time::Planning::Deadline
                    } else {
                        time::Planning::Scheduled
                    };
                    let p = lines.get(h + 1)?;
                    let c = kind.time_re().captures(p)?;
                    let g = c.get(1)?;
                    Some(p[g.start() - 1..g.end() + 1].to_owned())
                })
                .unwrap_or_default();
            show_new_time(s, &name, &mm, &ts, if deadline { " D" } else { " S" });
            if !ts.is_empty() {
                s.ed.set_msg(ts);
            }
            let _ = hd;
        },
    );
    Ok(())
}

/// org-agenda-date-later (unit `what`; negative `n` for earlier).
fn date_later(ed: &mut Editor, n: i64, unit: civil::Unit) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let (m, _) = markers(ed)?;
    let name = view(ed)?.name.clone();
    let out: Arc<Mutex<String>> = Arc::default();
    let o2 = out.clone();
    let mm = m.clone();
    super::remote(
        ed,
        m,
        move |src| {
            let l = src.cur.line;
            let text = src.buf.line(l);
            let col = src.cur.byte.min(text.len());
            let start = stamp::ts3()
                .find_iter(&text)
                .find(|x| x.start() <= col && col < x.end())
                .or_else(|| stamp::ts3().find_iter(&text).find(|x| x.start() >= col))
                .map(|x| x.start())
                .ok_or("Cannot find time stamp")?;
            let mut n = n;
            let range = stamp::tr_both()
                .find_iter(&text)
                .find(|x| x.start() <= start && start < x.end());
            if opt_bool("org-agenda-move-date-from-past-immediately-to-today", true)
                && n == 1
                && unit == civil::Unit::Day
                && range.is_none()
                && let Some(p) = stamp::parse_time_string(&text[start..])
            {
                let cdate = Tm::date(p.year, p.month, p.day).absolute();
                let today = time::today();
                if today > cdate {
                    n = today - cdate;
                }
            }
            let mut new = time::change_at(src, l, start, n, unit, false)?;
            if let Some(r) = range {
                let text = src.buf.line(l);
                if let Some(second) = stamp::ts3()
                    .find_iter(&text)
                    .map(|x| x.start())
                    .find(|&x| x > start && x < r.end() + 40)
                {
                    let end = time::change_at(src, l, second, n, unit, false)?;
                    new = format!("{new}--{end}");
                }
            }
            *o2.lock().unwrap() = new;
            Ok(())
        },
        move |s, _| {
            let ts = out.lock().unwrap().clone();
            if !ts.is_empty() {
                show_new_time(s, &name, &mm, &ts, "");
                s.ed.set_msg(format!("Time stamp changed to {ts}"));
            }
        },
    );
    Ok(())
}

fn do_date(ed: &mut Editor, arg: Prefix, later: bool) -> Result<(), String> {
    let last = ed
        .org_view
        .as_ref()
        .and_then(|v| v.last_command.clone())
        .unwrap_or_default();
    let sign = if later { 1 } else { -1 };
    let (n, unit, cmd) = if arg == Prefix::U(2) || last.ends_with("-minutes") {
        (
            stamp::rounding_minutes().1.max(1),
            civil::Unit::Minute,
            "org-agenda-date-later-minutes",
        )
    } else if arg == Prefix::U(1) || last.ends_with("-hours") {
        (1, civil::Unit::Hour, "org-agenda-date-later-hours")
    } else {
        (arg.value(), civil::Unit::Day, "")
    };
    if !cmd.is_empty()
        && let Some(v) = ed.org_view.as_deref_mut()
    {
        v.this_command = Some(cmd.to_owned());
    }
    date_later(ed, sign * n, unit)
}

fn date_prompt(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let (m, _) = markers(ed)?;
    let name = view(ed)?.name.clone();
    let mm = m.clone();
    super::remote(
        ed,
        m,
        move |src| {
            let l = src.cur.line;
            let text = src.buf.line(l);
            let col = src.cur.byte;
            let x = stamp::ts3()
                .find_iter(&text)
                .find(|x| x.end() > col)
                .ok_or("Cannot find time stamp")?;
            src.cur.byte = x.start() + 1;
            let inactive = text[x.start()..].starts_with('[');
            call(
                src,
                if inactive {
                    "org-timestamp-inactive"
                } else {
                    "org-timestamp"
                },
                arg,
            )
        },
        move |s, r| {
            let ts = s
                .org_text(&r.path)
                .ok()
                .and_then(|t| {
                    let line = t.lines().nth(r.old.line)?.to_owned();
                    stamp::ts3().find(&line).map(|m| m.as_str().to_owned())
                })
                .unwrap_or_default();
            show_new_time(s, &name, &mm, &ts, "");
            s.ed.set_msg(format!("Time stamp changed to {ts}"));
        },
    );
    Ok(())
}

fn kill(ed: &mut Editor) -> Result<(), String> {
    let it = item(ed)?;
    let (m, _) = markers(ed)?;
    let sexp = it.kind == "sexp";
    let name = view(ed)?.name.clone();
    let confirm = opt("org-agenda-confirm-kill").unwrap_or(Sexp::Int(1));
    effect(ed, move |s| {
        let Some((l, lines)) = views::find_line(s, &m) else {
            s.ed.set_err("Cannot find the entry in its file (it was changed or removed)");
            return;
        };
        let st = crate::org::settings_of(&lines.join("\n"), m.path.parent());
        let (beg, end) = match crate::org::syntax::heading_at_or_before(&lines, l) {
            Some(h) if !sexp && crate::org::syntax::headline(&lines[h], &st).is_some() => {
                (h, crate::org::syntax::subtree_end(&lines, h))
            }
            _ => (l, l + 1),
        };
        let n = lines[beg..end]
            .iter()
            .filter(|x| !x.trim().is_empty())
            .count() as i64;
        let ask = match &confirm {
            Sexp::T => true,
            Sexp::Int(k) => n > *k,
            _ => false,
        };
        let path = m.path.clone();
        let doit = move |s: &mut Session| {
            let Ok(i) = s.org_buffer(&path) else { return };
            s.org_with_buffer(i, |src| {
                let pos = src.cur.pos();
                src.undo.begin(pos);
                crate::org::delete_lines(src, beg, end - beg);
                let pos = src.cur.pos();
                src.undo.end(pos);
                src.clamp_cursor();
            });
            if let Some(a) = s.org_agenda_find(&name) {
                s.org_with_buffer(a, |e| {
                    remove_subtree_lines(e, &path, beg, end);
                    let r = Remote {
                        path: path.clone(),
                        old: Loc {
                            path: path.clone(),
                            line: beg,
                            text: String::new(),
                            col: 0,
                        },
                        head: None,
                        delta: -((end - beg) as i64),
                        ok: true,
                    };
                    shift_markers(e, &r);
                    if let Some(v) = e.org_view.as_deref_mut() {
                        v.undo.push(path.clone());
                    }
                    e.set_msg("Agenda item and source killed");
                });
            }
        };
        if ask {
            let buf = path_name(&m.path);
            crate::org::yes_or_no(
                &mut s.ed,
                &format!("Delete entry with {n} lines in buffer \"{buf}\"? "),
                move |ed| effect(ed, doit),
            );
        } else {
            doit(s);
        }
    });
    Ok(())
}

fn path_name(p: &std::path::Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// org-agenda-archive-with.
fn archive_with(ed: &mut Editor, cmd: &'static str, confirm: bool) -> Result<(), String> {
    let (m, _) = markers(ed)?;
    let name = view(ed)?.name.clone();
    let run = move |ed: &mut Editor| {
        let path = m.path.clone();
        let n2 = name.clone();
        super::remote(
            ed,
            m,
            move |src| {
                let h = crate::org::syntax::heading_at_or_before(&src.buf, src.cur.line)
                    .ok_or("Before first headline")?;
                src.set_cursor(h, 0);
                call(src, cmd, Prefix::None)
            },
            move |s, r| {
                if let Some(a) = s.org_agenda_find(&n2) {
                    let end = r.old.line + (-r.delta).max(1) as usize;
                    s.org_with_buffer(a, |e| {
                        remove_subtree_lines(e, &path, r.old.line, end);
                        shift_markers(e, &r);
                    });
                }
            },
        );
    };
    if confirm {
        crate::org::yes_or_no(ed, "Archive this subtree or entry? ", run);
    } else {
        run(ed);
    }
    Ok(())
}

fn archive_default_cmd() -> &'static str {
    match opt("org-archive-default-command")
        .and_then(|v| v.str().map(str::to_owned))
        .as_deref()
    {
        Some("org-archive-to-archive-sibling") => "org-archive-to-archive-sibling",
        Some("org-toggle-archive-tag") => "org-toggle-archive-tag",
        _ => "org-archive-subtree",
    }
}

fn refile(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    match arg {
        Prefix::Num(0) | Prefix::U(3) => return call(ed, "org-refile-cache-clear", Prefix::None),
        Prefix::U(2) => return call(ed, "org-refile-goto-last-stored", Prefix::None),
        _ => {}
    }
    let (_, hd) = markers(ed)?;
    let name = view(ed)?.name.clone();
    super::remote(
        ed,
        hd,
        move |src| call(src, "org-refile", arg),
        move |s, _| {
            if let Some(i) = s.org_agenda_find(&name) {
                s.org_with_buffer(i, |e| redo(e, false));
                let eff = s.org_with_buffer(i, |e| e.pending_effect.take());
                if let Some(e) = eff {
                    s.perform(e);
                }
            }
        },
    );
    Ok(())
}

fn clock_in(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if arg == Prefix::U(1) {
        return call(ed, "org-clock-in", arg);
    }
    let (m, hd) = markers(ed)?;
    remote_then(
        ed,
        m,
        hd,
        move |src| call(src, "org-clock-in", arg),
        |_, _, _| {},
    );
    Ok(())
}

/// Commands of the clock module that act on the running clock.
fn clock_cmd(ed: &mut Editor, name: &str) -> Result<(), String> {
    call(ed, name, Prefix::None)?;
    if name != "org-clock-goto" {
        redo(ed, false);
    }
    Ok(())
}

fn clock_goto(ed: &mut Editor) -> Result<(), String> {
    let v = view(ed)?;
    let vis = visible(ed);
    let cur = ed.cur.line;
    let best = vis
        .iter()
        .enumerate()
        .filter(|(_, i)| v.lines[**i].item.as_ref().is_some_and(|it| it.clocking))
        .min_by_key(|(b, _)| (*b as i64 - cur as i64).abs())
        .map(|(b, _)| b);
    match best {
        Some(b) => {
            let col = ed.cur.byte;
            ed.set_cursor(b, col);
            ed.clamp_cursor();
            Ok(())
        }
        None => match call(ed, "org-clock-goto", Prefix::None) {
            Ok(()) => Ok(()),
            Err(_) => {
                ed.set_msg("No running clock, use `C-c C-x C-j' to jump to the most recent one");
                Ok(())
            }
        },
    }
}

fn open_link(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let it = item(ed)?;
    let at = it.hd.clone().or(it.marker.clone());
    let line = ed.buf.line(ed.cur.line);
    let link_re = re!(r"\[\[[^\]]+\](?:\[[^\]]*\])?\]");
    let Some(at) = at else {
        if let Some(m) = link_re.find(&line) {
            return crate::org::call_with(ed, "org-link-open-from-string", arg, m.as_str().into());
        }
        ed.set_msg("No link to open here");
        return Ok(());
    };
    super::remote(
        ed,
        at,
        move |src| {
            let h = src.cur.line;
            let end = crate::org::syntax::entry_end(&src.buf, h);
            for l in h..end {
                let t = src.buf.line(l);
                if let Some(m) = re!(
                    r"\[\[[^\]]+\](?:\[[^\]]*\])?\]|\b(https?|file|mailto|id|doi):[^ \t\n\]>)]+"
                )
                .find(&t)
                {
                    src.set_cursor(l, m.start() + 1);
                    return call(src, "org-open-at-point", arg);
                }
            }
            Err("No link to open here".into())
        },
        |_, _| {},
    );
    Ok(())
}

// ---- filters ----

fn refilter(ed: &mut Editor) {
    let at = cur_line(ed);
    if let Some(v) = ed.org_view.as_deref_mut() {
        if v.modes.entry_text.is_some() {
            v.modes.entry_text = None;
            v.lines.retain(|l| l.kind != LineKind::EntryText);
        }
        super::filter::apply(v);
        v.mode_name = views::mode_name(v);
    }
    super::redisplay(ed);
    // Stay on a visible line at or before the old one.
    if let (Some(i), Ok(v)) = (at, view(ed)) {
        let b = v.buffer_line(i);
        ed.set_cursor(b.min(ed.line_count().saturating_sub(1)), 0);
    }
}

fn filter_category(ed: &mut Editor, strip: bool) -> Result<(), String> {
    let v = view_mut(ed)?;
    if v.filtered_by_category && !v.filters.category.is_empty() {
        v.filters.category.clear();
        v.filtered_by_category = false;
        refilter(ed);
        return Ok(());
    }
    let cat = item(ed)
        .map(|i| i.category)
        .map_err(|_| "No category at point".to_owned())?;
    let v = view_mut(ed)?;
    if strip {
        v.filters.category.insert(0, format!("-{cat}"));
        v.filtered_by_category = false;
    } else {
        v.filters.category = vec![format!("+{cat}")];
        v.filtered_by_category = true;
    }
    refilter(ed);
    Ok(())
}

fn filter_top(ed: &mut Editor, strip: bool) -> Result<(), String> {
    let v = view_mut(ed)?;
    if v.filters.top.is_some() {
        v.filters.top = None;
        refilter(ed);
        return Ok(());
    }
    let top = item(ed)
        .ok()
        .map(|i| i.top)
        .filter(|t| !t.is_empty())
        .ok_or("No top-level headline at point")?;
    view_mut(ed)?.filters.top = Some((top, strip));
    refilter(ed);
    Ok(())
}

fn filter_regexp(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let strip = arg == Prefix::U(1);
    let accumulate = arg == Prefix::U(2);
    let v = view_mut(ed)?;
    if !v.filters.regexp.is_empty() && !accumulate {
        v.filters.regexp.clear();
        refilter(ed);
        ed.set_msg("Regexp filter removed");
        return Ok(());
    }
    let prompt = if strip {
        "Hide entries matching regexp: "
    } else {
        "Narrow to entries matching regexp: "
    };
    crate::org::read(ed, prompt, "", move |ed, r| {
        if let Some(v) = ed.org_view.as_deref_mut() {
            v.filters
                .regexp
                .insert(0, format!("{}{r}", if strip { "-" } else { "+" }));
        }
        refilter(ed);
    });
    Ok(())
}

fn efforts() -> Vec<String> {
    let global = opt("org-global-properties").unwrap_or(Sexp::Nil);
    let prop = format!("{}_ALL", opt_string("org-effort-property", "Effort"));
    global
        .list()
        .unwrap_or(&[])
        .iter()
        .find(|e| {
            e.car()
                .and_then(Sexp::str)
                .is_some_and(|k| k.eq_ignore_ascii_case(&prop))
        })
        .and_then(|e| e.cdr().str().map(str::to_owned))
        .unwrap_or_else(|| "0 0:10 0:30 1:00 2:00 3:00 4:00 5:00 6:00 7:00".into())
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn filter_effort(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let keep = arg == Prefix::U(2);
    let negative = arg == Prefix::U(1);
    view(ed)?;
    let ops: Vec<(String, String)> = [
        ("<", "less than or equal"),
        (">", "greater than or equal"),
        ("=", "equal"),
        ("_", "remove filter"),
    ]
    .iter()
    .map(|(k, d)| ((*k).to_owned(), (*d).to_owned()))
    .collect();
    crate::org::menu(ed, "Effort operator? (> = or <)", ops, move |ed, op| {
        if op == "_" {
            if let Some(v) = ed.org_view.as_deref_mut() {
                v.filters.effort.clear();
            }
            refilter(ed);
            ed.set_msg("Effort filter removed");
            return;
        }
        let es = efforts();
        let entries: Vec<(String, String)> = es
            .iter()
            .enumerate()
            .take(10)
            .map(|(i, e)| (((i + 1) % 10).to_string(), e.clone()))
            .collect();
        crate::org::menu(ed, &format!("Effort {op}"), entries, move |ed, k| {
            let i: usize = k.parse().unwrap_or(1);
            let e = es[(i + 9) % 10].clone();
            if let Some(v) = ed.org_view.as_deref_mut() {
                let f = format!("{}{op}{e}", if negative { "-" } else { "+" });
                if keep {
                    v.filters.effort.insert(0, f);
                } else {
                    v.filters.effort = vec![f];
                }
            }
            refilter(ed);
        });
    });
    Ok(())
}

/// org-agenda-filter: the general filter string.
fn filter_general(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let v = view(ed)?;
    if arg == Prefix::U(3) {
        return Err("`org-agenda-auto-exclude-function' is undefined".into());
    }
    let negate = arg == Prefix::U(1);
    let tag_list = super::filter::represented_tags(v);
    let cat_list = super::filter::represented_categories(v);
    let ff = format!(
        "{}{}{}{}",
        v.filters.category.concat(),
        v.filters.tag.concat(),
        v.filters
            .effort
            .first()
            .map(|e| e.trim_start_matches('+').to_owned())
            .unwrap_or_default(),
        v.filters
            .regexp
            .first()
            .map(|r| format!("/{}/", r.trim_start_matches('+')))
            .unwrap_or_default()
    );
    let prompt = format!(
        "{} [+cat-tag<0:10-/regexp/]: ",
        if negate { "Negative filter" } else { "Filter" }
    );
    let accumulate = arg == Prefix::U(2);
    crate::org::read(ed, &prompt, &ff, move |ed, s| {
        let mut f = s.clone();
        let keep = if re!(r"^\+[+-]").is_match(&f) {
            f.remove(0);
            true
        } else {
            accumulate
        };
        let Some(v) = ed.org_view.as_deref_mut() else {
            return;
        };
        let (mut fc, mut ft, mut fe, mut fr) = if keep {
            (
                v.filters.category.clone(),
                v.filters.tag.clone(),
                v.filters.effort.clone(),
                v.filters.regexp.clone(),
            )
        } else {
            (vec![], vec![], vec![], vec![])
        };
        let f = re!(r#""([^"]*)-([^"]*)""#)
            .replace_all(&f, "\"$1~~~$2\"")
            .into_owned();
        let re = re!(r"^[ \t]*([-+])?(([^-+<>=/ \t]+)|([<>=][0-9:]+)|(/([^/]+)/?))");
        let mut rest = f.as_str();
        let mut msgs = vec![];
        let push = |v: &mut Vec<String>, x: String| {
            if !v.contains(&x) {
                v.push(x);
            }
        };
        while let Some(c) = re.captures(rest) {
            let mut pm = c.get(1).map_or("+", |m| m.as_str()).to_owned();
            if negate {
                pm = if pm == "+" { "-".into() } else { "+".into() };
            }
            if let Some(m) = c.get(3) {
                let s = m.as_str().replace("~~~", "-");
                if tag_list.contains(&s) {
                    push(&mut ft, format!("{pm}{s}"));
                } else if cat_list.contains(&s) {
                    push(&mut fc, format!("{pm}{}", s.trim_matches('"')));
                } else {
                    msgs.push(format!(
                        "`{pm}{s}' filter ignored because tag/category is not represented"
                    ));
                }
            } else if let Some(m) = c.get(4) {
                push(&mut fe, format!("{pm}{}", m.as_str()));
            } else if let Some(m) = c.get(6) {
                push(&mut fr, format!("{pm}{}", m.as_str()));
            }
            rest = &rest[c.get(0).unwrap().end()..];
        }
        v.filters.category = fc;
        v.filters.tag = ft;
        v.filters.effort = fe;
        v.filters.regexp = fr;
        v.filters.top = None;
        refilter(ed);
        if let Some(m) = msgs.pop() {
            ed.set_msg(m);
        }
    });
    Ok(())
}

fn filter_remove_all(ed: &mut Editor) -> Result<(), String> {
    let v = view_mut(ed)?;
    v.filters = super::Filters::default();
    v.filtered_by_category = false;
    refilter(ed);
    ed.set_msg("All agenda filters removed");
    Ok(())
}

fn filter_tag(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let v = view(ed)?;
    let exclude = arg == Prefix::U(1);
    let accumulate = arg == Prefix::U(2);
    let expand = arg != Prefix::U(3);
    let mut seen = vec![];
    let mut entries: Vec<(String, String)> = vec![];
    for (t, c) in &v.tag_alist {
        if let Some(c) = c
            && !seen.contains(c)
        {
            seen.push(*c);
            entries.push((c.to_string(), t.clone()));
        }
    }
    entries.push(("TAB".into(), "tag (completion)".into()));
    entries.push(("\\".into(), "off".into()));
    entries.push((".".into(), "tags of this line".into()));
    entries.push((" ".into(), "any tag".into()));
    entries.push(("-".into(), "exclude".into()));
    entries.push(("+".into(), "filter".into()));
    let title = format!(
        "{} by tag{}:",
        if exclude { "Exclude[+]" } else { "Filter[-]" },
        if expand { "" } else { " (no grouptag expand)" }
    );
    let alist = v.tag_alist.clone();
    let line_tags = item(ed).map(|i| i.tag_names()).unwrap_or_default();
    let all_tags = super::filter::represented_tags(v);
    crate::org::menu(ed, &title, entries, move |ed, k| {
        let apply = |ed: &mut Editor, tag: String, exclude: bool| {
            if let Some(v) = ed.org_view.as_deref_mut() {
                let f = format!("{}{tag}", if exclude { "-" } else { "+" });
                if accumulate {
                    v.filters.tag.insert(0, f);
                } else {
                    v.filters.tag = vec![f];
                }
            }
            refilter(ed);
        };
        match k.as_str() {
            "-" => {
                let _ = filter_tag(ed, Prefix::U(1));
            }
            "+" => {
                let _ = filter_tag(ed, Prefix::None);
            }
            "\\" => {
                if let Some(v) = ed.org_view.as_deref_mut() {
                    v.filters.tag.clear();
                }
                refilter(ed);
            }
            "." => {
                if let Some(v) = ed.org_view.as_deref_mut() {
                    v.filters.tag = line_tags.iter().map(|t| format!("+{t}")).collect();
                }
                refilter(ed);
            }
            " " => apply(ed, String::new(), exclude),
            "TAB" => {
                let mut cands: Vec<String> = alist.iter().map(|(t, _)| t.clone()).collect();
                for t in &all_tags {
                    if !cands.contains(t) {
                        cands.push(t.clone());
                    }
                }
                crate::org::complete(ed, "Tag: ", cands, true, move |ed, t| {
                    if let Some(v) = ed.org_view.as_deref_mut() {
                        let f = format!("{}{t}", if exclude { "-" } else { "+" });
                        if accumulate {
                            v.filters.tag.insert(0, f);
                        } else {
                            v.filters.tag = vec![f];
                        }
                    }
                    refilter(ed);
                });
            }
            c => {
                let ch = c.chars().next();
                if let Some((t, _)) = alist.iter().find(|(_, x)| *x == ch) {
                    apply(ed, t.clone(), exclude);
                }
            }
        }
    });
    Ok(())
}

fn limit_interactively(ed: &mut Editor, remove: bool) -> Result<(), String> {
    view(ed)?;
    if remove {
        for o in [
            "org-agenda-max-entries",
            "org-agenda-max-effort",
            "org-agenda-max-todos",
            "org-agenda-max-tags",
        ] {
            crate::org::options::remove(o);
        }
        redo(ed, false);
        return Ok(());
    }
    let entries: Vec<(String, String)> = [
        ("e", "max number of entries"),
        ("t", "max number of TODOs"),
        ("T", "max number of tagged entries"),
        ("E", "max effort"),
    ]
    .iter()
    .map(|(k, d)| ((*k).to_owned(), (*d).to_owned()))
    .collect();
    crate::org::menu(
        ed,
        "Number of [e]ntries [t]odos [T]ags [E]ffort?",
        entries,
        |ed, k| {
            let (opt_name, prompt) = match k.as_str() {
                "e" => ("org-agenda-max-entries", "How many entries? "),
                "t" => ("org-agenda-max-todos", "How many TODO entries? "),
                "T" => ("org-agenda-max-tags", "How many tagged entries? "),
                _ => ("org-agenda-max-effort", "How many minutes? "),
            };
            crate::org::read(ed, prompt, "", move |ed, n| {
                if let Ok(n) = n.trim().parse::<i64>() {
                    crate::org::options::put(opt_name, toml::Value::Integer(n));
                }
                redo(ed, false);
            });
        },
    );
    Ok(())
}

/// org-agenda-manipulate-query: add a word or regexp to a search.
fn manipulate_query(ed: &mut Editor, c: char) -> Result<(), String> {
    let i = cur_line(ed).unwrap_or(0);
    let v = view(ed)?;
    match v.kind_at(i) {
        Some(Kind::Agenda) => inactive(ed),
        Some(Kind::Search) => {
            let b = v.lines.get(i).map_or(0, |l| l.block);
            let Spec::Search { todo_only, string } = v.blocks[b].spec.clone() else {
                return Ok(());
            };
            let mut q = re!(r"(?: +| +[+-](?:\{\})?)$")
                .replace(&string, "")
                .into_owned();
            let boolean = string.starts_with(['+', '-', '{'])
                || opt_bool("org-agenda-search-view-always-boolean", false);
            q.push_str(if boolean {
                match c {
                    '[' => " +",
                    ']' => " -",
                    '{' => " +{}",
                    _ => " -{}",
                }
            } else {
                " "
            });
            let pos = if matches!(c, '{' | '}') {
                q.len() - 1
            } else {
                q.len()
            };
            let _ = pos;
            crate::org::read(ed, "Phrase or [+-]Word/{Regexp} ...: ", &q, move |ed, s| {
                if let Some(v) = ed.org_view.as_deref_mut() {
                    v.blocks[b].spec = Spec::Search {
                        todo_only,
                        string: s,
                    };
                }
                redo(ed, false);
            });
            Ok(())
        }
        Some(k) => Err(format!(
            "Cannot manipulate query for {}-type agenda buffers",
            k.name()
        )),
        None => Err("No Org agenda currently displayed".into()),
    }
}

// ---- bulk ----

fn marked(v: &View, i: usize) -> bool {
    v.lines[i]
        .item
        .as_ref()
        .and_then(|it| it.hd.as_ref())
        .is_some_and(|hd| v.marks.contains(hd))
}

fn bulk_mark(ed: &mut Editor, n: i64) -> Result<(), String> {
    let n = n.max(1);
    for _ in 0..n {
        let i = cur_line(ed).ok_or("Nothing to mark at point")?;
        let v = view_mut(ed)?;
        let it = v.lines[i].item.clone();
        if !it.as_ref().is_some_and(|x| x.diary_link) {
            let hd = it.and_then(|x| x.hd).ok_or("Nothing to mark at point")?;
            if !v.marks.contains(&hd) {
                v.marks.push(hd);
            }
        }
        let b = ed.cur.line;
        let col = ed.cur.byte;
        super::redisplay(ed);
        let v = view(ed)?;
        let vis = visible(ed);
        let next = (b + 1..vis.len())
            .find(|&x| is_item(v, vis[x]))
            .unwrap_or(b + 1);
        ed.set_cursor(next.min(ed.line_count() - 1), col);
        ed.clamp_cursor();
    }
    let c = view(ed)?.marks.len();
    ed.set_msg(format!("{c} entries marked for bulk action"));
    Ok(())
}

fn bulk_unmark(ed: &mut Editor, all: bool) -> Result<(), String> {
    let v = view_mut(ed)?;
    if all {
        if v.marks.is_empty() {
            ed.set_msg("No entry to unmark");
            return Ok(());
        }
        v.marks.clear();
        super::redisplay(ed);
        return Ok(());
    }
    let i = cur_line(ed).unwrap_or(0);
    let v = view_mut(ed)?;
    if !marked(v, i) {
        ed.set_msg("No entry to unmark here");
        return Ok(());
    }
    let hd = v.lines[i]
        .item
        .as_ref()
        .and_then(|it| it.hd.clone())
        .unwrap();
    v.marks.retain(|m| *m != hd);
    let left = v.marks.len();
    super::redisplay(ed);
    next_item(ed, 1);
    ed.set_msg(format!("{left} entries left marked for bulk action"));
    Ok(())
}

fn bulk_mark_regexp(ed: &mut Editor) -> Result<(), String> {
    view(ed)?;
    crate::org::read(ed, "Mark entries matching regexp: ", "", |ed, re| {
        let _ = mark_matching(ed, &re);
    });
    Ok(())
}

fn mark_matching(ed: &mut Editor, re: &str) -> Result<(), String> {
    let r = crate::org::re::compile(re, true)?;
    let v = view_mut(ed)?;
    let mut n = 0;
    for l in &v.lines {
        if !l.hidden.is_empty() {
            continue;
        }
        if let Some(it) = &l.item
            && let Some(hd) = &it.hd
            && r.is_match(&it.txt)
            && !it.diary_link
        {
            if !v.marks.contains(hd) {
                v.marks.push(hd.clone());
            }
            n += 1;
        }
    }
    let total = v.marks.len();
    super::redisplay(ed);
    if n == 0 {
        ed.set_msg("No entry matching this regexp.");
    } else {
        ed.set_msg(format!("{total} entries marked for bulk action"));
    }
    Ok(())
}

fn bulk_toggle_all(ed: &mut Editor) -> Result<(), String> {
    let v = view_mut(ed)?;
    let mut new = vec![];
    for l in &v.lines {
        if !l.hidden.is_empty() {
            continue;
        }
        if let Some(hd) = l.item.as_ref().and_then(|it| it.hd.clone())
            && !v.marks.contains(&hd)
            && !new.contains(&hd)
        {
            new.push(hd);
        }
    }
    v.marks = new;
    super::redisplay(ed);
    Ok(())
}

/// org-agenda-bulk-action.
fn bulk_action(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if view(ed)?.marks.is_empty() {
        let at = ed.cur.line;
        bulk_mark(ed, 1)?;
        ed.set_cursor(at, 0);
    }
    let persist = view(ed)?.bulk_persistent || opt_bool("org-agenda-persistent-marks", false);
    let mut entries: Vec<(String, String)> = [
        (
            "p",
            if persist {
                "persist marks (on)"
            } else {
                "persist marks (off)"
            },
        ),
        ("$", "archive"),
        ("A", "archive to sibling"),
        ("t", "todo"),
        ("+", "add tag"),
        ("-", "remove tag"),
        ("s", "schedule"),
        ("d", "deadline"),
        ("r", "refile"),
        ("S", "scatter"),
        ("f", "function"),
    ]
    .iter()
    .map(|(k, d)| ((*k).to_owned(), (*d).to_owned()))
    .collect();
    let custom = opt("org-agenda-bulk-custom-functions").unwrap_or(Sexp::Nil);
    for c in custom.list().unwrap_or(&[]) {
        if let Some(k) = c
            .car()
            .and_then(|x| x.int())
            .and_then(|n| char::from_u32(n as u32))
        {
            let f = c.cdr().car().map(|x| x.to_string()).unwrap_or_default();
            entries.push((k.to_string(), f));
        }
    }
    let title = format!(
        "Bulk ({}[p]ersist marks):",
        if persist { "" } else { "don't " }
    );
    crate::org::menu(ed, &title, entries, move |ed, k| {
        let r = bulk_choice(ed, &k, arg);
        if let Err(e) = r {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// A bulk command applied to one entry (the agenda cursor on it).
#[derive(Clone)]
enum BulkCmd {
    Archive(&'static str),
    Todo(String),
    Tag(String, bool),
    Schedule(bool, Option<String>),
    Scatter(i64, bool),
    Refile(String),
    Function(String),
}

fn bulk_choice(ed: &mut Editor, k: &str, arg: Prefix) -> Result<(), String> {
    match k {
        "p" => {
            let v = view_mut(ed)?;
            v.bulk_persistent = !v.bulk_persistent;
            bulk_action(ed, arg)
        }
        "$" => bulk_run(ed, BulkCmd::Archive("org-archive-subtree")),
        "A" => bulk_run(ed, BulkCmd::Archive("org-archive-to-archive-sibling")),
        "t" => {
            let kws = view(ed)?.todo_kwds.clone();
            crate::org::complete(ed, "Todo state: ", kws, false, |ed, s| {
                if let Err(e) = bulk_run(ed, BulkCmd::Todo(s)) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        "+" | "-" => {
            let on = k == "+";
            let tags: Vec<String> = view(ed)?.tag_alist.iter().map(|(t, _)| t.clone()).collect();
            let prompt = format!("Tag to {}: ", if on { "add" } else { "remove" });
            crate::org::complete(ed, &prompt, tags, false, move |ed, t| {
                if let Err(e) = bulk_run(ed, BulkCmd::Tag(t, on)) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        "s" | "d" => {
            let deadline = k == "d";
            if !arg.is_none() {
                return bulk_run(ed, BulkCmd::Schedule(deadline, None));
            }
            let opts = time::ReadOpts {
                prompt: Some(if deadline {
                    "(Re)Set Deadline to".into()
                } else {
                    "(Re)Schedule to".into()
                }),
                ..time::ReadOpts::default()
            };
            time::read_date(ed, opts, move |ed, a| {
                let ts = time::read::date_string(&a);
                if let Err(e) = bulk_run(ed, BulkCmd::Schedule(deadline, Some(ts))) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        "S" => {
            let i = cur_line(ed).unwrap_or(0);
            if !matches!(view(ed)?.kind_at(i), Some(Kind::Agenda | Kind::Todo)) {
                return Err(format!(
                    "Can't scatter tasks in \"{}\" agenda view",
                    view(ed)?.kind_at(i).map_or("", Kind::name)
                ));
            }
            let week = !arg.is_none();
            let prompt = format!(
                "Scatter tasks across how many {}days: ",
                if week { "week" } else { "" }
            );
            crate::org::read(ed, &prompt, "7", move |ed, n| {
                let n = n.trim().parse().unwrap_or(7);
                if let Err(e) = bulk_run(ed, BulkCmd::Scatter(n, week)) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        "r" | "w" => {
            // Refile every entry to one location: ask once, from the first entry.
            crate::org::read(
                ed,
                "Refile to (outline path or heading): ",
                "",
                |ed, target| {
                    if let Err(e) = bulk_run(ed, BulkCmd::Refile(target)) {
                        ed.set_err(e);
                    }
                },
            );
            Ok(())
        }
        "f" => {
            crate::org::read(ed, "Function: ", "", |ed, f| {
                if let Err(e) = bulk_run(ed, BulkCmd::Function(f)) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        k => {
            let custom = opt("org-agenda-bulk-custom-functions").unwrap_or(Sexp::Nil);
            let f = custom
                .list()
                .unwrap_or(&[])
                .iter()
                .find(|c| {
                    c.car()
                        .and_then(|x| x.int())
                        .and_then(|n| char::from_u32(n as u32))
                        .is_some_and(|c| c.to_string() == k)
                })
                .and_then(|c| c.cdr().car().and_then(|x| x.str().map(str::to_owned)))
                .ok_or_else(|| format!("Invalid bulk action: {k}"))?;
            bulk_run(ed, BulkCmd::Function(f))
        }
    }
}

/// Apply `cmd` to every marked entry (sorted by file and position).
fn bulk_run(ed: &mut Editor, cmd: BulkCmd) -> Result<(), String> {
    let v = view(ed)?;
    let mut entries = v.marks.clone();
    entries.sort_by_key(|a| (path_name(&a.path), a.line));
    let persist = v.bulk_persistent || opt_bool("org-agenda-persistent-marks", false);
    let name = v.name.clone();
    let today = time::today();
    let redo_at_end = matches!(
        cmd,
        BulkCmd::Refile(_) | BulkCmd::Function(_) | BulkCmd::Archive(_)
    );
    effect(ed, move |s| {
        let mut processed = 0;
        let mut skipped = 0;
        for e in entries {
            let Some((l, _)) = views::find_line(s, &e) else {
                skipped += 1;
                continue;
            };
            let Ok(i) = s.org_buffer(&e.path) else {
                skipped += 1;
                continue;
            };
            let cmd = cmd.clone();
            let r = s.org_with_buffer(i, |src| {
                if src.org.is_none() {
                    src.org = Some(Box::default());
                }
                crate::org::fold::widen(src);
                src.set_cursor(l, 0);
                let pos = src.cur.pos();
                src.undo.begin(pos);
                let r = match cmd {
                    BulkCmd::Archive(c) => call(src, c, Prefix::None),
                    BulkCmd::Todo(st) => {
                        let saved = crate::org::options::get("org-inhibit-logging");
                        crate::org::options::put(
                            "org-inhibit-logging",
                            toml::Value::String("note".into()),
                        );
                        let r = crate::org::todo::todo_to(src, Some(st));
                        match saved {
                            Some(v) => crate::org::options::put("org-inhibit-logging", v),
                            None => crate::org::options::remove("org-inhibit-logging"),
                        }
                        r
                    }
                    BulkCmd::Tag(t, on) => {
                        crate::org::tags::toggle_tag(src, l, &t, Some(on));
                        Ok(())
                    }
                    BulkCmd::Schedule(dl, ts) => time::schedule(src, dl, Prefix::None, ts),
                    BulkCmd::Scatter(days, week) => {
                        let mut dist = 1 + (rand_below(days.max(1)) as i64);
                        if week {
                            let weekend = |d: i64| {
                                let wd = Tm::from_absolute(d).weekday();
                                wd == 0 || wd == 6
                            };
                            let mut d = today;
                            let mut k = 0;
                            while k <= dist {
                                d += 1;
                                if !weekend(d) {
                                    k += 1;
                                }
                            }
                            dist = d - today - 1;
                        }
                        let t = Tm::from_absolute(today + dist);
                        let ts = civil::format("%Y-%m-%d", t);
                        time::schedule(src, false, Prefix::None, Some(ts)).or(Ok(()))
                    }
                    BulkCmd::Refile(target) => {
                        crate::org::call_with(src, "org-refile", Prefix::None, target)
                    }
                    BulkCmd::Function(f) => call(src, &f, Prefix::None),
                };
                let pos = src.cur.pos();
                src.undo.end(pos);
                src.clamp_cursor();
                r
            });
            let eff = s.org_with_buffer(i, |src| src.pending_effect.take());
            if let Some(eff) = eff {
                s.perform(eff);
            }
            match r {
                Ok(()) => processed += 1,
                Err(err) => {
                    s.ed.set_err(err);
                    skipped += 1;
                }
            }
        }
        if let Some(a) = s.org_agenda_find(&name) {
            s.org_with_buffer(a, |e| {
                if let Some(v) = e.org_view.as_deref_mut()
                    && !persist
                {
                    v.marks.clear();
                }
                let _ = redo_at_end;
                redo(e, false);
            });
            let eff = s.org_with_buffer(a, |e| e.pending_effect.take());
            if let Some(eff) = eff {
                s.perform(eff);
            }
        }
        s.ed.set_msg(format!(
            "Acted on {processed} entries{}{}",
            if skipped == 0 {
                String::new()
            } else {
                format!(", skipped {skipped} (disappeared before their turn)")
            },
            if persist { " (kept marked)" } else { "" }
        ));
    });
    Ok(())
}

/// A small pseudo-random number below `n` (org-agenda-bulk-action's `random`).
fn rand_below(n: i64) -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64);
    let x = t
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (x >> 33) % n.max(1) as u64
}

// ---- other commands ----

fn quit(ed: &mut Editor, kill: bool, release: bool) -> Result<(), String> {
    let v = view(ed)?;
    let bury = !kill && v.sticky;
    effect(ed, move |s| {
        if release {
            // org-release-buffers: the buffers the agenda opened, if unmodified.
            for p in super::take_new_buffers() {
                s.org_release(&p);
            }
        }
        if bury {
            if let Some(b) = s.ed.org_return {
                s.org_show(b);
            }
        } else {
            s.org_quit_temp();
            super::set_restriction(super::restriction().filter(|r| r.lock.is_some()));
        }
    });
    Ok(())
}

fn store_link(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let (_, hd) = markers(ed)?;
    super::remote(
        ed,
        hd,
        move |src| call(src, "org-store-link", arg),
        |_, _| {},
    );
    Ok(())
}

/// org-agenda-write: plain text, or an Org file of the entries.
fn write(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let v = view(ed)?;
    let text = ed.buf.text();
    let hds: Vec<Loc> = v
        .lines
        .iter()
        .filter(|l| l.hidden.is_empty())
        .filter_map(|l| l.item.as_ref().and_then(|it| it.hd.clone()))
        .collect();
    crate::org::read(ed, "Write agenda to file: ", "", move |ed, f| {
        let path = crate::org::options::expand(f.trim());
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let hds = hds.clone();
        let text = text.clone();
        let exists = path.exists();
        let shown = path.display().to_string();
        let doit = move |ed: &mut Editor| {
            effect(ed, move |s| {
                let r = if ext == "org" {
                    let mut content: Vec<String> = vec![];
                    for hd in hds {
                        if let Some((h, lines)) = views::find_line(s, &hd) {
                            let end = crate::org::syntax::subtree_end(&lines, h);
                            let tree = lines[h..end].join("\n");
                            if !content.contains(&tree) {
                                content.push(tree);
                            }
                        }
                    }
                    std::fs::write(&path, content.join("\n") + "\n")
                        .map(|()| format!("Org file written to {}", path.display()))
                } else if ext == "html" || ext == "htm" {
                    let body = text.lines().map(html_escape).collect::<Vec<_>>().join("\n");
                    std::fs::write(
                        &path,
                        format!("<html><head><meta charset=\"utf-8\"><title>Agenda View</title></head><body><pre>\n{body}\n</pre></body></html>\n"),
                    )
                    .map(|()| format!("HTML written to {}", path.display()))
                } else if ext == "ics" {
                    return match crate::org::call_with(
                        &mut s.ed,
                        "org-icalendar-export-current-agenda",
                        Prefix::None,
                        path.display().to_string(),
                    ) {
                        Ok(()) => {}
                        Err(e) => s.ed.set_err(e),
                    };
                } else if matches!(ext.as_str(), "ps" | "pdf") {
                    Err(std::io::Error::other(format!(
                        "Cannot write {ext}: no PostScript printer in Fred (write .txt or .html)"
                    )))
                } else {
                    std::fs::write(&path, text.clone() + "\n")
                        .map(|()| format!("Plain text written to {}", path.display()))
                };
                match r {
                    Ok(m) => s.ed.set_msg(m),
                    Err(e) => s.ed.set_err(format!(
                        "Cannot write agenda to file {}: {e}",
                        path.display()
                    )),
                }
                if !arg.is_none() {
                    let _ = s.org_visit(&path, 0);
                }
            });
        };
        if exists {
            crate::org::yes_or_no(ed, &format!("Overwrite existing file {shown}? "), doit);
        } else {
            doit(ed);
        }
    });
    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// org-agenda-undo: undo the last remote change in its buffer.
fn undo(ed: &mut Editor) -> Result<(), String> {
    if !opt_bool("org-agenda-allow-remote-undo", true) {
        return Err(
            "Check the variable `org-agenda-allow-remote-undo' to activate remote undo".into(),
        );
    }
    let v = view_mut(ed)?;
    let path = v.undo.pop().ok_or("No further undo information")?;
    effect(ed, move |s| {
        let Ok(i) = s.org_buffer(&path) else { return };
        let ok = s.org_with_buffer(i, |src| src.undo.undo(&mut src.buf).is_some());
        if ok {
            s.ed.set_msg(format!(
                "`org-agenda-undo' undone (buffer {})",
                path_name(&path)
            ));
            redo(&mut s.ed, false);
        } else {
            s.ed.set_err("No further undo information");
        }
    });
    Ok(())
}

fn drag_line(ed: &mut Editor, n: i64, backward: bool) -> Result<(), String> {
    let i = cur_line(ed).ok_or("Cannot move line forward")?;
    let v = view_mut(ed)?;
    let vis = v.map.clone();
    let b = vis.iter().position(|&x| x == i).unwrap_or(0);
    let target_b = if backward {
        b.checked_sub(n as usize)
    } else {
        Some(b + n as usize).filter(|&t| t < vis.len())
    };
    let ok = v.lines[i].item.is_some()
        && target_b.is_some_and(|t| {
            let range = if backward { t..b } else { b + 1..t + 1 };
            range.into_iter().all(|x| v.lines[vis[x]].item.is_some())
        });
    if !ok {
        ed.set_msg("Cannot move line forward");
        return Ok(());
    }
    let t = vis[target_b.unwrap()];
    let line = v.lines.remove(i);
    v.lines.insert(t, line);
    super::redisplay(ed);
    goto_view_line(ed, t);
    Ok(())
}

fn flagging_note(ed: &mut Editor) -> Result<(), String> {
    let it = item(ed)?;
    let hd = it.hd.clone().ok_or("No linked entry at point")?;
    let again = view(ed)?.last_command.as_deref() == Some("org-agenda-show-the-flagging-note");
    if again {
        let hd2 = hd.clone();
        crate::org::yes_or_no(ed, "Unflag and remove any flagging note? ", move |ed| {
            remote_then(
                ed,
                hd2.clone(),
                hd2,
                |src| {
                    let h = src.cur.line;
                    crate::org::tags::toggle_tag(src, h, "FLAGGED", Some(false));
                    crate::org::props::delete(src, Some(h), "THEFLAGGINGNOTE");
                    Ok(())
                },
                |s, _, _| s.ed.set_msg("Entry unflagged"),
            );
        });
        return Ok(());
    }
    effect(ed, move |s| {
        let Some((h, lines)) = views::find_line(s, &hd) else {
            return;
        };
        let st = crate::org::settings_of(&lines.join("\n"), hd.path.parent());
        let doc = crate::org::props::Doc::new(&lines[..], &st, Some(&hd.path));
        match doc.get(
            Some(h),
            "THEFLAGGINGNOTE",
            crate::org::props::Inherit::No,
            false,
        ) {
            None => s.ed.set_err("No flagging note"),
            Some(n) => {
                let text = n.replace("\\n", "\n");
                s.ed.reg.text = text.clone();
                s.org_temp_buffer("*Flagging Note*", &text, false);
                s.ed.set_msg(
                    "Flagging note pushed to kill ring.  Press `?' again to remove tag and note",
                );
            }
        }
    });
    Ok(())
}

fn capture(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let day = cursor_day(ed).unwrap_or_else(time::today);
    let with_time = arg == Prefix::Num(1);
    let mut t = Tm::from_absolute(day);
    if with_time {
        let now = civil::now();
        let tod = item(ed).ok().and_then(|i| i.time_of_day);
        match tod {
            Some(x) => {
                t.hour = x / 100;
                t.min = x % 100;
            }
            None => {
                t.hour = now.hour;
                t.min = now.min;
            }
        }
    } else {
        let now = civil::now();
        t.hour = now.hour;
        t.min = now.min;
    }
    crate::org::set_overriding_time(Some(t.epoch()));
    call(ed, "org-capture", Prefix::None)
}

/// org-agenda-diary-entry: into org-agenda-diary-file (Org) or the diary.
fn diary_entry(ed: &mut Editor) -> Result<(), String> {
    let day = cursor_day(ed).ok_or("No date defined in current line")?;
    let file = match opt("org-agenda-diary-file") {
        Some(Sexp::Str(f)) => crate::org::options::expand(&f),
        Some(Sexp::Sym(s)) if s != "diary-file" => crate::org::options::expand(&s),
        _ => super::diary::file(),
    };
    let org = file.extension().is_some_and(|e| e == "org");
    let entries: Vec<(String, String)> = if org {
        vec![
            ("d".into(), "day".into()),
            ("a".into(), "anniversary".into()),
            ("b".into(), "block".into()),
            ("j".into(), "jump to date tree".into()),
        ]
    } else {
        vec![
            ("d".into(), "day".into()),
            ("w".into(), "weekly".into()),
            ("m".into(), "monthly".into()),
            ("y".into(), "yearly".into()),
            ("a".into(), "anniversary".into()),
        ]
    };
    crate::org::menu(ed, "Diary entry:", entries, move |ed, k| {
        let t = Tm::from_absolute(day);
        if k == "j" {
            let f = file.clone();
            effect(ed, move |s| {
                let _ = s.org_visit(&f, 0);
            });
            return;
        }
        let f = file.clone();
        crate::org::read(ed, "Diary entry: ", "", move |ed, text| {
            let line = if org {
                match k.as_str() {
                    "a" => format!(
                        "%%(org-anniversary {} {:2} {:2}) {text}",
                        t.year, t.month, t.day
                    ),
                    _ => format!("* {text}\n  {}", stamp::stamp_text(t, false, false, "")),
                }
            } else {
                let mname = &civil::MONTH_NAMES[(t.month - 1) as usize][..3];
                match k.as_str() {
                    "w" => format!("{} {text}", civil::DAY_NAMES[t.weekday() as usize]),
                    "m" => format!("* {} {text}", t.day),
                    "y" => format!("{mname} {} {text}", t.day),
                    "a" => format!(
                        "%%(diary-anniversary {} {} {}) {text}",
                        t.month, t.day, t.year
                    ),
                    _ => format!("{mname} {}, {} {text}", t.day, t.year),
                }
            };
            effect(ed, move |s| {
                let mut old = std::fs::read_to_string(&f).unwrap_or_default();
                if !old.is_empty() && !old.ends_with('\n') {
                    old.push('\n');
                }
                old.push_str(&line);
                old.push('\n');
                match std::fs::write(&f, old) {
                    Ok(()) => {
                        s.ed.set_msg(format!("Entry added to {}", f.display()));
                        redo(&mut s.ed, false);
                    }
                    Err(e) => s.ed.set_err(e.to_string()),
                }
            });
        });
    });
    Ok(())
}

fn convert_date(ed: &mut Editor) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let day = cursor_day(ed).ok_or("Don't know which date to convert")?;
    let s = super::solar::convert(day);
    effect(ed, move |ss| {
        ss.org_temp_buffer("*Dates*", &s, false);
    });
    Ok(())
}

fn calendar_cmd(ed: &mut Editor, what: &str) -> Result<(), String> {
    views::check_type(ed, &[Kind::Agenda])?;
    let day = cursor_day(ed).ok_or("Don't know which date to use for the calendar command")?;
    let (name, text) = match what {
        "holidays" => ("*Holidays*", super::solar::holidays(day)),
        "moon" => ("*Phases of Moon*", super::solar::moon_phases(day)),
        _ => {
            let s = super::solar::sunrise_sunset(day)?;
            ed.set_msg(s);
            return Ok(());
        }
    };
    effect(ed, move |s| {
        s.org_temp_buffer(name, &text, false);
    });
    Ok(())
}

/// org-agenda-set-restriction-lock in an Org buffer.
pub fn set_restriction_lock(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let file = ed
        .path
        .clone()
        .and_then(|p| std::path::absolute(p).ok())
        .ok_or("Restriction lock needs a file")?;
    let h = crate::org::syntax::heading_at_or_before(&ed.buf, ed.cur.line);
    let cur = super::restriction();
    if let (Some(c), Some(h)) = (&cur, h)
        && c.lock == Some("subtree")
        && c.file == file
        && c.range.map(|r| r.0) == Some(h)
    {
        super::set_restriction(None);
        ed.set_msg("Agenda restriction lock removed");
        return maybe_redo(ed);
    }
    let file_lock = arg == Prefix::U(1) || h.is_none();
    if file_lock {
        super::set_restriction(Some(super::Restriction {
            file,
            range: None,
            lock: Some("file"),
        }));
        ed.set_msg("Locking agenda restriction to file");
    } else {
        let h = h.unwrap();
        super::set_restriction(Some(super::Restriction {
            file,
            range: Some((h, crate::org::syntax::subtree_end(&ed.buf, h))),
            lock: Some("subtree"),
        }));
        ed.set_msg("Locking agenda restriction to subtree");
    }
    maybe_redo(ed)
}

/// org-agenda-maybe-redo: rebuild open agenda buffers.
fn maybe_redo(ed: &mut Editor) -> Result<(), String> {
    let lock = super::restriction().and_then(|r| r.lock);
    effect(ed, move |s| {
        let names = s.org_agenda_names();
        for n in names {
            if let Some(i) = s.org_agenda_find(&n) {
                s.org_with_buffer(i, |e| redo(e, false));
                let eff = s.org_with_buffer(i, |e| e.pending_effect.take());
                if let Some(eff) = eff {
                    s.perform(eff);
                }
                s.ed.set_msg(match lock {
                    Some(l) => format!("Agenda view shifted to new {l} restriction"),
                    None => "Agenda restriction lock removed".to_owned(),
                });
            }
        }
    });
    Ok(())
}

fn remove_restriction_lock(ed: &mut Editor, noupdate: bool) -> Result<(), String> {
    if super::restriction().is_none() {
        ed.set_msg("No agenda restriction to remove.");
        return Ok(());
    }
    super::set_restriction(None);
    ed.set_msg("Agenda restriction lock removed");
    if noupdate { Ok(()) } else { maybe_redo(ed) }
}

/// The list form of org-agenda-files (for C-c [ and C-c ]).
fn agenda_file_list() -> (Vec<String>, Option<std::path::PathBuf>) {
    match crate::org::sexp::option("org-agenda-files") {
        Some(Sexp::Str(f)) => {
            let p = crate::org::options::expand(&f);
            let list = std::fs::read_to_string(&p)
                .map(|t| {
                    t.lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            (list, Some(p))
        }
        Some(v) => (
            v.list()
                .unwrap_or(&[])
                .iter()
                .filter_map(|x| x.str().map(str::to_owned))
                .collect(),
            None,
        ),
        None => (vec![], None),
    }
}

fn store_agenda_files(list: &[String], file: Option<std::path::PathBuf>) -> Result<(), String> {
    match file {
        Some(p) => std::fs::write(&p, list.join("\n") + "\n").map_err(|e| e.to_string()),
        None => {
            crate::org::options::put(
                "org-agenda-files",
                toml::Value::Array(
                    list.iter()
                        .map(|s| toml::Value::String(s.clone()))
                        .collect(),
                ),
            );
            Ok(())
        }
    }
}

fn abbreviate(p: &std::path::Path) -> String {
    crate::pick::browse::tilde(p)
}

/// org-agenda-file-to-front (C-c [).
pub fn file_to_front(ed: &mut Editor, to_end: bool) -> Result<(), String> {
    let p = ed.path.clone().ok_or("Buffer is not visiting a file")?;
    let p = std::path::absolute(&p).map_err(|e| e.to_string())?;
    let (mut list, file) = agenda_file_list();
    let canon = |s: &str| {
        std::fs::canonicalize(crate::org::options::expand(s))
            .unwrap_or_else(|_| crate::org::options::expand(s))
    };
    let me = std::fs::canonicalize(&p).unwrap_or(p.clone());
    let had = list.iter().position(|f| canon(f) == me);
    let entry = had
        .map(|i| list.remove(i))
        .unwrap_or_else(|| abbreviate(&p));
    if to_end {
        list.push(entry.clone());
    } else {
        list.insert(0, entry.clone());
    }
    store_agenda_files(&list, file)?;
    ed.set_msg(format!(
        "File {entry} {} list of agenda files",
        if had.is_some() {
            if to_end {
                "moved to end of"
            } else {
                "moved to front of"
            }
        } else if to_end {
            "added to end of"
        } else {
            "added to front of"
        }
    ));
    Ok(())
}

/// org-remove-file (C-c ]).
pub fn remove_file(ed: &mut Editor) -> Result<(), String> {
    let p = ed.path.clone().ok_or("Buffer is not visiting a file")?;
    let me = std::fs::canonicalize(&p).unwrap_or(p.clone());
    let (mut list, file) = agenda_file_list();
    let n = list.len();
    list.retain(|f| {
        let c = crate::org::options::expand(f);
        std::fs::canonicalize(&c).unwrap_or(c) != me
    });
    if list.len() == n {
        ed.set_msg(format!(
            "File {} was not in list of agenda files",
            abbreviate(&p)
        ));
        return Ok(());
    }
    store_agenda_files(&list, file)?;
    ed.set_msg(format!("Removed from Org Agenda list: {}", abbreviate(&p)));
    Ok(())
}

/// org-cycle-agenda-files (C-' and C-,).
pub fn cycle_agenda_files(ed: &mut Editor) -> Result<(), String> {
    let files = crate::org::agenda_files();
    if files.is_empty() {
        return Err("No agenda files".into());
    }
    let me = ed.path.clone().and_then(|p| std::fs::canonicalize(p).ok());
    let i = me
        .and_then(|m| {
            files
                .iter()
                .position(|f| std::fs::canonicalize(f).is_ok_and(|c| c == m))
        })
        .map_or(0, |i| (i + 1) % files.len());
    let f = files[i].clone();
    effect(ed, move |s| {
        let _ = s.org_visit(&f, 0);
    });
    Ok(())
}

/// org-agenda-to-appt: today's timed entries become appointments.
fn to_appt(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let filter = if arg.is_none() {
        None
    } else {
        Some(String::new())
    };
    let _ = filter;
    effect(ed, move |s| {
        let today = time::today();
        let mut ctx = super::collect::Ctx::new("agenda", today);
        ctx.habits = false;
        let saved = crate::org::options::get("org-deadline-warning-days");
        crate::org::options::put("org-deadline-warning-days", toml::Value::Integer(0));
        let mut appts = vec![];
        let types: Vec<String> = [":deadline*", ":scheduled*", ":timestamp"]
            .map(String::from)
            .to_vec();
        for f in super::files(true, false) {
            let Ok(text) = s.org_text(&f) else { continue };
            let src = Src::new(f, &text, None);
            for it in super::collect::day_entries(&src, &ctx, today, &types) {
                let Some(tod) = it.time_of_day else { continue };
                let done = it
                    .todo_state
                    .as_ref()
                    .is_some_and(|t| it.done_kwds.contains(t));
                if done {
                    continue;
                }
                let evt = re!(r"\[\[[^\]]*\]\[([^\]]*)\]\]")
                    .replace_all(it.txt.trim(), "$1")
                    .into_owned();
                appts.push((
                    format!("{}:{:02}", tod / 100, tod % 100),
                    evt,
                    it.warntime.clone(),
                ));
            }
        }
        match saved {
            Some(v) => crate::org::options::put("org-deadline-warning-days", v),
            None => crate::org::options::remove("org-deadline-warning-days"),
        }
        let n = appts.len();
        super::set_appointments(appts);
        s.ed.set_msg(if n == 0 {
            "No event to add".to_owned()
        } else {
            format!("Added {n} event{} for today", if n > 1 { "s" } else { "" })
        });
    });
    Ok(())
}

/// org-agenda-show-clocking-issues on built lines (clockcheck log mode).
pub fn clocking_issues(lines: Vec<Line>) -> Vec<Line> {
    let checks = opt("org-agenda-clock-consistency-checks").unwrap_or(Sexp::Nil);
    let dur = |k: &str, d: &str| {
        let v = checks
            .plist_get(k)
            .map(|v| v.str().map_or_else(|| v.to_string(), str::to_owned));
        time::duration::to_minutes(v.as_deref().unwrap_or(d), false).unwrap_or(0.0)
    };
    let maxtime = dur(":max-duration", "10:00");
    let mintime = dur(":min-duration", "0");
    let maxgap = dur(":max-gap", "0:05");
    let gapok: Vec<f64> = checks
        .plist_get(":gap-ok-around")
        .and_then(Sexp::list)
        .map(|l| {
            l.iter()
                .filter_map(|x| {
                    x.str()
                        .and_then(|s| time::duration::to_minutes(s, false).ok())
                })
                .collect()
        })
        .unwrap_or_else(|| vec![240.0, 720.0]);
    let re = re!(r"^[ \t]*CLOCK:[ \t]+(\[[^\]]*\])(-{1,3}(\[[^\]]*\]))?");
    let secs = |s: &str| stamp::time_string_to_seconds(s).unwrap_or(0) as f64;
    let mut tlend = 0.0;
    let mut out = vec![];
    for l in lines {
        let is_clock = l.item.as_ref().is_some_and(|it| it.kind == "clock");
        if !is_clock {
            out.push(l);
            continue;
        }
        let it = l.item.as_ref().unwrap();
        let text = it
            .marker
            .as_ref()
            .map(|m| m.text.clone())
            .unwrap_or_default();
        let mut issue = None;
        let mut te = None;
        match re.captures(&text) {
            None => issue = Some("No valid clock line".to_owned()),
            Some(c) => match c.get(3) {
                None => {
                    let start = secs(&c[1]);
                    let mins = ((civil::now_epoch() as f64 - start) / 60.0).floor();
                    issue = Some(format!(
                        "No end time: ({})",
                        time::duration::from_minutes(mins, None, false).unwrap_or_default()
                    ));
                }
                Some(e) => {
                    let ts = secs(&c[1]);
                    let tend = secs(e.as_str());
                    te = Some(tend);
                    let dt = tend - ts;
                    let fmt = |m: f64| {
                        time::duration::from_minutes(m.floor(), None, false).unwrap_or_default()
                    };
                    if dt > 60.0 * maxtime {
                        issue = Some(format!(
                            "Clocking interval is very long: {}",
                            fmt(dt / 60.0)
                        ));
                    } else if dt < 60.0 * mintime {
                        issue = Some(format!(
                            "Clocking interval is very short: {}",
                            fmt(dt / 60.0)
                        ));
                    } else if tlend > 0.0 && ts < tlend {
                        issue = Some(format!(
                            "Clocking overlap: {} minutes",
                            ((tlend - ts) / 60.0) as i64
                        ));
                    } else if tlend > 0.0
                        && ts > tlend + 60.0 * maxgap
                        && !gap_ok(tlend, ts, &gapok)
                    {
                        issue = Some(format!(
                            "Clocking gap: {} minutes",
                            ((ts - tlend) / 60.0) as i64
                        ));
                    }
                }
            },
        }
        if let Some(t) = te {
            tlend = t;
        }
        if let Some(i) = issue {
            out.push(Line {
                text: format!("{:<43}", format!(" {i}")),
                face: Face::Warning,
                kind: LineKind::Issue,
                day: l.day,
                ..Line::default()
            });
        }
        out.push(l);
    }
    out
}

/// org-agenda-check-clock-gap.
fn gap_ok(t1: f64, t2: f64, ok: &[f64]) -> bool {
    if ok.is_empty() {
        return false;
    }
    if (t2 - t1) / 3600.0 > 24.0 {
        return true;
    }
    let tod = |t: f64| {
        let tm = Tm::from_epoch(t as i64);
        (tm.hour * 60 + tm.min) as f64
    };
    let min1 = tod(t1);
    let mut min2 = tod(t2);
    if min2 < min1 {
        min2 += 1440.0;
    }
    ok.iter().any(|&x| {
        let x = if x < min1 { x + 1440.0 } else { x };
        min1 <= x && x <= min2
    })
}

// ---- the command table ----

/// The agenda buffer's commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    // Commands usable from any buffer.
    let r = match name {
        "org-agenda-set-restriction-lock" if ed.org_view.is_none() => set_restriction_lock(ed, arg),
        "org-agenda-remove-restriction-lock" => remove_restriction_lock(ed, !arg.is_none()),
        "org-agenda-file-to-front" => file_to_front(ed, !arg.is_none()),
        "org-remove-file" => remove_file(ed),
        "org-cycle-agenda-files" => cycle_agenda_files(ed),
        "org-agenda-to-appt" => to_appt(ed, arg),
        "org-calendar-goto-agenda" => {
            let d = time::calendar_date().map_or_else(time::today, Tm::absolute);
            views::start(
                ed,
                vec![super::Block {
                    spec: views::list_at(d, None),
                    lprops: vec![],
                }],
                None,
                Some("a".into()),
                Prefix::None,
            )
        }
        _ => return agenda_command(ed, name, arg),
    };
    Some(r)
}

fn agenda_command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let in_agenda = ed.org_view.is_some();
    // Remember the command for repeated-key behavior.
    let last = ed.org_view.as_ref().and_then(|v| v.this_command.clone());
    if let Some(v) = ed.org_view.as_deref_mut() {
        v.last_command = last;
        v.this_command = Some(name.to_owned());
    }
    let n = arg.value();
    let r = match name {
        "org-agenda-next-line" => {
            move_lines(ed, n);
            context_action(ed);
            Ok(())
        }
        "org-agenda-previous-line" => {
            move_lines(ed, -n);
            context_action(ed);
            Ok(())
        }
        "org-agenda-next-item" => {
            next_item(ed, n);
            context_action(ed);
            Ok(())
        }
        "org-agenda-previous-item" => {
            next_item(ed, -n);
            context_action(ed);
            Ok(())
        }
        "evil-goto-first-line" => {
            ed.set_cursor(0, 0);
            Ok(())
        }
        "evil-execute-in-emacs-state" => {
            if let Some(v) = ed.org_view.as_deref_mut() {
                v.emacs_next = true;
            }
            // The prefix argument carries over to the next command.
            ed.org_arg = arg;
            ed.set_msg("-- EMACS --");
            Ok(())
        }
        "ignore" => Ok(()),
        "org-agenda-end-of-line" => {
            let l = ed.cur.line;
            ed.cur.byte = ed.buf.line(l).len();
            ed.clamp_cursor();
            Ok(())
        }
        "org-agenda-next-date-line" => date_line(ed, n),
        "org-agenda-previous-date-line" => date_line(ed, -n),
        "org-agenda-forward-block" => forward_block(ed, false),
        "org-agenda-backward-block" => forward_block(ed, true),
        "org-agenda-later" => later(ed, n),
        "org-agenda-earlier" => later(ed, -n),
        "org-agenda-goto-today" => goto_today(ed),
        "org-agenda-goto-date" => goto_date(ed),
        "org-agenda-view-mode-dispatch" => {
            if !in_agenda {
                return Some(Err("Not in an Org agenda buffer".into()));
            }
            view_dispatch(ed);
            Ok(())
        }
        "org-agenda-reset-view" => change_span(ed, views::default_span(), None),
        "org-agenda-day-view" => change_span(ed, Span::Day, num(arg)),
        "org-agenda-week-view" => change_span(ed, Span::Week, num(arg)),
        "org-agenda-fortnight-view" => change_span(ed, Span::Fortnight, num(arg)),
        "org-agenda-month-view" => change_span(ed, Span::Month, num(arg)),
        "org-agenda-year-view" => {
            if !in_agenda {
                return Some(Err("Not in an Org agenda buffer".into()));
            }
            let y = num(arg).map(stamp::small_year_to_year);
            crate::org::yes_or_no(
                ed,
                "Are you sure you want to compute the agenda for an entire year? ",
                move |ed| {
                    if let Err(e) = change_span(ed, Span::Year, y) {
                        ed.set_err(e);
                    }
                },
            );
            Ok(())
        }
        "org-agenda-log-mode" => log_mode(ed, arg),
        "org-agenda-archives-mode" => archives_mode(ed, !arg.is_none()),
        "org-agenda-toggle-diary" => {
            toggle_opt(ed, "org-agenda-include-diary", false, "Diary inclusion")
        }
        "org-agenda-toggle-deadlines" => toggle_opt(
            ed,
            "org-agenda-include-deadlines",
            true,
            "Deadlines inclusion",
        ),
        "org-agenda-toggle-time-grid" => {
            toggle_opt(ed, "org-agenda-use-time-grid", true, "Time-grid")
        }
        "org-agenda-follow-mode" => follow_mode(ed),
        "org-agenda-entry-text-mode" => entry_text_mode(ed, arg),
        "org-agenda-clockreport-mode" => clockreport(ed),
        "org-agenda-entry-text-show" => {
            let v = view_mut(ed);
            match v {
                Ok(v) => {
                    v.modes.entry_text = Some(opt_int("org-agenda-entry-text-maxlines", 5));
                    redo(ed, false);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        "org-agenda-show-clocking-issues" => {
            let v = view_mut(ed);
            match v {
                Ok(v) => {
                    v.lines = clocking_issues(std::mem::take(&mut v.lines));
                    super::redisplay(ed);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        "org-agenda-dim-blocked-tasks" | "org-agenda-fontify-priorities" => {
            if !in_agenda {
                return Some(Err("Not in an Org agenda buffer".into()));
            }
            if name == "org-agenda-dim-blocked-tasks" {
                ed.set_msg("Dim or hide blocked tasks...done");
            }
            super::redisplay(ed);
            Ok(())
        }
        "org-habit-toggle-habits" => super::habit::toggle(ed, false),
        "org-habit-toggle-display-in-agenda" => super::habit::toggle(ed, !arg.is_none()),
        "org-agenda-redo" => {
            if !in_agenda {
                return Some(Err("Not in an Org agenda buffer".into()));
            }
            redo_with_arg(ed, arg)
        }
        "org-agenda-redo-all" => {
            if !in_agenda {
                return Some(Err("Not in an Org agenda buffer".into()));
            }
            if arg.is_none() {
                redo(ed, true);
                Ok(())
            } else {
                maybe_redo(ed)
            }
        }
        "org-agenda-quit" => quit(ed, false, false),
        "org-agenda-Quit" => quit(ed, true, false),
        "org-agenda-exit" => quit(ed, true, true),
        "org-agenda-kill-all-agenda-buffers" => {
            effect(ed, |s| s.org_agenda_kill_all());
            Ok(())
        }
        "org-agenda-execute" => {
            views::dispatch(ed, arg, None);
            Ok(())
        }
        "org-agenda-append-agenda" => match view_mut(ed) {
            Ok(v) => {
                v.appending = true;
                views::dispatch(ed, arg, None);
                Ok(())
            }
            Err(_) => Err("Can only append from within agenda buffer".into()),
        },
        "org-agenda-show-tags" => item(ed).map(|it| {
            let tags = it.tag_names();
            if tags.is_empty() {
                ed.set_msg("No tags associated with this line");
            } else {
                ed.set_msg(format!("Tags are :{}:", tags.join(":")));
            }
        }),
        "org-agenda-goto" | "org-agenda-goto-mouse" | "org-agenda-tree-to-indirect-buffer" => {
            visit(ed, false)
        }
        "org-agenda-switch-to" => visit(ed, false),
        "org-agenda-show"
        | "org-agenda-show-mouse"
        | "org-agenda-show-1"
        | "org-agenda-cycle-show"
        | "org-agenda-recenter" => {
            if item(ed).is_err() {
                return Some(Err("Command not allowed in this line".into()));
            }
            show_entry(ed, !arg.is_none() || name == "org-agenda-show-1");
            Ok(())
        }
        "org-agenda-show-and-scroll-up" => {
            if item(ed).is_err() {
                return Some(Err("Command not allowed in this line".into()));
            }
            show_entry(ed, true);
            Ok(())
        }
        "org-agenda-show-scroll-down" => {
            hide_shown(ed);
            Ok(())
        }
        "org-agenda-open-link" => open_link(ed, arg),
        "org-agenda-todo" => todo(ed, arg, ""),
        "org-agenda-todo-nextset" => todo(ed, arg, "nextset"),
        "org-agenda-todo-previousset" => todo(ed, arg, "previousset"),
        "org-agenda-todo-yesterday" => {
            let saved = (
                crate::org::options::get("org-use-effective-time"),
                crate::org::options::get("org-extend-today-until"),
            );
            crate::org::options::put("org-use-effective-time", toml::Value::Boolean(true));
            crate::org::options::put(
                "org-extend-today-until",
                toml::Value::Integer(civil::now().hour + 1),
            );
            let r = todo(ed, arg, "");
            let _ = saved;
            r
        }
        "org-agenda-add-note" => at_marker(ed, "org-add-note", Prefix::None),
        "org-agenda-priority" => {
            if arg == Prefix::U(1) {
                at_heading(ed, "org-priority", arg)
            } else {
                at_heading(ed, "org-priority", Prefix::None)
            }
        }
        "org-agenda-priority-up" => at_heading(ed, "org-priority-up", Prefix::None),
        "org-agenda-priority-down" => at_heading(ed, "org-priority-down", Prefix::None),
        "org-agenda-set-tags" | "org-agenda-ctrl-c-ctrl-c" => set_tags(ed, None),
        "org-agenda-set-property" => at_heading(ed, "org-set-property", Prefix::None),
        "org-agenda-set-effort" => at_heading(ed, "org-set-effort", Prefix::None),
        "org-agenda-toggle-archive-tag" => at_heading(ed, "org-toggle-archive-tag", Prefix::None),
        "org-agenda-do-date-later" => do_date(ed, arg, true),
        "org-agenda-do-date-earlier" => do_date(ed, arg, false),
        "org-agenda-date-later" => date_later(ed, n, civil::Unit::Day),
        "org-agenda-date-earlier" => date_later(ed, -n, civil::Unit::Day),
        "org-agenda-date-later-minutes" => date_later(
            ed,
            n * stamp::rounding_minutes().1.max(1),
            civil::Unit::Minute,
        ),
        "org-agenda-date-earlier-minutes" => date_later(
            ed,
            -n * stamp::rounding_minutes().1.max(1),
            civil::Unit::Minute,
        ),
        "org-agenda-date-later-hours" => date_later(ed, n, civil::Unit::Hour),
        "org-agenda-date-earlier-hours" => date_later(ed, -n, civil::Unit::Hour),
        "org-agenda-date-prompt" => date_prompt(ed, arg),
        "org-agenda-schedule" => schedule(ed, false, arg, crate::org::take_call_arg()),
        "org-agenda-deadline" => schedule(ed, true, arg, crate::org::take_call_arg()),
        "org-agenda-kill" => kill(ed),
        "org-agenda-archive-default" => archive_with(ed, archive_default_cmd(), false),
        "org-agenda-archive-default-with-confirmation" => {
            archive_with(ed, archive_default_cmd(), true)
        }
        "org-agenda-archive" => archive_with(ed, "org-archive-subtree", false),
        "org-agenda-archive-to-archive-sibling" => {
            archive_with(ed, "org-archive-to-archive-sibling", false)
        }
        "org-agenda-archive-with" => {
            let c = crate::org::take_call_arg().unwrap_or_else(|| archive_default_cmd().into());
            let c: &'static str = match c.as_str() {
                "org-archive-to-archive-sibling" => "org-archive-to-archive-sibling",
                "org-toggle-archive-tag" => "org-toggle-archive-tag",
                _ => "org-archive-subtree",
            };
            archive_with(ed, c, false)
        }
        "org-agenda-refile" => refile(ed, arg),
        "org-agenda-clock-in" => clock_in(ed, arg),
        "org-agenda-clock-out" => clock_cmd(ed, "org-clock-out"),
        "org-agenda-clock-cancel" => clock_cmd(ed, "org-clock-cancel"),
        "org-agenda-clock-goto" => clock_goto(ed),
        "org-agenda-capture" => capture(ed, arg),
        "org-agenda-diary-entry" => diary_entry(ed),
        "org-agenda-holidays" => calendar_cmd(ed, "holidays"),
        "org-agenda-phases-of-moon" => calendar_cmd(ed, "moon"),
        "org-agenda-sunrise-sunset" => calendar_cmd(ed, "sun"),
        "org-agenda-goto-calendar" => {
            if let Err(e) = views::check_type(ed, &[Kind::Agenda]) {
                return Some(Err(e));
            }
            match cursor_day(ed) {
                Some(d) => {
                    time::set_calendar_date(Some(Tm::from_absolute(d)));
                    let t = Tm::from_absolute(d);
                    let h = time::diary::holidays(t);
                    ed.set_msg(format!(
                        "Calendar date: {}{}",
                        civil::format("%A, %B %-d, %Y", t),
                        if h.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", h.join("; "))
                        }
                    ));
                    Ok(())
                }
                None => Err("Don't know which date to open in calendar".into()),
            }
        }
        "org-agenda-convert-date" => convert_date(ed),
        "org-agenda-bulk-mark" => bulk_mark(ed, n),
        "org-agenda-bulk-mark-all" => mark_matching(ed, "."),
        "org-agenda-bulk-mark-regexp" => bulk_mark_regexp(ed),
        "org-agenda-bulk-unmark" => bulk_unmark(ed, !arg.is_none()),
        "org-agenda-bulk-unmark-all" => bulk_unmark(ed, true),
        "org-agenda-bulk-toggle" => {
            let i = cur_line(ed).unwrap_or(0);
            match view(ed) {
                Ok(v) if marked(v, i) => bulk_unmark(ed, false),
                Ok(_) => bulk_mark(ed, 1),
                Err(e) => Err(e),
            }
        }
        "org-agenda-bulk-toggle-all" => bulk_toggle_all(ed),
        "org-agenda-bulk-remove-overlays" => {
            if view(ed).is_ok() {
                super::redisplay(ed);
            }
            Ok(())
        }
        "org-agenda-bulk-action" => bulk_action(ed, arg),
        "org-agenda-filter-by-category" => filter_category(ed, !arg.is_none()),
        "org-agenda-filter-by-top-headline" => filter_top(ed, !arg.is_none()),
        "org-agenda-filter-by-regexp" => filter_regexp(ed, arg),
        "org-agenda-filter-by-effort" => filter_effort(ed, arg),
        "org-agenda-filter" => filter_general(ed, arg),
        "org-agenda-filter-remove-all" => filter_remove_all(ed),
        "org-agenda-filter-by-tag" => filter_tag(ed, arg),
        "org-agenda-limit-interactively" => limit_interactively(ed, !arg.is_none()),
        "org-agenda-manipulate-query-add" => manipulate_query(ed, '['),
        "org-agenda-manipulate-query-subtract" => manipulate_query(ed, ']'),
        "org-agenda-manipulate-query-add-re" => manipulate_query(ed, '{'),
        "org-agenda-manipulate-query-subtract-re" => manipulate_query(ed, '}'),
        "org-agenda-drag-line-forward" => drag_line(ed, n, false),
        "org-agenda-drag-line-backward" => drag_line(ed, n, true),
        "org-agenda-show-the-flagging-note" => flagging_note(ed),
        "org-agenda-undo" => undo(ed),
        "org-agenda-write" => write(ed, arg),
        "org-agenda-store-link" => store_link(ed, arg),
        "org-agenda-set-restriction-lock-from-agenda" | "org-agenda-set-restriction-lock" => {
            match markers(ed) {
                Ok((m, _)) => {
                    super::remote(ed, m, move |src| set_restriction_lock(src, arg), |_, _| {});
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        "org-agenda-menu" => {
            views::dispatch(ed, arg, None);
            Ok(())
        }
        "org-agenda-mode" => Ok(()),
        "org-agenda-columns" => call(ed, "org-columns", arg)
            .map_err(|_| "org-agenda-columns: column view is not yet ported to fred".into()),
        _ => return None,
    };
    Some(r)
}

fn num(arg: Prefix) -> Option<i64> {
    match arg {
        Prefix::Num(n) => Some(n),
        _ => None,
    }
}

/// org-agenda-redo with a prefix: a fresh search, or the Nth keyword.
fn redo_with_arg(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let i = cur_line(ed).unwrap_or(0);
    let v = view_mut(ed)?;
    let b = v.lines.get(i).map_or(0, |l| l.block);
    match (&mut v.blocks[b].spec, arg) {
        (Spec::Todo { keywords, n }, Prefix::Num(k)) => {
            *n = Some(k);
            *keywords = None;
        }
        (Spec::Search { todo_only, string }, Prefix::U(_)) => {
            let t = *todo_only;
            let s0 = string.clone();
            crate::org::read(
                ed,
                "Phrase or [+-]Word/{Regexp} ...: ",
                &s0,
                move |ed, s| {
                    if let Some(v) = ed.org_view.as_deref_mut() {
                        v.blocks[b].spec = Spec::Search {
                            todo_only: t,
                            string: s,
                        };
                    }
                    redo(ed, false);
                },
            );
            return Ok(());
        }
        (Spec::Tags { todo_only, matcher }, Prefix::U(_)) => {
            let t = *todo_only;
            let m0 = matcher.clone();
            crate::org::read(ed, "Match: ", &m0, move |ed, m| {
                if let Some(v) = ed.org_view.as_deref_mut() {
                    v.blocks[b].spec = Spec::Tags {
                        todo_only: t,
                        matcher: m,
                    };
                }
                redo(ed, false);
            });
            return Ok(());
        }
        _ => {}
    }
    redo(ed, false);
    Ok(())
}
