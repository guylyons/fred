//! Clocking work time: a port of org-clock.el.
//!
//! The running clock, the clock history and the default and interrupted
//! tasks are session state. Fred has no markers: a task is found again by
//! its heading line (then its text), and the running clock by its open
//! `CLOCK: [start]` line.
//!
//! Fred adaptations (docs/org-port-notes.md, "Clocking"):
//! - The mode line clock (org-mode-line-string) is shown in Fred's status
//!   line ([`mode_line`]); a tick in the app loop refreshes it, raises the
//!   effort overrun notification and checks idleness.
//! - org-clock-display's overlays are per-heading display text the
//!   renderer draws over the heading ([`display`]).
//! - The task selection buffer is a key menu; the resolution splash is the
//!   menu's own labels.
//! - org-clock-persist saves to Fred's state directory whenever the clock
//!   changes, and resumes at the first tick in an Org buffer.
//! - Lisp-valued options (functions in org-clock-in-switch-to-state,
//!   org-clock-heading-function, scopes and formatters) are not evaluated.

pub mod sum;
pub mod table;
#[cfg(test)]
mod tests;

use super::time::civil::{self, Tm};
use super::time::stamp;
use super::{Prefix, fold, line, options, props, sexp, syntax, todo};
use crate::editor::Editor;
use crate::session::Session;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// A heading in an Org file, found again by its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub file: Option<PathBuf>,
    pub line: usize,
    /// The heading line's text.
    pub heading: String,
    /// For the selection menu: category and `org-get-heading 'notags`.
    pub category: String,
    pub label: String,
}

/// The running clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub task: Marker,
    pub clock_line: usize,
    /// The start timestamp text, `[2026-10-06 Tue 10:00]`.
    pub stamp: String,
    pub start: i64,
    /// org-clock-heading: the task as the mode line shows it.
    pub heading: String,
    /// org-clock-effort.
    pub effort: Option<String>,
    /// org-clock-total-time: minutes clocked before this clock.
    pub total: i64,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    history: Vec<Marker>,
    default_task: Option<Marker>,
    interrupted: Option<Marker>,
    /// org-clock-current-task.
    current_task: Option<String>,
    /// org-clock-out-time.
    out_time: Option<i64>,
    /// org-clock-leftover-time.
    leftover: Option<i64>,
    notified: bool,
    overrun: bool,
    has_been_used: bool,
    loaded: bool,
    /// org-clock-clocking-in, org-clock-resolving-clocks.
    clocking_in: bool,
    resolving: bool,
    /// org-clock-out-when-done is bound to nil (clock-out switching state).
    no_out_when_done: bool,
    /// org-clock-auto-clockout added to org-clock-in-hook.
    auto_clockout: Option<bool>,
    /// The minute and second the status line last showed.
    shown: (i64, i64),
    last_idle_check: i64,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

/// The running clock, if any (org-clocking-p).
pub fn running() -> Option<Running> {
    with(|s| s.running.clone())
}

/// org-clock-history, most recent first.
pub fn history() -> Vec<Marker> {
    with(|s| s.history.clone())
}

/// org-clock-out-time.
pub fn out_time() -> Option<i64> {
    with(|s| s.out_time)
}

/// Forget all clock state (tests).
pub fn reset() {
    with(|s| *s = State::default());
}

// ---- files and markers ----

/// The absolute file of a buffer.
pub fn file_of(ed: &Editor) -> Option<PathBuf> {
    ed.path
        .as_deref()
        .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()))
}

fn same_file(ed: &Editor, f: &Option<PathBuf>) -> bool {
    file_of(ed) == *f
}

/// The title of a heading line (org-get-heading t t t t).
pub fn title(line: &str, st: &syntax::Settings) -> String {
    syntax::headline(line, st)
        .map(|h| h.title(line).trim().to_owned())
        .unwrap_or_default()
}

/// org-get-heading 'notags: keyword, priority cookie and title.
fn notags(line: &str, st: &syntax::Settings) -> String {
    let Some(h) = syntax::headline(line, st) else {
        return String::new();
    };
    let mut parts = vec![];
    if let Some(r) = h.todo_range.clone() {
        parts.push(line[r].to_owned());
    }
    if let Some(r) = h.priority_range.clone() {
        parts.push(line[r].to_owned());
    }
    let end = h.tags_range.as_ref().map_or(line.len(), |r| r.start);
    let start = h
        .priority_range
        .as_ref()
        .or(h.todo_range.as_ref())
        .map_or(h.level + 1, |r| r.end);
    parts.push(line[start.min(end)..end].trim().to_owned());
    parts.retain(|p| !p.is_empty());
    parts.join(" ")
}

/// A marker for heading `h` of `ed`.
pub fn marker(ed: &Editor, h: usize) -> Marker {
    let st = super::settings(ed);
    let text = line(ed, h);
    let lines: Vec<String> = super::lines(ed, 0..ed.line_count());
    let doc = props::Doc::new(&lines, &st, ed.path.as_deref());
    Marker {
        file: file_of(ed),
        line: h,
        heading: text.clone(),
        category: doc.category(Some(h)),
        label: notags(&text, &st),
    }
}

/// Find a marker's heading in `ed`: its line if unchanged, else the
/// nearest line with the same text, else the same title.
pub fn locate(ed: &Editor, m: &Marker) -> Option<usize> {
    let n = ed.line_count();
    if m.line < n && line(ed, m.line) == m.heading {
        return Some(m.line);
    }
    let near = |v: Vec<usize>| v.into_iter().min_by_key(|i| i.abs_diff(m.line));
    if let Some(l) = near((0..n).filter(|&i| line(ed, i) == m.heading).collect()) {
        return Some(l);
    }
    let st = super::settings(ed);
    let t = title(&m.heading, &st);
    near(
        (0..n)
            .filter(|&i| syntax::level(&line(ed, i)).is_some() && title(&line(ed, i), &st) == t)
            .collect(),
    )
}

fn is_open_clock(t: &str, stamp: &str) -> bool {
    t.trim_start()
        .strip_prefix("CLOCK:")
        .is_some_and(|x| x.trim() == stamp)
}

/// The running clock's CLOCK line in `ed`.
pub fn locate_clock(ed: &Editor, r: &Running) -> Option<usize> {
    let n = ed.line_count();
    if r.clock_line < n && is_open_clock(&line(ed, r.clock_line), &r.stamp) {
        return Some(r.clock_line);
    }
    (0..n)
        .filter(|&i| is_open_clock(&line(ed, i), &r.stamp))
        .min_by_key(|i| i.abs_diff(r.clock_line))
}

/// The running clock's (heading, start) in the lines of `file`, for
/// org-clock-report-include-clocking-task.
pub fn clocking_in<L: syntax::Lines + ?Sized>(b: &L, file: Option<&Path>) -> Option<(usize, i64)> {
    let r = running()?;
    let f = file.map(|p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
    if f != r.task.file {
        return None;
    }
    let n = b.n_lines();
    let l = if r.clock_line < n && is_open_clock(&b.line_text(r.clock_line), &r.stamp) {
        r.clock_line
    } else {
        (0..n).find(|&i| is_open_clock(&b.line_text(i), &r.stamp))?
    };
    Some((syntax::heading_at_or_before(b, l)?, r.start))
}

// ---- running work in the clock's buffer ----

/// Queue `f` after the effect already pending in `ed`.
fn push_effect(ed: &mut Editor, f: impl FnOnce(&mut Session) + Send + 'static) {
    let prev = ed.pending_effect.take();
    super::effect(ed, move |s| {
        if let Some(p) = prev {
            s.perform(p);
        }
        f(s);
    });
}

/// Run `f` on the buffer visiting `p` as one undo step; its message and
/// effects reach the session.
pub fn run_in(s: &mut Session, p: &Path, f: impl FnOnce(&mut Editor)) {
    let r = s.org_with_file(p, |e| {
        let cur = (e.cur.line, e.cur.byte);
        e.undo.begin(e.cur.pos());
        f(e);
        e.undo.end(e.cur.pos());
        e.clamp_cursor();
        (e.msg.take(), e.pending_effect.take(), cur)
    });
    match r {
        Ok((msg, eff, _)) => {
            if let Some(m) = msg {
                s.ed.msg = Some(m);
            }
            if let Some(e) = eff {
                s.perform(e);
            }
        }
        Err(e) => s.ed.set_err(e),
    }
}

/// Run `f` in the buffer of `file`: this one, or another through the
/// session (with-current-buffer on the clock's marker).
fn in_file(ed: &mut Editor, file: Option<PathBuf>, f: impl FnOnce(&mut Editor) + Send + 'static) {
    match file {
        Some(p) if !same_file(ed, &Some(p.clone())) => push_effect(ed, move |s| run_in(s, &p, f)),
        // ponytail: a clock in an unnamed buffer is taken to be this one.
        _ => f(ed),
    }
}

// ---- clock drawer ----

/// What org-clock-into-drawer returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntoDrawer {
    No,
    Count(i64),
    Name(String),
}

