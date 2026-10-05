//! The Org agenda: a port of org-agenda.el and org-habit.el.
//!
//! Agenda views are generated read-only buffers (like Magit's): an
//! editor whose `org_view` holds the [`View`], the model of every line
//! (its item, faces, date). The buffer text is regenerated from the view
//! (filters hide lines, bulk marks and new-time notes are drawn into it).
//! Entries remember their source as [`Loc`]s (file, line and the line's
//! text for re-finding after edits); commands on entries run the Org
//! command by name in the source buffer through the session, and its
//! prompts move to the agenda buffer (the minibuffer).
//!
//! Submodules: [`collect`] (the entry finders), [`format`] (prefix
//! format, sorting, time grid), [`views`] (agenda, todo, tags, search,
//! stuck, block agendas, the dispatcher), [`cmds`] (agenda buffer
//! commands), [`habit`] (org-habit), [`solar`] (sunrise, moon, dates).

#![allow(clippy::regex_creation_in_loops)]

macro_rules! re {
    ($s:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($s).unwrap())
    }};
}

// ponytail: work in progress. The finders (collect), views, buffer
// commands, habits and solar modules are not written yet; until they
// are, `command` answers nothing and org-agenda stays "not yet ported".
pub mod format;

/// org-habit-parse-todo's result: scheduled day, `.+` repeat in days,
/// deadline day, deadline repeat, done days, repeater type.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Habit {
    pub scheduled: i64,
    pub repeat: i64,
    pub deadline: Option<i64>,
    pub deadline_repeat: Option<i64>,
    pub done: Vec<i64>,
    pub repeat_type: String,
}

thread_local! {
    static WIDTH: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// window-max-chars-per-line for the agenda (tests pin it).
pub fn width() -> usize {
    WIDTH.with(std::cell::Cell::get).unwrap_or_else(|| {
        crossterm::terminal::size()
            .map_or(80, |(w, _)| w as usize)
            .saturating_sub(1)
    })
}

pub fn set_width(w: Option<usize>) {
    WIDTH.with(|c| c.set(w));
}

use super::Prefix;
use super::sexp::{self, Sexp};
use crate::editor::{Editor, Mode};
use std::cell::RefCell;
use std::path::PathBuf;

/// A line of a source file: path, line number and the line's text when
/// it was seen (a marker that survives edits elsewhere).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loc {
    pub path: PathBuf,
    pub line: usize,
    pub text: String,
}

impl Loc {
    /// The line `self` is at now in `lines`: its old number if the text
    /// still matches there, else the nearest line with the same text.
    pub fn find(&self, lines: &[String]) -> Option<usize> {
        if lines.get(self.line).is_some_and(|l| *l == self.text) {
            return Some(self.line);
        }
        (1..lines.len().max(self.line + 1)).find_map(|d| {
            let up = self
                .line
                .checked_sub(d)
                .filter(|&i| lines.get(i).is_some_and(|l| *l == self.text));
            let down =
                Some(self.line + d).filter(|&i| lines.get(i).is_some_and(|l| *l == self.text));
            up.or(down)
        })
    }
}

/// Agenda faces (org-faces.el), resolved to styles when drawing.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Face {
    #[default]
    Default,
    Structure,
    StructureSecondary,
    StructureFilter,
    Date,
    DateToday,
    DateWeekend,
    DateWeekendToday,
    Scheduled,
    ScheduledToday,
    ScheduledPreviously,
    /// A deadline: the fraction of the warning period passed
    /// (org-agenda-deadline-faces).
    Deadline(f64),
    Done,
    Event,
    SexpEntry,
    Daterange,
    Diary,
    TimeGrid,
    CurrentTime,
    Warning,
    Dimmed,
    Clocking,
    Tag,
    Priority(u32),
    Todo,
    /// A TODO keyword (org-todo-keyword-faces of its file).
    Keyword,
    HabitClear,
    HabitClearFuture,
    HabitReady,
    HabitReadyFuture,
    HabitAlert,
    HabitAlertFuture,
    HabitOverdue,
    HabitOverdueFuture,
    BulkMark,
    NewTime,
}

/// How an item extracts its time of day (DOTIME of org-agenda-format-item).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Dotime {
    #[default]
    No,
    /// `'time` / t: search the text.
    Yes,
    /// A string searched before the text (a timestamp).
    Str(String),
}

