//! Clock tests, ported from testing/lisp/test-org-clock.el; expected
//! tables were produced by Emacs with the pinned Org.

use super::super::time::civil::{self, Tm};
use super::super::{Prefix, dblock, options, run, tests::org};
use crate::editor::Editor;

fn hmm() {
    options::put("org-duration-format", toml::Value::String("'h:mm".into()));
}

fn at(t: Tm) {
    civil::set_now(Some(t));
    super::super::set_now(Some(t.epoch()));
}

/// test-org-clock-clocktable-contents: the table of a clocktable with
/// OPTIONS at the top of TEXT, without the caption.
fn ct(text: &str, opts: &str) -> String {
    hmm();
    let mut e = org(&format!("#+BEGIN: clocktable {opts}\n#+END:\n{text}"), "");
    dblock::update(&mut e, 0).unwrap();
    body(&e, 0)
}

fn body(e: &Editor, b: usize) -> String {
    let mut out = vec![];
    for l in b + 1..e.line_count() {
        let t = e.buf.line(l);
        if t.starts_with("#+END:") {
            break;
        }
        if l == b + 1 && t.starts_with("#+CAPTION:") {
            continue;
        }
        out.push(t);
    }
    out.join("\n")
}

const FOO_BAR: &str = "* Foo
CLOCK: [2016-12-28 Wed 11:09]--[2016-12-28 Wed 15:09] =>  4:00
** Bar
CLOCK: [2016-12-28 Wed 13:09]--[2016-12-28 Wed 15:09] =>  2:00";

#[test]
fn clocktable_maxlevel() {
    let two = "| Headline     | Time   |      |
|--------------+--------+------|
| *Total time* | *6:00* |      |
|--------------+--------+------|
| Foo          | 6:00   |      |
| \\_  Bar      |        | 2:00 |";
    assert_eq!(ct(FOO_BAR, ":maxlevel 3"), two);
    assert_eq!(ct(FOO_BAR, ":maxlevel 2"), two);
    assert_eq!(
        ct(FOO_BAR, ":maxlevel 1"),
        "| Headline     | Time   |
|--------------+--------|
| *Total time* | *6:00* |
|--------------+--------|
| Foo          | 6:00   |"
    );
    assert_eq!(
        ct(FOO_BAR, ":maxlevel 0"),
        "| Headline     | Time   |
|--------------+--------|
| *Total time* | *6:00* |
|--------------+--------|"
    );
}

#[test]
fn clocktable_formula_and_lang() {
    assert_eq!(
        ct(
            "* Foo
  CLOCK: [2016-12-28 Wed 11:09]--[2016-12-28 Wed 15:09] =>  4:00
* Bar
  CLOCK: [2016-12-28 Wed 13:09]--[2016-12-28 Wed 15:09] =>  2:00",
            ":maxlevel 1 :formula %"
        ),
        "| Headline     |   Time |     % |
|--------------+--------+-------|
| *Total time* | *6:00* | 100.0 |
|--------------+--------+-------|
| Foo          |   4:00 |  66.7 |
| Bar          |   2:00 |  33.3 |"
    );
    let long = "* Foo
  CLOCK: [2016-12-27 Wed 13:09]--[2016-12-28 Wed 15:09] => 26:00";
    assert_eq!(
        ct(long, ":maxlevel 1 :lang fr"),
        "| En-tête        | Durée   |
|----------------+---------|
| *Durée totale* | *26:00* |
|----------------+---------|
| Foo            | 26:00   |"
    );
    assert_eq!(ct(long, ":maxlevel 1 :lang foo"), ct(long, ":maxlevel 1"));
    // A formula is written and computed.
    assert_eq!(
        ct(
            "* Test\nCLOCK: [2012-03-29 Thu 8:00]--[2012-03-29 Thu 16:40] => 8:40",
            ":formula \"$3=string(\\\"foo\\\")\""
        ),
        "| Headline     | Time   |     |
|--------------+--------+-----|
| *Total time* | *8:40* | foo |
|--------------+--------+-----|
| Test         | 8:40   | foo |
#+TBLFM: $3=string(\"foo\")"
    );
}