/// org-clock-into-drawer: the option, overruled by CLOCK_INTO_DRAWER.
pub fn into_drawer(ed: &Editor, h: usize) -> IntoDrawer {
    let st = super::settings(ed);
    let p = props::Doc::new(&ed.buf, &st, ed.path.as_deref()).get(
        Some(h),
        "CLOCK_INTO_DRAWER",
        props::Inherit::Yes,
        true,
    );
    let log = || todo::log_into_drawer(ed, h);
    match p.as_deref() {
        Some("nil") => return IntoDrawer::No,
        Some("t") => return IntoDrawer::Name(log().unwrap_or_else(|| "LOGBOOK".into())),
        Some(p) if !p.trim().is_empty() => {
            return match p.parse::<i64>() {
                Ok(n) if p.bytes().all(|b| b.is_ascii_digit()) => IntoDrawer::Count(n),
                _ => IntoDrawer::Name(p.to_owned()),
            };
        }
        _ => {}
    }
    match sexp::option("org-clock-into-drawer") {
        Some(sexp::Sexp::Str(s)) if !s.trim().is_empty() => IntoDrawer::Name(s),
        Some(sexp::Sexp::Int(n)) => IntoDrawer::Count(n),
        Some(sexp::Sexp::Nil) => IntoDrawer::No,
        _ => IntoDrawer::Name(log().unwrap_or_else(|| "LOGBOOK".into())),
    }
}

/// org-clock-drawer-name.
pub fn drawer_name(ed: &Editor, h: usize) -> Option<String> {
    match into_drawer(ed, h) {
        IntoDrawer::Count(_) => {
            Some(todo::log_into_drawer(ed, h).unwrap_or_else(|| "LOGBOOK".into()))
        }
        IntoDrawer::Name(n) => Some(n),
        IntoDrawer::No => None,
    }
}

/// A drawer `:NAME:` at line `l`: its :END: line.
fn drawer_end(ed: &Editor, l: usize) -> Option<usize> {
    let t = line(ed, l);
    let t = t.trim();
    if !(t.len() > 2 && t.starts_with(':') && t.ends_with(':')) || t.eq_ignore_ascii_case(":END:") {
        return None;
    }
    (l + 1..ed.line_count())
        .take_while(|&i| syntax::level(&line(ed, i)).is_none())
        .find(|&i| line(ed, i).trim().eq_ignore_ascii_case(":END:"))
}

/// org-remove-empty-drawer-at: delete the drawer at line `l` if it has
/// no contents.
pub fn remove_empty_drawer_at(ed: &mut Editor, l: usize) {
    if l < ed.line_count()
        && let Some(e) = drawer_end(ed, l)
        && e == l + 1
    {
        super::delete_lines(ed, l, 2);
    }
}

/// org-clock-remove-empty-clock-drawer for the subtree at `h`.
fn remove_empty_clock_drawers(ed: &mut Editor, h: usize) {
    let mut l = h;
    while l < syntax::subtree_end(&ed.buf, h) {
        if syntax::level(&line(ed, l)).is_some()
            && let Some(name) = drawer_name(ed, l)
        {
            let end = syntax::entry_end(&ed.buf, l);
            let open = format!(":{}:", name.to_ascii_uppercase());
            let mut i = l + 1;
            while i < end.min(ed.line_count()) {
                if line(ed, i).trim().eq_ignore_ascii_case(&open) {
                    let n = ed.line_count();
                    remove_empty_drawer_at(ed, i);
                    if ed.line_count() < n {
                        continue;
                    }
                }
                i += 1;
            }
        }
        l += 1;
    }
}

/// Where org-clock-find-position puts point.
#[derive(Debug, PartialEq, Eq)]
enum Pos {
    /// An unclosed clock line to resume.
    Open(usize),
    /// Insert the new clock line at this line.
    Insert(usize),
}

fn clock_element(ed: &Editor, l: usize) -> bool {
    line(ed, l).trim_start().starts_with("CLOCK:") && !super::ctx::in_block(ed, l)
}

/// org-clock-find-position.
fn find_position(ed: &mut Editor, h: usize, find_unclosed: bool) -> Pos {
    let end = syntax::entry_end(&ed.buf, h);
    let into = into_drawer(ed, h);
    let drawer = drawer_name(ed, h);
    let reversed = super::settings(ed).opt_bool("org-log-states-order-reversed", true);
    if find_unclosed {
        let open = crate::org_re!(
            r"^[ \t]*CLOCK: \[([0-9]{4}-[0-9]{2}-[0-9]{2} *[^\s\]]+ +[012][0-9]:[0-5][0-9])\][ \t]*$"
        );
        if let Some(l) = (h + 1..end).find(|&l| open.is_match(&line(ed, l)) && clock_element(ed, l))
        {
            return Pos::Open(l);
        }
    }
    if let Some(d) = &drawer {
        let open = format!(":{}:", d.to_ascii_uppercase());
        for l in h + 1..end {
            if line(ed, l).trim().eq_ignore_ascii_case(&open)
                && let Some(e) = drawer_end(ed, l)
            {
                return Pos::Insert(if !reversed { e } else { l + 1 });
            }
        }
    }
    let positions: Vec<usize> = (h + 1..end).filter(|&l| clock_element(ed, l)).collect();
    if positions.is_empty() {
        let m = todo::end_of_meta_data(ed, h, false);
        let wrap = match &into {
            IntoDrawer::No => false,
            IntoDrawer::Count(n) => *n < 2,
            IntoDrawer::Name(_) => true,
        };
        if wrap && let Some(d) = drawer {
            super::insert_lines(ed, m, &[format!(":{d}:"), ":END:".into()]);
            let _ = super::editing::indent_line(ed, m);
            let _ = super::editing::indent_line(ed, m + 1);
            fold::region(ed, m + 1, m + 1, true, fold::Spec::Drawer);
            return Pos::Insert(m + 1);
        }
        return Pos::Insert(m);
    }
    let wrap = match &into {
        IntoDrawer::Count(n) => positions.len() as i64 + 1 >= *n,
        _ => drawer.is_some(),
    };
    if wrap && let Some(d) = drawer {
        let m = todo::end_of_meta_data(ed, h, false);
        // Collect the clocks (last first, as upstream) with their blank
        // lines before them.
        let mut clocks = vec![];
        for &p in positions.iter().rev() {
            clocks.push(line(ed, p).trim().to_owned());
            let mut s = p;
            while s > m && line(ed, s - 1).trim().is_empty() {
                s -= 1;
            }
            super::delete_lines(ed, s, p + 1 - s);
        }
        let k = clocks.len();
        let mut block = vec![format!(":{d}:")];
        block.extend(clocks);
        block.push(":END:".into());
        super::insert_lines(ed, m, &block);
        for i in m..m + k + 2 {
            let _ = super::editing::indent_line(ed, i);
        }
        fold::region(ed, m + 1, m + k, true, fold::Spec::Drawer);
        return Pos::Insert(if reversed { m + 2 } else { m + k });
    }
    if reversed {
        Pos::Insert(positions[0])
    } else {
        Pos::Insert(*positions.last().unwrap())
    }
}

// ---- clocking in ----

fn rounding() -> i64 {
    match sexp::option("org-clock-rounding-minutes") {
        Some(sexp::Sexp::Int(i)) => i,
        Some(_) => stamp::rounding_minutes().0,
        None => 0,
    }
}

/// org-current-time with org-clock-rounding-minutes.
fn now_rounded(past: bool) -> Tm {
    super::time::read::current_time(rounding(), past)
}

fn opt_sym(name: &str) -> Option<String> {
    sexp::option(name).and_then(|v| match v {
        sexp::Sexp::Nil => None,
        v => v.str().or(v.sym()).map(str::to_owned),
    })
}

/// org-clock-get-sum-start: the start of the time the mode line totals,
/// and the message extra.
fn sum_start(ed: &Editor, h: usize) -> (Option<i64>, &'static str) {
    let cmt = props::get(
        ed,
        Some(h),
        "CLOCK_MODELINE_TOTAL",
        props::Inherit::Selective,
    )
    .unwrap_or_else(|| opt_sym("org-clock-mode-line-total").unwrap_or("auto".into()));
    let lr = props::get(ed, Some(h), "LAST_REPEAT", props::Inherit::No);
    let auto = cmt == "auto";
    match cmt.as_str() {
        "current" => (
            Some(civil::now_epoch()),
            "showing time in current clock instance",
        ),
        "today" => {
            let ext = options::int("org-extend-today-until", 0);
            let mut t = civil::now();
            if t.hour < ext {
                t.day -= 1;
            }
            (
                Some(Tm::at(t.year, t.month, t.day, ext, 0).epoch()),
                "showing today's task time.",
            )
        }
        "all" => (None, "showing entire task time."),
        _ if auto && lr.is_none() => (None, "showing entire task time."),
        _ if cmt == "repeat" || auto => (
            lr.and_then(|l| stamp::time_string_to_seconds(&l).ok()),
            "showing task time since last repeat.",
        ),
        _ => (None, ""),
    }
}

/// org-clock-sum-current-item from `tstart`.
fn sum_current_item(ed: &Editor, h: usize, tstart: Option<i64>) -> i64 {
    let s = sum::sum(
        &ed.buf,
        h,
        syntax::subtree_end(&ed.buf, h),
        tstart.map(|t| t as f64),
        None,
        None,
        None,
    );
    s.total
}

/// org-clock--mode-line-heading.
fn mode_line_heading(ed: &Editor, h: usize) -> String {
    super::links::display_format(&title(&line(ed, h), &super::settings(ed)))
}

/// org-clock-history-push.
fn history_push(m: Marker) {
    let len = options::int("org-clock-history-length", 5).max(1) as usize;
    with(|s| {
        s.history
            .retain(|x| !(x.file == m.file && (x.line == m.line || x.heading == m.heading)));
        s.history.truncate(len - 1);
        s.history.insert(0, m);
    });
}

/// The TODO state switching of org-clock-in/out-switch-to-state.
fn switch_state(ed: &mut Editor, h: usize, state: &str) {
    let st = super::settings(ed);
    let cur = syntax::headline(&line(ed, h), &st).and_then(|x| x.todo);
    if cur.as_deref() == Some(state) {
        return;
    }
    let pos = (ed.cur.line, ed.cur.byte);
    ed.set_cursor(h, 0);
    let _ = todo::todo_to(ed, Some(state.to_owned()));
    ed.set_cursor(pos.0.min(ed.line_count() - 1), pos.1);
}

