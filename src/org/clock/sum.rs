//! org-clock-sum and org-clock-special-range: pure functions over lines.

use super::super::syntax::{self, Lines};
use super::super::time::civil::{self, Tm};
use super::super::{options, time::stamp};
use std::collections::BTreeMap;

/// The result of org-clock-sum: minutes per heading line (the
/// `:org-clock-minutes` text property) and the file total
/// (org-clock-file-total-minutes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sum {
    pub minutes: BTreeMap<usize, i64>,
    pub total: i64,
}

/// Lines inside `#+BEGIN_X` ... `#+END_X` blocks of `lo..hi`: a CLOCK line
/// there is not a clock element.
pub fn block_mask<L: Lines + ?Sized>(b: &L, lo: usize, hi: usize) -> Vec<bool> {
    let begin = crate::org_re!(r"(?i)^[ \t]*#\+begin_(\S+)");
    let mut mask = vec![false; hi.saturating_sub(lo)];
    let mut i = lo;
    while i < hi {
        let t = b.line_text(i);
        if let Some(c) = begin.captures(&t) {
            let end = format!("#+end_{}", c[1].to_ascii_lowercase());
            let close = (i + 1..hi)
                .take_while(|&j| syntax::level(&b.line_text(j)).is_none())
                .find(|&j| b.line_text(j).trim().to_ascii_lowercase() == end);
            if let Some(e) = close {
                for m in &mut mask[i + 1 - lo..e - lo] {
                    *m = true;
                }
                i = e;
            }
        }
        i += 1;
    }
    mask
}

/// The seconds of a clock timestamp, which must have a time of day.
fn clock_seconds(ts: &str) -> Option<i64> {
    let p = stamp::parse_time_string(ts)?;
    Some(Tm::at(p.year, p.month, p.day, p.hour?, p.min?).epoch())
}

/// org-clock-sum on lines `lo..hi` (the accessible part of the buffer).
/// `tstart`/`tend` bound the counted time (seconds); `filter` is the
/// headline filter; `clocking` is the running clock's (heading line,
/// start) when it is in these lines.
pub fn sum<L: Lines + ?Sized>(
    b: &L,
    lo: usize,
    hi: usize,
    tstart: Option<f64>,
    tend: Option<f64>,
    filter: Option<&dyn Fn(usize) -> bool>,
    clocking: Option<(usize, i64)>,
) -> Sum {
    let clock_re =
        crate::org_re!(r"^[ \t]*CLOCK:[ \t]*(?:(\[.*?\])-+(\[.*?\])|=>[ \t]+([0-9]+):([0-9]+))");
    let hi = hi.min(b.n_lines());
    let mask = block_mask(b, lo, hi);
    let include_clocking = options::bool("org-clock-report-include-clocking-task", false);
    let mut ltimes: Vec<i64> = vec![0; 30];
    let mut forced = vec![false; hi.saturating_sub(lo)];
    let mut t1: i64 = 0;
    let mut out = Sum::default();
    for l in (lo..hi).rev() {
        let t = b.line_text(l);
        if let Some(level) = syntax::level(&t) {
            if include_clocking
                && let Some((h, start)) = clocking
                && h == l
                && let (Some(ts), Some(te)) = (tstart, tend)
                && start as f64 >= ts
                && start as f64 <= te
            {
                t1 += (civil::now_epoch() - start).div_euclid(60);
            }
            let included = filter.is_none_or(|f| f(l));
            if level >= ltimes.len() {
                ltimes.resize(level * 2, 0);
            }
            if t1 > 0 || ltimes[level] > 0 {
                if included || forced[l - lo] {
                    if included {
                        for x in &mut ltimes[..=level] {
                            *x += t1;
                        }
                    }
                    out.minutes.insert(l, ltimes[level]);
                    if filter.is_some() {
                        let mut p = l;
                        while let Some(up) = syntax::parent(b, p).filter(|&u| u >= lo) {
                            forced[up - lo] = true;
                            p = up;
                        }
                    }
                }
                t1 = 0;
                for x in &mut ltimes[level..] {
                    *x = 0;
                }
            }
            continue;
        }
        let Some(c) = clock_re.captures(&t) else {
            continue;
        };
        if let (Some(a), Some(z)) = (c.get(1), c.get(2)) {
            if mask[l - lo] {
                continue;
            }
            // org-clock-sum: Ignoring invalid clocks.
            let (Some(ts), Some(te)) = (clock_seconds(a.as_str()), clock_seconds(z.as_str()))
            else {
                continue;
            };
            let (ts, te) = (ts as f64, te as f64);
            let dt = tend.map_or(te, |e| te.min(e)) - tstart.map_or(ts, |s| ts.max(s));
            if dt > 0.0 {
                t1 += (dt / 60.0).floor() as i64;
            }
        } else if let (Some(h), Some(m)) = (c.get(3), c.get(4)) {
            t1 += m.as_str().parse::<i64>().unwrap_or(0)
                + 60 * h.as_str().parse::<i64>().unwrap_or(0);
        }
    }
    out.total = ltimes[0];
    out
}

