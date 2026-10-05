//! Port of org-table.el and org-plot.el: see docs/org-port-notes.md
//! ("Module contracts").
//!
//! Commands run on a [`doc::Doc`] (the buffer's lines and point with the
//! Emacs primitives the upstream code uses); the changed lines go back to
//! the buffer as one undoable edit.
//!
//! Adaptations to Fred:
//! - Shrunk columns (org-table-toggle-column-width, `<N>` cookies) are
//!   kept per table and drawn truncated with `…` by [`display`] on every
//!   line but the cursor's, which shows the whole text while editing.
//! - Coordinates (org-table-toggle-coordinate-overlays) show `$N` labels in
//!   rules and the row designator after each row.
//! - org-table-header-line-mode shows the table's first row in place of
//!   the top screen line once the header has scrolled away.
//! - Field editing (org-table-edit-field) and the formula editor
//!   (org-table-edit-formulas) use prompts: one prompt for the field, and
//!   a picker over the formulas whose entries are edited in turn.
//! - Regions are linewise (Visual line mode or `:'<,'>`): a rectangle
//!   spans the region's rows in the current column.
//! - Calc and Emacs Lisp formulas run in Rust evaluators (calc.rs,
//!   lisp.rs); what those do not cover falls back to `emacs --batch`.

pub mod calc;
pub mod doc;
pub mod edit;
pub mod formula;
pub mod lisp;
pub mod orgtbl;
pub mod plot;
#[cfg(test)]
mod tests;

use super::{Prefix, options};
use crate::editor::{Editor, Mode};
use crate::key::Key;
use doc::Doc;
use formula::{EvalArg, Recalc};
use std::cell::RefCell;
use std::collections::HashMap;

/// Per-table-buffer state (Emacs buffer-local and global variables).
#[derive(Clone, Debug, Default)]
pub struct State {
    /// Shrunk columns per table, keyed by a line of the table.
    pub shrunk: Vec<(usize, Vec<usize>)>,
    /// org-table-last-alignment and org-table-last-column-widths.
    pub last_alignment: Vec<char>,
    pub last_widths: Vec<usize>,
    /// The table last aligned (org-table-aligned-begin/end-marker).
    pub aligned: Option<(usize, usize)>,
    /// org-table-may-need-update.
    pub may_need_update: bool,
    /// org-table-automatic-realign bound to nil.
    pub no_realign: bool,
    /// The buffer version after this module last wrote it.
    pub version: u64,
    /// org-table-overlay-coordinates.
    pub overlay_coordinates: bool,
    /// org-table-formula-debug and the substitution histories.
    pub formula_debug: bool,
    pub debug_log: Vec<String>,
    /// org-table-follow-field-mode.
    pub follow_field: bool,
    /// org-table-header-line-mode.
    pub header_line: bool,
    /// org-last-recalc-line and org-recalc-commands.
    pub last_recalc_line: Option<usize>,
    pub recalc_commands: Vec<String>,
    /// A formula-editor session (org-table-edit-formulas).
    pub fedit: Option<Fedit>,
    /// orgtbl-mode is on in this (non-Org) buffer.
    pub orgtbl: bool,
}

/// The formula editor: the table line it edits and its working text.
#[derive(Clone, Debug, Default)]
pub struct Fedit {
    pub source: usize,
    pub lines: Vec<String>,
    /// References shown as A1 (org-table-buffer-is-an).
    pub an: bool,
}

thread_local! {
    static STATES: RefCell<HashMap<String, State>> = RefCell::new(HashMap::new());
    /// org-table-clip (global, as upstream).
    static CLIP: RefCell<Option<Vec<Vec<String>>>> = const { RefCell::new(None) };
    /// The region given with `:'<,'>` (first and last line).
    static REGION: RefCell<Option<(usize, usize)>> = const { RefCell::new(None) };
}

impl State {
    pub fn shrunk_in(&self, b: usize, e: usize) -> Vec<usize> {
        self.shrunk.iter().find(|(a, _)| *a >= b && *a < e).map(|(_, c)| c.clone()).unwrap_or_default()
    }

    pub fn set_shrunk(&mut self, b: usize, e: usize, mut cols: Vec<usize>) {
        self.shrunk.retain(|(a, _)| !(*a >= b && *a < e));
        cols.sort_unstable();
        cols.dedup();
        cols.retain(|&c| c > 0);
        if !cols.is_empty() {
            self.shrunk.push((b, cols));
        }
    }

    pub fn take_clip(&mut self) -> Option<Vec<Vec<String>>> {
        CLIP.with(|c| c.borrow_mut().take())
    }

    pub fn put_clip(&mut self, v: Option<Vec<Vec<String>>>) {
        CLIP.with(|c| *c.borrow_mut() = v);
    }

    pub fn clip(&self) -> Option<Vec<Vec<String>>> {
        CLIP.with(|c| c.borrow().clone())
    }

    /// Lines `at..at+removed` became `inserted` lines.
    pub fn line_change(&mut self, at: usize, removed: usize, inserted: usize) {
        let shift = |l: usize| -> usize {
            if l >= at + removed {
                l + inserted - removed
            } else if l >= at {
                at
            } else {
                l
            }
        };
        for (a, _) in &mut self.shrunk {
            *a = shift(*a);
        }
        if let Some((b, e)) = self.aligned {
            self.aligned = Some((shift(b), shift(e)));
        }
        self.last_recalc_line = self.last_recalc_line.map(shift);
        if let Some(f) = &mut self.fedit {
            f.source = shift(f.source);
        }
    }
}

fn key_of(ed: &Editor) -> String {
    ed.path.as_ref().map_or_else(String::new, |p| p.display().to_string())
}

/// This buffer's table state.
pub fn state(ed: &Editor) -> State {
    STATES.with(|s| s.borrow().get(&key_of(ed)).cloned().unwrap_or_default())
}

fn put_state(ed: &Editor, st: State) {
    STATES.with(|s| s.borrow_mut().insert(key_of(ed), st));
}

/// Line structure changed in the buffer (keeps table anchors in place).
pub fn line_change(ed: &Editor, at: usize, removed: usize, inserted: usize) {
    STATES.with(|s| {
        if let Some(st) = s.borrow_mut().get_mut(&key_of(ed)) {
            st.line_change(at, removed, inserted);
        }
    });
}

/// The region of a `:'<,'>org-...` command.
pub fn set_region(lines: Option<(usize, usize)>) {
    REGION.with(|r| *r.borrow_mut() = lines);
}

// ---- options ----

pub fn opt_bool(name: &str, default: bool) -> bool {
    options::bool(name, default)
}

pub fn opt_int(name: &str, default: i64) -> i64 {
    options::int(name, default)
}

pub fn opt_string(name: &str, default: &str) -> String {
    options::string(name, default)
}

// ---- dates ----

/// Days from 1970-01-01 to the proleptic Gregorian date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The date `days` after 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The English day name abbreviation (C locale) of a day number.
pub fn weekday_name(days: i64) -> &'static str {
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize]
}

/// The local time zone's offset from UTC, in seconds.
pub fn local_offset_secs() -> i64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as libc::time_t);
    // SAFETY: localtime_r writes into the zeroed tm we own.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff
        }
    }
}

/// org-duration-to-minutes for `1d 3:00`-like durations.
pub fn duration_minutes(s: &str) -> Option<f64> {
    let re = doc::re(r"^\s*(?:([0-9.]+)\s*(min|h|d|w|m|y)\s*)+(?:([0-9]+):([0-9]{2})(?::([0-9]{2}))?)?\s*$");
    if !re.is_match(s) || !s.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let unit = doc::re(r"([0-9.]+)\s*(min|h|d|w|m|y)");
    let mut total = 0.0;
    for c in unit.captures_iter(s) {
        let v: f64 = c[1].parse().ok()?;
        total += v * match &c[2] {
            "min" => 1.0,
            "h" => 60.0,
            "d" => 1440.0,
            "w" => 10080.0,
            "m" => 43200.0,
            _ => 525960.0,
        };
    }
    if let Some(c) = doc::re(r"([0-9]+):([0-9]{2})(?::([0-9]{2}))?\s*$").captures(s) {
        total += c[1].parse::<f64>().ok()? * 60.0 + c[2].parse::<f64>().ok()? + c.get(3).map_or(0.0, |x| x.as_str().parse::<f64>().unwrap_or(0.0) / 60.0);
    }
    Some(total)
}

// ---- Emacs regexps ----

