//! Tests ported from testing/lisp/test-org-table.el, plus the Calc and
//! Lisp evaluators against outputs recorded from Emacs.

use super::doc::{Doc, Row, to_lisp_text};
use super::formula::{self, Lispp, Recalc, Refv};
use super::orgtbl::{Params, collapse_header, translate};
use super::{calc, edit, lisp, orgtbl};
use crate::org::options;

/// org-test-with-temp-text: run `f` on `text` (with `<point>`), return the text.
fn with(text: &str, f: impl FnOnce(&mut Doc) -> Result<(), String>) -> String {
    let mut d = Doc::new(text);
    f(&mut d).unwrap();
    d.text()
}

fn fails(text: &str, f: impl FnOnce(&mut Doc) -> Result<(), String>) -> bool {
    let mut d = Doc::new(text);
    f(&mut d).is_err()
}

/// The expected buffer text (Emacs' trailing newline dropped).
fn t(s: &str) -> String {
    s.strip_suffix('\n').unwrap_or(s).to_owned()
}

/// org-test-table-target-expect.
fn target_expect(target: &str, expect: Option<&str>, laps: usize, tblfms: &[&str]) {
    let expect = expect.unwrap_or(target);
    let tblfms: Vec<&str> = if tblfms.is_empty() { vec![""] } else { tblfms.to_vec() };
    for tblfm in tblfms {
        let mut d = Doc::new(&format!("{target}{tblfm}"));
        d.goto(d.n() - 1, 0);
        while !d.at_table() {
            assert!(d.l > 0, "no table");
            d.goto(d.l - 1, 0);
        }
        for _ in 0..laps {
            formula::recalculate(&mut d, Recalc::All, true).unwrap_or_else(|e| panic!("{e}\n{tblfm}"));
        }
        d.align();
        assert_eq!(d.text(), t(&format!("{expect}{tblfm}")), "TBLFM: {tblfm}");
    }
}

fn set_opt(name: &str, v: toml::Value) {
    options::put(name, v);
}

fn reset_opts() {
    options::set(toml::Table::new());
}

// ---- simple formulas ----

#[test]
fn simple_formula_no_grouping() {
    target_expect(
        "\n|       2 |\n|       4 |\n|       8 |\n| replace |\n",
        Some("\n|  2 |\n|  4 |\n|  8 |\n| 14 |\n"),
        1,
        &["#+TBLFM: @>$1 = vsum(@<..@>>)", "#+TBLFM: @>$1 = '(+ @<..@>>); N"],
    );
    target_expect(
        "\n|     foo |\n|---------|\n|       2 |\n|       4 |\n|       8 |\n| replace |\n",
        Some("\n| foo |\n|-----|\n|   2 |\n|   4 |\n|   8 |\n|  14 |\n"),
        1,
        &["#+TBLFM: @>$1 = vsum(@I..@>>)", "#+TBLFM: @>$1 = '(+ @I..@>>); N"],
    );
}

#[test]
fn simple_formula_with_grouping() {
    target_expect(
        "\n|       2 |\n|       4 |\n|       8 |\n|---------|\n| replace |\n",
        Some("\n|  2 |\n|  4 |\n|  8 |\n|----|\n| 14 |\n"),
        1,
        &["#+TBLFM: $1 = vsum(@<..@>>)", "#+TBLFM: $1 = '(+ @<..@>>); N"],
    );
    target_expect(
        "\n|       2 | header  |\n|       4 | header  |\n|       8 | header  |\n|---------+---------|\n| replace | replace |\n",
        Some("\n|  2 | header |\n|  4 | header |\n|  8 | header |\n|----+--------|\n| 14 | 28     |\n"),
        2,
        &["#+TBLFM: @>$1 = vsum(@<..@>>) :: $2 = 2 * $1", "#+TBLFM: @>$1 = '(+ @<..@>>); N :: $2 = '(* 2 $1); N"],
    );
    target_expect(
        "\n|     foo | bar     |\n|---------+---------|\n|       2 | replace |\n|       4 | replace |\n|       8 | replace |\n|---------+---------|\n| replace | replace |\n",
        Some("\n| foo | bar |\n|-----+-----|\n|   2 |   4 |\n|   4 |   8 |\n|   8 |  16 |\n|-----+-----|\n|  14 |  28 |\n"),
        2,
        &["#+TBLFM: @>$1 = vsum(@I..@>>) :: $2 = 2 * $1", "#+TBLFM: @>$1 = '(+ @I..@>>); N :: $2 = '(* 2 $1); N"],
    );
}

// ---- references and mode strings ----

const TARGET_NORMAL: &str = "
| 0 | 1 | replace | replace | replace | replace | replace | replace |
| z | 1 | replace | replace | replace | replace | replace | replace |
|   | 1 | replace | replace | replace | replace | replace | replace |
|   |   | replace | replace | replace | replace | replace | replace |
";

const TARGET_SPECIAL: &str = "
|  nan | 1 | replace | replace | replace | replace | replace | replace |
| uinf | 1 | replace | replace | replace | replace | replace | replace |
| -inf | 1 | replace | replace | replace | replace | replace | replace |
|  inf | 1 | replace | replace | replace | replace | replace | replace |
";

#[test]
fn references_mode_string_el() {
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 | 0 |      1 |      1 |      1 | 2 | 2 |
| z | 1 | z | #ERROR | #ERROR | #ERROR | 2 | 2 |
|   | 1 |   |      1 |      1 |      1 | 1 | 1 |
|   |   |   |      0 |      0 |      0 | 0 | 0 |
",
        ),
        1,
        &[concat!(
            "#+TBLFM: $3 = '(identity \"$1\"); EL :: $4 = '(+ $1 $2); EL :: ",
            "$5 = '(+ $1..$2); EL :: $6 = '(+ @0$1..@0$2); EL :: ",
            "$7 = '(length '($1..$2)); EL :: $8 = '(length '(@0$1..@0$2)); EL"
        )],
    );
    target_expect(
        r#"
| "0" | "1" | repl | repl | repl | repl | repl | repl |
| "z" | "1" | repl | repl | repl | repl | repl | repl |
| ""  | "1" | repl | repl | repl | repl | repl | repl |
| ""  | ""  | repl | repl | repl | repl | repl | repl |
"#,
        Some(
            r#"
| "0" | "1" | "0" | 1 | #ERROR | #ERROR | 2 | 2 |
| "z" | "1" | "z" | 1 | #ERROR | #ERROR | 2 | 2 |
| ""  | "1" | ""  | 1 | #ERROR | #ERROR | 2 | 2 |
| ""  | ""  | ""  | 0 | #ERROR | #ERROR | 2 | 2 |
"#,
        ),
        1,
        &[concat!(
            "#+TBLFM: $3 = '(concat \"\\\"\" $1 \"\\\"\"); EL :: ",
            "$4 = '(+ (string-to-number $1) (string-to-number $2)); EL :: ",
            "$5 = '(+ $1..$2); EL :: $6 = '(+ @0$1..@0$2); EL :: ",
            "$7 = '(length '($1..$2)); EL :: $8 = '(length '(@0$1..@0$2)); EL"
        )],
    );
}

#[test]
fn references_mode_string_e() {
    let lisp = concat!(
        "#+TBLFM: $3 = '(identity $1); E :: $4 = '(+ $1 $2); E :: ",
        "$5 = '(+ $1..$2); E :: $6 = '(+ @0$1..@0$2); E :: ",
        "$7 = '(length '($1..$2)); E :: $8 = '(length '(@0$1..@0$2)); E"
    );
    let calc = concat!(
        "#+TBLFM: $3 = $1; E :: $4 = $1 + $2; E :: ",
        "$5 = vsum($1..$2); E :: $6 = vsum(@0$1..@0$2); E :: ",
        "$7 = vlen($1..$2); E :: $8 = vlen(@0$1..@0$2); E"
    );
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 | 0 | #ERROR | #ERROR | #ERROR | 2 | 2 |
| z | 1 | z | #ERROR | #ERROR | #ERROR | 2 | 2 |
|   | 1 |   | #ERROR | #ERROR | #ERROR | 2 | 2 |
|   |   |   | #ERROR | #ERROR | #ERROR | 2 | 2 |
",
        ),
        1,
        &[lisp],
    );
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 |   0 |     1 |     1 |     1 | 2 | 2 |
| z | 1 |   z | z + 1 | z + 1 | z + 1 | 2 | 2 |
|   | 1 | nan |   nan |   nan |   nan | 2 | 2 |
|   |   | nan |   nan |   nan |   nan | 2 | 2 |
",
        ),
        1,
        &[calc],
    );
    target_expect(
        TARGET_SPECIAL,
        Some(
            "
|  nan | 1 |  nan |  nan |  nan |  nan | 2 | 2 |
| uinf | 1 | uinf | uinf | uinf | uinf | 2 | 2 |
| -inf | 1 | -inf | -inf | -inf | -inf | 2 | 2 |
|  inf | 1 |  inf |  inf |  inf |  inf | 2 | 2 |
",
        ),
        1,
        &[calc],
    );
}

#[test]
fn references_mode_string_en() {
    let lisp = concat!(
        "#+TBLFM: $3 = '(identity $1); EN :: $4 = '(+ $1 $2); EN :: ",
        "$5 = '(+ $1..$2); EN :: $6 = '(+ @0$1..@0$2); EN :: ",
        "$7 = '(length '($1..$2)); EN :: ",
        "$8 = '(length '(@0$1..@0$2)); EN"
    );
    let calc = concat!(
        "#+TBLFM: $3 = $1; EN :: $4 = $1 + $2; EN :: ",
        "$5 = vsum($1..$2); EN :: $6 = vsum(@0$1..@0$2); EN :: ",
        "$7 = vlen($1..$2); EN :: $8 = vlen(@0$1..@0$2); EN"
    );
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
| z | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
|   | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
|   |   | 0 | 0 | 0 | 0 | 2 | 2 |
",
        ),
        1,
        &[lisp, calc],
    );
    target_expect(
        TARGET_SPECIAL,
        Some(
            "
|  nan | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
| uinf | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
| -inf | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
|  inf | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
",
        ),
        1,
        &[calc],
    );
}

#[test]
fn references_mode_string_l() {
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 | 0 |      1 |      1 |      1 | 2 | 2 |
| z | 1 | z | #ERROR | #ERROR | #ERROR | 2 | 2 |
|   | 1 |   |      1 |      1 |      1 | 1 | 1 |
|   |   |   |      0 |      0 |      0 | 0 | 0 |
",
        ),
        1,
        &[concat!(
            "#+TBLFM: $3 = '(identity \"$1\"); L :: $4 = '(+ $1 $2); L :: ",
            "$5 = '(+ $1..$2); L :: $6 = '(+ @0$1..@0$2); L :: ",
            "$7 = '(length '($1..$2)); L :: $8 = '(length '(@0$1..@0$2)); L"
        )],
    );
}

#[test]
fn references_mode_string_none() {
    let lisp = concat!(
        "#+TBLFM: $3 = '(identity $1) :: $4 = '(+ $1 $2) :: ",
        "$5 = '(+ $1..$2) :: $6 = '(+ @0$1..@0$2) :: ",
        "$7 = '(length '($1..$2)) :: $8 = '(length '(@0$1..@0$2))"
    );
    let calc = concat!(
        "#+TBLFM: $3 = $1 :: $4 = $1 + $2 :: ",
        "$5 = vsum($1..$2) :: $6 = vsum(@0$1..@0$2) :: ",
        "$7 = vlen($1..$2) :: $8 = vlen(@0$1..@0$2)"
    );
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 | 0 | #ERROR | #ERROR | #ERROR | 2 | 2 |
| z | 1 | z | #ERROR | #ERROR | #ERROR | 2 | 2 |
|   | 1 |   | #ERROR | #ERROR | #ERROR | 1 | 1 |
|   |   |   | #ERROR | 0      | 0      | 0 | 0 |
",
        ),
        1,
        &[lisp],
    );
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 | 0 |     1 |     1 |     1 | 2 | 2 |
| z | 1 | z | z + 1 | z + 1 | z + 1 | 2 | 2 |
|   | 1 | 0 |     1 |     1 |     1 | 1 | 1 |
|   |   | 0 |     0 |     0 |     0 | 0 | 0 |
",
        ),
        1,
        &[calc],
    );
    target_expect(
        TARGET_SPECIAL,
        Some(
            "
|  nan | 1 |  nan |  nan |  nan |  nan | 2 | 2 |
| uinf | 1 | uinf | uinf | uinf | uinf | 2 | 2 |
| -inf | 1 | -inf | -inf | -inf | -inf | 2 | 2 |
|  inf | 1 |  inf |  inf |  inf |  inf | 2 | 2 |
",
        ),
        1,
        &[calc],
    );
}

#[test]
fn references_mode_string_n() {
    let lisp = concat!(
        "#+TBLFM: $3 = '(identity $1); N :: $4 = '(+ $1 $2); N :: ",
        "$5 = '(+ $1..$2); N :: $6 = '(+ @0$1..@0$2); N :: ",
        "$7 = '(length '($1..$2)); N :: $8 = '(length '(@0$1..@0$2)); N"
    );
    let calc = concat!(
        "#+TBLFM: $3 = $1; N :: $4 = $1 + $2; N :: ",
        "$5 = vsum($1..$2); N :: $6 = vsum(@0$1..@0$2); N :: ",
        "$7 = vlen($1..$2); N :: $8 = vlen(@0$1..@0$2); N"
    );
    target_expect(
        TARGET_NORMAL,
        Some(
            "
| 0 | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
| z | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
|   | 1 | 0 | 1 | 1 | 1 | 1 | 1 |
|   |   | 0 | 0 | 0 | 0 | 0 | 0 |
",
        ),
        1,
        &[lisp, calc],
    );
    target_expect(
        TARGET_SPECIAL,
        Some(
            "
|  nan | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
| uinf | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
| -inf | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
|  inf | 1 | 0 | 1 | 1 | 1 | 2 | 2 |
",
        ),
        1,
        &[calc],
    );
}

