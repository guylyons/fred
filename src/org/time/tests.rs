//! Ports of the timestamp, org-read-date, org-timestamp-change,
//! schedule/deadline and repeater tests of testing/lisp/test-org.el
//! (expected values checked against upstream Emacs), with "now" pinned.

use super::civil::{self, Tm};
use super::read::{analyze, date_string, default_time};
use super::stamp::{self, Part, Timestamp};
use super::*;
use crate::org::tests::org;

/// org-test-at-time: pin now (local time).
fn at(s: &str) {
    let p = stamp::parse_time_string(s).unwrap();
    civil::set_now(Some(p.tm()));
}

/// org-test-without-dow.
fn without_dow() {
    options::put(
        "org-timestamp-formats",
        toml::Value::String("'(\"%Y-%m-%d\" . \"%Y-%m-%d %H:%M\")".into()),
    );
}

fn opt(name: &str, v: &str) {
    options::put(name, toml::Value::String(v.into()));
}

/// org-read-date with FROM-STRING: the string result, time given, end time.
fn rd(now: &str, ans: &str, def: Option<&str>) -> (String, bool, Option<String>) {
    at(now);
    let d = default_time(def.map(|d| stamp::time_string_to_tm(d).unwrap()), false);
    let a = analyze(ans, d);
    (date_string(&a), a.time_given, a.end_time)
}