/// One agenda item: a formatted line and the text properties upstream
/// keeps on it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// The whole line: prefix and text.
    pub line: String,
    /// Byte offset of the text after the prefix (`org-heading`).
    pub heading: usize,
    /// org-marker: the line the item came from.
    pub marker: Option<Loc>,
    /// org-hd-marker: its headline.
    pub hd: Option<Loc>,
    /// `type`: todo, tagsmatch, search, timestamp, sexp, block, scheduled,
    /// past-scheduled, deadline, upcoming-deadline, closed, state, clock,
    /// diary (with " scheduled"... suffixes from the sorting timestamp).
    pub kind: String,
    /// `date` (absolute day) and `ts-date`.
    pub date: Option<i64>,
    pub ts_date: Option<i64>,
    pub todo_state: Option<String>,
    pub priority: i64,
    pub urgency: i64,
    pub effort: Option<String>,
    pub effort_minutes: Option<f64>,
    /// `level`: spaces for the outline level.
    pub level: String,
    /// org-category (full, the prefix may truncate it).
    pub category: String,
    /// Tags, with whether each is inherited.
    pub tags: Vec<(String, bool)>,
    pub time_of_day: Option<i64>,
    pub duration: Option<f64>,
    pub breadcrumbs: String,
    /// `txt`: the text after the prefix.
    pub txt: String,
    /// `time`: the time string of the prefix.
    pub time: String,
    pub extra: String,
    pub dotime: Dotime,
    pub face: Face,
    pub undone_face: Option<Face>,
    pub done_face: Option<Face>,
    pub habit: Option<Habit>,
    pub warntime: Option<String>,
    /// The keywords of the item's file (org-todo-keywords-1) and which
    /// are done (for highlighting and todo-state sorting).
    pub todo_kwds: Vec<String>,
    pub done_kwds: Vec<String>,
    pub blocked: bool,
    /// The prefix format (agenda type) it was made with (`format`).
    pub fmt: &'static str,
    /// The heading text it was formatted from (for org-agenda-change-all-lines).
    pub head: String,
}

impl Item {
    pub fn tag_names(&self) -> Vec<String> {
        self.tags.iter().map(|(t, _)| t.clone()).collect()
    }

    /// org-agenda-get-any-marker.
    pub fn any_marker(&self) -> Option<&Loc> {
        self.hd.as_ref().or(self.marker.as_ref())
    }
}

/// What kind of line a view line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineKind {
    #[default]
    Text,
    /// Entry text shown by org-agenda-entry-text-mode.
    EntryText,
    /// A clocking issue (org-agenda-show-clocking-issues).
    Issue,
}

/// A line of the agenda buffer and its properties.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub text: String,
    pub item: Option<Item>,
    pub face: Face,
    /// Extra faces on byte ranges.
    pub spans: Vec<(std::ops::Range<usize>, Face)>,
    /// `day`: the absolute day of this part of the agenda.
    pub day: Option<i64>,
    pub day_cnt: Option<usize>,
    /// org-agenda-date-header / org-date-line.
    pub date_header: bool,
    pub today: bool,
    /// org-agenda-structural-header (block headers).
    pub structural: bool,
    /// Which block of the view this line belongs to.
    pub block: usize,
    /// Filters hiding this line (`org-filter-type`).
    pub hidden: Vec<&'static str>,
    pub kind: LineKind,
    /// org-agenda-show-new-time: " S => <date>" drawn at the right.
    pub new_time: Option<String>,
}

/// An agenda span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Day,
    Week,
    Fortnight,
    Month,
    Year,
    Days(i64),
}

impl Span {
    pub fn from_sexp(v: &Sexp) -> Option<Span> {
        match v {
            Sexp::Int(n) => Some(Span::Days(*n)),
            v => match v.str()? {
                "day" => Some(Span::Day),
                "week" => Some(Span::Week),
                "fortnight" => Some(Span::Fortnight),
                "month" => Some(Span::Month),
                "year" => Some(Span::Year),
                s => s.parse().ok().map(Span::Days),
            },
        }
    }

    /// org-agenda-ndays-to-span.
    pub fn from_days(n: i64) -> Span {
        match n {
            1 => Span::Day,
            7 => Span::Week,
            14 => Span::Fortnight,
            n => Span::Days(n),
        }
    }

    /// org-agenda-span-name.
    pub fn name(self) -> String {
        match self {
            Span::Day => "Day".into(),
            Span::Week => "Week".into(),
            Span::Fortnight => "Fortnight".into(),
            Span::Month => "Month".into(),
            Span::Year => "Year".into(),
            Span::Days(n) => format!("{n} days"),
        }
    }

    pub fn symbol(self) -> String {
        match self {
            Span::Days(n) => n.to_string(),
            s => s.name().to_lowercase(),
        }
    }