#[test]
fn mode_string_u() {
    target_expect("\n| 1.5 A/B | 2.0 B | |\n", Some("\n| 1.5 A/B | 2.0 B | 3. A |\n"), 1, &["#+TBLFM: $3=$1*$2;u"]);
}

#[test]
fn lisp_return_value() {
    target_expect(
        r#"
|                         | nil         | (list) | '() |
|-------------------------+-------------+--------+-----|
| type-of, no L           | replace (r) | r      | r   |
| type-of identity, no L  | r           | r      | r   |
| identity, no L          | r           | r      | r   |
|-------------------------+-------------+--------+-----|
| type-of "@1"            | r           | r      | r   |
| type-of (identity "@1") | r           | r      | r   |
| identity "@1"           | r           | r      | r   |
|-------------------------+-------------+--------+-----|
| type-of @1              | r           | r      | r   |
| type-of (identity @1)   | r           | r      | r   |
| identity @1             | r           | r      | r   |
"#,
        Some(
            r#"
|                         | nil    | (list) | '()    |
|-------------------------+--------+--------+--------|
| type-of, no L           | string | string | string |
| type-of identity, no L  | string | string | string |
| identity, no L          | nil    | (list) | '()    |
|-------------------------+--------+--------+--------|
| type-of "@1"            | string | string | string |
| type-of (identity "@1") | string | string | string |
| identity "@1"           | nil    | (list) | '()    |
|-------------------------+--------+--------+--------|
| type-of @1              | symbol | symbol | symbol |
| type-of (identity @1)   | symbol | symbol | symbol |
| identity @1             | nil    | nil    | nil    |
"#,
        ),
        1,
        &[concat!(
            "#+TBLFM: @2$<<..@2$> = '(type-of @1) :: ",
            "@3$<<..@3$> = '(type-of (identity @1)) :: ",
            "@4$<<..@4$> = '(identity @1) :: @5$<<..@>$> = '(@0$1); L"
        )],
    );
}

#[test]
fn compare() {
    target_expect(
        "
|      | 0    | z    |      | nan  | uinf | -inf | inf  |
|------+------+------+------+------+------+------+------|
|    0 | repl | repl | repl | repl | repl | repl | repl |
|    z | repl | repl | repl | repl | repl | repl | repl |
|      | repl | repl | repl | repl | repl | repl | repl |
|  nan | repl | repl | repl | repl | repl | repl | repl |
| uinf | repl | repl | repl | repl | repl | repl | repl |
| -inf | repl | repl | repl | repl | repl | repl | repl |
|  inf | repl | repl | repl | repl | repl | repl | repl |
",
        Some(
            "
|      | 0 | z |   | nan | uinf | -inf | inf |
|------+---+---+---+-----+------+------+-----|
|    0 | x |   |   |     |      |      |     |
|    z |   | x |   |     |      |      |     |
|      |   |   | x |     |      |      |     |
|  nan |   |   |   |   x |      |      |     |
| uinf |   |   |   |     |    x |      |     |
| -inf |   |   |   |     |      |    x |     |
|  inf |   |   |   |     |      |      |   x |
",
        ),
        1,
        &[
            "#+TBLFM: @<<$<<..@>$> = if(\"$1\" == \"@1\", x, string(\"\")); E",
            concat!(
                "#+TBLFM: ",
                "$2 = if(\"$1\" == \"(0)\"   , x, string(\"\")); E :: ",
                "$3 = if(\"$1\" == \"(z)\"   , x, string(\"\")); E :: ",
                "$4 = if(\"$1\" == \"nan\"   , x, string(\"\")); E :: ",
                "$5 = if(\"$1\" == \"(nan)\" , x, string(\"\")); E :: ",
                "$6 = if(\"$1\" == \"(uinf)\", x, string(\"\")); E :: ",
                "$7 = if(\"$1\" == \"(-inf)\", x, string(\"\")); E :: ",
                "$8 = if(\"$1\" == \"(inf)\" , x, string(\"\")); E"
            ),
        ],
    );
    target_expect(
        "\n|   0 | replace |\n|   z | replace |\n|     | replace |\n| nan | replace |\n",
        Some("\n|   0 |     1 |\n|   z | z + 1 |\n|     |       |\n| nan |   nan |\n"),
        1,
        &["#+TBLFM: $2 = if(\"$1\" == \"nan\", string(\"\"), $1 + 1); E"],
    );
}

#[test]
fn empty_field() {
    target_expect(
        "\n| -1 | replace |\n|  0 | replace |\n|    | replace |\n",
        Some("\n| -1 | 0 |\n|  0 | 1 |\n|    |   |\n"),
        1,
        &["#+TBLFM: $2 = if(\"$1\" == \"nan\", string(\"\"), $1 + 1); E", "#+TBLFM: $2 = '(if (eq \"$1\" \"\") \"\" (1+ $1)); L"],
    );
    target_expect(
        "\n| 1 | 2 | replace |\n| 4 |   | replace |\n|   | 8 | replace |\n|   |   | replace |\n",
        Some("\n| 1 | 2 | 3 |\n| 4 |   |   |\n|   | 8 |   |\n|   |   |   |\n"),
        1,
        &[
            "#+TBLFM: $3 = if(\"$1\" == \"nan\" || \"$2\" == \"nan\", string(\"\"), $1 + $2); E",
            "#+TBLFM: $3 = '(if (or (eq \"$1\" \"\") (eq \"$2\" \"\")) \"\" (+ $1 $2)); L",
        ],
    );
    target_expect(
        "\n| 1.5 | 0 |\n| 3.5 |   |\n|     | 5 |\n|     |   |\n",
        Some("\n| 1.5 | 2.0 |\n| 3.5 | 4.0 |\n|     | 5.0 |\n|     |     |\n"),
        1,
        &["#+TBLFM: $2 = if(\"$1\" == \"nan\", if(\"$2\" == \"nan\", string(\"\"), $2 +.0), $1 + 0.5); E f-1"],
    );
    target_expect(
        "
|   |   |   |   | repl | repl | repl | repl | repl | repl |
|   |   | 5 | 7 | repl | repl | repl | repl | repl | repl |
| 1 | 3 | 5 | 7 | repl | repl | repl | repl | repl | repl |
",
        Some(
            "
|   |   |   |   |   |   |   |   | 0 | 0 |
|   |   | 5 | 7 |   |   | 6 | 6 | 3 | 3 |
| 1 | 3 | 5 | 7 | 4 | 4 | 4 | 4 | 4 | 4 |
",
        ),
        1,
        &[
            concat!(
                "#+TBLFM: ",
                "$5 = if(typeof(vmean($1..$4)) == 12, ",
                "string(\"\"), vmean($1..$4)); E :: ",
                "$6 = if(typeof(vmean(@0$1..@0$4)) == 12, ",
                "string(\"\"), vmean(@0$1..@0$4)); E :: ",
                "$7 = if(\"$1..$4\" == \"[]\", string(\"\"), vmean($1..$4)) :: ",
                "$8 = if(\"@0$1..@0$4\" == \"[]\", string(\"\"), vmean(@0$1..@0$4)) :: ",
                "$9 = vmean($1..$4); EN :: ",
                "$10 = vmean(@0$1..@0$4); EN"
            ),
            concat!(
                "#+TBLFM: ",
                "$5 = '(let ((l '($1..$4))) (if (member \"\" l) \"\" ",
                "(/ (apply '+ (mapcar 'string-to-number l)) (length l)))); E :: ",
                "$6 = '(let ((l '(@0$1..@0$4))) (if (member \"\" l) \"\" ",
                "(/ (apply '+ (mapcar 'string-to-number l)) (length l)))); E :: ",
                "$7 = '(let ((l '($1..$4))) ",
                "(if l (/ (apply '+ l) (length l)) \"\")); N :: ",
                "$8 = '(let ((l '(@0$1..@0$4))) ",
                "(if l (/ (apply '+ l) (length l)) \"\")); N :: ",
                "$9 = '(/ (+ $1..$4) (length '($1..$4))); EN :: ",
                "$10 = '(/ (+ @0$1..@0$4) (length '(@0$1..@0$4))); EN"
            ),
        ],
    );
}

#[test]
fn copy_field() {
    let target = "
| 0                | replace |
| a b              | replace |
| c   d            | replace |
|                  | replace |
| 2012-12          | replace |
| [2012-12-31 Mon] | replace |
";
    let literal = "
| 0                | 0                |
| a b              | a b              |
| c   d            | c   d            |
|                  |                  |
| 2012-12          | 2012-12          |
| [2012-12-31 Mon] | [2012-12-31 Mon] |
";
    target_expect(target, Some(literal), 1, &["#+TBLFM: $2 = '(identity $1)"]);
    target_expect(target, Some(literal), 1, &["#+TBLFM: $2 = if(\"$1\" == \"nan\", string(\"\"), string(subvec(\"$1\", 2, vlen(\"$1\")))); E"]);
    target_expect(
        target,
        Some(
            "
| 0                | 0                |
| a b              | a b              |
| c   d            | c d              |
|                  |                  |
| 2012-12          | 2000             |
| [2012-12-31 Mon] | [2012-12-31 Mon] |
",
        ),
        1,
        &["#+TBLFM: $2 = if(\"$1\" == \"nan\", string(\"\"), $1); E"],
    );
}

#[test]
fn copy_down() {
    reset_opts();
    assert!(fails("|  |\n| <point> |", |d| edit::copy_down(d, 1)));
    assert!(fails("|    |\n| foo |\n| <point> |", |d| edit::copy_down(d, 2)));
    assert_eq!(with("| foo |\n| <point> |", |d| edit::copy_down(d, 1)), "| foo |\n| foo |");
    assert_eq!(with("| <point>foo |", |d| edit::copy_down(d, 1)), "| foo |\n| foo |");
    assert_eq!(with("| <point>1 |", |d| edit::copy_down(d, 1)), "| 1 |\n| 2 |");
    assert!(with("| <point><2012-03-29> |", |d| edit::copy_down(d, 1)).contains("<2012-03-30"));
    assert_eq!(with("| <point>A1 |", |d| edit::copy_down(d, 1)), "| A1 |\n| A2 |");
    assert_eq!(with("| <point>1A |", |d| edit::copy_down(d, 1)), "| 1A |\n| 2A |");
    set_opt("org-table-copy-increment", toml::Value::Boolean(false));
    assert_eq!(with("| <point>1 |", |d| edit::copy_down(d, 1)), "| 1 |\n| 1 |");
    reset_opts();
    assert_eq!(with("| <point>1 |", |d| edit::copy_down(d, 0)), "| 1 |\n| 1 |");
    assert_eq!(with("| 4 |\n| <point>3 |", |d| edit::copy_down(d, 1)), "| 4 |\n| 3 |\n| 2 |");
    assert_eq!(with("| A0 |\n| <point>A2 |", |d| edit::copy_down(d, 1)), "| A0 |\n| A2 |\n| A4 |");
    assert_eq!(with("| A4 |\n| <point>3 |", |d| edit::copy_down(d, 1)), "| A4 |\n|  3 |\n|  4 |");
    assert_eq!(with("| 0A |\n| <point>A2 |", |d| edit::copy_down(d, 1)), "| 0A |\n| A2 |\n| A3 |");
    assert_eq!(with("| A0 |\n| <point>2A |", |d| edit::copy_down(d, 1)), "| A0 |\n| 2A |\n| 3A |");
    assert_eq!(with("| 4 |\n|---|\n| <point>3 |", |d| edit::copy_down(d, 1)), "| 4 |\n|---|\n| 3 |\n| 4 |");
    assert_eq!(with("| 4 |\n|   |\n| <point>3 |", |d| edit::copy_down(d, 1)), "| 4 |\n|   |\n| 3 |\n| 4 |");
    set_opt("org-table-copy-increment", toml::Value::Integer(3));
    assert_eq!(with("| 1 |\n| <point>3 |", |d| edit::copy_down(d, 1)), "| 1 |\n| 3 |\n| 6 |");
    assert_eq!(with("| 1 |\n| <point>3 |", |d| edit::copy_down(d, 0)), "| 1 |\n| 3 |\n| 3 |");
    reset_opts();
    assert_eq!(with("| 1 |\n| <point>3 |", |d| edit::copy_down(d, 0)), "| 1 |\n| 3 |\n| 3 |");
}

#[test]
fn sub_total() {
    target_expect(
        "
|-------+---------+---------|
| Item  |    Item | Sub-    |
| name  |   value | total   |
|-------+---------+---------|
| a1    |     4.1 | replace |
| a2    |     8.2 | replace |
| a3    |         | replace |
|-------+---------+---------|
| b1    |    16.0 | replace |
|-------+---------+---------|
| c1    |      32 | replace |
| c2    |      64 | replace |
|-------+---------+---------|
| Total | replace | replace |
|-------+---------+---------|
",
        Some(
            "
|-------+-------+-------|
| Item  |  Item |  Sub- |
| name  | value | total |
|-------+-------+-------|
| a1    |   4.1 |       |
| a2    |   8.2 |       |
| a3    |       |  12.3 |
|-------+-------+-------|
| b1    |  16.0 |  16.0 |
|-------+-------+-------|
| c1    |    32 |       |
| c2    |    64 |  96.0 |
|-------+-------+-------|
| Total | 124.3 |       |
|-------+-------+-------|
",
        ),
        1,
        &[concat!(
            "#+TBLFM: @>$2 = vsum(@II..@>>) ::",
            "$3 = if(vlen(@0..@+I) == 1, ",
            "vsum(@-I$2..@+I$2) +.0, string(\"\")); EN f-1 :: ",
            "@>$3 = string(\"\")"
        )],
    );
}

