//! The date/time prompt language: org-read-date-analyze,
//! org-read-date-get-relative and Emacs `parse-time-string`
//! (iso8601-parse first, then the parse-time-rules tokenizer).

use super::civil::{self, Tm};
use super::stamp::{plain_time_of_day, small_year_to_year};

const MONTHS: [(&str, i64); 23] = [
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
];
/// parse-time-weekdays.
pub const WEEKDAYS: [(&str, i64); 14] = [
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
    ("sunday", 0),
    ("monday", 1),
    ("tuesday", 2),
    ("wednesday", 3),
    ("thursday", 4),
    ("friday", 5),
    ("saturday", 6),
];
const ZONES: [&str; 13] = [
    "z", "ut", "gmt", "pst", "pdt", "mst", "mdt", "cst", "cdt", "est", "edt", "", "",
];

/// `parse-time-string`'s decoded fields (None when not given).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub sec: Option<i64>,
    pub min: Option<i64>,
    pub hour: Option<i64>,
    pub day: Option<i64>,
    pub month: Option<i64>,
    pub year: Option<i64>,
    pub wday: Option<i64>,
}

fn num(s: &str) -> i64 {
    s.parse().unwrap_or(0)
}

/// iso8601-parse for whole-string dates (YYYY, YYYY-MM, YYYY-MM-DD,
/// YYYYMMDD, YYYY-DDD, --MM-DD) with an optional `T`/space time.
fn iso8601(s: &str) -> Option<Fields> {
    let c = re!(
        r"^(?:(?P<y>[0-9]{4})(?:-(?P<m>[0-9]{2})(?:-(?P<d>[0-9]{2}))?|(?P<m2>[0-9]{2})(?P<d2>[0-9]{2})|-(?P<o>[0-9]{3}))?|--(?P<m3>[0-9]{2})-?(?P<d3>[0-9]{2}))(?:[T ](?P<H>[0-9]{2}):?(?P<M>[0-9]{2})(?::?(?P<S>[0-9]{2}))?(?:Z|[+-][0-9]{2}(?::?[0-9]{2})?)?)?$"
    )
    .captures(s)?;
    let g = |n: &str| c.name(n).map(|m| num(m.as_str()));
    let mut f = Fields {
        year: g("y"),
        month: g("m").or(g("m2")).or(g("m3")),
        day: g("d").or(g("d2")).or(g("d3")),
        ..Fields::default()
    };
    if let (Some(o), Some(y)) = (g("o"), f.year) {
        let t = Tm::from_days(civil::days_from_civil(y, 1, 1) + o - 1);
        f.month = Some(t.month);
        f.day = Some(t.day);
    }
    if let Some(h) = g("H") {
        f.hour = Some(h);
        f.min = g("M");
        f.sec = Some(g("S").unwrap_or(0));
    }
    Some(f)
}

/// Emacs `parse-time-string`.
pub fn parse_time_string(s: &str) -> Fields {
    if let Some(f) = iso8601(s) {
        return f;
    }
    let s = s.to_lowercase();
    let mut f = Fields::default();
    for tok in tokenize(&s) {
        let n: Option<i64> = tok
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| tok.parse().unwrap_or(i64::MAX));
        let len = tok.len();
        let b = tok.as_bytes();
        let sub = |a: usize, z: usize| num(&tok[a..z]);
        // parse-time-rules, in order; the first rule whose slot is free
        // and whose predicate holds takes the token.
        if f.wday.is_none()
            && let Some(&(_, w)) = WEEKDAYS.iter().find(|(k, _)| *k == tok)
        {
            f.wday = Some(w);
        } else if f.day.is_none() && n.is_some_and(|n| (1..=31).contains(&n)) {
            f.day = n;
        } else if f.month.is_none()
            && let Some(&(_, m)) = MONTHS.iter().find(|(k, _)| *k == tok)
        {
            f.month = Some(m);
        } else if f.year.is_none() && n.is_some_and(|n| n >= 100) {
            f.year = n;
        } else if f.hour.is_none() && n.is_none() && len == 8 && b[2] == b':' && b[5] == b':' {
            (f.hour, f.min, f.sec) = (Some(sub(0, 2)), Some(sub(3, 5)), Some(sub(6, 8)));
        } else if ZONES[..11].contains(&tok.as_str()) {
            // A zone: ignored here.
        } else if n.is_none() && len == 5 && (b[0] == b'+' || b[0] == b'-') {
            // A numeric zone.
        } else if f.year.is_none() && n.is_none() && len == 10 && b[4] == b'-' && b[7] == b'-' {
            (f.year, f.month, f.day) = (Some(sub(0, 4)), Some(sub(5, 7)), Some(sub(8, 10)));
        } else if f.hour.is_none() && n.is_none() && len == 5 && b[2] == b':' {
            (f.hour, f.min, f.sec) = (Some(sub(0, 2)), Some(sub(3, 5)), Some(0));
        } else if f.hour.is_none() && n.is_none() && len == 4 && b[1] == b':' {
            (f.hour, f.min, f.sec) = (Some(sub(0, 1)), Some(sub(2, 4)), Some(0));
        } else if f.hour.is_none() && n.is_none() && len == 7 && b[1] == b':' {
            (f.hour, f.min, f.sec) = (Some(sub(0, 1)), Some(sub(2, 4)), Some(sub(5, 7)));
        } else if f.year.is_none() && n.is_some_and(|n| (50..=110).contains(&n)) {
            f.year = n.map(|n| n + 1900);
        } else if f.year.is_none() && n.is_some_and(|n| (0..=49).contains(&n)) {
            f.year = n.map(|n| n + 2000);
        }
    }
    f
}