#[test]
fn clocktable_columns() {
    let foo_bar = "* Foo
CLOCK: [2016-12-27 Wed 13:09]--[2016-12-28 Wed 15:09] => 26:00
** Bar
CLOCK: [2016-12-27 Wed 13:09]--[2016-12-28 Wed 15:09] => 26:00";
    let compact = "| Headline     |    Time |
|--------------+---------|
| *Total time* | *52:00* |
|--------------+---------|
| Foo          |   52:00 |
| \\_  Bar      |   26:00 |";
    assert_eq!(ct(foo_bar, ":compact t"), compact);
    assert_eq!(ct(foo_bar, ":compact t :indent nil"), compact);
    assert_eq!(ct(foo_bar, ":compact t :level t"), compact);
    assert_eq!(ct(foo_bar, ":tcolumns 1"), compact);
    assert_eq!(
        ct(foo_bar, ":tcolumns 3"),
        "| Headline     | Time    |       |
|--------------+---------+-------|
| *Total time* | *52:00* |       |
|--------------+---------+-------|
| Foo          | 52:00   |       |
| \\_  Bar      |         | 26:00 |"
    );
    assert_eq!(
        ct(
            "* Foo
CLOCK: [2016-12-27 Wed 13:09]--[2016-12-28 Wed 15:09] => 26:00
** Bar
:PROPERTIES:
:A: 1
:END:
CLOCK: [2016-12-27 Wed 13:09]--[2016-12-28 Wed 15:09] => 26:00",
            ":properties (\"A\")"
        ),
        "| A | Headline     | Time    |       |
|---+--------------+---------+-------|
|   | *Total time* | *52:00* |       |
|---+--------------+---------+-------|
|   | Foo          | 52:00   |       |
| 1 | \\_  Bar      |         | 26:00 |"
    );
    assert_eq!(
        ct(
            "* scheduled
SCHEDULED: <2025-10-24 Fri 10:00>
CLOCK: [2025-10-23 Thu 12:00]--[2025-10-23 Thu 13:00] =>  1:00
* inactive timestamp
[2025-10-24 Fri 10:00]
CLOCK: [2025-10-23 Thu 12:00]--[2025-10-23 Thu 13:00] =>  1:00
",
            ":timestamp t"
        ),
        "| Timestamp              | Headline           |   Time |
|------------------------+--------------------+--------|
|                        | *Total time*       | *2:00* |
|------------------------+--------------------+--------|
| <2025-10-24 Fri 10:00> | scheduled          |   1:00 |
| [2025-10-24 Fri 10:00] | inactive timestamp |   1:00 |"
    );
    // Pipes are removed; links point at the headline.
    let pipes = "* Foo | Bar\nCLOCK: [2016-12-27 Wed 13:09]--[2016-12-28 Wed 15:09] => 26:00";
    assert!(ct(pipes, ":indent nil").contains("| Foo Bar      | 26:00   |"));
    let t = ct(
        "* TODO Foo [1/2] :tag:\nCLOCK: [2016-12-27 Wed 13:09]--[2016-12-28 Wed 15:09] => 26:00",
        ":link t",
    );
    assert!(t.contains("::*Foo][Foo]]") && t.contains("[[file:/"), "{t}");
}

#[test]
fn clocktable_match_tags_and_scope() {
    // :match keeps the parents of matching entries.
    let mut e = org("** H1\n\n*** H2 :tag:\n\n*** H3\n", "");
    let _ = &mut e;
    let text = "** H1

*** H2 :tag:
CLOCK: [2026-10-06 Tue 09:00]--[2026-10-06 Tue 11:00] =>  2:00

*** H3
CLOCK: [2026-10-06 Tue 08:00]--[2026-10-06 Tue 09:00] =>  1:00";
    assert_eq!(
        ct(text, ":match \"tag\" :indent nil"),
        "| Headline     | Time   |      |
|--------------+--------+------|
| *Total time* | *2:00* |      |
|--------------+--------+------|
| H1           |        | 2:00 |"
    );
    assert_eq!(
        ct(
            "** H1 :tag:\nCLOCK: [2026-10-06 Tue 01:00]--[2026-10-06 Tue 02:00] =>  1:00\n\n*** H2 ",
            ":tags t :indent nil"
        ),
        "| Tags | Headline     | Time   |      |
|------+--------------+--------+------|
|      | *Total time* | *1:00* |      |
|------+--------------+--------+------|
| tag  | H1           |        | 1:00 |"
    );
    // :scope subtree from a block inside the subtree.
    hmm();
    let mut e = org(
        "* Test
CLOCK: [2012-03-29 Thu 8:00]--[2012-03-29 Thu 16:40] => 8:40
** subtree Test
#+BEGIN: clocktable :scope subtree
#+END:
CLOCK: [2012-03-29 Thu 16:00]--[2012-03-29 Thu 17:00] =>  1:00",
        "",
    );
    dblock::update(&mut e, 3).unwrap();
    assert_eq!(
        body(&e, 3),
        "| Headline         | Time   |      |
|------------------+--------+------|
| *Total time*     | *1:00* |      |
|------------------+--------+------|
| \\_  subtree Test |        | 1:00 |"
    );
    // Invalid clocks are ignored.
    assert_eq!(
        ct(
            "* H1\nCLOCK: [2012-01-01 sun. 00rr:04]--[2012-01-01 sun. 00:05] =>  0:01",
            ""
        ),
        "| Headline     | Time   |
|--------------+--------|
| *Total time* | *0:00* |"
    );
}

