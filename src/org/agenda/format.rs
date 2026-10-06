//! Agenda presentation and sorting: org-compile-prefix-format,
//! org-agenda-format-item, org-get-time-of-day, the time grid,
//! org-entries-lessp, org-agenda-highlight-todo, tag alignment and the
//! date line format.

use super::{Dotime, Face, Item, opt, opt_bool, opt_int, opt_string, opt_sym};
use crate::org::sexp::Sexp;
use crate::org::time::civil::{self, Tm};

/// A field of the prefix format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Var {
    Category,
    Time,
    Level,
    Extra,
    Icon,
    Tag,
    Effort,
    Breadcrumbs,
    Eval,
}

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Lit(String),
    Var {
        var: Var,
        opt: bool,
        sep: String,
        left: bool,
        width: Option<usize>,
        prec: Option<usize>,
    },
}

/// A compiled prefix format (org-prefix-format-compiled).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Prefix {
    segs: Vec<Seg>,
    pub has_time: bool,
    pub has_tag: bool,
    pub has_effort: bool,
    pub has_breadcrumbs: bool,
    pub cat_len: Option<usize>,
    pub cat_max: Option<usize>,
}

/// org-compile-prefix-format for agenda type `key`.
pub fn compile(key: &str) -> Prefix {
    let s = match opt("org-agenda-prefix-format") {
        Some(Sexp::Str(s)) => s,
        Some(v) => v
            .list()
            .unwrap_or(&[])
            .iter()
            .find(|e| e.car().and_then(Sexp::sym) == Some(key))
            .and_then(|e| e.cdr().str().map(str::to_owned))
            .unwrap_or_else(|| default_prefix(key).to_owned()),
        None => default_prefix(key).to_owned(),
    };
    compile_str(&s)
}

fn default_prefix(key: &str) -> &'static str {
    match key {
        "agenda" => " %i %-12:c%?-12t% s",
        "todo" | "tags" | "search" => " %i %-12:c",
        _ => "  %-12:c%?-12t% s",
    }
}

pub fn compile_str(s: &str) -> Prefix {
    let re = re!(r"(?i)%(\?)?([-+]?[0-9.]*)([ .;,:!?=|/<>]?)([cltseibT]|\(.+?\))");
    let mut p = Prefix::default();
    let mut last = 0;
    for c in re.captures_iter(s) {
        let m = c.get(0).unwrap();
        if m.start() > last {
            p.segs.push(Seg::Lit(s[last..m.start()].to_owned()));
        }
        last = m.end();
        let var = match &c[4] {
            "c" => Var::Category,
            "t" => Var::Time,
            "l" => Var::Level,
            "s" => Var::Extra,
            "i" => Var::Icon,
            "T" => Var::Tag,
            "e" => Var::Effort,
            "b" => Var::Breadcrumbs,
            _ => Var::Eval,
        };
        match var {
            Var::Time => p.has_time = true,
            Var::Tag => p.has_tag = true,
            Var::Effort => p.has_effort = true,
            Var::Breadcrumbs => p.has_breadcrumbs = true,
            _ => {}
        }
        let spec = &c[2];
        let left = spec.starts_with('-');
        let digits = spec.trim_start_matches(['-', '+']);
        let (w, prec) = match digits.split_once('.') {
            Some((w, pr)) => (w, pr.parse().ok()),
            None => (digits, None),
        };
        let width = w.parse::<usize>().ok();
        if var == Var::Category {
            p.cat_len = Some(width.unwrap_or(0));
            p.cat_max = prec;
        }
        p.segs.push(Seg::Var {
            var,
            opt: c.get(1).is_some(),
            sep: c[3].to_owned(),
            left,
            width,
            prec,
        });
    }
    if last < s.len() {
        p.segs.push(Seg::Lit(s[last..].to_owned()));
    }
    p
}

/// Emacs `format "%-12.5s"`.
pub fn pad(s: &str, left: bool, width: Option<usize>, prec: Option<usize>) -> String {
    let s: String = match prec {
        Some(n) => s.chars().take(n).collect(),
        None => s.to_owned(),
    };
    let n = unicode_width::UnicodeWidthStr::width(s.as_str());
    match width {
        Some(w) if w > n => {
            if left {
                format!("{s}{}", " ".repeat(w - n))
            } else {
                format!("{}{s}", " ".repeat(w - n))
            }
        }
        _ => s,
    }
}

