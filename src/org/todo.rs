//! TODO keywords (org-todo), state logging and notes, CLOSED, statistics
//! cookies, blocking, priorities and sparse trees (org.el).

use super::props::{self, Doc, Inherit};
use super::sexp::Sexp;
use super::syntax::{self, Lines, Log, Settings};
use super::{Prefix, call, ctx, fold, tags};
use crate::editor::Editor;

/// How a state change is logged (org-log-done & co).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Time,
    Note,
}

/// Logging settings in effect for an entry (LOGGING property applied).
struct Logging {
    done: Option<How>,
    repeat: Option<How>,
    states: Vec<(String, Option<Log>, Option<Log>)>,
}

fn how_of(v: Option<Sexp>, default: Option<How>) -> Option<How> {
    match v {
        None => default,
        Some(Sexp::Nil) => None,
        Some(v) => match v.sym().or(v.str()) {
            Some("note") => Some(How::Note),
            Some("nil") => None,
            _ => Some(How::Time),
        },
    }
}

fn logging(ed: &Editor, st: &Settings, h: usize) -> Logging {
    let mut lg = Logging {
        done: how_of(st.opt("org-log-done"), None),
        repeat: how_of(st.opt("org-log-repeat"), Some(How::Time)),
        states: st
            .seqs
            .iter()
            .flat_map(|s| s.todo.iter().chain(&s.done))
            .filter(|k| k.enter.is_some() || k.leave.is_some())
            .map(|k| (k.name.clone(), k.enter, k.leave))
            .collect(),
    };
    // org-local-logging from the LOGGING property.
    if let Some(v) = props::get(ed, Some(h), "LOGGING", Inherit::Yes) {
        lg.done = None;
        lg.repeat = None;
        lg.states.clear();
        for w in v.split_whitespace() {
            if let Some((var, val)) = syntax::startup_option(w) {
                if var == "org-log-done" {
                    lg.done = how_of(Some(val), None);
                } else if var == "org-log-repeat" {
                    lg.repeat = how_of(Some(val), None);
                }
            } else {
                let k = syntax::parse_keyword(w);
                if st.is_todo(&k.name) && (k.enter.is_some() || k.leave.is_some()) {
                    lg.states.push((k.name, k.enter, k.leave));
                }
            }
        }
    }
    lg
}

fn log_to_how(l: Option<Log>) -> Option<How> {
    l.map(|l| if l == Log::Note { How::Note } else { How::Time })
}

/// org-log-into-drawer for the entry: the drawer name, if any.
pub fn log_into_drawer(ed: &Editor, h: usize) -> Option<String> {
    let p = props::get(ed, Some(h), "LOG_INTO_DRAWER", Inherit::Yes);
    let v = match p {
        Some(p) if p == "nil" => return None,
        Some(p) if p == "t" => return Some("LOGBOOK".into()),
        Some(p) => return Some(p),
        None => super::settings(ed).opt("org-log-into-drawer"),
    };
    match v {
        None | Some(Sexp::Nil) => None,
        Some(Sexp::T) => Some("LOGBOOK".into()),
        Some(v) => v.str().map(str::to_owned).or(Some("LOGBOOK".into())),
    }
}

/// org-end-of-meta-data: the first line after the planning line and the
/// property drawer of heading `h` (with `full`, also clock lines, drawers
/// and blank lines).
pub fn end_of_meta_data(ed: &Editor, h: usize, full: bool) -> usize {
    let n = ed.line_count();
    let mut l = h + 1;
    if l < n && ctx::is_planning(&ed.buf.line(l)) {
        l += 1;
    }
    if let Some((_, e)) = props::drawer(&ed.buf, Some(h)) {
        l = e + 1;
    }
    if full {
        while l < n {
            let t = ed.buf.line(l);
            let tt = t.trim();
            if tt.starts_with("CLOCK:") || tt.is_empty() && false {
                l += 1;
            } else if tt.len() > 2
                && tt.starts_with(':')
                && tt.ends_with(':')
                && !tt.eq_ignore_ascii_case(":END:")
            {
                match (l + 1..n)
                    .take_while(|&i| !ctx::at_heading(ed, i))
                    .find(|&i| ed.buf.line(i).trim().eq_ignore_ascii_case(":END:"))
                {
                    Some(e) => l = e + 1,
                    None => break,
                }
            } else {
                break;
            }
        }
    }
    l
}

/// org-log-beginning: the line where a log item goes (creating the drawer).
pub fn log_beginning(ed: &mut Editor, h: usize, create: bool) -> usize {
    let reversed = super::settings(ed).opt_bool("org-log-states-order-reversed", true);
    if let Some(drawer) = log_into_drawer(ed, h) {
        let start = end_of_meta_data(ed, h, false);
        let end = syntax::entry_end(&ed.buf, h);
        let open = format!(":{}:", drawer.to_ascii_uppercase());
        for i in start..end {
            if ed.buf.line(i).trim().eq_ignore_ascii_case(&open) {
                if !reversed
                    && let Some(e) =
                        (i + 1..end).find(|&j| ed.buf.line(j).trim().eq_ignore_ascii_case(":END:"))
                {
                    return e;
                }
                return i + 1;
            }
        }
        if create {
            let indent = if super::sexp::option("org-adapt-indentation").is_some_and(|v| v.truthy())
            {
                " ".repeat(syntax::level(&ed.buf.line(h)).unwrap_or(0) + 1)
            } else {
                String::new()
            };
            super::insert_lines(
                ed,
                start,
                &[format!("{indent}:{drawer}:"), format!("{indent}:END:")],
            );
            fold::region(ed, start + 1, start + 1, true, fold::Spec::Drawer);
            return start + 1;
        }
        return start;
    }
    let after_drawers = super::options::bool("org-log-state-notes-insert-after-drawers", false);
    let mut l = end_of_meta_data(ed, h, after_drawers);
    let end = syntax::entry_end(&ed.buf, h);
    if !reversed {
        // Skip existing state notes.
        while l < end && ed.buf.line(l).trim_start().starts_with("- State ") {
            l += 1;
            while l < end
                && ed.buf.line(l).starts_with("  ")
                && !ed.buf.line(l).trim_start().starts_with("- ")
            {
                l += 1;
            }
        }
    }
    l
}

/// org-log-note-headings entry for `purpose`.
fn note_heading(purpose: &str) -> String {
    if let Some(v) = super::sexp::option("org-log-note-headings")
        && let Some(l) = v.list()
        && let Some(e) = l
            .iter()
            .find(|e| e.car().and_then(Sexp::str) == Some(purpose))
    {
        return e.cdr().str().unwrap_or("").to_owned();
    }
    match purpose {
        "done" => "CLOSING NOTE %t",
        "state" => "State %-12s from %-12S %t",
        "note" => "Note taken on %t",
        "reschedule" => "Rescheduled from %S on %t",
        "delschedule" => "Not scheduled, was %S on %t",
        "redeadline" => "New deadline from %S on %t",
        "deldeadline" => "Removed deadline, was %S on %t",
        "refile" => "Refiled on %t",
        _ => "",
    }
    .into()
}

/// org-replace-escapes with widths (`%-12s`).
pub fn replace_escapes(fmt: &str, table: &[(char, String)]) -> String {
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        while let Some(&d) = it.peek() {
            if d == '-' || d.is_ascii_digit() {
                spec.push(d);
                it.next();
            } else {
                break;
            }
        }
        match it.next() {
            Some(k) => match table.iter().find(|(c, _)| *c == k) {
                Some((_, v)) => {
                    let left = spec.starts_with('-');
                    let w: usize = spec.trim_start_matches('-').parse().unwrap_or(0);
                    if left {
                        out.push_str(&format!("{v:<w$}"));
                    } else {
                        out.push_str(&format!("{v:>w$}"));
                    }
                }
                None => {
                    out.push('%');
                    out.push_str(&spec);
                    out.push(k);
                }
            },
            None => out.push('%'),
        }
    }
    out
}

