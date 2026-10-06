//! The relative and countdown timers: a port of org-timer.el.
//!
//! The timer is session state; its value shows in Fred's status line
//! ([`mode_line`]) and the app loop's tick ends a countdown with
//! org-notify (org-timer--run-countdown-timer's timer).

use super::time::civil;
use super::{Prefix, line, options};
use crate::editor::Editor;
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Countdown {
    #[default]
    None,
    /// Ends at this time.
    Running(i64),
    Paused,
}

#[derive(Default)]
struct Timer {
    /// org-timer-start-time and org-timer-pause-time.
    start: Option<i64>,
    pause: Option<i64>,
    countdown: Countdown,
    /// org-timer-countdown-timer-title.
    title: String,
    /// org-timer-mode-line-string, None when the timer display is off.
    mode_line: Option<String>,
    shown: i64,
}

thread_local! {
    static T: RefCell<Timer> = RefCell::new(Timer::default());
}

fn with<R>(f: impl FnOnce(&mut Timer) -> R) -> R {
    T.with(|t| f(&mut t.borrow_mut()))
}

fn now() -> i64 {
    civil::now_epoch()
}

/// org-timer-secs-to-hms.
pub fn secs_to_hms(s: i64) -> String {
    let sign = if s < 0 { "-" } else { "" };
    let s = s.abs();
    format!("{sign}{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// org-timer-hms-to-secs: 0 when `hms` has no h:mm:ss.
pub fn hms_to_secs(hms: &str) -> i64 {
    let Some(c) = crate::org_re!(r"([-+]?[0-9]+):([0-9]{2}):([0-9]{2})").captures(hms) else {
        return 0;
    };
    let n = |i: usize| {
        c[i].trim_start_matches(['-', '+'])
            .parse::<i64>()
            .unwrap_or(0)
    };
    let secs = n(3) + 60 * (n(2) + 60 * n(1));
    if c[1].starts_with('-') { -secs } else { secs }
}

/// org-timer-fix-incomplete: `5` is 0:00:05, `1:30` 0:01:30.
pub fn fix_incomplete(hms: &str) -> Result<String, String> {
    let c = crate::org_re!(r"(?:([0-9]+:)?([0-9]+:))?([0-9]+)")
        .captures(hms)
        .ok_or_else(|| format!("Cannot parse HMS string \"{hms}\""))?;
    let n = |i: usize| {
        c.get(i).map_or(0, |m| {
            m.as_str().trim_end_matches(':').parse::<i64>().unwrap_or(0)
        })
    };
    let m = c.get(0).unwrap();
    Ok(format!(
        "{}{}:{:02}:{:02}{}",
        &hms[..m.start()],
        n(1),
        n(2),
        n(3),
        &hms[m.end()..]
    ))
}

/// org-timer-value-string.
pub fn value_string() -> String {
    let (start, pause, cd) = with(|t| (t.start, t.pause, t.countdown));
    let time = pause.unwrap_or_else(now) - start.unwrap_or_else(now);
    let v = if cd == Countdown::None { time } else { -time };
    options::string("org-timer-format", "%s ").replacen("%s", &secs_to_hms(v.abs()), 1)
}

fn display_on() -> bool {
    let d = options::string("org-timer-display", "mode-line");
    d == "mode-line" || d == "both"
}

/// org-timer-update-mode-line.
fn update_mode_line() {
    if with(|t| t.pause.is_some()) {
        return;
    }
    let v = value_string();
    let v = v.strip_suffix(' ').unwrap_or(&v);
    with(|t| t.mode_line = Some(format!(" <{v}>")));
}

/// org-timer-set-mode-line.
fn set_mode_line(value: &str) {
    match value {
        "off" => with(|t| t.mode_line = None),
        "on" if display_on() => update_mode_line(),
        _ => {}
    }
}

/// org-timer-mode-line-string, for Fred's status line.
pub fn mode_line() -> Option<String> {
    let (shown, paused) = with(|t| (t.mode_line.clone(), t.pause.is_some()));
    shown.as_ref()?;
    if !paused {
        update_mode_line();
    }
    with(|t| t.mode_line.clone())
}

/// Forget the timer (tests).
pub fn reset() {
    with(|t| *t = Timer::default());
}

/// org-timer-start: OFFSET as seconds (None: now).
fn start_at(ed: &mut Editor, delta: Option<i64>) {
    with(|t| {
        t.start = Some(now() - delta.unwrap_or(0));
        t.pause = None;
    });
    set_mode_line("on");
    let start = with(|t| t.start.unwrap());
    ed.set_msg(format!(
        "Timer start time set to {}, current value is {}",
        civil::format("%T", civil::Tm::from_epoch(start)),
        secs_to_hms(delta.unwrap_or(0))
    ));
}

/// org-timer-start.
pub fn start(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if arg == Prefix::U(2) {
        return change_times_in_region(ed, super::take_call_arg());
    }
    if with(|t| t.countdown != Countdown::None) {
        return Err("Countdown timer is running.  Cancel first".into());
    }
    match arg {
        Prefix::None => {
            start_at(ed, None);
            Ok(())
        }
        Prefix::Num(n) => {
            start_at(ed, Some(n));
            Ok(())
        }
        _ => {
            if let Some(s) = super::take_call_arg() {
                start_at(ed, Some(hms_to_secs(&fix_incomplete(&s)?)));
                return Ok(());
            }
            let t = line(ed, ed.cur.line);
            let def = crate::org_re!(r"[-+]?[0-9]+:[0-9]{2}:[0-9]{2}")
                .find_iter(&t)
                .find(|m| m.start() <= ed.cur.byte && ed.cur.byte <= m.end())
                .map_or("0:00:00".to_owned(), |m| m.as_str().to_owned());
            super::read(
                ed,
                &format!("Restart timer with offset [{def}]: "),
                "",
                move |ed, s| {
                    let s = if s.trim().is_empty() { def } else { s };
                    match fix_incomplete(&s) {
                        Ok(f) => start_at(ed, Some(hms_to_secs(&f))),
                        Err(e) => ed.set_err(e),
                    }
                },
            );
            Ok(())
        }
    }
}

/// org-timer-pause-or-continue.
pub fn pause_or_continue(ed: &mut Editor, stop_it: bool) -> Result<(), String> {
    if stop_it {
        return stop(ed);
    }
    let (start, pause, cd) = with(|t| (t.start, t.pause, t.countdown));
    let Some(start) = start else {
        return Err("No timer is running".into());
    };
    if let Some(pause) = pause {
        with(|t| {
            if cd != Countdown::None {
                let left = start - pause;
                t.countdown = Countdown::Running(now() + left);
                t.start = Some(now() + left);
            } else {
                t.start = Some(now() - (pause - start));
            }
            t.pause = None;
        });
        set_mode_line("on");
        ed.set_msg(format!("Timer continues at {}", value_string()));
    } else {
        with(|t| {
            if matches!(t.countdown, Countdown::Running(_)) {
                t.countdown = Countdown::Paused;
            }
            t.pause = Some(now());
        });
        ed.set_msg(format!("Timer paused at {}", value_string()));
    }
    Ok(())
}

/// org-timer-stop.
pub fn stop(ed: &mut Editor) -> Result<(), String> {
    if with(|t| t.start.is_none()) {
        return Err("No timer running".into());
    }
    with(|t| {
        t.start = None;
        t.pause = None;
        t.countdown = Countdown::None;
    });
    set_mode_line("off");
    ed.set_msg("Timer stopped");
    Ok(())
}

/// Insert `s` at the cursor, which moves after it.
fn insert(ed: &mut Editor, s: &str) {
    let (l, b) = (ed.cur.line, ed.cur.byte);
    let t = line(ed, l);
    let b = b.min(t.len());
    super::set_line(ed, l, &format!("{}{s}{}", &t[..b], &t[b..]));
    ed.set_cursor(l, b + s.len());
}

/// org-timer: the timer string (starting the timer first if needed).
pub fn timer_string(ed: &mut Editor, restart: bool) -> String {
    if restart || with(|t| t.start.is_none()) {
        start_at(ed, None);
    }
    value_string()
}

/// org-timer-change-times-in-region.
fn change_times_in_region(ed: &mut Editor, delta: Option<String>) -> Result<(), String> {
    let (lo, hi) = super::region(ed).ok_or("The mark is not set now, so there is no region")?;
    let go = move |ed: &mut Editor, delta: String| -> Result<(), String> {
        let re = crate::org_re!(r"[-+]?[0-9]+:[0-9]{2}:[0-9]{2}");
        let mut delta = delta;
        if delta.trim().is_empty()
            && let Some(m) =
                (lo..=hi).find_map(|l| re.find(&line(ed, l)).map(|m| m.as_str().to_owned()))
        {
            delta = match m.strip_prefix('-') {
                Some(x) => x.to_owned(),
                None => format!("-{m}"),
            };
        }
        let d = hms_to_secs(&fix_incomplete(&delta)?);
        if d == 0 {
            return Err("No change".into());
        }
        for l in lo..=hi.min(ed.line_count() - 1) {
            let t = line(ed, l);
            let new = re.replace_all(&t, |c: &regex::Captures| {
                secs_to_hms(hms_to_secs(&c[0]) + d)
            });
            super::set_line(ed, l, &new);
        }
        Ok(())
    };
    match delta {
        Some(d) => go(ed, d),
        None => {
            super::read(
                ed,
                "Enter time difference like \"-1:08:26\".  Default is first time to zero: ",
                "",
                move |ed, d| {
                    ed.undo.begin(ed.cur.pos());
                    if let Err(e) = go(ed, d) {
                        ed.set_err(e);
                    }
                    ed.undo.end(ed.cur.pos());
                },
            );
            Ok(())
        }
    }
}

/// org-timer-item.
pub fn item(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    use super::list::structure::{FULL_ITEM, in_item, insert_item, list_struct, write_struct};
    let restart = !arg.is_none();
    let in_list = super::list::with_buf(ed, |b| {
        let pos = b.pt;
        let Some(itemp) = in_item(b) else {
            return Ok(None);
        };
        b.goto(itemp);
        let timer = super::list::at_item_timer(b);
        b.goto(pos);
        Ok(Some(timer))
    })?;
    match in_list {
        Some(true) => {
            let s = format!("{}:: ", timer_string(ed, restart));
            super::list::with_buf(ed, |b| {
                let pos = b.pt;
                let itemp = in_item(b).unwrap_or(pos);
                b.goto(itemp);
                let mut st = list_struct(b);
                let prevs = st.prevs();
                insert_item(b, pos, &mut st, &prevs, false, &s, None);
                let parents = st.parents();
                write_struct(b, &mut st, &parents, None);
                b.forward_line(0);
                if let Some(e) = b.looking_at(&FULL_ITEM).and_then(|c| c.end(0)) {
                    b.goto(e);
                }
                Ok(())
            })
        }
        Some(false) => Err("This is not a timer list".into()),
        None => {
            let l = ed.cur.line;
            ed.set_cursor(l, 0);
            let _ = super::editing::indent_line(ed, l);
            let t = line(ed, l);
            ed.set_cursor(l, t.len() - t.trim_start().len());
            insert(ed, "- ");
            let s = timer_string(ed, restart);
            insert(ed, &s);
            insert(ed, ":: ");
            Ok(())
        }
    }
}

/// org-timer-show-remaining-time.
pub fn show_remaining_time(ed: &mut Editor) {
    let left = with(|t| match t.countdown {
        Countdown::None => None,
        Countdown::Running(end) => Some(end - now()),
        Countdown::Paused => Some(t.start.unwrap_or(0) - t.pause.unwrap_or(0)),
    });
    ed.set_msg(match left {
        None => "No timer set".into(),
        Some(s) => format!(
            "{} minute(s) {} seconds left before next time out",
            s / 60,
            s % 60
        ),
    });
}

/// org-timer--get-timer-title.
fn timer_title(ed: &Editor) -> String {
    let buffer = || {
        ed.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "[No Name]".into(), |f| f.to_string_lossy().into_owned())
    };
    if ed.org.is_none() {
        return buffer();
    }
    super::fold::back_to_heading(ed, ed.cur.line)
        .map(|h| line(ed, h).trim_start_matches('*').trim().to_owned())
        .unwrap_or_else(buffer)
}

/// Set the countdown to `minutes` (org-timer-set-timer after the prompt).
fn set_countdown(ed: &mut Editor, minutes: String, replace: bool) {
    let minutes =
        if minutes.trim().bytes().all(|b| b.is_ascii_digit()) && !minutes.trim().is_empty() {
            format!("{}:00", minutes.trim())
        } else {
            minutes
        };
    if !minutes.bytes().any(|b| b.is_ascii_digit()) {
        return show_remaining_time(ed);
    }
    let secs = match fix_incomplete(&minutes) {
        Ok(f) => hms_to_secs(&f),
        Err(e) => return ed.set_err(e),
    };
    let title = timer_title(ed);
    let set = move |ed: &mut Editor| {
        with(|t| {
            t.title = title;
            t.countdown = Countdown::Running(now() + secs);
            t.start = Some(now() + secs);
            t.pause = None;
        });
        set_mode_line("on");
        let _ = ed;
    };
    if with(|t| t.countdown != Countdown::None) && !replace {
        super::menu(
            ed,
            "Replace current timer? ",
            vec![("y".into(), "yes".into()), ("n".into(), "no".into())],
            move |ed, k| {
                if k == "y" {
                    set(ed)
                } else {
                    ed.set_msg("No timer set")
                }
            },
        );
    } else {
        set(ed);
    }
}

/// org-timer-set-timer.
pub fn set_timer(ed: &mut Editor, opt: Prefix) -> Result<(), String> {
    if with(|t| t.start.is_some() && t.countdown == Countdown::None) {
        return Err("Relative timer is running.  Stop first".into());
    }
    let default = options::string("org-timer-default-timer", "0");
    let effort = if ed.org.is_some() {
        let p = options::string("org-effort-property", "Effort");
        super::fold::back_to_heading(ed, ed.cur.line)
            .and_then(|h| super::props::get(ed, Some(h), &p, super::props::Inherit::No))
            .filter(|e| !e.trim().is_empty())
            .and_then(|e| super::time::duration::to_minutes(&e, false).ok())
            .map(|m| (m.floor() as i64).to_string())
    } else {
        None
    };
    let replace = opt == Prefix::U(2);
    let minutes = match opt {
        Prefix::Num(n) => Some(n.to_string()),
        _ if opt != Prefix::U(3) && effort.is_some() => effort,
        Prefix::U(1) | Prefix::U(2) => Some(default.clone()),
        _ => super::take_call_arg(),
    };
    match minutes {
        Some(m) => set_countdown(ed, m, replace),
        None => {
            let init = if default == "0" {
                String::new()
            } else {
                default
            };
            super::read(
                ed,
                "How much time left? (minutes or h:mm:ss) ",
                &init,
                move |ed, m| set_countdown(ed, m, replace),
            );
        }
    }
    Ok(())
}

/// The app loop's tick: end a finished countdown (org-notify), refresh
/// the mode line every second. True when the status line changed.
pub fn tick(ed: &mut Editor) -> bool {
    let (cd, on) = with(|t| (t.countdown, t.mode_line.is_some()));
    if let Countdown::Running(end) = cd
        && now() >= end
    {
        let title = with(|t| t.title.clone());
        super::clock::notify(ed, &format!("{title}: time out"), None);
        set_mode_line("off");
        with(|t| {
            t.countdown = Countdown::None;
            t.start = None;
        });
        return true;
    }
    if on && with(|t| t.pause.is_none() && t.start.is_some()) {
        let n = now();
        if with(|t| std::mem::replace(&mut t.shown, n) != n) {
            update_mode_line();
            return true;
        }
    }
    false
}

/// This module's interactive commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-timer-start" => start(ed, arg),
        "org-timer-pause-or-continue" => pause_or_continue(ed, !arg.is_none()),
        "org-timer-stop" => stop(ed),
        "org-timer" => {
            if arg == Prefix::U(2) {
                return Some(start(ed, arg));
            }
            let s = timer_string(ed, arg == Prefix::U(1));
            insert(ed, &s);
            Ok(())
        }
        "org-timer-change-times-in-region" => change_times_in_region(ed, super::take_call_arg()),
        "org-timer-item" => item(ed, arg),
        "org-timer-set-timer" => set_timer(ed, arg),
        "org-timer-show-remaining-time" => {
            show_remaining_time(ed);
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::tests::org;
    use crate::org::{call_with, run};

    fn at(secs: i64) {
        civil::set_now(Some(civil::Tm::from_epoch(secs)));
    }

    #[test]
    fn hms_conversions() {
        assert_eq!(secs_to_hms(0), "0:00:00");
        assert_eq!(secs_to_hms(3661), "1:01:01");
        assert_eq!(secs_to_hms(-3661), "-1:01:01");
        assert_eq!(secs_to_hms(360_000), "100:00:00");
        assert_eq!(hms_to_secs("-1:01:01"), -3661);
        assert_eq!(hms_to_secs("nope"), 0);
        assert_eq!(fix_incomplete("1:00:00").unwrap(), "1:00:00");
        assert_eq!(fix_incomplete("1:00").unwrap(), "0:01:00");
        assert_eq!(fix_incomplete("1").unwrap(), "0:00:01");
    }

    #[test]
    fn relative_timer_start_pause_stop() {
        reset();
        at(1_000_000);
        let mut e = org("- ", "A");
        run(&mut e, "org-timer-start", Prefix::None);
        assert!(
            e.msg
                .as_ref()
                .unwrap()
                .0
                .ends_with("current value is 0:00:00")
        );
        at(1_000_065);
        run(&mut e, "org-timer", Prefix::None);
        assert_eq!(e.buf.text(), "- 0:01:05 ");
        assert_eq!(mode_line(), Some(" <0:01:05>".into()));
        run(&mut e, "org-timer-pause-or-continue", Prefix::None);
        assert_eq!(e.msg.as_ref().unwrap().0, "Timer paused at 0:01:05 ");
        at(1_000_200);
        assert_eq!(value_string(), "0:01:05 ");
        run(&mut e, "org-timer-pause-or-continue", Prefix::None);
        assert_eq!(e.msg.as_ref().unwrap().0, "Timer continues at 0:01:05 ");
        at(1_000_210);
        assert_eq!(value_string(), "0:01:15 ");
        run(&mut e, "org-timer-set-timer", Prefix::Num(5));
        assert!(
            e.msg
                .as_ref()
                .unwrap()
                .0
                .ends_with("Relative timer is running.  Stop first")
        );
        run(&mut e, "org-timer-stop", Prefix::None);
        assert_eq!(e.msg.as_ref().unwrap().0, "Timer stopped");
        assert!(mode_line().is_none());
        run(&mut e, "org-timer-stop", Prefix::None);
        assert!(e.msg.as_ref().unwrap().0.ends_with("No timer running"));
        // A numeric offset.
        run(&mut e, "org-timer-start", Prefix::Num(70));
        assert_eq!(value_string(), "0:01:10 ");
        reset();
        civil::set_now(None);
    }

    #[test]
    fn countdown_ends_with_a_notification() {
        reset();
        at(2_000_000);
        let mut e = org("* Task\n:PROPERTIES:\n:Effort: 0:02\n:END:", "");
        run(&mut e, "org-timer-set-timer", Prefix::None);
        assert_eq!(value_string(), "0:02:00 ");
        run(&mut e, "org-timer-start", Prefix::None);
        assert!(
            e.msg
                .as_ref()
                .unwrap()
                .0
                .ends_with("Countdown timer is running.  Cancel first")
        );
        at(2_000_030);
        run(&mut e, "org-timer-show-remaining-time", Prefix::None);
        assert_eq!(
            e.msg.as_ref().unwrap().0,
            "1 minute(s) 30 seconds left before next time out"
        );
        at(2_000_121);
        assert!(tick(&mut e));
        assert_eq!(e.msg.as_ref().unwrap().0, "Task: time out");
        assert!(mode_line().is_none());
        // Without effort, the duration is typed.
        let mut e = org("text", "");
        call_with(
            &mut e,
            "org-timer-set-timer",
            Prefix::None,
            "1:00:00".into(),
        )
        .unwrap();
        assert_eq!(value_string(), "1:00:00 ");
        reset();
        civil::set_now(None);
    }

    #[test]
    fn timer_items_and_region_changes() {
        reset();
        at(3_000_000);
        let mut e = org("x", "");
        run(&mut e, "org-timer-item", Prefix::None);
        assert_eq!(e.buf.text(), "- 0:00:00 :: x");
        let mut e = org("- 0:00:05 :: a", "A");
        at(3_000_010);
        run(&mut e, "org-insert-item", Prefix::None);
        assert_eq!(e.buf.text(), "- 0:00:05 :: a\n- 0:00:10 :: ");
        let mut e = org("- 0:00:05 :: a\n- 1:00:00 :: b", "");
        e.org_region = Some((0, 1));
        call_with(
            &mut e,
            "org-timer-change-times-in-region",
            Prefix::None,
            String::new(),
        )
        .unwrap();
        assert_eq!(e.buf.text(), "- 0:00:00 :: a\n- 0:59:55 :: b");
        reset();
        civil::set_now(None);
    }
}