/// The values the prefix shows.
struct Vars<'a> {
    category: &'a str,
    extra_space: &'a str,
    time: &'a str,
    level: &'a str,
    extra: &'a str,
    tag: &'a str,
    effort: &'a str,
    breadcrumbs: &'a str,
}

impl Prefix {
    fn eval(&self, v: &Vars) -> String {
        let mut out = String::new();
        for s in &self.segs {
            match s {
                Seg::Lit(l) => out.push_str(l),
                Seg::Var {
                    var,
                    opt,
                    sep,
                    left,
                    width,
                    prec,
                } => {
                    let val = match var {
                        Var::Category => v.category,
                        Var::Time => v.time,
                        Var::Level => v.level,
                        Var::Extra => v.extra,
                        Var::Icon | Var::Eval => "",
                        Var::Tag => v.tag,
                        Var::Effort => v.effort,
                        Var::Breadcrumbs => v.breadcrumbs,
                    };
                    let es = if *var == Var::Category {
                        v.extra_space
                    } else {
                        ""
                    };
                    if *opt {
                        if !val.is_empty() {
                            out.push_str(&pad(&format!("{val}{sep}"), *left, *width, *prec));
                        }
                    } else if val.is_empty() {
                        out.push_str(&pad("", *left, *width, *prec));
                    } else {
                        out.push_str(&pad(&format!("{val}{sep}{es}"), *left, *width, *prec));
                    }
                }
            }
        }
        out
    }
}

// ---- time of day ----

/// org-get-time-of-day: military time (1330), or with `string` the
/// "13:30" form (`overtime`: "+1:30" past 24:00).
pub fn time_of_day(s: &str) -> Option<i64> {
    let (h, m) = tod(s)?;
    Some(h * 100 + m)
}

fn tod(s: &str) -> Option<(i64, i64)> {
    let c = re!(r"(?i)\b([012]?[0-9])(?::([0-5][0-9])(am|pm)?|(am|pm))\b").captures(s)?;
    let ampm = c.get(3).or(c.get(4)).map(|m| m.as_str().to_lowercase());
    let h: i64 = c[1].parse().ok()?;
    let hours = match ampm.as_deref() {
        None => h,
        Some(a) => {
            let am = a == "am";
            if h == 12 {
                if am { 0 } else { 12 }
            } else {
                h + if am { 0 } else { 12 }
            }
        }
    };
    let minutes = c.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
    Some((hours, minutes))
}

pub fn time_string(s: &str, overtime: bool) -> Option<String> {
    let (h, m) = tod(s)?;
    if overtime && (h > 24 || (h == 24 && m > 0)) {
        return Some(format!("+{}:{m:02}", h - 24));
    }
    Some(if opt_bool("org-agenda-time-leading-zero", false) {
        format!("{h:02}:{m:02}")
    } else {
        format!("{h}:{m:02}")
    })
}

fn minutes(s: &str) -> f64 {
    crate::org::time::duration::to_minutes(s, false).unwrap_or(0.0)
}

/// org-stamp-time-of-day-regexp on `s`: (whole match, start time, end time,
/// same-day range).
fn stamp_time_of_day(s: &str) -> Option<(String, String, Option<String>)> {
    let re = re!(
        r"<([0-9]{4}-[0-9]{2}-[0-9]{2} +\w+ +)([012][0-9]:[0-5][0-9])(-([012][0-9]:[0-5][0-9]))?[^\n\r>]*?>"
    );
    let c = re.captures(s)?;
    let whole = c.get(0).unwrap();
    let start = c[2].to_owned();
    if let Some(e) = c.get(4) {
        return Some((
            whole.as_str().to_owned(),
            start,
            Some(e.as_str().to_owned()),
        ));
    }
    // A same-day range <date time>--<date time>.
    let rest = &s[whole.end()..];
    let prefix = format!("--<{}", &c[1]);
    let prefix1 = format!("-<{}", &c[1]);
    for p in [prefix, prefix1] {
        if let Some(r) = rest.strip_prefix(&p)
            && let Some(t) = re!(r"^([012][0-9]:[0-5][0-9])>").captures(r)
        {
            let full = format!("{}{p}{}>", whole.as_str(), &t[1]);
            return Some((full, start, Some(t[1].to_owned())));
        }
    }
    Some((whole.as_str().to_owned(), start, None))
}