/// A pending log note (org-add-log-setup).
#[derive(Clone, Debug)]
pub struct Note {
    pub purpose: String,
    pub state: Option<String>,
    pub prev: Option<String>,
    pub how: How,
    pub extra: Option<String>,
    pub time: i64,
}

fn quote_state(s: &Option<String>) -> String {
    match s {
        None => String::new(),
        Some(s) if s.starts_with('<') && s.ends_with('>') => {
            format!("\"[{}]\"", &s[1..s.len() - 1])
        }
        Some(s) => format!("\"{s}\""),
    }
}

/// The heading line of a note (org-store-log-note's `note`).
fn note_line(n: &Note) -> String {
    let user = std::env::var("USER").unwrap_or_default();
    let full = super::options::string("user-full-name", &user);
    let t = n.time;
    replace_escapes(
        &note_heading(&n.purpose),
        &[
            ('u', user),
            ('U', full),
            ('t', super::timestamp(t, true, true)),
            ('T', super::timestamp(t, true, false)),
            ('d', super::timestamp(t, false, true)),
            ('D', super::timestamp(t, false, false)),
            ('s', quote_state(&n.state)),
            ('S', quote_state(&n.prev)),
        ],
    )
}

/// Insert a stored note under heading `h` (org-store-log-note).
pub fn store_note(ed: &mut Editor, h: usize, n: &Note, text: &str) {
    let mut lines: Vec<String> = vec![];
    let body: String = text
        .lines()
        .skip_while(|l| l.starts_with("# ") || l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim_end().to_owned();
    let mut note = note_line(n);
    let extra: Vec<String> = if body.is_empty() {
        vec![]
    } else {
        body.lines().map(str::to_owned).collect()
    };
    if !note.trim().is_empty() {
        if !extra.is_empty() {
            note.push_str(" \\\\");
        }
        lines.push(note);
    }
    lines.extend(extra);
    if lines.is_empty() {
        return;
    }
    let at = if n.purpose == "clock-out" {
        h
    } else {
        log_beginning(ed, h, true)
    };
    // Indent like the list there, or like regular text.
    let ind = if at < ed.line_count() {
        let t = ed.buf.line(at);
        if ctx::item_bullet(&t).is_some() {
            t.len() - t.trim_start().len()
        } else if super::sexp::option("org-adapt-indentation").is_some_and(|v| v.truthy()) {
            syntax::level(&ed.buf.line(h)).unwrap_or(0) + 1
        } else {
            0
        }
    } else {
        0
    };
    let pad = " ".repeat(ind);
    let mut out = vec![format!("{pad}- {}", lines[0])];
    for l in &lines[1..] {
        out.push(if l.is_empty() {
            String::new()
        } else {
            format!("{pad}  {l}")
        });
    }
    super::insert_lines(ed, at, &out);
}

/// org-add-log-setup: log now (time) or open the note buffer.
pub fn add_log(ed: &mut Editor, h: usize, n: Note) {
    if n.how == How::Time {
        store_note(ed, h, &n, "");
        return;
    }
    let what = match n.purpose.as_str() {
        "clock-out" => "stopped clock".to_owned(),
        "done" => "closed todo item".into(),
        "reschedule" => "rescheduling".into(),
        "delschedule" => "no longer scheduled".into(),
        "redeadline" => "changing deadline".into(),
        "deldeadline" => "removing deadline".into(),
        "refile" => "refiling".into(),
        "note" => "this entry".into(),
        _ => format!(
            "state change from \"{}\" to \"{}\"",
            n.prev.clone().unwrap_or_default(),
            n.state.clone().unwrap_or_default()
        ),
    };
    let text = format!(
        "# Insert note for {what}.\n# Finish with C-c C-c, or cancel with C-c C-k.\n\n{}",
        n.extra.clone().unwrap_or_default()
    );
    let path = ed.path.clone();
    let heading = ed.buf.line(h);
    super::effect(ed, move |s| {
        let origin = s.org_current();
        let lines = text.lines().count();
        s.org_special(
            "*Org Note*",
            &text,
            (lines, 0),
            Box::new(move |s, text, abort| {
                if abort {
                    return;
                }
                let store = |ed: &mut Editor| {
                    // The heading may have moved: find it again.
                    let line = if h < ed.line_count() && ed.buf.line(h) == heading {
                        Some(h)
                    } else {
                        (0..ed.line_count()).find(|&i| ed.buf.line(i) == heading)
                    };
                    if let Some(l) = line {
                        ed.undo.begin(ed.cur.pos());
                        store_note(ed, l, &n, &text);
                        ed.undo.end(ed.cur.pos());
                        ed.set_msg("Note stored");
                    }
                };
                match &path {
                    Some(p) => {
                        let _ = s.org_with_file(p, store);
                    }
                    None => s.org_with_buffer(origin.min(s.org_buffer_count() - 1), store),
                }
            }),
        );
    });
}

/// CLOSED, SCHEDULED, DEADLINE on the planning line of `h`: add (Some
/// timestamp) or remove (None) one keyword.
pub fn set_planning(ed: &mut Editor, h: usize, key: &str, ts: Option<String>) {
    let n = ed.line_count();
    let has = h + 1 < n && ctx::is_planning(&ed.buf.line(h + 1));
    let mut items: Vec<(String, String)> = vec![];
    if has {
        let t = ed.buf.line(h + 1);
        for k in ["CLOSED:", "DEADLINE:", "SCHEDULED:"] {
            if let Some(i) = t.find(k) {
                let rest = t[i + k.len()..].trim_start();
                if let Some(len) = super::face::timestamp_len(rest) {
                    items.push((k.trim_end_matches(':').to_owned(), rest[..len].to_owned()));
                }
            }
        }
        // Keep the original order.
        items.sort_by_key(|(k, _)| t.find(&format!("{k}:")).unwrap_or(0));
    }
    match ts {
        Some(ts) => match items.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = ts,
            None => {
                // org-add-planning-info inserts CLOSED first.
                if key == "CLOSED" {
                    items.insert(0, (key.into(), ts));
                } else {
                    items.push((key.into(), ts));
                }
            }
        },
        None => items.retain(|(k, _)| k != key),
    }
    let indent = if has {
        let t = ed.buf.line(h + 1);
        t[..t.len() - t.trim_start().len()].to_owned()
    } else if super::sexp::option("org-adapt-indentation").is_some_and(|v| v.truthy()) {
        " ".repeat(syntax::level(&ed.buf.line(h)).unwrap_or(0) + 1)
    } else {
        String::new()
    };
    let text = items
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(" ");
    match (has, items.is_empty()) {
        (true, true) => super::delete_lines(ed, h + 1, 1),
        (true, false) => super::set_line(ed, h + 1, &format!("{indent}{text}")),
        (false, false) => super::insert_lines(ed, h + 1, &[format!("{indent}{text}")]),
        (false, true) => {}
    }
}

