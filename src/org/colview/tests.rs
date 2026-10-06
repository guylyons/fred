//! Column view tests, ported from testing/lisp/test-org-colview.el.

use super::*;
use crate::org::tests::org;

fn spec(p: &str, t: &str, w: Option<usize>, o: Option<&str>, f: Option<&str>) -> Spec {
    Spec {
        property: p.into(),
        title: t.into(),
        width: w,
        operator: o.map(Into::into),
        format: f.map(Into::into),
    }
}

#[test]
fn compile_and_uncompile_formats() {
    assert_eq!(
        compile_format("%ITEM"),
        vec![spec("ITEM", "ITEM", None, None, None)]
    );
    assert_eq!(compile_format("%10ITEM").len(), 1);
    assert_eq!(compile_format("%10ITEM")[0].width, Some(10));
    assert_eq!(
        compile_format("%item"),
        vec![spec("ITEM", "item", None, None, None)]
    );
    assert_eq!(
        compile_format("%ITEM(some title)"),
        vec![spec("ITEM", "some title", None, None, None)]
    );
    assert_eq!(
        compile_format("%ITEM{+;%.1f}"),
        vec![spec("ITEM", "ITEM", None, Some("+"), Some("%.1f"))]
    );
    assert_eq!(
        compile_format("%ITEM(){X}"),
        vec![spec("ITEM", "ITEM", None, Some("X"), None)]
    );
    assert_eq!(compile_format("%A{$}")[0].format.as_deref(), Some("%.2f"));
    for f in [
        "%ITEM",
        "%ITEM %TODO",
        "%10ITEM",
        "%ITEM(some title)",
        "%ITEM{+}",
        "%ITEM{+;%.1f}",
    ] {
        assert_eq!(uncompile_format(&compile_format(f)), f);
    }
}

/// The value shown in the first column of the first row (the
/// `org-columns-value-modified` of point-min upstream).
fn first(text: &str, fmt: &str) -> Vec<String> {
    let mut e = org(text, "");
    let cap = capture(&mut e, false, 0, Some(fmt), false);
    cap.rows[0]
        .cells
        .iter()
        .map(|c| c.displayed.clone())
        .collect()
}

const TWO: &str = "* H\n** S1\n:PROPERTIES:\n:A: {1}\n:END:\n** S1\n:PROPERTIES:\n:A: {2}\n:END:";

fn two(a: &str, b: &str) -> String {
    TWO.replace("{1}", a).replace("{2}", b)
}

