//! The agenda entry finders (org-agenda-get-day-entries and its
//! org-agenda-get-* helpers, org-scan-tags for the agenda, the search
//! view matcher), working on an agenda file's lines ([`Src`]).

use super::format::{self, In, Prefix};
use super::{Dotime, Face, Item, Loc, opt, opt_bool, opt_int, opt_string, opt_sym};
use crate::org::props::{Doc, Inherit};
use crate::org::sexp::Sexp;
use crate::org::syntax::{self, Settings};
use crate::org::time::civil::Tm;
use crate::org::time::{self, stamp};
use std::path::PathBuf;

/// An agenda file: its lines, settings and outline index.
pub struct Src {
    pub path: PathBuf,
    pub lines: Vec<String>,
    pub st: Settings,
    /// The heading at or before each line.
    head_of: Vec<Option<usize>>,
    /// Lines inside a block (`#+begin_x` ... `#+end_x`), delimiters included.
    in_block: Vec<bool>,
    /// The part searched: lines `[begin, end)` (an agenda restriction).
    pub range: (usize, usize),
}

impl Src {
    pub fn new(path: PathBuf, text: &str, range: Option<(usize, usize)>) -> Src {
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let dir = path.parent().map(std::path::Path::to_path_buf);
        let st = crate::org::settings_of(text, dir.as_deref());
        let mut head_of = Vec::with_capacity(lines.len());
        let mut in_block = Vec::with_capacity(lines.len());
        let mut cur = None;
        let mut block: Option<String> = None;
        for (i, l) in lines.iter().enumerate() {
            if syntax::level(l).is_some() {
                cur = Some(i);
                block = None;
            }
            head_of.push(cur);
            let lower = l.trim_start().to_ascii_lowercase();
            if let Some(end) = &block {
                in_block.push(true);
                if lower.starts_with(end.as_str()) {
                    block = None;
                }
            } else if let Some(rest) = lower.strip_prefix("#+begin_") {
                let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
                block = Some(format!("#+end_{name}"));
                in_block.push(true);
            } else {
                in_block.push(false);
            }
        }
        let n = lines.len();
        let range = range.map_or((0, n), |(a, b)| (a.min(n), b.min(n)));
        Src {
            path,
            lines,
            st,
            head_of,
            in_block,
            range,
        }
    }

    pub fn doc(&self) -> Doc<'_, [String]> {
        Doc::new(&self.lines, &self.st, Some(&self.path))
    }

    pub fn loc(&self, l: usize) -> Loc {
        Loc {
            path: self.path.clone(),
            line: l,
            text: self.lines[l].clone(),
            col: 0,
        }
    }

    pub fn heading_of(&self, l: usize) -> Option<usize> {
        self.head_of.get(l).copied().flatten()
    }

    pub fn level(&self, h: usize) -> usize {
        syntax::level(&self.lines[h]).unwrap_or(0)
    }

    pub fn subtree_end(&self, h: usize) -> usize {
        syntax::subtree_end(&self.lines, h)
    }

    pub fn entry_end(&self, h: usize) -> usize {
        syntax::entry_end(&self.lines, h)
    }

    pub fn headline(&self, h: usize) -> Option<syntax::Headline> {
        syntax::headline(&self.lines[h], &self.st)
    }

    pub fn todo(&self, h: usize) -> Option<String> {
        self.headline(h).and_then(|x| x.todo)
    }

    /// The heading text after the stars (`\*+[ \t]+\(.*\)`).
    pub fn head_text(&self, h: usize) -> String {
        let l = &self.lines[h];
        l.trim_start_matches('*')
            .trim_start_matches([' ', '\t'])
            .to_owned()
    }

    /// org-get-tags with each tag's inheritance flag.
    pub fn tags(&self, h: usize, inherited: bool) -> Vec<(String, bool)> {
        let doc = self.doc();
        let local = doc.local_tags(h);
        if !inherited {
            return local.into_iter().map(|t| (t, false)).collect();
        }
        let all = doc.tags(Some(h), false);
        let n_inh = all.len().saturating_sub(local.len());
        all.into_iter()
            .enumerate()
            .map(|(i, t)| {
                let inh = i < n_inh && !local.contains(&t);
                (t, inh)
            })
            .collect()
    }

    pub fn category(&self, l: usize) -> String {
        self.doc().category(self.heading_of(l))
    }

    pub fn effort(&self, h: usize) -> Option<String> {
        let p = opt_string("org-effort-property", "Effort");
        self.doc().get(Some(h), &p, Inherit::No, false)
    }

    pub fn prop(&self, h: usize, p: &str, inherit: Inherit) -> Option<String> {
        self.doc().get(Some(h), p, inherit, false)
    }

    /// The planning line of heading `h`.
    pub fn planning(&self, h: usize) -> Option<usize> {
        let p = h + 1;
        (p < self.lines.len() && time::is_planning_line(&self.lines[p])).then_some(p)
    }

    /// `SCHEDULED`/`DEADLINE`/`CLOSED` of heading `h`, with brackets.
    pub fn planning_get(&self, h: usize, kind: time::Planning) -> Option<String> {
        let l = self.planning(h)?;
        let text = &self.lines[l];
        let c = kind.time_re().captures(text)?;
        let g = c.get(1)?;
        Some(text[g.start() - 1..g.end() + 1].to_owned())
    }

    /// The outline path of `h` (titles of its ancestors and itself).
    pub fn outline_path(&self, h: usize) -> Vec<String> {
        let mut v = vec![];
        let mut p = syntax::parent(&self.lines, h);
        while let Some(x) = p {
            v.push(self.title(x));
            p = syntax::parent(&self.lines, x);
        }
        v.reverse();
        v
    }

    /// org-find-top-headline: the topmost ancestor's title, without cookies.
    pub fn top_headline(&self, h: usize) -> String {
        let mut top = h;
        while let Some(p) = syntax::parent(&self.lines, top) {
            top = p;
        }
        re!(r"^\[[0-9]+/[0-9]+\] *|^\[%[0-9]+\] *")
            .replace(&self.title(top), "")
            .into_owned()
    }

    pub fn title(&self, h: usize) -> String {
        self.headline(h)
            .map(|x| x.title(&self.lines[h]).to_owned())
            .unwrap_or_default()
    }

    pub fn priority(&self, line: &str) -> i64 {
        let (_, lowest, default) = self.st.priorities;
        format::priority_of(line, lowest, default)
    }

    fn kwds(&self) -> (Vec<String>, Vec<String>) {
        (
            self.st.todo_names().into_iter().map(String::from).collect(),
            self.st.done_names().into_iter().map(String::from).collect(),
        )
    }

    pub fn is_done(&self, kw: Option<&str>) -> bool {
        kw.is_some_and(|k| self.st.is_done(k))
    }
}

// ---- context ----

/// What a finder needs besides the file: the day, today, the prefix
/// format and the agenda modes.
pub struct Ctx {
    pub today: i64,
    pub prefix: Prefix,
    pub strategy: Vec<String>,
    /// org-agenda-archives-mode is on (archived trees are included).
    pub archives: bool,
    pub include_inactive: bool,
    /// org-agenda-show-log-scoped: the log items (closed clock state).
    pub log_items: Vec<String>,
    pub habits: bool,
    pub fmt: &'static str,
}

impl Ctx {
    pub fn new(fmt: &'static str, today: i64) -> Ctx {
        Ctx {
            today,
            prefix: format::compile(fmt),
            strategy: format::strategy(fmt),
            archives: false,
            include_inactive: opt_bool("org-agenda-include-inactive-timestamps", false),
            log_items: vec!["closed".into(), "clock".into()],
            habits: super::habit::loaded(),
            fmt,
        }
    }