/// The blocker hooks (org-blocker-hook with the two upstream blockers).
/// Returns the blocking reason when blocked.
fn blocker<L: Lines + ?Sized>(
    doc: &Doc<L>,
    h: usize,
    from: Option<&str>,
    to: Option<&str>,
) -> Option<String> {
    let st = doc.st;
    let relevant = |from: Option<&str>, to: Option<&str>| {
        !(from.is_some_and(|f| st.is_done(f)) || to.is_none_or(|t| !st.is_done(t)))
    };
    if !relevant(from, to) {
        return None;
    }
    if super::options::bool("org-enforce-todo-dependencies", false) {
        let lvl = syntax::level(&doc.b.line_text(h)).unwrap_or(1);
        // Undone children.
        let end = syntax::subtree_end(doc.b, h);
        for l in h + 1..end {
            if let Some(c) = syntax::headline(&doc.b.line_text(l), st)
                && c.level > lvl
                && c.todo.as_deref().is_some_and(|k| !st.is_done(k))
            {
                return Some(format!("\"{}\"", heading_text(&doc.b.line_text(l), st)));
            }
        }
        // ORDERED parents with earlier undone siblings, up the hierarchy.
        let mut pos = h;
        let mut parent = syntax::parent(doc.b, pos);
        let mut first = true;
        while let Some(p) = parent {
            if !first
                && !syntax::headline(&doc.b.line_text(pos), st)
                    .is_some_and(|x| x.todo.as_deref().is_some_and(|k| !st.is_done(k)))
            {
                break;
            }
            first = false;
            let ordered = doc.get(Some(p), "ORDERED", Inherit::No, false).is_some();
            if ordered {
                for l in p + 1..pos {
                    if let Some(x) = syntax::headline(&doc.b.line_text(l), st)
                        && x.todo.as_deref().is_some_and(|k| !st.is_done(k))
                    {
                        return Some(format!("\"{}\"", heading_text(&doc.b.line_text(l), st)));
                    }
                }
            }
            pos = p;
            parent = syntax::parent(doc.b, p);
        }
    }
    if super::options::bool("org-enforce-todo-checkbox-dependencies", false) {
        let end = syntax::entry_end(doc.b, h);
        for l in h + 1..end {
            let t = doc.b.line_text(l);
            if let Some((ind, b)) = ctx::item_bullet(&t) {
                let rest = &t[ind + b.len()..];
                let rest = rest.strip_prefix("[@").map_or(rest, |r| {
                    r.find("] ").map_or(rest, |i| r[i + 2..].trim_start())
                });
                if rest.starts_with("[ ]") || rest.starts_with("[-]") {
                    return Some("contained checkboxes".into());
                }
            }
        }
    }
    None
}

fn heading_text(line: &str, st: &Settings) -> String {
    match syntax::headline(line, st) {
        Some(h) => {
            let mut parts = vec![];
            if let Some(k) = &h.todo {
                parts.push(k.clone());
            }
            if let Some(r) = &h.priority_range {
                parts.push(line[r.clone()].to_owned());
            }
            parts.push(h.title(line).to_owned());
            parts.join(" ")
        }
        None => line.to_owned(),
    }
}

/// org-entry-blocked-p.
pub fn blocked<L: Lines + ?Sized>(doc: &Doc<L>, h: usize) -> bool {
    if doc.get(Some(h), "NOBLOCKING", Inherit::No, false).is_some() {
        return false;
    }
    let st = doc.st;
    let Some(hl) = syntax::headline(&doc.b.line_text(h), st) else {
        return false;
    };
    let Some(k) = hl.todo else { return false };
    if st.is_done(&k) {
        return false;
    }
    let done = st.done_names().first().map(|s| s.to_string());
    blocker(doc, h, Some("TODO-not-done"), done.as_deref()).is_some()
}

/// The fast TODO selection menu (org-fast-todo-selection).
fn fast_todo_selection(
    ed: &mut Editor,
    st: &Settings,
    then: impl FnOnce(&mut Editor, Option<String>) + Send + 'static,
) {
    let mut entries: Vec<(String, String)> = vec![];
    for seq in &st.seqs {
        let names: Vec<String> = seq
            .todo
            .iter()
            .chain(&seq.done)
            .map(|k| k.name.clone())
            .collect();
        entries.push((String::new(), names.join(" ")));
        for k in seq.todo.iter().chain(&seq.done) {
            if let Some(c) = k.key {
                entries.push((c.to_string(), k.name.clone()));
            }
        }
    }
    entries.push((" ".into(), "clear (no keyword)".into()));
    let keys: Vec<(char, String)> = st
        .seqs
        .iter()
        .flat_map(|s| s.todo.iter().chain(&s.done))
        .filter_map(|k| k.key.map(|c| (c, k.name.clone())))
        .collect();
    super::menu(ed, "TODO state:", entries, move |ed, k| {
        if k == " " {
            then(ed, None);
        } else if let Some((_, name)) = keys.iter().find(|(c, _)| c.to_string() == k) {
            then(ed, Some(name.clone()));
        }
    });
}

/// org-todo with a computed target state (`None` = no keyword), as
/// org-todo does after choosing it.
pub fn todo_to(ed: &mut Editor, state: Option<String>) -> Result<(), String> {
    change_state(ed, Target::State(state), false, false, None)
}

#[derive(Clone, Debug)]
enum Target {
    /// Cycle forward (plain org-todo).
    Next,
    Right,
    Left,
    NextSet,
    PrevSet,
    Done,
    Nth(usize),
    State(Option<String>),
}