    /// org-agenda-span-to-ndays.
    pub fn ndays(self, start: i64) -> i64 {
        let t = crate::org::time::civil::Tm::from_absolute(start);
        match self {
            Span::Day => 1,
            Span::Week => 7,
            Span::Fortnight => 14,
            Span::Month => crate::org::time::civil::month_days(t.year, t.month),
            Span::Year => {
                if crate::org::time::civil::leap(t.year) {
                    366
                } else {
                    365
                }
            }
            Span::Days(n) => n,
        }
    }
}

/// The kind of a block (org-agenda-type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Agenda,
    Todo,
    Tags,
    Search,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Agenda => "agenda",
            Kind::Todo => "todo",
            Kind::Tags => "tags",
            Kind::Search => "search",
        }
    }
}

/// How a block is (re)built: the command and its last arguments
/// (org-redo-cmd, org-last-args).
#[derive(Clone, Debug, PartialEq)]
pub enum Spec {
    /// org-agenda-list: ARG, START-DAY (absolute), SPAN, WITH-HOUR.
    List {
        arg: Option<i64>,
        start: Option<i64>,
        span: Option<Span>,
        with_hour: bool,
    },
    /// org-todo-list: the keyword selection ("TODO|NEXT", "*") or a number.
    Todo {
        keywords: Option<String>,
        n: Option<i64>,
    },
    /// org-tags-view.
    Tags { todo_only: bool, matcher: String },
    /// org-search-view.
    Search { todo_only: bool, string: String },
    /// org-agenda-list-stuck-projects.
    Stuck,
}

impl Spec {
    pub fn kind(&self) -> Kind {
        match self {
            Spec::List { .. } => Kind::Agenda,
            Spec::Todo { .. } => Kind::Todo,
            Spec::Tags { .. } | Spec::Stuck => Kind::Tags,
            Spec::Search { .. } => Kind::Search,
        }
    }
}

/// Option bindings of a custom command (`((org-agenda-span 'day) ...)`).
pub type Bindings = Vec<(String, Sexp)>;

/// One block of a view.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub spec: Spec,
    pub lprops: Bindings,
}

/// A block agenda (org-agenda-run-series): its name, commands and
/// global settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: String,
    pub cmds: Vec<(String, Sexp, Bindings)>,
    pub gprops: Bindings,
}

/// Modes kept per agenda buffer (org-agenda-local-vars).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Modes {
    pub follow: bool,
    pub entry_text: Option<i64>,
    pub clockreport: bool,
    /// org-agenda-show-log: nil, t, only, clockcheck or a list of items.
    pub show_log: Option<Sexp>,
    /// org-agenda-archives-mode: nil, trees, t.
    pub archives: Option<&'static str>,
    /// Include inactive timestamps for the next build (`[`).
    pub inactive: bool,
}

impl Modes {
    /// The org-agenda-start-with-* defaults.
    pub fn start() -> Modes {
        Modes {
            follow: opt_bool("org-agenda-start-with-follow-mode", false),
            entry_text: opt_bool("org-agenda-start-with-entry-text-mode", false)
                .then(|| opt_int("org-agenda-entry-text-maxlines", 5)),
            clockreport: opt_bool("org-agenda-start-with-clockreport-mode", false),
            show_log: sexp::option("org-agenda-start-with-log-mode").filter(Sexp::truthy),
            archives: match sexp::option("org-agenda-start-with-archives-mode") {
                Some(Sexp::T) => Some("t"),
                Some(v) if v.truthy() => Some("trees"),
                _ => None,
            },
            inactive: false,
        }
    }
}

/// The agenda filters (tag, category, regexp, effort, top headline).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Filters {
    pub tag: Vec<String>,
    pub category: Vec<String>,
    pub regexp: Vec<String>,
    pub effort: Vec<String>,
    pub top: Option<(String, bool)>,
}

impl Filters {
    pub fn any(&self) -> bool {
        !self.tag.is_empty()
            || !self.category.is_empty()
            || !self.regexp.is_empty()
            || !self.effort.is_empty()
            || self.top.is_some()
    }
}

/// An agenda buffer.
#[derive(Clone, Debug, Default)]
pub struct View {
    /// The buffer name (`*Org Agenda*`, `*Org Agenda(a)*` when sticky).
    pub name: String,
    pub lines: Vec<Line>,
    pub blocks: Vec<Block>,
    pub series: Option<Series>,
    pub modes: Modes,
    pub filters: Filters,
    /// org-agenda-filters-preset.
    pub presets: Filters,
    pub filtered_by_category: bool,
    /// org-agenda-bulk-marked-entries (headline locations).
    pub marks: Vec<Loc>,
    /// Source files changed by agenda commands, newest last (org-agenda-undo-list).
    pub undo: Vec<PathBuf>,
    /// Buffer line to view line.
    pub map: Vec<usize>,
    pub sticky: bool,
    /// org-keys: the dispatcher keys of the command shown.
    pub keys: Option<String>,
    pub last_search_boolean: bool,
    pub query: Option<String>,
    pub cycle_counter: i64,
    pub width: usize,
    /// The mode line (org-agenda-set-mode-name).
    pub mode_name: String,
    /// The line of the last item shown in another window (`SPC` repeats scroll).
    pub show_window: bool,
    pub bulk_persistent: bool,
}