/// An Emacs regexp in the regex crate's syntax.
pub fn emacs_regex_src(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    let mut in_class = false;
    while let Some(c) = it.next() {
        if in_class {
            match c {
                ']' => {
                    in_class = false;
                    out.push(']');
                }
                '[' if it.peek() == Some(&':') => {
                    out.push('[');
                }
                '\\' => out.push_str("\\\\"),
                c => out.push(c),
            }
            continue;
        }
        match c {
            '\\' => match it.next() {
                Some('(') => {
                    if it.peek() == Some(&'?') {
                        it.next();
                        if it.peek() == Some(&':') {
                            it.next();
                        }
                        out.push_str("(?:");
                    } else {
                        out.push('(');
                    }
                }
                Some(')') => out.push(')'),
                Some('|') => out.push('|'),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                Some('`') => out.push_str("\\A"),
                Some('\'') => out.push_str("\\z"),
                Some('<') | Some('>') => out.push_str("\\b"),
                Some('b') => out.push_str("\\b"),
                Some('w') => out.push_str("\\w"),
                Some('W') => out.push_str("\\W"),
                Some('s') => match it.next() {
                    Some('-') | Some(' ') => out.push_str("\\s"),
                    Some('w') => out.push_str("\\w"),
                    _ => out.push_str("\\s"),
                },
                Some('S') => {
                    it.next();
                    out.push_str("\\S");
                }
                Some(d) if d.is_ascii_digit() => out.push_str(&format!("\\{d}")),
                Some(o) => {
                    out.push_str(&regex::escape(&o.to_string()));
                }
                None => out.push_str("\\\\"),
            },
            '(' | ')' | '|' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '[' => {
                in_class = true;
                out.push('[');
                if it.peek() == Some(&'^') {
                    out.push(it.next().unwrap());
                }
                if it.peek() == Some(&']') {
                    it.next();
                    out.push_str("\\]");
                }
            }
            c => out.push(c),
        }
    }
    out
}

pub fn emacs_regex(s: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(&format!("(?m){}", emacs_regex_src(s))).map_err(|e| format!("Invalid regexp: {e}"))
}

// ---- properties (org-entry-get with inheritance) ----

/// The value of property `prop` for the entry at line `l`.
pub fn entry_get(lines: &[String], l: usize, prop: &str, inherit: bool) -> Option<String> {
    use super::syntax::{heading_at_or_before, level};
    let find_in = |start: usize| -> Option<String> {
        let mut i = start;
        while i < lines.len() && lines[i].trim_start().starts_with(['S', 'D', 'C']) && super::ctx::is_planning(&lines[i]) {
            i += 1;
        }
        if i >= lines.len() || !lines[i].trim().eq_ignore_ascii_case(":PROPERTIES:") {
            return None;
        }
        for line in &lines[i + 1..] {
            let t = line.trim();
            if t.eq_ignore_ascii_case(":END:") || level(line).is_some() {
                break;
            }
            if let Some(rest) = t.strip_prefix(':')
                && let Some((k, v)) = rest.split_once(':')
                && k.eq_ignore_ascii_case(prop)
            {
                return Some(v.trim().to_owned());
            }
        }
        None
    };
    let mut h = heading_at_or_before(lines, l.min(lines.len().saturating_sub(1)));
    loop {
        match h {
            Some(hl) => {
                if let Some(v) = find_in(hl + 1) {
                    return Some(v);
                }
                if !inherit {
                    return None;
                }
                let lv = level(&lines[hl]).unwrap_or(1);
                h = (0..hl).rev().find(|&i| level(&lines[i]).is_some_and(|x| x < lv));
            }
            None => {
                let first = (0..lines.len()).find(|&i| !lines[i].trim().is_empty() && !lines[i].trim_start().starts_with('#'));
                if let Some(f) = first
                    && let Some(v) = find_in(f)
                {
                    return Some(v);
                }
                for line in lines {
                    let t = line.trim_start();
                    if t.len() > 11 && t[..11].eq_ignore_ascii_case("#+property:") {
                        let mut parts = t[11..].split_whitespace();
                        if parts.next().is_some_and(|k| k.eq_ignore_ascii_case(prop)) {
                            return Some(parts.collect::<Vec<_>>().join(" "));
                        }
                    }
                }
                return None;
            }
        }
    }
}

// ---- the editor bridge ----

/// The buffer as a Doc.
pub fn doc(ed: &Editor) -> Doc {
    let n = ed.line_count();
    let lines: Vec<String> = (0..n).map(|l| ed.buf.line(l)).collect();
    let mut st = state(ed);
    if st.version != ed.buf.version {
        st.may_need_update = true;
    }
    let region = REGION.with(|r| r.borrow_mut().take()).or(match ed.mode {
        Mode::VisualLine { anchor } => Some((anchor.min(ed.cur.line), anchor.max(ed.cur.line))),
        _ => None,
    });
    let col = doc::col_of(&lines[ed.cur.line], ed.cur.byte);
    let region = region.map(|(a, b)| {
        let b = b.min(n - 1);
        ((a, doc::byte_of_col(&lines[a], col)), (b, doc::byte_of_col(&lines[b], col)))
    });
    Doc {
        lines,
        l: ed.cur.line,
        c: ed.cur.byte,
        org: ed.org.is_some(),
        st,
        path: ed.path.clone(),
        last_command: ed.org.as_ref().and_then(|o| o.last_command.clone()),
        interactive: true,
        region,
        ..Doc::default()
    }
}

/// Write `d` back: the changed lines as one edit, point, messages.
pub fn finish(ed: &mut Editor, d: Doc) {
    let old_n = ed.line_count();
    let new_n = d.lines.len();
    let mut a = 0;
    while a < old_n && a < new_n && ed.buf.line(a) == d.lines[a] {
        a += 1;
    }
    if !(a == old_n && a == new_n) {
        let mut b = 0;
        while b < old_n - a && b < new_n - a && ed.buf.line(old_n - 1 - b) == d.lines[new_n - 1 - b] {
            b += 1;
        }
        super::splice(ed, a, old_n - a - b, &d.lines[a..new_n - b]);
    }
    let l = d.l.min(ed.line_count() - 1);
    ed.set_cursor(l, d.c.min(ed.buf.line_len(l)));
    ed.cur.want_col = doc::col_of(&ed.buf.line(l), ed.cur.byte);
    if let Some((text, linewise)) = d.kill {
        crate::vim::ops::set_reg(ed, crate::editor::Register { text, linewise });
    }
    if let Some(m) = d.msg {
        ed.set_msg(m);
    }
    let mut st = d.st;
    st.version = ed.buf.version;
    put_state(ed, st);
    if matches!(ed.mode, Mode::VisualLine { .. }) && d.region.is_some() {
        ed.mode = Mode::Normal;
    }
}

/// Run `f` on the buffer as a Doc.
pub fn with_doc<T>(ed: &mut Editor, name: &str, f: impl FnOnce(&mut Doc) -> Result<T, String>) -> Result<T, String> {
    let mut d = doc(ed);
    d.this_command = name.to_owned();
    let r = f(&mut d);
    let follow = d.st.follow_field;
    finish(ed, d);
    if r.is_ok() && follow && name != "org-table-edit-field" && is_motion(name) {
        follow_field(ed);
    }
    r
}

fn is_motion(name: &str) -> bool {
    matches!(
        name,
        "org-table-next-field" | "org-table-previous-field" | "org-table-next-row" | "org-table-beginning-of-field" | "org-table-end-of-field" | "orgtbl-tab" | "orgtbl-ret"
    )
}

/// org-table-follow-fields-with-editor.
fn follow_field(ed: &mut Editor) {
    let d = doc(ed);
    if opt_bool("org-table-exit-follow-field-mode-when-leaving-table", true) && !d.at_table() {
        let mut st = d.st;
        st.follow_field = false;
        put_state(ed, st);
        return;
    }
    if d.check_inside_data_field(true, false).unwrap_or(false) {
        edit_field_prompt(ed);
    }
}

fn ok(ed: &mut Editor, name: &str, f: impl FnOnce(&mut Doc) -> Result<(), String>) -> Result<(), String> {
    with_doc(ed, name, f)
}

// ---- prompts ----

fn read(ed: &mut Editor, prompt: &str, initial: &str, then: impl FnOnce(&mut Editor, String) + Send + 'static) {
    super::read(ed, prompt, initial, then);
}