/// The core of org-todo.
fn change_state(
    ed: &mut Editor,
    target: Target,
    force_log: bool,
    no_block: bool,
    inhibit: Option<How>,
) -> Result<(), String> {
    let l = ed.cur.line;
    let h = fold::back_to_heading(ed, l).ok_or("Before first headline")?;
    let st = super::settings(ed);
    let line = ed.buf.line(h);
    let hl = syntax::headline(&line, &st).ok_or("Not at a heading")?;
    let this = hl.todo.clone();
    let all: Vec<String> = st.todo_names().iter().map(|s| s.to_string()).collect();
    let seq = this.as_deref().and_then(|k| st.seq_of(k));
    let head = seq.and_then(|s| s.names().next()).map(str::to_owned);
    let pos = this.as_ref().and_then(|t| all.iter().position(|k| k == t));
    let tail: Vec<String> = pos.map_or(vec![], |p| all[p + 1..].to_vec());
    let repeat = ed.org.as_ref().and_then(|o| o.last_command.as_deref()) == Some("org-todo");
    let heads: Vec<String> = st
        .seqs
        .iter()
        .filter_map(|s| s.names().next().map(str::to_owned))
        .collect();
    let new: Option<String> = match &target {
        Target::Right => match &this {
            Some(_) => tail.first().cloned(),
            None => all.first().cloned(),
        },
        Target::Left => match pos {
            Some(0) => None,
            Some(p) => Some(all[p - 1].clone()),
            None => all.last().cloned(),
        },
        Target::Done => seq
            .and_then(|s| s.done.first())
            .map(|k| k.name.clone())
            .or_else(|| st.done_names().first().map(|s| s.to_string())),
        Target::NextSet | Target::PrevSet => {
            let mut hs = heads.clone();
            if matches!(target, Target::PrevSet) {
                hs.reverse();
            }
            let i = head.as_ref().and_then(|h| hs.iter().position(|x| x == h));
            match i {
                Some(i) if i + 1 < hs.len() => Some(hs[i + 1].clone()),
                _ => hs.first().cloned(),
            }
        }
        Target::Nth(n) => all.get(n.saturating_sub(1)).cloned(),
        Target::State(s) => {
            if let Some(s) = s
                && !all.contains(s)
            {
                return Err(format!("State `{s}' not valid in this file"));
            }
            s.clone()
        }
        Target::Next => {
            let s = seq;
            let final_done = s.and_then(|s| s.done.last()).map(|k| k.name.clone());
            let done_word = s.and_then(|s| s.done.first()).map(|k| k.name.clone());
            match (&this, pos) {
                (None, _) | (_, None) => head.clone().or_else(|| all.first().cloned()),
                (Some(t), _) if Some(t) == final_done.as_ref() => None,
                _ if tail.is_empty() => None,
                _ if s.is_some_and(|s| s.is_type) => {
                    if repeat {
                        tail.first().cloned()
                    } else {
                        done_word.or_else(|| st.done_names().first().map(|s| s.to_string()))
                    }
                }
                _ => tail.first().cloned(),
            }
        }
    };
    // Blocking.
    if !no_block {
        let path = ed.path.clone();
        let doc = Doc::new(&ed.buf, &st, path.as_deref());
        if doc.get(Some(h), "NOBLOCKING", Inherit::No, false).is_none()
            && !super::options::bool("org-inhibit-blocking", false)
            && let Some(reason) = blocker(&doc, h, this.as_deref(), new.as_deref())
        {
            return Err(format!(
                "TODO state change from {} to {} blocked (by {reason})",
                this.clone().unwrap_or_else(|| "nil".into()),
                new.clone().unwrap_or_else(|| "nil".into())
            ));
        }
    }
    // Replace the keyword.
    let rebuilt = {
        let after_stars = hl.level;
        let rest_start = match &hl.todo_range {
            Some(r) => r.end,
            None => after_stars,
        };
        let rest = line[rest_start..].trim_start_matches(' ');
        let mut s = line[..after_stars].to_owned();
        s.push(' ');
        if let Some(n) = &new {
            s.push_str(n);
            if !rest.is_empty() {
                s.push(' ');
            }
        }
        s.push_str(rest);
        if new.is_none() && rest.is_empty() {
            // Keep "* " as a heading.
        }
        s
    };
    let cursor_in = ed.cur.line == h;
    let old_byte = ed.cur.byte;
    super::set_line(ed, h, &rebuilt);
    if new.is_some() && new == this {
        ed.set_msg(format!("TODO state was already {}", new.clone().unwrap()));
    }
    if matches!(target, Target::NextSet | Target::PrevSet)
        && let Some(n) = &new
        && let Some(i) = st.seqs.iter().position(|s| s.names().any(|x| x == n))
    {
        ed.set_msg(format!(
            "Keyword-Set {}/{}: {}",
            i + 1,
            st.seqs.len(),
            st.seqs[i].names().collect::<Vec<_>>().join(" ")
        ));
    }
    let now_done = new.as_deref().is_some_and(|n| st.is_done(n))
        && !this.as_deref().is_some_and(|t| st.is_done(t));
    // Logging.
    let lg = logging(ed, &st, h);
    let mut logged: Option<How> = None;
    let set_change = matches!(target, Target::NextSet | Target::PrevSet);
    if (!lg.states.is_empty() || lg.done.is_some()) && inhibit != Some(How::Time) && !set_change
        || force_log
    {
        let mut dolog = if force_log {
            Some(How::Note)
        } else {
            new.as_ref()
                .and_then(|n| lg.states.iter().find(|(k, _, _)| k == n))
                .and_then(|(_, e, _)| log_to_how(*e))
                .or_else(|| {
                    this.as_ref()
                        .and_then(|t| lg.states.iter().find(|(k, _, _)| k == t))
                        .and_then(|(_, _, l)| log_to_how(*l))
                })
        };
        if dolog == Some(How::Note) && inhibit == Some(How::Note) {
            dolog = Some(How::Time);
        }
        let keep = super::options::bool("org-closed-keep-when-no-todo", false);
        let becomes_todo = new.as_deref().is_some_and(|n| !st.is_done(n))
            && !this
                .as_deref()
                .is_some_and(|t| !st.is_done(t) && st.is_todo(t));
        if (new.is_none() && !keep) || becomes_todo {
            set_planning(ed, h, "CLOSED", None);
        }
        let time = super::now();
        if now_done && let Some(how) = lg.done {
            set_planning(ed, h, "CLOSED", Some(super::timestamp(time, true, true)));
            if dolog.is_none() && how == How::Note {
                logged = Some(How::Note);
                pending_log(
                    ed,
                    h,
                    Note {
                        purpose: "done".into(),
                        state: new.clone(),
                        prev: this.clone(),
                        how: How::Note,
                        extra: None,
                        time,
                    },
                );
            }
        }
        if new.is_some()
            && let Some(how) = dolog
        {
            logged = Some(how);
            pending_log(
                ed,
                h,
                Note {
                    purpose: "state".into(),
                    state: new.clone(),
                    prev: this.clone(),
                    how,
                    extra: None,
                    time,
                },
            );
        }
    }
    // Tag triggers.
    trigger_tags(ed, h, &st, new.as_deref());
    if super::options::bool("org-auto-align-tags", true) {
        tags::align(ed, h);
    }
    if super::sexp::option("org-provide-todo-statistics").is_none_or(|v| v.truthy()) {
        update_parent_statistics(ed, h);
    }
    if now_done {
        if super::options::bool("org-clock-out-when-done", true) {
            let _ = call(ed, "org-clock-out-if-current", Prefix::None);
        }
        repeat_hook(ed, h, new.as_deref(), this.clone(), &lg, &mut logged)?;
    }
    flush_log(ed, h);
    // Fix up the cursor near the keyword.
    if cursor_in {
        let line = ed.buf.line(h);
        let hl = syntax::headline(&line, &st);
        let kw_end = hl
            .as_ref()
            .and_then(|x| x.todo_range.as_ref().map(|r| r.end))
            .unwrap_or(hl.as_ref().map_or(0, |x| x.level));
        if old_byte < kw_end + 2
            || old_byte
                <= this
                    .as_ref()
                    .map_or(0, |t| hl.as_ref().map_or(0, |x| x.level) + 1 + t.len() + 1)
        {
            let b = if line[kw_end..].starts_with(' ')
                && !line[kw_end..].trim_start().starts_with(':')
            {
                kw_end + 1
            } else {
                kw_end
            };
            ed.set_cursor(h, b.min(line.len()));
        } else {
            let delta = line.len() as i64 - rebuilt.len() as i64;
            ed.set_cursor(h, (old_byte as i64 + delta).max(0) as usize);
        }
    }
    Ok(())
}

thread_local! {
    /// A log set up by the current command (org-log-setup), stored when
    /// the command finishes (post-command-hook).
    static PENDING: std::cell::RefCell<Option<Note>> = const { std::cell::RefCell::new(None) };
}

/// org-add-log-setup: remember the note; [`flush_log`] stores it.
fn pending_log(_ed: &mut Editor, _h: usize, n: Note) {
    PENDING.with(|p| *p.borrow_mut() = Some(n));
}

/// org-add-log-note, run after the command.
fn flush_log(ed: &mut Editor, h: usize) {
    if let Some(n) = PENDING.with(|p| p.borrow_mut().take()) {
        // The heading may have moved by the planning-line edits.
        let h = if ctx::at_heading(ed, h) {
            h
        } else {
            fold::back_to_heading(ed, h).unwrap_or(h)
        };
        add_log(ed, h, n);
    }
}

/// org-get-repeat: the first repeater of an active timestamp in the entry.
pub fn get_repeat(ed: &Editor, h: usize) -> Option<String> {
    let re = crate::org_re!(r"<\d{4}-\d\d-\d\d [^>\n]*?([.+]?\+\d+[hdwmy](/\d+[hdwmy])?)");
    let end = syntax::entry_end(&ed.buf, h);
    (h..end).find_map(|l| re.captures(&ed.buf.line(l)).map(|c| c[1].to_owned()))
}