#[test]
fn org_lookup_all() {
    let data = "
#+NAME: data
| Purchase | Product | Shop | Rating |
|----------+---------+------+--------|
| a        | p1      | s1   |      1 |
| b        | p1      | s2   |      4 |
| c        | p2      | s1   |      2 |
| d        | p3      | s2   |      8 |
";
    target_expect(
        &format!("{data}\n| Product | Rating  | Ranking |\n|---------+---------+---------|\n| p1      | replace | replace |\n| p2      | replace | replace |\n| p3      | replace | replace |\n"),
        Some(&format!("{data}\n| Product | Rating | Ranking |\n|---------+--------+---------|\n| p1      |    2.5 |       2 |\n| p2      |    2.0 |       3 |\n| p3      |    8.0 |       1 |\n")),
        2,
        &[concat!(
            "#+TBLFM: $2 = '(let ((all (org-lookup-all '$1 ",
            "'(remote(data, @I$2..@>$2)) '(remote(data, @I$4..@>$4))))) ",
            "(/ (apply '+ all) (length all) 1.0)); L :: ",
            "$3 = '(+ 1 (length (org-lookup-all $2 '(@I$2..@>$2) nil '<))); N"
        )],
    );
    target_expect(
        &format!("{data}\n| Shop | Rating  | Ranking |\n|------+---------+---------|\n| s1   | replace | replace |\n| s2   | replace | replace |\n"),
        Some(&format!("{data}\n| Shop | Rating | Ranking |\n|------+--------+---------|\n| s1   |    1.5 |       2 |\n| s2   |    6.0 |       1 |\n")),
        2,
        &[concat!(
            "#+TBLFM: $2 = '(let ((all (org-lookup-all '$1 ",
            "'(remote(data, @I$3..@>$3)) '(remote(data, @I$4..@>$4))))) ",
            "(/ (apply '+ all) (length all) 1.0)); L :: ",
            "$3 = '(+ 1 (length (org-lookup-all $2 '(@I$2..@>$2) nil '<))); N"
        )],
    );
}

fn one(s: &str) -> Refv {
    Refv::One(s.into())
}

fn many(v: &[&str]) -> Refv {
    Refv::Many(v.iter().map(|s| (*s).to_owned()).collect())
}

#[test]
fn make_reference() {
    use formula::make_reference as mr;
    // EL
    assert_eq!(mr(&one("0"), true, false, Lispp::Literal), "0");
    assert_eq!(mr(&one(""), true, false, Lispp::Literal), "");
    assert_eq!(mr(&many(&["", "1"]), true, false, Lispp::Literal), " 1");
    assert_eq!(mr(&many(&["", ""]), true, false, Lispp::Literal), " ");
    // E
    assert_eq!(mr(&one("0"), true, false, Lispp::Lisp), "\"0\"");
    assert_eq!(mr(&one(""), true, false, Lispp::Lisp), "\"\"");
    assert_eq!(mr(&many(&["", "1"]), true, false, Lispp::Lisp), "\"\" \"1\"");
    assert_eq!(mr(&one("0"), true, false, Lispp::Calc), "(0)");
    assert_eq!(mr(&one("z"), true, false, Lispp::Calc), "(z)");
    assert_eq!(mr(&one(""), true, false, Lispp::Calc), "nan");
    assert_eq!(mr(&many(&["0", "1"]), true, false, Lispp::Calc), "[0,1]");
    assert_eq!(mr(&many(&["", "1"]), true, false, Lispp::Calc), "[nan,1]");
    assert_eq!(mr(&many(&["", ""]), true, false, Lispp::Calc), "[nan,nan]");
    assert_eq!(mr(&one("nan"), true, false, Lispp::Calc), "(nan)");
    assert_eq!(mr(&one("-inf"), true, false, Lispp::Calc), "(-inf)");
    assert_eq!(mr(&many(&["uinf", "1"]), true, false, Lispp::Calc), "[uinf,1]");
    // EN
    assert_eq!(mr(&one("z"), true, true, Lispp::Lisp), "0");
    assert_eq!(mr(&one(""), true, true, Lispp::Lisp), "0");
    assert_eq!(mr(&many(&["", ""]), true, true, Lispp::Lisp), "0 0");
    assert_eq!(mr(&one("z"), true, true, Lispp::Calc), "(0)");
    assert_eq!(mr(&many(&["z", "1"]), true, true, Lispp::Calc), "[0,1]");
    assert_eq!(mr(&many(&["", ""]), true, true, Lispp::Calc), "[0,0]");
    assert_eq!(mr(&one("inf"), true, true, Lispp::Calc), "(0)");
    // L
    assert_eq!(mr(&one("z"), false, false, Lispp::Literal), "z");
    assert_eq!(mr(&many(&["", "1"]), false, false, Lispp::Literal), "1");
    assert_eq!(mr(&many(&["", ""]), false, false, Lispp::Literal), "");
    // none
    assert_eq!(mr(&one("z"), false, false, Lispp::Lisp), "\"z\"");
    assert_eq!(mr(&one(""), false, false, Lispp::Lisp), "\"\"");
    assert_eq!(mr(&many(&["", "1"]), false, false, Lispp::Lisp), "\"1\"");
    assert_eq!(mr(&many(&["", ""]), false, false, Lispp::Lisp), "");
    assert_eq!(mr(&one(""), false, false, Lispp::Calc), "(0)");
    assert_eq!(mr(&many(&["z", "1"]), false, false, Lispp::Calc), "[z,1]");
    assert_eq!(mr(&many(&["", "1"]), false, false, Lispp::Calc), "[1]");
    assert_eq!(mr(&many(&["", ""]), false, false, Lispp::Calc), "[]");
    assert_eq!(mr(&one("uinf"), false, false, Lispp::Calc), "(uinf)");
    // N
    assert_eq!(mr(&one("z"), false, true, Lispp::Lisp), "0");
    assert_eq!(mr(&many(&["z", "1"]), false, true, Lispp::Lisp), "0 1");
    assert_eq!(mr(&many(&["", "1"]), false, true, Lispp::Lisp), "1");
    assert_eq!(mr(&many(&["", ""]), false, true, Lispp::Lisp), "");
    assert_eq!(mr(&one(""), false, true, Lispp::Calc), "(0)");
    assert_eq!(mr(&many(&["", "1"]), false, true, Lispp::Calc), "[1]");
    assert_eq!(mr(&many(&["nan", "1"]), false, true, Lispp::Calc), "[0,1]");
}

#[test]
fn convert_refs() {
    assert_eq!(formula::convert_refs_to_an("@2$1"), "A2");
    assert_eq!(formula::convert_refs_to_an("@1$1 = $0"), "A1 = $0");
    assert_eq!(formula::convert_refs_to_an("$3 = remote(FOO, @@#$2)"), "C& = remote(FOO, @@#B&)");
    assert_eq!(formula::convert_refs_to_rc("A2"), "@2$1");
    assert_eq!(formula::convert_refs_to_rc("A1 = $0"), "@1$1 = $0");
    assert_eq!(formula::convert_refs_to_rc("C& = remote(FOO, @@#B&)"), "$3 = remote(FOO, @@#$2)");
    assert_eq!(formula::letters_to_number("AB"), 28);
    assert_eq!(formula::number_to_letters(28), "AB");
}

#[test]
fn remote_reference_access() {
    target_expect(
        "\n#+NAME: table\n|   | x   42 |   |\n\n| replace | replace |\n",
        Some("\n#+NAME: table\n|   | x   42 |   |\n\n| x   42 | 84 x |\n"),
        1,
        &["#+TBLFM: $1 = '(identity remote(table, @1$2)) :: $2 = 2 * remote(table, @1$2)"],
    );
}

#[test]
fn remote_reference_indirect() {
    let src = "
#+NAME: 2012
| amount |
|--------|
|      1 |
|      2 |
|--------|
|      3 |
#+TBLFM: @>$1 = vsum(@I..@II)

#+NAME: 2013
| amount |
|--------|
|      4 |
|      8 |
|--------|
|     12 |
#+TBLFM: @>$1 = vsum(@I..@II)
";
    target_expect(
        &format!("{src}\n#+NAME: summary\n|  year | amount  |\n|-------+---------|\n|  2012 | replace |\n|  2013 | replace |\n|-------+---------|\n| total | replace |\n"),
        Some(&format!("{src}\n#+NAME: summary\n|  year | amount |\n|-------+--------|\n|  2012 |      3 |\n|  2013 |     12 |\n|-------+--------|\n| total |     15 |\n")),
        1,
        &[
            "#+TBLFM: @<<$2..@>>$2 = remote($<, @>$1) :: @>$2 = vsum(@I..@II)",
            "#+TBLFM: @<<$2..@>>$2 = '(identity remote($<, @>$1)); N :: @>$2 = '(+ @I..@II); N",
        ],
    );
    target_expect(
        &format!("{src}\n#+NAME: summary\n| year   |    2012 |    2013 | total   |\n|--------+---------+---------+---------|\n| amount | replace | replace | replace |\n"),
        Some(&format!("{src}\n#+NAME: summary\n| year   | 2012 | 2013 | total |\n|--------+------+------+-------|\n| amount |    3 |   12 |    15 |\n")),
        1,
        &[
            "#+TBLFM: @2$<<..@2$>> = remote(@<, @>$1) :: @2$> = vsum($<<..$>>)",
            "#+TBLFM: @2$<<..@2$>> = '(identity remote(@<, @>$1)); N :: @2$> = '(+ $<<..$>>); N",
        ],
    );
}

#[test]
fn at_tblfm_and_tblfm_begin() {
    let d = Doc::new("\n| 1 |\n| 2 |\n#+TBLFM: $2=$1*2\n\n");
    let at = |l: usize| {
        let mut x = d.clone();
        x.goto(l, 0);
        x.at_tblfm()
    };
    assert!(!at(2));
    assert!(at(3));
    assert!(!at(4));
    let begin = |text: &str, l: usize| {
        let mut x = Doc::new(text);
        x.goto(l, 0);
        formula::tblfm_begin(&x)
    };
    let one = "\n| 1 |\n| 2 |\n#+TBLFM: $2=$1*2\n\n";
    assert_eq!(begin(one, 0), None);
    assert_eq!(begin(one, 1), None);
    assert_eq!(begin(one, 3), Some(3));
    assert_eq!(begin(one, 4), Some(3));
    let two = "\n| 1 |\n| 2 |\n#+TBLFM: $2=$1*1\n#+TBLFM: $2=$1*2\n\n";
    assert_eq!(begin(two, 3), Some(3));
    assert_eq!(begin(two, 4), Some(3));
    assert_eq!(begin(two, 5), Some(3));
    let blocks = "\n| 1 |\n| 2 |\n#+TBLFM: $2=$1*1\n#+TBLFM: $2=$1*2\n\n| 6 |\n| 7 |\n#+TBLFM: $2=$1*1\n#+TBLFM: $2=$1*2\n\n";
    assert_eq!(begin(blocks, 6), Some(3));
    assert_eq!(begin(blocks, 8), Some(9));
    assert_eq!(begin(blocks, 10), Some(9));
}

#[test]
fn calc_current_tblfm() {
    let text = "\n| 1 |   |\n| 2 |   |\n#+TBLFM: $2=$1*1\n#+TBLFM: $2=$1*2\n#+TBLFM: $2=$1*3\n";
    let mut d = Doc::new(text);
    d.goto(3, 0);
    formula::calc_current_tblfm(&mut d, Recalc::All).unwrap();
    assert_eq!(d.text(), t("\n| 1 | 1 |\n| 2 | 2 |\n#+TBLFM: $2=$1*1\n#+TBLFM: $2=$1*2\n#+TBLFM: $2=$1*3\n"));
    d.goto(4, 0);
    formula::calc_current_tblfm(&mut d, Recalc::All).unwrap();
    assert_eq!(d.text(), t("\n| 1 | 2 |\n| 2 | 4 |\n#+TBLFM: $2=$1*1\n#+TBLFM: $2=$1*2\n#+TBLFM: $2=$1*3\n"));
    // An error leaves the buffer as it was.
    let expect = "\n| 1 | 1 |\n| 2 | 2 |\n#+TBLFM: $2=$1*1\n#+TBLFM: $2=$1*2::$2=$1*2\n#+TBLFM: $2=$1*3\n";
    let mut d = Doc::new(expect);
    d.goto(4, 0);
    assert!(formula::calc_current_tblfm(&mut d, Recalc::All).is_err());
    assert_eq!(d.text(), t(expect));
}

// ---- tables as Lisp ----

fn cells(v: &[&str]) -> Row {
    Row::Cells(v.iter().map(|s| (*s).to_owned()).collect())
}

#[test]
fn to_lisp() {
    assert_eq!(to_lisp_text("|a|b|\n|c|d|"), vec![cells(&["a", "b"]), cells(&["c", "d"])]);
    assert_eq!(to_lisp_text("|a|b|\n|-\n|c|d|"), vec![cells(&["a", "b"]), Row::Hline, cells(&["c", "d"])]);
    assert_eq!(to_lisp_text("|-\n|a|b|\n|-\n|c|d|"), vec![Row::Hline, cells(&["a", "b"]), Row::Hline, cells(&["c", "d"])]);
    assert_eq!(to_lisp_text("|-\n|-\n|-\n|-\n|a|b|\n|c|d|").len(), 6);
}

