//! The calendar commands of the agenda without an Emacs calendar:
//! org-agenda-convert-date, org-agenda-holidays,
//! org-agenda-phases-of-moon (lunar.el's Meeus approximation) and
//! org-agenda-sunrise-sunset (solar.el, NOAA equations).

use super::opt;
use crate::org::time::civil::{self, Tm};

/// calendar-date-string: "Monday, October 12, 2026".
fn date_string(t: Tm) -> String {
    format!(
        "{}, {} {}, {}",
        civil::DAY_NAMES[t.weekday() as usize],
        civil::MONTH_NAMES[(t.month - 1) as usize],
        t.day,
        t.year
    )
}

/// The Julian calendar date of absolute day `abs`.
fn julian(abs: i64) -> (i64, i64, i64) {
    // Julian Day Number of the date, then the Julian calendar algorithm.
    let jdn = abs + 1_721_425;
    let c = jdn + 32082;
    let d = (4 * c + 3) / 1461;
    let e = c - 1461 * d / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = d - 4800 + m / 10;
    (year, month, day)
}

/// org-agenda-convert-date's text.
pub fn convert(day: i64) -> String {
    let t = Tm::from_absolute(day);
    let (iw, _, iy) = civil::iso_week(t);
    let wd = if t.weekday() == 0 { 7 } else { t.weekday() };
    let doy = day - Tm::date(t.year, 1, 1).absolute() + 1;
    let ylen = if civil::leap(t.year) { 366 } else { 365 };
    let (jy, jm, jd) = julian(day);
    format!(
        "Gregorian:  {}\nISO:        Day {wd} of week {iw} of {iy}\nDay of Yr:  Day {doy} of {}; {} days remaining in the year\nJulian:     {} {jd}, {jy}\nAstron. JD: {} (Julian date number at noon UTC)\n",
        date_string(t),
        t.year,
        ylen - doy,
        civil::MONTH_NAMES[(jm - 1) as usize],
        day + 1_721_425
    )
}

/// The three months around `day`: [first day, last day].
fn three_months(day: i64) -> (i64, i64) {
    let t = Tm::from_absolute(day);
    let start = Tm::date(t.year, t.month - 1, 1).norm().absolute();
    let end = Tm::date(t.year, t.month + 2, 1).norm().absolute() - 1;
    (start, end)
}

/// org-agenda-holidays: the holidays of the three months around `day`.
pub fn holidays(day: i64) -> String {
    let (a, b) = three_months(day);
    let mut out = String::new();
    for d in a..=b {
        let t = Tm::from_absolute(d);
        for h in crate::org::time::diary::holidays(t) {
            out.push_str(&format!("{}: {h}\n", date_string(t)));
        }
    }
    if out.is_empty() {
        out.push_str("No holidays known\n");
    }
    out
}

/// Julian Ephemeris Day of lunar phase `k` (Meeus, Astronomical Algorithms ch. 49).
fn phase_jde(k: f64) -> f64 {
    let t = k / 1236.85;
    let rad = std::f64::consts::PI / 180.0;
    let jde = 2451550.09766 + 29.530588861 * k + 0.00015437 * t * t - 0.00000015 * t * t * t
        + 0.00000000073 * t.powi(4);
    let e = 1.0 - 0.002516 * t - 0.0000074 * t * t;
    let m = (2.5534 + 29.10535670 * k - 0.0000014 * t * t) * rad;
    let mp = (201.5643 + 385.81693528 * k + 0.0107582 * t * t) * rad;
    let f = (160.7108 + 390.67050284 * k - 0.0016118 * t * t) * rad;
    let om = (124.7746 - 1.56375588 * k + 0.0020672 * t * t) * rad;
    let phase = ((k.fract() + 1.0).fract() * 4.0).round() as i64 % 4;
    let corr = match phase {
        0 => {
            -0.40720 * mp.sin()
                + 0.17241 * e * m.sin()
                + 0.01608 * (2.0 * mp).sin()
                + 0.01039 * (2.0 * f).sin()
                + 0.00739 * e * (mp - m).sin()
                - 0.00514 * e * (mp + m).sin()
                + 0.00208 * e * e * (2.0 * m).sin()
                - 0.00111 * (mp - 2.0 * f).sin()
                - 0.00057 * (mp + 2.0 * f).sin()
                + 0.00056 * e * (2.0 * mp + m).sin()
                - 0.00042 * (3.0 * mp).sin()
                + 0.00042 * e * (m + 2.0 * f).sin()
                + 0.00038 * e * (m - 2.0 * f).sin()
                - 0.00024 * e * (2.0 * mp - m).sin()
                - 0.00017 * om.sin()
        }
        2 => {
            -0.40614 * mp.sin()
                + 0.17302 * e * m.sin()
                + 0.01614 * (2.0 * mp).sin()
                + 0.01043 * (2.0 * f).sin()
                + 0.00734 * e * (mp - m).sin()
                - 0.00515 * e * (mp + m).sin()
                + 0.00209 * e * e * (2.0 * m).sin()
                - 0.00111 * (mp - 2.0 * f).sin()
                - 0.00057 * (mp + 2.0 * f).sin()
                + 0.00056 * e * (2.0 * mp + m).sin()
                - 0.00042 * (3.0 * mp).sin()
                + 0.00042 * e * (m + 2.0 * f).sin()
                + 0.00038 * e * (m - 2.0 * f).sin()
                - 0.00024 * e * (2.0 * mp - m).sin()
                - 0.00017 * om.sin()
        }
        _ => {
            let c = -0.62801 * mp.sin() + 0.17172 * e * m.sin() - 0.01183 * e * (mp + m).sin()
                + 0.00862 * (2.0 * mp).sin()
                + 0.00804 * (2.0 * f).sin()
                + 0.00454 * e * (mp - m).sin()
                + 0.00204 * e * e * (2.0 * m).sin()
                - 0.00180 * (mp - 2.0 * f).sin()
                - 0.00070 * (mp + 2.0 * f).sin()
                - 0.00040 * (3.0 * mp).sin()
                - 0.00034 * e * (2.0 * mp - m).sin();
            let w = 0.00306 - 0.00038 * e * m.cos() + 0.00026 * mp.cos() - 0.00002 * (mp - m).cos()
                + 0.00002 * (mp + m).cos()
                + 0.00002 * (2.0 * f).cos();
            if phase == 1 { c + w } else { c - w }
        }
    };
    jde + corr
}