/// How org-clock-in was asked to start.
#[derive(Clone, Copy, Debug, Default)]
struct InOpts {
    start: Option<i64>,
    resume: bool,
    auto_restart: bool,
    continuously: bool,
    leftover: Option<i64>,
    interrupting: bool,
}

/// org-clock-in.
pub fn clock_in(ed: &mut Editor, select: Prefix, start: Option<i64>) -> Result<(), String> {
    clock_in_with(ed, select, start, false)
}

fn clock_in_with(
    ed: &mut Editor,
    select: Prefix,
    start: Option<i64>,
    resume: bool,
) -> Result<(), String> {
    with(|s| s.notified = false);
    let interrupting = running().is_some();
    let (resolving, clocking_in) = with(|s| (s.resolving, s.clocking_in));
    let leftover = if resolving {
        None
    } else {
        with(|s| s.leftover)
    };
    let auto =
        opt_sym("org-clock-auto-clock-resolution").unwrap_or("when-no-clock-is-running".into());
    let mut o = InOpts {
        start,
        resume: resume || options::bool("org-clock-in-resume", false),
        leftover,
        interrupting,
        ..InOpts::default()
    };
    if auto != "nil" && (!interrupting || auto == "t") && !clocking_in && !resolving {
        with(|s| s.leftover = None);
        let clocks = open_clocks(ed);
        let clocks: Vec<Open> = clocks.into_iter().filter(|c| !is_active(c)).collect();
        if !clocks.is_empty() {
            with(|s| s.clocking_in = true);
            resolve_all(ed, clocks, None, true, move |ed| {
                with(|s| s.clocking_in = false);
                if let Err(e) = clock_in_select(ed, select, o) {
                    ed.set_err(e);
                }
            });
            return Ok(());
        }
    }
    o.leftover = leftover;
    clock_in_select(ed, select, o)
}

fn clock_in_select(ed: &mut Editor, select: Prefix, mut o: InOpts) -> Result<(), String> {
    match select {
        Prefix::U(3) => {
            o.continuously = true;
            o.start = with(|s| s.out_time);
            clock_in_target(ed, None, o)
        }
        Prefix::U(1) => select_task(ed, "Clock-in on task: ", move |ed, m| match m {
            None => ed.set_err("Abort"),
            Some(m) => {
                if let Err(e) = clock_in_target(ed, Some(m), o) {
                    ed.set_err(e);
                }
            }
        }),
        Prefix::U(2) => {
            mark_default_task(ed)?;
            clock_in_target(ed, None, o)
        }
        _ => clock_in_target(ed, None, o),
    }
}

/// The task part of org-clock-in: check for clocking the same task, clock
/// out the interrupted one, then clock in at the target.
fn clock_in_target(ed: &mut Editor, selected: Option<Marker>, o: InOpts) -> Result<(), String> {
    if let Some(r) = running() {
        // Clocking into the same task: the clock continues.
        let same = match &selected {
            Some(m) => m.file == r.task.file && m.heading == r.task.heading,
            None => {
                ed.org.is_some()
                    && same_file(ed, &r.task.file)
                    && fold::back_to_heading(ed, ed.cur.line).is_some_and(|h| {
                        locate(ed, &r.task) == Some(h)
                            && Some(title(&line(ed, h), &super::settings(ed)))
                                == with(|s| s.current_task.clone())
                    })
            }
        };
        if same {
            ed.set_msg(format!("Clock continues in {:?}", r.heading));
            return Ok(());
        }
        with(|s| s.interrupted = Some(r.task.clone()));
        with(|s| s.clocking_in = true);
        let rr = r.clone();
        let switch = opt_sym("org-clock-out-switch-to-state");
        with(|s| {
            s.running = None;
            s.out_time = Some(now_rounded(false).epoch());
        });
        in_file(ed, r.task.file.clone(), move |e| {
            let _ = clock_out_record(e, &rr, None, switch, true, true);
        });
        with(|s| s.clocking_in = false);
    }
    match selected {
        Some(m) if !(same_file(ed, &m.file) || m.file.is_none()) => {
            let file = m.file.clone();
            in_file(ed, file, move |e| match locate(e, &m) {
                Some(h) => {
                    if let Err(err) = clock_in_at(e, h, o) {
                        e.set_err(err);
                    }
                }
                None => e.set_err("Cannot find the selected task"),
            });
            Ok(())
        }
        Some(m) => {
            let h = locate(ed, &m).ok_or("Cannot find the selected task")?;
            clock_in_at(ed, h, o)
        }
        None => {
            let h = fold::back_to_heading(ed, ed.cur.line)
                .ok_or_else(|| "Before first headline".to_owned())?;
            clock_in_at(ed, h, o)
        }
    }
}

/// Clock in at heading `h` of `ed`.
fn clock_in_at(ed: &mut Editor, h: usize, o: InOpts) -> Result<(), String> {
    if !o.interrupting {
        with(|s| s.interrupted = None);
    }
    let st = super::settings(ed);
    history_push(marker(ed, h));
    with(|s| s.current_task = Some(title(&line(ed, h), &st)));
    if let Some(state) = opt_sym("org-clock-in-switch-to-state") {
        switch_state(ed, h, &state);
    }
    let h = fold::back_to_heading(ed, h).unwrap_or(h);
    let heading = mode_line_heading(ed, h);
    match find_position(ed, h, o.resume) {
        Pos::Open(l) if o.resume => {
            let t = line(ed, l);
            let s = t
                .trim_start()
                .trim_start_matches("CLOCK:")
                .trim()
                .to_owned();
            ed.set_msg(format!("Matched {}", &s[1..s.len() - 1]));
            let start = stamp::time_string_to_seconds(&s)?;
            let h = fold::back_to_heading(ed, l).unwrap_or(h);
            let effort = effort_of(ed, h);
            let (ss, extra) = sum_start(ed, h);
            let total = sum_current_item(ed, h, ss);
            start_clock(ed, h, l, s, start, heading, effort, total, extra);
            Ok(())
        }
        _ if o.auto_restart => {
            Err("Cannot restart clock because task does not contain unfinished clock".into())
        }
        Pos::Open(l) | Pos::Insert(l) => {
            let leftover = o.leftover;
            let finish = move |ed: &mut Editor, from_leftover: bool| {
                let start = if o.continuously
                    && let Some(t) = with(|s| s.out_time)
                {
                    t
                } else if from_leftover && let Some(t) = leftover {
                    t
                } else if let Some(t) = o.start {
                    t
                } else {
                    now_rounded(true).epoch()
                };
                insert_clock(ed, h, l, start, heading);
            };
            match leftover {
                Some(lo) if !(o.continuously && with(|s| s.out_time.is_some())) => {
                    let mins = (now_rounded(true).epoch() - lo) / 60;
                    super::menu(
                        ed,
                        &format!(
                            "You stopped another clock {mins} mins ago; start this one from then? "
                        ),
                        vec![("y".into(), "yes".into()), ("n".into(), "no".into())],
                        move |ed, k| {
                            ed.undo.begin(ed.cur.pos());
                            finish(ed, k == "y");
                            ed.undo.end(ed.cur.pos());
                        },
                    );
                }
                _ => finish(ed, false),
            }
            Ok(())
        }
    }
}

fn effort_of(ed: &Editor, h: usize) -> Option<String> {
    let p = options::string("org-effort-property", "Effort");
    props::get(ed, Some(h), &p, props::Inherit::No)
}

/// Insert the CLOCK line at line `l` for heading `h`, started at `start`.
fn insert_clock(ed: &mut Editor, h: usize, l: usize, start: i64, heading: String) {
    let ts = stamp::stamp_text(Tm::from_epoch(start), true, true, "");
    let (cl, cb) = (ed.cur.line, ed.cur.byte);
    super::insert_lines(ed, l, &[format!("CLOCK: {ts}")]);
    let _ = super::editing::indent_line(ed, l);
    // Point moves after the clock line when it was there.
    let cl = if cl >= l && cl < ed.line_count() - 1 {
        cl + 1
    } else {
        cl
    };
    ed.set_cursor(cl.min(ed.line_count() - 1), cb);
    let effort = effort_of(ed, h);
    let (ss, extra) = sum_start(ed, h);
    let total = sum_current_item(ed, h, ss);
    start_clock(ed, h, l, ts, start, heading, effort, total, extra);
}

#[allow(clippy::too_many_arguments)]
fn start_clock(
    ed: &mut Editor,
    h: usize,
    l: usize,
    ts: String,
    start: i64,
    heading: String,
    effort: Option<String>,
    total: i64,
    extra: &str,
) {
    let task = marker(ed, h);
    with(|s| {
        s.running = Some(Running {
            task,
            clock_line: l,
            stamp: ts.clone(),
            start,
            heading,
            effort,
            total,
        });
        s.has_been_used = true;
        s.shown = (-1, -1);
    });
    ed.set_msg(format!("Clock starts at {ts} - {extra}"));
    save();
}

// ---- clocking out ----