#[test]
fn collapse_header_rows() {
    let c = |s: &str, sep: Option<&str>, max: Option<usize>| collapse_header(&to_lisp_text(s), sep, max);
    assert_eq!(c("|a|b|\n|c|d|", None, None), to_lisp_text("|a|b|\n|c|d|"));
    assert_eq!(c("|a|b|\n|-\n|c|d|", None, None), to_lisp_text("|a|b|\n|-\n|c|d|"));
    assert_eq!(c("|a|b|\n|A|B|\n|-\n|c|d|\n|aa|bb|", None, None), vec![cells(&["a A", "b B"]), Row::Hline, cells(&["c", "d"]), cells(&["aa", "bb"])]);
    assert_eq!(c("|a|b|\n|A|B|\n|-\n|c|d|\n|aa|bb|", Some("."), None)[0], cells(&["a.A", "b.B"]));
    assert_eq!(c("|a|b|\n|A|B|\n|-\n|c|d|\n|aa|bb|", None, Some(1)).len(), 5);
    assert_eq!(c("|a|b|\n|A|B|\n|-\n|c|d|\n|aa|bb|", None, Some(2))[0], cells(&["a A", "b B"]));
    assert_eq!(c("|a|b|\n|A|B|\n|a|b|\n|A|B|\n|a|b|\n|A|B|\n|-\n|c|d|\n|aa|bb|", None, None).len(), 9);
}

// ---- radio tables and translators ----

fn generic(table: &str, params: &str) -> String {
    translate("orgtbl-to-generic", &to_lisp_text(table), &Params::read(params).unwrap()).unwrap()
}

#[test]
fn to_generic() {
    assert_eq!(generic("| a |\n|---|\n| b |", ":hline nil"), "a\nb");
    assert_eq!(generic("| a |\n|---|\n| b |", ":hline \"~\""), "a\n~\nb");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":sep \"!\""), "a!b\nc!d");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":sep \"?\" :hsep \"!\""), "a!b\nc?d");
    assert_eq!(generic("| a |", ":tstart \"<begin>\""), "<begin>\na");
    assert_eq!(generic("| a |", ":tstart (lambda () \"<begin>\")"), "<begin>\na");
    assert_eq!(generic("| a |", ":tstart \"<begin>\" :splice t"), "a");
    assert_eq!(generic("| a |", ":tend \"<end>\""), "a\n<end>");
    assert_eq!(generic("| a |", ":tend (lambda () \"<end>\")"), "a\n<end>");
    assert_eq!(generic("| a |", ":tend \"<end>\" :splice t"), "a");
    assert_eq!(generic("| a |", ":lstart \"> \""), "> a");
    assert_eq!(generic("| a |", ":lstart (lambda () \"> \")"), "> a");
    assert_eq!(generic("| a |\n|---|\n| b |", ":lstart \"> \" :llstart \">> \""), "> a\n>> b");
    assert_eq!(generic("| a |\n|---|\n| b |", ":lstart \"> \" :hlstart \"!> \""), "!> a\n> b");
    assert_eq!(generic("| a |\n| b |\n|---|\n| c |", ":lstart \"> \" :hlstart \"!> \" :hllstart \"!!> \""), "!> a\n!!> b\n> c");
    assert_eq!(generic("| a |", ":lend \" <\""), "a <");
    assert_eq!(generic("| a |\n|---|\n| b |", ":lend \" <\" :llend \" <<\""), "a <\nb <<");
    assert_eq!(generic("| a |\n|---|\n| b |", ":lend \" <\" :hlend \" <!\""), "a <!\nb <");
    assert_eq!(generic("| a |\n| b |\n|---|\n| c |", ":lend \" <\" :hlend \" <!\" :hllend \" <!!\""), "a <!\nb <!!\nc <");
    assert_eq!(generic("| a | b |", ":lfmt \"%s!%s\""), "a!b");
    assert_eq!(generic("| a | b |", ":lfmt (lambda (c) (concat (car c) \"+\" (cadr c)))"), "a+b");
    assert_eq!(generic("| a | b |", ":lfmt \"%s!%s\" :lstart \">\" :lend \"<\" :sep \" \""), "a!b");
    assert_eq!(generic("| a | b |", ":llfmt \"%s!%s\""), "a!b");
    assert_eq!(generic("| a | b |\n| c | d |", ":lfmt \"%s!%s\" :llfmt (lambda (c) (concat (car c) \"+\" (cadr c)))"), "a!b\nc+d");
    assert_eq!(generic("| a | b |", ":llfmt \"%s!%s\" :lstart \">\" :lend \"<\" :sep \" \""), "a!b");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hlfmt \"%s!%s\""), "a!b\ncd");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hlfmt (lambda (c) (concat (car c) \"+\" (cadr c)))"), "a+b\ncd");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hlfmt \"%s!%s\" :lstart \">\" :lend \"<\" :sep \" \""), "a!b\n>c d<");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hllfmt \"%s!%s\""), "a!b\ncd");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hllfmt (lambda (c) (concat (car c) \"+\" (cadr c)))"), "a+b\ncd");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hllfmt \"%s!%s\" :lstart \">\" :lend \"<\" :sep \" \""), "a!b\n>c d<");
    assert_eq!(generic("| a |\n|---|\n| b |", ":fmt \">%s<\""), ">a<\n>b<");
    assert_eq!(generic("| a | b |", ":fmt (1 \">%s<\" 2 (lambda (c) c))"), ">a<b");
    assert_eq!(generic("| a | b |", ":fmt (2 \" %s\")"), "a b");
    assert_eq!(generic("| a |", ":fmt (lambda (c) (format \">%s<\" c))"), ">a<");
    assert_eq!(generic("| a |\n|---|\n| b |", ":hfmt \">%s<\""), ">a<\nb");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hfmt (1 \">%s<\" 2 identity)"), ">a<b\ncd");
    assert_eq!(generic("| a | b |\n|---+---|\n| c | d |", ":hfmt (2 \" %s\")"), "a b\ncd");
    assert_eq!(generic("| a |\n|---|\n| b |", ":hfmt (lambda (c) (format \">%s<\" c))"), ">a<\nb");
    assert_eq!(generic("| 2e3 |", ":efmt \"%sx10^%s\""), "2x10^3");
    assert_eq!(generic("| 2e3 |", ":efmt (lambda (m e) (concat m \"x10^\" e))"), "2x10^3");
    assert_eq!(generic("| 2e3 |", ":efmt (1 \"%sx10^%s\")"), "2x10^3");
    assert_eq!(generic("| 2e3 |", ":efmt (1 (lambda (m e) (format \"%sx10^%s\" m e)))"), "2x10^3");
    assert_eq!(generic("| 2e3 |", ":efmt nil"), "2e3");
    assert_eq!(generic("|  | <c> |\n| a | b |\n|---+---|\n| c | d |", ":skip 2"), "cd");
    assert_eq!(generic("| a | b |\n| c | d |", ":skipcols (2)"), "a\nc");
    assert_eq!(generic("| / | <c> | <c> |\n| # | a | b |\n|---+---+---|\n|   | c | d |", ":skipcols (2)"), "<c>\na\nc");
    assert_eq!(generic("| a |\n|---|\n| b |", ":hline nil"), "a\nb");
    assert_eq!(generic("| {{{macro}}} |", ""), "{{{macro}}}");
}

fn tr(name: &str, table: &str, params: &str) -> String {
    translate(name, &to_lisp_text(table), &Params::read(params).unwrap()).unwrap()
}

#[test]
fn to_latex() {
    assert_eq!(tr("orgtbl-to-latex", "| a |", ""), "\\begin{tabular}{l}\na\\\\\n\\end{tabular}");
    assert_eq!(tr("orgtbl-to-latex", "| a |", ":environment \"tabularx\""), "\\begin{tabularx}{l}\na\\\\\n\\end{tabularx}");
    assert!(tr("orgtbl-to-latex", "| a |", ":booktabs t").contains("\\toprule"));
    assert_eq!(tr("orgtbl-to-latex", "| $x$ |", ""), "\\begin{tabular}{l}\n\\(x\\)\\\\\n\\end{tabular}");
    assert!(tr("orgtbl-to-latex", "| $x$ |", ":raw t").contains("$x$"));
    // Recorded from Emacs.
    assert_eq!(tr("orgtbl-to-latex", "| x | 1 |\n| y | 22 |\n|---+---|\n| z | 3.5 |", ""), "\\begin{tabular}{lr}\nx & 1\\\\\ny & 22\\\\\n\\hline\nz & 3.5\\\\\n\\end{tabular}");
    assert_eq!(
        tr("orgtbl-to-latex", "| a | b |\n|---+---|\n| c | d |", ":booktabs t"),
        "\\begin{tabular}{ll}\n\\toprule\na & b\\\\\n\\midrule\nc & d\\\\\n\\bottomrule\n\\end{tabular}"
    );
    assert_eq!(tr("orgtbl-to-latex", "| a | b |\n|---+---|\n| c | d |", ":splice t"), "a & b\\\\\n\\hline\nc & d\\\\");
    assert_eq!(tr("orgtbl-to-latex", "| / | <c> | <c> |\n| # | a | b |\n|---+---+---|\n|   | c | d |", ""), "\\begin{tabular}{cc}\na & b\\\\\n\\hline\nc & d\\\\\n\\end{tabular}");
    assert_eq!(
        tr("orgtbl-to-latex", "| h1 | h2 |\n| h3 | h4 |\n|----+----|\n| 1 | |\n| /a/ | $x$ & _y_ |", ""),
        "\\begin{tabular}{ll}\nh1 & h2\\\\\nh3 & h4\\\\\n\\hline\n1 & \\\\\n\\emph{a} & \\(x\\) \\& \\uline{y}\\\\\n\\end{tabular}"
    );
}

#[test]
fn to_html() {
    assert_eq!(
        tr("orgtbl-to-html", "| a |", ""),
        "<table border=\"2\" cellspacing=\"0\" cellpadding=\"6\" rules=\"groups\" frame=\"hsides\">


<colgroup>
<col  class=\"org-left\" />
</colgroup>
<tbody>
<tr>
<td class=\"org-left\">a</td>
</tr>
</tbody>
</table>"
    );
    assert!(tr("orgtbl-to-html", "| a |", ":attributes nil").contains("<table>"));
    assert!(tr("orgtbl-to-html", "| a |", ":attributes (:border \"2\")").contains("<table border=\"2\">"));
    assert_eq!(
        tr("orgtbl-to-html", "| x | 1 |\n| y | 22 |\n|---+---|\n| z | 3.5 |", ":splice t"),
        "<thead>
<tr>
<th scope=\"col\" class=\"org-left\">x</th>
<th scope=\"col\" class=\"org-right\">1</th>
</tr>

<tr>
<th scope=\"col\" class=\"org-left\">y</th>
<th scope=\"col\" class=\"org-right\">22</th>
</tr>
</thead>
<tbody>
<tr>
<td class=\"org-left\">z</td>
<td class=\"org-right\">3.5</td>
</tr>
</tbody>"
    );
}

#[test]
fn to_texinfo() {
    assert_eq!(tr("orgtbl-to-texinfo", "| a |", ""), "@multitable {a}\n@item a\n@end multitable");
    assert_eq!(tr("orgtbl-to-texinfo", "| a | b |", ":columns \".4 .6\""), "@multitable @columnfractions .4 .6\n@item a\n@tab b\n@end multitable");
    assert_eq!(tr("orgtbl-to-texinfo", "| a | b |", ":columns \"@columnfractions .4 .6\""), "@multitable @columnfractions .4 .6\n@item a\n@tab b\n@end multitable");
    assert_eq!(tr("orgtbl-to-texinfo", "| a | b |", ":columns \"{xxx} {xx}\""), "@multitable {xxx} {xx}\n@item a\n@tab b\n@end multitable");
    assert_eq!(
        tr("orgtbl-to-texinfo", "| x | 1 |\n| y | 22 |\n|---+---|\n| z | 3.5 |", ""),
        "@multitable {a} {aaa}\n@headitem x\n@tab 1\n@headitem y\n@tab 22\n@item z\n@tab 3.5\n@end multitable"
    );
}

#[test]
fn to_orgtbl_and_table_el() {
    assert_eq!(tr("orgtbl-to-orgtbl", "| a | b |\n|---+---|\n| c | d |", ""), "| a | b |\n|---+---|\n| c | d |");
    assert_eq!(tr("orgtbl-to-orgtbl", "| x | 1 |\n| y | 22 |\n|---+---|\n| z | 3.5 |", ""), "| x |   1 |\n| y |  22 |\n|---+-----|\n| z | 3.5 |");
    assert_eq!(tr("orgtbl-to-orgtbl", "| / | <c> | <c> |\n| # | a | b |\n|---+---+---|\n|   | c | d |", ""), "| <c> | <c> |\n|  a  |  b  |\n|-----+-----|\n|  c  |  d  |");
    assert_eq!(tr("orgtbl-to-table.el", "| a | b |\n|---+---|\n| c | d |", ""), "| a | b |\n+---+---+\n| c | d |");
    assert_eq!(tr("orgtbl-to-tsv", "| h1 | h2 |\n| 1 | |", ""), "h1\th2\n1\t");
    assert_eq!(tr("orgtbl-to-csv", "| a,b | c\"d |\n| 1 | 2 |", ""), "\"a,b\",\"c\"\"d\"\n1,2");
}

#[test]
fn to_unicode() {
    assert_eq!(tr("orgtbl-to-unicode", "| a |", ""), "━━━\n a \n━━━");
    assert_eq!(tr("orgtbl-to-unicode", "| <2> |\n| xxx |", ":narrow t"), "━━━━\n => \n━━━━");
    assert_eq!(tr("orgtbl-to-unicode", "| x | 1 |\n| y | 22 |\n|---+---|\n| z | 3.5 |", ""), "━━━━━━━━\n x    1 \n y   22 \n────────\n z  3.5 \n━━━━━━━━");
    assert_eq!(tr("orgtbl-to-unicode", "|---+---|\n| a | b |\n|---+---|\n| 1 | 2 |\n| 3 | 4 |\n|---+---|", ""), "━━━━━━\n a  b \n──────\n 1  2 \n 3  4 \n━━━━━━");
}