impl View {
    /// The buffer text: visible lines, with bulk marks and new times.
    pub fn render(&mut self) -> String {
        self.map.clear();
        let mut out = String::new();
        let mark = opt_string("org-agenda-bulk-mark-char", ">");
        for (i, l) in self.lines.iter().enumerate() {
            if !l.hidden.is_empty() {
                continue;
            }
            if !self.map.is_empty() {
                out.push('\n');
            }
            self.map.push(i);
            let mut t = l.text.clone();
            if let Some(it) = &l.item
                && let Some(hd) = &it.hd
                && self.marks.contains(hd)
            {
                let rest: String = t.chars().skip(2).collect();
                t = format!("{mark} {rest}");
            }
            if let Some(s) = &l.new_time {
                let col = self.width.saturating_sub(s.chars().count()).max(1) - 1;
                let mut head: String = t.chars().take(col).collect();
                let n = head.chars().count();
                if n < col {
                    head.push_str(&" ".repeat(col - n));
                }
                t = format!("{head}{s}");
            }
            out.push_str(&t);
        }
        out
    }

    /// The view line of buffer line `l`.
    pub fn at(&self, l: usize) -> Option<usize> {
        self.map.get(l).copied()
    }

    /// The buffer line showing view line `i` (or the nearest one above).
    pub fn buffer_line(&self, i: usize) -> usize {
        match self.map.binary_search(&i) {
            Ok(b) => b,
            Err(b) => b.saturating_sub(1),
        }
    }

    pub fn kind_at(&self, i: usize) -> Option<Kind> {
        let b = self.lines.get(i).map_or(0, |l| l.block);
        self.blocks.get(b).map(|b| b.spec.kind())
    }
}

// ---- options ----

pub fn opt(name: &str) -> Option<Sexp> {
    sexp::option(name)
}

pub fn opt_bool(name: &str, default: bool) -> bool {
    opt(name).map_or(default, |v| v.truthy())
}

pub fn opt_int(name: &str, default: i64) -> i64 {
    opt(name).and_then(|v| v.int()).unwrap_or(default)
}

pub fn opt_string(name: &str, default: &str) -> String {
    match opt(name) {
        Some(Sexp::Nil) => String::new(),
        Some(v) => v.str().map_or_else(|| v.to_string(), str::to_owned),
        None => default.into(),
    }
}

/// A symbol option: "nil", "t" or the symbol.
pub fn opt_sym(name: &str, default: &str) -> String {
    match opt(name) {
        Some(Sexp::Nil) => "nil".into(),
        Some(Sexp::T) => "t".into(),
        Some(v) => v.str().map_or_else(|| v.to_string(), str::to_owned),
        None => default.into(),
    }
}

/// Lisp data as a config value (to bind an option for a command).
pub fn sexp_to_toml(v: &Sexp) -> toml::Value {
    match v {
        Sexp::Nil => toml::Value::Boolean(false),
        Sexp::T => toml::Value::Boolean(true),
        Sexp::Int(i) => toml::Value::Integer(*i),
        Sexp::Float(f) => toml::Value::Float(*f),
        Sexp::Str(s) if !s.starts_with(['(', '\'']) => toml::Value::String(s.clone()),
        Sexp::Sym(s) => toml::Value::String(s.clone()),
        v => toml::Value::String(format!("'{v}")),
    }
}

/// Evaluate the value form of a binding: quoted data, self-evaluating
/// atoms, and option names.
pub fn eval(v: &Sexp) -> Sexp {
    match v {
        Sexp::List(l) if l.len() == 2 && l[0].sym() == Some("quote") => l[1].clone(),
        Sexp::List(l) if l.first().and_then(Sexp::sym) == Some("list") => {
            Sexp::List(l[1..].iter().map(eval).collect())
        }
        Sexp::List(l) if l.first().and_then(Sexp::sym) == Some("concat") => Sexp::Str(
            l[1..]
                .iter()
                .map(|x| eval(x).str().unwrap_or("").to_owned())
                .collect(),
        ),
        Sexp::Sym(s) if s.starts_with(':') => v.clone(),
        Sexp::Sym(s) => opt(s).unwrap_or(Sexp::Nil),
        v => v.clone(),
    }
}