/// org-clock-out on the clock `r` in `ed` (its buffer): close the CLOCK
/// line at `at` (now) and log. Returns whether the line was removed.
fn clock_out_record(
    ed: &mut Editor,
    r: &Running,
    at: Option<i64>,
    switch: Option<String>,
    fail_quietly: bool,
    note: bool,
) -> Result<bool, String> {
    let Some(l) = locate_clock(ed, r) else {
        return if fail_quietly {
            Ok(false)
        } else {
            Err("Clock start time is gone".into())
        };
    };
    let now = now_rounded(false).epoch();
    let t = line(ed, l);
    let re = crate::org_re!(r"^([ \t]*CLOCK:[ \t]*)(\[[^\]\n]*\]|<[^>\n]*>)");
    let Some(c) = re.captures(&t) else {
        return if fail_quietly {
            Ok(false)
        } else {
            Err("Clock start time is gone".into())
        };
    };
    let ts = c[2].to_owned();
    let te = stamp::stamp_text(Tm::from_epoch(at.unwrap_or(now)), true, true, "");
    let s = stamp::time_string_to_seconds(&te).unwrap_or(0)
        - stamp::time_string_to_seconds(&ts).unwrap_or(0);
    let (hh, mm) = (s.div_euclid(3600), s.rem_euclid(3600).div_euclid(60));
    super::set_line(
        ed,
        l,
        &format!("{}--{te} => {hh:2}:{mm:02}", &t[..c.get(0).unwrap().end()]),
    );
    let h = fold::back_to_heading(ed, l).unwrap_or(0);
    let remove =
        options::bool("org-clock-out-remove-zero-time-clocks", false) && hh == 0 && mm == 0;
    if remove {
        super::delete_lines(ed, l, 1);
    }
    remove_empty_clock_drawers(ed, h);
    if let Some(state) = switch {
        with(|s| s.no_out_when_done = true);
        switch_state(ed, h, &state);
        with(|s| s.no_out_when_done = false);
    }
    ed.set_msg(format!(
        "Clock stopped at {te} after {}{}",
        dur(hh * 60 + mm),
        if remove { " => LINE REMOVED" } else { "" }
    ));
    if note && !remove && options::bool("org-log-note-clock-out", false) {
        let task = super::lines(ed, h..h + 1)
            .first()
            .map(|t| notags(t, &super::settings(ed)))
            .unwrap_or_default();
        let l = locate_after_clock_out(ed, h, &te).unwrap_or(l);
        todo::add_log(
            ed,
            l + 1,
            todo::Note {
                purpose: "clock-out".into(),
                state: None,
                prev: None,
                how: todo::How::Note,
                extra: Some(format!("# Task: {task}\n\n")),
                time: super::now(),
            },
        );
    }
    Ok(remove)
}

/// The closed clock line ending at `te` in heading `h`'s entry.
fn locate_after_clock_out(ed: &Editor, h: usize, te: &str) -> Option<usize> {
    (h..syntax::entry_end(&ed.buf, h)).find(|&i| {
        let t = line(ed, i);
        t.trim_start().starts_with("CLOCK:") && t.contains(&format!("--{te}"))
    })
}

fn dur(minutes: i64) -> String {
    super::time::duration::from_minutes(minutes as f64, None, false).unwrap_or_default()
}

/// org-clock-out.
pub fn clock_out(
    ed: &mut Editor,
    switch: Option<String>,
    fail_quietly: bool,
    at: Option<i64>,
) -> Result<(), String> {
    let Some(r) = running() else {
        return if fail_quietly {
            Ok(())
        } else {
            Err("No active clock".into())
        };
    };
    let switch = switch.or_else(|| opt_sym("org-clock-out-switch-to-state"));
    let now = now_rounded(false).epoch();
    with(|s| {
        s.out_time = Some(at.unwrap_or(now));
        s.running = None;
        s.current_task = None;
        s.shown = (-1, -1);
    });
    let file = r.task.file.clone();
    in_file(ed, file, move |e| {
        if let Err(err) = clock_out_record(e, &r, at, switch, fail_quietly, true) {
            e.set_err(err);
        }
    });
    save();
    Ok(())
}

/// org-clock-out-if-current: after org-todo marked the entry at the
/// cursor done (`state`).
pub fn out_if_current(ed: &mut Editor) -> Result<(), String> {
    let Some(r) = running() else { return Ok(()) };
    if with(|s| s.no_out_when_done) || !same_file(ed, &r.task.file) {
        return Ok(());
    }
    let st = super::settings(ed);
    let Some(h) = fold::back_to_heading(ed, ed.cur.line) else {
        return Ok(());
    };
    let state = syntax::headline(&line(ed, h), &st).and_then(|x| x.todo);
    let when = sexp::option("org-clock-out-when-done").unwrap_or(sexp::Sexp::T);
    let applies = match &when {
        sexp::Sexp::Nil => false,
        sexp::Sexp::T => state.as_deref().is_some_and(|s| st.is_done(s)),
        v => v
            .list()
            .unwrap_or(&[])
            .iter()
            .any(|x| x.str() == state.as_deref()),
    };
    let Some(l) = locate_clock(ed, &r) else {
        return Ok(());
    };
    if !applies || !(l > h && l < syntax::entry_end(&ed.buf, h)) {
        return Ok(());
    }
    with(|s| {
        s.out_time = Some(now_rounded(false).epoch());
        s.running = None;
        s.current_task = None;
    });
    clock_out_record(ed, &r, None, None, false, false)?;
    save();
    Ok(())
}

/// org-clock-cancel.
pub fn cancel(ed: &mut Editor) -> Result<(), String> {
    let r = running().ok_or("No active clock")?;
    with(|s| {
        s.running = None;
        s.current_task = None;
    });
    in_file(ed, r.task.file.clone(), move |e| {
        match locate_clock(e, &r) {
            Some(l) => {
                super::delete_lines(e, l, 1);
                if l > 0 {
                    remove_empty_drawer_at(e, l - 1);
                }
                e.set_msg("Clock canceled");
            }
            None => e.set_msg("Clock gone, cancel the timer anyway"),
        }
    });
    ed.set_msg("Clock canceled");
    save();
    Ok(())
}

// ---- selecting and going to tasks ----

/// org-clock-select-task: a key menu of the default, interrupted, current
/// and recent tasks; `then` gets the chosen marker (None: q or x).
fn select_task(
    ed: &mut Editor,
    prompt: &str,
    then: impl FnOnce(&mut Editor, Option<Marker>) + Send + 'static,
) -> Result<(), String> {
    let (hist, def, intr, cur) = with(|s| {
        let mut och: Vec<Marker> = vec![];
        for c in &s.history {
            if och.last() != Some(c) {
                och.push(c.clone());
            }
        }
        (
            och,
            s.default_task.clone(),
            s.interrupted.clone(),
            s.running.as_ref().map(|r| r.task.clone()),
        )
    });
    if hist.is_empty() {
        return Err("No recent clock".into());
    }
    let mut entries: Vec<(String, String)> = vec![];
    let mut sel: Vec<(String, Marker)> = vec![];
    let mut add = |key: char, m: Marker, entries: &mut Vec<(String, String)>| {
        entries.push((key.to_string(), format!("{:<12}  {}", m.category, m.label)));
        sel.push((key.to_string(), m));
    };
    if let Some(m) = def {
        entries.push((String::new(), "#Default Task".into()));
        add('d', m, &mut entries);
    }
    if let Some(m) = intr {
        entries.push((
            String::new(),
            "#The task interrupted by starting the last one".into(),
        ));
        add('i', m, &mut entries);
    }
    if let Some(m) = cur {
        entries.push((String::new(), "#Current Clocking Task".into()));
        add('c', m, &mut entries);
    }
    entries.push((String::new(), "#Recent Tasks".into()));
    for (i, m) in hist.into_iter().enumerate() {
        let i = i as u32 + 1;
        let k = if i < 10 {
            char::from_digit(i, 10).unwrap()
        } else {
            char::from(b'A' + (i - 10) as u8)
        };
        add(k, m, &mut entries);
    }
    entries.push(("q".into(), "quit".into()));
    super::menu(ed, prompt, entries, move |ed, k| {
        let m = sel.into_iter().find(|(x, _)| *x == k).map(|(_, m)| m);
        ed.undo.begin(ed.cur.pos());
        then(ed, m);
        ed.undo.end(ed.cur.pos());
    });
    Ok(())
}

/// org-clock-mark-default-task.
pub fn mark_default_task(ed: &mut Editor) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let m = marker(ed, h);
    with(|s| s.default_task = Some(m));
    Ok(())
}

/// Show marker `m`'s heading (org-clock-goto's display part).
fn goto_marker(ed: &mut Editor, m: Marker, clock: Option<Running>, recent: bool) {
    let (file, at) = (m.file.clone(), m.line);
    let show = move |e: &mut Editor| {
        let l = clock
            .as_ref()
            .and_then(|r| locate_clock(e, r))
            .or_else(|| locate(e, &m));
        let Some(l) = l else {
            e.set_err("Cannot find the clocked task");
            return;
        };
        let h = fold::back_to_heading(e, l).unwrap_or(l);
        fold::show_entry(e, h);
        e.set_cursor(h, 0);
        fold::show_context(e, h);
        e.reveal_cursor();
        if recent {
            e.set_msg("No running clock, this is the most recently clocked task");
        }
    };
    match file {
        Some(p) if !same_file(ed, &Some(p.clone())) => push_effect(ed, move |s| {
            if let Err(e) = s.org_visit(&p, at) {
                return s.ed.set_err(e);
            }
            show(&mut s.ed);
        }),
        _ => show(ed),
    }
}

/// org-clock-goto.
pub fn goto(ed: &mut Editor, select: bool) -> Result<(), String> {
    if select {
        return select_task(ed, "Select task to go to: ", |ed, m| match m {
            Some(m) => goto_marker(ed, m, None, false),
            None => ed.set_err("No task selected"),
        });
    }
    if let Some(r) = running() {
        goto_marker(ed, r.task.clone(), Some(r), false);
        return Ok(());
    }
    match history().into_iter().next() {
        Some(m) if options::bool("org-clock-goto-may-find-recent-task", true) => {
            goto_marker(ed, m, None, true);
            Ok(())
        }
        _ => Err("No active or recent clock task".into()),
    }
}