#[test]
fn send_region() {
    assert!(fails("Paragraph", |d| orgtbl::send_table(d, false).map(|_| ())));
    assert!(fails("#+ORGTBL: SEND\n<point>| a |", |d| orgtbl::send_table(d, false).map(|_| ())));
    assert!(fails("\n# BEGIN RECEIVE ORGTBL table\n# END RECEIVE ORGTBL table\n#+ORGTBL: SEND table\n<point>| a |", |d| orgtbl::send_table(d, false).map(|_| ())));
    let out = with("\n# BEGIN RECEIVE ORGTBL table\n# END RECEIVE ORGTBL table\n#+ORGTBL: SEND table orgtbl-to-orgtbl :hlines nil\n<point>| a |\n|---|\n| b |", |d| orgtbl::send_table(d, false).map(|_| ()));
    let s = out.find("# BEGIN RECEIVE ORGTBL table\n").unwrap() + "# BEGIN RECEIVE ORGTBL table\n".len();
    let e = out.find("# END RECEIVE ORGTBL table").unwrap();
    assert_eq!(&out[s..e], "| a |\n|---|\n| b |\n");
    let out = with(
        "\n# BEGIN RECEIVE ORGTBL table\n# END RECEIVE ORGTBL table\n\n#+ORGTBL: SEND table orgtbl-to-orgtbl :hlines nil\n<point>| a |\n\n# BEGIN RECEIVE ORGTBL table\n# END RECEIVE ORGTBL table",
        |d| orgtbl::send_table(d, false).map(|_| ()),
    );
    assert_eq!(out.matches("| a |").count(), 3);
}

// ---- align ----

#[test]
fn align() {
    let al = |s: &str| with(s, |d| {
        d.align();
        Ok(())
    });
    assert_eq!(al("|   a |"), "| a |");
    assert_eq!(al("  |   a |"), "  | a |");
    assert_eq!(al("| 123 |\n|-|"), "| 123 |\n|-----|");
    assert_eq!(al("| a | b |\n|-+-|"), "| a | b |\n|---+---|");
    assert_eq!(al("| a | bc |\n| bcd |  |"), "| a   | bc |\n| bcd |    |");
    assert_eq!(al("| abc | bc |\n| | bcd |"), "| abc | bc  |\n|     | bcd |");
    assert_eq!(al("| a | b |\n| c |"), "| a | b |\n| c |   |");
    assert_eq!(al("| a | b |\n|---|"), "| a | b |\n|---+---|");
    assert_eq!(al("| 1 |\n| 12 |\n| abc |"), "|   1 |\n|  12 |\n| abc |");
    assert_eq!(al("| 1 |\n| ab |\n| abc |"), "| 1   |\n| ab  |\n| abc |");
    assert_eq!(al("| <r> |\n| ab |\n| abc |"), "| <r> |\n|  ab |\n| abc |");
    assert_eq!(al("| <l> |\n| 12 |\n| 123 |"), "| <l> |\n| 12  |\n| 123 |");
    assert_eq!(al("| <c> |\n| 1 |\n| 123 |"), "| <c> |\n|  1  |\n| 123 |");
    al("|-<point>--|");
    al("|-<point>--|---------|\n|---|---|-----|");
    // Fred shows link markup: links count with their full width.
    assert_eq!(al("\n| a        | b |\n|----------+---|\n| [[c][c]] | d |<point>"), "\n| a        | b |\n|----------+---|\n| [[c][c]] | d |");
}

#[test]
fn align_buffer_tables() {
    let before = "\n|  a  b  |\n\n|  c  d  |\n";
    let after = t("\n| a  b |\n\n| c  d |\n");
    assert_eq!(with(before, formula::recalculate_buffer_tables), after);
    assert_eq!(with(before, formula::iterate_buffer_tables), after);
}

// ---- sorting ----

fn sort(text: &str, case: bool, kind: char) -> String {
    with(text, |d| edit::sort_lines(d, case, kind, None, None))
}

#[test]
fn sort_lines() {
    assert_eq!(sort("| <point>1 | 2 |\n| 5 | 3 |\n| 2 | 4 |\n", false, 'n'), t("| 1 | 2 |\n| 2 | 4 |\n| 5 | 3 |\n"));
    assert_eq!(sort("| <point>1 | 2 |\n| 5 | 3 |\n| 2 | 4 |\n", false, 'N'), t("| 5 | 3 |\n| 2 | 4 |\n| 1 | 2 |\n"));
    assert_eq!(sort("| <point>a | x |\n| c | 3 |\n| B | 4 |\n", false, 'a'), t("| a | x |\n| B | 4 |\n| c | 3 |\n"));
    assert_eq!(sort("| <point>a | x |\n| c | 3 |\n| B | 4 |\n", false, 'A'), t("| c | 3 |\n| B | 4 |\n| a | x |\n"));
    assert_eq!(sort("| <point>a |\n| C |\n| b |\n", true, 'a'), t("| C |\n| a |\n| b |\n"));
    assert_eq!(sort("| <point>a |\n| C |\n| b |\n", true, 'A'), t("| b |\n| a |\n| C |\n"));
    assert_eq!(
        sort("| <2014-03-04 tue.> |\n| <2008-08-08 sat.> |\n| <2012-03-29 thu.> |\n", false, 't'),
        t("| <2008-08-08 sat.> |\n| <2012-03-29 thu.> |\n| <2014-03-04 tue.> |\n")
    );
    assert_eq!(
        sort("| <2014-03-04 tue.> |\n| <2008-08-08 sat.> |\n| <2012-03-29 thu.> |\n", false, 'T'),
        t("| <2014-03-04 tue.> |\n| <2012-03-29 thu.> |\n| <2008-08-08 sat.> |\n")
    );
    assert_eq!(sort("| 114:00 |\n| 17:00 |\n| 1:00 |\n", false, 't'), t("| 1:00 |\n| 17:00 |\n| 114:00 |\n"));
    assert_eq!(sort("| 114:00 |\n| 17:00 |\n| 1:00 |\n", false, 'T'), t("| 114:00 |\n| 17:00 |\n| 1:00 |\n"));
    assert_eq!(sort("| 28:00 |\n| 1d 3:00 |\n", false, 't'), t("| 1d 3:00 |\n| 28:00 |\n"));
    let key = |s: &str| edit::SortKey::Num((lisp::string_to_number(s).f() as i64 % 10) as f64);
    assert_eq!(with("| 15 |\n| 22 |\n| 18 |\n", |d| edit::sort_lines(d, false, 'f', None, Some(&key))), t("| 22 |\n| 15 |\n| 18 |\n"));
    assert_eq!(with("| 15 |\n| 22 |\n| 18 |\n", |d| edit::sort_lines(d, false, 'F', None, Some(&key))), t("| 18 |\n| 15 |\n| 22 |\n"));
    assert_eq!(sort("| 1 | <point>2 |\n| 5 | 4 |\n| 7 | 3 |\n", false, 'n'), t("| 1 | 2 |\n| 7 | 3 |\n| 5 | 4 |\n"));
    assert_eq!(
        sort("| 9 | 8 |\n|---+---|\n| <point>7 | 4 |\n| 5 | 3 |\n|---+---|\n| 1 | 2 |\n", false, 'n'),
        t("| 9 | 8 |\n|---+---|\n| 5 | 3 |\n| 7 | 4 |\n|---+---|\n| 1 | 2 |\n")
    );
}

// ---- formulas ----

fn eval(text: &str, eq: &str) -> String {
    with(text, |d| formula::eval_formula(d, formula::EvalArg::None, Some(eq), false, false, true, false))
}

#[test]
fn eval_formula() {
    assert!(fails("Text", |d| formula::eval_formula(d, formula::EvalArg::None, Some("1"), false, false, false, false)));
    assert!(fails("| a |\n|---|<point>", |d| formula::eval_formula(d, formula::EvalArg::None, Some("1"), false, false, false, false)));
    assert!(fails("| a |\n#+TBLFM:<point>", |d| formula::eval_formula(d, formula::EvalArg::None, Some("1"), false, false, false, false)));
    assert_eq!(eval("| <point>  |\n| 1 |", "@>"), "| 1 |\n| 1 |");
    assert_eq!(eval("| 1 |\n| <point>  |", "@<"), "| 1 |\n| 1 |");
    assert_eq!(eval("| <point>  | 1 |", "$>"), "| 1 | 1 |");
    assert_eq!(eval("| 1 | <point>  |", "$<"), "| 1 | 1 |");
}

#[test]
fn field_formula_outside_table() {
    reset_opts();
    let text = "\n| 2 |\n| 4 |\n| 8 |\n<point>#+TBLFM: @1$2=5";
    assert!(fails(text, |d| formula::calc_current_tblfm(d, Recalc::All)));
    set_opt("org-table-formula-create-columns", toml::Value::Boolean(true));
    assert_eq!(with(text, |d| formula::calc_current_tblfm(d, Recalc::All)), "\n| 2 | 5 |\n| 4 |   |\n| 8 |   |\n#+TBLFM: @1$2=5");
    assert_eq!(
        with("\n| 2 |\n| 4 |\n| 8 |\n<point>#+TBLFM: $3=15", |d| formula::calc_current_tblfm(d, Recalc::All)),
        "\n| 2 |   | 15 |\n| 4 |   | 15 |\n| 8 |   | 15 |\n#+TBLFM: $3=15"
    );
    reset_opts();
}

#[test]
fn duration() {
    let calc = |s: &str| with(s, |d| formula::calc_current_tblfm(d, Recalc::All));
    assert!(calc("\n       | 2:12 | 1:47 | |\n       <point>#+TBLFM: @1$3=$1+$2;T").contains("| 2:12 | 1:47 | 03:59:00 |"));
    assert!(calc("\n       | 2:12 | 1:47 | |\n       <point>#+TBLFM: @1$3=$1+$2;U").contains("| 2:12 | 1:47 | 03:59 |"));
    assert!(calc("\n       | 3:02:20 | -2:07:00 | |\n       <point>#+TBLFM: @1$3=$1+$2;t").contains("| 3:02:20 | -2:07:00 | 0.92 |"));
    assert!(calc("* H\n  :PROPERTIES:\n  :time_constant: 08:00:00\n  :END:\n\n  |  |\n  <point>#+TBLFM: $1=2*$PROP_time_constant;T").contains("| 16:00:00 |"));
    assert!(calc("* H\n  :PROPERTIES:\n  :time_constant: 08:00:00\n  :END:\n\n  |  |\n  <point>#+TBLFM: $1=2*$PROP_time_constant;t").contains("| 16.00 |"));
}

#[test]
fn end_on_hline_and_names() {
    let calc = |s: &str| with(s, |d| formula::calc_current_tblfm(d, Recalc::All));
    assert_eq!(
        calc("\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n|   |   |   |\n|---+---+---|\n<point>#+TBLFM: @3$2..@3$>=vsum(@1..@2)"),
        "\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n|   | 7 | 9 |\n|---+---+---|\n#+TBLFM: @3$2..@3$>=vsum(@1..@2)"
    );
    let re = |p: &str, s: &str| regex::Regex::new(p).unwrap().is_match(s);
    assert!(re(r"\| +\| +1 +\|", &calc("\n|   |      |\n| ^ | name |\n<point>#+TBLFM: $name=1")));
    assert!(re(r"\| +\| +1 +\|", &calc("\n| _ | name |\n|   |      |\n<point>#+TBLFM: $name=1")));
    assert!(re(r"\| +\| +1 +\| +1 +\|", &calc("\n| ! | name |   |\n|   |    1 |   |\n<point>#+TBLFM: @2$3=$name")));
}

#[test]
fn formula_priority() {
    let out = with("| 1 |   |\n| 2 |   |\n<point>#+tblfm: $2=3*$1::@2$2=99", |d| formula::calc_current_tblfm(d, Recalc::All));
    assert!(out.starts_with("| 1 |  3 |\n| 2 | 99 |\n"));
    let mut d = Doc::new("| 1 |   |\n| 2 |   |\n#+tblfm: $2=3*$1<point>::@2$2=99");
    formula::calc_current_tblfm(&mut d, Recalc::All).unwrap();
    let l = d.l;
    let c = d.c;
    d.lines[l].truncate(c);
    formula::calc_current_tblfm(&mut d, Recalc::All).unwrap();
    assert!(d.text().starts_with("| 1 | 3 |\n| 2 | 6 |\n"));
}

#[test]
fn tab_indent_first_last_rc() {
    let calc = |s: &str| with(s, |d| formula::calc_current_tblfm(d, Recalc::All));
    assert!(calc("\n\t| ! |  sum |      | a |  b |   c |\n\t|---+------+------+---+----+-----|\n\t| # | 1011 | 1000 | 1 | 10 | 100 |\n\t<point>#+TBLFM: $2=$a+$b+$c\n").contains("| # | 111 |"));
    assert!(calc("|   | 2 |\n<point>#+TBLFM: $<=1").contains("| 1 | 2 |"));
    assert!(calc("| 2 |\n|   |\n<point>#+TBLFM: @2$1=@<").contains("| 2 |\n| 2 |"));
    assert!(calc("| 2 |   |\n<point>#+TBLFM: $>=1").contains("| 2 | 1 |"));
    assert!(calc("| 2 |\n|   |\n<point>#+TBLFM: @>$1=@<").contains("| 2 |\n| 2 |"));
}

#[test]
fn timestamps() {
    let calc = |s: &str| with(s, |d| formula::calc_current_tblfm(d, Recalc::All));
    assert!(calc("| <2016-07-07 Sun> | <2016-07-08 Fri> |   |\n<point>#+TBLFM: $3=$2-$1").contains("| 1 |"));
    assert!(calc("| <2016-07-07 Do> | <2016-07-08 Fr> |   |\n<point>#+TBLFM: $3=$2-$1").contains("| 1 |"));
}