/// Bind options for the duration of `f` (cl-progv over option names).
pub fn with_bindings<R>(b: &Bindings, f: impl FnOnce() -> R) -> R {
    let saved: Vec<(String, Option<toml::Value>)> = b
        .iter()
        .map(|(k, _)| (k.clone(), super::options::get(k)))
        .collect();
    for (k, v) in b {
        super::options::put(k, sexp_to_toml(&eval(v)));
    }
    let r = f();
    for (k, v) in saved.into_iter().rev() {
        match v {
            Some(v) => super::options::put(&k, v),
            None => super::options::remove(&k),
        }
    }
    r
}

/// Parse `((var value) ...)` into bindings.
pub fn bindings(v: Option<&Sexp>) -> Bindings {
    v.and_then(Sexp::list)
        .unwrap_or(&[])
        .iter()
        .filter_map(|b| {
            let l = b.list()?;
            Some((
                l.first()?.sym()?.to_owned(),
                l.get(1).cloned().unwrap_or(Sexp::Nil),
            ))
        })
        .collect()
}

// ---- session-wide state ----

/// The agenda restriction (org-agenda-restrict and its markers).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Restriction {
    pub file: PathBuf,
    /// Lines `[begin, end)` of a subtree or region restriction.
    pub range: Option<(usize, usize)>,
    /// org-agenda-overriding-restriction: a lock (`subtree`, `file`).
    pub lock: Option<&'static str>,
}

#[derive(Default)]
struct Global {
    restriction: Option<Restriction>,
    /// org-agenda-new-buffers: files the agenda opened.
    new_buffers: Vec<PathBuf>,
    /// org-agenda-sticky.
    sticky: Option<bool>,
    /// Appointments added by org-agenda-to-appt: (time, text, warntime).
    appts: Vec<(String, String, Option<String>)>,
    /// The clock table text returned by the clock module.
    result: Option<String>,
}

thread_local! {
    static G: RefCell<Global> = RefCell::new(Global::default());
}

pub fn restriction() -> Option<Restriction> {
    G.with(|g| g.borrow().restriction.clone())
}

pub fn set_restriction(r: Option<Restriction>) {
    G.with(|g| g.borrow_mut().restriction = r);
}

pub fn sticky() -> bool {
    G.with(|g| g.borrow().sticky)
        .unwrap_or_else(|| opt_bool("org-agenda-sticky", false))
}

pub fn set_sticky(v: bool) {
    G.with(|g| g.borrow_mut().sticky = Some(v));
}

pub fn note_new_buffer(p: PathBuf) {
    G.with(|g| {
        let mut g = g.borrow_mut();
        if !g.new_buffers.contains(&p) {
            g.new_buffers.push(p);
        }
    });
}

pub fn take_new_buffers() -> Vec<PathBuf> {
    G.with(|g| std::mem::take(&mut g.borrow_mut().new_buffers))
}

pub fn appointments() -> Vec<(String, String, Option<String>)> {
    G.with(|g| g.borrow().appts.clone())
}

/// The clock module answers org-clock-get-clocktable here.
pub fn put_result(s: String) {
    G.with(|g| g.borrow_mut().result = Some(s));
}

pub fn take_result() -> Option<String> {
    G.with(|g| g.borrow_mut().result.take())
}

/// The agenda files: org-agenda-files, restricted, with archives when
/// `archives` (org-agenda-files nil 'ifmode).
pub fn files(unrestricted: bool, archives: bool) -> Vec<PathBuf> {
    let mut v = match restriction() {
        Some(r) if !unrestricted => vec![r.file],
        _ => super::agenda_files(),
    };
    if opt_bool("org-agenda-skip-unavailable-files", false) {
        v.retain(|f| f.is_file());
    }
    // ponytail: archive files (org-add-archive-files) join when the
    // views module lands.
    let _ = archives;
    let mut seen = vec![];
    v.retain(|f| {
        let c = std::fs::canonicalize(f).unwrap_or_else(|_| f.clone());
        if seen.contains(&c) {
            false
        } else {
            seen.push(c);
            true
        }
    });
    v
}

// ---- the editor side ----

/// The view of this buffer.
pub fn view(ed: &Editor) -> Result<&View, String> {
    ed.org_view
        .as_deref()
        .ok_or_else(|| "Not in an Org agenda buffer".to_owned())
}

pub fn view_mut(ed: &mut Editor) -> Result<&mut View, String> {
    ed.org_view
        .as_deref_mut()
        .ok_or_else(|| "Not in an Org agenda buffer".to_owned())
}

/// The view line at the cursor.
pub fn cur_line(ed: &Editor) -> Option<usize> {
    view(ed).ok()?.at(ed.cur.line)
}