#[test]
fn clocktable_ranges_and_steps() {
    at(Tm::at(2026, 10, 6, 12, 0));
    let text = "* Relative times in clocktable
** Foo
CLOCK: [2026-10-03 Sat 08:00]--[2026-10-03 Sat 12:00] =>  4:00
CLOCK: [2026-10-04 Sun 15:00]--[2026-10-04 Sun 18:00] =>  3:00
CLOCK: [2026-10-05 Mon 08:00]--[2026-10-05 Mon 13:00] =>  5:00";
    assert_eq!(
        ct(text, ":tstart \"<-2d>\" :tend \"<today>\" :indent nil"),
        "| Headline                     | Time   |      |
|------------------------------+--------+------|
| *Total time*                 | *8:00* |      |
|------------------------------+--------+------|
| Relative times in clocktable | 8:00   |      |
| Foo                          |        | 8:00 |"
    );
    let steps = ct(
        "* Foo
CLOCK: [2017-09-30 Sat 12:00]--[2017-09-30 Sat 13:00] =>  1:00
CLOCK: [2017-10-01 Sun 11:00]--[2017-10-01 Sun 13:00] =>  2:00
CLOCK: [2017-10-02 Mon 11:00]--[2017-10-02 Mon 14:00] =>  3:00
CLOCK: [2017-10-08 Sun 09:00]--[2017-10-08 Sun 13:00] =>  4:00
CLOCK: [2017-10-09 Mon 09:00]--[2017-10-09 Mon 14:00] =>  5:00",
        ":step week :block 2017-10 :stepskip0 t",
    );
    assert_eq!(
        steps,
        "
Weekly report starting on: [2017-10-01 Sun]
| Headline     | Time   |
|--------------+--------|
| *Total time* | *2:00* |
|--------------+--------|
| Foo          | 2:00   |

Weekly report starting on: [2017-10-02 Mon]
| Headline     | Time   |
|--------------+--------|
| *Total time* | *7:00* |
|--------------+--------|
| Foo          | 7:00   |

Weekly report starting on: [2017-10-09 Mon]
| Headline     | Time   |
|--------------+--------|
| *Total time* | *5:00* |
|--------------+--------|
| Foo          | 5:00   |
"
    );
    let q = ct(
        "* Foo
CLOCK: [2014-03-04 Tue 08:00]--[2014-03-04 Tue 16:00] =>  8:00
CLOCK: [2014-04-03 Thu 08:00]--[2014-04-03 Thu 16:00] =>  8:00
CLOCK: [2014-06-04 Wed 08:00]--[2014-06-04 Wed 16:00] =>  8:00
CLOCK: [2014-07-03 Thu 08:00]--[2014-07-03 Thu 16:00] =>  8:00",
        ":step quarter :block 2014 :stepskip0 t",
    );
    assert!(
        q.contains("Quarterly report starting on: [2014-01-01 Wed]"),
        "{q}"
    );
    assert!(
        q.contains("Quarterly report starting on: [2014-04-01 Tue]"),
        "{q}"
    );
    assert!(q.contains("| *Total time* | *16:00* |"), "{q}");
    civil::set_now(None);
    super::super::set_now(None);
}