#[test]
fn orgtbl_ascii_draw() {
    let calc = |s: &str| with(s, |d| formula::calc_current_tblfm(d, Recalc::All));
    assert_eq!(
        calc(
            "
|    Value | <l>     |
|----------+---------|
|       19 | replace |
|----------+---------|
| -0.50001 | replace |
| -0.49999 | replace |
|  0.49999 | replace |
|  0.50001 | replace |
|  1.49999 | replace |
| 22.50001 | replace |
| 23.49999 | replace |
| 23.50001 | replace |
| 24.49999 | replace |
| 24.50001 | replace |
<point>#+TBLFM: $2 = '(orgtbl-ascii-draw $1 0 24 3 \" 12345678\")"
        ),
        "
|    Value | <l>       |
|----------+-----------|
|       19 | 883       |
|----------+-----------|
| -0.50001 | too small |
| -0.49999 |           |
|  0.49999 |           |
|  0.50001 | 1         |
|  1.49999 | 1         |
| 22.50001 | 887       |
| 23.49999 | 887       |
| 23.50001 | 888       |
| 24.49999 | 888       |
| 24.50001 | too large |
#+TBLFM: $2 = '(orgtbl-ascii-draw $1 0 24 3 \" 12345678\")"
    );
    assert_eq!(
        calc("\n| -1 | replace |\n|  0 | replace |\n|  1 | replace |\n|  2 | replace |\n|  3 | replace |\n|  4 | replace |\n<point>#+TBLFM: $2 = '(orgtbl-ascii-draw $1 0 3 3 \"$-\")"),
        "\n| -1 | too small |\n|  0 | $         |\n|  1 | -$        |\n|  2 | --$       |\n|  3 | ---$      |\n|  4 | too large |\n#+TBLFM: $2 = '(orgtbl-ascii-draw $1 0 3 3 \"$-\")"
    );
}

#[test]
fn single_rowgroup() {
    let calc = |s: &str| with(s, |d| formula::calc_current_tblfm(d, Recalc::All));
    assert_eq!(calc("\n|---+---|\n| 1 |   |\n|---+---|\n<point>#+TBLFM: $2=$1-1"), "\n|---+---|\n| 1 | 0 |\n|---+---|\n#+TBLFM: $2=$1-1");
    assert_eq!(calc("\n| 1 |   |\n<point>#+TBLFM: $2=$1-1"), "\n| 1 | 0 |\n#+TBLFM: $2=$1-1");
}

// ---- navigation ----

#[test]
fn next_field() {
    reset_opts();
    let mut d = Doc::new("| a<point> | b |");
    edit::next_field(&mut d).unwrap();
    assert_eq!(super::doc::trim(&d.get_field(None, None)), "b");
    assert_eq!(with("| a<point> |", edit::next_field), "| a |\n|   |");
    let mut d = Doc::new("| a<point> |\n|---|\n| b |");
    edit::next_field(&mut d).unwrap();
    assert_eq!(super::doc::trim(&d.get_field(None, None)), "b");
    set_opt("org-table-tab-jumps-over-hlines", toml::Value::Boolean(false));
    assert_eq!(with("| a<point> |\n|---|\n| b |", edit::next_field), "| a |\n|   |\n|---|\n| b |");
    reset_opts();
}

#[test]
fn previous_field() {
    let after = |s: &str| {
        let mut d = Doc::new(s);
        let r = edit::previous_field(&mut d);
        (r.is_ok(), d.char_after())
    };
    assert_eq!(after("| a | <point>b |"), (true, Some('a')));
    assert_eq!(after("| a |\n| <point>b |"), (true, Some('a')));
    assert_eq!(after("| a |\n|---|\n| <point>b |"), (true, Some('a')));
    assert_eq!(after("| a | b |\n|---+-<point>--|"), (true, Some('b')));
    assert!(!after("| <point> a|").0);
    assert_eq!(after("|---|\n| <point>a |"), (false, Some('a')));
}

#[test]
fn delete_column() {
    assert!(fails("Paragraph", edit::delete_column));
    assert_eq!(with("| <point>  | a |\n", edit::delete_column), "| a |");
    let mut d = Doc::new("| a | <point>b  | c |");
    edit::delete_column(&mut d).unwrap();
    assert_eq!(d.current_column(), 2);
    assert_eq!(with("| a | b |<point>\n", edit::delete_column), "| a |");
    assert_eq!(with("| a | b |   <point>\n", edit::delete_column), "| a |");
    assert_eq!(
        with("| a | b  | c<point> |", |d| {
            edit::delete_column(d)?;
            edit::delete_column(d)
        }),
        "| a |"
    );
}

#[test]
fn insert_column() {
    assert!(fails("Paragraph", edit::insert_column));
    assert_eq!(with("| a |", edit::insert_column), "|   | a |");
    assert_eq!(with("| <point>a | b |", edit::insert_column), "|   | a | b |");
    let rest = |s: &str| {
        let mut d = Doc::new(s);
        edit::insert_column(&mut d).unwrap();
        d.rest().to_owned()
    };
    assert_eq!(rest("| <point>a |"), "  | a |");
    assert_eq!(rest("| <point>a | b |"), "  | a | b |");
    assert_eq!(with("| a", edit::insert_column), "|   | a |");
    assert_eq!(rest("| <point>a"), "  | a |");
    assert_eq!(with(" | a |", edit::insert_column), " |   | a |");
    assert_eq!(with(" | a | b |", edit::insert_column), " |   | a | b |");
    assert_eq!(with("| 1<point> | 1 | 2 |\n#+TBLFM: $3=$1+$2", edit::insert_column), "|   | 1 | 1 | 2 |\n#+TBLFM: $4=$2+$3");
}

#[test]
fn move_cells() {
    for s in ["not in\na table\n", "| a |", "| a | <point>b |\n", "| a | b |\n| <point>c | d |\n", "| <point>a |\n|---|\n", "|<point>---|\n| a |\n"] {
        assert!(fails(s, edit::move_cell_down), "{s}");
    }
    assert_eq!(with("| <point>a | b |\n| c | d |\n| e | f |\n", edit::move_cell_down), t("| c | b |\n| a | d |\n| e | f |\n"));
    assert_eq!(with("| a | <point>b |\n| c | d |\n| e | f |\n", edit::move_cell_down), t("| a | d |\n| c | b |\n| e | f |\n"));
    assert_eq!(
        with("| a |<point> b |\n| c | d |\n| e | f |\n", |d| {
            edit::move_cell_down(d)?;
            edit::move_cell_down(d)
        }),
        t("| a | d |\n| c | f |\n| e | b |\n")
    );
    assert_eq!(with("| <point>a | b |\n|---+---|\n| c | d |\n| e | f |\n", edit::move_cell_down), t("| c | b |\n|---+---|\n| a | d |\n| e | f |\n"));
    assert_eq!(
        with("| a | <point>b |\n|---+---|\n| c | d |\n| e | f |", |d| {
            edit::move_cell_down(d)?;
            edit::move_cell_down(d)
        }),
        t("| a | d |\n|---+---|\n| c | f |\n| e | b |\n")
    );
    for s in ["| a |", "| <point>a | b |\n", "| a | <point>b |\n| c | d |\n"] {
        assert!(fails(s, edit::move_cell_up), "{s}");
    }
    assert_eq!(with("| a | b |\n| <point>c | d |\n| e | f |\n", edit::move_cell_up), t("| c | b |\n| a | d |\n| e | f |\n"));
    assert_eq!(
        with("| a | b |\n|---+---|\n| c | d |\n| e | <point>f |", |d| {
            edit::move_cell_up(d)?;
            edit::move_cell_up(d)
        }),
        t("| a | f |\n|---+---|\n| c | b |\n| e | d |\n")
    );
    for s in ["| a |", "| <point>a |\n| b |\n", "| a | <point>b |\n| c | d |\n"] {
        assert!(fails(s, edit::move_cell_right), "{s}");
    }
    assert_eq!(
        with("| <point>a | b | c |\n| d | e | f |\n", |d| {
            edit::move_cell_right(d)?;
            edit::move_cell_right(d)
        }),
        t("| b | c | a |\n| d | e | f |\n")
    );
    assert_eq!(with("| a | b | c |\n|---+---+---|\n| <point>d | e | f |", edit::move_cell_right), t("| a | b | c |\n|---+---+---|\n| e | d | f |\n"));
    for s in ["| a |", "| <point>a |\n| b |\n", "| <point>a | b |\n| c | d |\n"] {
        assert!(fails(s, edit::move_cell_left), "{s}");
    }
    assert_eq!(
        with("| a | b | <point>c |\n| d | e | f |\n", |d| {
            edit::move_cell_left(d)?;
            edit::move_cell_left(d)
        }),
        t("| c | a | b |\n| d | e | f |\n")
    );
    assert_eq!(with("| a | b | c |\n|---+---+---|\n| d | <point>e | f |", edit::move_cell_left), t("| a | b | c |\n|---+---+---|\n| e | d | f |\n"));
}

#[test]
fn move_rows() {
    assert!(fails("| a |", |d| edit::move_row(d, false)));
    assert!(fails("| a |\n| <point>b |", |d| edit::move_row(d, false)));
    assert_eq!(with("| a |\n| b |\n", |d| edit::move_row(d, false)), "| b |\n| a |");
    assert_eq!(with("| a |\n|---|\n", |d| edit::move_row(d, false)), "|---|\n| a |");
    assert_eq!(with("|---|\n| b |\n", |d| edit::move_row(d, false)), "| b |\n|---|");
    assert_eq!(with("|---|\n|---|\n", |d| edit::move_row(d, false)), "|---|\n|---|");
    assert!(fails("| a |", |d| edit::move_row(d, true)));
    assert_eq!(with("| a |\n| <point>b |\n", |d| edit::move_row(d, true)), "| b |\n| a |");
    assert_eq!(with("|---|\n| <point>b |\n", |d| edit::move_row(d, true)), "| b |\n|---|");
    assert_eq!(with("| a |\n|<point>---|\n", |d| edit::move_row(d, true)), "|---|\n| a |");
}

// ---- shrunk columns (state; display in super::display) ----

fn shrunk_after(text: &str, f: impl FnOnce(&mut Doc) -> Result<(), String>) -> Vec<usize> {
    let mut d = Doc::new(text);
    f(&mut d).unwrap();
    d.shrunk()
}

#[test]
fn toggle_column_width() {
    assert!(fails("<point>a", |d| super::toggle_column_width(d, crate::org::Prefix::None, None)));
    let toggle = |d: &mut Doc| super::toggle_column_width(d, crate::org::Prefix::None, None);
    assert_eq!(shrunk_after("| <point>a |", toggle), vec![1]);
    assert_eq!(shrunk_after("| a |\n|-<point>--|", toggle), vec![1]);
    assert_eq!(
        shrunk_after("| <point>a |", |d| {
            toggle(d)?;
            toggle(d)
        }),
        Vec::<usize>::new()
    );
    let sel = |s: &'static str| move |d: &mut Doc| super::toggle_column_width(d, crate::org::Prefix::None, Some(s));
    assert_eq!(shrunk_after("| <point>a | b |", sel("2")), vec![2]);
    assert_eq!(shrunk_after("| a | b | c | d |", sel("2-3")), vec![2, 3]);
    assert_eq!(shrunk_after("| a | b | c | d |", sel("2-")), vec![2, 3, 4]);
    assert_eq!(shrunk_after("| a | b | c | d |", sel("-2")), vec![1, 2]);
    assert_eq!(shrunk_after("| a | b | c | d |", sel("-")), vec![1, 2, 3, 4]);
    assert_eq!(
        shrunk_after("| a | b | c | d |", |d| {
            sel("1-3")(d)?;
            sel("2-4")(d)
        }),
        vec![1, 4]
    );
    assert_eq!(
        shrunk_after("| a | b | c | d |", |d| {
            sel("-")(d)?;
            super::toggle_column_width(d, crate::org::Prefix::U(2), None)
        }),
        Vec::<usize>::new()
    );
    // The truncated display.
    let table = vec!["| <3>  |".to_owned(), "| abcd |".to_owned()];
    assert_eq!(super::shrunk_line("| abcd |", &table, &[1]), "| abc…|");
    let table = vec!["| <3>  |".to_owned(), "| a   |".to_owned()];
    assert_eq!(super::shrunk_line("| a   |", &table, &[1]), "| a  …|");
    assert_eq!(super::shrunk_line("|-----|", &["| <3> |".into(), "|-----|".into()], &[1]), "|----…|");
    assert_eq!(super::shrunk_line("| a | b |", &["| a | b |".into()], &[1]), "|…| b |");
}

#[test]
fn shrunk_columns_follow_structure_edits() {
    let shrink1 = |d: &mut Doc| super::toggle_column_width(d, crate::org::Prefix::None, None);
    assert_eq!(
        shrunk_after("| <point>a | b |", |d| {
            shrink1(d)?;
            edit::move_column(d, false)
        }),
        vec![2]
    );
    assert_eq!(
        shrunk_after("| <point>a |", |d| {
            shrink1(d)?;
            edit::insert_column(d)
        }),
        vec![2]
    );
    assert_eq!(
        shrunk_after("| a | <point>b | c |", |d| {
            super::toggle_column_width(d, crate::org::Prefix::None, Some("-"))?;
            edit::delete_column(d)
        }),
        vec![1, 2]
    );
    assert_eq!(
        shrunk_after("| <point>a1 | a2 |\n| b1 | b2 |", |d| {
            super::toggle_column_width(d, crate::org::Prefix::None, Some("-"))?;
            edit::sort_lines(d, false, 'A', None, None)
        }),
        vec![2]
    );
    assert_eq!(
        shrunk_after("| 1 | <point>0 |\n#+TBLFM: $2=$1+1\n", |d| {
            shrink1(d)?;
            formula::recalculate(d, Recalc::Line, false)
        }),
        vec![2]
    );
}

