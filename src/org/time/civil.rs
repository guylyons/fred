//! Civil time: Gregorian arithmetic on decoded times, the local time zone
//! (libc `mktime`/`localtime_r`), `format-time-string`, and an injectable
//! "now" (`set_now`) so tests pin `current-time` like `org-test-at-time`.

use std::cell::Cell;

pub const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Calendar absolute day number of 1970-01-01 (day 1 is 0001-01-01).
pub const ABS_1970: i64 = 719_163;

/// A decoded local time. Fields may be out of range until [`Tm::norm`],
/// as with Emacs `encode-time` (day 34, month 13...).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tm {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub min: i64,
    pub sec: i64,
}

/// Units of org-timestamp-change and repeaters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl Unit {
    /// `h d w m y` (and `min`).
    pub fn parse(s: &str) -> Option<Unit> {
        Some(match s {
            "h" => Unit::Hour,
            "d" => Unit::Day,
            "w" => Unit::Week,
            "m" => Unit::Month,
            "y" => Unit::Year,
            "min" => Unit::Minute,
            _ => return None,
        })
    }

    pub fn letter(self) -> &'static str {
        match self {
            Unit::Minute => "min",
            Unit::Hour => "h",
            Unit::Day => "d",
            Unit::Week => "w",
            Unit::Month => "m",
            Unit::Year => "y",
        }
    }
}

pub fn leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn month_days(y: i64, m: i64) -> i64 {
    match m {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a valid date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

impl Tm {
    pub fn date(year: i64, month: i64, day: i64) -> Tm {
        Tm {
            year,
            month,
            day,
            ..Tm::default()
        }
    }

    pub fn at(year: i64, month: i64, day: i64, hour: i64, min: i64) -> Tm {
        Tm {
            year,
            month,
            day,
            hour,
            min,
            sec: 0,
        }
    }

    /// Normalize out-of-range fields (mktime without the time zone).
    pub fn norm(self) -> Tm {
        let mut min = self.min + self.sec.div_euclid(60);
        let sec = self.sec.rem_euclid(60);
        let mut hour = self.hour + min.div_euclid(60);
        min = min.rem_euclid(60);
        let dcarry = hour.div_euclid(24);
        hour = hour.rem_euclid(24);
        let months = self.year * 12 + self.month - 1;
        let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
        let days = days_from_civil(y, m, 1) + self.day - 1 + dcarry;
        let (year, month, day) = civil_from_days(days);
        Tm {
            year,
            month,
            day,
            hour,
            min,
            sec,
        }
    }

    /// Days since 1970-01-01 (`time-to-days` minus [`ABS_1970`]).
    pub fn days(self) -> i64 {
        let t = self.norm();
        days_from_civil(t.year, t.month, t.day)
    }

    /// Calendar absolute day number (`time-to-days`).
    pub fn absolute(self) -> i64 {
        self.days() + ABS_1970
    }

    pub fn from_days(d: i64) -> Tm {
        let (y, m, dd) = civil_from_days(d);
        Tm::date(y, m, dd)
    }

    pub fn from_absolute(a: i64) -> Tm {
        Tm::from_days(a - ABS_1970)
    }

    /// 0 = Sunday.
    pub fn weekday(self) -> i64 {
        (self.days() + 4).rem_euclid(7)
    }

    /// Seconds from the epoch in the local time zone (DST guessed, as
    /// `org-encode-time` with DST -1).
    pub fn epoch(self) -> i64 {
        let t = self.norm();
        // SAFETY: plain libc calls on an owned, zeroed struct.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            tm.tm_year = (t.year - 1900) as i32;
            tm.tm_mon = (t.month - 1) as i32;
            tm.tm_mday = t.day as i32;
            tm.tm_hour = t.hour as i32;
            tm.tm_min = t.min as i32;
            tm.tm_sec = t.sec as i32;
            tm.tm_isdst = -1;
            libc::mktime(&mut tm) as i64
        }
    }

    /// Local decoded time of epoch seconds.
    pub fn from_epoch(e: i64) -> Tm {
        local(e).0
    }

    /// `decoded-time-add` of N units: months and years clamp the day to
    /// the month's length; days, hours and minutes carry.
    pub fn add(self, unit: Unit, n: i64) -> Tm {
        let mut t = self;
        match unit {
            Unit::Year => t.year += n,
            Unit::Month => {
                let m = t.month - 1 + n;
                t.month = m.rem_euclid(12) + 1;
                t.year += m.div_euclid(12);
            }
            _ => {}
        }
        t.day = t.day.min(month_days(t.year, t.month));
        match unit {
            Unit::Day => t.day += n,
            Unit::Week => t.day += 7 * n,
            Unit::Hour => t.hour += n,
            Unit::Minute => t.min += n,
            _ => {}
        }
        t.norm()
    }

    /// A day earlier/later at midnight.
    pub fn midnight(self) -> Tm {
        Tm::date(self.year, self.month, self.day).norm()
    }
}

/// localtime_r: decoded time, UTC offset in seconds, zone abbreviation.
fn local(e: i64) -> (Tm, i64, String) {
    // SAFETY: localtime_r writes into the owned struct; tm_zone points to
    // static zone data when set.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t: libc::time_t = e as libc::time_t;
        libc::localtime_r(&t, &mut tm);
        let zone = if tm.tm_zone.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(tm.tm_zone)
                .to_string_lossy()
                .into_owned()
        };
        (
            Tm {
                year: tm.tm_year as i64 + 1900,
                month: tm.tm_mon as i64 + 1,
                day: tm.tm_mday as i64,
                hour: tm.tm_hour as i64,
                min: tm.tm_min as i64,
                sec: tm.tm_sec as i64,
            },
            tm.tm_gmtoff as i64,
            zone,
        )
    }
}