/// org-auto-repeat-maybe.
fn repeat_hook(
    ed: &mut Editor,
    h: usize,
    done_word: Option<&str>,
    last: Option<String>,
    lg: &Logging,
    logged: &mut Option<How>,
) -> Result<(), String> {
    let Some(rep) = get_repeat(ed, h) else {
        return Ok(());
    };
    let n: i64 = rep
        .trim_start_matches(['.', '+'])
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .split('/')
        .next()
        .unwrap_or("0")
        .trim_end_matches(|c: char| c.is_alphabetic())
        .parse()
        .unwrap_or(0);
    if n == 0 {
        return Ok(());
    }
    let st = super::settings(ed);
    let seq = last.as_deref().and_then(|l| st.seq_of(l));
    let head = seq.and_then(|s| s.names().next()).map(str::to_owned);
    let is_type = seq.is_some_and(|s| s.is_type);
    let to_state = props::get(ed, Some(h), "REPEAT_TO_STATE", Inherit::Selective).or_else(|| {
        match super::sexp::option("org-todo-repeat-to-state") {
            Some(Sexp::Str(s)) => Some(s),
            Some(v) if v.truthy() => last.clone(),
            _ => None,
        }
    });
    let target = match to_state {
        Some(t) if st.is_todo(&t) => Some(t),
        _ if is_type => last.clone(),
        _ => head,
    };
    // The reset itself is not logged (org-log-done and states bound to nil).
    let saved = ed.cur;
    ed.set_cursor(h, 0);
    change_state(ed, Target::State(target), false, true, Some(How::Time))?;
    ed.cur = saved;
    set_planning(ed, h, "CLOSED", None);
    let log_repeat = lg.repeat;
    let end = syntax::entry_end(&ed.buf, h);
    let has_clock = (h..end).any(|l| ed.buf.line(l).trim_start().starts_with("CLOCK:"));
    if log_repeat.is_some() || has_clock {
        props::put(
            ed,
            Some(h),
            "LAST_REPEAT",
            &super::timestamp(super::now(), true, true),
        )?;
    }
    if let Some(how) = log_repeat {
        if logged.is_some() {
            if how == How::Note {
                PENDING.with(|p| {
                    if let Some(n) = p.borrow_mut().as_mut() {
                        n.how = How::Note;
                    }
                });
            }
        } else {
            *logged = Some(how);
            let state = done_word
                .map(str::to_owned)
                .or_else(|| st.done_names().first().map(|s| s.to_string()));
            pending_log(
                ed,
                h,
                Note {
                    purpose: "state".into(),
                    state,
                    prev: last,
                    how,
                    extra: None,
                    time: super::now(),
                },
            );
        }
    }
    if let Some(msg) = super::time::auto_repeat(ed, h)? {
        ed.set_msg(msg);
    }
    Ok(())
}