#[test]
fn current_column_and_get_field() {
    let col = |s: &str| Doc::new(s).current_column();
    assert_eq!(col("| <point>a |"), 1);
    assert_eq!(col("|-<point>--|"), 1);
    assert_eq!(col("| 1 | <point>2 |"), 2);
    assert_eq!(col("|---+-<point>--|"), 2);
    let get = |s: &str, n: Option<usize>| Doc::new(s).get_field(n, None);
    assert_eq!(get("| <point>a |", None), " a ");
    assert_eq!(get("| <point>a ", None), " a ");
    assert_eq!(get("|<point>|", None), "");
    assert_eq!(get("| <point>|", None), " ");
    assert_eq!(get("<point>| a |", None), "");
    assert_eq!(get("| a |<point>", None), "");
    assert_eq!(get("| 1 | 2 | 3 |", Some(3)), " 3 ");
    assert_eq!(get("| 1 | 2 |\n<point>| 3 | 4 |", Some(2)), " 4 ");
    let rep = |s: &str, n: Option<usize>, r: &str| {
        let mut d = Doc::new(s);
        let old = d.get_field(n, Some(r));
        (old, d.text())
    };
    assert_eq!(rep("| <point>1 |", None, " foo ").1, "| foo |");
    assert_eq!(rep("| 1 | 2 | 3 |", Some(3), " foo ").1, "| 1 | 2 | foo |");
    assert_eq!(rep("| <point>1 |", None, "").1, "| |");
    assert_eq!(rep("| <point>1 |", None, " foo ").0, " 1 ");
}

// ---- structure commands not covered upstream ----

#[test]
fn rows_hlines_and_kill() {
    assert_eq!(with("| a | b |\n| <point>c | d |", |d| edit::insert_row(d, false)), "| a | b |\n|   |   |\n| c | d |");
    assert_eq!(with("| a | b |\n| <point>c | d |", |d| edit::insert_row(d, true)), "| a | b |\n| c | d |\n|   |   |");
    assert_eq!(with("| <point>a | b |\n| c | d |", |d| edit::insert_hline(d, false)), "| a | b |\n|---+---|\n| c | d |");
    assert_eq!(with("| a | b |\n| <point>c | d |", |d| edit::insert_hline(d, true)), "| a | b |\n|---+---|\n| c | d |");
    assert_eq!(with("| a |\n| <point>b |\n| c |\n#+TBLFM: @3$1=1", edit::kill_row), "| a |\n| c |\n#+TBLFM: @2$1=1");
    assert_eq!(with("| a |\n| <point>b |", |d| edit::hline_and_move(d, false)), "| a |\n| b |\n|---|\n|   |");
    assert_eq!(with("| <point>a |\n| b |\n#+TBLFM: @2$1=7", |d| edit::insert_row(d, false)), "|   |\n| a |\n| b |\n#+TBLFM: @3$1=7");
}

#[test]
fn create_convert_import() {
    assert_eq!(with("<point>", |d| edit::create(d, "3x2")), "|   |   |   |\n|---+---+---|\n|   |   |   |");
    assert_eq!(with("a,b\nc,d", |d| edit::convert_region(d, 0, 1, edit::Sep::Auto)), "| a | b |\n| c | d |");
    assert_eq!(with("a\tb c\nd\te", |d| edit::convert_region(d, 0, 1, edit::Sep::Auto)), "| a | b c |\n| d | e   |");
    assert_eq!(with("a  b\nc d", |d| edit::convert_region(d, 0, 1, edit::Sep::Auto)), "| a | b |\n| c | d |");
    assert_eq!(with("a  b\nc d", |d| edit::convert_region(d, 0, 1, edit::Sep::Spaces(2))), "| a | b   |\n| c d |     |");
    assert_eq!(with("\"x, y\",z", |d| edit::convert_region(d, 0, 0, edit::Sep::Comma)), "| x, y | z |");
    assert_eq!(with("<point>", |d| edit::import(d, "1;2\n3;4\n", edit::Sep::Regex(";".into()))), "| 1 | 2 |\n| 3 | 4 |");
    assert_eq!(with("| <point>a | b |\n|---+---|\n| c | d |", edit::convert), "+---+---+\n| a | b |\n+---+---+\n| c | d |\n+---+---+");
    assert_eq!(with("+---+---+\n| <point>a | b |\n+---+---+", edit::convert), "| a | b |");
}

#[test]
fn rectangles_transpose_wrap_sum() {
    let mut d = Doc::new("| <point>a | b |\n| c | d |");
    edit::copy_region(&mut d, (0, 2), (1, 2), true).unwrap();
    assert_eq!(d.text(), "|   | b |\n|   | d |");
    d.goto(0, 6);
    edit::paste_rectangle(&mut d).unwrap();
    assert_eq!(d.text(), "|   | a |\n|   | c |");
    assert_eq!(with("| 1 | 2 | 4 | 5 |\n|---+---+---+---|\n| a | b | c | d |\n| e | f | g | h |", edit::transpose), "| 1 | a | e |\n| 2 | b | f |\n| 4 | c | g |\n| 5 | d | h |");
    assert_eq!(with("| abc <point>def |\n|  |", |d| edit::wrap_region(d, None)), "| abc |\n| def |");
    assert_eq!(edit::wrap_text("one two three four", 2), vec!["one two", "three four"]);
    let mut d = Doc::new("| <point>1 |\n| 2.5 |\n| x |");
    assert_eq!(edit::sum(&mut d, None).unwrap(), "3.5");
    let mut d = Doc::new("| <point>1:30 |\n| 2:45 |");
    assert_eq!(edit::sum(&mut d, None).unwrap(), "4:15:00");
}

#[test]
fn recalc_marks_and_auto_line() {
    assert_eq!(with("| <point>a | 1 |", |d| edit::rotate_recalc_marks(d, None)), "| # | a | 1 |");
    assert_eq!(with("| # | <point>a |", |d| edit::rotate_recalc_marks(d, None)), "| * | a |");
    // `#` lines recalculate when leaving them with TAB.
    let out = with("| # | 2 | <point>  |\n#+TBLFM: $3=$2*2", |d| {
        d.st.may_need_update = true;
        edit::next_field(d)
    });
    assert!(out.starts_with("| # | 2 | 4 |"), "{out}");
    // `*` lines only with the whole table; header lines not at all.
    let out = with("| <point>! | x | y |\n| * | 2 |   |\n|   | 3 |   |\n#+TBLFM: $3=$2+1", |d| formula::recalculate(d, Recalc::All, false));
    assert!(out.contains("| * | 2 | 3 |") && out.contains("|   | 3 |   |"), "{out}");
}

#[test]
fn inline_formula_and_field_info() {
    let out = with("| 2 | <point>=$1*3 |", edit::next_field);
    assert!(out.starts_with("| 2 | 6 |"), "{out}");
    assert!(out.contains("#+TBLFM: $2=$1*3"), "{out}");
    let out = with("| 2 | <point>:=$1+1 |", edit::next_field);
    assert!(out.contains("#+TBLFM: @1$2=$1+1"), "{out}");
    let mut d = Doc::new("| 1 | <point>2 |\n#+TBLFM: $2=$1*2");
    formula::field_info(&mut d).unwrap();
    assert_eq!(d.msg.as_deref(), Some("line @1, col $2, ref @1$2 or B1, formula: $2=$1*2"));
}

#[test]
fn constants_parameters_and_formats() {
    let calc = |s: &str| with(s, |d| formula::calc_current_tblfm(d, Recalc::All));
    assert!(calc("#+CONSTANTS: c=3\n| 2 |   |\n<point>#+TBLFM: $2=$1*$c").contains("| 2 | 6 |"));
    assert!(calc("| $ | k=10 |\n|   | 2    |   |\n<point>#+TBLFM: @2$3=$2*$k").contains("| 20 |"));
    assert!(calc("| 1 | 3 |   |\n<point>#+TBLFM: $3=$1/$2;%.2f").contains("| 0.33 |"));
    assert!(calc("| 1 | 3 |   |\n<point>#+TBLFM: $3=$1/$2;f3").contains("| 0.333 |"));
    assert!(calc("| 1 | 3 |   |\n<point>#+TBLFM: $3=$1/$2;F").contains("| 1:3 |"));
    assert!(calc("| 4 |   |\n<point>#+TBLFM: $2=sqrt($1);S").contains("| 4 | 2 |"));
    assert!(calc("| 3 |   |\n<point>#+TBLFM: $2=sqrt($1);S").contains("sqrt(3)"));
    assert!(calc("| 90 |   |\n<point>#+TBLFM: $2=sin($1)").contains("| 90 | 1 |"));
    assert!(calc("| 0 |   |\n<point>#+TBLFM: $2=cos($1);R").contains("| 0 | 1 |"));
    assert!(calc("| 2 | 3 |   |\n<point>#+TBLFM: $3=$1..$2").contains("[2, 3]"));
    assert!(calc("| 5 |   |\n<point>#+TBLFM: $2=@#*10 + $#").contains("| 5 | 12 |"));
    assert!(calc("| 1 | 2 |   |\n<point>#+TBLFM: $3=vsum($1..$2);p20").contains("| 3 |"));
}

// ---- Calc ----