#[test]
fn summary_types() {
    assert_eq!(first(&two("1", "2"), "%A{+}"), ["3"]);
    assert_eq!(first(&two("1", "2"), "%A{+;%.1f}"), ["3.0"]);
    assert_eq!(first(&two("1.50", "2.10"), "%A{$}"), ["3.60"]);
    assert_eq!(
        first("* H\n:PROPERTIES:\n:A: 1\n:END:", "%A{+;%.1f}"),
        ["1.0"]
    );
    assert_eq!(first(&two("1:30", "2:40"), "%A{:}"), ["4:10"]);
    assert_eq!(first(&two("1:30", "2"), "%A{:}"), ["1:32"]);
    assert_eq!(first(&two("4:40:10", "1:20"), "%A{:}"), ["6:00:10"]);
    assert_eq!(first(&two("[ ]", "[ ]"), "%A{X}"), ["[ ]"]);
    assert_eq!(first(&two("[ ]", "[X]"), "%A{X}"), ["[-]"]);
    assert_eq!(first(&two("[X]", "[X]"), "%A{X}"), ["[X]"]);
    assert_eq!(first(&two("[ ]", "[X]"), "%A{X/}"), ["[1/2]"]);
    assert_eq!(first(&two("[ ]", "[X]"), "%A{X%}"), ["[50%]"]);
    assert_eq!(first(&two("[10/10]", "[ ]"), "%A{X/}"), ["[1/2]"]);
    assert_eq!(first(&two("99", "42"), "%A{min}"), ["42"]);
    assert_eq!(first(&two("99", "42"), "%A{max}"), ["99"]);
    assert_eq!(first(&two("60", "42"), "%A{mean}"), ["51.0"]);
    assert_eq!(first(&two("4:40", "1:20"), "%A{:min}"), ["1:20"]);
    assert_eq!(first(&two("4:40", "1:20"), "%A{:max}"), ["4:40"]);
    assert_eq!(first(&two("4:40", "1:20"), "%A{:mean}"), ["3:00"]);
    assert_eq!(first(&two("0-10", "0-10"), "%A{est+}"), ["3-17"]);
    assert_eq!(
        first("* H\n** S1\n:PROPERTIES:\n:A: 4\n:END:\n", "%A{est+}"),
        ["4-4"]
    );
    assert_eq!(first(&two("1d 10h 0min", "5d 3h"), "%A{@min}"), ["1d 10h"]);
    assert_eq!(first(&two("99", "42"), "%A{min} %A{max}"), ["42", "99"]);
    // Recursive checkbox summaries.
    assert_eq!(
        first(
            "* H\n** S1\n:PROPERTIES:\n:A: [ ]\n:END:\n** S2\n*** S21\n:PROPERTIES:\n:A: [X]\n:END:\n*** S22\n:PROPERTIES:\n:A: [X]\n:END:",
            "%A{X/}"
        ),
        ["[1/2]"]
    );
    // The first column of a property updates the entry's value.
    let mut e = org(
        "* H\n:PROPERTIES:\n:A: 1000\n:END:\n** S1\n:PROPERTIES:\n:A: 99\n:END:\n** S1\n:PROPERTIES:\n:A: 42\n:END:",
        "",
    );
    let cap = capture(&mut e, false, 0, Some("%A %A{min}"), false);
    assert_eq!(cap.rows[0].cells[0].displayed, "1000");
    assert_eq!(cap.rows[0].cells[1].displayed, "42");
    let cap = capture(&mut e, false, 0, Some("%A{min} %A"), false);
    assert_eq!(cap.rows[0].cells[0].displayed, "42");
    assert_eq!(e.buf.line(2), ":A: 42");
    // Custom summary types are Lisp.
    options::put(
        "org-columns-summary-types",
        toml::Value::String(
            "'((\"custom\" . (lambda (s _) (mapconcat #'identity s \"|\"))))".into(),
        ),
    );
    assert_eq!(first(&two("1", "2"), "%A{custom}"), ["1|2"]);
    options::remove("org-columns-summary-types");
}

#[test]
fn ages_count_from_now() {
    civil::set_now(Some(civil::Tm::date(2014, 3, 4)));
    assert_eq!(
        first(&two("<2012-03-29 Thu>", "<2014-03-04 Tue>"), "%A{@min}"),
        ["0min"]
    );
    assert_eq!(
        first(&two("<2014-03-03 Mon>", "<2014-03-02 Sun>"), "%A{@max}"),
        ["2d"]
    );
    assert_eq!(
        first(&two("<2014-03-03 Mon>", "<2014-03-02 Sun>"), "%A{@mean}"),
        ["1d 12h"]
    );
    civil::set_now(None);
}

#[test]
fn clocksum_columns() {
    options::put("org-duration-format", toml::Value::String("'h:mm".into()));
    let mut e = org(
        "* H
CLOCK: [2022-11-03 06:00]--[2022-11-03 06:03] =>  0:03
** S1
CLOCK: [2022-11-03 06:03]--[2022-11-03 06:05] =>  0:02
** S2
empty
** S3
CLOCK: [2022-11-03 06:05]--[2022-11-03 06:06] =>  0:01",
        "",
    );
    let cap = capture(&mut e, true, 0, Some("%CLOCKSUM"), false);
    let v: Vec<&str> = cap
        .rows
        .iter()
        .map(|r| r.cells[0].displayed.as_str())
        .collect();
    assert_eq!(v, ["0:06", "0:02", "", "0:01"]);
}

