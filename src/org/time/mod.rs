//! Timestamps, the date prompt, scheduling, repeaters and durations:
//! the time parts of org.el, org-macs.el and org-duration.el.
//!
//! Submodules: [`civil`] (calendar arithmetic, local time, injectable
//! now), [`stamp`] (the timestamp model and string operations),
//! [`read`] (the org-read-date language), [`diary`] (diary sexps),
//! [`duration`] (org-duration).
//!
//! Fred adaptations (see docs/org-port-notes.md, "Time"):
//! - org-read-date has no calendar window. The live interpretation is
//!   shown in the prompt (`Date+time [2026-10-04] => <2026-10-09 Fri>: `)
//!   and the calendar keys of org-read-date-minibuffer-local-map move a
//!   virtual calendar cursor whose date joins the answer as upstream's
//!   org-ans2 does (S-arrows day/week, M-S-arrows month/year, `<` `>`
//!   C-v M-v months, `.` on an empty answer: today).
//! - org-goto-calendar sets that calendar date (session-wide, like the
//!   one *Calendar* buffer) and reports it; the org-calendar-* motion
//!   commands move it; org-date-from-calendar inserts it.
//! - org-display-custom-times is a per-buffer flag ([`display_custom_times`])
//!   the renderer queries with [`stamp::custom_overlays`].

// `re!` compiles once per call site (OnceLock), also inside loops.
#![allow(clippy::regex_creation_in_loops)]

macro_rules! re {
    ($s:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($s).unwrap())
    }};
}

pub mod civil;
pub mod diary;
pub mod duration;
pub mod read;
pub mod stamp;
#[cfg(test)]
mod tests;

use super::{Prefix, ctx, fold, line, options, set_line, sexp, syntax};
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use civil::{Tm, Unit};
use read::Analysis;
use stamp::{Brackets, ChangeOpts, Part, What, stamp_text};
use std::cell::RefCell;

const STAMP_COMMANDS: [&str; 4] = [
    "org-time-stamp",
    "org-time-stamp-inactive",
    "org-timestamp",
    "org-timestamp-inactive",
];

/// This module's interactive commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-timestamp" | "org-time-stamp" => timestamp(ed, arg, false),
        "org-timestamp-inactive" | "org-time-stamp-inactive" => timestamp(ed, arg, true),
        "org-timestamp-up" => change_at_point(ed, arg.value(), None, opts(arg, true)).map(drop),
        "org-timestamp-down" => change_at_point(ed, -arg.value(), None, opts(arg, true)).map(drop),
        "org-timestamp-up-day" | "org-timestamp-down-day" => {
            let up = name == "org-timestamp-up-day";
            let l = ed.cur.line;
            if stamp::at_timestamp(&line(ed, l), ed.cur.byte).is_none() && ctx::at_heading(ed, l) {
                super::call(
                    ed,
                    if up {
                        "org-shiftright"
                    } else {
                        "org-shiftleft"
                    },
                    Prefix::None,
                )
            } else {
                let n = if up { arg.value() } else { -arg.value() };
                change_at_point(ed, n, Some(What::Unit(Unit::Day)), opts(arg, up)).map(drop)
            }
        }
        "org-toggle-timestamp-type" => {
            let (l, b) = (ed.cur.line, ed.cur.byte);
            let text = line(ed, l);
            if let Some((r, _)) = stamp::at_timestamp(&text, b) {
                let new = stamp::toggle_type(&text[r.clone()]);
                let active = new.starts_with('<');
                set_line(
                    ed,
                    l,
                    &format!("{}{new}{}", &text[..r.start], &text[r.end..]),
                );
                ed.set_msg(format!(
                    "Timestamp is now {}active",
                    if active { "" } else { "in" }
                ));
            }
            Ok(())
        }
        "org-evaluate-time-range" => evaluate_time_range(ed, !arg.is_none()),
        "org-toggle-timestamp-overlays" | "org-toggle-time-stamp-overlays" => {
            let on = !display_custom_times(ed);
            if let Some(o) = &mut ed.org {
                o.custom_times = Some(on);
            }
            ed.set_msg(if on {
                "Time stamps are overlaid with custom format"
            } else {
                "Time stamp overlays removed"
            });
            Ok(())
        }
        "org-schedule" => schedule(ed, false, arg, None),
        "org-deadline" => schedule(ed, true, arg, None),
        "org-check-deadlines" => {
            check_deadlines(ed, arg);
            Ok(())
        }
        "org-check-before-date" | "org-check-after-date" => {
            let before = name == "org-check-before-date";
            read_date(ed, ReadOpts::default(), move |ed, a| {
                let d = read::date_string(&a);
                let n = check_dates(ed, Some(&d), None, before);
                ed.set_msg(format!(
                    "{n} entries {} {d}",
                    if before { "before" } else { "after" }
                ));
            });
            Ok(())
        }
        "org-check-dates-range" => {
            let start = ReadOpts {
                prompt: Some("Range starts".into()),
                ..ReadOpts::default()
            };
            read_date(ed, start, |ed, a| {
                let s = read::date_string(&a);
                let end = ReadOpts {
                    prompt: Some("Range end".into()),
                    ..ReadOpts::default()
                };
                read_date(ed, end, move |ed, b| {
                    let e = read::date_string(&b);
                    let n = check_dates(ed, None, Some((&s, &e)), false);
                    ed.set_msg(format!("{n} entries between {s} and {e}"));
                });
            });
            Ok(())
        }
        "org-goto-calendar" => {
            goto_calendar(ed, !arg.is_none());
            Ok(())
        }
        "org-date-from-calendar" => date_from_calendar(ed),
        "org-calendar-goto-today"
        | "org-calendar-goto-today-or-insert-dot"
        | "org-calendar-backward-month"
        | "org-calendar-forward-month"
        | "org-calendar-backward-year"
        | "org-calendar-forward-year"
        | "org-calendar-backward-week"
        | "org-calendar-forward-week"
        | "org-calendar-backward-day"
        | "org-calendar-forward-day"
        | "org-calendar-scroll-month-left"
        | "org-calendar-scroll-month-right"
        | "org-calendar-scroll-three-months-left"
        | "org-calendar-scroll-three-months-right"
        | "org-calendar-view-entries"
        | "org-calendar-select" => {
            let d = calendar_date().unwrap_or_else(|| civil::now().midnight());
            let d = calendar_move(name, d);
            set_calendar_date(Some(d));
            ed.set_msg(calendar_message(d));
            Ok(())
        }
        "org-duration-set-regexps" => {
            duration::set_regexps();
            Ok(())
        }
        _ => return None,
    })
}