/// org-clock-in-last.
pub fn in_last(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if arg == Prefix::U(1) {
        return clock_in(ed, arg, None);
    }
    let start = if options::bool("org-clock-continuously", false) || arg == Prefix::U(2) {
        with(|s| s.out_time).unwrap_or_else(|| now_rounded(true).epoch())
    } else {
        now_rounded(true).epoch()
    };
    let Some(m) = history().into_iter().next() else {
        ed.set_msg("No last clock");
        return Ok(());
    };
    let already = with(|s| s.current_task.is_some());
    let go = move |ed: &mut Editor, state: Option<String>| {
        let file = m.file.clone();
        in_file(ed, file, move |e| {
            let Some(h) = locate(e, &m) else {
                return e.set_err("Cannot find the last clocked task");
            };
            e.set_cursor(h, 0);
            let saved = state.as_ref().map(|s| {
                let old = options::get("org-clock-in-switch-to-state");
                options::put(
                    "org-clock-in-switch-to-state",
                    toml::Value::String(s.clone()),
                );
                old
            });
            if let Err(err) = clock_in(e, Prefix::None, Some(start)) {
                e.set_err(err);
            } else if !already {
                let name = e
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let task = with(|s| s.current_task.clone()).unwrap_or_default();
                e.set_msg(format!("Clocking back: {task} (in {name})"));
            }
            if let Some(old) = saved {
                match old {
                    Some(v) => options::put("org-clock-in-switch-to-state", v),
                    None => options::remove("org-clock-in-switch-to-state"),
                }
            }
        });
    };
    if !already && arg == Prefix::U(3) {
        let kws: Vec<String> = super::settings(ed)
            .todo_names()
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        super::complete(ed, "Switch to state: ", kws, false, move |ed, s| {
            go(ed, Some(s).filter(|s| !s.is_empty()))
        });
        return Ok(());
    }
    go(ed, None);
    Ok(())
}

// ---- effort and the mode line ----

/// org-clock-get-clocked-time.
pub fn clocked_time(r: &Running) -> i64 {
    (civil::now_epoch() - r.start).div_euclid(60) + r.total
}

/// org-clock-get-clock-string with MAX-LENGTH (0: no limit).
pub fn clock_string(r: &Running, max: usize) -> String {
    let clock = dur(clocked_time(r));
    let time = match &r.effort {
        None => format!("[{clock}]"),
        Some(e) => {
            let m = super::time::duration::to_minutes(e, false).unwrap_or(0.0);
            format!("[{clock}/{}]", dur(m as i64))
        }
    };
    let spaces = 1 + 4;
    let tl = time.chars().count();
    let untruncated = spaces + tl + r.heading.chars().count();
    if max == 0 || max >= untruncated {
        format!("{time} ({}) ", r.heading)
    } else if max <= spaces + tl {
        format!("{} ", time.chars().take(max.min(tl)).collect::<String>())
    } else {
        let n = max - (spaces + tl);
        let head: String = r.heading.chars().take(n).collect();
        format!("{time} ({head}…) ")
    }
}

/// org-mode-line-string, for Fred's status line.
pub fn mode_line() -> Option<String> {
    let r = running()?;
    let display = opt_sym("org-clock-clocked-in-display").unwrap_or("mode-line".into());
    if display != "mode-line" && display != "both" {
        return None;
    }
    let s = clock_string(
        &r,
        options::int("org-clock-string-limit", 0).max(0) as usize,
    );
    let overrun = with(|s| s.overrun);
    Some(match opt_sym("org-clock-task-overrun-text") {
        Some(t) if overrun => format!("{t}{s}"),
        _ => s,
    })
}

/// Whether the running task is over its effort (org-clock-task-overrun).
pub fn overrun() -> bool {
    with(|s| s.overrun)
}

/// org-clock-modify-effort-estimate with a typed VALUE.
fn modify_effort(ed: &mut Editor, value: String) {
    let Some(mut r) = running() else {
        return ed.set_msg("Clock is not currently active");
    };
    let current = r.effort.clone();
    let sign = value.chars().next();
    let mut cur = 0.0;
    let mut v = value.as_str();
    if matches!(sign, Some('-' | '+')) {
        cur = current
            .as_deref()
            .and_then(|c| super::time::duration::to_minutes(c, false).ok())
            .unwrap_or(0.0);
        v = &value[1..];
    }
    let mut val = super::time::duration::to_minutes(v, false).unwrap_or(0.0);
    if sign == Some('-') {
        val = cur - val;
    } else if sign == Some('+') {
        val += cur;
    }
    let effort = dur(val.max(0.0) as i64);
    r.effort = Some(effort.clone());
    with(|s| s.running = Some(r.clone()));
    let prop = options::string("org-effort-property", "Effort");
    let msg = format!("Effort is now {effort}");
    in_file(ed, r.task.file.clone(), move |e| {
        if let Some(h) = locate(e, &r.task) {
            let _ = props::put(e, Some(h), &prop, &effort);
        }
    });
    ed.set_msg(msg);
}

/// org-notify: org-show-notification (a program, or the message).
pub fn notify(ed: &mut Editor, text: &str, sound: Option<sexp::Sexp>) {
    match opt_sym("org-show-notification-handler") {
        Some(p) if !p.is_empty() => {
            let _ = std::process::Command::new(&p)
                .arg(text)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        _ => ed.set_msg(text),
    }
    // org-clock-play-sound: a sound file through aplay/afplay.
    if let Some(sexp::Sexp::Str(f)) = sound.or_else(|| sexp::option("org-clock-sound")) {
        let f = options::expand(&f);
        if f.exists() {
            for player in ["aplay", "afplay"] {
                if std::process::Command::new(player)
                    .arg(&f)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .is_ok()
                {
                    break;
                }
            }
        }
    }
}

/// org-clock-notify-once-if-expired.
fn notify_once_if_expired(ed: &mut Editor) {
    let Some(r) = running() else { return };
    let effort = r
        .effort
        .as_deref()
        .and_then(|e| super::time::duration::to_minutes(e, false).ok())
        .unwrap_or(0.0);
    let over = effort > 0.0 && clocked_time(&r) as f64 >= effort;
    let notify_now = with(|s| {
        s.overrun = over;
        if over && !s.notified {
            s.notified = true;
            true
        } else {
            if !over {
                s.notified = false;
            }
            false
        }
    });
    if notify_now {
        notify(
            ed,
            &format!(
                "Task ‘{}’ should be finished by now. ({})",
                r.heading,
                r.effort.unwrap_or_default()
            ),
            None,
        );
    }
}

/// The user's idle time in seconds (org-user-idle-seconds): macOS ioreg,
/// X11 xprintidle; None when unknown.
fn user_idle_seconds() -> Option<f64> {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    if cfg!(target_os = "macos") {
        let out = run("ioreg", &["-c", "IOHIDSystem"])?;
        let l = out.lines().find(|l| l.contains("HIDIdleTime"))?;
        let n: f64 = l.rsplit('=').next()?.trim().parse().ok()?;
        return Some(n / 1e9);
    }
    let prog = options::string("org-clock-x11idle-program-name", "xprintidle");
    let out = run(&prog, &[])?;
    Some(out.trim().parse::<f64>().ok()? / 1000.0)
}

/// The app loop's tick: the mode line minute, effort notification, idle
/// resolution and auto clock-out. True when the status line changed.
pub fn tick(s: &mut Session) -> bool {
    if !with(|st| st.loaded) && s.ed.org.is_some() {
        load(s);
    }
    let Some(r) = running() else { return false };
    let now = civil::now_epoch();
    let minute = clocked_time(&r);
    let changed = with(|st| {
        let c = st.shown.0 != minute;
        st.shown.0 = minute;
        c
    });
    if !changed {
        return false;
    }
    notify_once_if_expired(&mut s.ed);
    // org-resolve-clocks-if-idle and org-clock--auto-clockout-maybe, once
    // a minute.
    let idle_time = options::int("org-clock-idle-time", 0);
    let auto_out = with(|st| st.auto_clockout)
        .unwrap_or_else(|| options::bool("org-clock-auto-clockout-insinuate", false));
    let auto_secs = options::int("org-clock-auto-clockout-timer", 0);
    if (idle_time > 0 || (auto_out && auto_secs > 0)) && with(|st| now - st.last_idle_check >= 60) {
        with(|st| st.last_idle_check = now);
        if let Some(idle) = user_idle_seconds() {
            if auto_out && auto_secs > 0 && idle >= auto_secs as f64 {
                let _ = clock_out(&mut s.ed, None, true, None);
            } else if idle_time > 0 && idle > 60.0 * idle_time as f64 && !with(|st| st.resolving) {
                let idle_start = now - idle as i64;
                let clock = Open {
                    file: r.task.file.clone(),
                    line: r.clock_line,
                    stamp: r.stamp.clone(),
                    start: r.start,
                };
                let prompt = format!("Clocked in & idle for {:.1} mins", idle / 60.0);
                resolve(&mut s.ed, clock, prompt, idle_start, false, |_| {});
            }
        }
    }
    true
}

// ---- dangling clocks ----

/// An open clock found in a file (org-find-open-clocks).
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub file: Option<PathBuf>,
    pub line: usize,
    pub stamp: String,
    pub start: i64,
}

impl Open {
    fn running(&self) -> Running {
        Running {
            task: Marker {
                file: self.file.clone(),
                line: self.line,
                heading: String::new(),
                category: String::new(),
                label: String::new(),
            },
            clock_line: self.line,
            stamp: self.stamp.clone(),
            start: self.start,
            heading: String::new(),
            effort: None,
            total: 0,
        }
    }
}