    fn inherited_tags(&self, kind: &str) -> bool {
        match opt("org-agenda-show-inherited-tags") {
            Some(Sexp::Sym(s)) if s == "always" => true,
            Some(v @ Sexp::List(_)) => v
                .list()
                .unwrap_or(&[])
                .iter()
                .any(|x| x.sym() == Some(kind)),
            Some(v) if !v.truthy() => use_inheritance_for(kind),
            _ => use_inheritance_for(kind),
        }
    }
}

/// org-agenda-use-tag-inheritance for an agenda type.
fn use_inheritance_for(kind: &str) -> bool {
    match opt("org-agenda-use-tag-inheritance") {
        None | Some(Sexp::T) => true,
        Some(v) => v
            .list()
            .unwrap_or(&[])
            .iter()
            .any(|x| x.sym() == Some(kind)),
    }
}

// ---- skipping ----

/// org-agenda-skip: Some(line to continue from) when line `l` is skipped.
pub struct Skipper {
    until: usize,
    fun: Option<Sexp>,
    global: Option<Sexp>,
}

impl Default for Skipper {
    fn default() -> Skipper {
        Skipper::new()
    }
}

impl Skipper {
    pub fn new() -> Skipper {
        Skipper {
            until: 0,
            fun: opt("org-agenda-skip-function").filter(Sexp::truthy),
            global: opt("org-agenda-skip-function-global").filter(Sexp::truthy),
        }
    }

    /// True when the match at line `l` is skipped (and moves on).
    pub fn skip(&mut self, src: &Src, ctx: &Ctx, l: usize) -> bool {
        if l < self.until {
            return true;
        }
        match skip_at(src, ctx, l, self.fun.as_ref(), self.global.as_ref()) {
            Some(to) => {
                self.until = to.max(l);
                true
            }
            None => false,
        }
    }

    /// Continue after line `l` only from `to` (org-end-of-subtree).
    pub fn jump(&mut self, to: usize) {
        self.until = self.until.max(to);
    }
}

fn is_comment_line(l: &str) -> bool {
    let t = l.trim_start();
    t == "#" || t.starts_with("# ")
}

/// org-agenda-skip at line `l`: Some(position) to skip to (`l` itself
/// when skipping without moving).
fn skip_at(
    src: &Src,
    ctx: &Ctx,
    l: usize,
    fun: Option<&Sexp>,
    global: Option<&Sexp>,
) -> Option<usize> {
    if is_comment_line(&src.lines[l]) {
        return Some(l);
    }
    let h = src.heading_of(l);
    if let Some(h) = h {
        // Archived and commented trees (the heading or an ancestor).
        let archive_tag = opt_string("org-archive-tag", "ARCHIVE");
        let mut cur = Some(h);
        while let Some(x) = cur {
            if let Some(hl) = src.headline(x) {
                if opt_bool("org-agenda-skip-archived-trees", true)
                    && !ctx.archives
                    && hl.tags.contains(&archive_tag)
                {
                    return Some(src.subtree_end(x));
                }
                if opt_bool("org-agenda-skip-comment-trees", true) && hl.commented {
                    return Some(src.subtree_end(x));
                }
            }
            cur = syntax::parent(&src.lines, x);
        }
        for f in [global, fun].into_iter().flatten() {
            if let Some(to) = eval_skip(src, h, f) {
                return Some(to);
            }
        }
    }
    if opt_bool("org-agenda-skip-archived-trees", true)
        && !ctx.archives
        && src
            .st
            .file_tags
            .contains(&opt_string("org-archive-tag", "ARCHIVE"))
    {
        return Some(src.lines.len());
    }
    if src.in_block[l] {
        return Some(l);
    }
    None
}

/// Evaluate an org-agenda-skip-function form at heading `h`.
fn eval_skip(src: &Src, h: usize, f: &Sexp) -> Option<usize> {
    let f = match f {
        Sexp::List(v) if v.len() == 2 && v[0].sym() == Some("quote") => &v[1],
        f => f,
    };
    let l = f.list()?;
    let head = l.first()?.sym()?;
    match head {
        "lambda" => l.iter().skip(2).find_map(|x| eval_skip(src, h, x)),
        "org-agenda-skip-entry-if" | "org-agenda-skip-subtree-if" => {
            let conds: Vec<Sexp> = l[1..].iter().map(super::eval).collect();
            skip_if(src, h, head == "org-agenda-skip-subtree-if", &conds)
        }
        "or" => l[1..].iter().find_map(|x| eval_skip(src, h, x)),
        _ => None,
    }
}

/// org-agenda-skip-if.
pub fn skip_if(src: &Src, h: usize, subtree: bool, conds: &[Sexp]) -> Option<usize> {
    let end = if subtree {
        src.subtree_end(h)
    } else {
        src.entry_end(h)
    };
    let planning_end = if subtree {
        end
    } else {
        (h + 2).min(end.max(h + 1))
    };
    let text = |a: usize, b: usize| src.lines[a..b.min(src.lines.len())].join("\n");
    let has = |c: &str| conds.iter().any(|x| x.str() == Some(c));
    let arg = |c: &str| {
        conds
            .iter()
            .position(|x| x.str() == Some(c))
            .and_then(|i| conds.get(i + 1))
    };
    let body = text(h, end);
    let plan = text(h, planning_end);
    let sched = time::Planning::Scheduled.time_re().is_match(&plan);
    let dl = time::Planning::Deadline.time_re().is_match(&plan);
    let ts = re!(r"<[0-9]{4}-[0-9]{2}-[0-9]{2}[^>\n]*>").is_match(&body);
    let re_hit = |c: &str| {
        arg(c)
            .and_then(Sexp::str)
            .and_then(|r| crate::org::re::compile(r, false).ok())
            .map(|r| r.is_match(&body))
    };
    let hit = (has("scheduled") && sched)
        || (has("notscheduled") && !sched)
        || (has("deadline") && dl)
        || (has("notdeadline") && !dl)
        || (has("timestamp") && ts)
        || (has("nottimestamp") && !ts)
        || re_hit("regexp") == Some(true)
        || re_hit("notregexp") == Some(false)
        || skip_if_todo(src, h, end, conds);
    hit.then_some(end)
}

fn skip_if_todo(src: &Src, h: usize, end: usize, conds: &[Sexp]) -> bool {
    let Some(i) = conds.iter().position(|x| {
        matches!(
            x.str(),
            Some("nottodo" | "todo-unblocked" | "nottodo-unblocked" | "todo")
        )
    }) else {
        return false;
    };
    let kind = conds[i].str().unwrap_or("");
    let words: Vec<String> = match conds.get(i + 1) {
        Some(v) if v.str() == Some("todo") => src
            .st
            .not_done_names()
            .into_iter()
            .map(String::from)
            .collect(),
        Some(v) if v.str() == Some("done") => {
            src.st.done_names().into_iter().map(String::from).collect()
        }
        Some(v) if v.str() == Some("any") => {
            src.st.todo_names().into_iter().map(String::from).collect()
        }
        Some(v) => {
            let l: Vec<String> = v
                .list()
                .unwrap_or(&[])
                .iter()
                .filter_map(|x| x.str().map(str::to_owned))
                .collect();
            if l.iter().any(|x| x == "*") {
                src.st.todo_names().into_iter().map(String::from).collect()
            } else {
                l
            }
        }
        None => vec![],
    };
    let hits: Vec<usize> = (h..end)
        .filter(|&l| {
            syntax::headline(&src.lines[l], &src.st)
                .and_then(|x| x.todo)
                .is_some_and(|t| words.contains(&t))
        })
        .collect();
    let blocked = |l: usize| crate::org::todo::blocked(&src.doc(), l);
    match kind {
        "todo" => !hits.is_empty(),
        "nottodo" => hits.is_empty(),
        "todo-unblocked" => hits.iter().any(|&l| blocked(l)),
        _ => !hits.iter().any(|&l| blocked(l)),
    }
}