// ---- tags ----

/// org-tag-group-re on `txt`: (start of the spaces, the tag group).
fn tag_group(txt: &str) -> Option<(usize, String)> {
    let c = re!(r"[ \t]+(:([\w@#%:]+):)[ \t]*$").captures(txt)?;
    Some((c.get(0).unwrap().start(), c[1].to_owned()))
}

/// org-agenda-fix-displayed-tags.
fn fix_tags(
    txt: &str,
    tags: &[(String, bool)],
    add_inherited: bool,
    hide: Option<&regex::Regex>,
) -> String {
    if !add_inherited && hide.is_none() {
        return txt.to_owned();
    }
    let mut txt = match tag_group(txt) {
        Some((i, _)) => txt[..i].to_owned(),
        None => txt.to_owned(),
    };
    let tags: Vec<&(String, bool)> = tags
        .iter()
        .filter(|(t, inh)| !(hide.is_some_and(|r| r.is_match(t)) || (!add_inherited && *inh)))
        .collect();
    if !tags.is_empty() {
        let mut have_i = tags[0].1;
        let mut parts = vec![];
        for (t, i) in &tags {
            if have_i && !*i {
                have_i = false;
                parts.push(format!(":{t}"));
            } else {
                parts.push(t.clone());
            }
        }
        txt.push_str(" :");
        txt.push_str(&parts.join(":"));
        txt.push_str(if have_i { "::" } else { ":" });
    }
    txt
}

// ---- formatting an item ----

/// Inputs of org-agenda-format-item.
#[derive(Default)]
pub struct In<'a> {
    pub extra: Option<String>,
    pub txt: String,
    pub level: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<(String, bool)>,
    pub dotime: Dotime,
    pub remove_re: Option<&'a regex::Regex>,
    pub habit: bool,
    pub effort: Option<String>,
    /// The outline path for %b.
    pub outline: Vec<String>,
    /// Bound to nil by some finders.
    pub no_headline_time: bool,
    /// org-agenda-default-appointment-duration bound to nil.
    pub no_default_duration: bool,
}