#[test]
fn view_text_and_widths() {
    let mut e = org("* TODO H [#A] :t:\n** Sub\n:PROPERTIES:\n:A: 1\n:END:", "");
    let cap = capture(&mut e, false, 0, Some("%25ITEM %TODO %A %TAGS"), false);
    let w = widths(&cap.specs, &cap.rows);
    assert_eq!(w, [25, 4, 1, 4]);
    let shown: Vec<&Row> = cap.rows.iter().collect();
    assert_eq!(
        render(&cap.specs, &w, &shown),
        [
            "ITEM                      | TODO | A | TAGS |",
            "* H [#A]                  | TODO |   | :t:  |",
            "** Sub                    |      | 1 |      |",
        ]
    );
    assert_eq!(column_at(&w, 0), 0);
    assert_eq!(column_at(&w, 28), 1);
    assert_eq!(column_start(&w, 2), 35);
    assert_eq!(add_ellipses("Hello world", 6), "Hell..");
    assert_eq!(add_ellipses("Hello", 1), ".");
}

/// The dblock written at line `b`, BEGIN through END.
fn block(e: &Editor, b: usize) -> String {
    let mut out = vec![];
    for l in b..e.line_count() {
        let t = e.buf.line(l);
        out.push(t.clone());
        if t.starts_with("#+END:") {
            break;
        }
    }
    out.join("\n")
}

fn dblock(text: &str, b: usize, fmt: &str) -> String {
    options::put(
        "org-columns-default-format",
        toml::Value::String(fmt.into()),
    );
    let mut e = org(text, "");
    dblock::update(&mut e, b).unwrap();
    options::remove("org-columns-default-format");
    block(&e, b)
}