fn opts(arg: Prefix, updown: bool) -> ChangeOpts {
    ChangeOpts {
        updown,
        prefix: !arg.is_none(),
        suppress_tmp_delay: false,
    }
}

/// Run `f` as one undo step (prompt continuations run after the
/// command's own group closed).
fn group(ed: &mut Editor, f: impl FnOnce(&mut Editor)) {
    ed.undo.begin(ed.cur.pos());
    f(ed);
    ed.undo.end(ed.cur.pos());
    ed.clamp_cursor();
}

fn replace(ed: &mut Editor, l: usize, r: std::ops::Range<usize>, with: &str) {
    let t = line(ed, l);
    set_line(ed, l, &format!("{}{with}{}", &t[..r.start], &t[r.end..]));
}

fn insert_at(ed: &mut Editor, l: usize, b: usize, s: &str) {
    replace(ed, l, b..b, s);
    ed.cur.line = l;
    ed.cur.byte = b + s.len();
}

/// org-insert-timestamp: insert PRE, the stamp for `t` (with `extra`
/// before its closing bracket), POST at the cursor; returns the stamp
/// (org-last-inserted-timestamp).
pub fn insert_timestamp(
    ed: &mut Editor,
    t: Tm,
    with_hm: bool,
    inactive: bool,
    pre: &str,
    post: &str,
    extra: &str,
) -> String {
    let s = stamp_text(t, with_hm, inactive, extra);
    let (l, b) = (ed.cur.line, ed.cur.byte);
    insert_at(ed, l, b, &format!("{pre}{s}{post}"));
    s
}

/// org-current-effective-time: now, or yesterday 23:59 before
/// org-extend-today-until when org-use-effective-time.
/// ponytail: org-use-last-clock-out-time-as-effective-time needs the
/// clock module's last clock-out; not consulted here.
pub fn current_effective_time() -> Tm {
    let ct = read::current_time(stamp::rounding_minutes().0, false);
    if options::bool("org-use-effective-time", false)
        && ct.hour < options::int("org-extend-today-until", 0)
    {
        Tm::at(ct.year, ct.month, ct.day - 1, 23, 59).norm()
    } else {
        ct
    }
}

// ---- the date prompt ----

/// org-read-date's arguments.
#[derive(Clone, Debug, Default)]
pub struct ReadOpts {
    /// WITH-TIME: suggest a time.
    pub with_time: bool,
    /// WITH-TIME is `(16)`: no rounding of the current time.
    pub exact: bool,
    /// PROMPT before "Date+time [...]: ".
    pub prompt: Option<String>,
    /// DEFAULT-TIME.
    pub default: Option<Tm>,
    /// DEFAULT-INPUT.
    pub input: String,
    /// Display the interpretation as an inactive timestamp.
    pub inactive: bool,
}

struct PromptState {
    head: String,
    def: Tm,
    with_time: bool,
    inactive: bool,
    /// The calendar cursor once moved (org-ans2).
    cal: Option<Tm>,
}

thread_local! {
    static PROMPT: RefCell<Option<PromptState>> = const { RefCell::new(None) };
    static CALENDAR: std::cell::Cell<Option<Tm>> = const { std::cell::Cell::new(None) };
    static TS_TYPE: RefCell<Option<String>> = const { RefCell::new(None) };
    static LOG: RefCell<Option<LogRequest>> = const { RefCell::new(None) };
}

/// org-read-date: prompt, then `then(analysis)` (to-time: the analysis'
/// `tm`; string: [`read::date_string`]).
pub fn read_date(
    ed: &mut Editor,
    o: ReadOpts,
    then: impl FnOnce(&mut Editor, Analysis) + Send + 'static,
) {
    let def = read::default_time(o.default, o.exact);
    let timestr = civil::format(
        if o.with_time {
            "%Y-%m-%d %H:%M"
        } else {
            "%Y-%m-%d"
        },
        def,
    );
    let head = format!(
        "{}Date+time [{timestr}]",
        o.prompt
            .as_deref()
            .map(|p| format!("{p} "))
            .unwrap_or_default()
    );
    PROMPT.with(|p| {
        *p.borrow_mut() = Some(PromptState {
            head,
            def,
            with_time: o.with_time,
            inactive: o.inactive,
            cal: None,
        })
    });
    let prompt = live_prompt(&o.input);
    super::read(ed, &prompt, &o.input, move |ed, ans| {
        let st = PROMPT.with(|p| p.borrow_mut().take());
        let cal = st
            .as_ref()
            .and_then(|s| s.cal)
            .map(|c| civil::format("%Y-%m-%d", c))
            .unwrap_or_default();
        let a = read::analyze(&format!("{ans} {cal}"), def);
        if a.forced_year {
            ed.set_msg("Year was forced into compatible range (1970-2037)");
        }
        then(ed, a);
    });
}

/// The prompt with org-read-date-display's interpretation of `text`.
fn live_prompt(text: &str) -> String {
    PROMPT.with(|p| {
        let p = p.borrow();
        let Some(st) = p.as_ref() else {
            return String::new();
        };
        if !options::bool("org-read-date-display-live", true) {
            return format!("{}: ", st.head);
        }
        let cal = st
            .cal
            .map(|c| civil::format("%Y-%m-%d", c))
            .unwrap_or_default();
        let a = read::analyze(&format!("{text} {cal}"), st.def);
        let fmt = stamp::time_stamp_format(
            st.with_time || a.time_given,
            Brackets::of(st.inactive),
            false,
        );
        let mut txt = civil::format(&fmt, a.tm.norm());
        if let Some(e) = &a.end_time
            && let Some(m) = stamp::plain_time_of_day().find(&txt)
        {
            txt.insert_str(m.end(), &format!("-{e}"));
        }
        if a.future {
            txt.push_str(" (=>F)");
        }
        format!("{} => {txt}: ", st.head)
    })
}

fn refresh_prompt(ed: &mut Editor) {
    if let Mode::Command(cl) = &ed.mode {
        let p = live_prompt(&cl.text.clone());
        if let Mode::Command(cl) = &mut ed.mode {
            cl.prompt = p;
        }
    }
}