/// The minutes clocked in the subtree at heading `h` (org-clock-sum on the
/// narrowed subtree), within `range` (seconds).
pub fn subtree_minutes<L: Lines + ?Sized>(
    b: &L,
    h: usize,
    range: Option<(f64, f64)>,
    clocking: Option<(usize, i64)>,
) -> i64 {
    let end = syntax::subtree_end(b, h);
    let s = sum(
        b,
        h,
        end,
        range.map(|r| r.0),
        range.map(|r| r.1),
        None,
        clocking,
    );
    s.minutes.get(&h).copied().unwrap_or(0)
}

/// A time range from org-clock-special-range.
#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub start: Option<Tm>,
    pub end: Tm,
    pub text: String,
}

impl Range {
    /// The range as seconds (untilnow starts at 0).
    pub fn seconds(&self) -> (f64, f64) {
        (
            self.start.map_or(0.0, |s| s.epoch() as f64),
            self.end.epoch() as f64,
        )
    }

    /// As org-clock-special-range AS-STRINGS: `<2026-10-04 Sun 00:00>`.
    pub fn strings(&self) -> (Option<String>, String) {
        let f = stamp::time_stamp_format(true, stamp::Brackets::Active, false);
        (
            self.start.map(|s| civil::format(&f, s)),
            civil::format(&f, self.end),
        )
    }
}

/// calendar-iso-to-absolute: (week day year), day 0 = Sunday.
pub fn iso_to_absolute(week: i64, day: i64, year: i64) -> i64 {
    let jan4 = Tm::date(year, 1, 4).absolute();
    // calendar-dayname-on-or-before Monday.
    let monday = jan4 - (jan4 - 1).rem_euclid(7);
    monday + 7 * (week - 1) + if day == 0 { 6 } else { day - 1 }
}

/// org-day-of-week.
fn day_of_week(day: i64, month: i64, year: i64) -> i64 {
    Tm::date(year, month, day).weekday()
}

/// org-quarter-to-date: (week day year) of a quarter's first day.
pub fn quarter_to_date(quarter: i64, year: i64) -> (i64, i64, i64) {
    match quarter {
        1 => {
            let s = day_of_week(1, 1, year);
            match s {
                0 => (52, 7, year - 1),
                6 => (52, 6, year - 1),
                s if s <= 4 => (1, s, year),
                s => (53, s, year - 1),
            }
        }
        2 => {
            let s = day_of_week(1, 4, year);
            (if s == 0 || s >= 4 { 13 } else { 14 }, s, year)
        }
        3 => {
            let s = day_of_week(1, 7, year);
            (if s == 0 || s >= 4 { 26 } else { 27 }, s, year)
        }
        _ => {
            let s = day_of_week(1, 10, year);
            (if s == 0 || s > 4 { 39 } else { 40 }, s, year)
        }
    }
}

fn count_quarter(n: i64) -> &'static str {
    match n {
        1 => "1st",
        2 => "2nd",
        3 => "3rd",
        _ => "4th",
    }
}