#[test]
fn columnview_dblocks() {
    assert_eq!(
        dblock("* H\n#+BEGIN: columnview\n#+END:", 1, "%ITEM"),
        "#+BEGIN: columnview\n| ITEM |\n|------|\n| H    |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H\n:PROPERTIES:\n:A: 1\n:END:\n#+BEGIN: columnview\n#+END:",
            4,
            "%ITEM %A"
        ),
        "#+BEGIN: columnview\n| ITEM | A |\n|------+---|\n| H    | 1 |\n#+END:"
    );
    assert_eq!(
        dblock("* H\n#+BEGIN: columnview\n#+END:", 1, "%5ITEM"),
        "#+BEGIN: columnview\n| <5>  |\n| ITEM |\n|------|\n| H    |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H\n:PROPERTIES:\n:A: 1\n:END:\n#+BEGIN: columnview\n#+END:",
            4,
            "%a"
        ),
        "#+BEGIN: columnview\n| a |\n|---|\n| 1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H\n:PROPERTIES:\n:A: 1\n:END:\n#+BEGIN: columnview\n#+END:",
            4,
            "%ITEM(Name) %A(Prop)"
        ),
        "#+BEGIN: columnview\n| Name | Prop |\n|------+------|\n| H    |    1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H1\n#+BEGIN: columnview :id local\n#+END:\n** H1.1\n* H2",
            1,
            "%ITEM"
        ),
        "#+BEGIN: columnview :id local\n| ITEM |\n|------|\n| H1   |\n| H1.1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* P\n:PROPERTIES:\n:COLUMNS: %ITEM\n:END:\n** C\n#+BEGIN: columnview\n#+END:",
            5,
            "%A"
        ),
        "#+BEGIN: columnview\n| ITEM |\n|------|\n| P    |\n| C    |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H1\n#+BEGIN: columnview :id global\n#+END:\n** H1.1\n* H2",
            2,
            "%ITEM"
        ),
        "#+BEGIN: columnview :id global\n| ITEM |\n|------|\n| H1   |\n| H1.1 |\n| H2   |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H\n#+BEGIN: columnview :hlines t :id global\n#+END:\n* H2\n** H2.1",
            2,
            "%ITEM"
        ),
        "#+BEGIN: columnview :hlines t :id global\n| ITEM |\n|------|\n| H    |\n|------|\n| H2   |\n|------|\n| H2.1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H\n#+BEGIN: columnview :hlines 1 :id global\n#+END:\n* H2\n** H2.1",
            2,
            "%ITEM"
        ),
        "#+BEGIN: columnview :hlines 1 :id global\n| ITEM |\n|------|\n| H    |\n|------|\n| H2   |\n| H2.1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H\n#+BEGIN: columnview :hlines 1 :id \"id\"\n#+END:\n* H2\n:PROPERTIES:\n:ID: id\n:END:\n** H2.1",
            2,
            "%ITEM"
        ),
        "#+BEGIN: columnview :hlines 1 :id \"id\"\n| ITEM |\n|------|\n| H2   |\n| H2.1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H1\n#+BEGIN: columnview :indent t\n#+END:\n** H1.1",
            1,
            "%A(Prop) %ITEM(Name)"
        ),
        "#+BEGIN: columnview :indent t\n| Prop | Name     |\n|------+----------|\n|      | H1       |\n|      | \\_  H1.1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H\n:PROPERTIES:\n:A: 1\n:END:\n#+BEGIN: columnview :vlines t\n#+END:",
            4,
            "%ITEM %A"
        ),
        "#+BEGIN: columnview :vlines t\n|   | ITEM | A  |\n|---+------+----|\n|   | H    | 1  |\n| / | <>   | <> |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H1\n#+BEGIN: columnview :skip-empty-rows t\n#+END:\n** H1.1\n:PROPERTIES:\n:A: 1\n:END:",
            2,
            "%ITEM %A"
        ),
        "#+BEGIN: columnview :skip-empty-rows t\n| ITEM | A |\n|------+---|\n| H1.1 | 1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H1\n#+BEGIN: columnview :exclude-tags (\"excludeme\")\n#+END:\n** H1.1 :excludeme:\n:PROPERTIES:\n:A: 1\n:END:",
            2,
            "%ITEM %A"
        ),
        "#+BEGIN: columnview :exclude-tags (\"excludeme\")\n| ITEM | A |\n|------+---|\n| H1   |   |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H1\n#+BEGIN: columnview :match \"A=1\"\n#+END:\n** H1.1\n:PROPERTIES:\n:A: 1\n:END:\n** H1.2\n:PROPERTIES:\n:A: 2\n:END:",
            2,
            "%ITEM %A"
        ),
        "#+BEGIN: columnview :match \"A=1\"\n| ITEM | A |\n|------+---|\n| H1.1 | 1 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H1\n#+BEGIN: columnview :maxlevel 1 :id global\n#+END:\n** H1.1\n* H2",
            2,
            "%ITEM"
        ),
        "#+BEGIN: columnview :maxlevel 1 :id global\n| ITEM |\n|------|\n| H1   |\n| H2   |\n#+END:"
    );
    assert_eq!(
        dblock(
            "\n* H1\n#+BEGIN: columnview\n#+END:\n** H1.1 :ARCHIVE:\n** COMMENT H1.2\n** H1.3",
            2,
            "%ITEM"
        ),
        "#+BEGIN: columnview\n| ITEM |\n|------|\n| H1   |\n| H1.3 |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H\n#+BEGIN: columnview :format \"%ITEM(Name)\"\n#+END:",
            1,
            "%ITEM"
        ),
        "#+BEGIN: columnview :format \"%ITEM(Name)\"\n| Name |\n|------|\n| H    |\n#+END:"
    );
    assert_eq!(
        dblock("* H <<target>> 1\n#+BEGIN: columnview\n#+END:", 1, "%ITEM"),
        "#+BEGIN: columnview\n| ITEM |\n|------|\n| H 1  |\n#+END:"
    );
    assert_eq!(
        dblock(
            "* H\nSCHEDULED: <2020-05-11 Mon> DEADLINE: <2020-05-14 Thu>\n<2020-06-10 Wed>\n#+BEGIN: columnview\n#+END:",
            3,
            "%ITEM %DEADLINE(d) %SCHEDULED(s) %TIMESTAMP(t)"
        ),
        "#+BEGIN: columnview
| ITEM | d                | s                | t                |
|------+------------------+------------------+------------------|
| H    | [2020-05-14 Thu] | [2020-05-11 Mon] | [2020-06-10 Wed] |
#+END:"
    );
    let linked = dblock("* H\n#+BEGIN: columnview :link t\n#+END:", 1, "%ITEM");
    assert!(linked.contains("::*H][H]]    |"), "{linked}");
    // A formula is kept and computed.
    assert_eq!(
        dblock(
            "* H\n:PROPERTIES:\n:A: 2\n:END:\n#+BEGIN: columnview\n| x |\n#+TBLFM: $3=$2*2\n#+END:",
            4,
            "%ITEM %A"
        ),
        "#+BEGIN: columnview\n| ITEM | A |   |\n|------+---+---|\n| H    | 2 | 4 |\n#+TBLFM: $3=$2*2\n#+END:"
    );
}