// ---- helpers ----

fn abs_of(s: &str) -> Option<i64> {
    stamp::time_string_to_absolute(s, None, "").ok()
}

fn abs_near(s: &str, day: i64, prefer: &str) -> Option<i64> {
    stamp::time_string_to_absolute(s, Some(day), prefer).ok()
}

/// org-agenda-entry-get-agenda-timestamp: (ts-date, type suffix).
fn agenda_timestamp(src: &Src, ctx: &Ctx, h: usize) -> (Option<i64>, &'static str) {
    let has = |a: &str, b: &str| ctx.strategy.iter().any(|s| s == a || s == b);
    let get = |p: &str| src.prop(h, p, Inherit::No);
    let (ts, ty) = if has("scheduled-up", "scheduled-down") {
        (get("SCHEDULED"), " scheduled")
    } else if has("deadline-up", "deadline-down") {
        (get("DEADLINE"), " deadline")
    } else if has("ts-up", "ts-down") {
        (get("TIMESTAMP"), " timestamp")
    } else if has("tsia-up", "tsia-down") {
        (get("TIMESTAMP_IA"), " timestamp_ia")
    } else if has("timestamp-up", "timestamp-down") {
        (
            get("SCHEDULED")
                .or_else(|| get("DEADLINE"))
                .or_else(|| get("TIMESTAMP"))
                .or_else(|| get("TIMESTAMP_IA")),
            "",
        )
    } else {
        (None, "")
    };
    (ts.and_then(|t| abs_of(&t)), ty)
}

/// The common part of every item: markers, keywords, category, effort.
fn base(src: &Src, ctx: &Ctx, it: &mut Item, h: Option<usize>, marker: usize) {
    it.marker = Some(src.loc(marker));
    it.hd = h.map(|h| src.loc(h));
    let (t, d) = src.kwds();
    it.todo_kwds = t;
    it.done_kwds = d;
    it.fmt = ctx.fmt;
    if let Some(h) = h {
        it.top = src.top_headline(h);
        it.clocking = (h + 1..src.entry_end(h)).any(|l| {
            let t = src.lines[l].trim_start();
            t.starts_with("CLOCK:") && !t.contains("]--")
        });
        it.todo_state = src.todo(h);
        it.blocked = it.todo_state.is_some()
            && opt_bool("org-enforce-todo-dependencies", false)
            && opt("org-agenda-dim-blocked-tasks").is_none_or(|v| v.truthy())
            && crate::org::todo::blocked(&src.doc(), h);
    }
}

/// org-agenda-format-item with this file's values.
#[allow(clippy::too_many_arguments)]
fn fmt_item<'r>(
    src: &Src,
    ctx: &Ctx,
    h: usize,
    extra: Option<String>,
    txt: String,
    kind: &str,
    dotime: Dotime,
    f: impl FnOnce(&mut In<'r>),
) -> Item {
    let effort = src.effort(h);
    let mut i = In {
        extra,
        txt: txt.clone(),
        level: Some(" ".repeat(src.level(h))),
        category: Some(src.category(h)),
        tags: src.tags(h, ctx.inherited_tags(kind)),
        dotime,
        effort: effort.clone(),
        outline: if ctx.prefix.has_breadcrumbs {
            src.outline_path(h)
        } else {
            vec![]
        },
        ..In::default()
    };
    f(&mut i);
    let mut it = format::item(&ctx.prefix, i);
    it.head = txt;
    it.effort_minutes = effort
        .as_deref()
        .and_then(|e| time::duration::to_minutes(e, false).ok());
    it.priority = src.priority(&it.line);
    it
}

fn time_in(s: &str) -> Dotime {
    match re!(r" ([012]?[0-9]:[0-9][0-9])").find(s) {
        Some(m) => Dotime::Str(format!("{} ", &s[m.start() + 1..])),
        None => Dotime::Yes,
    }
}

// ---- the finders ----

/// org-agenda-get-todos: `select` is org-select-this-todo-keyword.
pub fn todos(src: &Src, ctx: &Ctx, select: Option<&str>) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let sublevels = opt_bool("org-agenda-todo-list-sublevels", true);
    let wanted = |kw: &str| match select {
        Some("*") => true,
        Some(s) => s.split('|').any(|x| x == kw),
        None => !src.st.is_done(kw),
    };
    for h in src.range.0..src.range.1 {
        let Some(hl) = syntax::headline(&src.lines[h], &src.st) else {
            continue;
        };
        let Some(kw) = hl.todo.clone() else { continue };
        if !wanted(&kw) || sk.skip(src, ctx, h) {
            continue;
        }
        let end = src.entry_end(h);
        if ignore_todo(src, h, end) {
            if !sublevels {
                sk.jump(src.subtree_end(h));
            }
            continue;
        }
        let start = hl.todo_range.as_ref().map_or(0, |r| r.start);
        let txt = src.lines[h][start..].trim().to_owned();
        let (ts_date, ty) = agenda_timestamp(src, ctx, h);
        let mut it = fmt_item(
            src,
            ctx,
            h,
            Some(String::new()),
            txt,
            "todo",
            Dotime::Yes,
            |_| {},
        );
        base(src, ctx, &mut it, Some(h), h);
        it.urgency = 1 + it.priority;
        it.ts_date = ts_date;
        it.kind = format!("todo{ty}");
        it.undone_face = None;
        it.done_face = Some(Face::Done);
        out.push(it);
        if !sublevels {
            sk.jump(src.subtree_end(h));
        }
    }
    out
}

fn ignore_custom(days: i64, n: i64) -> bool {
    if n >= 0 { days >= n } else { days <= n }
}

fn ignore_option(name: &str, ts: &str, far: impl Fn() -> bool) -> Option<bool> {
    let v = opt(name).filter(Sexp::truthy)?;
    let days = || stamp::timestamp_to_now(ts, false).unwrap_or(0);
    Some(match &v {
        Sexp::Int(n) => ignore_custom(days(), *n),
        v => match v.sym() {
            Some("future") => days() > 0,
            Some("past") => days() <= 0,
            Some("all") => true,
            Some("far") => far(),
            _ => !far() || name != "org-agenda-todo-ignore-deadlines",
        },
    })
}

/// org-agenda-check-for-timestamp-as-reason-to-ignore-todo-item.
pub fn ignore_todo(src: &Src, h: usize, end: usize) -> bool {
    let body = src.lines[h..end].join("\n");
    let ts_re = re!(r"<([0-9]{4}-[0-9]{2}-[0-9]{2}[^>\n]*)>");
    if opt_bool("org-agenda-todo-ignore-with-date", false) && ts_re.is_match(&body) {
        return true;
    }
    if let Some(c) = time::Planning::Scheduled.time_re().captures(&body)
        && ignore_option("org-agenda-todo-ignore-scheduled", &c[1], || true) == Some(true)
    {
        return true;
    }
    if let Some(c) = time::Planning::Deadline.time_re().captures(&body) {
        let ts = c[1].to_owned();
        let close = || deadline_close(&ts);
        let ignore = match opt("org-agenda-todo-ignore-deadlines").filter(Sexp::truthy) {
            None => false,
            Some(v) if v.sym() == Some("far") => !close(),
            Some(v) if v.sym() == Some("t") => close(),
            Some(_) => {
                ignore_option("org-agenda-todo-ignore-deadlines", &ts, || false) == Some(true)
            }
        };
        if ignore {
            return true;
        }
    }
    if opt("org-agenda-todo-ignore-timestamp").is_some_and(|v| v.truthy()) {
        let stripped = re!(r"\b(SCHEDULED|DEADLINE): *<[^>]+>").replace_all(&body, "");
        if let Some(c) = ts_re.captures(&stripped)
            && ignore_option("org-agenda-todo-ignore-timestamp", &c[1], || true) == Some(true)
        {
            return true;
        }
    }
    false
}