/// Calendar motion by an org-calendar-* command name.
fn calendar_move(name: &str, d: Tm) -> Tm {
    let today = civil::now().midnight();
    match name {
        "org-calendar-goto-today" | "org-calendar-goto-today-or-insert-dot" => today,
        "org-calendar-backward-month" | "org-calendar-scroll-month-right" => d.add(Unit::Month, -1),
        "org-calendar-forward-month" | "org-calendar-scroll-month-left" => d.add(Unit::Month, 1),
        "org-calendar-backward-year" => d.add(Unit::Year, -1),
        "org-calendar-forward-year" => d.add(Unit::Year, 1),
        "org-calendar-backward-week" => d.add(Unit::Day, -7),
        "org-calendar-forward-week" => d.add(Unit::Day, 7),
        "org-calendar-backward-day" => d.add(Unit::Day, -1),
        "org-calendar-forward-day" => d.add(Unit::Day, 1),
        "org-calendar-scroll-three-months-left" => d.add(Unit::Month, 3),
        "org-calendar-scroll-three-months-right" => d.add(Unit::Month, -3),
        _ => d,
    }
}

/// org-read-date-minibuffer-local-map: Org keys while the date prompt is
/// open; editing keys refresh the live interpretation. Called by
/// `org::key` before anything else.
pub fn prompt_key(ed: &mut Editor, k: Key) -> bool {
    if PROMPT.with(|p| p.borrow().is_none()) {
        return false;
    }
    let Mode::Command(cl) = &mut ed.mode else {
        PROMPT.with(|p| *p.borrow_mut() = None);
        return false;
    };
    if cl.kind != 'o' {
        PROMPT.with(|p| *p.borrow_mut() = None);
        return false;
    }
    let leaving = k.is(KeyCode::Esc)
        || k == Key::ctrl('c')
        || k == Key::ctrl('g')
        || (k.code == KeyCode::Backspace && cl.text.is_empty());
    if leaving {
        PROMPT.with(|p| *p.borrow_mut() = None);
        if k == Key::ctrl('g') {
            ed.mode = Mode::Normal;
            ed.org_then = None;
            return true;
        }
        return false;
    }
    let at_start = cl.cursor == 0;
    let cmd = match k.code {
        KeyCode::Left if k.shift && k.alt => Some("org-calendar-backward-month"),
        KeyCode::Right if k.shift && k.alt => Some("org-calendar-forward-month"),
        KeyCode::Up if k.shift && k.alt => Some("org-calendar-backward-year"),
        KeyCode::Down if k.shift && k.alt => Some("org-calendar-forward-year"),
        KeyCode::Left if k.shift => Some("org-calendar-backward-day"),
        KeyCode::Right if k.shift => Some("org-calendar-forward-day"),
        KeyCode::Up if k.shift => Some("org-calendar-backward-week"),
        KeyCode::Down if k.shift => Some("org-calendar-forward-week"),
        KeyCode::Char('.') if k.ctrl => Some("org-calendar-goto-today"),
        KeyCode::Char('.') if !k.alt && at_start => Some("org-calendar-goto-today"),
        KeyCode::Char('>') if !k.ctrl && !k.alt => Some("org-calendar-scroll-month-left"),
        KeyCode::Char('<') if !k.ctrl && !k.alt => Some("org-calendar-scroll-month-right"),
        KeyCode::Char('v') if k.ctrl => Some("org-calendar-scroll-three-months-left"),
        KeyCode::Char('v') if k.alt => Some("org-calendar-scroll-three-months-right"),
        KeyCode::Char('!') if !k.ctrl && !k.alt => Some("org-calendar-view-entries"),
        _ => None,
    };
    if let Some(cmd) = cmd {
        let d = PROMPT.with(|p| {
            let mut p = p.borrow_mut();
            let st = p.as_mut().unwrap();
            let d = calendar_move(cmd, st.cal.unwrap_or_else(|| st.def.midnight()));
            st.cal = Some(d);
            d
        });
        if cmd == "org-calendar-view-entries" {
            ed.set_msg(format!(
                "No diary entries for {}",
                civil::format("%A, %B %e, %Y", d)
            ));
        }
        refresh_prompt(ed);
        return true;
    }
    let edits = match k.code {
        KeyCode::Char(_) => !k.alt,
        KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End => !k.alt && !k.shift,
        _ => false,
    };
    if !edits {
        return false;
    }
    cl.edit(k);
    refresh_prompt(ed);
    true
}

// ---- org-timestamp ----

fn timestamp(ed: &mut Editor, arg: Prefix, inactive: bool) -> Result<(), String> {
    let (l, b) = (ed.cur.line, ed.cur.byte);
    let text = line(ed, l);
    let last = ed.org.as_ref().and_then(|o| o.last_command.clone());
    // The timestamp at point, or the range end point is in.
    let found: Option<(std::ops::Range<usize>, String)> =
        if let Some((_, r1, r2)) = stamp::date_range_at(&text, b, true) {
            let inner = if b + 2 < r2.start { r1 } else { r2 };
            let r = inner.start - 1..inner.end + 1;
            Some((r.clone(), text[r].to_owned()))
        } else {
            stamp::at_timestamp(&text, b).map(|(r, _)| (r.clone(), text[r].to_owned()))
        };
    let default = found
        .as_ref()
        .and_then(|(_, ts)| stamp::time_string_to_tm(ts).ok());
    let input = found
        .as_ref()
        .and_then(|(_, ts)| stamp::compact_tod(ts))
        .unwrap_or_default();
    let repeater = found.as_ref().and_then(|(_, ts)| {
        re!(r"([.+-]+[0-9]+[hdwmy] ?)+")
            .find(ts)
            .map(|m| m.as_str().to_owned())
    });
    let range_append =
        found.is_some() && last.as_deref().is_some_and(|c| STAMP_COMMANDS.contains(&c));
    let finish = move |ed: &mut Editor, time: Tm, given: bool, end: Option<String>| {
        group(ed, |ed| {
            let extra = stamp::end_time_extra(end.as_deref());
            match &found {
                Some((r, _)) if range_append => {
                    let s = format!("--{}", stamp_text(time, given, inactive, ""));
                    insert_at(ed, l, r.end, &s);
                }
                Some((r, _)) => {
                    let mut s = stamp_text(time, given, inactive, &extra);
                    if let Some(rep) = &repeater {
                        s.insert_str(s.len() - 1, &format!(" {rep}"));
                    }
                    replace(ed, l, r.clone(), &s);
                    ed.cur.line = l;
                    ed.cur.byte = r.start + s.len() - 1;
                    ed.set_msg("Timestamp updated");
                }
                None => {
                    let s = stamp_text(time, given, inactive, &extra);
                    insert_at(ed, l, b, &s);
                }
            }
        });
    };
    if arg == Prefix::U(2) {
        finish(ed, civil::now(), true, None);
        return Ok(());
    }
    let o = ReadOpts {
        with_time: !arg.is_none(),
        exact: false,
        prompt: None,
        default,
        input,
        inactive,
    };
    let with_time = !arg.is_none();
    read_date(ed, o, move |ed, a| {
        finish(
            ed,
            a.tm.norm(),
            a.time_given || with_time,
            a.end_time.clone(),
        )
    });
    Ok(())
}