/// The local time of a Julian Day.
fn jd_to_local(jd: f64) -> Tm {
    let epoch = ((jd - 2440587.5) * 86400.0).round() as i64;
    Tm::from_epoch(epoch)
}

fn clock(t: Tm) -> String {
    let (h, ampm) = match t.hour {
        0 => (12, "am"),
        h if h < 12 => (h, "am"),
        12 => (12, "pm"),
        h => (h - 12, "pm"),
    };
    format!("{h}:{:02}{ampm} ({})", t.min, civil::zone_name(t))
}

/// org-agenda-phases-of-moon: the phases of the three months around `day`.
pub fn moon_phases(day: i64) -> String {
    let (a, b) = three_months(day);
    let names = [
        "New Moon",
        "First Quarter Moon",
        "Full Moon",
        "Last Quarter Moon",
    ];
    let start = Tm::from_absolute(a);
    let year = start.year as f64 + (start.month as f64 - 1.0) / 12.0;
    let mut k = ((year - 2000.0) * 12.3685).floor() - 1.0;
    let mut out = String::new();
    loop {
        let jde = phase_jde(k);
        let t = jd_to_local(jde);
        let d = t.absolute();
        if d > b {
            break;
        }
        if d >= a {
            let i = ((k.fract() + 1.0).fract() * 4.0).round() as usize % 4;
            out.push_str(&format!("{}: {} {}\n", date_string(t), names[i], clock(t)));
        }
        k += 0.25;
    }
    out
}

fn coord(name: &str) -> Option<f64> {
    match opt(name)? {
        crate::org::sexp::Sexp::Int(n) => Some(n as f64),
        crate::org::sexp::Sexp::Float(f) => Some(f),
        v => v.str().and_then(|s| s.parse().ok()),
    }
}

/// org-agenda-sunrise-sunset for `day` at calendar-latitude/longitude.
pub fn sunrise_sunset(day: i64) -> Result<String, String> {
    let (Some(lat), Some(lon)) = (coord("calendar-latitude"), coord("calendar-longitude")) else {
        return Err(
            "Set calendar-latitude and calendar-longitude in the [org] table of the config".into(),
        );
    };
    let t = Tm::from_absolute(day);
    let n = (day - Tm::date(t.year, 1, 1).absolute() + 1) as f64;
    let rad = std::f64::consts::PI / 180.0;
    // NOAA: fractional year, equation of time and declination.
    let g =
        2.0 * std::f64::consts::PI / if civil::leap(t.year) { 366.0 } else { 365.0 } * (n - 1.0);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * g.cos()
            - 0.032077 * g.sin()
            - 0.014615 * (2.0 * g).cos()
            - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    let cos_ha =
        ((90.833 * rad).cos() / ((lat * rad).cos() * decl.cos())) - (lat * rad).tan() * decl.tan();
    let place = opt("calendar-location-name")
        .and_then(|v| v.str().map(str::to_owned))
        .unwrap_or_else(|| format!("{lat:.1}N, {lon:.1}E"));
    if !(-1.0..=1.0).contains(&cos_ha) {
        return Ok(format!(
            "No sunrise or sunset on {} at {place}",
            date_string(t)
        ));
    }
    let ha = cos_ha.acos() / rad;
    let utc_min = |h: f64| 720.0 - 4.0 * (lon + h) - eqtime;
    let at = |m: f64| {
        let midnight_utc = (day - civil::ABS_1970) * 86400;
        Tm::from_epoch(midnight_utc + (m * 60.0).round() as i64)
    };
    let rise = at(utc_min(ha));
    let set = at(utc_min(-ha));
    let len = 8.0 * ha;
    Ok(format!(
        "Sunrise {}, sunset {} at {place} ({}:{:02} hours daylight)",
        clock(rise),
        clock(set),
        (len / 60.0) as i64,
        (len % 60.0) as i64
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_dates() {
        let d = Tm::date(2026, 10, 12).absolute();
        let s = convert(d);
        assert!(
            s.starts_with("Gregorian:  Monday, October 12, 2026\n"),
            "{s}"
        );
        assert!(s.contains("ISO:        Day 1 of week 42 of 2026"), "{s}");
        assert!(
            s.contains("Day of Yr:  Day 285 of 2026; 80 days remaining in the year"),
            "{s}"
        );
        assert!(s.contains("Julian:     September 29, 2026"), "{s}");
        assert!(s.contains("Astron. JD: 2461326"), "{s}");
    }

    #[test]
    fn moon() {
        // Full moon of 2026-10-26 (UTC 04:12) per published tables.
        let s = moon_phases(Tm::date(2026, 10, 12).absolute());
        assert!(
            s.contains("October 26, 2026: Full Moon") || s.contains("October 25, 2026: Full Moon"),
            "{s}"
        );
        assert!(s.contains("New Moon"));
    }
}