/// org-deadline-close-p for a deadline string.
fn deadline_close(ts: &str) -> bool {
    let days = stamp::timestamp_to_now(ts, false).unwrap_or(i64::MAX);
    days <= stamp::get_wdays(ts, false, false)
}

/// A timestamp occurrence on a line: (start, end) byte range.
fn timestamps_on(
    line: &str,
    ctx: &Ctx,
    date_str: &str,
) -> Vec<(usize, usize, Option<String>, Option<String>)> {
    let mut out = vec![];
    let open = if ctx.include_inactive { "[<[]" } else { "<" };
    let re = regex::Regex::new(&format!(
        r"{open}{}|(<[0-9]+-[0-9]+-[0-9]+[^>\n]+?\+[0-9]+[hdwmy]>)|(<%%(\([^>\n]+\))([^\n>]*)>)",
        regex::escape(date_str)
    ))
    .unwrap();
    for c in re.captures_iter(line) {
        let m = c.get(0).unwrap();
        out.push((
            m.start(),
            m.end(),
            c.get(1).map(|x| x.as_str().to_owned()),
            c.get(3).map(|x| x.as_str().to_owned()),
        ));
    }
    out
}

fn at_date_range(line: &str, start: usize) -> bool {
    stamp::tr_both()
        .find_iter(line)
        .any(|m| m.start() <= start && start < m.end())
}

fn is_clock_line(l: &str) -> bool {
    l.trim_start().starts_with("CLOCK:")
}

/// org-agenda-get-timestamps.
pub fn timestamps(src: &Src, ctx: &Ctx, date: i64, deadlines: &[Item]) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let date_str = crate::org::time::civil::format("%Y-%m-%d", Tm::from_absolute(date));
    let today = ctx.today;
    let dl_heads: Vec<Loc> = deadlines.iter().filter_map(|d| d.hd.clone()).collect();
    let mut l = src.range.0;
    while l < src.range.1 {
        let line = &src.lines[l];
        let Some(h) = src.heading_of(l) else {
            l += 1;
            continue;
        };
        let mut next_line = l + 1;
        for (start, end, repeat, sexp) in timestamps_on(line, ctx, &date_str) {
            if at_date_range(line, start)
                || time::is_planning_line(line)
                || (ctx.include_inactive && is_clock_line(line))
                || l == h && start == 0
            {
                continue;
            }
            if sk.skip(src, ctx, l) {
                break;
            }
            let timestamp = if repeat.is_some() || sexp.is_some() {
                line[start..end].to_owned()
            } else {
                stamp::ts_both()
                    .find_at(line, start)
                    .filter(|m| m.start() == start)
                    .map_or_else(|| line[start..end].to_owned(), |m| m.as_str().to_owned())
            };
            let todo = src.todo(h);
            let done = src.is_done(todo.as_deref());
            if done && opt_bool("org-agenda-skip-timestamp-if-done", false) {
                continue;
            }
            if let Some(sx) = &sexp
                && time::diary::entry(sx, "", Tm::from_absolute(date)).is_none()
            {
                continue;
            }
            if let Some(rep) = &repeat {
                let prefer_last = prefer_last_repeat(todo.as_deref());
                let base = if date > today || prefer_last {
                    today
                } else {
                    date
                };
                let Some(past) = abs_near(rep, base, "past") else {
                    continue;
                };
                let future = if date <= today || !show_future_repeats() {
                    Some(past)
                } else {
                    let base = if opt_sym("org-agenda-show-future-repeats", "t") == "next" {
                        today + 1
                    } else {
                        date
                    };
                    abs_near(rep, base, "future")
                };
                if date != past && Some(date) != future {
                    continue;
                }
            }
            if opt_bool("org-agenda-skip-timestamp-if-deadline-is-shown", false)
                && dl_heads.iter().any(|d| d.line == h && d.path == src.path)
            {
                continue;
            }
            let inactive = line[start..].starts_with('[');
            let habit = ctx.habits && super::habit::is_habit(src, h);
            let extra = inactive.then(|| opt_string("org-agenda-inactive-leader", "["));
            let mut it = fmt_item(
                src,
                ctx,
                h,
                extra,
                src.head_text(h),
                "agenda",
                Dotime::Str(timestamp.clone()),
                |i| {
                    i.remove_re = Some(re!(r"<[0-9]{4}-[0-9]{2}-[0-9]{2}[^>\n]*>"));
                    i.habit = habit;
                },
            );
            base(src, ctx, &mut it, Some(h), l);
            if let Some(m) = &mut it.marker {
                m.col = start;
            }
            it.urgency = if habit {
                super::habit::parse(src, h, ctx.today)
                    .map_or(it.priority, |hb| super::habit::urgency(&hb, ctx.today))
            } else {
                it.priority
            };
            it.date = Some(date);
            it.ts_date = Some(match &repeat {
                Some(r) => abs_of(r).unwrap_or(date),
                None => date,
            });
            it.warntime = src.prop(h, "APPT_WARNTIME", Inherit::Selective);
            it.kind = "timestamp".into();
            it.face = Face::Event;
            out.push(it);
            if opt_bool("org-agenda-skip-additional-timestamps-same-entry", false) {
                next_line = src.entry_end(h);
                break;
            }
        }
        l = next_line.max(l + 1);
    }
    out
}

fn prefer_last_repeat(todo: Option<&str>) -> bool {
    match opt("org-agenda-prefer-last-repeat") {
        Some(Sexp::T) => true,
        Some(v) => v
            .list()
            .unwrap_or(&[])
            .iter()
            .any(|x| x.str().is_some() && x.str() == todo),
        None => false,
    }
}

fn show_future_repeats() -> bool {
    opt_bool("org-agenda-show-future-repeats", true)
}

/// The `(...)` form at the start of `s` (balanced parentheses).
fn sexp_at(s: &str) -> Option<&str> {
    let mut depth = 0;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in s.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// org-agenda-get-sexps: `%%(...)` diary sexp lines.
pub fn sexps(src: &Src, ctx: &Ctx, date: i64) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    for l in src.range.0..src.range.1 {
        let line = &src.lines[l];
        let Some(rest) = line.strip_prefix("&%%").or_else(|| line.strip_prefix("%%")) else {
            continue;
        };
        let Some(sx) = sexp_at(rest) else { continue };
        let entry = rest[sx.len()..].trim().to_owned();
        let Some(result) = time::diary::entry(sx, &entry, Tm::from_absolute(date)) else {
            continue;
        };
        if sk.skip(src, ctx, l) {
            continue;
        }
        let h = src.heading_of(l);
        let prefix_re = opt("org-agenda-diary-sexp-prefix")
            .and_then(|v| v.str().map(str::to_owned))
            .and_then(|r| crate::org::re::compile(&r, false).ok());
        for r in result {
            let (mut extra, mut r) = (None, r);
            if let Some(re) = &prefix_re
                && let Some(m) = re.find(&r)
            {
                extra = Some(m.as_str().to_owned());
                r.replace_range(m.range(), "");
            }
            let txt = if r.trim().is_empty() {
                "SEXP entry returned empty string".to_owned()
            } else {
                r
            };
            let mut it = match h {
                Some(h) => fmt_item(src, ctx, h, extra, txt, "agenda", Dotime::Yes, |_| {}),
                None => {
                    let mut it = format::item(
                        &ctx.prefix,
                        In {
                            extra,
                            txt: txt.clone(),
                            category: Some(src.category(l)),
                            dotime: Dotime::Yes,
                            ..In::default()
                        },
                    );
                    it.head = txt;
                    it.priority = src.priority(&it.line);
                    it
                }
            };
            base(src, ctx, &mut it, None, l);
            if let Some(h) = h {
                it.todo_state = src.todo(h);
                it.warntime = src.prop(h, "APPT_WARNTIME", Inherit::Selective);
            }
            it.date = Some(date);
            it.kind = "sexp".into();
            it.face = Face::SexpEntry;
            out.push(it);
        }
    }
    out
}