// ---- org-timestamp-change ----

/// org-timestamp-change at point (the cursor): N times WHAT (None: by
/// the part point is on). Returns the new timestamp text
/// (org-last-changed-timestamp).
pub fn change_at_point(
    ed: &mut Editor,
    n: i64,
    what: Option<What>,
    o: ChangeOpts,
) -> Result<String, String> {
    let (l, b) = (ed.cur.line, ed.cur.byte);
    let text = line(ed, l);
    let (r, part) = stamp::at_timestamp(&text, b).ok_or("Not at a timestamp")?;
    let c = stamp::change(&text[r.clone()], b - r.start, part, n, what, o)?;
    replace(ed, l, r.clone(), &c.text);
    ed.cur.byte = r.start + c.point;
    // org-calendar-follow-timestamp-change.
    let dated = matches!(what, Some(What::Unit(Unit::Day | Unit::Month | Unit::Year)))
        || (what.is_none() && matches!(part, Part::Day | Part::Month | Part::Year));
    if dated
        && calendar_date().is_some()
        && options::bool("org-calendar-follow-timestamp-change", true)
    {
        set_calendar_date(stamp::time_string_to_tm(&c.text).ok().map(Tm::midnight));
    }
    if c.toggled {
        ed.set_msg(format!(
            "Timestamp is now {}active",
            if c.text.starts_with('<') { "" } else { "in" }
        ));
        return Ok(c.text);
    }
    clock_update_time_maybe(ed, l);
    Ok(c.text)
}

/// org-timestamp-change on the timestamp starting at byte `start` of
/// line `l` (point at its start, so the "day" part).
pub fn change_at(
    ed: &mut Editor,
    l: usize,
    start: usize,
    n: i64,
    unit: Unit,
    suppress_tmp_delay: bool,
) -> Result<String, String> {
    let text = line(ed, l);
    let m = stamp::ts3()
        .find_at(&text, start)
        .filter(|m| m.start() == start)
        .ok_or("Not at a timestamp")?;
    let o = ChangeOpts {
        suppress_tmp_delay,
        ..ChangeOpts::default()
    };
    let c = stamp::change(m.as_str(), 1, Part::Day, n, Some(What::Unit(unit)), o)?;
    replace(ed, l, m.range(), &c.text);
    Ok(c.text)
}

/// org-clock-update-time-maybe on line `l`: normalize a CLOCK range and
/// recompute its `=> H:MM`. True when it was a CLOCK range.
pub fn clock_update_time_maybe(ed: &mut Editor, l: usize) -> bool {
    let re = re!(r"^[ \t]*CLOCK: *[\[<]([^\]>]+)[\]>](-+[\[<]([^\]>]+)[\]>]([ \t]*=>.*)?)?");
    let text = line(ed, l);
    let Some(c) = re.captures(&text) else {
        return false;
    };
    if c.get(2).is_none() {
        return false;
    }
    let normalize = |ts: &str| -> String {
        stamp::change(
            ts,
            1,
            Part::Day,
            0,
            Some(What::Unit(Unit::Day)),
            ChangeOpts::default(),
        )
        .map_or(ts.to_owned(), |c| c.text)
    };
    let r1 = c.get(1).unwrap().start() - 1..c.get(1).unwrap().end() + 1;
    let r2 = c.get(3).unwrap().start() - 1..c.get(3).unwrap().end() + 1;
    let t1 = normalize(&text[r1.clone()]);
    let t2 = normalize(&text[r2.clone()]);
    let mid = &text[r1.end..r2.start];
    let s = stamp::time_string_to_seconds(&t2).unwrap_or(0)
        - stamp::time_string_to_seconds(&t1).unwrap_or(0);
    let (neg, s) = (s < 0, s.abs());
    let (h, m) = (s / 3600, s % 3600 / 60);
    let dur = if neg {
        format!("-{h}:{m:02}")
    } else {
        format!("{h:2}:{m:02}")
    };
    let new = format!("{}{t1}{mid}{t2} => {dur}", &text[..r1.start]);
    set_line(ed, l, &new);
    true
}

// ---- org-evaluate-time-range ----

fn evaluate_time_range(ed: &mut Editor, to_buffer: bool) -> Result<(), String> {
    let l = ed.cur.line;
    if clock_update_time_maybe(ed, l) {
        return Ok(());
    }
    let text = line(ed, l);
    let (r, i1, i2) = stamp::date_range_at(&text, ed.cur.byte, true)
        .or_else(|| {
            let c = stamp::tr_both().captures(&text)?;
            Some((c.get(0)?.range(), c.get(1)?.range(), c.get(2)?.range()))
        })
        .ok_or("Not at a timestamp range, and none found in current line")?;
    let (ts1, ts2) = (&text[i1], &text[i2]);
    let havetime = ts1.chars().count() > 15 || ts2.chars().count() > 15;
    let (t1, t2) = (
        stamp::time_string_to_seconds(ts1)?,
        stamp::time_string_to_seconds(ts2)?,
    );
    let negative = t2 < t1;
    let diff = (t2 - t1).abs() as f64;
    let (d, h, m) = if havetime {
        let d = (diff / 86400.0).floor();
        let rest = diff - d * 86400.0;
        let h = (rest / 3600.0).floor();
        (
            d as i64,
            h as i64,
            ((rest - h * 3600.0) / 60.0).floor() as i64,
        )
    } else {
        ((diff / 86400.0).round_ties_even() as i64, 0, 0)
    };
    if !to_buffer {
        ed.set_msg(stamp::tdiff_string(0, d, h, m));
        return Ok(());
    }
    let mut at = r.end;
    let in_table = ctx::at_table(ed, l);
    if in_table && let Some(m) = re!(r"^ *\|").find(&text[at..]) {
        at += m.end();
    }
    let mut new = text.clone();
    if let Some(m) = re!(r"^( *-? *[0-9]+y)?( *[0-9]+d)? *[0-9][0-9]:[0-9][0-9]").find(&new[at..]) {
        new.replace_range(at..at + m.end(), "");
    }
    let mut ins = String::new();
    if negative {
        ins.push_str(" -");
    }
    ins.push(' ');
    ins.push_str(&if d > 0 {
        if havetime {
            format!("{d}d {h:02}:{m:02}")
        } else {
            format!("{d}d")
        }
    } else {
        format!("{h:02}:{m:02}")
    });
    new.insert_str(at, &ins);
    set_line(ed, l, &new);
    if in_table {
        let _ = super::call(ed, "org-table-align", Prefix::None);
    }
    ed.set_msg("Time difference inserted");
    Ok(())
}