/// org-agenda-format-item: the item with its prefix and properties.
pub fn item(p: &Prefix, i: In) -> Item {
    let width = super::width();
    let mut txt = i.txt.trim().to_owned();
    let show_inh = opt("org-agenda-show-inherited-tags").is_none_or(|v| v.truthy());
    let hide = match opt("org-agenda-hide-tags-regexp") {
        Some(Sexp::Str(s)) => crate::org::re::compile(&s, false).ok(),
        _ => None,
    };
    txt = fix_tags(&txt, &i.tags, show_inh, hide.as_ref());
    let category = i.category.unwrap_or_default();
    let full_category = category.clone();
    let effort = i.effort.clone().unwrap_or_default();
    let tag = i.tags.last().map(|t| t.0.clone()).unwrap_or_default();
    let grid = grid_spec();
    let extra = if i.habit {
        String::new()
    } else {
        i.extra.unwrap_or_default()
    };
    let search_head = opt_bool("org-agenda-search-headline-for-time", true) && !i.no_headline_time;
    let containing = match &i.dotime {
        Dotime::No => None,
        d => {
            let mut s = if let Dotime::Str(s) = d {
                s.clone()
            } else {
                String::new()
            };
            if search_head {
                s.push_str(&crate::org::time::stamp::ts_both().replace_all(&txt, ""));
            }
            Some(s)
        }
    };
    let tod = containing.as_deref().and_then(time_of_day);
    let (mut start, mut end): (Option<String>, Option<String>) = (None, None);
    let mut duration = None;
    if let (Some(sct), Some(_)) = (&containing, tod) {
        let found = stamp_time_of_day(sct).or_else(|| {
            crate::org::time::stamp::plain_time_of_day()
                .captures(sct)
                .map(|c| {
                    (
                        c[0].to_owned(),
                        c[1].to_owned(),
                        c.get(8).map(|m| m.as_str().to_owned()),
                    )
                })
        });
        if let Some((ts, s, e)) = found {
            start = Some(s);
            end = e;
            let rm = opt_sym("org-agenda-remove-times-when-in-prefix", "t");
            if p.has_time && rm != "nil" {
                let re = regex::Regex::new(&format!("{} *", regex::escape(&ts))).unwrap();
                if let Some(m) = re.find(&txt)
                    && !txt[m.end()..].starts_with(']')
                    && (rm != "beg" || m.start() == 0)
                {
                    txt.replace_range(m.range(), "");
                }
            }
        }
        start = start.and_then(|s| time_string(&s, false));
        end = end.and_then(|s| time_string(&s, false));
        if let (Some(s), None) = (&start, &end)
            && !i.no_default_duration
            && let Some(d) = opt("org-agenda-default-appointment-duration").and_then(|v| v.int())
        {
            let m = minutes(s) as i64 + d;
            end = Some(format!("{}:{:02}", m / 60, m % 60));
        }
        if let (Some(s), Some(e)) = (&start, &end) {
            duration = Some(minutes(e) - minutes(s));
        }
        start = start.map(|s| format!("{:>5}", time_string(&s, true).unwrap_or(s)));
        end = end.map(|e| time_string(&e, true).unwrap_or(e));
    }
    if let Some((at, group)) = tag_group(&txt) {
        let remove = opt_sym("org-agenda-remove-tags", "nil");
        if remove == "t" || (remove != "nil" && p.has_tag) {
            txt.truncate(at);
        } else {
            let n = txt.chars().count();
            let spaces = 50usize.saturating_sub(n).max(1);
            txt = format!("{}{}{group}", &txt[..at], " ".repeat(spaces));
        }
    }
    if let Some(re) = i.remove_re {
        while let Some(m) = re.find(&txt) {
            if m.range().is_empty() {
                break;
            }
            txt.replace_range(m.range(), "");
        }
    }
    let breadcrumbs = if p.has_breadcrumbs && !i.outline.is_empty() {
        let sep = opt_string("org-agenda-breadcrumbs-separator", "->");
        let mut s = i
            .outline
            .iter()
            .map(|h| h.trim_end().to_owned())
            .collect::<Vec<_>>()
            .join(&sep);
        let w = width.saturating_sub(1).max(1);
        if s.chars().count() > w {
            s = s.chars().take(w.saturating_sub(2)).collect::<String>() + "..";
        }
        format!("{s}{sep}")
    } else {
        String::new()
    };
    let ampm = opt_bool("org-agenda-timegrid-use-ampm", false);
    let am = |t: &str| {
        if ampm {
            to_ampm(t.trim())
        } else {
            t.to_owned()
        }
    };
    let time = match (&start, &end) {
        (Some(s), Some(e)) => format!("{}-{}{}", am(s), am(e), if ampm { " " } else { "" }),
        (Some(s), None) => format!("{}{}{}", am(s), grid.2, if ampm { " " } else { "" }),
        _ => String::new(),
    };
    let mut cat = category.clone();
    let mut extra_space = String::new();
    if let Some(c) = re!(r"\[\[([^\]]+)\](?:\[([^\]]+)\])?\]").captures(&cat) {
        let w = c
            .get(2)
            .or(c.get(1))
            .map_or(0, |m| m.as_str().chars().count());
        if w < p.cat_len.unwrap_or(0) {
            extra_space = " ".repeat(p.cat_len.unwrap_or(0) - w - 1);
        }
    } else if let Some(max) = p.cat_max
        && cat.chars().count() >= max
    {
        cat = cat.chars().take(max.saturating_sub(1)).collect();
    }
    let level = i.level.clone().unwrap_or_default();
    let prefix = p.eval(&Vars {
        category: &cat,
        extra_space: &extra_space,
        time: &time,
        level: &level,
        extra: &extra,
        tag: &tag,
        effort: &effort,
        breadcrumbs: &breadcrumbs,
    });
    Item {
        line: format!("{prefix}{txt}"),
        heading: prefix.len(),
        category: full_category,
        tags: i.tags,
        time_of_day: tod,
        duration,
        breadcrumbs,
        txt,
        level,
        time,
        extra,
        dotime: i.dotime,
        effort: i.effort,
        ..Item::default()
    }
}

/// org-agenda-time-of-day-to-ampm.
fn to_ampm(t: &str) -> String {
    let (h, m) = t.split_once(':').unwrap_or((t, "00"));
    let mut h: i64 = h.trim_start_matches('+').parse().unwrap_or(0);
    let mut ampm = "am";
    if h == 12 {
        ampm = "pm";
    } else if h > 12 {
        ampm = "pm";
        h -= 12;
    }
    let hs = if opt_bool("org-agenda-time-leading-zero", false) {
        format!("{h:02}")
    } else {
        format!("{h:>2}")
    };
    format!("{hs}:{m}{ampm}")
}