/// parse-time-tokenize: runs of lowercase letters, digits, `+ - :`.
fn tokenize(s: &str) -> Vec<String> {
    let valid = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-:".contains(c);
    s.split(|c: char| !valid(c))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// org-read-date-get-relative: (shift, unit, relative-to-default) and the
/// matched length.
pub fn get_relative(s: &str, today: Tm, default: Tm) -> Option<(i64, String, bool, usize)> {
    let c = re!(
        r"(?i)^[ \t]*([-+]{0,2})([0-9]+)?([hdwmy]|(sun|mon|tue|wed|thu|fri|sat|sunday|monday|tuesday|wednesday|thursday|friday|saturday))?([ \t]|$)"
    )
    .captures(s)?;
    let sign = &c[1];
    if sign.is_empty() && c.get(4).is_none() {
        return None;
    }
    let dir = sign.chars().last().unwrap_or('+');
    let rel = sign.len() == 2;
    let n = c.get(2).map_or(1, |m| num(m.as_str()));
    let what = c.get(3).map_or("d".to_owned(), |m| m.as_str().to_owned());
    let len = c.get(0).unwrap().end();
    let date = if rel { default } else { today };
    if let Some(&(_, wday1)) = WEEKDAYS.iter().find(|(k, _)| *k == what.to_lowercase()) {
        let wday = date.weekday();
        let mut delta = (7 + wday1 - wday).rem_euclid(7);
        if delta == 0 {
            delta = 7;
        }
        if dir == '-' {
            delta -= 7;
            if delta == 0 {
                delta = -7;
            }
        }
        if n > 1 {
            delta += (n - 1) * if dir == '-' { -7 } else { 7 };
        }
        return Some((delta, "d".into(), rel, len));
    }
    Some((n * if dir == '-' { -1 } else { 1 }, what, rel, len))
}

/// The result of org-read-date-analyze.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    /// The date, not normalized (day 34 rolls over on use).
    pub tm: Tm,
    /// org-time-was-given.
    pub time_given: bool,
    /// org-end-time-was-given ("13:15").
    pub end_time: Option<String>,
    /// org-read-date-analyze-futurep.
    pub future: bool,
    /// org-read-date-analyze-forced-year.
    pub forced_year: bool,
}

fn replace(ans: &str, r: std::ops::Range<usize>, with: &str) -> String {
    format!("{}{with}{}", &ans[..r.start], &ans[r.end..])
}