// ---- custom time display ----

/// org-display-custom-times for this buffer: toggled by
/// org-toggle-timestamp-overlays, else `#+STARTUP: customtime` or the
/// option. The renderer shows [`stamp::custom_overlays`] when true.
pub fn display_custom_times(ed: &Editor) -> bool {
    if let Some(v) = ed.org.as_ref().and_then(|o| o.custom_times) {
        return v;
    }
    super::settings(ed).opt_bool("org-display-custom-times", false)
}

// ---- calendar ----

/// The calendar cursor date (org-goto-calendar's *Calendar*).
pub fn calendar_date() -> Option<Tm> {
    CALENDAR.with(std::cell::Cell::get)
}

pub fn set_calendar_date(d: Option<Tm>) {
    CALENDAR.with(|c| c.set(d));
}

fn calendar_message(d: Tm) -> String {
    let hol = diary::holidays(d);
    format!(
        "Calendar: {}{}",
        civil::format("%A, %B %e, %Y (week %V)", d),
        if hol.is_empty() {
            String::new()
        } else {
            format!(": {}", hol.join("; "))
        }
    )
}

fn goto_calendar(ed: &mut Editor, today: bool) {
    let text = line(ed, ed.cur.line);
    let ts = stamp::at_timestamp(&text, ed.cur.byte)
        .map(|(r, _)| text[r].to_owned())
        .or_else(|| {
            re!(r".*<([0-9]{4}-[0-9]{2}-[0-9]{2}(?: [^\n]*?)?)>")
                .captures(&text)
                .map(|c| c[1].to_owned())
        });
    let mut d = civil::now().midnight();
    if !today && let Some(t) = ts.and_then(|t| stamp::time_string_to_tm(&t).ok()) {
        d = t.midnight();
    }
    set_calendar_date(Some(d));
    ed.set_msg(calendar_message(d));
}

fn date_from_calendar(ed: &mut Editor) -> Result<(), String> {
    let d = calendar_date().ok_or("No calendar date: use org-goto-calendar first")?;
    let text = line(ed, ed.cur.line);
    if stamp::at_timestamp(&text, ed.cur.byte).is_some() {
        change_at_point(ed, 0, Some(What::Calendar(d)), ChangeOpts::default()).map(drop)
    } else {
        let s = stamp_text(d, false, false, "");
        insert_at(ed, ed.cur.line, ed.cur.byte, &s);
        Ok(())
    }
}

// ---- planning lines ----

/// A planning keyword.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Planning {
    Closed,
    Deadline,
    Scheduled,
}

impl Planning {
    pub fn keyword(self) -> &'static str {
        match self {
            Planning::Closed => "CLOSED:",
            Planning::Deadline => "DEADLINE:",
            Planning::Scheduled => "SCHEDULED:",
        }
    }

    /// org-closed-time-regexp / org-deadline-time-regexp /
    /// org-scheduled-time-regexp (group 1 the inside).
    pub fn time_re(self) -> &'static regex::Regex {
        match self {
            Planning::Closed => re!(r"\bCLOSED: *\[([^\]]+)\]"),
            Planning::Deadline => re!(r"\bDEADLINE: *<([^>]+)>"),
            Planning::Scheduled => re!(r"\bSCHEDULED: *<([^>]+)>"),
        }
    }
}

fn keyword_time_not_clock() -> &'static regex::Regex {
    re!(r"\b(SCHEDULED:|DEADLINE:|CLOSED:) *[\[<]([^\]>]+)[\]>]")
}

/// org-planning-line-re.
pub fn is_planning_line(text: &str) -> bool {
    re!(r"^[ \t]*(CLOSED:|DEADLINE:|SCHEDULED:)").is_match(text)
}

fn planning_of(ed: &Editor, h: usize) -> Option<usize> {
    (h + 1 < ed.line_count() && is_planning_line(&line(ed, h + 1))).then_some(h + 1)
}

/// org-entry-get "SCHEDULED"/"DEADLINE"/"CLOSED" of the entry at
/// heading `h`: the timestamp with its brackets.
pub fn planning_get(ed: &Editor, h: usize, kind: Planning) -> Option<String> {
    let l = planning_of(ed, h)?;
    let text = line(ed, l);
    let c = kind.time_re().captures(&text)?;
    let g = c.get(1)?;
    Some(text[g.start() - 1..g.end() + 1].to_owned())
}

fn adapt_indentation(ed: &Editor) -> bool {
    super::settings(ed).opt_bool("org-adapt-indentation", false)
}

/// org-add-planning-info on the entry at heading `h` with a ready
/// timestamp text: remove the `remove` kinds (and `what`'s old one),
/// then put `KEYWORD ts` first on the planning line (creating it after
/// the heading, indented when org-adapt-indentation). A planning line
/// left empty is deleted.
pub fn set_planning(
    ed: &mut Editor,
    h: usize,
    what: Option<(Planning, &str)>,
    remove: &[Planning],
) {
    if let Some(l) = planning_of(ed, h) {
        let mut t = line(ed, l);
        let ind = t.len() - t.trim_start_matches([' ', '\t']).len();
        for ty in what.iter().map(|w| w.0).chain(remove.iter().copied()) {
            if let Some(m) = ty.time_re().find_at(&t, ind) {
                let end = keyword_time_not_clock()
                    .find_at(&t, m.end())
                    .map_or(t.len(), |k| k.start());
                t.replace_range(m.start()..end, "");
            }
        }
        if t[ind..].trim_matches([' ', '\t']).is_empty() && what.is_none() {
            super::delete_lines(ed, l, 1);
            return;
        }
        let keep = t.trim_end_matches([' ', '\t']).len().max(ind);
        t.truncate(keep);
        if let Some((ty, ts)) = what {
            let sep = if ind < t.len() { " " } else { "" };
            t.insert_str(ind, &format!("{} {ts}{sep}", ty.keyword()));
        }
        set_line(ed, l, &t);
    } else if let Some((ty, ts)) = what {
        let indent = if adapt_indentation(ed) {
            " ".repeat(syntax::level(&line(ed, h)).unwrap_or(0) + 1)
        } else {
            String::new()
        };
        super::insert_lines(ed, h + 1, &[format!("{indent}{} {ts}", ty.keyword())]);
    }
}