fn text(e: &crate::editor::Editor) -> String {
    (0..e.line_count())
        .map(|l| e.buf.line(l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn read_date_language() {
    let now = "2026-10-04 10:30";
    let cases: &[(&str, &str, bool, Option<&str>)] = &[
        ("+3d", "2026-10-07", false, None),
        ("-2w", "2026-09-20", false, None),
        ("++1m", "2026-11-04", false, None),
        ("fri", "2026-10-09", false, None),
        ("+2fri", "2026-10-16", false, None),
        ("-fri", "2026-10-02", false, None),
        ("3/15", "2027-03-15", false, None),
        ("2026-03-15", "2026-03-15", false, None),
        ("15", "2026-10-15", false, None),
        ("3", "2026-11-03", false, None),
        ("sep 15", "2027-09-15", false, None),
        ("feb 2", "2027-02-02", false, None),
        ("12:45", "2026-10-04 12:45", true, None),
        ("1pm", "2026-10-04 13:00", true, None),
        ("11am-1:15pm", "2026-10-04 11:00", true, Some("13:15")),
        (".", "2026-10-04", false, None),
        ("+0", "2026-10-04", false, None),
        ("10:00+1:30", "2026-10-04 10:00", true, Some("11:30")),
        ("9h30", "2026-10-04 09:30", true, None),
        ("w42", "2026-10-12", false, None),
        ("2027-w01-3", "2027-01-06", false, None),
        ("29.03. 16:40", "2027-03-29 16:40", true, None),
        ("12-3-29 16:40", "2012-03-29 16:40", true, None),
        ("29.03.2012 16:40", "2012-03-29 16:40", true, None),
        ("+1y", "2027-10-04", false, None),
        ("+4", "2026-10-08", false, None),
        ("+2h", "2026-10-04 12:30", true, None),
        ("thu 14:00", "2026-10-08 14:00", true, None),
        ("oct 4", "2026-10-04", false, None),
        ("oct 3", "2027-10-03", false, None),
        ("2040-01-01", "2037-01-01", false, None),
        ("1/2/27", "2027-01-02", false, None),
        ("8am-13:15", "2026-10-04", false, Some("13:15")),
        ("22 sept 0:34", "2026-10-22 00:34", true, None),
        ("sep 12 9", "2009-09-12", false, None),
        ("5", "2026-10-05", false, None),
        ("+3D", "2026-10-04", false, None),
        ("", "2026-10-04", false, None),
        ("mon", "2026-10-05", false, None),
        ("--3d", "2026-10-01", false, None),
        ("2026-10-04 Sun 10:00", "2026-10-04 10:00", true, None),
        ("20261231", "2026-12-31", false, None),
        ("14h", "2026-10-04 14:00", true, None),
    ];
    for (ans, want, given, end) in cases {
        let (got, g, e) = rd(now, ans, None);
        assert_eq!(
            (got.as_str(), g, e.as_deref()),
            (*want, *given, *end),
            "input {ans:?}"
        );
    }
    // Relative to the default date (++), or to today (+).
    assert_eq!(rd(now, "++3d", Some("2026-12-25")).0, "2026-12-28");
    assert_eq!(rd(now, "+3d", Some("2026-12-25")).0, "2026-10-07");
    assert_eq!(rd(now, "15", Some("2026-12-25")).0, "2026-10-15");
    assert_eq!(rd(now, "", Some("2026-12-25 09:15")).0, "2026-12-25");
    opt("org-read-date-prefer-future", "time");
    assert_eq!(rd(now, "09:00", None).0, "2026-10-05 09:00");
    opt("org-read-date-prefer-future", "nil");
    assert_eq!(rd(now, "3", None).0, "2026-10-03");
    options::set(toml::Table::new());
    options::put("org-extend-today-until", toml::Value::Integer(4));
    assert_eq!(rd("2026-10-04 02:30", "", None).0, "2026-10-03");
    assert_eq!(rd("2026-10-04 02:30", "+1d", None).0, "2026-10-05");
    civil::set_now(None);
}

#[test]
fn read_date_upstream_spec() {
    assert_eq!(
        rd("2026-10-04", "12-3-29 16:40", None).0,
        "2012-03-29 16:40"
    );
    assert_eq!(
        rd("2026-10-04", "29.03.2012 16:40", None).0,
        "2012-03-29 16:40"
    );
    assert!(
        rd("2026-10-04", "29.03. 16:40", None)
            .0
            .ends_with("-03-29 16:40")
    );
    assert_eq!(rd("2014-03-04", "+1y", Some("2012-03-29")).0, "2015-03-04");
    assert_eq!(rd("2014-03-04", "++1y", Some("2012-03-29")).0, "2013-03-29");
    assert_eq!(rd("2014-03-04", "1", None).0, "2014-04-01");
    assert_eq!(rd("2012-03-29", "3-4", None).0, "2013-03-04");
    opt("org-read-date-prefer-future", "nil");
    assert_eq!(rd("2012-03-29", "3-4", None).0, "2012-03-04");
    opt("org-read-date-prefer-future", "time");
    assert_eq!(rd("2012-03-29 16:40", "00:40", None).0, "2012-03-30 00:40");
    assert_ne!(
        &rd("2012-03-29 16:40", "29 00:40", None).0[..10],
        "2012-03-30"
    );
    opt("org-read-date-prefer-future", "t");
    assert_eq!(rd("2014-03-04", "1", Some("2012-03-29")).0, "2014-04-01");
    assert_eq!(rd("2014-03-04", "25", Some("2012-03-29")).0, "2014-03-25");
}

#[test]
fn parse_time_string_spec() {
    let p = |s: &str| stamp::parse_time_string(s).unwrap();
    assert_eq!(p("2012-03-29 16:40").tm(), Tm::at(2012, 3, 29, 16, 40));
    assert_eq!(p("[2012-03-29 16:40]").tm(), Tm::at(2012, 3, 29, 16, 40));
    assert_eq!(p("<2012-03-29>").tm(), Tm::date(2012, 3, 29));
    assert_eq!(p("<2012-03-29>").hour, None);
    assert!(stamp::parse_time_string("nope").is_none());
    // Emacs parse-time-string.
    let f = read::parse_time_string("2026-03-15 fri 10:00");
    assert_eq!(
        (f.year, f.month, f.day, f.hour, f.min, f.wday),
        (Some(2026), Some(3), Some(15), Some(10), Some(0), Some(5))
    );
    assert_eq!(read::parse_time_string("sep 12 9").year, Some(2009));
    assert_eq!(read::parse_time_string("2026-075").day, Some(16));
}

#[test]
fn closest_date_spec() {
    let c =
        |s: &str, cur: &str, p: &str| Tm::from_absolute(stamp::closest_date(s, cur, p).unwrap());
    assert_eq!(c("<2012-03-29>", "<2014-03-04>", ""), Tm::date(2012, 3, 29));
    assert_eq!(
        c("<2012-03-29 +0d>", "<2014-03-04>", ""),
        Tm::date(2012, 3, 29)
    );
    assert_eq!(
        c("<2012-03-29 +1m>", "<2014-03-04>", "past"),
        Tm::date(2014, 3, 1)
    );
    assert_eq!(
        c("<2012-03-04 +1m>", "<2014-03-04>", "past"),
        Tm::date(2014, 3, 4)
    );
    assert_eq!(
        c("<2012-03-29 +1m>", "<2014-03-04>", "future"),
        Tm::date(2014, 3, 29)
    );
    assert_eq!(
        c("<2012-03-04 +1m>", "<2014-03-04>", "future"),
        Tm::date(2014, 3, 4)
    );
    assert_eq!(
        c("<2012-03-29 +1m>", "<2014-03-04>", ""),
        Tm::date(2014, 3, 1)
    );
    assert_eq!(
        c("<2012-03-04 +1m>", "<2014-04-28>", ""),
        Tm::date(2014, 5, 4)
    );
    assert_eq!(
        c("<2014-03-04 +50h>", "<2014-03-09>", "past"),
        Tm::date(2014, 3, 8)
    );
    assert_eq!(
        c("<2014-03-04 +50h>", "<2014-03-09>", "future"),
        Tm::date(2014, 3, 10)
    );
    assert_eq!(
        c("<2014-03-04 +2d>", "<2014-03-09>", "past"),
        Tm::date(2014, 3, 8)
    );
    assert_eq!(
        c("<2014-03-04 +2d>", "<2014-03-09>", "future"),
        Tm::date(2014, 3, 10)
    );
    assert_eq!(
        c("<2014-03-04 +1w>", "<2014-03-09>", "past"),
        Tm::date(2014, 3, 4)
    );
    assert_eq!(
        c("<2014-03-04 +1w>", "<2014-03-09>", "future"),
        Tm::date(2014, 3, 11)
    );
    assert_eq!(
        c("<2014-03-05 +2m>", "<2015-02-04>", "past"),
        Tm::date(2015, 1, 5)
    );
    assert_eq!(
        c("<2012-03-29 +2m>", "<2014-03-04>", "future"),
        Tm::date(2014, 3, 29)
    );
    assert_eq!(
        c("<2014-03-05 +2y>", "<2015-02-04>", "past"),
        Tm::date(2014, 3, 5)
    );
    assert_eq!(
        c("<2012-03-29 +2y>", "<2014-03-04>", "future"),
        Tm::date(2014, 3, 29)
    );
}

#[test]
fn deadline_close_p_spec() {
    at("2016-06-03 Fri 01:43");
    let e = org("* Heading", "");
    let close = |ts: &str, n: Option<i64>| deadline_close_p(&e, 0, ts, n);
    assert!(close("2016-06-03 Fri", Some(0)));
    assert!(close("2016-06-02 Thu", Some(0)));
    assert!(!close("2016-06-04 Sat", Some(0)));
    assert!(close("2016-06-04 Sat", Some(1)));
    assert!(close("2016-06-03 Fri 12:00", Some(0)));
    assert!(close("2016-06-04 Sat -1d", None));
    assert!(!close("2016-06-04 Sat -0d", None));
    assert!(close("2016-06-10 Fri -1w", None));
    assert!(!close("2016-06-11 Sat -1w", None));
    assert!(close("2016-06-04 Sat -0d", Some(1)));
    assert!(!close("2016-06-04 Sat -0d", Some(0)));
    let done = org("* DONE Heading", "");
    assert!(!deadline_close_p(&done, 0, "2016-06-03", None));
    let todo = org("* TODO Heading", "");
    assert!(deadline_close_p(&todo, 0, "2016-06-03", None));
}

#[test]
fn at_timestamp_parts() {
    let p = |s: &str, pos: usize| stamp::at_timestamp(s, pos).map(|x| x.1);
    assert_eq!(p("<2012-03-29 Thu>", 0), Some(Part::Bracket));
    assert_eq!(p("2012-03-29 Thu", 0), None);
    assert_eq!(p("<2012-03-29 Thu>", 1), Some(Part::Year));
    assert_eq!(p("<2012-03-29 Thu>", 6), Some(Part::Month));
    assert_eq!(p("<2012-03-29 Thu>", 9), Some(Part::Day));
    assert_eq!(p("<2012-03-29 Thu>", 13), Some(Part::Day));
    assert_eq!(p("<2012-03-29 Thu 12:34>", 16), Some(Part::Hour));
    assert_eq!(p("<2012-03-29 Thu 12:34>", 19), Some(Part::Minute));
    assert!(matches!(
        p("<2012-03-29 Thu +2y>", 17),
        Some(Part::Extra(_))
    ));
    assert_eq!(p("<2012-03-29 Thu>", 15), Some(Part::Bracket));
    assert_eq!(p("<2012-03-29 Thu>»", 16), Some(Part::After));
    assert_eq!(p("<2012-03-29>", 10), Some(Part::Day));
    assert_eq!(p("<2012-03-29 12:34>", 11), Some(Part::Day));
    assert_eq!(p("<2012-03-29 12:34>", 12), Some(Part::Hour));
    assert_eq!(p("<2012-03-29 12:34>", 15), Some(Part::Minute));
    assert!(p("[2012-03-29 Thu]", 3).is_some());
}

#[test]
fn timestamp_change_round_trips() {
    for dow in [true, false] {
        if !dow {
            without_dow();
        }
        let now = Tm::at(2026, 3, 14, 14, 7);
        let (d, t) = stamp::formats();
        for ts in [
            format!("<{}>", civil::format(&d, now)),
            format!("<{}>", civil::format(&t, now)),
            format!("<{}-23:00>", civil::format(&t, now)),
        ] {
            for pos in 1..ts.len() - 1 {
                if ts.as_bytes()[pos] == b' ' {
                    continue;
                }
                let mut s = ts.clone();
                let mut p = pos;
                for n in [-1, 2, -1] {
                    let (r, part) = stamp::at_timestamp(&s, p).unwrap();
                    let c = stamp::change(
                        &s[r.clone()],
                        p - r.start,
                        part,
                        n,
                        None,
                        Default::default(),
                    )
                    .unwrap();
                    s = c.text;
                    p = c.point;
                }
                assert_eq!(s, ts, "at {pos} of {ts}");
            }
        }
    }
}

#[test]
fn timestamp_change_corner_cases() {
    without_dow();
    let ch = |s: &str, n: i64, u: Unit| {
        stamp::change(s, 1, Part::Day, n, Some(What::Unit(u)), Default::default())
            .unwrap()
            .text
    };
    assert_eq!(ch("<2026-01-31>", 1, Unit::Month), "<2026-02-28>");
    assert_eq!(ch("<2026-02-28>", 1, Unit::Month), "<2026-03-28>");
    assert_eq!(ch("<2026-03-31>", -1, Unit::Month), "<2026-02-28>");
    assert_eq!(ch("<2025-12-31>", 1, Unit::Month), "<2026-01-31>");
    assert_eq!(ch("<2025-12-31>", 2, Unit::Month), "<2026-02-28>");
    assert_eq!(ch("<2026-02-28>", -2, Unit::Month), "<2025-12-28>");
    for i in 1..=9 {
        options::put(
            "org-timestamp-rounding-minutes",
            toml::Value::Array(vec![0.into(), i.into()]),
        );
        let up = stamp::change(
            "<2026-03-14 12:00>",
            12,
            Part::Minute,
            1,
            Some(What::Unit(Unit::Minute)),
            ChangeOpts {
                updown: true,
                ..Default::default()
            },
        );
        assert_eq!(up.unwrap().text, format!("<2026-03-14 12:0{i}>"));
        let down = stamp::change(
            "<2026-03-14 12:00>",
            12,
            Part::Minute,
            -1,
            Some(What::Unit(Unit::Minute)),
            ChangeOpts {
                updown: true,
                ..Default::default()
            },
        );
        assert_eq!(down.unwrap().text, format!("<2026-03-14 11:5{}>", 10 - i));
    }
    // Time ranges move together; repeaters and warnings by position.
    options::set(toml::Table::new());
    without_dow();
    assert_eq!(
        ch("<2026-04-18 21:00-22:00>", 1, Unit::Hour),
        "<2026-04-18 22:00-23:00>"
    );
    assert_eq!(stamp::modify_ts_extra(" +1w -2d", 3, 1, 1), " +1m -2d");
    assert_eq!(stamp::modify_ts_extra(" +1w -2d", 2, -3, 1), " +1w -2d");
    assert_eq!(stamp::modify_ts_extra(" +1w -2d", 2, 1, 1), " +2w -2d");
    assert_eq!(stamp::modify_ts_extra(" +1w -2d", 7, -1, 1), " +1w -2d");
    assert_eq!(stamp::modify_ts_extra(" +1w -2d", 6, -3, 1), " +1w -0d");
    assert_eq!(stamp::modify_ts_extra("-10:03", 5, 1, 5), "-10:05");
    // Checked against upstream Emacs.
    let up = ChangeOpts {
        updown: true,
        ..Default::default()
    };
    options::set(toml::Table::new());
    let c = stamp::change("<2026-03-14 Sat 12:03-13:00>", 16, Part::Hour, 1, None, up).unwrap();
    assert_eq!(c.text, "<2026-03-14 Sat 13:03-14:00>");
    let c = stamp::change(
        "<2026-03-14 Sat 12:03 +1w>",
        24,
        Part::Extra(3),
        1,
        None,
        up,
    )
    .unwrap();
    assert_eq!(
        (c.text.as_str(), c.point),
        ("<2026-03-14 Sat 12:03 +1m>", 24)
    );
    assert_eq!(
        stamp::at_timestamp("<2026-03-14 Sat 12:03 +1w>", 24)
            .unwrap()
            .1,
        Part::Extra(3)
    );
    without_dow();
    // Brackets toggle the type.
    let c = stamp::change(
        "<2026-04-18>",
        0,
        Part::Bracket,
        1,
        None,
        Default::default(),
    )
    .unwrap();
    assert_eq!(c.text, "[2026-04-18]");
    // Suppressed delays when repeating.
    let s = stamp::change(
        "<2026-04-18 +1w --2d>",
        1,
        Part::Day,
        7,
        Some(What::Unit(Unit::Day)),
        ChangeOpts {
            suppress_tmp_delay: true,
            ..Default::default()
        },
    );
    assert_eq!(s.unwrap().text, "<2026-04-25 +1w>");
}

#[test]
fn timestamp_object() {
    without_dow();
    let t = Timestamp::parse("<2012-03-29 Thu 16:40-18:00 ++2y/3d --1w> rest").unwrap();
    assert_eq!(t.kind, stamp::Kind::ActiveRange);
    assert_eq!(t.range, Some(stamp::RangeType::Time));
    assert_eq!(t.start.hour, Some(16));
    assert_eq!(t.end.hour, Some(18));
    let r = t.repeater.unwrap();
    assert_eq!(
        (r.kind, r.value, r.unit, r.deadline),
        (
            stamp::RepeatType::CatchUp,
            2,
            Unit::Year,
            Some((3, Unit::Day))
        )
    );
    let w = t.warning.unwrap();
    assert!(w.first && w.value == 1 && w.unit == Unit::Week);
    assert_eq!(t.len, "<2012-03-29 Thu 16:40-18:00 ++2y/3d --1w> ".len());
    assert_eq!(t.interpret(), "<2012-03-29 16:40-18:00 ++2y/3d --1w>");
    let r = Timestamp::parse("[2011-07-14 Thu]--[2012-03-29 Thu]").unwrap();
    assert_eq!(r.kind, stamp::Kind::InactiveRange);
    assert_eq!(r.format("%Y-%m-%d", true), "2012-03-29");
    assert_eq!(r.split_range(true).interpret(), "[2012-03-29]");
    assert_eq!(r.split_range(false).interpret(), "[2011-07-14]");
    assert_eq!(r.interpret(), "[2011-07-14]--[2012-03-29]");
    assert_eq!(
        Timestamp::parse("<2012-03-29 Thu 16:40>")
            .unwrap()
            .format("%Y-%m-%d %R", false),
        "2012-03-29 16:40"
    );
    let d = Timestamp::parse("<%%(diary-float t 4 2) 10:00-11:00>").unwrap();
    assert_eq!(d.kind, stamp::Kind::Diary);
    assert_eq!(d.diary.as_deref(), Some("(diary-float t 4 2)"));
    assert_eq!(d.interpret(), "<%%(diary-float t 4 2) 10:00-11:00>");
    assert!(Timestamp::from_string(" ").is_none());
    assert_eq!(
        Timestamp::from_time(Tm::at(2012, 3, 29, 16, 40), true, true).interpret(),
        "[2012-03-29 16:40]"
    );
    assert_eq!(r.translate(true, None), "07/14/11 Thu--03/29/12 Thu");
    assert_eq!(
        stamp::custom_display("2012-03-29 Thu 16:40-18:00 +1w"),
        "03/29/12 Thu 16:40-18:00 +1w"
    );
    assert_eq!(
        stamp::repeat_of("<2012-03-29 Thu 16:40 +2y>").as_deref(),
        Some("+2y")
    );
    assert_eq!(stamp::repeat_of("<2012-03-29 Thu 16:40>"), None);
    assert_eq!(
        stamp::compact_tod("2012-03-29 10:00-11:30").as_deref(),
        Some("10:00+1:30")
    );
    assert_eq!(
        stamp::time_stamp_format(true, stamp::Brackets::Inactive, false),
        "[%Y-%m-%d %H:%M]"
    );
}

#[test]
fn org_timestamp_command() {
    at("2014-03-04 00:41");
    // The prompt inserts the chosen stamp.
    let e = org("Text", "l<C-c>.2014-03-04<CR>");
    assert!(text(&e).starts_with("T<2014-03-04 Tue>ext"), "{}", text(&e));
    // The live interpretation shows in the prompt.
    let e = org("Text", "<C-c>.+2d");
    match &e.mode {
        Mode::Command(cl) => assert!(
            cl.prompt.ends_with("=> <2014-03-06 Thu>: "),
            "{}",
            cl.prompt
        ),
        m => panic!("{m:?}"),
    }
    // Calendar keys move the virtual calendar date.
    let e = org("Text", "<C-c>.<S-Right><S-Right><CR>");
    assert!(text(&e).starts_with("<2014-03-06 Thu>"), "{}", text(&e));
    // C-u: with time; C-u C-u: now, no prompt.
    let e = org("Text", "<C-c>.12:30<CR>");
    assert!(text(&e).starts_with("<2014-03-04 Tue 12:30>"));
    let mut e = org("x", "");
    crate::org::run(&mut e, "org-timestamp", Prefix::U(2));
    assert_eq!(text(&e), "<2014-03-04 Tue 00:41>x");
    // Inactive.
    let e = org("x", "<C-c>!<CR>");
    assert_eq!(text(&e), "[2014-03-04 Tue]x");
    // Replace a stamp, keeping its repeater.
    let e = org("<2012-03-29 thu. +2y>", "4l<C-c>.2014-03-04<CR>");
    assert_eq!(text(&e), "<2014-03-04 Tue +2y>");
    // Twice in a row: a range.
    let e = org("x", "<C-c>.2012-03-29<CR><C-c>.2014-03-05<CR>");
    assert_eq!(text(&e), "<2012-03-29 Thu>--<2014-03-05 Wed>x");
    // Time ranges from the prompt.
    let e = org("x", "<C-c>.11am-1:15pm<CR>");
    assert_eq!(text(&e), "<2014-03-04 Tue 11:00-13:15>x");
    civil::set_now(None);
}

#[test]
fn shift_commands_on_timestamps() {
    let mut e = org("<2026-03-14 Sat 12:03>", "");
    e.cur.byte = 16;
    crate::org::run(&mut e, "org-timestamp-up", Prefix::None);
    assert_eq!(text(&e), "<2026-03-14 Sat 13:03>");
    e.cur.byte = 20;
    crate::org::run(&mut e, "org-timestamp-up", Prefix::None);
    assert_eq!(text(&e), "<2026-03-14 Sat 13:05>");
    crate::org::run(&mut e, "org-timestamp-down", Prefix::Num(3));
    assert_eq!(text(&e), "<2026-03-14 Sat 13:02>");
    crate::org::run(&mut e, "org-timestamp-up-day", Prefix::None);
    assert_eq!(text(&e), "<2026-03-15 Sun 13:02>");
    crate::org::run(&mut e, "org-timestamp-down-day", Prefix::Num(15));
    assert_eq!(text(&e), "<2026-02-28 Sat 13:02>");
    e.cur.byte = 0;
    crate::org::run(&mut e, "org-timestamp-up", Prefix::None);
    assert_eq!(text(&e), "[2026-02-28 Sat 13:02]");
    let mut e = org("x", "");
    crate::org::run(&mut e, "org-timestamp-up", Prefix::None);
    assert_eq!(e.msg.as_ref().unwrap().0, "? Not at a timestamp");
    // A CLOCK line updates its duration.
    let mut e = org(
        "CLOCK: [2026-03-14 Sat 10:00]--[2026-03-14 Sat 11:00] =>  1:00",
        "",
    );
    e.cur.byte = 47;
    crate::org::run(&mut e, "org-timestamp-up", Prefix::None);
    assert_eq!(
        text(&e),
        "CLOCK: [2026-03-14 Sat 10:00]--[2026-03-14 Sat 12:00] =>  2:00"
    );
}

#[test]
fn add_planning_info_spec() {
    without_dow();
    let run = |t: &str, adapt: bool, what: Option<(Planning, &str)>, remove: &[Planning]| {
        options::put("org-adapt-indentation", toml::Value::Boolean(adapt));
        let mut e = org(t, "");
        set_planning(&mut e, 0, what, remove);
        text(&e)
    };
    let dl = Some((Planning::Deadline, "<2015-06-25>"));
    assert_eq!(
        run("* H\nParagraph", true, dl, &[]),
        "* H\n  DEADLINE: <2015-06-25>\nParagraph"
    );
    assert_eq!(
        run("* H\nParagraph", false, dl, &[]),
        "* H\nDEADLINE: <2015-06-25>\nParagraph"
    );
    assert_eq!(
        run(
            "* H\n  DEADLINE: <2015-06-24 Wed>\nParagraph",
            true,
            dl,
            &[]
        ),
        "* H\n  DEADLINE: <2015-06-25>\nParagraph"
    );
    assert_eq!(
        run("* H\nDEADLINE: <2015-06-24 Wed>\nParagraph", false, dl, &[]),
        "* H\nDEADLINE: <2015-06-25>\nParagraph"
    );
    let sc = Some((Planning::Scheduled, "<2015-06-25>"));
    assert_eq!(
        run("* H\nParagraph", true, sc, &[]),
        "* H\n  SCHEDULED: <2015-06-25>\nParagraph"
    );
    assert_eq!(
        run("* H\n  SCHEDULED: <2015-06-24>\nParagraph", true, dl, &[]),
        "* H\n  DEADLINE: <2015-06-25> SCHEDULED: <2015-06-24>\nParagraph"
    );
    let three =
        "* H\n  CLOSED: [2015-06-24] DEADLINE: <2015-06-25 Thu> SCHEDULED: <2015-06-24>\nParagraph";
    assert_eq!(
        run(three, true, None, &[Planning::Deadline]),
        "* H\n  CLOSED: [2015-06-24] SCHEDULED: <2015-06-24>\nParagraph"
    );
    assert_eq!(
        run(
            three,
            true,
            None,
            &[Planning::Scheduled, Planning::Deadline]
        ),
        "* H\n  CLOSED: [2015-06-24]\nParagraph"
    );
    assert_eq!(
        run(
            "* H\n  CLOSED: [2015-06-25 Thu] DEADLINE: <2015-06-25>\nParagraph",
            true,
            None,
            &[Planning::Closed]
        ),
        "* H\n  DEADLINE: <2015-06-25>\nParagraph"
    );
    assert_eq!(
        run(
            "* H\n  CLOSED: [2015-06-25 Thu]\n  Paragraph",
            true,
            None,
            &[Planning::Closed]
        ),
        "* H\n  Paragraph"
    );
    assert_eq!(
        run(
            "* H\nCLOSED: [2015-06-25 Thu] DEADLINE: <2015-06-25>\nParagraph",
            false,
            None,
            &[Planning::Closed]
        ),
        "* H\nDEADLINE: <2015-06-25>\nParagraph"
    );
    assert_eq!(
        run(
            "* H\n  CLOSED: [2015-06-25 Thu]\nParagraph",
            false,
            None,
            &[Planning::Closed]
        ),
        "* H\nParagraph"
    );
    assert_eq!(
        run(
            "* H\n  SCHEDULED: <2015-06-23 Tue> DEADLINE: <2015-06-24 Wed>\nParagraph",
            true,
            dl,
            &[Planning::Scheduled]
        ),
        "* H\n  DEADLINE: <2015-06-25>\nParagraph"
    );
}

fn sched(t: &str, deadline: bool, arg: Prefix, time: Option<&str>) -> String {
    let mut e = org(t, "");
    let r = schedule(&mut e, deadline, arg, time.map(str::to_owned));
    if let Err(m) = r {
        return format!("error: {m}");
    }
    text(&e)
}

#[test]
fn deadline_and_schedule_spec() {
    without_dow();
    for (dl, kw) in [(true, "DEADLINE"), (false, "SCHEDULED")] {
        let s =
            |t: &str, arg: Prefix, time: Option<&str>| sched(&t.replace("KW", kw), dl, arg, time);
        let want = |w: &str| w.replace("KW", kw);
        assert_eq!(
            s("* H", Prefix::None, Some("<2012-03-29 Tue>")),
            want("* H\nKW: <2012-03-29>")
        );
        assert_eq!(
            s(
                "* H\nKW: <2012-03-29>",
                Prefix::None,
                Some("<2014-03-04 Thu>")
            ),
            want("* H\nKW: <2014-03-04>")
        );
        at("2014-03-04");
        assert_eq!(
            s("* H", Prefix::None, Some("+1y")),
            want("* H\nKW: <2015-03-04>")
        );
        civil::set_now(None);
        assert_eq!(
            s("* H", Prefix::None, Some("<2012-03-29 Tue +2y>")),
            want("* H\nKW: <2012-03-29 +2y>")
        );
        assert_eq!(
            s(
                "* H\nCLOSED: [2017-01-25 Wed]",
                Prefix::None,
                Some("<2012-03-29 Tue>")
            ),
            want("* H\nKW: <2012-03-29>")
        );
        assert_eq!(s("* H\nKW: <2012-03-29>", Prefix::U(1), None), "* H");
        let post = "\nThis one\nKW: <2012-03-29>\nshould not be touched.";
        assert_eq!(
            s(
                &format!("* H\nKW: <2012-03-29>\n{post}"),
                Prefix::U(1),
                None
            ),
            want(&format!("* H\n{post}"))
        );
        assert_eq!(s("* H", Prefix::U(1), None), "* H");
        assert!(s("* H", Prefix::U(2), None).starts_with("error: No "));
        assert_eq!(
            s(
                "* H\nKW: <2017-01-19 ++7d>",
                Prefix::None,
                Some("2017-02-01")
            ),
            want("* H\nKW: <2017-02-01 ++7d>")
        );
    }
    assert_eq!(
        sched("* H", true, Prefix::None, Some("<2021-07-20 Tue -1d>")),
        "* H\nDEADLINE: <2021-07-20 -1d>"
    );
    assert_eq!(
        sched("* H", true, Prefix::None, Some("<2021-07-20 Tue +1m -3d>")),
        "* H\nDEADLINE: <2021-07-20 +1m -3d>"
    );
}

#[test]
fn schedule_prompts_and_delays() {
    without_dow();
    let e = org("* H\nSCHEDULED: <2012-03-29>", "<C-c><C-s>2014-03-04<CR>");
    assert_eq!(text(&e), "* H\nSCHEDULED: <2014-03-04>");
    assert_eq!(e.msg.as_ref().unwrap().0, "Scheduled to <2014-03-04>");
    // C-u C-u: a delay cookie from the read date.
    let mut e = org("* H\nDEADLINE: <2012-03-29>", "");
    crate::org::run(&mut e, "org-deadline", Prefix::U(2));
    for k in crate::key::parse_keys("2014-03-04<CR>") {
        e.handle_key(k);
    }
    assert_eq!(text(&e), "* H\nDEADLINE: <2012-03-29 -705d>");
    let mut e = org("* H\nSCHEDULED: <2012-03-29>", "");
    crate::org::run(&mut e, "org-schedule", Prefix::U(1));
    assert_eq!(text(&e), "* H");
    assert_eq!(e.msg.as_ref().unwrap().0, "Entry is no longer scheduled.");
    // Undo restores in one step.
    let e = org("* H", "<C-c><C-d>2014-03-04<CR>u");
    assert_eq!(text(&e), "* H");
}

/// org-todo DONE's repeat step: auto_repeat on the heading.
fn repeat(t: &str, now: Option<&str>) -> String {
    if let Some(n) = now {
        at(n);
    }
    let mut e = org(t, "");
    let r = auto_repeat(&mut e, 0);
    civil::set_now(None);
    match r {
        Err(m) => format!("error: {m}"),
        Ok(_) => text(&e),
    }
}

#[test]
fn auto_repeat_spec() {
    assert_eq!(
        repeat("* TODO H\n<2012-03-29 Thu>", None),
        "* TODO H\n<2012-03-29 Thu>"
    );
    assert_eq!(
        repeat("* TODO H\n<2012-03-29 Thu +2y>", None),
        "* TODO H\n<2014-03-29 Sat +2y>"
    );
    assert!(
        repeat(
            "* TODO Not Done Yet\n<2026-02-01 Sun 23:29 +2w>",
            Some("2026-02-04 08:00")
        )
        .contains("<2026-02-15 Sun 23:29 +2w>")
    );
    assert!(
        repeat(
            "* TODO Pending <2026-03-16 Mon .+3w>",
            Some("2026-03-16 13:11")
        )
        .contains("<2026-04-06 Mon .+3w>")
    );
    assert!(
        repeat(
            "* TODO X\nSCHEDULED: <2026-02-18 Wed 09:30 ++1w>",
            Some("2026-02-19 09:30")
        )
        .contains("<2026-02-25 Wed 09:30 ++1w")
    );
    assert!(
        repeat(
            "* TODO X\n<2026-02-11 Wed 11:00 +2m>",
            Some("2026-02-12 11:18")
        )
        .contains("<2026-04-11 Sat 11:00 +2m>")
    );
    assert!(
        repeat(
            "* TODO Pending <2026-06-20 Sat .+3m>",
            Some("2026-06-20 02:12")
        )
        .contains("<2026-09-20 Sun .+3m>")
    );
    assert!(
        repeat(
            "* TODO X\nSCHEDULED: <2026-05-09 Wed 05:30 ++1m>",
            Some("2026-05-09 06:10")
        )
        .contains("<2026-06-09 Tue 05:30 ++1m")
    );
    let two = repeat(
        "* TODO H\n<2012-03-29 Thu. +2y>\n<2014-03-04 Tue +1y>",
        None,
    );
    assert!(two.contains("<2015-03-04 Wed +1y>"), "{two}");
    assert!(
        repeat("* TODO H\n<2012-03-29 Thu +2h>", None)
            .starts_with("error: Cannot repeat in 2 hour(s)")
    );
    without_dow();
    assert!(
        repeat("* TODO H\n<2014-03-03 18:00 +8h>", Some("2014-03-04 02:35"))
            .contains("<2014-03-04 02:00 +8h>")
    );
    assert!(
        repeat(
            "* TODO H\n<2014-03-03 18:00 ++8h>",
            Some("2014-03-04 02:35")
        )
        .contains("<2014-03-04 10:00 ++8h>")
    );
    assert!(
        repeat(
            "* TODO H\n<2014-03-03 18:00 .+8h>",
            Some("2014-03-04 02:35")
        )
        .contains("<2014-03-04 10:35 .+8h>")
    );
    options::put("org-extend-today-until", toml::Value::Integer(4));
    assert!(
        repeat("* TODO H\n<2014-03-03 ++1d>", Some("2014-03-04 02:35"))
            .contains("<2014-03-04 ++1d>")
    );
    assert!(
        repeat(
            "* TODO H\n<2014-03-04 17:00 ++1d>",
            Some("2014-03-05 18:00")
        )
        .contains("<2014-03-06 17:00 ++1d>")
    );
    assert!(
        repeat(
            "* TODO H\n<2014-03-03 18:00 .+1d>",
            Some("2014-03-04 02:35")
        )
        .contains("<2014-03-04 18:00 .+1d>")
    );
    assert!(
        repeat(
            "* TODO H\n<2014-03-03 18:00 .+8h>",
            Some("2014-03-04 02:35")
        )
        .contains("<2014-03-04 10:35 .+8h>")
    );
    assert!(
        repeat(
            "* TODO Pending <2026-03-16 Mon 03:45 .+3w>",
            Some("2026-03-16 03:45")
        )
        .contains("<2026-04-05 03:45 .+3w>")
    );
    options::set(toml::Table::new());
    // Inactive, commented and verbatim stamps do not repeat.
    assert!(repeat("* TODO H\n[2012-03-29 Thu. +2y]", None).contains("[2012-03-29 Thu. +2y]"));
    let c = repeat(
        "* TODO H\n<2012-03-29 Thu +2y>\n# <2014-03-04 Tue +1y>",
        None,
    );
    assert!(c.contains("# <2014-03-04 Tue +1y>"), "{c}");
    let b = repeat(
        "* TODO H\n<2012-03-29 Thu. +2y>\n#+BEGIN_EXAMPLE\n<2014-03-04 Tue +1y>\n#+END_EXAMPLE",
        None,
    );
    assert!(b.contains("\n<2014-03-04 Tue +1y>\n"), "{b}");
    // SCHEDULED without repeater goes; CLOSED goes.
    let s = repeat(
        "* TODO H\nSCHEDULED: <2014-03-04 Tue>\n<2012-03-29 Thu +2y>",
        None,
    );
    assert!(!s.contains("SCHEDULED:"), "{s}");
    let s = repeat(
        "* TODO H\nCLOSED: [2014-03-04 Tue] SCHEDULED: <2012-03-29 Thu +2y>",
        None,
    );
    assert_eq!(s, "* TODO H\nSCHEDULED: <2014-03-29 Sat +2y>");
    let mut e = org("* H\n<2012-03-29 Thu 16:40 +2y>", "");
    assert_eq!(get_repeat(&e, 0).as_deref(), Some("+2y"));
    assert_eq!(
        auto_repeat(&mut e, 0).unwrap().as_deref(),
        Some("Entry repeats: Plain: <2014-03-29 Sat 16:40 +2y> ")
    );
    assert_eq!(
        get_repeat(
            &org(
                "* H\n#+BEGIN_EXAMPLE\n<2012-03-29 Thu +2y>\n#+END_EXAMPLE",
                ""
            ),
            0
        ),
        None
    );
}

#[test]
fn sparse_trees_and_ranges() {
    at("2026-10-04 10:00");
    let doc = "* A\nDEADLINE: <2026-10-06 Tue>\n* B\nDEADLINE: <2026-12-25 Fri>\n* DONE C\nDEADLINE: <2026-10-05 Mon>\n* D\nSCHEDULED: <2026-10-01 Thu>\n";
    let mut e = org(doc, "");
    crate::org::run(&mut e, "org-check-deadlines", Prefix::None);
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "1 deadlines past-due or due within 14 days"
    );
    assert_eq!(
        crate::org::tests::shown(&e)[..2],
        ["* A".to_owned(), "DEADLINE: <2026-10-06 Tue>".to_owned()]
    );
    let mut e = org(doc, "");
    crate::org::run(&mut e, "org-check-deadlines", Prefix::U(1));
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "2 deadlines past-due or due within 100000 days"
    );
    let e = org(doc, "<C-c>/");
    drop(e);
    let mut e = org(doc, "");
    crate::org::run(&mut e, "org-check-before-date", Prefix::None);
    for k in crate::key::parse_keys("2026-10-07<CR>") {
        e.handle_key(k);
    }
    assert_eq!(e.msg.as_ref().unwrap().0, "3 entries before 2026-10-07");
    let mut e = org(doc, "");
    crate::org::run(&mut e, "org-check-dates-range", Prefix::None);
    for k in crate::key::parse_keys("2026-10-02<CR>2026-12-31<CR>") {
        e.handle_key(k);
    }
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "3 entries between 2026-10-02 and 2026-12-31"
    );
    // Time ranges.
    let mut e = org("<2026-10-04 Sun 10:00>--<2026-10-05 Mon 12:30>", "");
    crate::org::run(&mut e, "org-evaluate-time-range", Prefix::None);
    assert_eq!(e.msg.as_ref().unwrap().0, "1 day 2 hours 30 minutes ");
    crate::org::run(&mut e, "org-evaluate-time-range", Prefix::U(1));
    assert_eq!(
        text(&e),
        "<2026-10-04 Sun 10:00>--<2026-10-05 Mon 12:30> 1d 02:30"
    );
    let mut e = org("x <2026-10-08 Thu>--<2026-10-04 Sun>", "");
    crate::org::run(&mut e, "org-evaluate-time-range", Prefix::U(1));
    assert_eq!(text(&e), "x <2026-10-08 Thu>--<2026-10-04 Sun> - 4d");
    let mut e = org("nothing", "");
    crate::org::run(&mut e, "org-evaluate-time-range", Prefix::None);
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "? Not at a timestamp range, and none found in current line"
    );
    civil::set_now(None);
}