/// The separator a prefix argument selects (org-table-convert-region).
fn separator(arg: Prefix) -> Option<edit::Sep> {
    Some(match arg {
        Prefix::None => edit::Sep::Auto,
        Prefix::U(1) => edit::Sep::Comma,
        Prefix::U(2) => edit::Sep::Tab,
        Prefix::U(_) => return None,
        Prefix::Num(n) => edit::Sep::Spaces(n),
        Prefix::Minus => edit::Sep::Spaces(-1),
    })
}

/// Report an error from a prompt continuation.
fn report(ed: &mut Editor, r: Result<(), String>) {
    if let Err(e) = r {
        ed.set_err(e);
    }
}

fn convert_region_cmd(ed: &mut Editor, arg: Prefix, name: &str) -> Result<(), String> {
    let d = doc(ed);
    let Some(((a, _), (b, _))) = d.region else {
        return Err("The mark is not set now, so there is no region".into());
    };
    let name = name.to_owned();
    match separator(arg) {
        Some(sep) => ok(ed, &name, |d| edit::convert_region(d, a, b, sep)),
        None => {
            read(ed, "Regexp for field separator: ", "", move |ed, re| {
                let r = ok(ed, &name, |d| edit::convert_region(d, a, b, edit::Sep::Regex(re)));
                report(ed, r);
            });
            Ok(())
        }
    }
}

fn create_cmd(ed: &mut Editor, name: &str) -> Result<(), String> {
    let dflt = edit::default_size();
    let name = name.to_owned();
    read(ed, &format!("Table size Columns x Rows [e.g. {dflt}]: "), "", move |ed, ans| {
        let size = if ans.trim().is_empty() { dflt } else { ans };
        let r = ok(ed, &name, |d| edit::create(d, &size));
        report(ed, r);
    });
    Ok(())
}

/// org-table-edit-field's prompt.
fn edit_field_prompt(ed: &mut Editor) {
    let mut d = doc(ed);
    let col = d.current_column();
    let dline = d.current_dline();
    let coord = if opt_string("org-table-use-standard-references", "from") == "t" {
        format!("{}{}", formula::number_to_letters(col), dline)
    } else {
        format!("@{dline}${col}")
    };
    let field = d.get_field(None, None);
    let (line, c) = (ed.cur.line, ed.cur.byte);
    read(ed, &format!("Edit field {coord}: "), doc::trim(&field), move |ed, text| {
        ed.set_cursor(line, c);
        let r = ok(ed, "org-table-finish-edit-field", |d| {
            d.check_inside_data_field(false, false)?;
            let t = doc::trim(&text.replace('\n', " ")).to_owned();
            d.get_field(None, Some(&t));
            d.align();
            d.message("New field value inserted");
            Ok(())
        });
        report(ed, r);
    });
}

fn eval_formula_cmd(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if arg == Prefix::U(2) {
        return ok(ed, "org-table-eval-formula", |d| formula::eval_formula(d, EvalArg::Insert, None, false, false, false, false));
    }
    let mut d = doc(ed);
    d.check_inside_data_field(false, true)?;
    formula::analyze(&mut d);
    let named = arg == Prefix::U(1);
    let (prompt, initial) = formula::formula_prompt(&mut d, named)?;
    let ea = match arg {
        Prefix::U(1) => EvalArg::Field,
        Prefix::Num(n) => EvalArg::Down(n),
        _ => EvalArg::None,
    };
    read(ed, &prompt, &initial, move |ed, ans| {
        let eq = formula::from_user(&ans);
        let r = ok(ed, "org-table-eval-formula", |d| formula::eval_formula(d, ea, Some(&eq), false, false, false, false));
        report(ed, r);
    });
    Ok(())
}

fn toggle_column_width(d: &mut Doc, arg: Prefix, selection: Option<&str>) -> Result<(), String> {
    if !d.at_table() {
        return Err("Not in a table".into());
    }
    match arg {
        Prefix::U(1) => return shrink(d),
        Prefix::U(2) => {
            d.set_shrunk(vec![]);
            return Ok(());
        }
        _ => {}
    }
    let max = d.line().len() / 2;
    let columns = match selection {
        Some(s) => read_column_selection(s, max),
        None => vec![d.current_column()],
    };
    let shrunk = d.shrunk();
    // cl-set-exclusive-or.
    let mut new: Vec<usize> = columns.iter().filter(|c| !shrunk.contains(c)).copied().collect();
    new.extend(shrunk.iter().filter(|c| !columns.contains(c)));
    d.set_shrunk(new);
    Ok(())
}

/// Point is before the first column or after the last one.
fn outside_columns(d: &Doc) -> bool {
    let b = d.before();
    let start = b.rfind('|').map_or(0, |i| i + 1);
    start == 0 || d.lines[d.l][start..].chars().all(|c| c == ' ' || c == '\t')
}