/// org-todo-trigger-tag-changes.
fn trigger_tags(ed: &mut Editor, h: usize, st: &Settings, state: Option<&str>) {
    let Some(v) = super::sexp::option("org-todo-state-tags-triggers") else {
        return;
    };
    let Some(list) = v.list() else { return };
    let find = |key: &dyn Fn(&Sexp) -> bool| -> Vec<(String, bool)> {
        list.iter()
            .filter(|e| e.car().is_some_and(key))
            .flat_map(|e| {
                e.cdr()
                    .list()
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|c| Some((c.car()?.str()?.to_owned(), c.cdr().truthy())))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let mut changes = vec![];
    match state {
        None | Some("") => changes.extend(find(&|k| k.str() == Some(""))),
        Some(s) => {
            changes.extend(find(&|k| matches!(k, Sexp::Str(x) if x == s)));
            if st.is_todo(s) && !st.is_done(s) {
                changes.extend(find(&|k| matches!(k, Sexp::Sym(x) if x == "todo")));
            }
            if st.is_done(s) {
                changes.extend(find(&|k| matches!(k, Sexp::Sym(x) if x == "done")));
            }
        }
    }
    for (t, on) in changes {
        tags::toggle_tag(ed, h, &t, Some(on));
    }
}

/// org-format-percent-cookie.
pub fn percent_cookie(done: usize, total: usize) -> String {
    let p = (100 * done) / total.max(1);
    format!("[{}%]", if p == 0 && done > 0 { 1 } else { p })
}

/// The statistics cookies on line `l`: byte ranges and percent-ness.
fn cookies(line: &str) -> Vec<(std::ops::Range<usize>, bool)> {
    let re = crate::org_re!(r"\[(\d*%|\d*/\d*)\]");
    re.find_iter(line)
        .map(|m| (m.range(), m.as_str().ends_with("%]")))
        .collect()
}

/// org-update-parent-todo-statistics from heading `h`.
pub fn update_parent_statistics(ed: &mut Editor, h: usize) {
    let st = super::settings(ed);
    let provide = super::sexp::option("org-provide-todo-statistics").unwrap_or(Sexp::T);
    let children_only = super::options::bool("org-todo-children-only-statistics", true);
    let ltoggle = syntax::level(&ed.buf.line(h)).unwrap_or(1);
    let parent0 = syntax::parent(&ed.buf, h);
    let cookie_data = |ed: &Editor, p: usize| props::get(ed, Some(p), "COOKIE_DATA", Inherit::Yes);
    let prop = parent0.and_then(|p| cookie_data(ed, p));
    let recursive = !children_only
        || prop
            .as_deref()
            .is_some_and(|p| p.split_whitespace().any(|w| w == "recursive"));
    let mut first = true;
    let mut cur = h;
    while let Some(p) = syntax::parent(&ed.buf, cur) {
        if !(recursive || first) {
            break;
        }
        first = false;
        cur = p;
        let cd = props::get(ed, Some(p), "COOKIE_DATA", Inherit::No)
            .unwrap_or_default()
            .to_lowercase();
        if cd.split_whitespace().any(|w| w == "checkbox") {
            break;
        }
        let level = syntax::level(&ed.buf.line(p)).unwrap();
        let line = ed.buf.line(p);
        let cs = cookies(&line);
        if cs.is_empty() {
            continue;
        }
        let end = syntax::subtree_end(&ed.buf, p);
        let (mut all, mut done) = (0usize, 0usize);
        for l in p + 1..end {
            let Some(c) = syntax::headline(&ed.buf.line(l), &st) else {
                continue;
            };
            if c.level <= level {
                break;
            }
            let kwd = if recursive || c.level == ltoggle {
                c.todo.clone()
            } else {
                None
            };
            let is_done = kwd.as_deref().is_some_and(|k| st.is_done(k));
            let counts = match &provide {
                Sexp::Sym(s) if s == "all-headlines" => true,
                Sexp::T => kwd.is_some(),
                Sexp::List(v) if v.first().is_some_and(|x| matches!(x, Sexp::Str(_))) => kwd
                    .as_deref()
                    .is_some_and(|k| v.iter().any(|x| x.str() == Some(k)) || st.is_done(k)),
                Sexp::List(v) => {
                    let todo = v.first().and_then(Sexp::list).unwrap_or(&[]);
                    let dn = v.get(1).and_then(Sexp::list).unwrap_or(&[]);
                    kwd.as_deref().is_some_and(|k| {
                        todo.iter().any(|x| x.str() == Some(k))
                            || (st.is_done(k) && dn.iter().any(|x| x.str() == Some(k)))
                    })
                }
                _ => false,
            };
            if counts {
                all += 1;
            }
            let done_counts = match &provide {
                Sexp::T => is_done,
                Sexp::Sym(s) if s == "all-headlines" => is_done,
                Sexp::List(v) if v.first().is_some_and(|x| matches!(x, Sexp::Str(_))) => is_done,
                Sexp::List(v) => {
                    is_done
                        && v.get(1)
                            .and_then(Sexp::list)
                            .unwrap_or(&[])
                            .iter()
                            .any(|x| Some(x.str().unwrap_or("")) == kwd.as_deref())
                }
                _ => false,
            };
            if done_counts {
                done += 1;
            }
        }
        let mut newline = line.clone();
        for (r, pct) in cs.into_iter().rev() {
            let new = if pct {
                percent_cookie(done, all)
            } else {
                format!("[{done}/{all}]")
            };
            newline.replace_range(r, &new);
        }
        if newline != line {
            super::set_line(ed, p, &newline);
            if super::options::bool("org-auto-align-tags", true) {
                tags::align(ed, p);
            }
        }
    }
}

/// org-update-statistics-cookies.
pub fn update_statistics_cookies(ed: &mut Editor, all: bool) -> Result<(), String> {
    if all {
        let _ = call(ed, "org-update-checkbox-count", Prefix::U(1));
        let heads: Vec<usize> = (0..ed.line_count())
            .filter(|&l| ctx::at_heading(ed, l))
            .collect();
        for h in heads {
            update_parent_statistics(ed, h);
        }
        return Ok(());
    }
    let l = ed.cur.line;
    if !ctx::at_heading(ed, l) {
        return call(ed, "org-update-checkbox-count", Prefix::None);
    }
    let end = syntax::entry_end(&ed.buf, l);
    let has_boxes = (l + 1..end).any(|i| ctx::at_checkbox(ed, i));
    let todo_cookie = (l + 1..end).any(|i| {
        let t = ed.buf.line(i);
        t.contains(":COOKIE_DATA:") && t.contains("todo")
    });
    if has_boxes && !todo_cookie {
        return call(ed, "org-update-checkbox-count", Prefix::None);
    }
    let child = end < ed.line_count()
        && syntax::level(&ed.buf.line(end))
            .is_some_and(|c| c > syntax::level(&ed.buf.line(l)).unwrap());
    if child {
        update_parent_statistics(ed, end);
    } else {
        let line = ed.buf.line(l);
        let mut newline = line.clone();
        for (r, pct) in cookies(&line).into_iter().rev() {
            newline.replace_range(r, if pct { "[100%]" } else { "[0/0]" });
        }
        super::set_line(ed, l, &newline);
    }
    Ok(())
}

/// org-todo.
pub fn todo(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if let Some((lo, hi)) = ed.org_region
        && super::options::bool("org-loop-over-headlines-in-active-region", true)
    {
        let heads: Vec<usize> = (lo..=hi).filter(|&l| ctx::at_heading(ed, l)).collect();
        ed.org_region = None;
        for h in heads {
            ed.set_cursor(h, 0);
            todo(ed, arg)?;
        }
        return Ok(());
    }
    let st = super::settings(ed);
    match arg {
        Prefix::U(2) => change_state(ed, Target::NextSet, false, false, None),
        Prefix::U(1) => change_state(ed, Target::Next, true, false, None),
        Prefix::U(_) => change_state(ed, Target::Next, false, true, None),
        Prefix::Minus | Prefix::Num(-1) => {
            let _ = call(ed, "org-cancel-repeaters", Prefix::None);
            change_state(ed, Target::Next, false, false, None)
        }
        Prefix::Num(0) => change_state(ed, Target::Next, false, false, Some(How::Note)),
        Prefix::Num(n) if n < -1 => Err(format!("Prefix argument {n} not supported")),
        Prefix::Num(n) => change_state(ed, Target::Nth(n as usize), false, false, None),
        Prefix::None => {
            let has_keys = st
                .seqs
                .iter()
                .any(|s| s.todo.iter().chain(&s.done).any(|k| k.key.is_some()));
            let fast = match super::sexp::option("org-use-fast-todo-selection") {
                Some(Sexp::Nil) => false,
                Some(Sexp::Sym(s)) if s == "expert" || s == "auto" => has_keys,
                Some(Sexp::T) => has_keys,
                None => has_keys,
                _ => has_keys,
            };
            if fast {
                fast_todo_selection(ed, &st, |ed, s| {
                    ed.undo.begin(ed.cur.pos());
                    if let Err(e) = change_state(ed, Target::State(s), false, false, None) {
                        ed.set_err(e);
                    }
                    ed.undo.end(ed.cur.pos());
                });
                Ok(())
            } else {
                change_state(ed, Target::Next, false, false, None)
            }
        }
    }
}

/// org-todo 'done: the first DONE state of the entry's sequence.
pub fn mark_done(ed: &mut Editor) -> Result<(), String> {
    change_state(ed, Target::Done, false, false, None)
}

/// S-left/right on a heading: org-todo 'left / 'right.
pub fn shift_todo(ed: &mut Editor, right: bool, inhibit_logging: bool) -> Result<(), String> {
    let t = if right { Target::Right } else { Target::Left };
    change_state(
        ed,
        t,
        false,
        inhibit_logging,
        if inhibit_logging {
            Some(How::Time)
        } else {
            None
        },
    )
}

/// C-S-left/right on a heading: previous/next keyword set.
pub fn shift_set(ed: &mut Editor, right: bool) -> Result<(), String> {
    change_state(
        ed,
        if right {
            Target::NextSet
        } else {
            Target::PrevSet
        },
        false,
        false,
        None,
    )
}

/// org-priority with an action.
pub fn set_priority(ed: &mut Editor, value: Option<u32>) -> Result<(), String> {
    priority(ed, PriorityAction::Set(value))
}

#[derive(Clone, Copy, Debug)]
pub enum PriorityAction {
    Set(Option<u32>),
    Up,
    Down,
}

fn priority(ed: &mut Editor, action: PriorityAction) -> Result<(), String> {
    if !super::options::bool("org-priority-enable-commands", true) {
        return Err("Priority commands are disabled".into());
    }
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let st = super::settings(ed);
    let (high, low, default) = st.priorities;
    let line = ed.buf.line(h);
    let hl = syntax::headline(&line, &st).ok_or("Not at a heading")?;
    let current = hl.priority;
    let repeat = ed
        .org
        .as_ref()
        .and_then(|o| o.last_command.as_deref())
        .is_some_and(|c| {
            c.starts_with("org-priority") || c == "org-shiftup" || c == "org-shiftdown"
        });
    let start_default = super::options::bool("org-priority-start-cycle-with-default", true);
    let valid = |v: u32| v >= high && v <= low;
    let mut remove = false;
    let new: u32 = match action {
        PriorityAction::Set(None) => {
            remove = true;
            0
        }
        PriorityAction::Set(Some(v)) => {
            if !valid(v) {
                return Err(format!(
                    "Priority must be between `{}' and `{}'",
                    props::priority_string(high),
                    props::priority_string(low)
                ));
            }
            v
        }
        PriorityAction::Up => match current {
            Some(c) => c.wrapping_sub(1),
            None if repeat => low,
            None => {
                if start_default {
                    default
                } else {
                    default - 1
                }
            }
        },
        PriorityAction::Down => match current {
            Some(c) => c + 1,
            None if repeat => high,
            None => {
                if start_default {
                    default
                } else {
                    default + 1
                }
            }
        },
    };
    if !remove && !valid(new) {
        if matches!(action, PriorityAction::Up | PriorityAction::Down)
            && current.is_none()
            && !repeat
        {
            return Err("The default can not be set, see `org-priority-default' why".into());
        }
        remove = true;
    }
    let newline = match (&hl.priority_range, remove) {
        (Some(r), true) => {
            let end = if line[r.end..].starts_with(' ') {
                r.end + 1
            } else {
                r.end
            };
            format!("{}{}", &line[..r.start], &line[end..])
        }
        (Some(r), false) => format!(
            "{}[#{}]{}",
            &line[..r.start],
            props::priority_string(new),
            &line[r.end..]
        ),
        (None, true) => return Err("No priority cookie found in line".into()),
        (None, false) => {
            let at = hl.todo_range.as_ref().map_or(hl.level, |r| r.end);
            let rest = &line[at..];
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if rest.is_empty() {
                format!("{} [#{}]", &line[..at], props::priority_string(new))
            } else {
                format!(
                    "{} [#{}] {}",
                    &line[..at],
                    props::priority_string(new),
                    rest
                )
            }
        }
    };
    super::set_line(ed, h, &newline);
    if super::options::bool("org-auto-align-tags", true) {
        tags::align(ed, h);
    }
    ed.set_msg(format!(
        "Priority of current item set to {}",
        if remove {
            "removed".into()
        } else {
            props::priority_string(new)
        }
    ));
    Ok(())
}

/// org-priority-show.
fn priority_show(ed: &mut Editor) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let st = super::settings(ed);
    let hl = syntax::headline(&ed.buf.line(h), &st).ok_or("Not at a heading")?;
    let p = hl.priority.unwrap_or(st.priorities.2);
    ed.set_msg(format!("Priority is {}", p as i64 * 1000));
    Ok(())
}

/// org-occur: show lines matching `re`, return the number of matches.
pub fn occur(ed: &mut Editor, re: &str, keep: bool) -> Result<usize, String> {
    let r = super::re::compile(re, super::re::smart_fold(re))?;
    let hits: Vec<usize> = (0..ed.line_count())
        .filter(|&l| r.is_match(&ed.buf.line(l)))
        .collect();
    if !keep {
        tags::sparse_show(ed, &[], "occur-tree");
    }
    for &l in &hits {
        fold::show_context_for(ed, l, "occur-tree");
        if fold::hidden(ed, l) {
            fold::show_set_visibility(ed, l, "lineage");
        }
    }
    if let Some(o) = &mut ed.org {
        if keep {
            o.sparse_hits.extend(&hits);
        } else {
            o.sparse_hits = hits.clone();
        }
    }
    ed.set_msg(format!("{} match(es) for regexp {re}", hits.len()));
    Ok(hits.len())
}

/// org-show-todo-tree.
fn show_todo_tree(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let st = super::settings(ed);
    let not_done: Vec<String> = st.not_done_names().iter().map(|s| s.to_string()).collect();
    let go = move |ed: &mut Editor, kwds: Vec<String>| {
        let alt = kwds
            .iter()
            .map(|k| regex::escape(k))
            .collect::<Vec<_>>()
            .join("|");
        let re = format!(r"^\*+ +({alt})(\s|$)");
        let r = regex::Regex::new(&re).unwrap();
        let hits: Vec<usize> = (0..ed.line_count())
            .filter(|&l| r.is_match(&ed.buf.line(l)))
            .collect();
        tags::sparse_show(ed, &hits, "occur-tree");
        ed.set_msg(format!("{} TODO entries found", hits.len()));
        if let Some(o) = &mut ed.org {
            o.sparse_hits = hits;
        }
    };
    match arg {
        Prefix::None => go(ed, not_done),
        Prefix::U(1) => {
            let all: Vec<String> = st.todo_names().iter().map(|s| s.to_string()).collect();
            super::complete(
                ed,
                "Keyword (or KWD1|KWD2|...): ",
                all,
                false,
                move |ed, k| {
                    go(ed, k.split('|').map(str::to_owned).collect());
                },
            );
        }
        p => {
            let n = p.value().max(1) as usize;
            let all: Vec<String> = st.todo_names().iter().map(|s| s.to_string()).collect();
            match all.get(n - 1) {
                Some(k) => go(ed, vec![k.clone()]),
                None => return Err(format!("No keyword number {n}")),
            }
        }
    }
    Ok(())
}

/// org-sparse-tree (C-c /).
fn sparse_tree(ed: &mut Editor, arg: Prefix) {
    let keep = !arg.is_none();
    super::menu(
        ed,
        "Sparse tree:",
        vec![
            ("r".into(), "regexp".into()),
            ("t".into(), "todo".into()),
            ("T".into(), "todo-kwd".into()),
            ("m".into(), "tags-match".into()),
            ("p".into(), "property".into()),
            ("d".into(), "deadlines".into()),
            ("b".into(), "before-date".into()),
            ("a".into(), "after-date".into()),
            ("D".into(), "dates range".into()),
            ("/".into(), "regexp".into()),
            ("q".into(), "quit".into()),
        ],
        move |ed, k| {
            let r = match k.as_str() {
                "r" | "/" => {
                    super::read(ed, "Regexp: ", "", move |ed, re| {
                        if let Err(e) = occur(ed, &re, keep) {
                            ed.set_err(e);
                        }
                    });
                    Ok(())
                }
                "t" => show_todo_tree(ed, Prefix::None),
                "T" => show_todo_tree(ed, Prefix::U(1)),
                "m" => call(ed, "org-match-sparse-tree", Prefix::None),
                "p" => {
                    let keys = props::buffer_keys(ed, false, false, false);
                    super::complete(ed, "Property: ", keys, false, |ed, p| {
                        let vals = props::values(ed, &p);
                        super::complete(
                            ed,
                            &format!("Value for {p}: "),
                            vals,
                            false,
                            move |ed, v| {
                                let v = if v.contains(char::is_whitespace) {
                                    format!("\"{v}\"")
                                } else {
                                    format!("{{^{}$}}", regex::escape(&v))
                                };
                                tags::match_sparse_tree(ed, false, &format!("{p}={v}"));
                            },
                        );
                    });
                    Ok(())
                }
                "d" => call(ed, "org-check-deadlines", Prefix::None),
                "b" => call(ed, "org-check-before-date", Prefix::None),
                "a" => call(ed, "org-check-after-date", Prefix::None),
                "D" => call(ed, "org-check-dates-range", Prefix::None),
                _ => Ok(()),
            };
            if let Err(e) = r {
                ed.set_err(e);
            }
        },
    );
}

/// org-toggle-ordered-property.
fn toggle_ordered(ed: &mut Editor) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let tag = match super::sexp::option("org-track-ordered-property-with-tag") {
        None | Some(Sexp::Nil) => None,
        Some(Sexp::Str(s)) => Some(s),
        Some(_) => Some("ORDERED".to_owned()),
    };
    if props::get(ed, Some(h), "ORDERED", Inherit::No).is_some() {
        props::delete(ed, Some(h), "ORDERED");
        if let Some(t) = &tag {
            tags::toggle_tag(ed, h, t, Some(false));
        }
        ed.set_msg("Subtasks can be completed in arbitrary order");
    } else {
        props::put(ed, Some(h), "ORDERED", "t")?;
        if let Some(t) = &tag {
            tags::toggle_tag(ed, h, t, Some(true));
        }
        ed.set_msg("Subtasks must be completed in sequence");
    }
    Ok(())
}