/// The time zone abbreviation in effect at `t` (`current-time-zone`).
pub fn zone_name(t: Tm) -> String {
    local(t.epoch()).2
}

thread_local! {
    static NOW: Cell<Option<Tm>> = const { Cell::new(None) };
}

/// Pin `current-time` (tests); `None` returns to the clock.
pub fn set_now(t: Option<Tm>) {
    NOW.with(|n| n.set(t.map(Tm::norm)));
}

/// `(decode-time)`: the current local time.
pub fn now() -> Tm {
    NOW.with(Cell::get)
        .unwrap_or_else(|| Tm::from_epoch(now_epoch()))
}

/// `(float-time)` truncated to seconds.
pub fn now_epoch() -> i64 {
    match NOW.with(Cell::get) {
        Some(t) => t.epoch(),
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
    }
}

/// ISO 8601 (week, weekday 1-7, year) of a date.
pub fn iso_week(t: Tm) -> (i64, i64, i64) {
    let a = t.absolute();
    let wd = (t.weekday() + 6) % 7 + 1;
    // The Thursday of this week decides the ISO year.
    let thu = Tm::from_absolute(a - wd + 4);
    let jan1 = Tm::date(thu.year, 1, 1).absolute();
    ((a - wd + 4 - jan1) / 7 + 1, wd, thu.year)
}

/// `format-time-string` for the directives Org uses (English names).
pub fn format(fmt: &str, t: Tm) -> String {
    let t = t.norm();
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(d) = it.next() else {
            out.push('%');
            break;
        };
        let h12 = if t.hour % 12 == 0 { 12 } else { t.hour % 12 };
        let yday = t.days() - days_from_civil(t.year, 1, 1) + 1;
        match d {
            'Y' => out.push_str(&format!("{:04}", t.year)),
            'y' => out.push_str(&format!("{:02}", t.year.rem_euclid(100))),
            'C' => out.push_str(&format!("{:02}", t.year.div_euclid(100))),
            'm' => out.push_str(&format!("{:02}", t.month)),
            'd' => out.push_str(&format!("{:02}", t.day)),
            'e' => out.push_str(&format!("{:2}", t.day)),
            'a' => out.push_str(&DAY_NAMES[t.weekday() as usize][..3]),
            'A' => out.push_str(DAY_NAMES[t.weekday() as usize]),
            'b' | 'h' => out.push_str(&MONTH_NAMES[t.month as usize - 1][..3]),
            'B' => out.push_str(MONTH_NAMES[t.month as usize - 1]),
            'H' => out.push_str(&format!("{:02}", t.hour)),
            'k' => out.push_str(&format!("{:2}", t.hour)),
            'I' => out.push_str(&format!("{h12:02}")),
            'l' => out.push_str(&format!("{h12:2}")),
            'M' => out.push_str(&format!("{:02}", t.min)),
            'S' => out.push_str(&format!("{:02}", t.sec)),
            'p' => out.push_str(if t.hour < 12 { "AM" } else { "PM" }),
            'P' => out.push_str(if t.hour < 12 { "am" } else { "pm" }),
            'j' => out.push_str(&format!("{yday:03}")),
            'u' => out.push_str(&((t.weekday() + 6) % 7 + 1).to_string()),
            'w' => out.push_str(&t.weekday().to_string()),
            'U' => out.push_str(&format!("{:02}", (yday + 6 - t.weekday()) / 7)),
            'W' => out.push_str(&format!("{:02}", (yday + 6 - (t.weekday() + 6) % 7) / 7)),
            'V' => out.push_str(&format!("{:02}", iso_week(t).0)),
            'G' => out.push_str(&iso_week(t).2.to_string()),
            'F' => out.push_str(&format("%Y-%m-%d", t)),
            'R' => out.push_str(&format("%H:%M", t)),
            'T' => out.push_str(&format("%H:%M:%S", t)),
            'D' => out.push_str(&format("%m/%d/%y", t)),
            's' => out.push_str(&t.epoch().to_string()),
            'Z' => out.push_str(&local(t.epoch()).2),
            'z' => {
                let off = local(t.epoch()).1;
                let sign = if off < 0 { '-' } else { '+' };
                out.push_str(&format!(
                    "{sign}{:02}{:02}",
                    off.abs() / 3600,
                    off.abs() % 3600 / 60
                ));
            }
            'n' => out.push('\n'),
            't' => out.push('\t'),
            '%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips_and_normalizes() {
        for d in [-800_000, -1, 0, 1, 19_000, 20_365, 2_000_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(Tm::date(1970, 1, 1).absolute(), ABS_1970);
        assert_eq!(Tm::date(2012, 2, 30).norm(), Tm::date(2012, 3, 1));
        assert_eq!(Tm::date(2012, 13, 0).norm(), Tm::date(2012, 12, 31));
        assert_eq!(Tm::at(2014, 3, 4, 25, -1).norm(), Tm::at(2014, 3, 5, 0, 59));
        assert_eq!(Tm::date(2026, 10, 4).weekday(), 0);
        assert_eq!(
            Tm::date(2026, 1, 31).add(Unit::Month, 1),
            Tm::date(2026, 2, 28)
        );
        assert_eq!(
            Tm::date(2025, 12, 31).add(Unit::Month, 2),
            Tm::date(2026, 2, 28)
        );
        assert_eq!(
            Tm::date(2024, 2, 29).add(Unit::Year, 1),
            Tm::date(2025, 2, 28)
        );
        assert_eq!(
            format("%Y-%m-%d %a %H:%M %j %V", Tm::at(2026, 1, 1, 9, 5)),
            "2026-01-01 Thu 09:05 001 01"
        );
        assert_eq!(iso_week(Tm::date(2021, 1, 3)), (53, 7, 2020));
    }
}