/// org-find-open-clocks in lines, last first.
pub fn find_open_clocks(lines: &[String], file: Option<PathBuf>) -> Vec<Open> {
    let re = crate::org_re!(r"^[ \t]*CLOCK: (\[.*?\])$");
    let mask = sum::block_mask(lines, 0, lines.len());
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate() {
        if let Some(c) = re.captures(l)
            && !mask[i]
            && let Ok(start) = stamp::time_string_to_seconds(&c[1])
        {
            out.push(Open {
                file: file.clone(),
                line: i,
                stamp: c[1].to_owned(),
                start,
            });
        }
    }
    out.reverse();
    out
}

/// The open clocks of this buffer and of the agenda files on disk
/// (org-files-list).
fn open_clocks(ed: &Editor) -> Vec<Open> {
    let me = file_of(ed);
    let mut out = find_open_clocks(&super::lines(ed, 0..ed.line_count()), me.clone());
    for f in super::agenda_files() {
        let f = std::path::absolute(&f).unwrap_or(f);
        if Some(&f) == me.as_ref() {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&f) {
            let lines: Vec<String> = text.lines().map(str::to_owned).collect();
            out.extend(find_open_clocks(&lines, Some(f)));
        }
    }
    out
}

fn is_active(c: &Open) -> bool {
    running().is_some_and(|r| r.task.file == c.file && r.stamp == c.stamp)
}

/// org-resolve-clocks: resolve `clocks` one after another, then `then`.
fn resolve_all(
    ed: &mut Editor,
    mut clocks: Vec<Open>,
    last_valid: Option<i64>,
    only_dangling: bool,
    then: impl FnOnce(&mut Editor) + Send + 'static,
) {
    clocks.retain(|c| !only_dangling || !is_active(c));
    if clocks.is_empty() {
        with(|s| s.resolving = false);
        return then(ed);
    }
    with(|s| s.resolving = true);
    let c = clocks.remove(0);
    let mins = (civil::now_epoch() - c.start).div_euclid(60);
    let lv = last_valid.unwrap_or(c.start);
    resolve(
        ed,
        c,
        format!("Dangling clock started {mins} mins ago"),
        lv,
        false,
        move |ed| resolve_all(ed, clocks, last_valid, only_dangling, then),
    );
}

