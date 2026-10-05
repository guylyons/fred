//! The timestamp model: org-element-timestamp-parser/interpreter, the
//! org.el timestamp regexps (org-ts-regexp*, org-tr-regexp*,
//! org-repeat-re), org-parse-time-string, org-time-stamp-format,
//! org-timestamp-change on a timestamp string, repeaters and warnings.

use super::civil::{self, Tm, Unit, format};
use super::options;
use std::ops::Range;

/// org-ts--internal-regexp.
pub const TS_INNER: &str = r"[0-9]{4}-[0-9]{2}-[0-9]{2}(?: [^\n]*?)?";
/// org-ts-regexp1: groups 1 all, 2 year, 3 month, 4 day, 5 day name,
/// 6 time part, 7 hour, 8 minute.
pub const TS1: &str =
    r"(([0-9]{4})-([0-9]{2})-([0-9]{2})(?: *([^\]+0-9>\r\n -]+))?( ([0-9]{1,2}):([0-9]{2}))?)";
/// org-ts-regexp0.
pub const TS0: &str =
    r"(([0-9]{4})-([0-9]{2})-([0-9]{2})( +[^\]+0-9>\r\n -]+)?( +([0-9]{1,2}):([0-9]{2}))?)";

/// org-ts-regexp3: active or inactive timestamps, with TS1's groups.
pub fn ts3() -> &'static regex::Regex {
    re!(&format!(r"[\[<]{TS1}[^\]>\n]{{0,16}}[\]>]"))
}

/// org-ts-regexp-both (group 1 the inside).
pub fn ts_both() -> &'static regex::Regex {
    re!(&format!(r"[\[<]({TS_INNER})[\]>]"))
}

/// org-tr-regexp-both: a date range (groups 1 and 2 the insides).
pub fn tr_both() -> &'static regex::Regex {
    re!(&format!(
        r"[\[<]({TS_INNER})[\]>]--?-?[\[<]({TS_INNER})[\]>]"
    ))
}

/// org-tsr-regexp-both: a timestamp or range (group 1 first inside,
/// group 3 the second's).
pub fn tsr_both() -> &'static regex::Regex {
    re!(&format!(
        r"[\[<]({TS_INNER})[\]>](--?-?[\[<]({TS_INNER})[\]>])?"
    ))
}

/// org-repeat-re: group 1 the repeater.
pub fn repeat_re() -> &'static regex::Regex {
    re!(r"<[0-9]{4}-[0-9]{2}-[0-9]{2} [^>\n]*?([.+]?\+[0-9]+[hdwmy](/[0-9]+[hdwmy])?)")
}

/// org-plain-time-of-day-regexp: group 1 the first time, 7 the `-` part,
/// 8 the second time.
pub fn plain_time_of_day() -> &'static regex::Regex {
    re!(
        r"(\b[012]?[0-9]((:([0-5][0-9]([AaPp][Mm])?))|([AaPp][Mm]))\b)(--?(\b[012]?[0-9]((:([0-5][0-9]([AaPp][Mm])?))|([AaPp][Mm]))\b))?"
    )
}

// ---- formats ----

/// The second argument of org-time-stamp-format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brackets {
    Active,
    Inactive,
    None,
    Keep,
}

impl Brackets {
    pub fn of(inactive: bool) -> Brackets {
        if inactive {
            Brackets::Inactive
        } else {
            Brackets::Active
        }
    }
}

fn format_pair(name: &str, default: (&str, &str)) -> (String, String) {
    let v = super::sexp::option(name);
    let pair = v.as_ref().and_then(|v| match v {
        super::sexp::Sexp::Dotted(a, b) if a.len() == 1 => {
            Some((a[0].str()?.to_owned(), b.str()?.to_owned()))
        }
        super::sexp::Sexp::List(l) if l.len() == 2 => {
            Some((l[0].str()?.to_owned(), l[1].str()?.to_owned()))
        }
        _ => None,
    });
    pair.unwrap_or_else(|| (default.0.to_owned(), default.1.to_owned()))
}

/// org-timestamp-formats (a defconst upstream; overridable here for
/// tests like org-test-without-dow).
pub fn formats() -> (String, String) {
    format_pair(
        "org-timestamp-formats",
        ("%Y-%m-%d %a", "%Y-%m-%d %a %H:%M"),
    )
}

/// org-timestamp-custom-formats.
pub fn custom_formats() -> (String, String) {
    format_pair(
        "org-timestamp-custom-formats",
        ("%m/%d/%y %a", "%m/%d/%y %a %H:%M"),
    )
}