#[test]
fn calc_matches_emacs() {
    // Results recorded from Emacs Calc with org-calc-default-modes.
    let cases = [
        ("1/3", "0.33333333"),
        ("7/2", "3.5"),
        ("6/3", "2"),
        ("2.5*2", "5."),
        ("3.0", "3."),
        ("1+2.0", "3."),
        ("(z)+(1)", "z + 1"),
        ("(1)+(z)", "1 + z"),
        ("2*z", "2 z"),
        ("z*2", "2 z"),
        ("z/2", "z / 2"),
        ("(c   d)", "c d"),
        ("(2012-12)", "2000"),
        ("1e20", "1e20"),
        ("1.23456789e20", "1.2345679e20"),
        ("123456789012", "123456789012"),
        ("0.0001", "1e-4"),
        ("12345678.9", "12345679."),
        ("123456789.5", "123456790."),
        ("1/7*1e10", "1.4285714e-11"),
        ("2^0.5", "1.4142136"),
        ("sqrt(2)", "1.4142136"),
        ("sqrt(4)", "2"),
        ("exp(1)", "2.7182818"),
        ("ln(10)", "2.3025851"),
        ("log10(100)", "2"),
        ("sin(30)", "0.5"),
        ("vsum([2,4,8])", "14"),
        ("vmean([1,3,5,7])", "4"),
        ("vmean([4.1,8.2])", "6.15"),
        ("vmean([])", "vmean([])"),
        ("typeof(vmean([]))", "vmean"),
        ("typeof(1.5)", "3"),
        ("typeof(z)", "100"),
        ("typeof(nan)", "12"),
        ("typeof(\"ab\")", "101"),
        ("vlen(\"abc\")", "3"),
        ("\"(z)\" == \"nan\"", "0"),
        ("\"nan\" == \"nan\"", "1"),
        ("if(\"nan\" == \"nan\", x, string(\"\"))", "x"),
        ("string(subvec(\"(a b)\", 2, vlen(\"(a b)\")))", "a b"),
        ("(<2016-07-08 Fri>)-(<2016-07-07 Thu>)", "1"),
        ("(<2012-12-31 Mon>)", "<2012-12-31 Mon>"),
        ("(nan)+1", "nan"),
        ("inf+1", "inf"),
        ("vmax([1,5,3])", "5"),
        ("vmedian([1,5,3,4])", "3.5"),
        ("vsdev([1,2,3,4])", "1.2909944"),
        ("vvar([1,2,3,4])", "1.6666667"),
        ("vpsdev([1,2,3,4])", "1.1180340"),
        ("vpvar([1,2,3,4])", "1.25"),
        ("vprod([1,2,3,4])", "24"),
        ("round(2.5)", "3"),
        ("round(-2.5)", "-3"),
        ("floor(2.7)", "2"),
        ("ceil(2.1)", "3"),
        ("trunc(-2.7)", "-2"),
        ("-7 % 3", "2"),
        ("idiv(7,2)", "3"),
        ("2^10", "1024"),
        ("2^-1", "0.5"),
        ("10%", "0.1"),
        ("50% * 200", "100."),
        ("1:3 + 1", "4:3"),
        ("3 == 3.0", "1"),
        ("3 || 0", "3"),
        ("5!", "120"),
        ("z + 1 + 2", "z + 3"),
        ("1 + z + 2", "1 + z + 2"),
        ("z*z", "z^2"),
        ("z/z", "1"),
        ("(a b)/(a)", "b"),
        ("a/(b c)", "a / (b c)"),
        ("-(z+1)", "-z - 1"),
        ("z + -1", "z - 1"),
        ("3 z - z", "2 z"),
        ("z/2.0", "z / 2."),
        ("z * 2.5", "2.5 z"),
        ("foo(3, z)", "foo(3, z)"),
        ("[1,2,3]+1", "[2, 3, 4]"),
        ("[1,2]*[3,4]", "11"),
        ("\"abc\"", "[97, 98, 99]"),
        ("if(z, 1, 2)", "z ? 1 : 2"),
        ("(z) == (1)", "z = 1"),
        ("1 2", "2"),
        ("inf - inf", "nan"),
        ("1/inf", "0"),
        ("1.5e3", "1500."),
        ("12.3 +.0", "12.3"),
        ("(1:3)+1.0", "1.3333333"),
        ("1@ 2' 3\" + 1@", "2@ 2' 3\""),
        ("2:12", "1:6"),
        ("(<2016-07-08 Fri 10:00>) - (<2016-07-08 Fri 08:30>)", "0.0625"),
        ("(<2016-07-08 Fri>) + 1", "<2016-07-09 Sat>"),
        ("date(2016, 7, 8)", "<2016-07-08 Fri>"),
        ("round(1.23456789, 3)", "1.235"),
        ("0.1 + 0.2", "0.3"),
        ("1/3 * 3", "0.11111111"),
        ("0.01", "0.01"),
        ("2^100", "1267650600228229401496703205376"),
        ("(2.0)*(3)", "6."),
        ("(-7)\\2", "-4"),
        ("7 % -3", "-2"),
        ("7.5 % 2", "1.5"),
        ("vsum([])", "0"),
        ("vmax([])", "-inf"),
        ("vcount([])", "0"),
        ("vsdev([1])", "vsdev([1])"),
        ("sqrt(2) * sqrt(2)", "2.0000000"),
        ("arctan(1)", "45"),
        ("pi * 2", "2 pi"),
        ("evalv(pi)", "3.1415927"),
        ("[1, 2; 3, 4]", "[[1, 2], [3, 4]]"),
        ("date(1970,1,1)", "<1970-01-01 Thu>"),
        ("year(<2016-07-08 Fri>)", "2016"),
        ("weekday(<2016-07-08 Fri>)", "5"),
        ("<2016-07-08 Fri 10:30> + 0.5", "<2016-07-08 Fri 22:30>"),
        ("(<2016-07-08 Fri>) - 1.5", "<2016-07-06 Wed 12:00>"),
        ("hms(1,30,0) + 1", "2@ 30' 0\""),
        ("sqrt(8)", "2.8284271"),
        ("sqrt(1:4)", "1:2"),
        ("log10(1000)", "3"),
        ("log10(5)", "0.69897000"),
        ("log(8, 2)", "3"),
        ("cos(90)", "0"),
        ("cos(0.0)", "1."),
        ("arcsin(1)", "90"),
        ("sinh(1)", "1.1752012"),
        ("gcd(12, 18)", "6"),
        ("lcm(4, 6)", "12"),
        ("choose(5, 2)", "10"),
        ("perm(5, 2)", "20"),
        ("inv(4)", "0.25"),
        ("sqr(3)", "9"),
        ("exp10(2)", "100."),
        ("round(-2.45, 1)", "-2.5"),
        ("trunc(2.789, 2)", "2.78"),
        ("floor(2.789, 1)", "2.7"),
        ("frac(2.75)", "11:4"),
        ("rev([1,2,3])", "[3, 2, 1]"),
        ("sort([3,1,2])", "[1, 2, 3]"),
        ("rsort([3,1,2])", "[3, 2, 1]"),
        ("head([1,2,3])", "1"),
        ("tail([1,2,3])", "[2, 3]"),
        ("subvec([1,2,3,4], 2, 4)", "[2, 3]"),
        ("subvec([1,2,3,4], 2)", "[2, 3, 4]"),
        ("index(4)", "[1, 2, 3, 4]"),
        ("cvec(0, 3)", "[0, 0, 0]"),
        ("find([5,6,7], 6)", "2"),
        ("[1] | [2]", "[1, 2]"),
        ("vsum(1, 2, 3)", "6"),
        ("vmax([1.5, 2])", "2"),
        ("vgmean([1, 4])", "2"),
        ("vhmean([1, 4])", "1.6"),
        ("vsum([1, z])", "1 + z"),
        ("0.1 * 3", "0.3"),
        ("1e12 + 1", "1e12"),
        ("99999999.5", "100000000."),
        ("0.000123456789", "1.2345679e-4"),
        ("2 - -3", "5"),
        ("z - -1", "z + 1"),
        ("3 * -1", "-3"),
        ("-z * 2", "-2 z"),
        ("(z + 1) * 2", "2 z + 2"),
        ("(z + 1) y", "(z + 1) y"),
        ("(z + 1)^2", "(z + 1)^2"),
        ("z^2 / z", "z"),
        ("6 z / 3", "2 z"),
        ("z / 3 * 2", "z / 6"),
        ("2^3^2", "512"),
        ("-2^2", "-4"),
        ("(-2)^2", "4"),
        ("0^0", "1"),
        ("4^0.5", "2."),
        ("10^-2", "0.01"),
        ("(-2)^-1", "-0.5"),
        ("inf * -1", "-inf"),
        ("uinf + 1", "uinf"),
        ("nan == nan", "nan = nan"),
        ("1 < 2 && 2 < 3", "1"),
        ("!0", "1"),
        ("if(1 == 1, \"a\", \"b\")", "[97]"),
        ("typeof([1,2])", "101"),
        ("typeof(<2016-07-08 Fri>)", "10"),
        ("typeof(<2016-07-08 Fri 10:00>)", "11"),
        ("typeof(1:2)", "2"),
        ("typeof(-inf)", "neg"),
        ("typeof(z + 1)", "add"),
        ("string(\"a\" | \"b\")", "ab"),
        ("string(123)", "string(123)"),
    ];
    let m = calc::Modes::default();
    let mut bad = vec![];
    for (e, want) in cases {
        let got = calc::eval_str(e, &m).unwrap_or_else(|x| format!("ERR {x}"));
        if got != want {
            bad.push(format!("{e}: got {got:?}, want {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn calc_float_formats() {
    let f = |e: &str, fmt: calc::FloatFmt, figs: i32| {
        let m = calc::Modes { float_fmt: fmt, figs, ..calc::Modes::default() };
        calc::eval_str(e, &m).unwrap()
    };
    assert_eq!(f("1/2", calc::FloatFmt::Fix, 3), "0.500");
    assert_eq!(f("4/2", calc::FloatFmt::Fix, 3), "2");
    assert_eq!(f("1/3", calc::FloatFmt::Eng, 3), "333e-3");
    assert_eq!(f("10/3", calc::FloatFmt::Sci, 3), "3.33e0");
    assert_eq!(f("2/3", calc::FloatFmt::Sci, 3), "6.67e-1");
    assert_eq!(f("2/9", calc::FloatFmt::Float, 3), "0.222");
    assert_eq!(f("12.3", calc::FloatFmt::Fix, 1), "12.3");
    assert_eq!(f("96.", calc::FloatFmt::Fix, 1), "96.0");
    assert!(calc::eval_full("z", &calc::Modes::default(), true).is_err());
    assert!(calc::parse("1 +").is_err());
}

// ---- Lisp ----

#[test]
fn lisp_evaluator() {
    let ev = |s: &str| lisp::eval_formula(&format!("'{s}")).unwrap_or_else(|e| format!("ERR {e}"));
    assert_eq!(ev("(+ 1 2)"), "3");
    assert_eq!(ev("(/ 7 2)"), "3");
    assert_eq!(ev("(/ 7 2.0)"), "3.5");
    assert_eq!(ev("(* 1.0 3)"), "3.0");
    assert_eq!(ev("(format \"%.2f\" 3.14159)"), "3.14");
    assert_eq!(ev("(format \"%05d|%-3s|%x\" 42 \"a\" 255)"), "00042|a  |ff");
    assert_eq!(ev("(concat \"a\" \"b\")"), "ab");
    assert_eq!(ev("(let ((x 2) (y 3)) (* x y))"), "6");
    assert_eq!(ev("(let* ((x 2) (y (* x 3))) y)"), "6");
    assert_eq!(ev("(mapcar (lambda (x) (* x x)) '(1 2 3))"), "(1 4 9)");
    assert_eq!(ev("(apply '+ '(1 2 3))"), "6");
    assert_eq!(ev("(cond ((> 1 2) \"a\") (t \"b\"))"), "b");
    assert_eq!(ev("(substring \"hello\" 1 3)"), "el");
    assert_eq!(ev("(string-to-number \"12.5\")"), "12.5");
    assert_eq!(ev("(number-to-string 1e21)"), "1e+21");
    assert_eq!(ev("(number-to-string 100.0)"), "100.0");
    assert_eq!(ev("(upcase \"abc\")"), "ABC");
    assert_eq!(ev("(mapconcat 'identity '(\"a\" \"b\") \"-\")"), "a-b");
    assert_eq!(ev("(length \"abc\")"), "3");
    assert_eq!(ev("(nth 1 '(a b c))"), "b");
    assert_eq!(ev("(and 1 nil)"), "nil");
    assert_eq!(ev("(or nil 2)"), "2");
    assert_eq!(ev("(eq \"\" \"\")"), "t");
    assert_eq!(ev("(eq \"a\" \"a\")"), "nil");
    assert_eq!(ev("(equal \"a\" \"a\")"), "t");
    assert_eq!(ev("(replace-regexp-in-string \"a\\\\(b\\\\)\" \"<\\\\1>\" \"xaby\")"), "x<b>y");
    assert_eq!(ev("(if (string-match \"[0-9]+\" \"ab12\") \"yes\" \"no\")"), "yes");
    assert_eq!(ev("(sort '(3 1 2) '<)"), "(1 2 3)");
    assert_eq!(ev("(org-lookup-first 2 '(1 2 3) '(a b c))"), "b");
    assert_eq!(ev("(org-lookup-last 2 '(2 2 3) '(a b c))"), "b");
    assert_eq!(ev("(orgtbl-ascii-draw 19 0 24 3 \" 12345678\")"), "883");
    assert_eq!(ev("(calc-eval \"1/4\")"), "0.25");
    assert_eq!(ev("(type-of 1.5)"), "float");
    assert_eq!(ev("(floor 7 2)"), "3");
    assert_eq!(ev("(round 2.5)"), "2");
    assert_eq!(ev("(mod -7 3)"), "2");
    assert!(ev("(no-such-function 1)").starts_with("ERR"));
    assert_eq!(lisp::format_number("%.1f", "3.25").unwrap(), "3.2");
    assert_eq!(lisp::float_to_string(0.1), "0.1");
    assert_eq!(lisp::float_to_string(1e-5), "1e-05");
}

// ---- durations and references ----

#[test]
fn durations_and_time_strings() {
    assert_eq!(formula::time_string_to_seconds("2:12"), "7920");
    assert_eq!(formula::time_string_to_seconds("-2:07:00"), "-7620");
    assert_eq!(formula::time_string_to_seconds("12.5"), "12.5");
    assert_eq!(formula::time_string_to_seconds(""), "");
    assert_eq!(formula::time_seconds_to_string(14340.0, ""), "03:59:00");
    assert_eq!(formula::time_seconds_to_string(14340.0, "hh:mm"), "03:59");
    assert_eq!(formula::time_seconds_to_string(3320.0, "hours"), "0.92");
    assert_eq!(formula::time_seconds_to_string(-60.0, "minutes"), "-1.0");
    assert_eq!(formula::time_seconds_to_string(86400.0 * 1.5, "days"), "1.500");
}

#[test]
fn shift_references() {
    assert_eq!(formula::shift_refpart("3", false, false).unwrap(), "4");
    assert_eq!(formula::shift_refpart("-1", false, false).unwrap(), "+0");
    assert_eq!(formula::shift_refpart("1", true, false).unwrap(), "1");
    assert_eq!(formula::shift_refpart("II", true, true).unwrap(), "I");
    assert_eq!(formula::shift_refpart("B", false, false).unwrap(), "C");
    assert_eq!(formula::shift_reference("$2 = @3$1", 6, 'd').unwrap(), "$2 = @4$1");
    assert_eq!(formula::shift_reference("$2 = @3$1", 6, 'r').unwrap(), "$2 = @3$2");
    assert_eq!(formula::shift_reference("B3+1", 0, 'u').unwrap(), "B2+1");
    let lines = formula::fedit_lines(&[("$2".into(), "$1*2".into()), ("@1$1".into(), "5".into()), ("name".into(), "3".into())]);
    assert_eq!(lines, vec!["# Column Formulas", "$2 = $1*2", "", "# Field and Range Formulas", "@1$1 = 5", "", "# Named Field Formulas", "$name = 3"]);
    assert_eq!(formula::fedit_parse("# x\n$2 = $1*2\n@1$1 = 5\n").unwrap(), vec![("@1$1".to_owned(), "5".to_owned()), ("$2".to_owned(), "$1*2".to_owned())]);
    assert!(formula::fedit_parse("$2 = 1\n$2 = 2").is_err());
}

// ---- org-plot ----

#[test]
fn plot_script_and_data() {
    use super::plot;
    let mut p = plot::Plot::default();
    plot::add_options(&mut p, "title:\"Citas\" ind:1 deps:(3) type:2d with:histograms set:\"yrange [0:]\" set:\"xlabel 'x'\"");
    assert_eq!(p.get(":title").map(lisp::princ), Some("Citas".into()));
    assert_eq!(p.get(":ind").map(lisp::princ), Some("1".into()));
    let mut d = Doc::new("#+PLOT: title:\"T\" ind:1 deps:(2)\n| x | y |\n|---+---|\n| 1 | 2 |\n| 2 | 4 |");
    let (params, rows, ncol) = plot::prepare(&mut d, plot::Plot::default()).unwrap();
    assert_eq!(ncol, 2);
    assert_eq!(rows, vec![vec!["1".to_owned(), "2".to_owned()], vec!["2".to_owned(), "4".to_owned()]]);
    let s = plot::script(&rows, "/tmp/data", ncol, &params, false).unwrap();
    assert_eq!(s, "reset\nset term GNUTERM \n\nset title 'T'\nset datafile separator \"\\t\"\nplot '/tmp/data' using 1:2 with lines title 'y'");
    let data = plot::to_data(&rows.iter().map(|r| Row::Cells(r.clone())).collect::<Vec<_>>(), &params).unwrap();
    assert_eq!(data, "1\t2\n2\t4");
    assert_eq!(plot::quote_tsv_field("a\"b", "%Y"), "\"a\"\"b\"");
    assert_eq!(plot::quote_tsv_field("<2020-01-02 Thu>", "%Y-%m-%d"), "2020-01-02");
}