/// org-agenda-get-progress (log mode): closed, clocked and state items.
pub fn progress(src: &Src, ctx: &Ctx, date: i64) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let date_str = crate::org::time::civil::format("%Y-%m-%d", Tm::from_absolute(date));
    let kws: Vec<String> = src
        .st
        .todo_names()
        .iter()
        .map(|k| regex::escape(k))
        .collect();
    let mut parts = vec![];
    let has = |s: &str| ctx.log_items.iter().any(|x| x == s);
    if has("closed") {
        parts.push(r"\bCLOSED:".to_owned());
    }
    if has("clock") {
        parts.push(r"\bCLOCK:".to_owned());
    }
    if has("state") && !kws.is_empty() {
        parts.push(format!(r#"- +State "({})".*?"#, kws.join("|")));
    }
    if parts.is_empty() {
        return out;
    }
    let re = regex::Regex::new(&format!(
        r"({}) *\[{}",
        parts.join("|"),
        regex::escape(&date_str)
    ))
    .unwrap();
    let add_notes = opt_bool("org-agenda-log-mode-add-notes", true);
    for l in src.range.0..src.range.1 {
        let line = &src.lines[l];
        let Some(c) = re.captures(line) else { continue };
        if sk.skip(src, ctx, l) {
            continue;
        }
        let m0 = c.get(0).unwrap();
        let kw = c.get(1).unwrap().as_str();
        let closedp = kw == "CLOSED:";
        let statep = kw.starts_with('-');
        let state = statep.then(|| c.get(2).map(|x| x.as_str().to_owned()).unwrap_or_default());
        let mut timestr = line[m0.start()..].to_owned();
        let mut clocked = String::new();
        if let Some(m) = re!(r"\[[0-9]{4}-[0-9]{2}-[0-9]{2}[^\]\n]*\]").find(&timestr) {
            let rest = timestr[m.end()..].to_owned();
            let ts = timestr[..m.end()].to_owned();
            timestr = ts;
            if !closedp && !statep {
                if let Some(c2) =
                    re!(r"([0-9]{1,2}:[0-9]{2})\].*?([0-9]{1,2}:[0-9]{2})").captures(&rest)
                {
                    timestr = format!("{}-{}]", &timestr[..timestr.len() - 1], &c2[1]);
                    clocked = c2[2].to_owned();
                } else {
                    clocked = "-".into();
                }
            }
        }
        let next = src.lines.get(l + 1).map(String::as_str).unwrap_or("");
        let extra_note = if !add_notes {
            None
        } else if statep {
            line.trim_end()
                .ends_with("\\\\")
                .then(|| next.trim())
                .filter(|n| !n.is_empty() && !n.starts_with('-'))
                .map(str::to_owned)
        } else if !closedp {
            re!(r"^[ \t]*-[ \t]+([^-\n \t].*?)[ \t]*$")
                .captures(next)
                .map(|c| c[1].to_owned())
        } else {
            None
        };
        let Some(h) = src.heading_of(l) else { continue };
        let mut txt = src.head_text(h);
        if let Some(x) = &extra_note {
            if let Some(c) = re!(r"([ \t]+)(:[^ \n\t]*?:)[ \t]*$").captures(&txt) {
                let at = c.get(1).unwrap().start();
                txt = format!("{} - {x} {}", &txt[..at], &c[2]);
            } else {
                txt = format!("{txt} - {x}");
            }
        }
        let extra = if closedp {
            "Closed:    ".to_owned()
        } else if statep {
            format!("State:     ({})", state.unwrap_or_default())
        } else {
            format!("Clocked:   ({clocked})")
        };
        let mut it = fmt_item(
            src,
            ctx,
            h,
            Some(extra),
            txt,
            "todo",
            Dotime::Str(timestr),
            |i| i.no_headline_time = true,
        );
        base(src, ctx, &mut it, Some(h), l);
        if let Some(m) = &mut it.marker {
            m.col = m0.start();
        }
        it.face = Face::Done;
        it.priority = 100000;
        it.urgency = 100000;
        it.kind = if closedp {
            "closed"
        } else if statep {
            "state"
        } else {
            "clock"
        }
        .into();
        it.date = Some(date);
        it.undone_face = Some(Face::Warning);
        it.done_face = Some(Face::Done);
        out.push(it);
    }
    out
}

/// org-agenda-get-deadlines.
pub fn deadlines(src: &Src, ctx: &Ctx, date: i64, with_hour: bool) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let today = ctx.today;
    let todayp = date == today;
    for h in src.range.0..src.range.1 {
        if syntax::level(&src.lines[h]).is_none() {
            continue;
        }
        let Some(p) = src.planning(h) else { continue };
        let Some(c) = time::Planning::Deadline.time_re().captures(&src.lines[p]) else {
            continue;
        };
        let s = c[1].to_owned();
        let pcol = c.get(1).unwrap().start() - 1;
        if with_hour && !re!(r"[0-9]{1,2}:[0-9]{2}").is_match(&s) {
            continue;
        }
        if sk.skip(src, ctx, h) {
            continue;
        }
        let todo = src.todo(h);
        let done = src.is_done(todo.as_deref());
        let sexp = s.starts_with("%%");
        let deadline = if sexp {
            abs_near(&s, date, "")
        } else if prefer_last_repeat(todo.as_deref()) {
            abs_near(&s, today, "past")
        } else {
            abs_of(&s)
        };
        let Some(deadline) = deadline else { continue };
        let repeat = if sexp || date <= today || !show_future_repeats() {
            Some(deadline)
        } else {
            let base = if opt_sym("org-agenda-show-future-repeats", "t") == "next" {
                today + 1
            } else {
                date
            };
            abs_near(&s, base, "future")
        };
        let diff = deadline - date;
        let max_warning = match opt("org-agenda-skip-deadline-prewarning-if-scheduled") {
            Some(v) if v.truthy() => match src.planning_get(h, time::Planning::Scheduled) {
                None => i64::MAX,
                Some(sch) => match v {
                    Sexp::Int(n) => n,
                    v if v.sym() == Some("pre-scheduled") => (deadline
                        - abs_of(&sch).unwrap_or(deadline))
                    .min(opt_int("org-deadline-warning-days", 14)),
                    _ => 0,
                },
            },
            _ => i64::MAX,
        };
        let warning = max_warning.min(stamp::get_wdays(&s, false, false));
        if date == deadline || Some(date) == repeat {
        } else if !todayp {
            continue;
        } else if deadline > date {
            if diff > warning {
                continue;
            }
        } else if opt_int("org-deadline-past-days", 10000) < -diff {
            continue;
        }
        if done && (opt_bool("org-agenda-skip-deadline-if-done", false) || deadline != date) {
            continue;
        }
        let dotime = if date != deadline && Some(date) != repeat {
            Dotime::No
        } else {
            time_in(&s)
        };
        let leaders = leaders(
            "org-agenda-deadline-leaders",
            &["Deadline:  ", "In %3d d.: ", "%2d d. ago: "],
        );
        let extra = if todayp && deadline < today {
            cformat(&leaders[2], -diff)
        } else if todayp && deadline > today {
            cformat(&leaders[1], diff)
        } else {
            leaders[0].clone()
        };
        let fraction = 1.0 - diff as f64 / warning.max(1) as f64;
        let face = format::deadline_face(fraction);
        let upcoming = todayp && deadline > today;
        let mut it = fmt_item(
            src,
            ctx,
            h,
            Some(extra),
            src.head_text(h),
            "agenda",
            dotime,
            |_| {},
        );
        base(src, ctx, &mut it, Some(h), p);
        if let Some(m) = &mut it.marker {
            m.col = pcol;
        }
        it.warntime = src.prop(h, "APPT_WARNTIME", Inherit::Selective);
        it.ts_date = Some(deadline);
        it.urgency = if todayp { -diff } else { 0 } + it.priority;
        it.kind = if upcoming {
            "upcoming-deadline"
        } else {
            "deadline"
        }
        .into();
        it.date = Some(if upcoming { date } else { deadline });
        it.face = if done { Face::Done } else { face };
        it.undone_face = Some(face);
        it.done_face = Some(Face::Done);
        out.push(it);
    }
    out
}