/// org-time-stamp-format.
pub fn time_stamp_format(with_time: bool, brackets: Brackets, custom: bool) -> String {
    let (d, t) = if custom { custom_formats() } else { formats() };
    let mut f = if with_time { t } else { d };
    if brackets != Brackets::Keep
        && f.len() >= 2
        && ((f.starts_with('<') && f.ends_with('>')) || (f.starts_with('[') && f.ends_with(']')))
    {
        f = f[1..f.len() - 1].to_owned();
    }
    match brackets {
        Brackets::None | Brackets::Keep => f,
        Brackets::Active => format!("<{f}>"),
        Brackets::Inactive => format!("[{f}]"),
    }
}

/// The text org-insert-timestamp inserts: `extra` (time range end,
/// repeaters) goes before the closing bracket.
pub fn stamp_text(t: Tm, with_hm: bool, inactive: bool, extra: &str) -> String {
    let mut fmt = time_stamp_format(with_hm, Brackets::of(inactive), false);
    if !extra.is_empty() {
        let close = fmt.pop().unwrap_or('>');
        fmt.push_str(&extra.replace('%', "%%"));
        fmt.push(close);
    }
    format(&fmt, t)
}

/// The `(list org-end-time-was-given)` extra of org-insert-timestamp:
/// "13:15" becomes "-13:15".
pub fn end_time_extra(end: Option<&str>) -> String {
    let Some(e) = end else { return String::new() };
    match re!(r"([0-9]+):([0-9]+)").captures(e) {
        Some(c) => format!(
            "-{:02}:{:02}",
            c[1].parse::<i64>().unwrap_or(0),
            c[2].parse::<i64>().unwrap_or(0)
        ),
        None => String::new(),
    }
}

// ---- parsing time strings ----

/// org-parse-time-string with NODEFAULT: hour/minute absent when not given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: Option<i64>,
    pub min: Option<i64>,
}

impl Parsed {
    /// With hour and minute defaulting to 0.
    pub fn tm(self) -> Tm {
        Tm::at(
            self.year,
            self.month,
            self.day,
            self.hour.unwrap_or(0),
            self.min.unwrap_or(0),
        )
    }
}

/// org-parse-time-string: the first YYYY-MM-DD[ day][ HH:MM] in `s`.
pub fn parse_time_string(s: &str) -> Option<Parsed> {
    let c = re!(TS0).captures(s)?;
    let n = |i: usize| c.get(i).map(|m| m.as_str().parse::<i64>().unwrap_or(0));
    Some(Parsed {
        year: n(2)?,
        month: n(3)?,
        day: n(4)?,
        hour: n(7),
        min: n(8),
    })
}

/// org-time-string-to-time (a decoded local time).
pub fn time_string_to_tm(s: &str) -> Result<Tm, String> {
    parse_time_string(s)
        .map(Parsed::tm)
        .ok_or_else(|| format!("Not an Org time string: {s}"))
}

/// org-time-string-to-seconds.
pub fn time_string_to_seconds(s: &str) -> Result<i64, String> {
    Ok(time_string_to_tm(s)?.epoch())
}

/// org-2ft: seconds, 0 when not a time string.
pub fn to_ft(s: &str) -> f64 {
    time_string_to_seconds(s).map_or(0.0, |x| x as f64)
}

/// org-time=, org-time<, ...: `op` on two time strings, false when either
/// is not a time.
pub fn time_cmp(a: &str, b: &str, op: fn(f64, f64) -> bool) -> bool {
    let (a, b) = (to_ft(a), to_ft(b));
    a > 0.0 && b > 0.0 && op(a, b)
}

/// org-matcher-time: `<now>`, `<today>`, `<tomorrow>`, `<yesterday>`,
/// `<+2d>` or a timestamp, as seconds (0 when not recognized).
pub fn matcher_time(s: &str) -> f64 {
    let now = civil::now();
    let today = now.midnight().epoch() as f64;
    match s {
        "<now>" => civil::now_epoch() as f64,
        "<today>" => today,
        "<tomorrow>" => today + 86400.0,
        "<yesterday>" => today - 86400.0,
        _ => {
            if let Some(c) = re!(r"^<([-+][0-9]+)([hdwmy])>$").captures(s) {
                let n: f64 = c[1].parse().unwrap_or(0.0);
                let (base, f) = match &c[2] {
                    "h" => (civil::now_epoch() as f64, 3600.0),
                    "d" => (today, 86400.0),
                    "w" => (today, 604_800.0),
                    "m" => (today, 2_678_400.0),
                    _ => (today, 31_557_600.0),
                };
                base + n * f
            } else if re!(TS0).is_match(s) {
                to_ft(s)
            } else {
                0.0
            }
        }
    }
}

/// org-get-compact-tod: "10:00", or "10:00+1:30" for a time range.
pub fn compact_tod(s: &str) -> Option<String> {
    let c = re!(r"(([012]?[0-9]):([0-5][0-9]))(-(([012]?[0-9]):([0-5][0-9])))?").captures(s)?;
    let t1 = c[1].to_owned();
    if c.get(4).is_none() {
        return Some(t1);
    }
    let n = |i: usize| c[i].parse::<i64>().unwrap_or(0);
    let (mut dh, mut dm) = (n(6) - n(2), n(7) - n(3));
    if dm < 0 {
        dm += 60;
        dh -= 1;
    }
    Some(format!(
        "{t1}+{dh}{}",
        if dm != 0 {
            format!(":{dm:02}")
        } else {
            String::new()
        }
    ))
}