/// The item at the cursor (org-get-at-bol 'org-marker), or
/// "Command not allowed in this line" (org-agenda-error).
pub fn item(ed: &Editor) -> Result<Item, String> {
    let v = view(ed)?;
    cur_line(ed)
        .and_then(|i| v.lines[i].item.clone())
        .filter(|it| it.marker.is_some() || it.hd.is_some())
        .ok_or_else(|| "Command not allowed in this line".to_owned())
}

/// Rewrite the buffer from the view, keeping the cursor on its view line.
pub fn redisplay(ed: &mut Editor) {
    let at = cur_line(ed);
    let col = ed.cur.byte;
    let Some(v) = ed.org_view.as_deref_mut() else {
        return;
    };
    let text = v.render();
    let line = at.map_or(0, |i| v.buffer_line(i));
    ed.buf = crate::buffer::Buffer::from_text(&text);
    ed.undo = crate::undo::Undo::default();
    ed.folds.clear();
    ed.readonly = true;
    let n = ed.line_count().saturating_sub(1);
    ed.set_cursor(line.min(n), 0);
    let len = ed.buf.line(ed.cur.line).len();
    ed.cur.byte = col.min(len);
    ed.clamp_cursor();
}

/// Every agenda command name (org-agenda.el and org-habit.el), for `:org-` completion.
pub const NAMES: &[&str] = &[
    "org-agenda",
    "org-agenda-list",
    "org-todo-list",
    "org-tags-view",
    "org-search-view",
    "org-agenda-list-stuck-projects",
    "org-agenda-append-agenda",
    "org-store-agenda-views",
    "org-agenda-write",
    "org-agenda-archives-mode",
    "org-agenda-dim-blocked-tasks",
    "org-agenda-end-of-line",
    "org-agenda-fontify-priorities",
    "org-agenda-menu",
    "org-agenda-follow-mode",
    "org-agenda-entry-text-mode",
    "org-agenda-clockreport-mode",
    "org-toggle-sticky-agenda",
    "org-agenda-mode",
    "org-agenda-undo",
    "org-agenda-entry-text-show",
    "org-agenda-show-clocking-issues",
    "org-agenda-limit-interactively",
    "org-agenda-set-restriction-lock-from-agenda",
    "org-agenda-set-restriction-lock",
    "org-agenda-remove-restriction-lock",
    "org-agenda-Quit",
    "org-agenda-quit",
    "org-agenda-exit",
    "org-agenda-kill-all-agenda-buffers",
    "org-agenda-execute",
    "org-agenda-redo",
    "org-agenda-redo-all",
    "org-agenda-filter-by-category",
    "org-agenda-filter-by-top-headline",
    "org-agenda-filter-by-regexp",
    "org-agenda-filter-by-effort",
    "org-agenda-filter",
    "org-agenda-filter-remove-all",
    "org-agenda-filter-by-tag",
    "org-agenda-manipulate-query-add",
    "org-agenda-manipulate-query-subtract",
    "org-agenda-manipulate-query-add-re",
    "org-agenda-manipulate-query-subtract-re",
    "org-agenda-goto-date",
    "org-agenda-goto-today",
    "org-agenda-backward-block",
    "org-agenda-forward-block",
    "org-agenda-later",
    "org-agenda-earlier",
    "org-agenda-view-mode-dispatch",
    "org-agenda-reset-view",
    "org-agenda-day-view",
    "org-agenda-week-view",
    "org-agenda-fortnight-view",
    "org-agenda-month-view",
    "org-agenda-year-view",
    "org-agenda-next-date-line",
    "org-agenda-previous-date-line",
    "org-agenda-log-mode",
    "org-agenda-toggle-diary",
    "org-agenda-toggle-deadlines",
    "org-agenda-toggle-time-grid",
    "org-agenda-next-line",
    "org-agenda-previous-line",
    "org-agenda-next-item",
    "org-agenda-previous-item",
    "org-agenda-show-tags",
    "org-agenda-goto",
    "org-agenda-kill",
    "org-agenda-archive-default",
    "org-agenda-archive-default-with-confirmation",
    "org-agenda-archive",
    "org-agenda-archive-to-archive-sibling",
    "org-agenda-archive-with",
    "org-agenda-refile",
    "org-agenda-open-link",
    "org-agenda-switch-to",
    "org-agenda-goto-mouse",
    "org-agenda-show",
    "org-agenda-show-and-scroll-up",
    "org-agenda-show-scroll-down",
    "org-agenda-show-1",
    "org-agenda-cycle-show",
    "org-agenda-recenter",
    "org-agenda-show-mouse",
    "org-agenda-tree-to-indirect-buffer",
    "org-agenda-todo-nextset",
    "org-agenda-todo-previousset",
    "org-agenda-todo",
    "org-agenda-add-note",
    "org-agenda-priority-up",
    "org-agenda-priority-down",
    "org-agenda-priority",
    "org-agenda-set-tags",
    "org-agenda-set-property",
    "org-agenda-set-effort",
    "org-agenda-toggle-archive-tag",
    "org-agenda-do-date-later",
    "org-agenda-do-date-earlier",
    "org-agenda-date-later",
    "org-agenda-date-earlier",
    "org-agenda-date-later-minutes",
    "org-agenda-date-earlier-minutes",
    "org-agenda-date-later-hours",
    "org-agenda-date-earlier-hours",
    "org-agenda-date-prompt",
    "org-agenda-schedule",
    "org-agenda-deadline",
    "org-agenda-clock-in",
    "org-agenda-clock-out",
    "org-agenda-clock-cancel",
    "org-agenda-clock-goto",
    "org-agenda-diary-entry",
    "org-agenda-phases-of-moon",
    "org-agenda-holidays",
    "org-agenda-sunrise-sunset",
    "org-agenda-goto-calendar",
    "org-calendar-goto-agenda",
    "org-agenda-convert-date",
    "org-agenda-bulk-mark",
    "org-agenda-bulk-mark-all",
    "org-agenda-bulk-mark-regexp",
    "org-agenda-bulk-unmark",
    "org-agenda-bulk-toggle-all",
    "org-agenda-bulk-toggle",
    "org-agenda-bulk-remove-overlays",
    "org-agenda-bulk-unmark-all",
    "org-agenda-bulk-action",
    "org-agenda-capture",
    "org-agenda-drag-line-forward",
    "org-agenda-drag-line-backward",
    "org-agenda-show-the-flagging-note",
    "org-agenda-to-appt",
    "org-agenda-todo-yesterday",
    "org-agenda-ctrl-c-ctrl-c",
    "org-habit-toggle-habits",
    "org-habit-toggle-display-in-agenda",
    "org-agenda-set-restriction-lock",
    "org-agenda-file-to-front",
    "org-remove-file",
    "org-cycle-agenda-files",
    "org-agenda-store-link",
    "org-batch-agenda",
    "org-batch-agenda-csv",
    "org-batch-store-agenda-views",
    "org-diary",
];