/// org-clock-resolve: ask what to do with `clock`, idle since `last_valid`.
fn resolve(
    ed: &mut Editor,
    clock: Open,
    prompt: String,
    last_valid: i64,
    fail_quietly: bool,
    then: impl FnOnce(&mut Editor) + Send + 'static,
) {
    // org-clock-jump-to-current-clock (in this buffer).
    if same_file(ed, &clock.file) && clock.line < ed.line_count() {
        ed.set_cursor(clock.line, 0);
        fold::show_context(ed, clock.line);
    }
    let entries = vec![
        (
            "i".into(),
            "Ignore this question; the same as keeping all the idle time".into(),
        ),
        ("q".into(), "Ignore this question".into()),
        (
            "k".into(),
            "Keep X minutes of the idle time (default is all)".into(),
        ),
        ("K".into(), "Keep X minutes, then clock out".into()),
        (
            "t".into(),
            "Like k, but ask for the time you got distracted".into(),
        ),
        ("T".into(), "Like t, then clock out".into()),
        ("g".into(), "Got back X minutes ago".into()),
        ("G".into(), "Got back X minutes ago, then clock out".into()),
        (
            "s".into(),
            "Subtract the idle time from the current clock".into(),
        ),
        ("S".into(), "Subtract the idle time, then clock out".into()),
        ("C".into(), "Cancel the open clock altogether".into()),
        ("j".into(), "Jump to the current clock".into()),
        ("J".into(), "Jump to the current clock, clocked out".into()),
    ];
    let title = format!("{prompt} [jkKtTgGSscCiq]? ");
    super::menu(ed, &title, entries, move |ed, k| {
        let ch = k.chars().next().unwrap_or('q');
        let default = (civil::now_epoch() - last_valid).div_euclid(60);
        let act = move |ed: &mut Editor, keep: Option<i64>, gotback: Option<i64>| {
            ed.undo.begin(ed.cur.pos());
            let r = resolve_choice(
                ed,
                &clock,
                ch,
                keep,
                gotback,
                default,
                last_valid,
                fail_quietly,
            );
            ed.undo.end(ed.cur.pos());
            if let Err(e) = r {
                ed.set_err(e);
            }
            then(ed);
        };
        match ch {
            'k' | 'K' | 'g' | 'G' => {
                let p = if ch.eq_ignore_ascii_case(&'k') {
                    "Keep how many minutes: "
                } else {
                    "Got back how many minutes ago: "
                };
                super::read(ed, &format!("{p}(default {default}) "), "", move |ed, a| {
                    let n = a.trim().parse::<i64>().unwrap_or(default);
                    if ch.eq_ignore_ascii_case(&'k') {
                        act(ed, Some(n), None)
                    } else {
                        act(ed, None, Some(n))
                    }
                });
            }
            't' | 'T' => {
                super::time::read_date(ed, super::time::ReadOpts::default(), move |ed, a| {
                    let t = a.tm.norm().epoch();
                    act(ed, Some((t - last_valid).div_euclid(60)), None)
                });
            }
            _ => act(ed, None, None),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn resolve_choice(
    ed: &mut Editor,
    clock: &Open,
    ch: char,
    keep: Option<i64>,
    gotback: Option<i64>,
    default: i64,
    last_valid: i64,
    fail_quietly: bool,
) -> Result<(), String> {
    let subtract = matches!(ch, 's' | 'S');
    let barely = last_valid - clock.start < 45;
    let start_over = subtract && barely;
    match ch {
        'j' | 'J' => {
            if ch == 'J' {
                resolve_clock(ed, clock, To::Now, None, true, false, fail_quietly)?;
            }
            if same_file(ed, &clock.file) {
                ed.set_cursor(clock.line.min(ed.line_count() - 1), 0);
            }
            return Ok(());
        }
        'k' | 'K' | 'g' | 'G' | 's' | 'S' | 'C' | 't' | 'T' => {}
        _ => {
            ed.set_msg("");
            return Ok(());
        }
    }
    let to = if ch == 'C' || start_over {
        To::Cancel
    } else if subtract || gotback == Some(0) {
        To::At(last_valid)
    } else if keep == Some(default) || gotback == Some(default) {
        To::Now
    } else if let Some(k) = keep {
        To::At(last_valid + 60 * k)
    } else if let Some(g) = gotback {
        To::At(civil::now_epoch() - 60 * g)
    } else {
        return Err("Unexpected, please report this as a bug".into());
    };
    resolve_clock(
        ed,
        clock,
        to,
        gotback.map(|_| last_valid),
        matches!(ch, 'K' | 'G' | 'S' | 'T'),
        start_over && !matches!(ch, 'K' | 'G' | 'S' | 'C'),
        fail_quietly,
    )
}

#[derive(Clone, Copy, Debug)]
enum To {
    Cancel,
    Now,
    At(i64),
}

/// org-clock-resolve-clock.
fn resolve_clock(
    ed: &mut Editor,
    clock: &Open,
    to: To,
    clock_out_time: Option<i64>,
    close: bool,
    restart: bool,
    fail_quietly: bool,
) -> Result<(), String> {
    let clocking_in = with(|s| s.clocking_in);
    let active = is_active(clock);
    let rec = clock.running();
    let file = clock.file.clone();
    match to {
        To::Cancel => {
            if active {
                cancel(ed)?;
            } else {
                in_file(ed, file.clone(), move |e| {
                    if let Some(l) = locate_clock(e, &rec) {
                        super::delete_lines(e, l, 1);
                        if l > 0 {
                            remove_empty_drawer_at(e, l - 1);
                        }
                    }
                });
            }
            if restart && !clocking_in {
                let c = clock.clone();
                in_file(ed, file, move |e| {
                    if let Some(h) = e_heading_of(e, &c) {
                        e.set_cursor(h, 0);
                        let _ = clock_in(e, Prefix::None, None);
                    }
                });
            }
        }
        To::Now => {
            if restart {
                return Err("RESTART is not valid here".into());
            }
            if close || clocking_in {
                clock_out_open(ed, clock, None, fail_quietly)?;
            } else if !active {
                let c = clock.clone();
                in_file(ed, file, move |e| {
                    if let Some(l) = locate_clock(e, &c.running()) {
                        e.set_cursor(l, 0);
                        let _ = clock_in_with(e, Prefix::None, None, true);
                    }
                });
            }
        }
        To::At(t) => {
            if t > civil::now_epoch() {
                return Err("RESOLVE-TO must refer to a time in the past".into());
            }
            if restart {
                return Err("RESTART is not valid here".into());
            }
            let c = clock.clone();
            let heading = in_heading_hint(ed, clock);
            clock_out_open(ed, clock, Some(clock_out_time.unwrap_or(t)), fail_quietly)?;
            if clocking_in {
            } else if close {
                with(|s| s.leftover = clock_out_time.is_none().then_some(t));
            } else {
                let start = clock_out_time.map(|_| t);
                in_file(ed, c.file.clone(), move |e| {
                    let h = heading.and_then(|m| locate(e, &m));
                    if let Some(h) = h {
                        e.set_cursor(h, 0);
                        let _ = clock_in(e, Prefix::None, start);
                    }
                });
            }
        }
    }
    Ok(())
}

/// The heading of an open clock, when its buffer is this one.
fn e_heading_of(e: &Editor, c: &Open) -> Option<usize> {
    locate_clock(e, &c.running()).and_then(|l| fold::back_to_heading(e, l))
}

fn in_heading_hint(ed: &Editor, c: &Open) -> Option<Marker> {
    if same_file(ed, &c.file) {
        return e_heading_of(ed, c).map(|h| marker(ed, h));
    }
    let text = std::fs::read_to_string(c.file.as_ref()?).ok()?;
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let h = syntax::heading_at_or_before(&lines, c.line)?;
    Some(Marker {
        file: c.file.clone(),
        line: h,
        heading: lines[h].clone(),
        category: String::new(),
        label: String::new(),
    })
}

/// org-clock-clock-out on an open clock (the active one or another).
fn clock_out_open(
    ed: &mut Editor,
    c: &Open,
    at: Option<i64>,
    fail_quietly: bool,
) -> Result<(), String> {
    if is_active(c) {
        return clock_out(ed, None, fail_quietly, at);
    }
    let rec = c.running();
    with(|s| s.out_time = Some(at.unwrap_or_else(|| now_rounded(false).epoch())));
    in_file(ed, c.file.clone(), move |e| {
        if let Err(err) = clock_out_record(e, &rec, at, None, fail_quietly, true) {
            e.set_err(err);
        }
    });
    Ok(())
}

// ---- clock lines ----

/// org-clock-update-time-maybe: true when line `l` is a CLOCK line it
/// handled.
pub fn update_time_maybe(ed: &mut Editor, l: usize) -> bool {
    if super::time::clock_update_time_maybe(ed, l) {
        return true;
    }
    let t = line(ed, l);
    let re = crate::org_re!(r"^[ \t]*CLOCK: *[\[<]([^\]>]+)[\]>][ \t]*$");
    if let Some(c) = re.captures(&t)
        && let Some(mut r) = running()
        && same_file(ed, &r.task.file)
        && r.clock_line == l
        && let Ok(start) = stamp::time_string_to_seconds(&c[1])
    {
        r.start = start;
        r.stamp = t
            .trim_start()
            .trim_start_matches("CLOCK:")
            .trim()
            .to_owned();
        with(|s| {
            s.running = Some(r);
            s.shown = (-1, -1);
        });
    }
    false
}

/// org-clock-timestamps-change.
pub fn timestamps_change(ed: &mut Editor, up: bool, n: i64) -> Result<(), String> {
    let (l, b) = (ed.cur.line, ed.cur.byte);
    let t = line(ed, l);
    let Some((_, part)) = stamp::at_timestamp(&t, b) else {
        return Ok(());
    };
    use super::time::civil::Unit;
    let unit = match part {
        stamp::Part::Bracket | stamp::Part::After => return Ok(()),
        stamp::Part::Year => Unit::Year,
        stamp::Part::Month => Unit::Month,
        stamp::Part::Hour => Unit::Hour,
        stamp::Part::Minute => Unit::Minute,
        _ => Unit::Day,
    };
    let re = crate::org_re!(r"^[ \t]*CLOCK:[ \t]*(\[[^\]\n]+\])--(\[[^\]\n]+\])");
    if !re.is_match(&t) {
        return Ok(());
    }
    let change = if up { n } else { -n };
    let (first, second) = if change > 0 { (2, 1) } else { (1, 2) };
    let opts = stamp::ChangeOpts {
        updown: true,
        ..stamp::ChangeOpts::default()
    };
    for stop in [first, second] {
        let t = line(ed, l);
        let Some(c) = re.captures(&t) else { break };
        let g = c.get(stop).unwrap();
        let ch = stamp::change(
            g.as_str(),
            1,
            stamp::Part::Day,
            change,
            Some(stamp::What::Unit(unit)),
            opts,
        )?;
        super::set_line(
            ed,
            l,
            &format!("{}{}{}", &t[..g.start()], ch.text, &t[g.end()..]),
        );
    }
    super::time::clock_update_time_maybe(ed, l);
    ed.set_cursor(l, b.min(line(ed, l).len()));
    Ok(())
}

// ---- org-clock-display ----

/// The org-clock-display overlays of a buffer: heading line -> minutes,
/// for buffer version `.0`.
pub type Overlays = Option<(u64, Vec<(usize, i64)>)>;

/// org-clock-display for the range of ARG.
fn display_cmd(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    remove_overlays(ed);
    match arg {
        Prefix::None => {
            let range = opt_sym("org-clock-display-default-range").unwrap_or("thisyear".into());
            display_range(
                ed,
                &range,
                "Total file time: %s (%d hours and %d minutes)",
                true,
            )
        }
        Prefix::U(1) => display_range(
            ed,
            "today",
            "Total file time for today: %s (%d hours and %d minutes)",
            true,
        ),
        Prefix::U(2) => {
            let keys = [
                "today",
                "yesterday",
                "thisweek",
                "lastweek",
                "thismonth",
                "lastmonth",
                "thisyear",
                "lastyear",
                "interactive",
            ];
            super::complete(
                ed,
                "Range: ",
                keys.iter().map(|s| (*s).to_owned()).collect(),
                true,
                |ed, k| {
                    if let Err(e) = display_range(
                        ed,
                        &k,
                        "Total file time (custom): %s (%d hours and %d minutes)",
                        true,
                    ) {
                        ed.set_err(e);
                    }
                },
            );
            Ok(())
        }
        Prefix::U(3) => display_sum(
            ed,
            None,
            "Total file time: %s (%d hours and %d minutes)",
            false,
        ),
        _ => display_sum(
            ed,
            None,
            "Total file time: %s (%d hours and %d minutes)",
            true,
        ),
    }
}

fn display_range(ed: &mut Editor, key: &str, msg: &'static str, show: bool) -> Result<(), String> {
    if key == "interactive" {
        let o = super::time::ReadOpts {
            prompt: Some("Range start? ".into()),
            ..super::time::ReadOpts::default()
        };
        super::time::read_date(ed, o, move |ed, a| {
            let s = a.tm.norm().epoch() as f64;
            let o = super::time::ReadOpts {
                prompt: Some("Range end? ".into()),
                ..super::time::ReadOpts::default()
            };
            super::time::read_date(ed, o, move |ed, b| {
                let e = b.tm.norm().epoch() as f64;
                let _ = display_sum(ed, Some((s, e)), msg, show);
            });
        });
        return Ok(());
    }
    let r = sum::special_range(key, civil::now(), None, None)?;
    display_sum(ed, Some(r.seconds()), msg, show)
}

fn display_sum(
    ed: &mut Editor,
    range: Option<(f64, f64)>,
    msg: &str,
    show: bool,
) -> Result<(), String> {
    let (lo, hi) = fold::narrowed(ed).unwrap_or((0, ed.line_count()));
    let clocking = clocking_in(&ed.buf, ed.path.as_deref());
    let s = sum::sum(
        &ed.buf,
        lo,
        hi,
        range.map(|r| r.0),
        range.map(|r| r.1),
        None,
        clocking,
    );
    if show {
        let v = ed.buf.version;
        if let Some(o) = &mut ed.org {
            o.clock_overlays = Some((v, s.minutes.iter().map(|(a, b)| (*a, *b)).collect()));
        }
    }
    let (h, m) = (s.total / 60, s.total % 60);
    ed.set_msg(
        msg.replacen("%s", &dur(s.total), 1)
            .replacen("%d", &h.to_string(), 1)
            .replacen("%d", &m.to_string(), 1),
    );
    Ok(())
}

/// org-clock-remove-overlays.
pub fn remove_overlays(ed: &mut Editor) {
    if let Some(o) = &mut ed.org {
        o.clock_overlays = None;
    }
}

/// The display of heading line `l` under org-clock-display (the
/// org-clock-put-overlay text), if any.
pub fn display(ed: &Editor, l: usize, text: &str) -> Option<String> {
    let o = ed.org.as_ref()?;
    let (v, list) = o.clock_overlays.as_ref()?;
    if *v != ed.buf.version && options::bool("org-remove-highlights-with-change", true) {
        return None;
    }
    let (_, time) = list.iter().find(|(x, _)| *x == l)?;
    let st = super::settings(ed);
    let h = syntax::headline(text, &st)?;
    let start = h.title.start;
    let end = h
        .tags_range
        .as_ref()
        .map_or(text.len(), |r| r.start)
        .max(start);
    let headline = &text[start..end];
    let col = unicode_width::UnicodeWidthStr::width(&text[..start]);
    let w = super::dblock::string_width(headline);
    let dots = 60usize.saturating_sub(col + w);
    Some(format!(
        "{}{headline}{} {:>9} ",
        &text[..start],
        "·".repeat(dots),
        dur(*time)
    ))
}

// ---- persistence ----

fn persist() -> String {
    match sexp::option("org-clock-persist") {
        None | Some(sexp::Sexp::Nil) => "nil".into(),
        Some(sexp::Sexp::T) => "t".into(),
        Some(v) => v.str().or(v.sym()).unwrap_or("t").to_owned(),
    }
}

/// org-clock-persist-file: the option, or Fred's state directory.
pub fn persist_file() -> PathBuf {
    match options::get("org-clock-persist-file") {
        Some(toml::Value::String(s)) => options::expand(&s),
        _ => crate::swap::swap_dir().parent().map_or_else(
            || PathBuf::from("org-clock-save.el"),
            |d| d.join("org-clock-save.el"),
        ),
    }
}

fn quote(s: &str) -> String {
    format!("{:?}", s)
}

fn marker_sexp(m: &Marker) -> Option<String> {
    let f = m.file.as_ref()?;
    Some(format!(
        "({} {} {})",
        quote(&f.display().to_string()),
        m.line + 1,
        quote(&m.heading)
    ))
}

/// org-clock-save: the running clock and the history, as Lisp.
pub fn save() {
    let p = persist();
    if p == "nil" || cfg!(test) {
        return;
    }
    let file = persist_file();
    let (used, loaded) = with(|s| (s.has_been_used, s.loaded));
    if !(loaded || used || !file.exists()) {
        return;
    }
    let mut out = format!(
        ";; {} - fred at {}  -*- lexical-binding: t; -*-\n",
        file.file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default(),
        civil::format("[%Y-%m-%d %a %H:%M]", civil::now())
    );
    if (p == "t" || p == "clock")
        && let Some(r) = running()
        && let Some(m) = marker_sexp(&r.task)
    {
        out.push_str(&format!("(setq org-clock-stored-resume-clock '{m})\n"));
    }
    if p == "t" || p == "history" {
        let h: Vec<String> = history().iter().rev().filter_map(marker_sexp).collect();
        if !h.is_empty() {
            out.push_str(&format!(
                "(setq org-clock-stored-history '({}))\n",
                h.join(" ")
            ));
        }
    }
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(file, out);
}

fn marker_of(v: &sexp::Sexp) -> Option<Marker> {
    let l = v.list()?;
    Some(Marker {
        file: Some(PathBuf::from(l.first()?.str()?)),
        line: (l.get(1)?.int()? - 1).max(0) as usize,
        heading: l.get(2)?.str()?.to_owned(),
        category: String::new(),
        label: String::new(),
    })
}

/// org-clock-load: restore the history and offer to resume the clock.
fn load(s: &mut Session) {
    with(|st| st.loaded = true);
    if persist() == "nil" {
        return;
    }
    let file = persist_file();
    let Ok(text) = std::fs::read_to_string(&file) else {
        s.ed.set_msg(format!(
            "Not restoring clock data; {:?} not found",
            file.display().to_string()
        ));
        return;
    };
    s.ed.set_msg("Restoring clock data");
    let mut resume = None;
    for form in text.lines().filter(|l| l.starts_with("(setq ")) {
        let Ok(v) = sexp::read(form) else { continue };
        let items = v.list().unwrap_or(&[]);
        let (Some(var), Some(val)) = (items.get(1).and_then(|x| x.sym()), items.get(2)) else {
            continue;
        };
        let val = sexp::unquote(val.clone());
        match var {
            "org-clock-stored-history" => {
                for m in val.list().unwrap_or(&[]).iter().filter_map(marker_of) {
                    if m.file.as_ref().is_some_and(|f| f.exists()) {
                        history_push(m);
                    }
                }
            }
            "org-clock-stored-resume-clock" => resume = marker_of(&val),
            _ => {}
        }
    }
    let Some(m) = resume.filter(|m| m.file.as_ref().is_some_and(|f| f.exists())) else {
        return;
    };
    let st = super::settings(&s.ed);
    let q = format!("Resume clock ({})?", title(&m.heading, &st));
    let go = move |ed: &mut Editor| {
        let p = m.file.clone().unwrap();
        push_effect(ed, move |s| {
            run_in(s, &p, |e| {
                let Some(h) = locate(e, &m) else { return };
                e.set_cursor(h, 0);
                let old = options::get("org-clock-auto-clock-resolution");
                options::put(
                    "org-clock-auto-clock-resolution",
                    toml::Value::Boolean(false),
                );
                let o = InOpts {
                    resume: true,
                    auto_restart: true,
                    ..InOpts::default()
                };
                if let Err(err) = clock_in_at(e, h, o) {
                    e.set_err(err);
                }
                match old {
                    Some(v) => options::put("org-clock-auto-clock-resolution", v),
                    None => options::remove("org-clock-auto-clock-resolution"),
                }
            })
        });
    };
    if options::bool("org-clock-persist-query-resume", true) {
        super::yes_or_no(&mut s.ed, &q, go);
    } else {
        go(&mut s.ed);
    }
}

// ---- refile and properties ----

/// The running clock's heading as a refile target (org-refile with 2).
pub fn target(s: &Session) -> Option<super::refile::Target> {
    let r = running()?;
    let file = r.task.file.clone()?;
    let text = s.org_text(&file).ok()?;
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let n = lines.len();
    let l = if r.clock_line < n && is_open_clock(&lines[r.clock_line], &r.stamp) {
        r.clock_line
    } else {
        (0..n).find(|&i| is_open_clock(&lines[i], &r.stamp))?
    };
    let h = syntax::heading_at_or_before(&lines, l)?;
    Some(super::refile::Target {
        name: r.heading.clone(),
        file,
        line: Some(h),
        heading: lines[h].clone(),
    })
}

/// CLOCKSUM and CLOCKSUM_T of heading `h` (org-entry-get after
/// org-clock-sum / org-clock-sum-today).
pub fn clocksum<L: syntax::Lines + ?Sized>(
    b: &L,
    h: usize,
    file: Option<&Path>,
    today: bool,
) -> Option<String> {
    let range = if today {
        Some(
            sum::special_range("today", civil::now(), None, None)
                .ok()?
                .seconds(),
        )
    } else {
        None
    };
    let m = sum::subtree_minutes(b, h, range, clocking_in(b, file));
    (m > 0).then(|| dur(m))
}

/// org-clock-sum-current-item (org-sort-entries' `k`).
pub fn minutes(ed: &Editor, h: usize) -> i64 {
    sum::subtree_minutes(&ed.buf, h, None, clocking_in(&ed.buf, ed.path.as_deref()))
}

// ---- commands ----

/// This module's interactive commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-clock-in" => clock_in(ed, arg, None),
        "org-clock-out" => {
            if arg.is_none() {
                clock_out(ed, None, false, None)
            } else if running().is_none() {
                Err("No active clock".into())
            } else {
                let kws: Vec<String> = super::settings(ed)
                    .todo_names()
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect();
                super::complete(ed, "Switch to state: ", kws, true, |ed, s| {
                    let s = if s.is_empty() { "DONE".to_owned() } else { s };
                    ed.undo.begin(ed.cur.pos());
                    if let Err(e) = clock_out(ed, Some(s), false, None) {
                        ed.set_err(e);
                    }
                    ed.undo.end(ed.cur.pos());
                });
                Ok(())
            }
        }
        "org-clock-out-if-current" => out_if_current(ed),
        "org-clock-cancel" => cancel(ed),
        "org-clock-goto" => goto(ed, !arg.is_none()),
        "org-clock-in-last" => in_last(ed, arg),
        "org-clock-mark-default-task" => mark_default_task(ed),
        "org-clock-select-task" => select_task(ed, "Select task for clocking:", |ed, m| {
            if let Some(m) = m {
                ed.set_msg(format!("Selected: {}", m.label));
            }
        }),
        "org-clock-display" => display_cmd(ed, arg),
        "org-clock-remove-overlays" => {
            remove_overlays(ed);
            Ok(())
        }
        "org-clock-update-time-maybe" => {
            update_time_maybe(ed, ed.cur.line);
            Ok(())
        }
        "org-clock-timestamps-up" => timestamps_change(ed, true, arg.value()),
        "org-clock-timestamps-down" => timestamps_change(ed, false, arg.value()),
        "org-resolve-clocks" => {
            let clocks = open_clocks(ed);
            resolve_all(ed, clocks, None, !arg.is_none(), |_| {});
            Ok(())
        }
        "org-clock-modify-effort-estimate" => {
            if running().is_none() {
                ed.set_msg("Clock is not currently active");
                return Some(Ok(()));
            }
            match super::take_call_arg() {
                Some(v) => modify_effort(ed, v),
                None => {
                    let cur = running().and_then(|r| r.effort);
                    let p = format!(
                        "Set effort (hh:mm or mm{}): ",
                        cur.map_or(String::new(), |c| format!(", prefix + to add to {c}"))
                    );
                    super::read(ed, &p, "", |ed, v| {
                        ed.undo.begin(ed.cur.pos());
                        modify_effort(ed, v);
                        ed.undo.end(ed.cur.pos());
                    });
                }
            }
            Ok(())
        }
        "org-clock-toggle-auto-clockout" => {
            let on = !with(|s| s.auto_clockout)
                .unwrap_or_else(|| options::bool("org-clock-auto-clockout-insinuate", false));
            with(|s| s.auto_clockout = Some(on));
            ed.set_msg(if on {
                "Auto clock-out after idle time turned on"
            } else {
                "Auto clock-out after idle time turned off"
            });
            Ok(())
        }
        "org-clock-menu" => {
            let entries = vec![
                ("o".into(), "Clock out".into()),
                ("e".into(), "Change effort estimate".into()),
                ("g".into(), "Go to clock entry".into()),
                ("s".into(), "Switch task".into()),
            ];
            super::menu(ed, "Clock", entries, |ed, k| {
                let (cmd, a) = match k.as_str() {
                    "o" => ("org-clock-out", Prefix::None),
                    "e" => ("org-clock-modify-effort-estimate", Prefix::None),
                    "g" => ("org-clock-goto", Prefix::None),
                    _ => ("org-clock-in", Prefix::U(1)),
                };
                super::run(ed, cmd, a);
            });
            Ok(())
        }
        "org-clock-report" => table::report(ed, arg),
        "org-clocktable-shift-up" => table::shift(ed, "up", arg),
        "org-clocktable-shift-down" => table::shift(ed, "down", arg),
        "org-clocktable-shift-left" => table::shift(ed, "left", arg),
        "org-clocktable-shift-right" => table::shift(ed, "right", arg),
        "org-clocktable-try-shift" => table::shift(ed, "right", arg),
        "org-clock-sum" => display_sum(
            ed,
            None,
            "Total file time: %s (%d hours and %d minutes)",
            false,
        ),
        "org-clock-sum-today" => display_range(
            ed,
            "today",
            "Total file time for today: %s (%d hours and %d minutes)",
            false,
        ),
        "org-clock-save" => {
            save();
            Ok(())
        }
        _ => return None,
    })
}