/// org-small-year-to-year.
pub fn small_year_to_year(year: i64) -> i64 {
    if year >= 100 {
        return year;
    }
    let current = civil::now().year;
    let century = current / 100;
    let offset = year - current % 100;
    if offset > 30 {
        (century - 1) * 100 + year
    } else if offset > -70 {
        century * 100 + year
    } else {
        (century + 1) * 100 + year
    }
}

/// org-days-to-iso-week of an absolute day number.
pub fn days_to_iso_week(abs: i64) -> i64 {
    civil::iso_week(Tm::from_absolute(abs)).0
}

/// org-timestamp-to-now: days (or seconds) from now to the time string.
pub fn timestamp_to_now(s: &str, seconds: bool) -> Result<i64, String> {
    let t = time_string_to_tm(s)?;
    Ok(if seconds {
        t.epoch() - civil::now_epoch()
    } else {
        t.absolute() - civil::now().absolute()
    })
}

fn opt_int(name: &str, default: i64) -> i64 {
    options::int(name, default)
}

/// org-get-wdays: deadline lead time (or scheduled delay with `delay`).
pub fn get_wdays(ts: &str, delay: bool, zero_delay: bool) -> i64 {
    let tv = if delay {
        opt_int("org-scheduled-delay-days", 0)
    } else {
        opt_int("org-deadline-warning-days", 14)
    };
    if (delay && tv < 0) || (delay && zero_delay && tv <= 0) || (!delay && tv <= 0) {
        return -tv;
    }
    if let Some(c) = re!(r"-([0-9]+)([hdwmy])(?:$|>| )").captures(ts) {
        let n: f64 = c[1].parse().unwrap_or(0.0);
        let f = match &c[2] {
            "d" => 1.0,
            "w" => 7.0,
            "m" => 30.4,
            "y" => 365.25,
            _ => 0.041667,
        };
        return (n * f).floor() as i64;
    }
    tv
}

/// org-get-repeat with a TIMESTAMP string.
pub fn repeat_of(ts: &str) -> Option<String> {
    repeat_re().captures(ts).map(|c| c[1].to_owned())
}

/// org-closest-date: the absolute day of the repetition of `start`
/// closest to `current` (`prefer` "past", "future" or "").
pub fn closest_date(start: &str, current: &str, prefer: &str) -> Result<i64, String> {
    let st = time_string_to_tm(start)?;
    let Some(c) = re!(r"\+([0-9]+)([hdwmy])").captures(start) else {
        return Ok(st.absolute());
    };
    let value: i64 = c[1].parse().unwrap_or(0);
    if value == 0 {
        return Ok(st.absolute());
    }
    let base = st;
    let target = time_string_to_tm(current)?;
    let (sday, cday) = (base.absolute(), target.absolute());
    if cday <= sday {
        return Ok(sday);
    }
    let (n1, n2) = match &c[2] {
        "h" => {
            let missing = (24 * (cday - sday) - base.hour + opt_int("org-extend-today-until", 0))
                .rem_euclid(value);
            let n1 = if missing == 0 {
                cday
            } else {
                cday - (1 + missing / 24)
            };
            (n1, cday + (value - missing) / 24)
        }
        "d" | "w" => {
            let v = if &c[2] == "w" { 7 * value } else { value };
            let n1 = sday + v * ((cday - sday) / v);
            (n1, n1 + v)
        }
        "m" => {
            // Months as (month day year) lists, as upstream (no clamping).
            let add = |y: i64, m: i64, d: i64, n: i64| {
                let mm = m + n;
                (y + mm.div_euclid(12), mm.rem_euclid(12), d)
            };
            let abs = |(y, m, d): (i64, i64, i64)| Tm::date(y, m, d).absolute();
            let months = (12 * (target.year - base.year)
                + (target.month - base.month)
                + if target.day > base.day { 0 } else { -1 })
                / value
                * value;
            let before = add(base.year, base.month, base.day, months);
            (abs(before), abs(add(before.0, before.1, before.2, value)))
        }
        _ => {
            let (d, m, y) = (base.day, base.month, base.year);
            let incomplete = if target.month > m || (target.month == m && target.day > d) {
                0
            } else {
                1
            };
            let years = (target.year - y - incomplete) / value * value;
            let before = Tm::date(y + years, m, d);
            (
                before.absolute(),
                Tm::date(before.year + value, m, d).absolute(),
            )
        }
    };
    Ok(match prefer {
        "past" => {
            if cday == n2 {
                n2
            } else {
                n1
            }
        }
        "future" => {
            if cday == n1 {
                n1
            } else {
                n2
            }
        }
        _ => {
            if (cday - n1).abs() > (cday - n2).abs() {
                n2
            } else {
                n1
            }
        }
    })
}