/// org-table--read-column-selection.
pub fn read_column_selection(select: &str, max: usize) -> Vec<usize> {
    let mut out = vec![];
    for s in select.split_whitespace() {
        if s == "-" || s == "1-" {
            return (1..=max).collect();
        }
        if let Ok(n) = s.parse::<usize>() {
            if n > 0 && n <= max {
                out.push(n);
            }
            continue;
        }
        if let Some((a, b)) = s.split_once('-') {
            let a = if a.is_empty() { Some(1) } else { a.parse::<usize>().ok().map(|x| x.max(1)) };
            let b = if b.is_empty() { Some(max) } else { b.parse::<usize>().ok().map(|x| x.min(max)) };
            if let (Some(a), Some(b)) = (a, b) {
                out.extend(a..=b);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// org-table-shrink: shrink the columns with a width cookie.
fn shrink(d: &mut Doc) -> Result<(), String> {
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    let (b, e) = (d.table_begin(), d.table_end());
    let cookie = doc::re(r"^<[lrc]?[0-9]+>$");
    let mut cols = vec![];
    for l in b..e {
        if let doc::Row::Cells(c) = doc::row_of(&d.lines[l][d.lines[l].find('|').unwrap() + 1..]) {
            for (i, x) in c.iter().enumerate() {
                if cookie.is_match(x) {
                    cols.push(i + 1);
                }
            }
        }
    }
    d.set_shrunk(cols);
    Ok(())
}

fn sort_cmd(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let with_case = !arg.is_none();
    let d = doc(ed);
    let need_col = d.current_column() == 0;
    let entries = [
        ("a", "alphabetic"),
        ("A", "alphabetic, reversed"),
        ("n", "numeric"),
        ("N", "numeric, reversed"),
        ("t", "time"),
        ("T", "time, reversed"),
        ("f", "function"),
        ("F", "function, reversed"),
    ];
    let run = move |ed: &mut Editor, column: Option<usize>| {
        super::menu(
            ed,
            "Sort Table: [a]lphabetic, [n]umeric, [t]ime, [f]unc.  A/N/T/F means reversed",
            entries.iter().map(|(k, l)| ((*k).to_owned(), (*l).to_owned())).collect(),
            move |ed, k| {
                let kind = k.chars().next().unwrap_or('a');
                if kind.eq_ignore_ascii_case(&'f') {
                    read(ed, "Function for extracting keys: ", "", move |ed, getkey| {
                        read(ed, "Function for comparing keys (empty for default `sort-subr' predicate): ", "", move |ed, cmp| {
                            let r = sort_with_lisp(ed, with_case, kind, column, &getkey, &cmp);
                            report(ed, r);
                        });
                    });
                } else {
                    let r = ok(ed, "org-table-sort-lines", |d| edit::sort_lines(d, with_case, kind, column, None));
                    report(ed, r);
                }
            },
        );
    };
    if need_col {
        read(ed, "Use column N for sorting: ", "", move |ed, n| {
            let c = n.trim().parse::<usize>().ok();
            run(ed, c);
        });
    } else {
        run(ed, None);
    }
    Ok(())
}

/// Sort with Lisp key extractor and comparator functions.
fn sort_with_lisp(ed: &mut Editor, with_case: bool, kind: char, column: Option<usize>, getkey: &str, cmp: &str) -> Result<(), String> {
    let f = lisp::read(getkey)?;
    let c = if cmp.trim().is_empty() { None } else { Some(lisp::read(cmp)?) };
    let key = move |s: &str| -> edit::SortKey {
        let mut i = lisp::Interp::new();
        match i.apply(&f, vec![lisp::str_(s)]) {
            Ok(lisp::L::Int(n)) => edit::SortKey::Num(n as f64),
            Ok(lisp::L::Float(x)) => edit::SortKey::Num(x),
            Ok(lisp::L::Str(t)) => edit::SortKey::Str(t.to_string()),
            Ok(o) => edit::SortKey::Str(lisp::princ(&o)),
            Err(_) => edit::SortKey::Num(0.0),
        }
    };
    let _ = c;
    ok(ed, "org-table-sort-lines", |d| edit::sort_lines(d, with_case, kind, column, Some(&key)))
}

fn export_cmd(ed: &mut Editor) -> Result<(), String> {
    let mut d = doc(ed);
    if !d.at_table() {
        return Err("No table at point".into());
    }
    d.align();
    let file = entry_get(&d.lines, d.l, "TABLE_EXPORT_FILE", true);
    let format = entry_get(&d.lines, d.l, "TABLE_EXPORT_FORMAT", true);
    finish(ed, d);
    let step2 = move |ed: &mut Editor, file: String| {
        let path = options::expand(&file);
        if path.is_dir() {
            ed.set_err("This is a directory path, not a file");
            return;
        }
        if ed.path.as_ref().is_some_and(|p| std::fs::canonicalize(p).ok() == std::fs::canonicalize(&path).ok() && path.exists()) {
            ed.set_err("Please specify a file name that is different from current");
            return;
        }
        let write = move |ed: &mut Editor, fmt: String| {
            let d = doc(ed);
            let table = d.to_lisp();
            match orgtbl::export_text(&table, &fmt).and_then(|t| std::fs::write(&path, t).map_err(|e| e.to_string())) {
                Ok(()) => ed.set_msg("Export done."),
                Err(e) => ed.set_err(e),
            }
        };
        match format {
            Some(f) => write(ed, f),
            None => {
                let dflt = orgtbl::default_export_format(&file);
                super::complete(ed, &format!("Format (default {dflt}): "), orgtbl::EXPORT_FORMATS.iter().map(|s| (*s).to_owned()).collect(), false, move |ed, f| {
                    write(ed, if f.trim().is_empty() { dflt } else { f })
                });
            }
        }
    };
    match file {
        Some(f) => step2(ed, f),
        None => read(ed, "Export table to: ", "", move |ed, f| {
            if options::expand(&f).exists() {
                super::yes_or_no(ed, &format!("Overwrite file {f}? "), move |ed| step2(ed, f));
            } else {
                step2(ed, f);
            }
        }),
    }
    Ok(())
}

fn import_cmd(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let Some(sep) = separator(arg) else {
        return Err("Regexp separators: use org-table-convert-region".into());
    };
    read(ed, "Import file: ", "", move |ed, file| {
        let ext_ok = [".txt", ".tsv", ".csv"].iter().any(|e| file.ends_with(e));
        let go = move |ed: &mut Editor| {
            let r = std::fs::read_to_string(options::expand(&file)).map_err(|e| format!("{file}: {e}")).and_then(|text| ok(ed, "org-table-import", |d| edit::import(d, &text, sep)));
            report(ed, r);
        };
        if ext_ok {
            go(ed);
        } else {
            super::yes_or_no(ed, "The file's extension is not .txt, .tsv or .csv.  Import? ", go);
        }
    });
    Ok(())
}

// ---- formula editor ----

fn edit_formulas(ed: &mut Editor) -> Result<(), String> {
    let mut d = doc(ed);
    let at_tblfm = d.at_tblfm();
    if !at_tblfm && !d.at_table() {
        return Err("Not at a table".into());
    }
    if d.st.fedit.is_none() {
        let (l0, c0) = (d.l, d.c);
        if at_tblfm {
            let t = (0..d.l).rev().find(|&l| doc::is_table_line(&d.lines[l])).ok_or("Not at a table")?;
            d.goto(t, 0);
        }
        formula::analyze(&mut d);
        let mut eql = formula::get_stored_formulas(&mut d, true, at_tblfm.then_some(l0))?;
        let an = d.an.clone();
        formula::sort_formulas(&mut eql, &an);
        d.goto(l0, c0);
        let an_refs = opt_string("org-table-use-standard-references", "from") == "t";
        let mut lines = formula::fedit_lines(&eql);
        if an_refs {
            lines = lines.iter().map(|l| formula::convert_refs_to_an(l)).collect();
        }
        d.st.fedit = Some(Fedit { source: d.l, lines, an: an_refs });
    }
    finish(ed, d);
    fedit_menu(ed);
    Ok(())
}

/// The formula editor: pick a formula to edit, add one, or finish.
fn fedit_menu(ed: &mut Editor) {
    let st = state(ed);
    let Some(f) = st.fedit else { return };
    let mut items: Vec<String> = f.lines.iter().filter(|l| !l.is_empty() && !l.starts_with('#')).cloned().collect();
    items.push("[Add formula]".into());
    items.push("[Finish and install]".into());
    items.push("[Finish, install, and apply]".into());
    items.push("[Toggle reference type (A1 / @row$column)]".into());
    items.push("[Abort]".into());
    super::complete(ed, "Edit formula: ", items, true, |ed, choice| {
        let r = match choice.as_str() {
            "[Add formula]" => {
                read(ed, "New formula (LHS = RHS): ", "", |ed, text| {
                    if !text.trim().is_empty() {
                        fedit_update(ed, None, Some(text));
                    }
                    fedit_menu(ed);
                });
                Ok(())
            }
            "[Finish and install]" => fedit_finish(ed, false),
            "[Finish, install, and apply]" => fedit_finish(ed, true),
            "[Abort]" => fedit_abort(ed),
            "[Toggle reference type (A1 / @row$column)]" => {
                fedit_toggle_ref_type(ed);
                fedit_menu(ed);
                Ok(())
            }
            line => {
                let line = line.to_owned();
                let initial = line.clone();
                read(ed, "Formula (empty removes): ", &initial, move |ed, text| {
                    fedit_update(ed, Some(line), (!text.trim().is_empty()).then_some(text));
                    fedit_menu(ed);
                });
                Ok(())
            }
        };
        report(ed, r);
    });
}

fn fedit_update(ed: &mut Editor, old: Option<String>, new: Option<String>) {
    let mut st = state(ed);
    if let Some(f) = &mut st.fedit {
        match (old.and_then(|o| f.lines.iter().position(|l| *l == o)), new) {
            (Some(i), Some(n)) => f.lines[i] = n,
            (Some(i), None) => {
                f.lines.remove(i);
            }
            (None, Some(n)) => f.lines.push(n),
            (None, None) => {}
        }
    }
    put_state(ed, st);
}

fn fedit_toggle_ref_type(ed: &mut Editor) {
    let mut st = state(ed);
    if let Some(f) = &mut st.fedit {
        f.an = !f.an;
        f.lines = f.lines.iter().map(|l| if f.an { formula::convert_refs_to_an(l) } else { formula::convert_refs_to_rc(l) }).collect();
        ed.set_msg(format!("Reference type switched to {}", if f.an { "A1 etc" } else { "@row$column" }));
    }
    put_state(ed, st);
}

/// org-table-fedit-finish.
fn fedit_finish(ed: &mut Editor, apply: bool) -> Result<(), String> {
    let mut st = state(ed);
    let Some(f) = st.fedit.take() else {
        return Err("No formula editing in progress".into());
    };
    put_state(ed, st);
    let text = f.lines.iter().map(|l| if opt_string("org-table-use-standard-references", "from") != "nil" { formula::convert_refs_to_rc(l) } else { l.clone() }).collect::<Vec<_>>().join("\n");
    let eql = formula::fedit_parse(&text)?;
    ok(ed, "org-table-fedit-finish", |d| {
        d.goto(f.source.min(d.n() - 1), 0);
        if d.at_tblfm() {
            let t = (0..d.l).rev().find(|&l| doc::is_table_line(&d.lines[l])).ok_or("Not at a table")?;
            d.goto(t, 0);
        }
        formula::analyze(d);
        formula::store_formulas(d, &eql, None);
        if apply {
            formula::recalculate(d, Recalc::All, false)
        } else {
            d.message("New formulas installed - press C-u C-c C-c to apply.");
            Ok(())
        }
    })
}

fn fedit_abort(ed: &mut Editor) -> Result<(), String> {
    let mut st = state(ed);
    st.fedit = None;
    put_state(ed, st);
    ed.set_msg("Formula editing aborted without installing changes");
    Ok(())
}

/// Shift the reference in the last formula line being edited.
fn fedit_shift(ed: &mut Editor, dir: char) -> Result<(), String> {
    let mut st = state(ed);
    let Some(f) = &mut st.fedit else {
        return Err("No formula editing in progress".into());
    };
    let Some(i) = f.lines.iter().rposition(|l| !l.is_empty() && !l.starts_with('#')) else {
        return Err("No reference at point".into());
    };
    let line = f.lines[i].clone();
    let pos = line.find('=').map_or(0, |e| line[e..].find(['@', '$']).map_or(e, |k| e + k + 1));
    f.lines[i] = formula::shift_reference(&line, pos, dir)?;
    put_state(ed, st);
    Ok(())
}

// ---- orgtbl-mode ----

/// orgtbl-mode is on in this buffer.
pub fn orgtbl_active(ed: &Editor) -> bool {
    ed.org.is_none() && state(ed).orgtbl
}

/// The orgtbl-mode keymap (orgtbl-make-binding numbering).
pub const ORGTBL_MAP: &[(&str, &str)] = &[
    ("M-S-<left>", "org-table-delete-column"),
    ("M-<left>", "org-table-move-column-left"),
    ("M-<right>", "org-table-move-column-right"),
    ("M-S-<right>", "org-table-insert-column"),
    ("M-S-<up>", "org-table-kill-row"),
    ("M-S-<down>", "org-table-insert-row"),
    ("M-<up>", "org-table-move-row-up"),
    ("M-<down>", "org-table-move-row-down"),
    ("C-c C-w", "org-table-cut-region"),
    ("C-c M-w", "org-table-copy-region"),
    ("C-c C-y", "org-table-paste-rectangle"),
    ("C-c -", "org-table-insert-hline"),
    ("C-c }", "org-table-toggle-coordinate-overlays"),
    ("C-c {", "org-table-toggle-formula-debugger"),
    ("S-RET", "org-table-copy-down"),
    ("C-c ?", "org-table-field-info"),
    ("C-c SPC", "org-table-blank-field"),
    ("C-c +", "org-table-sum"),
    ("C-c =", "org-table-eval-formula"),
    ("C-c '", "org-table-edit-formulas"),
    ("C-c `", "org-table-edit-field"),
    ("C-c *", "org-table-recalculate"),
    ("C-c ^", "org-table-sort-lines"),
    ("M-a", "org-table-beginning-of-field"),
    ("M-e", "org-table-end-of-field"),
    ("C-#", "org-table-rotate-recalc-marks"),
    ("RET", "orgtbl-ret"),
    ("TAB", "orgtbl-tab"),
    ("S-TAB", "org-table-previous-field"),
    ("<backtab>", "org-table-previous-field"),
    ("M-RET", "org-table-wrap-region"),
    ("C-c C-c", "orgtbl-ctrl-c-ctrl-c"),
    ("C-c |", "orgtbl-create-or-convert-from-region"),
    ("C-c \" a", "orgtbl-ascii-plot"),
    ("C-c \" g", "org-plot/gnuplot"),
];

/// The command behind orgtbl-hijacker-command-N.
fn hijacker(n: u32) -> Option<&'static str> {
    Some(match n {
        1 => "org-table-delete-column",
        2 => "org-table-move-column-left",
        3 => "org-table-move-column-right",
        4 => "org-table-insert-column",
        5 => "org-table-kill-row",
        6 => "org-table-insert-row",
        7 => "org-table-move-row-up",
        8 => "org-table-move-row-down",
        9 => "org-table-cut-region",
        10 => "org-table-copy-region",
        11 => "org-table-paste-rectangle",
        12 | 105 | 106 => "org-table-wrap-region",
        13 => "org-table-insert-hline",
        14 => "org-table-toggle-coordinate-overlays",
        15 => "org-table-toggle-formula-debugger",
        16 => "org-table-next-row",
        17 => "org-table-copy-down",
        18 => "org-table-field-info",
        19 => "org-table-blank-field",
        20 => "org-table-sum",
        21 => "org-table-eval-formula",
        22 => "org-table-edit-formulas",
        23 => "org-table-edit-field",
        24 => "org-table-recalculate",
        25 => "org-table-sort-lines",
        26 => "org-table-beginning-of-field",
        27 => "org-table-end-of-field",
        28 => "org-table-rotate-recalc-marks",
        100 | 101 => "orgtbl-ret",
        102 | 103 => "orgtbl-tab",
        104 | 107 | 108 => "org-table-previous-field",
        109 => "org-delete-backward-char",
        _ => return None,
    })
}

/// Keys in a buffer with orgtbl-mode: in a table, the table editor's;
/// elsewhere the buffer's own (the hijacker commands fall through).
pub fn orgtbl_key(ed: &mut Editor, k: Key) -> bool {
    if !orgtbl_active(ed) {
        return false;
    }
    let normal = ed.mode == Mode::Normal;
    let insert = ed.mode == Mode::Insert;
    if !normal && !insert {
        return false;
    }
    // orgtbl-mode runs outside Org: a table line is any `|` line.
    let in_table = doc::is_table_line(&ed.buf.line(ed.cur.line));
    let mut seq = ed.org_keys.clone();
    seq.push(k);
    let mut prefix = false;
    for (desc, cmd) in ORGTBL_MAP {
        let Some(keys) = super::parse_keys(desc) else { continue };
        if keys == seq {
            ed.org_keys.clear();
            if !in_table && !matches!(*cmd, "orgtbl-ctrl-c-ctrl-c" | "orgtbl-create-or-convert-from-region") {
                return false;
            }
            // Plain keys stay Vim's in Normal state.
            if normal && seq.len() == 1 && matches!(*cmd, "orgtbl-ret") {
                return false;
            }
            let arg = std::mem::take(&mut ed.org_arg);
            super::run(ed, cmd, arg);
            return true;
        }
        if keys.len() > seq.len() && keys[..seq.len()] == seq[..] {
            prefix = true;
        }
    }
    if prefix && (in_table || seq[0] == Key::ctrl('c')) && ed.org_keys.len() + 1 < 4 {
        if seq.len() == 1 && !seq[0].ctrl && !seq[0].alt {
            return false;
        }
        ed.org_keys = seq;
        ed.set_msg(format!("{}-", super::describe(&ed.org_keys)));
        return true;
    }
    ed.org_keys.clear();
    false
}

/// org-table auto-blank and overwrite on typing (org-self-insert-command
/// and orgtbl-self-insert-command in tables). Insert mode calls this for
/// a typed character; true when it handled the insertion.
///
/// Right after a field motion (TAB, S-TAB, RET), typing blanks the field
/// first (org-table-auto-blank-field). Typing in front of the padding of
/// an aligned field overwrites a blank instead of widening the field, so
/// the table stays aligned.
pub fn self_insert(ed: &mut Editor, ch: char) -> bool {
    if ch == '|' || ch == '\n' {
        return false;
    }
    let d = doc(ed);
    if !d.at_table() || d.at_hline() {
        return false;
    }
    let last = ed.org.as_ref().and_then(|o| o.last_command.clone());
    let motion = matches!(
        last.as_deref(),
        Some("org-cycle" | "org-return" | "org-shifttab" | "org-ctrl-c-ctrl-c" | "org-table-next-field" | "org-table-previous-field" | "org-table-next-row" | "orgtbl-tab" | "orgtbl-ret")
    );
    let blank = opt_bool("org-table-auto-blank-field", true) && motion && d.check_inside_data_field(true, false).unwrap_or(false);
    let r = with_doc(ed, "org-self-insert-command", |d| {
        if blank {
            edit::blank_field(d)?;
        }
        // Overwrite a blank of the field's padding: `[^|\n]* \( \)|`.
        let rest = d.rest().to_owned();
        let bar = rest.find('|');
        let overwrite = bar.is_some_and(|i| i >= 2 && rest[..i].ends_with("  "));
        if overwrite {
            let i = bar.unwrap();
            let c = d.c;
            d.replace_in_line(c + i - 1..c + i, "");
            let saved = d.st.may_need_update;
            d.insert(&ch.to_string());
            d.st.may_need_update = saved;
        } else {
            d.insert(&ch.to_string());
            d.st.may_need_update = true;
        }
        // Typing in a shrunk column expands it.
        let col = d.current_column();
        let shrunk = d.shrunk();
        if shrunk.contains(&col) {
            d.set_shrunk(shrunk.into_iter().filter(|&c| c != col).collect());
        }
        Ok(())
    });
    if let Some(o) = &mut ed.org {
        o.last_command = Some("org-self-insert-command".into());
    }
    r.is_ok()
}

// ---- display ----

/// The text Fred shows for line `l` of a buffer with tables (render
/// hook): shrunk columns truncated, coordinates, the header line.
/// `top` is the first screen line. None: the line as it is.
pub fn display(ed: &Editor, l: usize, top: bool, line: &str) -> Option<String> {
    let st = STATES.with(|s| s.borrow().get(&key_of(ed)).cloned())?;
    if st.shrunk.is_empty() && !st.overlay_coordinates && !st.header_line {
        return None;
    }
    if !doc::is_table_line(line) {
        return None;
    }
    let lines_at = |i: usize| ed.buf.line(i);
    let mut b = l;
    while b > 0 && doc::is_table_line(&lines_at(b - 1)) {
        b -= 1;
    }
    let mut e = l + 1;
    while e < ed.line_count() && doc::is_table_line(&lines_at(e)) {
        e += 1;
    }
    let table: Vec<String> = (b..e).map(lines_at).collect();
    let mut text = line.to_owned();
    // Header line: the first data row in place of the top line.
    if top && st.header_line && l > b {
        let first = table.iter().position(|t| doc::is_dataline(t) && !doc::re(r"\|\s+<[rcl]?([0-9]+)?>").is_match(t));
        if let Some(f) = first
            && b + f < l
        {
            text = table[f].clone();
        }
    }
    if l != ed.cur.line {
        let cols = st.shrunk_in(b, e);
        if !cols.is_empty() {
            text = shrunk_line(&text, &table, &cols);
        }
    }
    if st.overlay_coordinates {
        let idx = l - b;
        if doc::is_hline(&text) {
            let ih = table[..=idx].iter().filter(|t| doc::is_hline(t)).count();
            text = coordinate_rule(&text);
            text.push_str(&format!("  I*{ih}"));
        } else {
            let id = table[..=idx].iter().filter(|t| !doc::is_hline(t)).count();
            text.push_str(&format!("  @{id}"));
        }
    }
    (text != line).then_some(text)
}

/// A rule with `$N` labels over each column (org-table-overlay-coordinates).
fn coordinate_rule(line: &str) -> String {
    let use_an = opt_string("org-table-use-standard-references", "from") == "t";
    let mut out = String::new();
    let mut ic = 0;
    let mut chars = line.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        out.push(c);
        if (c == '|' || c == '+') && chars.peek().is_some_and(|(_, n)| *n == '-') {
            ic += 1;
            let label = if use_an { formula::number_to_letters(ic) } else { format!("${ic}") };
            let mut run = 0;
            let mut tmp = vec![];
            while let Some(&(_, n)) = chars.peek() {
                if n != '-' {
                    break;
                }
                tmp.push(n);
                chars.next();
                run += 1;
            }
            if label.len() <= run {
                out.push_str(&label);
                out.push_str(&"-".repeat(run - label.len()));
            } else {
                out.extend(tmp);
            }
        }
    }
    out
}

/// `line` with the fields of `cols` cut to their cookie width and an
/// indicator (org-table--shrink-field).
pub fn shrunk_line(line: &str, table: &[String], cols: &[usize]) -> String {
    let ind = opt_string("org-table-shrunk-column-indicator", "…");
    let cookie = doc::re(r"^<([lrc])?([0-9]+)>$");
    let hline = doc::is_hline(line);
    let Some(first) = line.find('|') else {
        return line.to_owned();
    };
    let mut out = line[..=first].to_owned();
    let rest = &line[first + 1..];
    let seps: Vec<usize> = rest.char_indices().filter(|(_, c)| *c == '|' || (hline && *c == '+')).map(|(i, _)| i).collect();
    let mut start = 0;
    for (k, &end) in seps.iter().chain(std::iter::once(&rest.len())).enumerate() {
        let field = &rest[start..end];
        let col = k + 1;
        if cols.contains(&col) && end <= rest.len() {
            // The column's cookie: width and alignment.
            let (mut w, mut align) = (None, 'l');
            for t in table {
                if let doc::Row::Cells(c) = doc::row_of(&t[t.find('|').map_or(0, |i| i + 1)..])
                    && let Some(m) = c.get(col - 1).and_then(|x| cookie.captures(x))
                {
                    w = m[2].parse().ok();
                    align = m.get(1).and_then(|a| a.as_str().chars().next()).unwrap_or('l');
                    break;
                }
            }
            let width = w.unwrap_or(0);
            let new = if hline {
                if width == 0 { ind.clone() } else { format!("{}{ind}", "-".repeat(width + 1)) }
            } else if width == 0 {
                ind.clone()
            } else {
                let contents = doc::trim(field);
                let cw = doc::width(contents);
                if cw >= width {
                    let cut: String = {
                        let mut s = String::new();
                        for c in contents.chars() {
                            if doc::width(&s) + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) > width {
                                break;
                            }
                            s.push(c);
                        }
                        s
                    };
                    format!(" {cut}{ind}")
                } else {
                    let pad = width - cw;
                    let pre = match align {
                        'r' => pad,
                        'c' => pad / 2,
                        _ => 0,
                    };
                    format!(" {}{contents}{}{ind}", " ".repeat(pre), " ".repeat(pad - pre))
                }
            };
            out.push_str(&new);
        } else {
            out.push_str(field);
        }
        if end < rest.len() {
            out.push_str(&rest[end..end + 1]);
        }
        start = end + 1;
        if start > rest.len() {
            break;
        }
    }
    out
}

// ---- commands ----

/// org-table-ctrl-c-ctrl-c: the table part of org-ctrl-c-ctrl-c.
fn ctrl_c_ctrl_c(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let d = doc(ed);
    let line = d.line().to_owned();
    if doc::re(r"(?i)^[ \t]*#\+plot:").is_match(&line) {
        return ok(ed, "org-plot/gnuplot", |d| plot::gnuplot(d, plot::Plot::default()));
    }
    if d.at_table_el() && !d.at_table() {
        ed.set_msg("Use `C-c '' to edit table.el tables");
        return Ok(());
    }
    if d.at_tblfm() {
        let all = if arg == Prefix::U(2) { Recalc::Iterate } else { Recalc::All };
        return ok(ed, "org-table-calc-current-TBLFM", |d| formula::calc_current_tblfm(d, all));
    }
    // An affiliated keyword of the table: recalculate it all and send it.
    if !d.at_table() && line.trim_start().starts_with("#+") {
        let mut t = d.l + 1;
        while t < d.n() && d.lines[t].trim_start().starts_with("#+") {
            t += 1;
        }
        if t < d.n() && d.table_line_at(t) {
            return ok(ed, "org-table-recalculate", |d| {
                let (l0, c0) = (d.l, d.c);
                d.goto(t, 0);
                formula::recalculate(d, if arg == Prefix::U(2) { Recalc::Iterate } else { Recalc::All }, false)?;
                orgtbl::send_table(d, true)?;
                d.goto(l0, c0);
                Ok(())
            });
        }
    }
    if !d.at_table() {
        return Err("Not at a table".into());
    }
    ok(ed, "org-table-ctrl-c-ctrl-c", |d| {
        formula::maybe_eval_formula(d)?;
        match arg {
            Prefix::U(1) => formula::recalculate(d, Recalc::All, false),
            Prefix::U(_) => formula::recalculate(d, Recalc::Iterate, false),
            Prefix::None => {
                if !formula::maybe_recalculate_line(d)? {
                    d.align();
                }
                Ok(())
            }
            _ => formula::recalculate(d, Recalc::Line, false),
        }
    })
}

fn need_table(ed: &Editor) -> Result<(), String> {
    if doc(ed).at_table() { Ok(()) } else { Err("Not at a table".into()) }
}

/// This module's interactive commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    if !(name.starts_with("org-table") || name.starts_with("orgtbl") || name.starts_with("org-plot/") || name == "turn-on-orgtbl") {
        return None;
    }
    let n = arg.value();
    Some(match name {
        "org-table-align" => ok(ed, name, |d| {
            if !d.at_table() {
                return Err("Not at a table".into());
            }
            d.align();
            Ok(())
        }),
        "org-table-ctrl-c-ctrl-c" => ctrl_c_ctrl_c(ed, arg),
        "org-table-next-field" => ok(ed, name, edit::next_field),
        "org-table-previous-field" => ok(ed, name, edit::previous_field),
        "org-table-next-row" => ok(ed, name, edit::next_row),
        "org-table-beginning-of-field" => ok(ed, name, |d| edit::beginning_of_field(d, n)),
        "org-table-end-of-field" => ok(ed, name, |d| edit::end_of_field(d, n)),
        "org-table-current-column" => ok(ed, name, |d| {
            let c = d.current_column();
            d.message(format!("Column {c}"));
            Ok(())
        }),
        "org-table-goto-column" => ok(ed, name, |d| {
            d.goto_column(n.max(0) as usize, false, false);
            Ok(())
        }),
        "org-table-blank-field" => ok(ed, name, |d| edit::blank_field(d).map(|_| ())),
        "org-table-field-info" => ok(ed, name, formula::field_info),
        "org-table-insert-column" => ok(ed, name, edit::insert_column),
        "org-table-delete-column" => ok(ed, name, edit::delete_column),
        "org-table-move-column-right" => ok(ed, name, |d| edit::move_column(d, false)),
        "org-table-move-column-left" => ok(ed, name, |d| edit::move_column(d, true)),
        "org-table-move-column" => ok(ed, name, |d| edit::move_column(d, !arg.is_none())),
        "org-table-move-cell-up" => ok(ed, name, edit::move_cell_up),
        "org-table-move-cell-down" => ok(ed, name, edit::move_cell_down),
        "org-table-move-cell-left" => ok(ed, name, edit::move_cell_left),
        "org-table-move-cell-right" => ok(ed, name, edit::move_cell_right),
        "org-table-move-row-down" => ok(ed, name, |d| edit::move_row(d, false)),
        "org-table-move-row-up" => ok(ed, name, |d| edit::move_row(d, true)),
        "org-table-move-row" => ok(ed, name, |d| edit::move_row(d, !arg.is_none())),
        "org-table-insert-row" => ok(ed, name, |d| edit::insert_row(d, !arg.is_none())),
        "org-table-insert-hline" => ok(ed, name, |d| edit::insert_hline(d, !arg.is_none())),
        "org-table-hline-and-move" => ok(ed, name, |d| edit::hline_and_move(d, !arg.is_none())),
        "org-table-kill-row" => ok(ed, name, edit::kill_row),
        "org-table-copy-down" => ok(ed, name, |d| edit::copy_down(d, if let Prefix::U(_) = arg { 4 } else { n })),
        "org-table-copy-region" | "org-table-cut-region" => ok(ed, name, |d| {
            let (a, b) = d.region.unwrap_or(((d.l, d.c), (d.l, d.c)));
            edit::copy_region(d, a, b, name == "org-table-cut-region" || !arg.is_none())
        }),
        "org-table-paste-rectangle" => ok(ed, name, edit::paste_rectangle),
        "org-table-sort-lines" => sort_cmd(ed, arg),
        "org-table-transpose-table-at-point" => ok(ed, name, |d| {
            if !d.at_table() {
                return Err("Not at a table".into());
            }
            edit::transpose(d)
        }),
        "org-table-wrap-region" => ok(ed, name, |d| edit::wrap_region(d, (!arg.is_none()).then_some(n))),
        "org-table-sum" => ok(ed, name, |d| edit::sum(d, None).map(|_| ())),
        "org-table-rotate-recalc-marks" => {
            let d = doc(ed);
            if d.region.is_some() && d.at_table() {
                let entries = [("#", "#"), ("*", "*"), ("!", "!"), ("$", "$"), ("SPC", "SPC")];
                super::menu(ed, "Change region to what mark?  Type # * ! $ or SPC", entries.iter().map(|(k, l)| ((*k).to_owned(), (*l).to_owned())).collect(), move |ed, k| {
                    let c = if k == "SPC" { ' ' } else { k.chars().next().unwrap_or(' ') };
                    let r = ok(ed, "org-table-rotate-recalc-marks", |d| edit::rotate_recalc_marks(d, Some(c)));
                    report(ed, r);
                });
                Ok(())
            } else {
                ok(ed, name, |d| edit::rotate_recalc_marks(d, None))
            }
        }
        "org-table-create" => {
            if let Prefix::Num(_) = arg {
                ok(ed, name, |d| edit::create(d, &edit::default_size()))
            } else {
                create_cmd(ed, name)
            }
        }
        "org-table-create-or-convert-from-region" => {
            if doc(ed).region.is_some() {
                convert_region_cmd(ed, arg, name)
            } else {
                create_cmd(ed, "org-table-create")
            }
        }
        "org-table-convert-region" => convert_region_cmd(ed, arg, name),
        "org-table-import" => import_cmd(ed, arg),
        "org-table-export" => export_cmd(ed),
        "org-table-convert" => ok(ed, name, edit::convert),
        "org-table-create-with-table.el" => {
            let d = doc(ed);
            if d.at_table_el() && !d.at_table() {
                super::yes_or_no(ed, "Convert table to Org table? ", |ed| {
                    let r = ok(ed, "org-table-convert", edit::convert);
                    report(ed, r);
                });
            } else if d.at_table() {
                super::yes_or_no(ed, "Convert table to table.el table? ", |ed| {
                    let r = ok(ed, "org-table-convert", |d| {
                        d.align();
                        edit::convert(d)
                    });
                    report(ed, r);
                });
            } else {
                read(ed, "Table size Columns x Rows: ", "3x3", |ed, size| {
                    let r = ok(ed, "org-table-create-with-table.el", |d| {
                        edit::create(d, &size)?;
                        let (b, e) = (d.table_begin(), d.table_end());
                        let rows: Vec<String> = d.lines[b..e].iter().filter(|l| !doc::is_hline(l)).cloned().collect();
                        d.lines.splice(b..e, rows);
                        d.goto(b, 0);
                        d.align();
                        edit::convert(d)
                    });
                    report(ed, r);
                });
            }
            Ok(())
        }
        "org-table-edit-field" => match arg {
            Prefix::U(2) => {
                let mut st = state(ed);
                st.follow_field = !st.follow_field;
                ed.set_msg(format!("Org-Table-Follow-Field mode {}", if st.follow_field { "enabled" } else { "disabled" }));
                put_state(ed, st);
                Ok(())
            }
            Prefix::None => need_table(ed).map(|()| edit_field_prompt(ed)),
            _ => ok(ed, name, |d| {
                if !d.at_table() {
                    return Err("Not at a table".into());
                }
                let col = d.current_column();
                let s = d.shrunk();
                d.set_shrunk(s.into_iter().filter(|&c| c != col).collect());
                Ok(())
            }),
        },
        "org-table-follow-field-mode" => {
            let mut st = state(ed);
            st.follow_field = match arg {
                Prefix::None => !st.follow_field,
                _ => n > 0,
            };
            ed.set_msg(format!("Org-Table-Follow-Field mode {}", if st.follow_field { "enabled" } else { "disabled" }));
            put_state(ed, st);
            Ok(())
        }
        "org-table-eval-formula" => eval_formula_cmd(ed, arg),
        "org-table-maybe-eval-formula" => ok(ed, name, formula::maybe_eval_formula),
        "org-table-maybe-recalculate-line" => ok(ed, name, |d| formula::maybe_recalculate_line(d).map(|_| ())),
        "org-table-recalculate" => ok(ed, name, |d| {
            let all = match arg {
                Prefix::None => Recalc::Line,
                Prefix::U(1) => Recalc::All,
                Prefix::U(_) => Recalc::Iterate,
                _ => Recalc::All,
            };
            formula::recalculate(d, all, false)
        }),
        "org-table-iterate" => ok(ed, name, |d| formula::iterate(d, (!arg.is_none()).then_some(n))),
        "org-table-recalculate-buffer-tables" => ok(ed, name, formula::recalculate_buffer_tables),
        "org-table-iterate-buffer-tables" => ok(ed, name, formula::iterate_buffer_tables),
        "org-table-calc-current-TBLFM" => ok(ed, name, |d| formula::calc_current_tblfm(d, if arg == Prefix::U(2) { Recalc::Iterate } else { Recalc::All })),
        "org-table-edit-formulas" => edit_formulas(ed),
        "org-table-fedit-finish" => fedit_finish(ed, !arg.is_none()),
        "org-table-fedit-abort" => fedit_abort(ed),
        "org-table-fedit-toggle-ref-type" => {
            fedit_toggle_ref_type(ed);
            Ok(())
        }
        "org-table-fedit-ref-up" => fedit_shift(ed, 'u'),
        "org-table-fedit-ref-down" => fedit_shift(ed, 'd'),
        "org-table-fedit-ref-left" => fedit_shift(ed, 'l'),
        "org-table-fedit-ref-right" => fedit_shift(ed, 'r'),
        "org-table-fedit-toggle-coordinates" => command(ed, "org-table-toggle-coordinate-overlays", Prefix::None).unwrap(),
        "org-table-fedit-line-up" | "org-table-fedit-line-down" => ok(ed, name, |d| {
            let up = name.ends_with("up");
            loop {
                let next = if up { d.l.checked_sub(1) } else { (d.l + 1 < d.n()).then_some(d.l + 1) };
                let Some(nl) = next else { break };
                if !d.table_line_at(nl) {
                    break;
                }
                d.goto(nl, d.c);
                if !d.at_hline() {
                    break;
                }
            }
            Ok(())
        }),
        "org-table-fedit-scroll" | "org-table-fedit-scroll-down" => {
            ed.set_msg("The table stays in view: Fred edits formulas in prompts");
            Ok(())
        }
        "org-table-fedit-lisp-indent" => {
            let mut st = state(ed);
            if let Some(f) = &mut st.fedit {
                // A prompt holds one line: collapse multi-line Lisp.
                f.lines = f.lines.iter().map(|l| doc::re(r"[ \t]*\n[ \t]*").replace_all(l, " ").into_owned()).collect();
            }
            put_state(ed, st);
            Ok(())
        }
        "org-table-fedit-menu" => {
            fedit_menu(ed);
            Ok(())
        }
        "org-table-show-reference" => ok(ed, name, |d| {
            let line = d.line().to_owned();
            if doc::is_tblfm(&line) {
                let t = (0..d.l).rev().find(|&l| doc::is_table_line(&d.lines[l])).ok_or("Not at a table")?;
                let pos = d.c;
                d.goto(t, 0);
                formula::show_reference(d, &line, pos)
            } else {
                let f = d.get_field(None, None);
                formula::show_reference(d, &f, 0)
            }
        }),
        "org-table-toggle-coordinate-overlays" | "org-table-overlay-coordinates" => {
            if !doc(ed).at_table() && name.ends_with("overlays") {
                return Some(Err("Not on a table".into()));
            }
            let mut st = state(ed);
            st.overlay_coordinates = if name.ends_with("overlays") { !st.overlay_coordinates } else { true };
            let on = st.overlay_coordinates;
            put_state(ed, st);
            ed.set_msg(format!("Tables Row/Column numbers display turned {}", if on { "on" } else { "off" }));
            Ok(())
        }
        "org-table-toggle-formula-debugger" => {
            let mut st = state(ed);
            st.formula_debug = !st.formula_debug;
            let on = st.formula_debug;
            put_state(ed, st);
            ed.set_msg(format!("Formula debugging has been turned {}", if on { "on" } else { "off" }));
            Ok(())
        }
        "org-table-header-line-mode" => {
            if ed.org.is_none() {
                return Some(Err("Cannot turn org table header mode outside org-mode buffers".into()));
            }
            let mut st = state(ed);
            st.header_line = match arg {
                Prefix::None => !st.header_line,
                _ => n > 0,
            };
            ed.set_msg(format!("Org-Table-Header-Line mode {}", if st.header_line { "enabled" } else { "disabled" }));
            put_state(ed, st);
            Ok(())
        }
        "org-table-toggle-column-width" => {
            let d = doc(ed);
            if !d.at_table() {
                return Some(Err("Not in a table".into()));
            }
            if arg.is_none() && outside_columns(&d) {
                read(ed, "Column ranges (e.g. 2-4 6-): ", "", |ed, sel| {
                    let r = ok(ed, "org-table-toggle-column-width", |d| toggle_column_width(d, Prefix::None, Some(&sel)));
                    report(ed, r);
                });
                Ok(())
            } else {
                ok(ed, name, |d| toggle_column_width(d, arg, None))
            }
        }
        "org-table-shrink" => ok(ed, name, shrink),
        "org-table-expand" => ok(ed, name, |d| {
            if !d.at_table() {
                return Err("Not at a table".into());
            }
            d.set_shrunk(vec![]);
            Ok(())
        }),
        "org-table-justify-field-maybe" => ok(ed, name, |d| {
            edit::justify_field_maybe(d, None);
            Ok(())
        }),
        "orgtbl-mode" | "turn-on-orgtbl" => {
            if ed.org.is_some() {
                ed.set_msg("Orgtbl mode is not useful in Org mode, command ignored");
                return Some(Ok(()));
            }
            let mut st = state(ed);
            st.orgtbl = if name == "turn-on-orgtbl" {
                true
            } else {
                match arg {
                    Prefix::None => !st.orgtbl,
                    _ => n > 0,
                }
            };
            ed.set_msg(format!("Orgtbl mode {}", if st.orgtbl { "enabled" } else { "disabled" }));
            st.may_need_update = true;
            put_state(ed, st);
            Ok(())
        }
        "orgtbl-ctrl-c-ctrl-c" => {
            let d = doc(ed);
            let line = d.line().to_owned();
            let send_next = doc::re(r"(?i)^[ \t]*#\+ORGTBL:").is_match(&line) && d.l + 1 < d.n() && doc::is_table_line(&d.lines[d.l + 1]);
            if send_next || doc::is_table_line(&line) {
                ok(ed, name, |d| {
                    if send_next {
                        d.goto(d.l + 1, d.lines[d.l + 1].find('|').unwrap() + 1);
                    }
                    formula::maybe_eval_formula(d)?;
                    if arg.is_none() {
                        formula::maybe_recalculate_line(d)?;
                    } else {
                        formula::recalculate(d, if arg == Prefix::U(2) { Recalc::Iterate } else { Recalc::All }, false)?;
                    }
                    d.align();
                    d.interactive = false;
                    orgtbl::send_table(d, true).map(|_| ())
                })
            } else if doc::is_tblfm(&line) {
                ok(ed, name, |d| {
                    let t = (0..d.l).rev().find(|&l| !d.lines[l].trim().is_empty()).ok_or("Not at a table")?;
                    if d.table_line_at(t) {
                        d.goto(t, 0);
                        formula::recalculate(d, Recalc::All, false)?;
                    }
                    Ok(())
                })
            } else {
                Err("C-c C-c is undefined outside tables in orgtbl-mode".into())
            }
        }
        "orgtbl-create-or-convert-from-region" => command(ed, "org-table-create-or-convert-from-region", arg).unwrap(),
        "orgtbl-tab" => {
            if arg.is_none() {
                ok(ed, name, |d| {
                    edit::justify_field_maybe(d, None);
                    edit::next_field(d)
                })
            } else {
                command(ed, "org-table-edit-field", Prefix::U(1)).unwrap()
            }
        }
        "orgtbl-ret" => ok(ed, name, |d| {
            if d.bobp() {
                d.insert("\n");
                return Ok(());
            }
            edit::justify_field_maybe(d, None);
            edit::next_row(d)
        }),
        "orgtbl-self-insert-command" => Err("orgtbl-self-insert-command runs from typed characters (Insert mode)".into()),
        "orgtbl-send-table" => ok(ed, name, |d| orgtbl::send_table(d, false).map(|_| ())),
        "orgtbl-toggle-comment" => ok(ed, name, orgtbl::toggle_comment),
        "orgtbl-insert-radio-table" => {
            read(ed, "Table name: ", "", |ed, n| {
                let r = ok(ed, "orgtbl-insert-radio-table", |d| orgtbl::insert_radio_table(d, &n));
                report(ed, r);
            });
            Ok(())
        }
        "orgtbl-ascii-plot" => match arg {
            Prefix::U(_) => {
                read(ed, "Length of column ", "12", |ed, len| {
                    let n = len.trim().parse().unwrap_or(12);
                    let r = ok(ed, "orgtbl-ascii-plot", |d| orgtbl::ascii_plot(d, n));
                    report(ed, r);
                });
                Ok(())
            }
            Prefix::Num(k) => ok(ed, name, |d| orgtbl::ascii_plot(d, k)),
            _ => ok(ed, name, |d| orgtbl::ascii_plot(d, 12)),
        },
        "orgtbl-error" => Err("This key has no function outside tables".into()),
        "orgtbl-setup" => Ok(()),
        "org-plot/gnuplot" => ok(ed, name, |d| plot::gnuplot(d, plot::Plot::default())),
        "org-plot/goto-nearest-table" => ok(ed, name, plot::goto_nearest_table),
        _ => {
            if let Some(num) = name.strip_prefix("orgtbl-hijacker-command-")
                && let Some(cmd) = num.parse().ok().and_then(hijacker)
            {
                if doc(ed).at_table() {
                    return Some(super::call(ed, cmd, arg));
                }
                return Some(Err("This key has no function outside tables".into()));
            }
            return None;
        }
    })
}