/// org-get-priority on a formatted line.
pub fn priority_of(s: &str, lowest: u32, default: u32) -> i64 {
    match re!(r"\[#([A-Z0-9]+)\] ?").captures(s) {
        Some(c) => {
            let v = crate::org::syntax::priority_value(&c[1]).unwrap_or(default) as i64;
            1000 * (lowest as i64 - v)
        }
        None => 1000 * (lowest as i64 - default as i64),
    }
}

// ---- the time grid ----

/// org-agenda-time-grid: (conditions, times, after-time string, line string).
pub fn grid_spec() -> (Vec<String>, Vec<i64>, String, String) {
    let def = (
        vec!["daily".into(), "today".into(), "require-timed".into()],
        vec![800, 1000, 1200, 1400, 1600, 1800, 2000],
        " ┄┄┄┄┄ ".to_owned(),
        "┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄".to_owned(),
    );
    let Some(v) = opt("org-agenda-time-grid") else {
        return def;
    };
    let Some(l) = v.list() else { return def };
    let conds = l
        .first()
        .and_then(Sexp::list)
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| x.str().map(str::to_owned))
        .collect();
    let times = l
        .get(1)
        .and_then(Sexp::list)
        .unwrap_or(&[])
        .iter()
        .filter_map(Sexp::int)
        .collect();
    let s2 = l.get(2).and_then(Sexp::str).unwrap_or("").to_owned();
    let s3 = l.get(3).and_then(Sexp::str).unwrap_or("").to_owned();
    (conds, times, s2, s3)
}

pub fn current_time_string() -> String {
    opt_string(
        "org-agenda-current-time-string",
        "← now ───────────────────────────────────────────────",
    )
}

/// org-agenda-add-time-grid-maybe.
pub fn add_grid(
    p: &Prefix,
    mut list: Vec<Item>,
    ndays: i64,
    todayp: bool,
    sorting: &[String],
) -> Vec<Item> {
    if !opt_bool("org-agenda-use-time-grid", true) {
        return list;
    }
    let (req, times, _, string) = grid_spec();
    let has = |s: &str| req.iter().any(|r| r == s);
    if !((todayp && has("today")) || (ndays == 1 && has("daily")) || has("weekly")) {
        return list;
    }
    let have: Vec<i64> = list.iter().filter_map(|x| x.time_of_day).collect();
    if has("require-timed") && have.is_empty() {
        return list;
    }
    let remove = has("remove-match");
    let mut new = vec![];
    for t in times {
        if remove && have.contains(&t) {
            continue;
        }
        let t = format!("{t:04}");
        let mut it = item(
            p,
            In {
                txt: string.clone(),
                category: Some(String::new()),
                dotime: Dotime::Str(format!("{}:{}", &t[..2], &t[2..])),
                no_default_duration: true,
                ..In::default()
            },
        );
        it.face = Face::TimeGrid;
        new.push(it);
    }
    if todayp && opt_bool("org-agenda-show-current-time-in-grid", true) {
        let now = civil::now();
        let mut it = item(
            p,
            In {
                txt: current_time_string(),
                category: Some(String::new()),
                dotime: Dotime::Str(format!("{:02}:{:02} ", now.hour, now.min)),
                no_default_duration: true,
                ..In::default()
            },
        );
        it.face = Face::CurrentTime;
        new.push(it);
    }
    new.reverse();
    if sorting.iter().any(|s| s == "time-up") {
        new.append(&mut list);
        new
    } else {
        list.append(&mut new);
        list
    }
}

// ---- sorting ----