/// This module's interactive commands, by upstream name.
pub fn command(_ed: &mut Editor, _name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    None
}

/// Keys the agenda leaves to Vim: the `Space` leader, `:`, counts, `G`.
pub fn vim_keeps(ed: &Editor, k: crate::key::Key) -> bool {
    use crate::key::KeyCode;
    if k.ctrl || k.alt {
        return false;
    }
    match k.code {
        KeyCode::Char(' ') => ed.vim.pending.is_empty(),
        KeyCode::Char(':') | KeyCode::Char('G') | KeyCode::Esc => true,
        KeyCode::Char(c) if c.is_ascii_digit() => {
            c != '0'
                || ed
                    .vim
                    .pending
                    .iter()
                    .any(|p| p.char().is_some_and(|c| c.is_ascii_digit()))
        }
        _ => false,
    }
}

// ---- prompts of commands run in source buffers ----

/// An Org prompt in progress on an editor (the minibuffer state).
pub struct Prompt {
    mode: Mode,
    then: super::Then,
    menu: Vec<(String, String)>,
    typed: String,
    enter: Option<String>,
    require_match: bool,
}

/// Take the prompt an Org command left open on `ed`.
pub fn take_prompt(ed: &mut Editor) -> Option<Prompt> {
    let open = match &ed.mode {
        Mode::Command(cl) => cl.kind == 'o',
        Mode::Pick(p) => matches!(
            p.kind,
            crate::pick::Kind::OrgMenu | crate::pick::Kind::OrgChoice
        ),
        _ => false,
    };
    if !open {
        return None;
    }
    let then = ed.org_then.take()?;
    Some(Prompt {
        mode: std::mem::replace(&mut ed.mode, Mode::Normal),
        then,
        menu: std::mem::take(&mut ed.org_menu),
        typed: std::mem::take(&mut ed.org_menu_typed),
        enter: ed.org_menu_enter.take(),
        require_match: ed.org_require_match,
    })
}

/// Open `p` on `ed`, its answer going to `then` instead.
pub fn put_prompt(ed: &mut Editor, p: Prompt, then: super::Then) {
    ed.mode = p.mode;
    ed.org_menu = p.menu;
    ed.org_menu_typed = p.typed;
    ed.org_menu_enter = p.enter;
    ed.org_require_match = p.require_match;
    ed.org_then = Some(then);
    let _ = p.then;
}