fn leaders(name: &str, def: &[&str]) -> Vec<String> {
    let v: Vec<String> = opt(name)
        .and_then(|v| {
            v.list().map(|l| {
                l.iter()
                    .filter_map(|x| x.str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default();
    if v.len() >= def.len() {
        v
    } else {
        def.iter().map(|s| (*s).to_owned()).collect()
    }
}

/// `format` with one integer argument (`%d`, `%3d`, `%2d`).
fn cformat(f: &str, n: i64) -> String {
    let re = re!(r"%(-?)([0-9]*)d");
    re.replace(f, |c: &regex::Captures| {
        let w: usize = c[2].parse().unwrap_or(0);
        if &c[1] == "-" {
            format!("{n:<w$}")
        } else {
            format!("{n:>w$}")
        }
    })
    .into_owned()
}

/// org-agenda-get-scheduled.
pub fn scheduled(
    src: &Src,
    ctx: &Ctx,
    date: i64,
    deadlines: &[Item],
    with_hour: bool,
) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let today = ctx.today;
    let todayp = date == today;
    let dl_pos: Vec<usize> = deadlines
        .iter()
        .filter(|d| d.hd.as_ref().is_some_and(|x| x.path == src.path))
        .filter_map(|d| d.marker.as_ref().map(|m| m.line))
        .collect();
    for h in src.range.0..src.range.1 {
        if syntax::level(&src.lines[h]).is_none() {
            continue;
        }
        let Some(p) = src.planning(h) else { continue };
        let Some(c) = time::Planning::Scheduled.time_re().captures(&src.lines[p]) else {
            continue;
        };
        let s = c[1].to_owned();
        let pcol = c.get(1).unwrap().start() - 1;
        if with_hour && !re!(r"[0-9]{1,2}:[0-9]{2}").is_match(&s) {
            continue;
        }
        if sk.skip(src, ctx, h) {
            continue;
        }
        let todo = src.todo(h);
        let done = src.is_done(todo.as_deref());
        let sexp = s.starts_with("%%");
        let schedule = if sexp {
            abs_near(&s, date, "")
        } else if prefer_last_repeat(todo.as_deref()) {
            abs_near(&s, today, "past")
        } else {
            abs_of(&s)
        };
        let Some(schedule) = schedule else { continue };
        let repeat = if sexp || date <= today || !show_future_repeats() {
            Some(schedule)
        } else {
            let base = if opt_sym("org-agenda-show-future-repeats", "t") == "next" {
                today + 1
            } else {
                date
            };
            abs_near(&s, base, "future")
        };
        let diff = date - schedule;
        let past = schedule < today;
        let future = schedule > today;
        let habit = ctx.habits && super::habit::is_habit_local(src, h);
        let max_delay = match opt("org-agenda-skip-scheduled-delay-if-deadline") {
            Some(v) if v.truthy() => match src.planning_get(h, time::Planning::Deadline) {
                None => i64::MAX,
                Some(dl) => match v {
                    Sexp::Int(n) => -n,
                    v if v.sym() == Some("post-deadline") => (schedule
                        - abs_of(&dl).unwrap_or(schedule))
                    .min(opt_int("org-scheduled-delay-days", 0)),
                    _ => 0,
                },
            },
            _ => i64::MAX,
        };
        let delay =
            if re!(r"--[0-9]+[hdwmy]").is_match(&s) && abs_of(&s).is_some_and(|b| schedule > b) {
                0
            } else {
                max_delay.min(stamp::get_wdays(&s, true, false))
            };
        let past_days = if habit {
            opt("org-habit-scheduled-past-days")
                .and_then(|v| v.int())
                .unwrap_or_else(|| opt_int("org-scheduled-past-days", 10000))
        } else {
            opt_int("org-scheduled-past-days", 10000)
        };
        if !(todayp && habit && opt_bool("org-habit-show-all-today", false))
            && ((delay > 0 && diff < delay)
                || diff > past_days
                || schedule > date
                || (date != schedule && date != today && Some(date) != repeat))
        {
            continue;
        }
        if done && (opt_bool("org-agenda-skip-scheduled-if-done", false) || schedule != date) {
            continue;
        }
        if opt_bool("org-agenda-skip-scheduled-repeats-after-deadline", false)
            || opt_sym("org-agenda-skip-scheduled-if-deadline-is-shown", "nil")
                == "repeated-after-deadline"
        {
            let dl = src
                .planning_get(h, time::Planning::Deadline)
                .and_then(|d| abs_of(&d));
            if let Some(dl) = dl
                && (abs_of(&s).is_some_and(|b| b <= dl) || schedule != date)
                && date > dl
            {
                continue;
            }
        }
        let shown = dl_pos.contains(&p) && !habit;
        if shown {
            match opt_sym("org-agenda-skip-scheduled-if-deadline-is-shown", "nil").as_str() {
                "not-today" if past => continue,
                "t" => continue,
                _ => {}
            }
        }
        if habit
            && (done
                || !opt_bool("org-habit-show-habits", true)
                || (!todayp && opt_bool("org-habit-show-habits-only-for-today", true)))
        {
            continue;
        }
        let dotime = if !habit && date != schedule && Some(date) != repeat {
            Dotime::No
        } else {
            time_in(&s)
        };
        let leaders = leaders(
            "org-agenda-scheduled-leaders",
            &["Scheduled: ", "Sched.%2dx: "],
        );
        let extra = if todayp && past {
            cformat(&leaders[1], diff)
        } else {
            leaders[0].clone()
        };
        let mut it = fmt_item(
            src,
            ctx,
            h,
            Some(extra),
            src.head_text(h),
            "agenda",
            dotime,
            |i| {
                i.habit = habit;
            },
        );
        let face = if !habit && past {
            Face::ScheduledPreviously
        } else if habit && future {
            Face::Done
        } else if todayp {
            Face::ScheduledToday
        } else {
            Face::Scheduled
        };
        base(src, ctx, &mut it, Some(h), p);
        if let Some(m) = &mut it.marker {
            m.col = pcol;
        }
        it.undone_face = Some(face);
        it.done_face = Some(Face::Done);
        it.face = if done { Face::Done } else { face };
        it.kind = if past { "past-scheduled" } else { "scheduled" }.into();
        it.date = Some(if past { schedule } else { date });
        it.ts_date = Some(schedule);
        it.warntime = src.prop(h, "APPT_WARNTIME", Inherit::Selective);
        it.habit = if habit {
            super::habit::parse(src, h, today)
        } else {
            None
        };
        it.urgency = match &it.habit {
            Some(hb) => super::habit::urgency(hb, today),
            None => 99 + diff + it.priority,
        };
        out.push(it);
    }
    out
}

/// org-agenda-get-blocks: date ranges covering `date`.
pub fn blocks(src: &Src, ctx: &Ctx, date: i64) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let re = if ctx.include_inactive {
        stamp::tr_both()
    } else {
        re!(
            r"<([0-9]{4}-[0-9]{2}-[0-9]{2}(?: [^\n]*?)?)>--?-?<([0-9]{4}-[0-9]{2}-[0-9]{2}(?: [^\n]*?)?)>"
        )
    };
    for l in src.range.0..src.range.1 {
        let line = &src.lines[l];
        for c in re.captures_iter(line) {
            let (s1, s2) = (c[1].to_owned(), c[2].to_owned());
            let inactive = line[c.get(0).unwrap().start()..].starts_with('[');
            let (Some(start), Some(end)) = (abs_of(&s1), abs_of(&s2)) else {
                continue;
            };
            if !(start <= date && date <= end) {
                continue;
            }
            if sk.skip(src, ctx, l) {
                break;
            }
            let Some(h) = src.heading_of(l) else { continue };
            let todo = src.todo(h);
            if src.is_done(todo.as_deref()) && opt_bool("org-agenda-skip-timestamp-if-done", false)
            {
                continue;
            }
            let face = if start == end {
                Face::Event
            } else {
                Face::Daterange
            };
            let remove = opt_bool("org-agenda-remove-timeranges-from-blocks", false).then(|| {
                regex::Regex::new(&format!(
                    "<{}.*?>--<{}.*?>",
                    regex::escape(&s1),
                    regex::escape(&s2)
                ))
                .unwrap()
            });
            let leaders = leaders("org-agenda-timerange-leaders", &["", "(%d/%d): "]);
            let f = if start == end {
                &leaders[0]
            } else {
                &leaders[1]
            };
            let extra = format!(
                "{}{}",
                if inactive {
                    opt_string("org-agenda-inactive-leader", "[")
                } else {
                    String::new()
                },
                cformat(&cformat_first(f, date - start + 1), end - start + 1)
            );
            let dotime = if start == date && end == date {
                Dotime::Str(format!("<{s1}>--<{s2}>"))
            } else if start == date {
                Dotime::Str(format!("<{s1}>"))
            } else if end == date {
                Dotime::Str(format!("<{s2}>"))
            } else {
                Dotime::No
            };
            let mut it = fmt_item(
                src,
                ctx,
                h,
                Some(extra),
                src.head_text(h),
                "agenda",
                dotime,
                |i| {
                    i.remove_re = remove.as_ref();
                    i.no_default_duration = true;
                },
            );
            base(src, ctx, &mut it, Some(h), l);
            if let Some(m) = &mut it.marker {
                m.col = c.get(0).unwrap().start();
            }
            it.face = face;
            it.kind = "block".into();
            it.date = Some(date);
            it.urgency = it.priority;
            out.push(it);
        }
    }
    out
}

/// Replace the first `%d` only.
fn cformat_first(f: &str, n: i64) -> String {
    match re!(r"%(-?)([0-9]*)d").find(f) {
        Some(m) => format!(
            "{}{}{}",
            &f[..m.start()],
            cformat(m.as_str(), n),
            &f[m.end()..]
        ),
        None => f.to_owned(),
    }
}

/// org-agenda-get-day-entries for `types` (`:deadline`, `:scheduled`...).
pub fn day_entries(src: &Src, ctx: &Ctx, date: i64, types: &[String]) -> Vec<Item> {
    let mut args: Vec<String> = vec![];
    for t in types {
        if !args.contains(t) {
            args.push(t.clone());
        }
    }
    if args.iter().any(|a| a == ":scheduled") {
        args.retain(|a| a != ":scheduled*");
    }
    if let Some(i) = args.iter().position(|a| a == ":deadline") {
        args.remove(i);
        args.retain(|a| a != ":deadline*");
        args.insert(0, ":deadline".into());
    } else if let Some(i) = args.iter().position(|a| a == ":deadline*") {
        args.remove(i);
        args.insert(0, ":deadline*".into());
    }
    let mut out = vec![];
    let mut dls: Vec<Item> = vec![];
    for a in &args {
        match a.as_str() {
            ":todo" if date == ctx.today => out.extend(todos(src, ctx, None)),
            ":timestamp" => {
                out.extend(blocks(src, ctx, date));
                out.extend(timestamps(src, ctx, date, &dls));
            }
            ":sexp" => out.extend(sexps(src, ctx, date)),
            ":scheduled" => out.extend(scheduled(src, ctx, date, &dls, false)),
            ":scheduled*" => out.extend(scheduled(src, ctx, date, &dls, true)),
            ":closed" => out.extend(progress(src, ctx, date)),
            ":deadline" | ":deadline*" => {
                dls = deadlines(src, ctx, date, a == ":deadline*");
                out.extend(dls.iter().cloned());
            }
            _ => {}
        }
    }
    out
}

// ---- tags and search ----

/// org-scan-tags 'agenda: entries of `src` matching `m`.
pub fn tags_match(
    src: &Src,
    ctx: &Ctx,
    m: &crate::org::tags::Matcher,
    todo_only: bool,
) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let doc = src.doc();
    let sublevels = opt("org-tags-match-list-sublevels");
    let indented = sublevels.as_ref().and_then(Sexp::sym) == Some("indented");
    let sublevels = sublevels.is_none_or(|v| v.truthy());
    let honor = opt_bool("org-agenda-tags-todo-honor-ignore-options", false);
    let mut skip_to = 0;
    for h in src.range.0..src.range.1 {
        if h < skip_to {
            continue;
        }
        let Some(hl) = syntax::headline(&src.lines[h], &src.st) else {
            continue;
        };
        let tags = doc.tags(Some(h), false);
        let prop = |p: &str| doc.get(Some(h), p, Inherit::Selective, false);
        let not_done = |t: &str| !src.st.is_done(t);
        let info = crate::org::tags::EntryInfo {
            todo: hl.todo.as_deref(),
            tags: &tags,
            level: hl.level,
            prop: &prop,
            not_done: &not_done,
        };
        // org--matcher-tags-todo-only: not-done TODO entries.
        if todo_only && !hl.todo.as_ref().is_some_and(|t| !src.st.is_done(t)) {
            continue;
        }
        if !m.matches(&info) {
            continue;
        }
        if sk.skip(src, ctx, h) {
            continue;
        }
        if todo_only && honor && ignore_todo(src, h, src.entry_end(h)) {
            continue;
        }
        let (ts_date, ty) = agenda_timestamp(src, ctx, h);
        let title = format!(
            "{}{}",
            if indented {
                ".".repeat(hl.level.saturating_sub(1))
            } else {
                String::new()
            },
            heading_for_tags(src, h)
        );
        let mut it = fmt_item(
            src,
            ctx,
            h,
            Some(String::new()),
            title,
            "tags",
            Dotime::No,
            |i| {
                i.level = Some(" ".repeat(hl.level));
                i.tags = tags
                    .iter()
                    .map(|t| (t.clone(), !doc.local_tags(h).contains(t)))
                    .collect();
            },
        );
        base(src, ctx, &mut it, Some(h), h);
        it.urgency = it.priority;
        it.ts_date = ts_date;
        it.kind = format!("tagsmatch{ty}");
        it.done_face = Some(Face::Done);
        it.undone_face = Some(Face::Default);
        out.push(it);
        if !sublevels {
            skip_to = src.subtree_end(h);
        }
    }
    out
}

/// org-get-heading for the tags view: keyword, priority and title, no tags.
fn heading_for_tags(src: &Src, h: usize) -> String {
    let line = &src.lines[h];
    match src.headline(h) {
        Some(hl) => {
            let end = hl.tags_range.as_ref().map_or(line.len(), |r| r.start);
            line[..end]
                .trim_start_matches('*')
                .trim_start_matches([' ', '\t'])
                .trim_end()
                .to_owned()
        }
        None => line.clone(),
    }
}

/// A parsed search string (org-search-view).
#[derive(Clone, Debug, Default)]
pub struct Search {
    pub hdl_only: bool,
    pub todo_only: bool,
    pub boolean: bool,
    pub plus: Vec<String>,
    pub minus: Vec<String>,
}

/// Split `string` into the regexps org-search-view matches.
pub fn parse_search(string: &str, todo_only: bool) -> Search {
    let mut s = Search {
        todo_only,
        ..Search::default()
    };
    let mut words = string;
    if let Some(r) = words.strip_prefix('*') {
        s.hdl_only = true;
        words = r;
    }
    if let Some(r) = words.strip_prefix('!') {
        s.todo_only = true;
        words = r;
    }
    let mut full = opt_bool("org-agenda-search-view-force-full-words", false);
    if let Some(r) = words.strip_prefix(':') {
        full = true;
        words = r;
    }
    s.boolean = opt_bool("org-agenda-search-view-always-boolean", false)
        || words.starts_with(['-', '+', '{']);
    let raw: Vec<&str> = words.split_whitespace().collect();
    // Join `\ ` escapes and `{...}` regexps.
    let mut ws: Vec<String> = vec![];
    let mut i = 0;
    while i < raw.len() {
        let mut w = raw[i].to_owned();
        i += 1;
        while w.ends_with('\\') && i < raw.len() {
            w = format!("{} {}", &w[..w.len() - 1], raw[i]);
            i += 1;
        }
        if re!(r"^[-+]?\{").is_match(&w) && !w.ends_with('}') {
            while i < raw.len() && !raw[i].ends_with('}') {
                w = format!("{w} {}", raw[i]);
                i += 1;
            }
            if i < raw.len() {
                w = format!("{w} {}", raw[i]);
                i += 1;
            }
        }
        ws.push(w);
    }
    if s.boolean {
        let mut joined: Vec<String> = vec![];
        let mut it = ws.into_iter();
        while let Some(mut w) = it.next() {
            if w.starts_with('"')
                || (w.len() > 1 && w[1..].starts_with('"') && w.starts_with(['+', '-']))
            {
                while !w.ends_with('"') || w.len() == 1 || w.ends_with("+\"") || w.ends_with("-\"")
                {
                    match it.next() {
                        Some(n) => w = format!("{w} {n}"),
                        None => break,
                    }
                }
            }
            let w = re!(r#"^([-+]?)""#).replace(&w, "$1").into_owned();
            let w = w.strip_suffix('"').map(str::to_owned).unwrap_or(w);
            joined.push(w);
        }
        for w in joined {
            let (neg, w) = match w.strip_prefix('-') {
                Some(r) => (true, r.to_owned()),
                None => (false, w.strip_prefix('+').unwrap_or(&w).to_owned()),
            };
            let re = if w.len() >= 2 && w.starts_with('{') && w.ends_with('}') {
                w[1..w.len() - 1].to_owned()
            } else if full {
                format!(r"\<{}\>", elisp_quote(&w.to_lowercase()))
            } else {
                elisp_quote(&w.to_lowercase())
            };
            if neg {
                s.minus.push(re);
            } else {
                s.plus.push(re);
            }
        }
    } else {
        s.plus.push(
            ws.iter()
                .map(|w| elisp_quote(w))
                .collect::<Vec<_>>()
                .join(r"\s-+"),
        );
    }
    // Upstream pushes (newest first), then sorts longest first (stable).
    s.plus.reverse();
    s.minus.reverse();
    s.plus.sort_by_key(|r| std::cmp::Reverse(r.len()));
    s
}

/// regexp-quote (Emacs regexp syntax).
fn elisp_quote(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if "[*.\\?+^$".contains(c) {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// org-search-view's matches in `src`.
pub fn search(src: &Src, ctx: &Ctx, s: &Search) -> Vec<Item> {
    let mut out = vec![];
    let mut sk = Skipper::new();
    let compile = |r: &str| crate::org::re::compile(r, true).ok();
    let plus: Vec<regex::Regex> = s.plus.iter().filter_map(|r| compile(r)).collect();
    let minus: Vec<regex::Regex> = s.minus.iter().filter_map(|r| compile(r)).collect();
    let max_level = opt_int("org-agenda-search-view-max-outline-level", 0) as usize;
    let todo_re = |l: &str| {
        syntax::headline(l, &src.st)
            .and_then(|x| x.todo)
            .is_some_and(|t| !src.st.is_done(&t))
    };
    let mut h = src.range.0;
    while h < src.range.1 {
        let Some(lvl) = syntax::level(&src.lines[h]) else {
            h += 1;
            continue;
        };
        if max_level > 0 && lvl > max_level {
            h += 1;
            continue;
        }
        let mut end = h + 1;
        while end < src.range.1 {
            match syntax::level(&src.lines[end]) {
                Some(l) if max_level == 0 || l <= max_level => break,
                _ => end += 1,
            }
        }
        let text = if s.hdl_only {
            src.lines[h].clone()
        } else {
            src.lines[h..end].join("\n")
        };
        let ok = !minus.iter().any(|r| r.is_match(&text))
            && plus.iter().all(|r| r.is_match(&text))
            && (!s.todo_only || todo_re(&src.lines[h]));
        if ok && !sk.skip(src, ctx, h) {
            let txt = src.head_text(h);
            let mut it = fmt_item(
                src,
                ctx,
                h,
                Some(String::new()),
                txt,
                "todo",
                Dotime::Yes,
                |_| {},
            );
            base(src, ctx, &mut it, Some(h), h);
            it.priority = 1000;
            it.urgency = 1000;
            it.kind = "search".into();
            it.done_face = Some(Face::Done);
            out.push(it);
        }
        h = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_strings() {
        let s = parse_search("+foo -bar {b.z}", false);
        assert!(s.boolean);
        assert_eq!(s.plus, vec!["b.z".to_owned(), "foo".to_owned()]);
        assert_eq!(s.minus, vec!["bar".to_owned()]);
        let s = parse_search("*!hello world", false);
        assert!(s.hdl_only && s.todo_only && !s.boolean);
        assert_eq!(s.plus, vec![r"hello\s-+world".to_owned()]);
    }

    #[test]
    fn leaders_format() {
        assert_eq!(cformat("In %3d d.: ", 3), "In   3 d.: ");
        assert_eq!(cformat("%2d d. ago: ", 12), "12 d. ago: ");
        assert_eq!(cformat(&cformat_first("(%d/%d): ", 1), 3), "(1/3): ");
    }
}