/// org-clock-special-range for KEY (a symbol or block string) relative to
/// `time`; `interactive` is answered by the caller (it prompts).
pub fn special_range(
    key: &str,
    time: Tm,
    wstart: Option<i64>,
    mstart: Option<i64>,
) -> Result<Range, String> {
    let extend = options::int("org-extend-today-until", 0);
    let tm = time.norm();
    let (mut m, mut h, mut d, mut month, mut y) = (tm.min, tm.hour, tm.day, tm.month, tm.year);
    let mut dow = tm.weekday();
    let mut shift = 0i64;
    let mut q = match month {
        10.. => 4,
        7.. => 3,
        4.. => 2,
        _ => 1,
    };
    let mut key = key.to_owned();
    let num = |s: &str| s.parse::<i64>().unwrap_or(0);
    if let Some(c) = crate::org_re!(r"^([0-9]+)$").captures(&key.clone()) {
        (y, month, d) = (num(&c[1]), 1, 1);
        key = "year".into();
    } else if let Some(c) = crate::org_re!(r"^([0-9]+)-([0-9]{1,2})$").captures(&key.clone()) {
        (y, month, d) = (num(&c[1]), num(&c[2]), 1);
        key = "month".into();
    } else if let Some(c) = crate::org_re!(r"^([0-9]+)-[wW]([0-9]{1,2})$").captures(&key.clone()) {
        let date = Tm::from_absolute(iso_to_absolute(num(&c[2]), 1, num(&c[1])));
        (d, month, y, dow) = (date.day, date.month, date.year, 1);
        key = "week".into();
    } else if let Some(c) = crate::org_re!(r"^([0-9]+)-[qQ]([1-4])$").captures(&key.clone()) {
        q = num(&c[2]);
        let (w, dd, yy) = quarter_to_date(q, num(&c[1]));
        let date = Tm::from_absolute(iso_to_absolute(w, dd, yy));
        (d, month, y, dow) = (date.day, date.month, date.year, 1);
        key = "quarter".into();
    } else if let Some(c) =
        crate::org_re!(r"^([0-9]+)-([0-9]{1,2})-([0-9]{1,2})$").captures(&key.clone())
    {
        (y, month, d) = (num(&c[1]), num(&c[2]), num(&c[3]));
        key = "day".into();
    } else if let Some(c) = crate::org_re!(r"([-+][0-9]+)$").captures(&key.clone()) {
        shift = num(c[1].trim_start_matches('+'));
        key = key[..c.get(1).unwrap().start()].to_owned();
        if (key == "quarter" || key == "thisq") && shift > 0 {
            return Err("Looking forward with quarters isn't implemented".into());
        }
    }
    if shift == 0 {
        let (k, s) = match key.as_str() {
            "yesterday" => ("today", -1),
            "lastweek" => ("week", -1),
            "lastmonth" => ("month", -1),
            "lastyear" => ("year", -1),
            "lastq" => ("quarter", -1),
            k => (k, 0),
        };
        key = k.to_owned();
        shift = s;
    }
    let (mut h1, mut d1, mut month1, mut y1) = (None, None, None, None);
    let mut shiftedq = q;
    let mut shiftedy = y;
    match key.as_str() {
        "day" | "today" => {
            m = 0;
            h = extend;
            h1 = Some(24 + extend);
            d += shift;
        }
        "week" | "thisweek" => {
            let ws = wstart.unwrap_or(1);
            let diff = -7 * shift + (dow + 7 - ws).rem_euclid(7);
            m = 0;
            h = extend;
            d -= diff;
            d1 = Some(d + 7);
        }
        "month" | "thismonth" => {
            h = extend;
            m = 0;
            d = mstart.unwrap_or(1);
            month += shift;
            month1 = Some(month + 1);
        }
        "quarter" | "thisq" => {
            if q - 1 + shift < 0 {
                let interval = -(q - 1 + shift);
                let (years, quarters) = (interval.div_euclid(4), interval.rem_euclid(4));
                if quarters == 0 {
                    shiftedy = y - years;
                    month = 1;
                    shiftedq = 1;
                } else {
                    shiftedy = y - (1 + years);
                    month = 13 - 3 * quarters;
                    shiftedq = 5 - quarters;
                }
                m = 0;
                h = extend;
                d = 1;
                month1 = Some(month + 3);
                y = shiftedy;
            } else if q + shift > 0 {
                shiftedq = q + shift;
                shiftedy = y;
                let qshift = 3 * (q + shift - 1);
                m = 0;
                h = extend;
                d = 1;
                month = 1 + qshift;
                month1 = Some(4 + qshift);
            }
        }
        "year" | "thisyear" => {
            m = 0;
            h = extend;
            d = 1;
            month = 1;
            y += shift;
            y1 = Some(y + 1);
        }
        "untilnow" => {}
        "interactive" => return Err("interactive range needs a prompt".into()),
        k => return Err(format!("No such time block {k}")),
    }
    let start = (key != "untilnow").then(|| Tm::at(y, month, d, h, m).norm());
    let end = if key == "untilnow" {
        civil::now()
    } else {
        Tm::at(
            y1.unwrap_or(y),
            month1.unwrap_or(month),
            d1.unwrap_or(d),
            h1.unwrap_or(h),
            m,
        )
        .norm()
    };
    let s = start.unwrap_or_default();
    let text = match key.as_str() {
        "day" | "today" => civil::format("%A, %B %d, %Y", s),
        "week" | "thisweek" => civil::format("week %G-W%V", s),
        "month" | "thismonth" => civil::format("%B %Y", s),
        "year" | "thisyear" => civil::format("the year %Y", s),
        "quarter" | "thisq" => format!("{} quarter of {shiftedy}", count_quarter(shiftedq)),
        _ => "now".into(),
    };
    Ok(Range { start, end, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    #[test]
    fn sums_clock_lines_per_heading() {
        let b = v("* This is test
CLOCK: [2025-10-18 Sat 09:00]--[2025-10-18 Sat 10:00] =>  1:00
*************** Here
:LOGBOOK:
CLOCK: [2025-10-18 Sat 13:00]
CLOCK: [2025-10-18 Sat 10:00]--[2025-10-18 Sat 11:00] =>  1:00
CLOCK: [2025-10-18 Sat 14:43]
:END:
*************** END");
        let s = sum(&b, 0, b.len(), None, None, None, None);
        assert_eq!(s.total, 120);
        assert_eq!(s.minutes.get(&0), Some(&120));
        assert_eq!(s.minutes.get(&2), Some(&60));
        // Clocks inside blocks are not clocks; naked times count.
        let b = v("* T
CLOCK: [2025-10-18 Sat 09:00]--[2025-10-18 Sat 10:00] =>  1:00
#+begin_src org
CLOCK: [2025-10-18 Sat 10:00]--[2025-10-18 Sat 11:00] =>  1:00
#+end_src
CLOCK: => 0:30
CLOCK: [2012-01-01 sun. 00rr:04]--[2012-01-01 sun. 00:05] =>  0:01");
        assert_eq!(sum(&b, 0, b.len(), None, None, None, None).total, 90);
    }

    #[test]
    fn special_ranges() {
        let t = Tm::at(2023, 4, 26, 15, 0);
        let r = special_range("thisweek", t, None, None).unwrap();
        assert_eq!(r.start, Some(Tm::date(2023, 4, 24)));
        assert_eq!(r.end, Tm::date(2023, 5, 1));
        assert_eq!(r.text, "week 2023-W17");
        let r = special_range("thisweek", t, Some(6), None).unwrap();
        assert_eq!(r.start, Some(Tm::date(2023, 4, 22)));
        let r = special_range("2017-W40", t, None, None).unwrap();
        assert_eq!(r.start, Some(Tm::date(2017, 10, 2)));
        let r = special_range("2014-Q2", t, None, None).unwrap();
        assert_eq!(
            (r.start, r.end),
            (Some(Tm::date(2014, 4, 1)), Tm::date(2014, 7, 1))
        );
        assert_eq!(r.text, "2nd quarter of 2014");
        let r = special_range("lastq", Tm::date(2014, 2, 3), None, None).unwrap();
        assert_eq!(r.start, Some(Tm::date(2013, 10, 1)));
        let r = special_range("today-1", t, None, None).unwrap();
        assert_eq!(r.text, "Tuesday, April 25, 2023");
        let r = special_range("2017-09", t, None, None).unwrap();
        assert_eq!(r.end, Tm::date(2017, 10, 1));
        assert!(special_range("nope", t, None, None).is_err());
    }
}