/// org-set-sorting-strategy.
pub fn strategy(key: &str) -> Vec<String> {
    let def: Vec<String> = match key {
        "agenda" => vec!["habit-down", "time-up", "urgency-down", "category-keep"],
        "todo" | "tags" => vec!["urgency-down", "category-keep"],
        "search" => vec!["category-keep"],
        _ => vec!["time-up", "category-keep", "urgency-down"],
    }
    .into_iter()
    .map(String::from)
    .collect();
    let Some(v) = opt("org-agenda-sorting-strategy") else {
        return def;
    };
    let Some(l) = v.list() else { return def };
    let syms = |l: &[Sexp]| {
        l.iter()
            .filter_map(|x| x.str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    if l.first()
        .is_some_and(|x| x.sym().is_some() && x.list().is_none())
    {
        return syms(l);
    }
    let find = |k: &str| {
        l.iter()
            .find(|e| e.car().and_then(Sexp::sym) == Some(k))
            .map(|e| syms(e.cdr().list().unwrap_or(&[])))
    };
    find(key).or_else(|| find("agenda")).unwrap_or_else(|| {
        ["time-up", "category-keep", "urgency-down"]
            .map(String::from)
            .to_vec()
    })
}

fn cmp_num<T: PartialOrd>(a: T, b: T) -> Option<i8> {
    if a > b {
        Some(1)
    } else if a < b {
        Some(-1)
    } else {
        None
    }
}

fn alpha_key(x: &Item) -> Option<String> {
    if x.line.len() < x.heading && x.marker.is_none() {
        return None;
    }
    let t = x.line.get(x.heading..)?;
    let t = t.trim_start();
    let t = match t.split_once(' ') {
        Some((k, rest)) if x.todo_kwds.iter().any(|kw| kw == k) => rest,
        _ => t,
    };
    let t = re!(r"^[ \t]*\[[a-zA-Z0-9]\] *").replace(t, "");
    Some(t.to_lowercase())
}

/// One strategy's comparison of `a` and `b`: -1, +1 or undecided.
fn cmp(s: &str, a: &Item, b: &Item, done: &[String]) -> Option<i8> {
    let late = opt_bool("org-agenda-sort-notime-is-late", true);
    let ts = |x: &Item, t: &str| {
        if x.kind.contains(t) {
            x.ts_date.unwrap_or(if late { i64::MAX } else { -1 })
        } else if late {
            i64::MAX
        } else {
            -1
        }
    };
    let effort = |x: &Item| {
        x.effort_minutes
            .unwrap_or(if opt_bool("org-agenda-sort-noeffort-is-high", true) {
                f64::MAX
            } else {
                -1.0
            })
    };
    let time = |x: &Item| x.time_of_day.unwrap_or(if late { 9901 } else { -1 });
    let todo = |a: &Item, b: &Item| {
        let kw = if a.todo_kwds.is_empty() {
            &b.todo_kwds
        } else {
            &a.todo_kwds
        };
        let pos = |x: &Item| {
            let t = x.todo_state.as_deref().unwrap_or("");
            kw.iter()
                .position(|k| k == t)
                .map_or(0, |i| -((kw.len() - i) as i64))
        };
        let da = a.todo_state.as_ref().is_some_and(|t| done.contains(t));
        let db = b.todo_state.as_ref().is_some_and(|t| done.contains(t));
        if da && !db {
            Some(-1)
        } else if !da && db {
            Some(1)
        } else {
            cmp_num(pos(b), pos(a)).map(|x| -x)
        }
    };
    let alpha = |a: &Item, b: &Item| match (alpha_key(a), alpha_key(b)) {
        (None, None) => None,
        (None, _) => Some(1),
        (_, None) => Some(-1),
        (Some(x), Some(y)) => cmp_num(x, y),
    };
    let tag = |a: &Item, b: &Item| match (a.tags.last(), b.tags.last()) {
        (None, None) => None,
        (None, _) => Some(1),
        (_, None) => Some(-1),
        (Some(x), Some(y)) => cmp_num(&x.0, &y.0),
    };
    match s {
        "timestamp-up" => cmp_num(ts(a, ""), ts(b, "")),
        "timestamp-down" => cmp_num(ts(b, ""), ts(a, "")),
        "scheduled-up" => cmp_num(ts(a, "scheduled"), ts(b, "scheduled")),
        "scheduled-down" => cmp_num(ts(b, "scheduled"), ts(a, "scheduled")),
        "deadline-up" => cmp_num(ts(a, "deadline"), ts(b, "deadline")),
        "deadline-down" => cmp_num(ts(b, "deadline"), ts(a, "deadline")),
        "tsia-up" => cmp_num(ts(a, "timestamp_ia"), ts(b, "timestamp_ia")),
        "tsia-down" => cmp_num(ts(b, "timestamp_ia"), ts(a, "timestamp_ia")),
        "ts-up" => cmp_num(ts(a, "timestamp"), ts(b, "timestamp")),
        "ts-down" => cmp_num(ts(b, "timestamp"), ts(a, "timestamp")),
        "time-up" => cmp_num(time(a), time(b)),
        "time-down" => cmp_num(time(b), time(a)),
        "stats-up" | "stats-down" => None,
        "priority-up" => cmp_num(a.priority, b.priority),
        "priority-down" => cmp_num(b.priority, a.priority),
        "urgency-up" => cmp_num(a.urgency, b.urgency),
        "urgency-down" => cmp_num(b.urgency, a.urgency),
        "effort-up" => cmp_num(effort(a), effort(b)),
        "effort-down" => cmp_num(effort(b), effort(a)),
        "category-up" => cmp_num(&a.category, &b.category),
        "category-down" => cmp_num(&b.category, &a.category),
        "category-keep" => (a.category != b.category).then_some(1),
        "tag-up" => tag(a, b),
        "tag-down" => tag(b, a),
        "todo-state-up" => todo(a, b),
        "todo-state-down" => todo(b, a),
        "habit-up" => cmp_num(b.habit.is_some(), a.habit.is_some()),
        "habit-down" => cmp_num(a.habit.is_some(), b.habit.is_some()),
        "alpha-up" => alpha(a, b),
        "alpha-down" => alpha(b, a),
        _ => None,
    }
}

/// org-entries-lessp.
pub fn lessp(strategy: &[String], a: &Item, b: &Item, done: &[String]) -> bool {
    for s in strategy {
        match cmp(s, a, b, done) {
            Some(-1) => return true,
            Some(1) => return false,
            _ => {}
        }
    }
    false
}

/// A stable merge sort with a `lessp` predicate (Emacs `sort`).
pub fn sort(v: Vec<Item>, less: &dyn Fn(&Item, &Item) -> bool) -> Vec<Item> {
    if v.len() <= 1 {
        return v;
    }
    let mut v = v;
    let right = v.split_off(v.len() / 2);
    let (l, r) = (sort(v, less), sort(right, less));
    let mut out = Vec::with_capacity(l.len() + r.len());
    let (mut l, mut r) = (l.into_iter().peekable(), r.into_iter().peekable());
    loop {
        match (l.peek(), r.peek()) {
            (Some(a), Some(b)) => {
                if less(b, a) {
                    out.push(r.next().unwrap());
                } else {
                    out.push(l.next().unwrap());
                }
            }
            (Some(_), None) => out.push(l.next().unwrap()),
            (None, Some(_)) => out.push(r.next().unwrap()),
            (None, None) => break,
        }
    }
    out
}

/// org-agenda-highlight-todo: normalize the keyword (org-agenda-todo-keyword-format).
pub fn highlight_todo(x: &mut Item) {
    let Some(rest) = x.line.get(x.heading..) else {
        return;
    };
    let dots = rest.len() - rest.trim_start_matches('.').len();
    let after = &rest[dots..];
    let Some(kw) = x
        .todo_kwds
        .iter()
        .filter(|k| after.starts_with(k.as_str()))
        .max_by_key(|k| k.len())
        .cloned()
    else {
        return;
    };
    let tail = &after[kw.len()..];
    let spaces = tail.len() - tail.trim_start_matches(' ').len();
    if spaces == 0 {
        return;
    }
    let fmt = opt_string("org-agenda-todo-keyword-format", "%-1s");
    if fmt.is_empty() {
        return;
    }
    let (left, w) = parse_s_format(&fmt);
    let shown = pad(&kw, left, w, None);
    let new = format!(
        "{}{}{shown} {}",
        &x.line[..x.heading],
        &rest[..dots],
        &tail[spaces..]
    );
    x.line = new;
}

fn parse_s_format(f: &str) -> (bool, Option<usize>) {
    let inner = f.trim_start_matches('%').trim_end_matches('s');
    (
        inner.starts_with('-'),
        inner.trim_start_matches('-').parse().ok(),
    )
}

/// org-agenda-limit-entries.
pub fn limit(list: Vec<Item>, limit: Option<i64>, val: &dyn Fn(&Item) -> Option<f64>) -> Vec<Item> {
    let Some(limit) = limit else { return list };
    let include = limit < 0;
    let mut lim = 0.0;
    list.into_iter()
        .filter(|e| match val(e) {
            Some(p) => {
                lim += p;
                lim <= limit.abs() as f64
            }
            None => include,
        })
        .collect()
}

fn max_for(name: &str, kind: &str) -> Option<i64> {
    match opt(name)? {
        Sexp::Int(n) => Some(n),
        v => v
            .list()?
            .iter()
            .find(|e| e.car().and_then(Sexp::sym) == Some(kind))
            .and_then(|e| e.cdr().int()),
    }
}

/// org-agenda-finalize-entries: highlight, sort and limit.
pub fn finalize_entries(
    list: Vec<Item>,
    kind: &str,
    strategy: &[String],
    done: &[String],
) -> Vec<Item> {
    let mut list = list;
    for x in &mut list {
        highlight_todo(x);
    }
    let mut list = sort(list, &|a, b| lessp(strategy, a, b, done));
    let high = opt_bool("org-agenda-sort-noeffort-is-high", true);
    list = limit(list, max_for("org-agenda-max-effort", kind), &|e| {
        Some(
            e.effort_minutes
                .unwrap_or(if high { f64::MAX } else { -1.0 }),
        )
    });
    list = limit(list, max_for("org-agenda-max-todos", kind), &|e| {
        e.todo_state.as_ref().map(|_| 1.0)
    });
    list = limit(list, max_for("org-agenda-max-tags", kind), &|e| {
        (!e.tags.is_empty()).then_some(1.0)
    });
    list = limit(list, max_for("org-agenda-max-entries", kind), &|e| {
        e.hd.as_ref().map(|_| 1.0)
    });
    list
}

/// org-agenda-align-tags on one line.
pub fn align_tags(line: &str, width: usize) -> String {
    let Some((at, group)) = tag_group(line) else {
        return line.to_owned();
    };
    let col = match opt("org-agenda-tags-column") {
        Some(Sexp::Int(n)) => n,
        _ => -(width as i64),
    };
    let l = unicode_width::UnicodeWidthStr::width(group.as_str()) as i64;
    let c = if col < 0 { col.abs() - l } else { col };
    let head = line[..at].trim_end_matches([' ', '\t']);
    let cur = unicode_width::UnicodeWidthStr::width(head) as i64;
    let n = (c - cur).max(1) as usize;
    format!("{head}{}{group}", " ".repeat(n))
}

// ---- dates ----

/// org-agenda-format-date (org-agenda-format-date-aligned by default).
pub fn format_date(day: i64) -> String {
    let t = Tm::from_absolute(day);
    if let Some(Sexp::Str(f)) = opt("org-agenda-format-date") {
        return civil::format(&f, t);
    }
    let wd = t.weekday();
    let week = if wd == 1 {
        format!(" W{:02}", civil::iso_week(t).0)
    } else {
        String::new()
    };
    format!(
        "{:<10} {:2} {} {:4}{week}",
        civil::DAY_NAMES[wd as usize],
        t.day,
        civil::MONTH_NAMES[(t.month - 1) as usize],
        t.year
    )
}

/// org-agenda-get-day-face.
pub fn day_face(day: i64, today: i64) -> Face {
    let wd = Tm::from_absolute(day).weekday();
    let weekend = match opt("org-agenda-weekend-days") {
        Some(v) => v.list().unwrap_or(&[]).iter().any(|x| x.int() == Some(wd)),
        None => wd == 6 || wd == 0,
    };
    match (day == today, weekend) {
        (true, true) => Face::DateWeekendToday,
        (true, false) => Face::DateToday,
        (false, true) => Face::DateWeekend,
        _ => Face::Date,
    }
}

/// org-agenda-deadline-face: the face for a fraction of the warning period.
pub fn deadline_face(fraction: f64) -> Face {
    Face::Deadline(fraction)
}

/// The org-agenda-deadline-faces entry for a fraction: (min fraction, face spec).
pub fn deadline_face_spec(fraction: f64) -> Option<Sexp> {
    let v = opt("org-agenda-deadline-faces")?;
    v.list()?
        .iter()
        .find(|e| {
            e.car()
                .map(|c| match c {
                    Sexp::Float(f) => *f,
                    Sexp::Int(i) => *i as f64,
                    _ => 2.0,
                })
                .is_some_and(|f| f <= fraction)
        })
        .map(Sexp::cdr)
}

/// org-agenda-entry-text-maxlines and friends: the default line count.
pub fn entry_text_lines() -> i64 {
    opt_int("org-agenda-entry-text-maxlines", 5)
}