#[test]
fn clocktable_emphasis_level_narrow_sort() {
    assert_eq!(
        ct(
            "* Foo heading that is long
CLOCK: [2017-10-02 Mon 11:00]--[2017-10-02 Mon 14:00] =>  3:00
** Bar with a long name here
CLOCK: [2017-10-08 Sun 09:00]--[2017-10-08 Sun 13:00] =>  4:00
*** Baz
CLOCK: [2017-10-08 Sun 09:00]--[2017-10-08 Sun 10:00] =>  1:00",
            ":maxlevel 3 :emphasize t :level t :narrow 10 :link nil :scope file :fileskip0 t"
        ),
        "|   | <10>                            |        |        |      |
| L | Headline                        | Time   |        |      |
|---+---------------------------------+--------+--------+------|
|   | *Total time*                    | *8:00* |        |      |
|---+---------------------------------+--------+--------+------|
| 1 | *Foo heading that is long*      | *8:00* |        |      |
| 2 | \\_  /Bar with a long name here/ |        | /5:00/ |      |
| 3 | \\_    Baz                       |        |        | 1:00 |"
    );
    assert_eq!(
        ct(
            "* Zed heading that is long
CLOCK: [2017-10-02 Mon 11:00]--[2017-10-02 Mon 14:00] =>  3:00
* Alpha with a long name here
CLOCK: [2017-10-08 Sun 09:00]--[2017-10-08 Sun 13:00] =>  4:00",
            ":maxlevel 3 :narrow 12! :sort (1 . ?a) :indent nil"
        ),
        "| Headline     |   Time |
|--------------+--------|
| *Total time* | *7:00* |
|--------------+--------|
| Alpha...     |   4:00 |
| Zed...       |   3:00 |"
    );
}

#[test]
fn clocktable_block_caption_and_shift() {
    at(Tm::at(2026, 10, 6, 12, 0));
    hmm();
    let mut e = org(
        "#+BEGIN: clocktable :block 2026-W40\n#+END:\n* A\nCLOCK: [2026-09-29 Tue 10:00]--[2026-09-29 Tue 11:00] =>  1:00",
        "",
    );
    dblock::update(&mut e, 0).unwrap();
    assert_eq!(
        e.buf.line(1),
        "#+CAPTION: Clock summary at [2026-10-06 Tue 12:00], for week 2026-W40."
    );
    assert!(body(&e, 0).contains("| *Total time* | *1:00* |"));
    run(&mut e, "org-clocktable-shift-right", Prefix::None);
    assert!(
        e.buf
            .line(0)
            .starts_with("#+BEGIN: clocktable :block 2026-W41")
    );
    assert!(body(&e, 0).contains("*0:00*"), "{}", body(&e, 0));
    run(&mut e, "org-shiftleft", Prefix::None);
    assert!(
        e.buf
            .line(0)
            .starts_with("#+BEGIN: clocktable :block 2026-W40")
    );
    civil::set_now(None);
}

#[test]
fn clock_report_inserts_a_block() {
    hmm();
    let mut e = org(
        "* H1\nCLOCK: [2026-10-06 Tue 10:00]--[2026-10-06 Tue 11:00] =>  1:00\nCLOCK: => 1:00\n",
        "jj",
    );
    run(&mut e, "org-clock-report", Prefix::None);
    assert_eq!(
        e.buf.line(3),
        "#+BEGIN: clocktable :scope subtree :maxlevel 2"
    );
    assert_eq!(
        body(&e, 3),
        "| Headline     | Time   |
|--------------+--------|
| *Total time* | *2:00* |
|--------------+--------|
| H1           | 2:00   |"
    );
    let t = super::table::get_clocktable(
        &dblock::Params::parse(":scope (\"/nonexistent.org\")").unwrap(),
    )
    .unwrap();
    assert!(t.starts_with("| File | Headline"), "{t}");
}