#[test]
fn insert_dblock_from_the_dynamic_block_menu() {
    options::put(
        "org-columns-default-format",
        toml::Value::String("%ITEM".into()),
    );
    let mut e = org("* H\n", "j");
    crate::org::call_with(
        &mut e,
        "org-columns-insert-dblock",
        Prefix::None,
        "local".into(),
    )
    .unwrap();
    assert_eq!(
        e.buf.text(),
        "* H\n#+BEGIN: columnview :hlines 1 :id local\n| ITEM |\n|------|\n| H    |\n#+END:\n"
    );
    options::remove("org-columns-default-format");
}

mod session {
    use crate::config::Config;
    use crate::key::parse_keys;
    use crate::session::Session;

    #[test]
    fn column_view_edits_entries_and_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.org");
        std::fs::write(
            &p,
            "#+COLUMNS: %ITEM %A\n* H\n:PROPERTIES:\n:A: 1\n:A_ALL: 1 2 3\n:END:\n** Sub\n",
        )
        .unwrap();
        let cfg = Config::default();
        let (mut s, _) = Session::open(Some(p.clone()), &cfg, &dir.path().join("swap")).unwrap();
        let keys = |s: &mut Session, k: &str| {
            for key in parse_keys(k) {
                s.handle_key(key);
            }
        };
        keys(&mut s, "j<C-c><C-x><C-c>");
        assert_eq!(
            s.ed.org_buffer_name.as_deref(),
            Some("*Org Columns: a.org*")
        );
        assert_eq!(s.ed.buf.text(), "ITEM   | A |\n* H    | 1 |\n** Sub |   |");
        assert_eq!(s.ed.cur.line, 1);
        keys(&mut s, "<Right>n");
        assert_eq!(s.ed.buf.line(1), "* H    | 2 |", "{:?}", s.ed.msg);
        keys(&mut s, "3");
        assert_eq!(s.ed.buf.line(1), "* H    | 3 |", "{:?}", s.ed.msg);
        keys(&mut s, "v");
        assert_eq!(s.ed.msg.as_ref().unwrap().0, "Value is: 3");
        // Widen the column: the format is stored in #+COLUMNS.
        keys(&mut s, ">");
        assert_eq!(s.ed.buf.line(1), "* H    | 3  |");
        keys(&mut s, "q");
        assert!(s.ed.org_buffer_name.is_none());
        let text = s.ed.buf.text();
        assert!(text.starts_with("#+COLUMNS: %ITEM %2A\n"), "{text}");
        assert!(text.contains(":A: 3\n"), "{text}");
    }
}