#[test]
fn calendar_and_overlays() {
    at("2026-10-04 10:00");
    let mut e = org("See <2026-12-25 Fri>", "");
    crate::org::run(&mut e, "org-goto-calendar", Prefix::None);
    assert!(
        e.msg
            .as_ref()
            .unwrap()
            .0
            .starts_with("Calendar: Friday, December 25, 2026"),
        "{:?}",
        e.msg
    );
    assert!(e.msg.as_ref().unwrap().0.ends_with("Christmas"));
    crate::org::run(&mut e, "org-calendar-forward-day", Prefix::None);
    e.cur.byte = 0;
    crate::org::run(&mut e, "org-date-from-calendar", Prefix::None);
    assert_eq!(text(&e), "<2026-12-26 Sat>See <2026-12-25 Fri>");
    e.cur.byte = 22;
    crate::org::run(&mut e, "org-date-from-calendar", Prefix::None);
    assert_eq!(text(&e), "<2026-12-26 Sat>See <2026-12-26 Sat>");
    let mut e = org("<2026-12-25 Fri>", "");
    assert!(!display_custom_times(&e));
    crate::org::run(&mut e, "org-toggle-timestamp-overlays", Prefix::None);
    assert!(display_custom_times(&e));
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "Time stamps are overlaid with custom format"
    );
    assert_eq!(
        stamp::custom_overlays("<2026-12-25 Fri>"),
        vec![(1..15, "12/25/26 Fri".to_owned())]
    );
    let e = org("#+STARTUP: customtime\n* H", "");
    assert!(display_custom_times(&e));
    set_calendar_date(None);
    civil::set_now(None);
}

#[test]
fn diary_timestamps_absolute() {
    let d = Tm::date(2026, 11, 26).absolute();
    assert_eq!(
        stamp::time_string_to_absolute("%%(diary-float 11 4 4)", Some(d), ""),
        Ok(d)
    );
    assert!(stamp::time_string_to_absolute("%%(diary-float 11 4 4)", Some(d + 1), "").is_err());
    assert_eq!(
        stamp::time_string_to_absolute("<2026-11-26 Thu>", None, ""),
        Ok(d)
    );
    assert_eq!(stamp::small_year_to_year(99), 1999);
    assert_eq!(stamp::days_to_iso_week(d), 48);
    assert_eq!(stamp::get_wdays("2016-06-10 Fri -1w", false, false), 7);
    assert_eq!(stamp::tdiff_string(0, 1, 1, 0), "1 day 1 hour ");
}