/// org-add-planning-info with a time: the inserted timestamp text.
/// CLOSED stamps are inactive and carry the time when `with_time` or
/// org-log-done-with-time.
pub fn add_planning_info(
    ed: &mut Editor,
    h: usize,
    what: Option<(Planning, Tm, bool)>,
    remove: &[Planning],
) -> Option<String> {
    let ts = what.map(|(ty, t, with_time)| {
        let closed = ty == Planning::Closed;
        let wt = with_time || (closed && options::bool("org-log-done-with-time", true));
        (ty, stamp_text(t, wt, closed, ""))
    });
    set_planning(ed, h, ts.as_ref().map(|(ty, s)| (*ty, s.as_str())), remove);
    ts.map(|(_, s)| s)
}

/// org-remove-timestamp-with-keyword on the planning line of `h`.
pub fn remove_timestamp_with_keyword(ed: &mut Editor, h: usize, kind: Planning) {
    let Some(l) = planning_of(ed, h) else { return };
    let mut t = line(ed, l);
    let re = match kind {
        Planning::Closed => re!(r"\bCLOSED: +<[^>\n]+>[ \t]*"),
        Planning::Deadline => re!(r"\bDEADLINE: +<[^>\n]+>[ \t]*"),
        Planning::Scheduled => re!(r"\bSCHEDULED: +<[^>\n]+>[ \t]*"),
    };
    let ranges: Vec<_> = re.find_iter(&t).map(|m| m.range()).collect();
    for r in ranges.into_iter().rev() {
        t.replace_range(r.clone(), "");
        if t[..r.start].chars().any(|c| !c.is_whitespace()) && t[..r.start].ends_with(' ') {
            t.remove(r.start - 1);
        } else if t.trim().is_empty() {
            super::delete_lines(ed, l, 1);
            return;
        }
    }
    set_line(ed, l, &t);
}

/// A note the logging module records (org-add-log-setup's arguments):
/// `purpose` is reschedule, redeadline, delschedule or deldeadline;
/// `how` the org-log-reschedule/org-log-redeadline value (time, note).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRequest {
    pub heading: usize,
    pub purpose: &'static str,
    pub state: Option<String>,
    pub previous: String,
    pub how: String,
}

/// The pending reschedule/redeadline log request. Contract: after
/// org-schedule/org-deadline change a date with logging on, this module
/// stores the request and calls the command `org-add-log-setup`; the
/// logging module answers that name by taking the request here.
pub fn take_log_request() -> Option<LogRequest> {
    LOG.with(|l| l.borrow_mut().take())
}

fn log_option(ed: &Editor, deadline: bool) -> Option<String> {
    let v = super::settings(ed).opt(if deadline {
        "org-log-redeadline"
    } else {
        "org-log-reschedule"
    })?;
    if v.is_nil() {
        return None;
    }
    Some(v.str().unwrap_or("time").to_owned())
}

fn request_log(ed: &mut Editor, r: LogRequest) {
    LOG.with(|l| *l.borrow_mut() = Some(r));
    if super::call(ed, "org-add-log-setup", Prefix::None).is_err() {
        LOG.with(|l| *l.borrow_mut() = None);
    }
}

fn heading_of(ed: &Editor) -> Result<usize, String> {
    syntax::heading_at_or_before(&ed.buf, ed.cur.line)
        .ok_or_else(|| "Before first headline".to_owned())
}