#[test]
fn clock_in_and_out() {
    super::reset();
    at(Tm::at(2026, 10, 6, 9, 0));
    let mut e = org("* H\nbody\n** Sub", "");
    run(&mut e, "org-clock-in", Prefix::None);
    assert_eq!(
        e.buf.text(),
        "* H\n:LOGBOOK:\nCLOCK: [2026-10-06 Tue 09:00]\n:END:\nbody\n** Sub"
    );
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "Clock starts at [2026-10-06 Tue 09:00] - showing entire task time."
    );
    assert_eq!(super::mode_line().unwrap(), "[0:00] (H) ");
    at(Tm::at(2026, 10, 6, 10, 30));
    assert_eq!(super::mode_line().unwrap(), "[1:30] (H) ");
    run(&mut e, "org-clock-out", Prefix::None);
    assert_eq!(
        e.buf.line(2),
        "CLOCK: [2026-10-06 Tue 09:00]--[2026-10-06 Tue 10:30] =>  1:30"
    );
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "Clock stopped at [2026-10-06 Tue 10:30] after 1:30"
    );
    assert!(super::running().is_none());
    // Clocking in again goes into the drawer, newest first.
    at(Tm::at(2026, 10, 6, 11, 0));
    run(&mut e, "org-clock-in", Prefix::None);
    assert_eq!(e.buf.line(2), "CLOCK: [2026-10-06 Tue 11:00]");
    // Marking the entry DONE clocks out.
    run(&mut e, "org-todo", Prefix::None);
    run(&mut e, "org-todo", Prefix::None);
    assert!(super::running().is_none());
    assert!(e.buf.line(2).ends_with("=>  0:00"), "{}", e.buf.text());
    // Clocking into a subheading interrupts nothing; org-clock-in-last
    // returns to the last task.
    e.set_cursor(6, 0);
    run(&mut e, "org-clock-in", Prefix::None);
    assert_eq!(super::running().unwrap().heading, "Sub");
    e.set_cursor(0, 0);
    run(&mut e, "org-clock-in", Prefix::None);
    assert!(super::running().unwrap().heading.ends_with("H"));
    assert_eq!(super::history().len(), 2);
    run(&mut e, "org-clock-cancel", Prefix::None);
    assert!(super::running().is_none());
    assert!(!e.buf.text().contains("CLOCK: [2026-10-06 Tue 11:00]\n"));
    assert_eq!(e.msg.as_ref().unwrap().0, "Clock canceled");
    super::reset();
    civil::set_now(None);
}

#[test]
fn clock_drawer_options() {
    super::reset();
    at(Tm::at(2026, 10, 6, 9, 0));
    options::put("org-clock-into-drawer", toml::Value::Boolean(false));
    let mut e = org("* H\nbody", "");
    run(&mut e, "org-clock-in", Prefix::None);
    assert_eq!(e.buf.text(), "* H\nCLOCK: [2026-10-06 Tue 09:00]\nbody");
    super::reset();
    options::put("org-clock-into-drawer", toml::Value::Integer(2));
    let mut e = org(
        "* H\n- item\n  CLOCK: [2020-01-01 Wed 10:00]--[2020-01-01 Wed 11:00] =>  1:00\n",
        "",
    );
    run(&mut e, "org-clock-in", Prefix::None);
    assert_eq!(
        e.buf.text(),
        "* H\n:LOGBOOK:\nCLOCK: [2020-01-01 Wed 10:00]--[2020-01-01 Wed 11:00] =>  1:00\nCLOCK: [2026-10-06 Tue 09:00]\n:END:\n- item"
    );
    super::reset();
    options::put("org-clock-into-drawer", toml::Value::String("FOO".into()));
    let e = org("* H", "");
    assert_eq!(super::drawer_name(&e, 0).as_deref(), Some("FOO"));
    options::remove("org-clock-into-drawer");
    let e = org(
        "* H\n:PROPERTIES:\n:CLOCK_INTO_DRAWER: nil\n:END:\n** H2",
        "",
    );
    assert_eq!(super::into_drawer(&e, 4), super::IntoDrawer::No);
    let e = org("* H\n:PROPERTIES:\n:CLOCK_INTO_DRAWER: 3\n:END:", "");
    assert_eq!(super::drawer_name(&e, 0).as_deref(), Some("LOGBOOK"));
    // Zero time clocks can be removed with their drawer.
    options::put(
        "org-clock-out-remove-zero-time-clocks",
        toml::Value::Boolean(true),
    );
    let mut e = org("* H\nbody", "");
    run(&mut e, "org-clock-in", Prefix::None);
    run(&mut e, "org-clock-out", Prefix::None);
    assert_eq!(e.buf.text(), "* H\nbody");
    assert!(e.msg.as_ref().unwrap().0.ends_with("=> LINE REMOVED"));
    options::remove("org-clock-out-remove-zero-time-clocks");
    super::reset();
    civil::set_now(None);
}