/// org-read-date-analyze: ANS against the default time `def` (decoded),
/// with "now" from [`civil::now`].
pub fn analyze(ans: &str, def: Tm) -> Analysis {
    let now = civil::now();
    let mut ans = ans.to_owned();
    if re!(r"^[ \t]*\.[ \t]*$").is_match(&ans) {
        ans = "+0".into();
    }
    let mut delta = None;
    if let Some((n, w, rel, len)) = get_relative(&ans, now, def) {
        ans = ans[len..].to_owned();
        delta = Some((n, w, rel));
    }
    let mut iso = None;
    if let Some(c) = re!(r"\b(?:([0-9]+)-)?[wW]([0-9]{1,2})(?:-([0-6]))?([ \t]|$)").captures(&ans) {
        iso = Some((
            c.get(1).map(|m| small_year_to_year(num(m.as_str()))),
            c.get(3).map(|m| num(m.as_str())),
            num(&c[2]),
        ));
        ans = replace(&ans, c.get(0).unwrap().range(), "");
    }
    let mut kill_year = false;
    if let Some(c) = re!(r"^ *(([0-9]+)-)?([0-1]?[0-9])-([0-3]?[0-9])([^-0-9]|$)").captures(&ans) {
        let year = match c.get(2) {
            Some(y) => num(y.as_str()),
            None => {
                kill_year = true;
                now.year
            }
        };
        let year = small_year_to_year(year);
        let with = format!("{year:04}-{:02}-{:02}{}", num(&c[3]), num(&c[4]), &c[5]);
        ans = replace(&ans, c.get(0).unwrap().range(), &with);
    }
    if let Some(c) =
        re!(r"^ *(3[01]|0?[1-9]|[12][0-9])\. ?(0?[1-9]|1[012])\.( ?[1-9][0-9]{3})?").captures(&ans)
    {
        let year = match c.get(3) {
            Some(y) => num(y.as_str().trim()),
            None => {
                kill_year = true;
                now.year
            }
        };
        let with = format!("{year:04}-{:02}-{:02}", num(&c[2]), num(&c[1]));
        ans = replace(&ans, c.get(0).unwrap().range(), &with);
    }
    if let Some(c) =
        re!(r"^ *(0?[1-9]|1[012])/(0?[1-9]|[12][0-9]|3[01])(/([0-9]+))?([^/0-9]|$)").captures(&ans)
    {
        let year = match c.get(4) {
            Some(y) => num(y.as_str()),
            None => {
                kill_year = true;
                now.year
            }
        };
        let year = small_year_to_year(year);
        let with = format!("{year:04}-{:02}-{:02}{}", num(&c[1]), num(&c[2]), &c[5]);
        ans = replace(&ans, c.get(0).unwrap().range(), &with);
    }
    let plain_time = re!(r"(?:^|[^+])[012]?[0-9]:[0-9][0-9](?:[ \t\n]|$)");
    for _ in 0..2 {
        if plain_time.is_match(&ans) {
            break;
        }
        let Some(c) = re!(r"([012]?[0-9])(:([0-5][0-9]))?(am|AM|pm|PM)\b").captures(&ans) else {
            break;
        };
        let mut hour = num(&c[1]);
        let minute = c.get(3).map_or(0, |m| num(m.as_str()));
        let pm = c[4].eq_ignore_ascii_case("pm");
        if hour == 12 && !pm {
            hour = 0;
        } else if pm && hour < 12 {
            hour += 12;
        }
        ans = replace(
            &ans,
            c.get(0).unwrap().range(),
            &format!("{hour:02}:{minute:02}"),
        );
    }
    for _ in 0..2 {
        if plain_time.is_match(&ans) {
            break;
        }
        let Some(c) = re!(r"(?:([012]?[0-9])?h([0-5][0-9]))|(?:([012]?[0-9])h([0-5][0-9])?\b)")
            .captures(&ans)
        else {
            break;
        };
        let hour = c.get(1).or(c.get(3)).map_or(0, |m| num(m.as_str()));
        let minute = c.get(2).or(c.get(4)).map_or(0, |m| num(m.as_str()));
        ans = replace(
            &ans,
            c.get(0).unwrap().range(),
            &format!("{hour:02}:{minute:02}"),
        );
    }
    if let Some(c) =
        re!(r"([012]?[0-9]):([0-6][0-9])\+([012]?[0-9])(:([0-5][0-9]))?").captures(&ans)
    {
        let hour = num(&c[1]);
        let minute = num(&c[2]);
        let mut h2 = hour + num(&c[3]);
        let mut m2 = minute + c.get(5).map_or(0, |m| num(m.as_str()));
        if m2 >= 60 {
            h2 += 1;
            m2 -= 60;
        }
        ans = replace(
            &ans,
            c.get(0).unwrap().range(),
            &format!("{hour:02}:{minute:02}-{h2:02}:{m2:02}"),
        );
    }
    let mut end_time = None;
    if let Some(c) = plain_time_of_day().captures(&ans)
        && let Some(e) = c.get(8)
    {
        end_time = Some(e.as_str().to_owned());
        ans = replace(&ans, c.get(7).unwrap().range(), "");
    }
    let tl = parse_time_string(&ans);
    let prefer = super::options::string("org-read-date-prefer-future", "t");
    let prefer_future = prefer != "nil";
    let mut futurep = false;
    let mut day = tl.day.unwrap_or(def.day);
    let month = match (tl.month, tl.day) {
        (Some(m), _) => m,
        _ if !prefer_future => def.month,
        (None, Some(_)) => {
            futurep = true;
            if day < now.day {
                now.month + 1
            } else {
                now.month
            }
        }
        _ => def.month,
    };
    let year = if let (false, Some(y)) = (kill_year, tl.year) {
        y
    } else if !prefer_future {
        def.year
    } else if futurep {
        if month > now.month || day >= now.day {
            now.year
        } else {
            now.year + 1
        }
    } else if tl.month.is_some() {
        futurep = true;
        if month > now.month {
            now.year
        } else if month < now.month || day < now.day {
            now.year + 1
        } else {
            now.year
        }
    } else {
        def.year
    };
    let mut hour = tl.hour.unwrap_or(def.hour);
    let minute = tl.min.unwrap_or(def.min);
    let second = tl.sec.unwrap_or(0);
    let wday = tl.wday;
    let (mut month, mut year) = (month, year);
    if prefer == "time"
        && tl.day.is_none()
        && tl.month.is_none()
        && tl.year.is_none()
        && (day, month, year) == (now.day, now.month, now.year)
        && let Some(h) = tl.hour
        && (h < now.hour || (h == now.hour && tl.min.is_some_and(|m| m < now.min)))
    {
        day += 1;
        futurep = true;
    }
    let mut time_given = false;
    if let Some((iso_year, iso_wday, iso_week)) = iso {
        futurep = false;
        let y = iso_year.unwrap_or(year);
        let d = iso_wday.or(wday).unwrap_or(1);
        let jan4 = Tm::date(y, 1, 4).absolute();
        let monday = jan4 - (jan4 - 1).rem_euclid(7);
        let abs = monday + 7 * (iso_week - 1) + if d == 0 { 6 } else { d - 1 };
        let t = Tm::from_absolute(abs);
        (year, month, day) = (t.year, t.month, t.day);
    } else if let Some((n, w, rel)) = delta {
        futurep = false;
        if !rel {
            (day, month, year) = (now.day, now.month, now.year);
        }
        match w.as_str() {
            "h" | "" => {
                time_given = true;
                hour += n;
            }
            "d" => day += n,
            "w" => day += 7 * n,
            "m" => month += n,
            "y" => year += n,
            _ => {}
        }
    } else if let (Some(w), None) = (wday, tl.day) {
        let wday1 = Tm::date(year, month, day).weekday();
        if w != wday1 {
            day += (w - wday1 + 7) % 7;
        }
    }
    if tl.hour.is_some() {
        time_given = true;
    }
    if year < 100 {
        year += 2000;
    }
    let mut forced_year = false;
    if super::options::bool("org-read-date-force-compatible-dates", true) {
        if year < 1970 {
            year = 1970;
            forced_year = true;
        }
        if year > 2037 {
            year = 2037;
            forced_year = true;
        }
    }
    Analysis {
        tm: Tm {
            year,
            month,
            day,
            hour,
            min: minute,
            sec: second,
        },
        time_given,
        end_time,
        future: futurep,
        forced_year,
    }
}