/// org-deadline / org-schedule (org--deadline-or-schedule). `time`:
/// an Org date or a delta like "+2d" (Lisp callers); None prompts.
pub fn schedule(
    ed: &mut Editor,
    deadline: bool,
    arg: Prefix,
    time: Option<String>,
) -> Result<(), String> {
    let h = heading_of(ed)?;
    let kind = if deadline {
        Planning::Deadline
    } else {
        Planning::Scheduled
    };
    let old = planning_get(ed, h, kind);
    let log = log_option(ed, deadline);
    let rep_re = re!(r"([.+-]+[0-9]+[hdwmy](?:[/ ][-+]?[0-9]+[hdwmy])?)");
    let repeater = time
        .as_deref()
        .filter(|t| !t.trim().is_empty() && stamp::ts_both().is_match(t))
        .and_then(|t| rep_re.captures(t).map(|c| c[1].to_owned()))
        .or_else(|| {
            old.as_deref()
                .and_then(|o| rep_re.captures(o).map(|c| c[1].to_owned()))
        });
    match arg {
        Prefix::U(1) => {
            match &old {
                None => ed.set_msg(if deadline {
                    "Entry had no deadline to remove"
                } else {
                    "Entry was not scheduled"
                }),
                Some(o) => {
                    if let Some(how) = log {
                        let purpose = if deadline {
                            "deldeadline"
                        } else {
                            "delschedule"
                        };
                        request_log(
                            ed,
                            LogRequest {
                                heading: h,
                                purpose,
                                state: None,
                                previous: o.clone(),
                                how,
                            },
                        );
                    }
                    remove_timestamp_with_keyword(ed, h, kind);
                    ed.set_msg(if deadline {
                        "Entry no longer has a deadline."
                    } else {
                        "Entry is no longer scheduled."
                    });
                }
            }
            Ok(())
        }
        Prefix::U(2) => {
            let found = (h..(h + 2).min(ed.line_count())).find_map(|l| {
                let t = line(ed, l);
                kind.time_re()
                    .captures(&t)
                    .map(|c| (l, c.get(0).unwrap().range(), c[1].to_owned()))
            });
            let Some((l, r, inner)) = found else {
                return Err(if deadline {
                    "No deadline information to update"
                } else {
                    "No scheduled information to update"
                }
                .into());
            };
            let old_time = old
                .as_deref()
                .and_then(|o| stamp::time_string_to_tm(o).ok());
            let msg = if deadline {
                "Warn starting from"
            } else {
                "Delay until"
            };
            let o = ReadOpts {
                prompt: Some(msg.into()),
                default: old_time,
                ..ReadOpts::default()
            };
            read_date(ed, o, move |ed, a| {
                let base = old_time.unwrap_or_else(civil::now);
                let n = (a.tm.absolute() - base.absolute()).abs();
                let rpl = re!(r" -[0-9]+[hdwmy]").replace_all(&inner, "").into_owned();
                group(ed, |ed| {
                    replace(ed, l, r, &format!("{} <{rpl} -{n}d>", kind.keyword()))
                });
            });
            Ok(())
        }
        _ => {
            let finish = move |ed: &mut Editor, a: Analysis| {
                group(ed, |ed| {
                    let ts = stamp_text(
                        a.tm.norm(),
                        a.time_given,
                        false,
                        &stamp::end_time_extra(a.end_time.as_deref()),
                    );
                    set_planning(ed, h, Some((kind, &ts)), &[Planning::Closed]);
                    let mut last = ts.clone();
                    if let (Some(o), Some(how)) = (&old, log)
                        && *o != ts
                    {
                        let purpose = if deadline { "redeadline" } else { "reschedule" };
                        request_log(
                            ed,
                            LogRequest {
                                heading: h,
                                purpose,
                                state: Some(ts.clone()),
                                previous: o.clone(),
                                how,
                            },
                        );
                    }
                    if let Some(rep) = &repeater
                        && let Some(pl) = planning_of(ed, h)
                    {
                        let t = line(ed, pl);
                        if let Some(i) = t.find(&format!("{} {ts}", kind.keyword())) {
                            let at = i + kind.keyword().len() + 1 + ts.len() - 1;
                            replace(ed, pl, at..at, &format!(" {rep}"));
                            last.insert_str(last.len() - 1, &format!(" {rep}"));
                        }
                    }
                    ed.set_msg(if deadline {
                        format!("Deadline on {last}")
                    } else {
                        format!("Scheduled to {last}")
                    });
                });
            };
            // Default date and input from the entry's current one.
            let relative = time
                .as_deref()
                .is_none_or(|t| re!(r"^[-+]+[0-9]").is_match(t));
            let existing = if relative {
                let end = syntax::entry_end(&ed.buf, h);
                (h..end).find_map(|l| {
                    let t = line(ed, l);
                    let c = kind.time_re().captures(&t)?;
                    (planning_of(ed, h) == Some(l)).then(|| c[1].to_owned())
                })
            } else {
                None
            };
            let default = existing
                .as_deref()
                .and_then(|t| stamp::time_string_to_tm(t).ok());
            match time {
                Some(t) => {
                    let a = read::analyze(&t, default.unwrap_or_else(civil::now));
                    finish(ed, a);
                }
                None => {
                    let input = existing
                        .as_deref()
                        .and_then(stamp::compact_tod)
                        .unwrap_or_default();
                    let prompt = Some(if deadline { "DEADLINE" } else { "SCHEDULED" }.to_owned());
                    read_date(
                        ed,
                        ReadOpts {
                            prompt,
                            default,
                            input,
                            ..ReadOpts::default()
                        },
                        finish,
                    );
                }
            }
            Ok(())
        }
    }
}

// ---- repeaters ----

/// A line whose timestamps are not timestamp objects: comments,
/// fixed-width lines, keywords, verbatim blocks.
fn verbatim(ed: &Editor, l: usize) -> bool {
    let t = line(ed, l);
    let s = t.trim_start();
    if s == "#" || s.starts_with("# ") || s == ":" || s.starts_with(": ") || s.starts_with("#+") {
        return true;
    }
    ctx::block_at(ed, l)
        .is_some_and(|(name, ..)| ["src", "example", "export", "comment"].contains(&name.as_str()))
}

/// org-at-timestamp-p 'agenda for a timestamp on line `l`.
pub fn agenda_timestamp(ed: &Editor, l: usize) -> bool {
    ctx::at_planning(ed, l) || ctx::at_property(ed, l) || !verbatim(ed, l)
}

/// org-get-repeat: the first repeater of the entry at heading `h`.
pub fn get_repeat(ed: &Editor, h: usize) -> Option<String> {
    let end = syntax::entry_end(&ed.buf, h);
    (h..end).find_map(|l| {
        let t = line(ed, l);
        stamp::repeat_re()
            .captures(&t)
            .filter(|_| agenda_timestamp(ed, l))
            .map(|c| c[1].to_owned())
    })
}

/// org-today: today's absolute day under org-extend-today-until.
pub fn today() -> i64 {
    Tm::from_epoch(civil::now_epoch() - 3600 * options::int("org-extend-today-until", 0)).absolute()
}