/// org-time-string-to-absolute: Err("org-diary-sexp-no-match") for a
/// diary sexp that does not match `daynr`.
pub fn time_string_to_absolute(s: &str, daynr: Option<i64>, prefer: &str) -> Result<i64, String> {
    if let Some(c) = re!(r"^%%(\(.*\))").captures(s) {
        return match daynr {
            Some(d) if super::diary::entry(&c[1], "", Tm::from_absolute(d)).is_some() => Ok(d),
            _ => Err(format!("org-diary-sexp-no-match: {s}")),
        };
    }
    match daynr {
        Some(d) => closest_date(s, &civil::format("%Y-%m-%d", Tm::from_absolute(d)), prefer),
        None => time_string_to_tm(s)
            .map(Tm::absolute)
            .map_err(|e| format!("Bad timestamp `{s}'\nError was: {e}")),
    }
}

/// org-make-tdiff-string.
pub fn tdiff_string(y: i64, d: i64, h: i64, m: i64) -> String {
    let mut s = String::new();
    for (n, w) in [(y, "year"), (d, "day"), (h, "hour"), (m, "minute")] {
        if n > 0 {
            s.push_str(&format!("{n} {w}{} ", if n > 1 { "s" } else { "" }));
        }
    }
    s
}

// ---- the timestamp object ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Active,
    Inactive,
    ActiveRange,
    InactiveRange,
    Diary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeType {
    Date,
    Time,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatType {
    /// `+`
    Cumulate,
    /// `++`
    CatchUp,
    /// `.+`
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repeater {
    pub kind: RepeatType,
    pub value: i64,
    pub unit: Unit,
    /// org-habit's `/3d` deadline.
    pub deadline: Option<(i64, Unit)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Warning {
    /// `--2d` (first occurrence only) rather than `-2d`.
    pub first: bool,
    pub value: i64,
    pub unit: Unit,
}

/// One end of a timestamp. Diary timestamps have no date (zeros).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Moment {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: Option<i64>,
    pub minute: Option<i64>,
}

impl Moment {
    pub fn tm(self) -> Tm {
        Tm::at(
            self.year,
            self.month,
            self.day,
            self.hour.unwrap_or(0),
            self.minute.unwrap_or(0),
        )
    }
}

/// A timestamp object (org-element-timestamp-parser's properties).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub kind: Kind,
    pub range: Option<RangeType>,
    pub raw: String,
    pub start: Moment,
    pub end: Moment,
    /// The `(sexp)` of a diary timestamp.
    pub diary: Option<String>,
    pub repeater: Option<Repeater>,
    pub warning: Option<Warning>,
    /// Bytes from the start through the trailing blanks (`:end`).
    pub len: usize,
}

fn unit(c: &str) -> Unit {
    Unit::parse(c).unwrap_or(Unit::Year)
}

impl Timestamp {
    /// org-element-timestamp-parser on the text at the start of `s`.
    pub fn parse(s: &str) -> Option<Timestamp> {
        let elem = re!(&format!(
            r"^(?:[\[<]({TS_INNER})[\]>]|<%%(?:\([^>\n]+\))([^\n>]*)>)"
        ));
        let em = elem.captures(s)?;
        let activep = s.starts_with('<');
        let raw_c = re!(&format!(
            r"^([<\[](%%)?.*?)[\]>](?:--([\[<]({TS_INNER})[\]>]))?"
        ))
        .captures(s)?;
        let raw = raw_c[0].to_owned();
        let diaryp = raw_c.get(2).is_some();
        let (date_start, diary) = if diaryp {
            let g2 = em.get(2)?;
            (g2.as_str().to_owned(), Some(s[3..g2.start()].to_owned()))
        } else {
            (raw_c[1].to_owned(), None)
        };
        let date_end = if diaryp {
            None
        } else {
            raw_c.get(3).map(|m| m.as_str().to_owned())
        };
        let after = raw.len();
        let len = after
            + s[after..]
                .bytes()
                .take_while(|&b| b == b' ' || b == b'\t')
                .count();
        let time_range = re!(r"[012]?[0-9]:[0-5][0-9](-([012]?[0-9]):([0-5][0-9]))")
            .captures(&date_start)
            .map(|c| {
                (
                    c[2].parse::<i64>().unwrap_or(0),
                    c[3].parse::<i64>().unwrap_or(0),
                )
            });
        let ranged = date_end.is_some() || time_range.is_some();
        let kind = match (diaryp, activep, ranged) {
            (true, ..) => Kind::Diary,
            (_, true, true) => Kind::ActiveRange,
            (_, true, false) => Kind::Active,
            (_, false, true) => Kind::InactiveRange,
            _ => Kind::Inactive,
        };
        let range = if date_end.is_some() {
            Some(RangeType::Date)
        } else {
            time_range.map(|_| RangeType::Time)
        };
        let mut repeater = None;
        let mut warning = None;
        if !diaryp {
            if let Some(c) =
                re!(r"([+.]?\+)([0-9]+)([hdwmy])(?:/([0-9]+)([hdwmy]))?").captures(&raw)
            {
                repeater = Some(Repeater {
                    kind: match &c[1] {
                        "++" => RepeatType::CatchUp,
                        ".+" => RepeatType::Restart,
                        _ => RepeatType::Cumulate,
                    },
                    value: c[2].parse().unwrap_or(0),
                    unit: unit(&c[3]),
                    deadline: c
                        .get(4)
                        .map(|v| (v.as_str().parse().unwrap_or(0), unit(&c[5]))),
                });
            }
            if let Some(c) = re!(r"(-)?-([0-9]+)([hdwmy])").captures(&raw) {
                warning = Some(Warning {
                    first: c.get(1).is_some(),
                    value: c[2].parse().unwrap_or(0),
                    unit: unit(&c[3]),
                });
            }
        }
        let (mut start, mut end) = (Moment::default(), Moment::default());
        if !diaryp {
            let d = parse_time_string(&date_start)?;
            start = Moment {
                year: d.year,
                month: d.month,
                day: d.day,
                hour: d.hour,
                minute: d.min,
            };
            let e = date_end.as_deref().and_then(parse_time_string);
            end = Moment {
                year: e.map_or(start.year, |e| e.year),
                month: e.map_or(start.month, |e| e.month),
                day: e.map_or(start.day, |e| e.day),
                hour: e
                    .and_then(|e| e.hour)
                    .or(time_range.map(|t| t.0))
                    .or(start.hour),
                minute: e
                    .and_then(|e| e.min)
                    .or(time_range.map(|t| t.1))
                    .or(start.minute),
            };
        } else if let Some(c) =
            re!(r"([012]?[0-9]):([0-5][0-9])(-([012]?[0-9]):([0-5][0-9]))?").captures(&date_start)
        {
            let n = |i: usize| c.get(i).map(|m| m.as_str().parse::<i64>().unwrap_or(0));
            start.hour = n(1);
            start.minute = n(2);
            end.hour = n(4);
            end.minute = n(5);
        }
        Some(Timestamp {
            kind,
            range,
            raw,
            start,
            end,
            diary,
            repeater,
            warning,
            len,
        })
    }

    /// org-timestamp-from-string.
    pub fn from_string(s: &str) -> Option<Timestamp> {
        if s.trim().is_empty() {
            None
        } else {
            Timestamp::parse(s)
        }
    }

    /// org-timestamp-from-time.
    pub fn from_time(t: Tm, with_time: bool, inactive: bool) -> Timestamp {
        let t = t.norm();
        let m = Moment {
            year: t.year,
            month: t.month,
            day: t.day,
            hour: with_time.then_some(t.hour),
            minute: with_time.then_some(t.min),
        };
        let mut ts = Timestamp {
            kind: if inactive {
                Kind::Inactive
            } else {
                Kind::Active
            },
            range: None,
            raw: String::new(),
            start: m,
            end: Moment::default(),
            diary: None,
            repeater: None,
            warning: None,
            len: 0,
        };
        ts.raw = ts.interpret();
        ts.len = ts.raw.len();
        ts
    }

    pub fn active(&self) -> bool {
        matches!(self.kind, Kind::Active | Kind::ActiveRange | Kind::Diary)
    }

    /// org-timestamp-has-time-p.
    pub fn has_time(&self) -> bool {
        self.start.hour.is_some()
    }

    /// org-timestamp-to-time (start, or end with `end`).
    pub fn to_tm(&self, end: bool) -> Tm {
        let m = if end { self.end } else { self.start };
        m.tm()
    }

    /// org-format-timestamp.
    pub fn format(&self, fmt: &str, end: bool) -> String {
        civil::format(fmt, self.to_tm(end))
    }

    /// org-timestamp-split-range.
    pub fn split_range(&self, end: bool) -> Timestamp {
        if matches!(self.kind, Kind::Active | Kind::Inactive | Kind::Diary) {
            return self.clone();
        }
        let mut t = self.clone();
        t.kind = if self.kind == Kind::ActiveRange {
            Kind::Active
        } else {
            Kind::Inactive
        };
        t.range = None;
        if end {
            t.start = t.end;
        } else {
            t.end = t.start;
        }
        t.raw = t.interpret();
        t
    }

    /// org-timestamp-translate: the custom-format text when
    /// org-display-custom-times is on (`custom`), else the timestamp.
    pub fn translate(&self, custom: bool, boundary: Option<bool>) -> String {
        if !custom || self.kind == Kind::Diary {
            return self.interpret();
        }
        let fmt = time_stamp_format(self.has_time(), Brackets::Keep, true);
        if boundary.is_none() && matches!(self.kind, Kind::ActiveRange | Kind::InactiveRange) {
            format!("{}--{}", self.format(&fmt, false), self.format(&fmt, true))
        } else {
            self.format(&fmt, boundary == Some(true))
        }
    }

    /// org-element-timestamp-interpreter.
    pub fn interpret(&self) -> String {
        let u = |x: Unit| x.letter();
        let mut repeat = String::new();
        if let Some(r) = self.repeater {
            repeat.push_str(match r.kind {
                RepeatType::Cumulate => "+",
                RepeatType::CatchUp => "++",
                RepeatType::Restart => ".+",
            });
            repeat.push_str(&format!("{}{}", r.value, u(r.unit)));
            if let Some((v, du)) = r.deadline {
                repeat.push_str(&format!("/{v}{}", u(du)));
            }
        }
        let warning = self
            .warning
            .map(|w| {
                format!(
                    "{}{}{}",
                    if w.first { "--" } else { "-" },
                    w.value,
                    u(w.unit)
                )
            })
            .unwrap_or_default();
        let (open, close) = if matches!(self.kind, Kind::Inactive | Kind::InactiveRange) {
            ("[", "]")
        } else {
            ("<", ">")
        };
        let mut tail = String::new();
        if !repeat.is_empty() {
            tail.push(' ');
            tail.push_str(&repeat);
        }
        if !warning.is_empty() {
            tail.push(' ');
            tail.push_str(&warning);
        }
        tail.push_str(close);
        let moment = |m: Moment| {
            let with_time = m.hour.is_some() && m.minute.is_some();
            civil::format(&time_stamp_format(with_time, Brackets::None, false), m.tm())
        };
        let s = &self.start;
        let mut out = String::from(open);
        if self.kind == Kind::Diary {
            out.push_str("%%");
            out.push_str(self.diary.as_deref().unwrap_or(""));
            if let (Some(h), Some(m)) = (s.hour, s.minute) {
                out.push_str(&format!(" {h:02}:{m:02}"));
            }
        } else {
            out.push_str(&moment(*s));
        }
        let e = &self.end;
        match (self.kind, self.range) {
            (Kind::Active | Kind::Inactive, _) => {
                if let (Some(hs), Some(he), Some(ms), Some(me)) =
                    (s.hour, e.hour, s.minute, e.minute)
                    && (hs != he || ms != me)
                {
                    out.push_str(&format!("-{he:02}:{me:02}"));
                }
            }
            (Kind::Diary, r) if r != Some(RangeType::Time) => {}
            (_, Some(RangeType::Time)) => {
                out.push_str(&format!(
                    "-{:02}:{:02}",
                    e.hour.or(s.hour).unwrap_or(0),
                    e.minute.or(s.minute).unwrap_or(0)
                ));
            }
            _ => {
                out.push_str(&tail);
                out.push_str("--");
                out.push_str(open);
                let em = Moment {
                    year: if e.year == 0 { s.year } else { e.year },
                    month: if e.month == 0 { s.month } else { e.month },
                    day: if e.day == 0 { s.day } else { e.day },
                    ..*e
                };
                out.push_str(&moment(em));
            }
        }
        out.push_str(&tail);
        out
    }
}

// ---- point in a timestamp ----

/// org-at-timestamp-p's answer: where point is in the timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Bracket,
    After,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    /// Characters after the last known part (day name or minutes).
    Extra(usize),
}