/// org-toggle-comment.
fn toggle_comment(ed: &mut Editor) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let st = super::settings(ed);
    let line = ed.buf.line(h);
    let hl = syntax::headline(&line, &st).unwrap();
    let comment = super::options::string("org-comment-string", "COMMENT");
    let new = if hl.commented {
        let start = hl
            .priority_range
            .as_ref()
            .map(|r| r.end)
            .or(hl.todo_range.as_ref().map(|r| r.end))
            .unwrap_or(hl.level);
        let i = line[start..].find(&comment).map(|i| i + start).unwrap();
        let mut end = i + comment.len();
        while line[end..].starts_with(' ') && end < hl.title.start {
            end += 1;
        }
        format!("{}{}", &line[..i], &line[end..])
    } else {
        let at = hl
            .priority_range
            .as_ref()
            .map(|r| r.end)
            .or(hl.todo_range.as_ref().map(|r| r.end))
            .unwrap_or(hl.level);
        let rest = line[at..].trim_start_matches(' ');
        if rest.is_empty() {
            format!("{} {comment}", &line[..at])
        } else {
            format!("{} {comment} {rest}", &line[..at])
        }
    };
    super::set_line(ed, h, &new);
    if super::options::bool("org-auto-align-tags", true) {
        tags::align(ed, h);
    }
    Ok(())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-todo" => todo(ed, arg),
        "org-todo-yesterday" => {
            let saved = super::now();
            super::set_now(Some(saved - 86400));
            let r = todo(ed, arg);
            super::set_now(None);
            r
        }
        "org-add-log-setup" => {
            // A reschedule/redeadline note requested by the time module.
            if let Some(r) = super::time::take_log_request() {
                let how = if r.how == "note" {
                    How::Note
                } else {
                    How::Time
                };
                let note = Note {
                    purpose: r.purpose.into(),
                    state: r.state,
                    prev: Some(r.previous),
                    how,
                    extra: None,
                    time: super::now(),
                };
                let h = r.heading;
                add_log(ed, h, note);
            }
            Ok(())
        }
        "org-add-note" => {
            let h = fold::back_to_heading(ed, ed.cur.line)
                .ok_or_else(|| "Before first headline".to_owned());
            h.map(|h| {
                add_log(
                    ed,
                    h,
                    Note {
                        purpose: "note".into(),
                        state: None,
                        prev: None,
                        how: How::Note,
                        extra: None,
                        time: super::now(),
                    },
                )
            })
        }
        "org-priority" => match arg {
            Prefix::U(1) => priority_show(ed),
            _ => {
                let st = super::settings(ed);
                let (high, low, _) = st.priorities;
                let msg = format!(
                    "Priority {}-{}, SPC to remove: ",
                    props::priority_string(high),
                    props::priority_string(low)
                );
                if high < 65 && low >= 10 {
                    super::read(ed, &msg, "", |ed, s| {
                        let v = if s == " " {
                            None
                        } else {
                            s.trim().parse().ok()
                        };
                        if v.is_none() && s != " " {
                            ed.set_err("Priority must be a number");
                            return;
                        }
                        ed.undo.begin(ed.cur.pos());
                        if let Err(e) = set_priority(ed, v) {
                            ed.set_err(e);
                        }
                        ed.undo.end(ed.cur.pos());
                    });
                } else {
                    let mut entries: Vec<(String, String)> = (high..=low)
                        .map(|p| {
                            (
                                props::priority_string(p).to_lowercase(),
                                props::priority_string(p),
                            )
                        })
                        .collect();
                    entries.push((" ".into(), "remove".into()));
                    super::menu(ed, &msg, entries, |ed, k| {
                        let v = if k == " " {
                            None
                        } else {
                            syntax::priority_value(&k.to_uppercase())
                        };
                        ed.undo.begin(ed.cur.pos());
                        if let Err(e) = set_priority(ed, v) {
                            ed.set_err(e);
                        }
                        ed.undo.end(ed.cur.pos());
                    });
                }
                Ok(())
            }
        },
        "org-priority-up" => priority(ed, PriorityAction::Up),
        "org-priority-down" => priority(ed, PriorityAction::Down),
        "org-priority-show" => priority_show(ed),
        "org-update-statistics-cookies" => update_statistics_cookies(ed, !arg.is_none()),
        "org-update-parent-todo-statistics" => {
            if let Some(h) = fold::back_to_heading(ed, ed.cur.line) {
                update_parent_statistics(ed, h);
            }
            Ok(())
        }
        "org-sparse-tree" => {
            sparse_tree(ed, arg);
            Ok(())
        }
        "org-occur" => {
            super::read(ed, "Regexp: ", "", |ed, re| {
                if let Err(e) = occur(ed, &re, false) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        "org-show-todo-tree" => show_todo_tree(ed, arg),
        "org-toggle-ordered-property" => toggle_ordered(ed),
        "org-toggle-comment" => toggle_comment(ed),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::org;
    use super::*;

    #[test]
    fn todo_cycles_and_logs() {
        let e = org("* A", "<C-c><C-t>");
        assert_eq!(e.buf.text(), "* TODO A");
        let e = org("* A", "<C-c><C-t><C-c><C-t><C-c><C-t>");
        assert_eq!(e.buf.text(), "* A");
        super::super::set_now(Some(1_780_000_000));
        super::super::options::put("org-log-done", toml::Value::String("time".into()));
        let e = org("* TODO A", "<C-c><C-t>");
        let ts = super::super::timestamp(1_780_000_000, true, true);
        assert_eq!(e.buf.text(), format!("* DONE A\nCLOSED: {ts}"));
        let e = org(&format!("* DONE A\nCLOSED: {ts}"), "<C-c><C-t>");
        assert_eq!(e.buf.text(), "* A");
        super::super::options::put("org-log-done", toml::Value::Boolean(false));
        // Logging a state change with `!`.
        let e = org("#+TODO: TODO WAIT(w!) | DONE\n* TODO A", "j<C-c><C-t>w");
        assert_eq!(
            e.buf.text(),
            format!(
                "#+TODO: TODO WAIT(w!) | DONE\n* WAIT A\n- State \"WAIT\"       from \"TODO\"       {ts}"
            )
        );
        // Into a drawer.
        super::super::options::put("org-log-into-drawer", toml::Value::Boolean(true));
        let e = org("#+TODO: TODO WAIT(w!) | DONE\n* TODO A", "j<C-c><C-t>w");
        assert_eq!(e.buf.line(2), ":LOGBOOK:");
        assert!(e.buf.line(3).starts_with("- State \"WAIT\""));
        assert_eq!(e.buf.line(4), ":END:");
        super::super::options::put("org-log-into-drawer", toml::Value::Boolean(false));
        super::super::set_now(None);
    }

    #[test]
    fn sets_and_shift_states() {
        let e = org(
            "#+TODO: A B | C\n#+TODO: X | Y\n* A t",
            "jj<Space>u<Space>u<C-c><C-t>"
                .replace("<Space>", " ")
                .as_str(),
        );
        assert_eq!(e.buf.line(2), "* X t");
        let e = org("* TODO a", "3<C-c><C-t>");
        assert!(e.msg.is_some());
    }

    #[test]
    fn statistics_cookies_update() {
        let e = org("* P [/]\n** TODO a\n** TODO b", "jj<C-c><C-t>");
        assert_eq!(e.buf.line(0), "* P [1/2]");
        let e = org("* P [%]\n** TODO a\n** DONE b", "j<C-c><C-t>");
        assert_eq!(e.buf.line(0), "* P [100%]");
    }

    #[test]
    fn priorities() {
        let e = org("* TODO A", "<C-c>,b");
        assert_eq!(e.buf.text(), "* TODO [#B] A");
        let e = org("* TODO [#B] A", "<S-Up>");
        assert_eq!(e.buf.text(), "* TODO [#A] A");
        let e = org("* TODO [#A] A", "<C-c>, ");
        assert_eq!(e.buf.text(), "* TODO A");
        let e = org("* A", "<S-Down>");
        assert_eq!(e.buf.text(), "* [#B] A");
    }

    #[test]
    fn escapes() {
        assert_eq!(
            replace_escapes(
                "State %-12s from %-12S",
                &[('s', "\"A\"".into()), ('S', "\"B\"".into())]
            ),
            "State \"A\"          from \"B\"         "
        );
        assert_eq!(percent_cookie(1, 300), "[1%]");
    }
}

#[cfg(test)]
mod repeat_tests {
    use super::super::tests::org;

    #[test]
    fn repeating_task_returns_to_todo_and_logs() {
        super::super::set_now(Some(1_780_000_000));
        let e = org("* TODO A\nSCHEDULED: <2026-10-04 Sun +1w>", "<C-c><C-t>");
        let ts = super::super::timestamp(1_780_000_000, true, true);
        assert_eq!(e.buf.line(0), "* TODO A");
        assert_eq!(e.buf.line(1), "SCHEDULED: <2026-10-11 Sun +1w>");
        assert_eq!(e.buf.line(2), ":PROPERTIES:");
        assert_eq!(e.buf.line(3), format!(":LAST_REPEAT: {ts}"));
        assert_eq!(
            e.buf.line(5),
            format!("- State \"DONE\"       from \"TODO\"       {ts}")
        );
        super::super::set_now(None);
    }
}