impl Prompt {
    pub fn split(self) -> (super::Then, Prompt) {
        let then = self.then;
        let rest = Prompt {
            then: Box::new(|_, _| {}),
            ..self
        };
        (then, rest)
    }
}

/// Run `f` on the buffer visiting `at.path` with the cursor on `at`
/// (the Org command's buffer); `after` runs when it is done (also after
/// prompts it opens, which move to the agenda buffer).
pub fn remote(
    ed: &mut Editor,
    at: Loc,
    f: impl FnOnce(&mut Editor) -> Result<(), String> + Send + 'static,
    after: impl FnOnce(&mut crate::session::Session, Remote) + Send + 'static,
) {
    super::effect(ed, move |s| remote_now(s, at, Box::new(f), Box::new(after)));
}

/// What a remote command changed: the file, the headline line before
/// and after (the cursor's headline), and the line count change.
#[derive(Clone, Debug)]
pub struct Remote {
    pub path: PathBuf,
    pub old: Loc,
    pub head: Option<(usize, String)>,
    pub delta: i64,
    pub ok: bool,
}

type RunFn = Box<dyn FnOnce(&mut Editor) -> Result<(), String> + Send>;
type AfterFn = Box<dyn FnOnce(&mut crate::session::Session, Remote) + Send>;

/// Run `f` in the source buffer now (from a session effect).
pub fn remote_now(s: &mut crate::session::Session, at: Loc, f: RunFn, after: AfterFn) {
    let open = (0..s.org_buffer_count()).any(|i| {
        s.org_buffer_path(i)
            .is_some_and(|p| same_file(&p, &at.path))
    });
    let i = match s.org_buffer(&at.path) {
        Ok(i) => i,
        Err(e) => return s.ed.set_err(e),
    };
    if !open {
        note_new_buffer(at.path.clone());
    }
    let n0 = s.org_with_buffer(i, |src| src.line_count());
    let res = s.org_with_buffer(i, |src| {
        if src.org.is_none() {
            src.org = Some(Box::default());
        }
        let lines: Vec<String> = (0..src.line_count()).map(|l| src.buf.line(l)).collect();
        let Some(l) = at.find(&lines) else {
            return Err("Cannot find the entry in its file (it was changed or removed)".to_owned());
        };
        crate::org::fold::widen(src);
        src.set_cursor(l, 0);
        if src.folds.hidden(l) {
            src.reveal_cursor();
        }
        let pos = src.cur.pos();
        src.undo.begin(pos);
        let r = f(src);
        let pos = src.cur.pos();
        src.undo.end(pos);
        src.clamp_cursor();
        r
    });
    finish_remote(s, i, at, n0, res, after);
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(x, y)| x == y)
}

/// After a step of a remote command: forward an open prompt to the
/// agenda, perform session effects, else call `after`.
fn finish_remote(
    s: &mut crate::session::Session,
    i: usize,
    at: Loc,
    n0: usize,
    res: Result<(), String>,
    after: AfterFn,
) {
    let (prompt, eff, msg) = s.org_with_buffer(i, |src| {
        (take_prompt(src), src.pending_effect.take(), src.msg.take())
    });
    if let Some((m, err)) = msg {
        if err {
            s.ed.set_err(m)
        } else {
            s.ed.set_msg(m)
        }
    }
    if let Err(e) = &res {
        s.ed.set_err(e.clone());
    }
    if let Some(p) = prompt {
        let (orig, p) = p.split();
        let at2 = at.clone();
        let then: super::Then = Box::new(move |ed: &mut Editor, ans: String| {
            super::effect(ed, move |s| {
                let Ok(i) = s.org_buffer(&at2.path) else {
                    return;
                };
                s.org_with_buffer(i, |src| orig(src, ans));
                finish_remote(s, i, at2, n0, Ok(()), after);
            });
        });
        put_prompt(&mut s.ed, p, then);
        return;
    }
    if let Some(e) = eff {
        s.perform(e);
    }
    let (head, n1) = s.org_with_buffer(i, |src| {
        let h = super::syntax::heading_at_or_before(&src.buf, src.cur.line);
        (h.map(|h| (h, src.buf.line(h))), src.line_count())
    });
    let r = Remote {
        path: at.path.clone(),
        old: at,
        head,
        delta: n1 as i64 - n0 as i64,
        ok: res.is_ok(),
    };
    // Remember the change for org-agenda-undo.
    if let Some(v) = s.ed.org_view.as_deref_mut() {
        v.undo.push(r.path.clone());
    }
    after(s, r);
}