/// org-at-timestamp-p 'lax on a line: the timestamp's byte range
/// (org-ts-regexp3) and the part at byte `pos`.
pub fn at_timestamp(line: &str, pos: usize) -> Option<(Range<usize>, Part)> {
    for c in ts3().captures_iter(line) {
        let m = c.get(0).unwrap();
        if m.start() > pos {
            break;
        }
        if m.end() < pos {
            continue;
        }
        let inr = |i: usize| c.get(i).is_some_and(|g| g.start() <= pos && pos <= g.end());
        let part = if pos == m.start() || pos + 1 == m.end() {
            Part::Bracket
        } else if pos == m.end() {
            Part::After
        } else if inr(2) {
            Part::Year
        } else if inr(3) {
            Part::Month
        } else if inr(7) {
            Part::Hour
        } else if inr(8) {
            Part::Minute
        } else if inr(4) || inr(5) {
            Part::Day
        } else if let Some(last) = c.get(8).or(c.get(5)).map(|g| g.end())
            && pos > last
        {
            Part::Extra(pos - last)
        } else {
            Part::Day
        };
        return Some((m.range(), part));
    }
    None
}

/// org-at-date-range-p: the range around byte `pos` and its two insides.
pub fn date_range_at(
    line: &str,
    pos: usize,
    inactive_ok: bool,
) -> Option<(Range<usize>, Range<usize>, Range<usize>)> {
    let active = re!(&format!(r"<({TS_INNER})>--?-?<({TS_INNER})>"));
    let r = if inactive_ok { tr_both() } else { active };
    r.captures_iter(line)
        .find(|c| {
            let m = c.get(0).unwrap();
            m.start() <= pos && pos <= m.end()
        })
        .map(|c| {
            (
                c.get(0).unwrap().range(),
                c.get(1).unwrap().range(),
                c.get(2).unwrap().range(),
            )
        })
}