/// The default time of org-read-date: `default` or now, rounded by the
/// first org-timestamp-rounding-minutes (unless `exact`), with
/// org-extend-today-until moving early hours to yesterday 23:59.
pub fn default_time(default: Option<Tm>, exact: bool) -> Tm {
    if let Some(d) = default {
        return d.norm();
    }
    let mut t = current_time(
        if exact {
            0
        } else {
            super::stamp::rounding_minutes().0
        },
        false,
    );
    if t.hour < super::options::int("org-extend-today-until", 0) {
        t.hour = -1;
        t.min = 59;
        t = t.norm();
    }
    t
}

/// org-current-time: now rounded to `r` minutes (ties to even, as
/// Emacs `round`), into the past with `past`.
pub fn current_time(r: i64, past: bool) -> Tm {
    let now = civil::now();
    if r < 1 {
        return now;
    }
    let q = now.min as f64 / r as f64;
    let res = Tm {
        min: r * q.round_ties_even() as i64,
        sec: 0,
        ..now
    }
    .norm();
    if !past || res.epoch() < now.epoch() {
        res
    } else {
        Tm {
            min: r * now.min.div_euclid(r),
            sec: 0,
            ..now
        }
        .norm()
    }
}

/// org-read-date's string result for an answer.
pub fn date_string(a: &Analysis) -> String {
    let t = a.tm.norm();
    if a.time_given {
        civil::format("%Y-%m-%d %H:%M", t)
    } else {
        civil::format("%Y-%m-%d", t)
    }
}