/// The timestamp part of org-auto-repeat-maybe, for the TODO module.
///
/// `fn auto_repeat(ed: &mut Editor, h: usize) -> Result<Option<String>, String>`
///
/// When the entry at heading `h` has a non-zero repeater: removes
/// CLOSED, removes a SCHEDULED without repeater, and shifts every active
/// repeating timestamp of the entry by its repeater (`+` from its date,
/// `++` past today/now, `.+` from today/now; org-extend-today-until
/// honored). Returns the message "Entry repeats: SCHEDULED: <...> " (also
/// shown; org-log-post-message), None when nothing repeats. The caller
/// does the rest of org-auto-repeat-maybe first: the TODO state reset
/// (REPEAT_TO_STATE, org-todo-repeat-to-state), LAST_REPEAT and the
/// org-log-repeat note.
pub fn auto_repeat(ed: &mut Editor, h: usize) -> Result<Option<String>, String> {
    let Some(rep) = get_repeat(ed, h) else {
        return Ok(None);
    };
    let digits: String = rep[1..]
        .trim_start_matches('+')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.parse::<i64>().unwrap_or(0) == 0 {
        return Ok(None);
    }
    set_planning(ed, h, None, &[Planning::Closed]);
    if let Some(s) = planning_get(ed, h, Planning::Scheduled)
        && stamp::Timestamp::parse(&s).is_some_and(|t| t.repeater.is_none())
    {
        remove_timestamp_with_keyword(ed, h, Planning::Scheduled);
    }
    let mut msg = String::from("Entry repeats: ");
    let mut l = h;
    while l < syntax::entry_end(&ed.buf, h) {
        let mut from = 0;
        loop {
            let t = line(ed, l);
            let Some(m) = stamp::repeat_re().find_at(&t, from) else {
                break;
            };
            let start = m.start();
            from = m.end();
            if !agenda_timestamp(ed, l) {
                continue;
            }
            let Some(ts) = stamp::Timestamp::parse(&t[start..]) else {
                continue;
            };
            let Some(r) = ts.repeater else { continue };
            let label = if ctx::at_planning(ed, l) {
                re!(r"SCHEDULED:|DEADLINE:")
                    .find_iter(&t[..start])
                    .last()
                    .map_or("Plain:".to_owned(), |k| k.as_str().to_owned())
            } else {
                "Plain:".to_owned()
            };
            let (mut value, mut unit) = (r.value, r.unit);
            if unit == Unit::Week {
                value *= 7;
                unit = Unit::Day;
            }
            if unit == Unit::Hour && ts.start.hour.is_none() {
                return Err(format!(
                    "Cannot repeat in {value} hour(s) because no hour has been set"
                ));
            }
            let mut time = ts.start.tm();
            match r.kind {
                stamp::RepeatType::Restart => {
                    if unit == Unit::Hour {
                        let mins = (civil::now_epoch() - time.epoch()).div_euclid(60);
                        change_at(ed, l, start, mins, Unit::Minute, false)?;
                    } else {
                        change_at(ed, l, start, today() - time.absolute(), Unit::Day, false)?;
                    }
                }
                stamp::RepeatType::CatchUp => {
                    // ponytail: no "Continue?" question every 10 shifts
                    // (prompts are asynchronous); capped instead.
                    let mut n = 0;
                    while n == 0
                        || if unit == Unit::Hour {
                            civil::now_epoch() >= time.epoch()
                        } else {
                            today() >= time.absolute()
                        }
                    {
                        n += 1;
                        if n > 100_000 {
                            return Err("Abort".into());
                        }
                        let new = change_at(ed, l, start, value, unit, false)?;
                        time = stamp::time_string_to_tm(&new)?;
                    }
                    change_at(ed, l, start, -value, unit, false)?;
                }
                stamp::RepeatType::Cumulate => {}
            }
            let new = change_at(ed, l, start, value, unit, true)?;
            from = start + new.len();
            msg.push_str(&format!("{label} {new} "));
        }
        l += 1;
    }
    ed.set_msg(msg.clone());
    Ok(Some(msg))
}

// ---- sparse trees ----

/// org-ts-type: the timestamp kind the date sparse trees match (nil,
/// all, scheduled, deadline, active, inactive, closed); set by
/// org-sparse-tree's `c`.
pub fn set_ts_type(t: Option<&str>) {
    TS_TYPE.with(|x| *x.borrow_mut() = t.map(str::to_owned));
}

fn ts_type() -> Option<String> {
    TS_TYPE.with(|x| x.borrow().clone())
}

/// org-re-timestamp.
pub fn re_timestamp(t: Option<&str>) -> &'static regex::Regex {
    match t {
        Some("all") => stamp::ts_both(),
        Some("active") => re!(&format!(r"<({})>", stamp::TS_INNER)),
        Some("inactive") => re!(&format!(r"\[({})\]", stamp::TS_INNER)),
        Some("scheduled") => Planning::Scheduled.time_re(),
        Some("deadline") => Planning::Deadline.time_re(),
        Some("closed") => Planning::Closed.time_re(),
        _ => re!(r"\b(?:DEADLINE:|SCHEDULED:) *<([^>]+)>"),
    }
}

/// org-occur with a callback: fold to an overview and reveal the lines
/// where `re` matches and `keep(ed, line, group 1)` holds. Returns the
/// number of matches.
pub fn occur(
    ed: &mut Editor,
    re: &regex::Regex,
    keep: impl Fn(&Editor, usize, &str) -> bool,
) -> usize {
    let mut hits = vec![];
    let mut count = 0;
    for l in 0..ed.line_count() {
        let t = line(ed, l);
        for c in re.captures_iter(&t) {
            let g1 = c.get(1).map_or("", |m| m.as_str());
            if keep(ed, l, g1) {
                count += 1;
                if hits.last() != Some(&l) {
                    hits.push(l);
                }
            }
        }
    }
    fold::overview(ed);
    for l in hits {
        fold::show_context_for(ed, l, "occur-tree");
    }
    if !options::bool("org-sparse-tree-open-archived-trees", false) {
        let n = ed.line_count();
        fold::hide_archived(ed, 0, n);
    }
    count
}

fn entry_done(ed: &Editor, l: usize) -> bool {
    let Some(h) = syntax::heading_at_or_before(&ed.buf, l) else {
        return false;
    };
    let st = super::settings(ed);
    syntax::headline(&line(ed, h), &st)
        .and_then(|x| x.todo)
        .is_some_and(|k| st.is_done(&k))
}

/// org-deadline-close-p for an entry line.
pub fn deadline_close_p(ed: &Editor, l: usize, ts: &str, ndays: Option<i64>) -> bool {
    let nd = ndays.unwrap_or_else(|| stamp::get_wdays(ts, false, false));
    stamp::timestamp_to_now(ts, false).is_ok_and(|d| d <= nd) && !entry_done(ed, l)
}

fn check_deadlines(ed: &mut Editor, arg: Prefix) {
    let days = match arg {
        Prefix::U(1) => 100_000,
        Prefix::None => options::int("org-deadline-warning-days", 14).abs(),
        a => a.value(),
    };
    let n = occur(ed, Planning::Deadline.time_re(), |ed, l, ts| {
        deadline_close_p(ed, l, ts, Some(days))
    });
    ed.set_msg(format!("{n} deadlines past-due or due within {days} days"));
}

fn check_dates(
    ed: &mut Editor,
    single: Option<&str>,
    range: Option<(&str, &str)>,
    before: bool,
) -> usize {
    let ty = ts_type();
    let re = re_timestamp(ty.as_deref());
    let plain = matches!(ty.as_deref(), Some("active" | "inactive" | "all"));
    let tm = |s: &str| stamp::time_string_to_tm(s).ok().map(Tm::norm);
    let d = single.and_then(tm);
    let (s, e) = range.map_or((None, None), |(s, e)| (tm(s), tm(e)));
    occur(ed, re, |ed, l, m| {
        let context = if plain {
            !verbatim(ed, l)
        } else {
            ctx::at_planning(ed, l)
        };
        let Some(t) = tm(m) else { return false };
        context
            && match (d, s, e) {
                (Some(d), ..) => (t < d) == before,
                (_, Some(s), Some(e)) => t >= s && t < e,
                _ => false,
            }
    })
}