// ---- changing a timestamp ----

/// What org-timestamp-change changes: a unit, or the calendar's date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Unit(Unit),
    Calendar(Tm),
}

/// The result of [`change`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub text: String,
    /// Point's byte offset in the new text.
    pub point: usize,
    /// The bracket was toggled (org-toggle-timestamp-type) instead.
    pub toggled: bool,
}

/// org-toggle-timestamp-type on a timestamp's text.
pub fn toggle_type(ts: &str) -> String {
    ts.chars()
        .map(|c| match c {
            '[' => '<',
            ']' => '>',
            '<' => '[',
            '>' => ']',
            c => c,
        })
        .collect()
}

/// org-modify-ts-extra.
pub fn modify_ts_extra(s: &str, pos: usize, n: i64, step: i64) -> String {
    const IDX: [(&str, i64); 6] = [("d", 0), ("w", 1), ("m", 2), ("y", 3), ("d", -1), ("y", 4)];
    let Some(c) =
        re!(r"(-([012][0-9]):([0-5][0-9]))?( +\+([0-9]+)([dmwy]))?( +-([0-9]+)([dmwy]))?")
            .captures(s)
    else {
        return s.to_owned();
    };
    let inr = |i: usize| c.get(i).is_some_and(|g| g.start() <= pos && pos <= g.end());
    let cycle = |i: usize| {
        let cur = IDX.iter().find(|(u, _)| *u == &c[i]).map_or(0, |x| x.1);
        IDX.iter()
            .find(|(_, v)| *v == cur + n)
            .map_or("", |x| x.0)
            .to_owned()
    };
    let num = |i: usize| c[i].parse::<i64>().unwrap_or(0);
    let (group, new) = if inr(2) || inr(3) {
        let (mut hour, mut minute) = (num(2), num(3));
        if inr(2) {
            hour += n;
        } else {
            let n = step * n;
            let rem = minute % step;
            if rem != 0 {
                minute += if n > 0 { -rem } else { step - rem };
            }
            minute += n;
        }
        if minute < 0 {
            minute += 60;
            hour -= 1;
        }
        if minute > 59 {
            minute -= 60;
            hour += 1;
        }
        (1, format!("-{:02}:{minute:02}", hour.rem_euclid(24)))
    } else if inr(6) {
        (6, cycle(6))
    } else if inr(5) {
        (5, (n + num(5)).max(1).to_string())
    } else if inr(9) {
        (9, cycle(9))
    } else if inr(8) {
        (8, (n + num(8)).max(0).to_string())
    } else {
        return s.to_owned();
    };
    let g = c.get(group).unwrap();
    format!("{}{new}{}", &s[..g.start()], &s[g.end()..])
}