#[test]
fn clock_lines_update_and_shift() {
    let mut e = org("CLOCK: [2023-04-29 Sat 00:00]--[2023-05-04 Thu 01:00]", "");
    run(&mut e, "org-ctrl-c-ctrl-c", Prefix::None);
    assert_eq!(
        e.buf.text(),
        "CLOCK: [2023-04-29 Sat 00:00]--[2023-05-04 Thu 01:00] => 121:00"
    );
    let mut e = org(
        "CLOCK: [2023-02-19 Sun 22:30]--[2023-02-20 Mon 00:35] =>  2:05",
        "",
    );
    e.set_cursor(0, 24);
    super::timestamps_change(&mut e, false, 1).unwrap();
    assert_eq!(
        e.buf.text(),
        "CLOCK: [2023-02-19 Sun 21:30]--[2023-02-19 Sun 23:35] =>  2:05"
    );
    let mut e = org(
        "CLOCK: [2023-02-19 Sun 23:30]--[2023-02-20 Mon 00:35] =>  1:05",
        "",
    );
    e.set_cursor(0, 24);
    super::timestamps_change(&mut e, true, 1).unwrap();
    assert_eq!(
        e.buf.text(),
        "CLOCK: [2023-02-20 Mon 00:30]--[2023-02-20 Mon 01:35] =>  1:05"
    );
    let mut e = org("[2026-03-28 23:33]\n[2026-03-29 00:33]", "");
    e.set_cursor(0, 9);
    super::timestamps_change(&mut e, true, 1).unwrap();
    assert_eq!(e.buf.text(), "[2026-03-28 23:33]\n[2026-03-29 00:33]");
}

#[test]
fn mode_line_strings() {
    let mut r = super::Running {
        task: super::Marker {
            file: None,
            line: 0,
            heading: String::new(),
            category: String::new(),
            label: String::new(),
        },
        clock_line: 1,
        stamp: String::new(),
        start: civil::now_epoch(),
        heading: "This is a long headline blah blah blah".into(),
        effort: None,
        total: 0,
    };
    assert_eq!(super::clock_string(&r, 20), "[0:00] (This is a…) ");
    assert_eq!(super::clock_string(&r, 12), "[0:00] (T…) ");
    assert_eq!(super::clock_string(&r, 10), "[0:00] ");
    assert_eq!(super::clock_string(&r, 3), "[0: ");
    r.effort = Some("1h".into());
    assert_eq!(super::clock_string(&r, 20), "[0:00/1:00] (This…) ");
    r.heading = "Heading".into();
    assert_eq!(super::clock_string(&r, 0), "[0:00/1:00] (Heading) ");
}

#[test]
fn clock_sums_and_display() {
    hmm();
    let mut e = org(
        "* H
CLOCK: [2022-11-03 06:00]--[2022-11-03 06:03] =>  0:03
** S1
CLOCK: [2022-11-03 06:03]--[2022-11-03 06:05] =>  0:02
** S2
empty",
        "",
    );
    assert_eq!(
        super::super::props::get(&e, Some(0), "CLOCKSUM", super::super::props::Inherit::No)
            .as_deref(),
        Some("0:05")
    );
    assert_eq!(
        super::super::props::get(&e, Some(4), "CLOCKSUM", super::super::props::Inherit::No),
        None
    );
    run(&mut e, "org-clock-display", Prefix::Num(1));
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "Total file time: 0:05 (0 hours and 5 minutes)"
    );
    let shown = super::display(&e, 0, "* H").unwrap();
    assert_eq!(shown, format!("* H{}      0:05 ", "·".repeat(57)));
    assert!(super::display(&e, 4, "** S2").is_none());
    run(&mut e, "org-clock-remove-overlays", Prefix::None);
    assert!(super::display(&e, 0, "* H").is_none());
    // CLOCKSUM_T counts today's part of the clocks.
    at(Tm::date(2022, 11, 3));
    let e = org(
        "* H
CLOCK: [2022-11-02 12:00]--[2022-11-03 02:00] =>  14:00
** S1
CLOCK: [2022-11-03 23:50]--[2022-11-04 01:50] =>  2:00
** S3
CLOCK: [2022-11-03 06:05]--[2022-11-03 06:06] =>  0:01
",
        "",
    );
    let get =
        |h| super::super::props::get(&e, Some(h), "CLOCKSUM_T", super::super::props::Inherit::No);
    assert_eq!(get(0).as_deref(), Some("2:11"));
    assert_eq!(get(2).as_deref(), Some("0:10"));
    civil::set_now(None);
}