/// Options of [`change`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ChangeOpts {
    /// S-up/S-down: round minutes to org-timestamp-rounding-minutes.
    pub updown: bool,
    /// A prefix argument was given (`current-prefix-arg`).
    pub prefix: bool,
    /// Drop `--Nd` delays (repeating a timestamp).
    pub suppress_tmp_delay: bool,
}

/// The second element of org-timestamp-rounding-minutes.
pub fn rounding_minutes() -> (i64, i64) {
    match super::sexp::option("org-timestamp-rounding-minutes")
        .or_else(|| super::sexp::option("org-time-stamp-rounding-minutes"))
    {
        Some(super::sexp::Sexp::Int(i)) => (i, i),
        Some(v) => {
            let l = v.list().unwrap_or(&[]);
            (
                l.first().and_then(|x| x.int()).unwrap_or(0),
                l.get(1).and_then(|x| x.int()).unwrap_or(5),
            )
        }
        None => (0, 5),
    }
}

/// org-timestamp-change on the timestamp text `ts` (a match of
/// org-ts-regexp3), point at byte `origin` in it, its part `part`.
pub fn change(
    ts: &str,
    origin: usize,
    part: Part,
    n: i64,
    what: Option<What>,
    o: ChangeOpts,
) -> Result<Changed, String> {
    if what.is_none() && part == Part::Bracket {
        return Ok(Changed {
            text: toggle_type(ts),
            point: origin,
            toggled: true,
        });
    }
    let mut dm = rounding_minutes().1.max(1);
    let inactive = ts.starts_with('[');
    let mut extra =
        re!(r"((-[012][0-9]:[0-5][0-9])?( +[.+]?-?[-+][0-9]+[hdwmy](/[0-9]+[hdwmy])?)*)[\]>]")
            .captures(ts)
            .map(|c| c[1].to_owned())
            .unwrap_or_default();
    if o.suppress_tmp_delay {
        extra = re!(r" --[0-9]+[hdwmy]")
            .replace_all(&extra, "")
            .into_owned();
    }
    let with_hm = re!(r"^.{10}.*?[0-9]+:[0-9][0-9]").is_match(ts);
    let mut time0 = time_string_to_tm(ts)?;
    let unit = match what {
        Some(What::Unit(u)) => Some(u),
        Some(What::Calendar(_)) => None,
        None => match part {
            Part::Minute => Some(Unit::Minute),
            Part::Hour => Some(Unit::Hour),
            Part::Day => Some(Unit::Day),
            Part::Month => Some(Unit::Month),
            Part::Year => Some(Unit::Year),
            _ => None,
        },
    };
    let mut increment = n;
    if o.updown && unit == Some(Unit::Minute) && !o.prefix {
        increment = dm * n.signum();
        let rem = time0.min % dm;
        if rem != 0 {
            time0.min += if n > 0 { -rem } else { dm - rem };
        }
    } else {
        dm = 1;
    }
    let mut time = match unit {
        Some(u) => time0.add(u, increment),
        None => time0,
    };
    // encode-time then decode: DST gaps move wall times.
    time = Tm::from_epoch(time.epoch());
    if with_hm && matches!(unit, Some(Unit::Hour | Unit::Minute)) && n != 0 {
        let (a, b) = (time_string_to_tm(ts)?.epoch(), time.epoch());
        if !(if n > 0 { a < b } else { b < a }) {
            return Err(format!(
                "Cannot shift {ts} into the DST gap (according to current timezone '{}')",
                civil::zone_name(time_string_to_tm(ts)?)
            ));
        }
    }
    if matches!(unit, Some(Unit::Hour | Unit::Minute))
        && re!(r"-([012][0-9]):([0-5][0-9])").is_match(&extra)
    {
        extra = modify_ts_extra(&extra, if unit == Some(Unit::Hour) { 2 } else { 5 }, n, dm);
    }
    if let Part::Extra(k) = part
        && what.is_none()
    {
        extra = modify_ts_extra(&extra, k, n, dm);
    }
    if let Some(What::Calendar(d)) = what {
        time = Tm {
            year: d.year,
            month: d.month,
            day: d.day,
            ..time0
        };
    }
    let text = stamp_text(time, with_hm, inactive, &extra);
    let point = match ts3().captures(&text) {
        Some(c) if c.get(0).unwrap().start() == 0 => {
            let end = |i: usize| c.get(i).map(|g| g.end());
            match part {
                Part::Day => {
                    let p = if let Some(e5) = end(5) {
                        e5 - 1
                    } else if let Some(s7) = c.get(7).map(|g| g.start()) {
                        s7
                    } else {
                        end(1).unwrap_or(1) - 1
                    };
                    p.min(origin)
                }
                Part::Hour => end(7).unwrap_or(origin).min(origin),
                Part::Minute => end(8).map_or(origin, |e| e - 1).min(origin),
                Part::Extra(_) => (text.len() - 1).min(origin),
                Part::After => text.len(),
                _ => origin,
            }
        }
        _ => origin.min(text.len()),
    };
    Ok(Changed {
        text,
        point,
        toggled: false,
    })
}

/// org-display-custom-time: the display of a timestamp's inside (no
/// brackets) under org-timestamp-custom-formats.
pub fn custom_display(inner: &str) -> String {
    let Some(t1) = parse_time_string(inner) else {
        return inner.to_owned();
    };
    let off = re!(r"(-[0-9]+:[0-9]+)?( [.+]?\+[0-9]+[hdwmy](/[0-9]+[hdwmy])?)?$")
        .find(inner)
        .map_or(0, |m| m.len());
    let with_hm = t1.hour.is_some() && t1.min.is_some();
    let fmt = time_stamp_format(with_hm, Brackets::None, true);
    format!(
        "{}{}",
        civil::format(&fmt, t1.tm()),
        &inner[inner.len() - off..]
    )
}

/// The custom-time display of a line (org-activate-dates): byte ranges
/// of timestamp insides and the text shown instead.
pub fn custom_overlays(line: &str) -> Vec<(Range<usize>, String)> {
    let mut out = vec![];
    for c in tsr_both().captures_iter(line) {
        let m = c.get(0).unwrap();
        if m.start() > 0 && line.as_bytes()[m.start() - 1] == b'[' {
            continue;
        }
        for i in [1, 3] {
            if let Some(g) = c.get(i) {
                out.push((g.range(), custom_display(g.as_str())));
            }
        }
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}
