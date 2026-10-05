//! Tests ported from testing/lisp/test-org-list.el (and checked against
//! upstream Emacs where the file has none).

use super::buf::Buf;
use super::export::{self, Gen, Params};
use super::structure::*;
use super::*;
use crate::editor::Editor;
use crate::org::Prefix;
use crate::org::tests::{org, shown};

/// An Org buffer with `text`; `<point>` marks point (else the start).
fn ed(text: &str) -> Editor {
    options::set(toml::Table::new());
    let (t, pt) = match text.find("<point>") {
        Some(i) => (text.replacen("<point>", "", 1), i),
        None => (text.to_owned(), 0),
    };
    let mut e = org(&t, "");
    let b = Buf::new(t.clone(), pt);
    e.cur.line = b.line_of(pt);
    e.cur.byte = pt - b.bol_at(pt);
    e
}

fn opt(name: &str, v: toml::Value) {
    options::put(name, v);
}

fn opt_lisp(name: &str, lisp: &str) {
    options::put(name, toml::Value::String(lisp.into()));
}

/// buffer-string.
fn text(e: &Editor) -> String {
    let mut t = e.buf.text();
    if e.buf.final_newline {
        t.push('\n');
    }
    t
}

fn pt(e: &Editor) -> usize {
    load(e).pt
}

fn rest(e: &Editor) -> String {
    text(e)[pt(e)..].to_owned()
}

fn run(e: &mut Editor, name: &str) -> Result<(), String> {
    run_arg(e, name, Prefix::None)
}

fn run_arg(e: &mut Editor, name: &str, arg: Prefix) -> Result<(), String> {
    command(e, name, arg).expect("a list command")
}

/// Move point to the start of line `n` (1-based, goto-line).
fn goto_line(e: &mut Editor, n: usize) {
    e.cur.line = n - 1;
    e.cur.byte = 0;
}

/// search-forward: point after `s`.
fn search(e: &mut Editor, s: &str) {
    let t = text(e);
    let p = pt(e) + t[pt(e)..].find(s).expect("search") + s.len();
    set_pt(e, p);
}

fn set_pt(e: &mut Editor, p: usize) {
    let b = Buf::new(text(e), p);
    e.cur.line = b.line_of(p);
    e.cur.byte = p - b.bol_at(p);
}

fn bol(e: &mut Editor) {
    e.cur.byte = 0;
}

/// Select from point to the end of the buffer (push-mark, point-max).
fn region_to_end(e: &mut Editor) {
    let lo = e.cur.line;
    let hi = e.line_count() - 1;
    e.org.as_mut().unwrap().region = Some((lo, hi));
}

fn after(t: &str, cmd: &str) -> String {
    let mut e = ed(t);
    run(&mut e, cmd).unwrap();
    text(&e)
}

fn in_item_at(t: &str, line: usize) -> Option<usize> {
    let mut e = ed(t);
    goto_line(&mut e, line);
    let mut b = load(&e);
    in_item(&mut b)
}

#[test]
fn list_ending() {
    assert_eq!(in_item_at("- item\n\n\n  Text", 4), None);
    assert_eq!(in_item_at("- item\nText", 2), None);
    assert_eq!(
        in_item_at(
            "- item\n  #+begin_quote\n\n\nText at column 0\n  #+end_quote\n Text",
            7
        ),
        Some(0)
    );
}

#[test]
fn list_navigation() {
    let t = "\n- item A\n- item B\n\n\n- item 1\n  - item 1.1\n  - item 1.2\n  - item 1.3\n- item 2\n\n\n- item X\n- item Y";
    let mut e = ed(t);
    let at = |e: &Editor, s: &str| rest(e).starts_with(s);
    goto_line(&mut e, 9);
    assert!(run(&mut e, "org-next-item").is_err());
    opt("org-list-use-circular-motion", true.into());
    assert!(run(&mut e, "org-next-item").is_ok());
    options::set(toml::Table::new());
    goto_line(&mut e, 14);
    assert!(run(&mut e, "org-next-item").is_err());
    goto_line(&mut e, 6);
    run(&mut e, "org-next-item").unwrap();
    assert!(at(&e, "- item 2"));
    goto_line(&mut e, 3);
    assert!(run(&mut e, "org-next-item").is_err());
    assert!(!at(&e, "- item 1"));
    opt("org-list-use-circular-motion", true.into());
    goto_line(&mut e, 10);
    run(&mut e, "org-next-item").unwrap();
    assert!(at(&e, "- item 1"));
    goto_line(&mut e, 9);
    run(&mut e, "org-next-item").unwrap();
    assert!(at(&e, "  - item 1.1"));
    options::set(toml::Table::new());
    goto_line(&mut e, 7);
    assert!(run(&mut e, "org-previous-item").is_err());
    goto_line(&mut e, 13);
    assert!(run(&mut e, "org-previous-item").is_err());
    goto_line(&mut e, 10);
    run(&mut e, "org-previous-item").unwrap();
    assert!(at(&e, "- item 1"));
    goto_line(&mut e, 6);
    assert!(run(&mut e, "org-previous-item").is_err());
    assert!(!at(&e, "- item B"));
    opt("org-list-use-circular-motion", true.into());
    goto_line(&mut e, 6);
    run(&mut e, "org-previous-item").unwrap();
    assert!(at(&e, "- item 2"));
    goto_line(&mut e, 7);
    run(&mut e, "org-previous-item").unwrap();
    assert!(at(&e, "  - item 1.3"));
    // Beginning and end of items and lists.
    goto_line(&mut e, 8);
    e.cur.byte = 5;
    run(&mut e, "org-beginning-of-item").unwrap();
    assert!(at(&e, "  - item 1.2"));
    run(&mut e, "org-beginning-of-item-list").unwrap();
    assert!(at(&e, "  - item 1.1"));
    run(&mut e, "org-end-of-item-list").unwrap();
    assert!(at(&e, "- item 2"));
    goto_line(&mut e, 6);
    run(&mut e, "org-end-of-item").unwrap();
    assert!(at(&e, "- item 2"));
    goto_line(&mut e, 1);
    assert_eq!(run(&mut e, "org-end-of-item"), Err("Not in an item".into()));
}

fn bullet(t: &str, which: Which) -> String {
    let mut e = ed(t);
    with_buf(&mut e, |b| cycle_list_bullet(b, &which)).unwrap();
    text(&e)
}

#[test]
fn cycle_bullet() {
    assert!(after_err("Paragraph", "org-cycle-list-bullet"));
    let mut e = ed("  - item");
    let mut seen = vec![];
    for _ in 0..4 {
        run(&mut e, "org-cycle-list-bullet").unwrap();
        seen.push(text(&e));
    }
    assert_eq!(seen, vec!["  + item", "  * item", "  1. item", "  1) item"]);
    assert_eq!(bullet("- item", Which::Bullet("1.".into())), "1. item");
    assert_eq!(bullet("1. item", Which::Index(1)), "+ item");
    assert_eq!(bullet("+ item", Which::Previous), "- item");
    assert_eq!(after("+ item", "org-cycle-list-bullet"), "1. item");
    assert_ne!(
        after("+ tag :: item", "org-cycle-list-bullet"),
        "1. tag :: item"
    );
    let mut e = ed("  * item");
    opt("org-plain-list-ordered-item-terminator", 41.into());
    run(&mut e, "org-cycle-list-bullet").unwrap();
    assert_eq!(text(&e), "  1) item");
    let mut e = ed("1) item");
    opt("org-list-allow-alphabetical", true.into());
    run(&mut e, "org-cycle-list-bullet").unwrap();
    assert_eq!(text(&e), "a. item");
    let long: String = (1..=27).map(|i| format!("{i}) item {i}\n")).collect();
    let mut e = ed(long.trim_end());
    opt("org-list-allow-alphabetical", true.into());
    run(&mut e, "org-cycle-list-bullet").unwrap();
    assert!(!text(&e).starts_with("a. item 1"));
    // Point is preserved while cycling.
    let mut e = ed("- this is test\n\n  - asd\n    - asd\n <point> - this is\n* headline\n");
    assert_eq!(pt(&e), 35);
    for _ in 0..10 {
        run(&mut e, "org-cycle-list-bullet").unwrap();
        assert_eq!(e.cur.byte, 1);
    }
    let mut e = ed("\n- this is test\n  + asd\n    - asd\n  <point>+ this is\n* headline\n");
    for _ in 0..10 {
        run(&mut e, "org-cycle-list-bullet").unwrap();
        assert_eq!(e.cur.byte, 2);
    }
    let mut e = ed("\n- this is test\n  + asd\n    - asd\n  +<point> this is\n* headline\n");
    for _ in 0..10 {
        run(&mut e, "org-cycle-list-bullet").unwrap();
        assert_eq!(e.cur.byte, 3);
    }
    let mut e = ed("\n- this is test\n  - asd\n    - asd\n  - <point>this is\n* headline\n");
    for i in 0..5 {
        run(&mut e, "org-cycle-list-bullet").unwrap();
        assert_eq!(e.cur.byte, if i < 2 || i == 4 { 4 } else { 5 }, "step {i}");
    }
    assert_eq!(
        after("\n1) text<point>\n text", "org-cycle-list-bullet"),
        "\n- text\n text"
    );
}

fn after_err(t: &str, cmd: &str) -> bool {
    let mut e = ed(t);
    run(&mut e, cmd).is_err()
}

#[test]
fn indent_item() {
    assert!(after_err("Paragraph.", "org-indent-item"));
    let mut e = ed("\n- Item 1\n- Item 2");
    goto_line(&mut e, 2);
    assert!(run(&mut e, "org-indent-item").is_err());
    let mut e = ed("\n- Item 1\n- Item 2");
    goto_line(&mut e, 2);
    opt_lisp("org-list-automatic-rules", "nil");
    assert!(run(&mut e, "org-indent-item").is_err());
    assert_eq!(
        after(
            "\n- Item 1\n- Item 2<point>\n  - Item 2.1",
            "org-indent-item"
        ),
        "\n- Item 1\n  - Item 2\n  - Item 2.1"
    );
    let mut e = ed("\n- Item 1\n- Item 2<point>");
    opt_lisp("org-list-demote-modify-bullet", "((\"-\" . \"+\"))");
    run(&mut e, "org-indent-item").unwrap();
    assert_eq!(text(&e), "\n- Item 1\n  + Item 2");
    let mut e = ed("\n- [ ] list item 1\n- [ ] list item 2<point>");
    opt_lisp("org-list-demote-modify-bullet", "((\"-\" . \"+\"))");
    run(&mut e, "org-indent-item").unwrap();
    assert_eq!(text(&e), "\n- [ ] list item 1\n  + [ ] list item 2");
    let mut e = ed("\n1. Item 1\n2. Item 2<point>");
    opt_lisp("org-list-demote-modify-bullet", "((\"1.\" . \"+\"))");
    run(&mut e, "org-indent-item").unwrap();
    assert_eq!(text(&e), "\n1. Item 1\n   + Item 2");
    let mut e = ed("\na. Item 1\nb. Item 2<point>");
    opt("org-list-allow-alphabetical", true.into());
    opt_lisp(
        "org-list-demote-modify-bullet",
        "((\"A.\" . \"a.\") (\"a.\" . \"-\"))",
    );
    run(&mut e, "org-indent-item").unwrap();
    assert_eq!(text(&e), "\na. Item 1\n   - Item 2");
    // A region.
    let mut e = ed("\n- Item 1\n<point>- Item 2\n- Item 3\n");
    region_to_end(&mut e);
    run(&mut e, "org-indent-item").unwrap();
    assert_eq!(text(&e), "\n- Item 1\n  - Item 2\n  - Item 3\n");
    // Point right after an empty item stays after it.
    let mut e = ed("\n- item\n- <point> ::");
    run(&mut e, "org-indent-item").unwrap();
    assert_eq!(pt(&e), 12);
    let mut e = ed("\n- item\n- <point> \t::");
    run(&mut e, "org-indent-item").unwrap();
    assert!(rest(&e).starts_with(" \t"));
}

#[test]
fn indent_item_tree() {
    assert!(after_err("Paragraph.", "org-indent-item-tree"));
    let mut e = ed("\n- Item 1\n- Item 2\n  - Item 2.1");
    search(&mut e, "- Item 2");
    run(&mut e, "org-indent-item-tree").unwrap();
    assert_eq!(text(&e), "\n- Item 1\n  - Item 2\n    - Item 2.1");
    let mut e = ed("\n- Item 1\n- Item 2");
    search(&mut e, "- Item 1");
    run(&mut e, "org-indent-item-tree").unwrap();
    assert_eq!(text(&e), "\n - Item 1\n - Item 2");
    let mut e = ed("\n- Item 1\n- Item 2\n  + Item 2.1");
    search(&mut e, "- Item 2");
    opt_lisp(
        "org-list-demote-modify-bullet",
        "((\"-\" . \"+\") (\"+\" . \"-\"))",
    );
    run(&mut e, "org-indent-item-tree").unwrap();
    assert_eq!(text(&e), "\n- Item 1\n  + Item 2\n    - Item 2.1");
    let mut e = ed("\n1. Item 1\n2. Item 2\n   + Item 2.1");
    search(&mut e, "2. Item 2");
    opt_lisp(
        "org-list-demote-modify-bullet",
        "((\"1.\" . \"+\") (\"+\" . \"1.\"))",
    );
    run(&mut e, "org-indent-item-tree").unwrap();
    assert_eq!(text(&e), "\n1. Item 1\n   + Item 2\n     1. Item 2.1");
    let mut e = ed("\n- Item 1\n- Item 2\n  - Item 2.1\n- Item 3\n  - Item 3.1\n");
    search(&mut e, "- Item 2");
    bol(&mut e);
    region_to_end(&mut e);
    run(&mut e, "org-indent-item-tree").unwrap();
    assert_eq!(
        text(&e),
        "\n- Item 1\n  - Item 2\n    - Item 2.1\n  - Item 3\n    - Item 3.1\n"
    );
}

#[test]
fn outdent_item() {
    assert!(after_err("Paragraph.", "org-outdent-item"));
    let mut e = ed("\n- Item 1\n- Item 2");
    goto_line(&mut e, 2);
    assert!(run(&mut e, "org-outdent-item").is_err());
    let mut e = ed("\n- Item 1\n  - Item 1.1\n    - Item 1.1.1");
    search(&mut e, "- Item 1.1");
    assert_eq!(
        run(&mut e, "org-outdent-item"),
        Err("Cannot outdent an item without its children".into())
    );
    let mut e = ed("\n  - Item 1\n  - Item 2");
    search(&mut e, "- Item 2");
    assert!(run(&mut e, "org-outdent-item").is_err());
    let mut e = ed("\n- Item 1\n  - Item 2\n  - Item 3\n");
    search(&mut e, "- Item 2");
    bol(&mut e);
    region_to_end(&mut e);
    run(&mut e, "org-outdent-item").unwrap();
    assert_eq!(text(&e), "\n- Item 1\n- Item 2\n- Item 3\n");
}

#[test]
fn outdent_item_tree() {
    assert!(after_err("Paragraph.", "org-outdent-item-tree"));
    let mut e = ed("\n  - Item 1\n  - Item 2");
    search(&mut e, "- Item 2");
    assert!(run(&mut e, "org-outdent-item-tree").is_err());
    let mut e = ed("\n- Item 1\n  - Item 2\n    - Item 2.1");
    search(&mut e, "- Item 2");
    run(&mut e, "org-outdent-item-tree").unwrap();
    assert_eq!(text(&e), "\n- Item 1\n- Item 2\n  - Item 2.1");
    let mut e = ed("\n - Item 1\n - Item 2");
    search(&mut e, "- Item 1");
    run(&mut e, "org-outdent-item-tree").unwrap();
    assert_eq!(text(&e), "\n- Item 1\n- Item 2");
    let mut e = ed("\n- Item 1\n  - Item 2\n    - Item 2.1\n  - Item 3\n    - Item 3.1\n");
    search(&mut e, "- Item 2");
    bol(&mut e);
    region_to_end(&mut e);
    run(&mut e, "org-outdent-item-tree").unwrap();
    assert_eq!(
        text(&e),
        "\n- Item 1\n- Item 2\n  - Item 2.1\n- Item 3\n  - Item 3.1\n"
    );
    // The `*' bullet at column 0 becomes `-'.
    let mut e = ed(" * Item 1\n * Item 2");
    run(&mut e, "org-outdent-item-tree").unwrap();
    assert_eq!(text(&e), "- Item 1\n- Item 2");
}

/// org-cycle-item-indentation, `last` times as a repeated command.
fn cycle_ind(t: &str, repeats: usize) -> (Result<bool, String>, Editor) {
    let mut e = ed(t);
    let mut state = None;
    let mut res = Ok(false);
    for i in 0..=repeats {
        let mut b = load(&e);
        let r = cycle_indentation(&mut b, i > 0, state.clone(), false);
        store(&mut e, b);
        res = match r {
            None => Ok(false),
            Some(Ok(s)) => {
                state = s;
                Ok(true)
            }
            Some(Err(m)) => Err(m),
        };
    }
    (res, e)
}

#[test]
fn cycle_item_indentation_spec() {
    assert_eq!(cycle_ind("- item - item2<point>", 0).0, Ok(false));
    assert_eq!(
        text(&cycle_ind("- item\n  - sub-item\n  - <point>", 0).1),
        "- item\n  - sub-item\n    - "
    );
    assert_eq!(text(&cycle_ind("- item\n  - <point>", 0).1), "- item\n- ");
    assert!(cycle_ind("- ", 0).0.is_err());
    let mut e = ed("- i0\n  - i1\n    - s1\n  - <point>");
    let mut state = None;
    let mut inds = vec![];
    for i in 0..4 {
        let mut b = load(&e);
        if let Some(Ok(s)) = cycle_indentation(&mut b, i > 0, state.clone(), false) {
            state = s;
        }
        store(&mut e, b);
        let l = e.buf.line(e.cur.line);
        inds.push(l.len() - l.trim_start().len());
    }
    assert_eq!(inds, vec![4, 6, 0, 2]);
    assert!(
        cycle_ind("- item\n  - <point>\n  - sub-item 2", 0)
            .0
            .is_err()
    );
    assert_eq!(
        text(&cycle_ind("1. item\n  - <point>", 1).1),
        "1. item\n   - "
    );
    assert_eq!(
        text(&cycle_ind("1. item\n  - tag :: <point>", 1).1),
        "1. item\n   - tag :: "
    );
    assert!(cycle_ind("- item\n- <point>", 1).0.is_ok());
    // Through TAB: org-cycle runs it on an empty item.
    let mut e = ed("- item\n- ");
    goto_line(&mut e, 2);
    e.cur.byte = 2;
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(text(&e), "- item\n  - ");
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(text(&e), "- item\n- ");
}

#[test]
fn move_item_down() {
    assert_eq!(
        after("- item 1\n- item 2", "org-move-item-down"),
        "- item 2\n- item 1"
    );
    let mut e = ed("- it<point>em 1\n- item 2");
    run(&mut e, "org-move-item-down").unwrap();
    assert!(rest(&e).starts_with("em 1"));
    assert_eq!(
        after("- item 1\n  - sub-item 1\n- item 2", "org-move-item-down"),
        "- item 2\n- item 1\n  - sub-item 1"
    );
    assert_eq!(
        after("- item 1\n\n- item 2", "org-move-item-down"),
        "- item 2\n\n- item 1"
    );
    let mut e = ed("- item 1\n- item 2");
    goto_line(&mut e, 2);
    assert!(run(&mut e, "org-move-item-down").is_err());
    let mut e = ed("- item 1\n- item 2\n<point>- item 3");
    opt("org-list-use-circular-motion", true.into());
    run(&mut e, "org-move-item-down").unwrap();
    assert_eq!(text(&e), "- item 3\n- item 1\n- item 2\n");
    // Item visibility is preserved.
    let mut e = ed("* Headline\n<point>- item 1\n  body 1\n- item 2\n  body 2");
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    goto_line(&mut e, 4);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(shown(&e), vec!["* Headline", "- item 1...", "- item 2..."]);
    goto_line(&mut e, 2);
    run(&mut e, "org-move-item-down").unwrap();
    assert_eq!(shown(&e), vec!["* Headline", "- item 2...", "- item 1..."]);
    assert_eq!(
        text(&e),
        "* Headline\n- item 2\n  body 2\n- item 1\n  body 1"
    );
    // Children visibility too.
    let mut e = ed(
        "* Headline\n- item 1\n  - sub-item 1\n    sub-body 1\n- item 2\n  - sub-item 2\n    sub-body 2",
    );
    goto_line(&mut e, 3);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    goto_line(&mut e, 6);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    goto_line(&mut e, 2);
    run(&mut e, "org-move-item-down").unwrap();
    assert_eq!(
        shown(&e),
        vec![
            "* Headline",
            "- item 2",
            "  - sub-item 2...",
            "- item 1",
            "  - sub-item 1..."
        ]
    );
}

#[test]
fn move_item_keeps_contents_visibility() {
    let t = "\n- item 1\n  #+BEGIN_CENTER\n  Text1\n  #+END_CENTER\n- item 2\n  #+BEGIN_CENTER\n  Text2\n  #+END_CENTER";
    let mut e = ed(t);
    crate::org::run(&mut e, "org-fold-hide-block-all", Prefix::None);
    let before = shown(&e);
    assert!(!before.iter().any(|l| l.contains("Text")));
    search(&mut e, "- item 1");
    run(&mut e, "org-move-item-down").unwrap();
    assert_eq!(
        shown(&e),
        vec![
            "",
            "- item 2",
            "  #+BEGIN_CENTER...",
            "- item 1",
            "  #+BEGIN_CENTER..."
        ]
    );
    let mut e = ed(t);
    crate::org::run(&mut e, "org-fold-hide-block-all", Prefix::None);
    search(&mut e, "- item 2");
    run(&mut e, "org-move-item-up").unwrap();
    assert_eq!(text(&e).matches("Text2").count(), 1);
    assert!(!shown(&e).iter().any(|l| l.contains("Text")));
}

#[test]
fn move_item_up() {
    let mut e = ed("- item 1\n- item 2");
    goto_line(&mut e, 2);
    run(&mut e, "org-move-item-up").unwrap();
    assert_eq!(text(&e), "- item 2\n- item 1");
    let mut e = ed("- item 1\n- item 2");
    goto_line(&mut e, 2);
    e.cur.byte = 4;
    run(&mut e, "org-move-item-up").unwrap();
    assert!(rest(&e).starts_with("em 2"));
    let mut e = ed("- item 1\n- item 2\n  - sub-item 2");
    goto_line(&mut e, 2);
    run(&mut e, "org-move-item-up").unwrap();
    assert_eq!(text(&e), "- item 2\n  - sub-item 2\n- item 1");
    let mut e = ed("- item 1\n\n- item 2");
    search(&mut e, "- item 2");
    run(&mut e, "org-move-item-up").unwrap();
    assert_eq!(text(&e), "- item 2\n\n- item 1");
    assert!(after_err("- item 1\n- item 2", "org-move-item-up"));
    let mut e = ed("- item 1\n- item 2\n- item 3");
    opt("org-list-use-circular-motion", true.into());
    run(&mut e, "org-move-item-up").unwrap();
    assert_eq!(text(&e), "- item 2\n- item 3\n- item 1");
    // Visibility.
    let mut e = ed("* Headline\n- item 1\n  body 1\n- item 2\n  body 2");
    goto_line(&mut e, 2);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    goto_line(&mut e, 4);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    run(&mut e, "org-move-item-up").unwrap();
    assert_eq!(shown(&e), vec!["* Headline", "- item 2...", "- item 1..."]);
}

fn blank_before(t: &str, v: &str, line_offset: i64) -> bool {
    let mut e = ed(t);
    opt_lisp(
        "org-blank-before-new-entry",
        &format!("((plain-list-item . {v}))"),
    );
    if !t.contains("<point>") {
        let mut b = load(&e);
        b.goto(b.eol());
        store(&mut e, b);
    }
    run(&mut e, "org-insert-item").unwrap();
    let l = (e.cur.line as i64 + line_offset) as usize;
    e.buf.line(l).trim().is_empty()
}

#[test]
fn insert_item() {
    assert!(blank_before("- a", "t", -1));
    assert!(!blank_before("- a", "nil", -1));
    assert!(!blank_before("- a", "auto", -1));
    assert!(blank_before("- a\n\n  b<point>", "auto", -1));
    // Fred has no line after a final newline: point on the last line.
    assert!(blank_before("- a\n\n<point>\n", "auto", -1));
    assert!(blank_before("- a\n\n- b<point>", "auto", -1));
    let mut e = ed("- a\n\n- b");
    run(&mut e, "org-insert-item").unwrap();
    assert!(e.buf.line(e.cur.line + 1).is_empty());
    assert!(blank_before(
        "- a\n  #+BEGIN_EXAMPLE\n\n  x\n  #+END_EXAMPLE<point>",
        "auto",
        -1
    ));
    assert_eq!(after("- item", "org-insert-item"), "- \n- item");
    assert_eq!(after("- <point>item", "org-insert-item"), "- \n- item");
    assert_eq!(
        after("- A\n\n - B\n\n<point>\n", "org-insert-item"),
        "- A\n\n  - B\n\n  - \n"
    );
    assert_eq!(
        after("- A\n\n  - B\n\n  <point>\n", "org-insert-item"),
        "- A\n\n  - B\n\n  - \n"
    );
    assert_eq!(
        after("- tag <point>:: item", "org-insert-item"),
        "-  :: \n- tag :: item"
    );
    assert_eq!(
        after("- ta<point>g :: item", "org-insert-item"),
        "-  :: \n- tag :: item"
    );
    assert_eq!(after("- it<point>em", "org-insert-item"), "- it\n- em");
    let mut e = ed("- it<point>em");
    opt_lisp("org-M-RET-may-split-line", "((default . nil))");
    run(&mut e, "org-insert-item").unwrap();
    assert_eq!(text(&e), "- item\n- ");
    assert_eq!(
        after("1. A<point>\n\n2. \n\n3. B", "org-insert-item"),
        "1. A\n\n2. \n\n3. \n\n4. B"
    );
    assert_eq!(
        after("1. a<point>\n   b\n2. c", "org-insert-item"),
        "1. a\n2. \n   b\n3. c"
    );
    let mut e = ed("- item\n  - sub-list\n<point>  resume item");
    run(&mut e, "org-insert-item").unwrap();
    assert_eq!(
        (e.cur.line, e.cur.byte),
        (2, 4),
        "{:?} {}",
        text(&e),
        pt(&e)
    );
    let mut e = ed("- item\n  - sub-list\n  resume item<point>");
    run(&mut e, "org-insert-item").unwrap();
    let l = e.buf.line(e.cur.line);
    assert_eq!(l.len() - l.trim_start().len(), 0);
    assert_eq!(
        after("- A\n  B <point> C\n  - D\n- [ ] E", "org-insert-item"),
        "- A\n  B\n- C\n  - D\n- [ ] E"
    );
    // Point after the new bullet, and with a checkbox (C-u).
    let mut e = ed("- a<point>");
    run_arg(&mut e, "org-insert-item", Prefix::U(1)).unwrap();
    assert_eq!(text(&e), "- a\n- [ ] ");
    assert_eq!(pt(&e), text(&e).len());
    // Description lists ask for the term.
    let mut e = ed("- a :: b<point>");
    run(&mut e, "org-insert-item").unwrap();
    assert_eq!(text(&e), "- a :: b\n-  :: ");
    assert_eq!(pt(&e), 11);
    // List visibility is preserved.
    let mut e = ed("- A\n  - B\n- C\n  - D");
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    goto_line(&mut e, 3);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    run(&mut e, "org-insert-item").unwrap();
    assert_eq!(shown(&e), vec!["- A...", "- ", "- C..."]);
    // Not in an item: nothing.
    let mut e = ed("text");
    assert_eq!(insert_item_cmd(&mut e, false), Ok(false));
}

fn send(
    t: &str,
    item: impl Fn(&Struct, &Buf) -> usize,
    dest: impl Fn(&Struct, &Buf) -> Dest,
) -> (String, Option<String>) {
    let mut e = ed(t);
    let mut b = load(&e);
    let st = list_struct(&mut b);
    let (i, d) = (item(&st, &b), dest(&st, &b));
    let (_, k) = send_item(&mut b, i, d, &st);
    store(&mut e, b);
    (text(&e), k)
}

#[test]
fn send_item_spec() {
    let at = |s: &'static str| move |_: &Struct, b: &Buf| b.s.find(s).unwrap();
    assert_eq!(
        send(
            "- item1\n- item2\n- item3\n",
            |s, _| s.0.last().unwrap().pos,
            |_, _| Dest::Begin
        )
        .0,
        "- item3\n- item1\n- item2\n"
    );
    assert_eq!(
        send(
            "- item1\n- item2\n- item3\n  - item4\n",
            |s, _| s.0[2].pos,
            |_, _| Dest::Begin
        )
        .0,
        "- item3\n  - item4\n- item1\n- item2\n"
    );
    assert_eq!(
        send(
            "- item1\n  - item1child\n- item2\n- item3\n  - item4\n",
            |s, _| s.0[0].pos,
            |_, _| Dest::End
        )
        .0,
        "- item2\n- item3\n  - item4\n- item1\n  - item1child\n"
    );
    let t = "- item1\n  - item1child\n- item2\n- item3\n- item4\n- item5\n";
    assert_eq!(
        send(t, |s, _| s.0[0].pos, |_, _| Dest::Nth(3)).0,
        "- item2\n- item1\n  - item1child\n- item3\n- item4\n- item5\n"
    );
    assert_eq!(
        send(
            t,
            |s, _| s.0[0].pos,
            |_, b| Dest::Pos(b.bol_at(b.s.find("item3").unwrap()))
        )
        .0,
        "- item2\n- item1\n  - item1child\n- item3\n- item4\n- item5\n"
    );
    assert_eq!(
        send(
            t,
            |_, b| b.bol_at(at("item3")(&Struct::default(), b)),
            |_, _| Dest::Delete
        )
        .0,
        "- item1\n  - item1child\n- item2\n- item4\n- item5\n"
    );
    let (t2, k) = send(
        "- item1\n  - item1child\n- item2\n- item3\n  - item3child\n- item4\n- item5\n",
        |_, b| b.bol_at(b.s.find("item3").unwrap()),
        |_, _| Dest::Kill,
    );
    assert_eq!(t2, "- item1\n  - item1child\n- item2\n- item4\n- item5\n");
    assert_eq!(k.as_deref(), Some("item3\n  - item3child"));
}

#[test]
fn repair() {
    assert_eq!(
        after("- item\n - child", "org-list-repair"),
        "- item\n  - child"
    );
    assert_eq!(after("- a\n+ b", "org-list-repair"), "- a\n- b");
    assert_eq!(after("1. a\n1. b", "org-list-repair"), "1. a\n2. b");
    assert_eq!(
        after("- [ ] item\n  - [X] child", "org-list-repair"),
        "- [X] item\n  - [X] child"
    );
    assert_eq!(
        after("- item\n    - child\n    within item", "org-list-repair"),
        "- item\n  - child\n  within item"
    );
    assert_eq!(
        after(
            "- item\n    - child\n   within item\n     indented",
            "org-list-repair"
        ),
        "- item\n  - child\n  within item\n    indented"
    );
    // Counters and alphabetical bullets.
    assert_eq!(
        after("1. a\n5. [@5] b\n1. c", "org-list-repair"),
        "1. a\n5. [@5] b\n6. c"
    );
    let mut e = ed("a) x\nb) y\n[@c] z");
    opt("org-list-allow-alphabetical", true.into());
    run(&mut e, "org-list-repair").unwrap();
    assert_eq!(text(&e), "a) x\nb) y\n[@c] z");
    let mut e = ed("a) x\nq) y\nz) [@d] z");
    opt("org-list-allow-alphabetical", true.into());
    run(&mut e, "org-list-repair").unwrap();
    assert_eq!(text(&e), "a) x\nb) y\nd) [@d] z");
    // org-list-indent-offset.
    let mut e = ed("- a\n  - b");
    opt("org-list-indent-offset", 2.into());
    run(&mut e, "org-list-repair").unwrap();
    assert_eq!(text(&e), "- a\n    - b");
    // Two spaces after numbered bullets.
    let mut e = ed("1. a\n2. b");
    opt("org-list-two-spaces-after-bullet-regexp", "[0-9]".into());
    run(&mut e, "org-list-repair").unwrap();
    assert_eq!(text(&e), "1.  a\n2.  b");
}

fn count(t: &str, all: bool) -> String {
    let mut e = ed(t);
    run_arg(
        &mut e,
        "org-update-checkbox-count",
        if all { Prefix::U(1) } else { Prefix::None },
    )
    .unwrap();
    text(&e)
}

#[test]
fn update_checkbox_count_spec() {
    assert!(count("* [/]\n- [ ] item", false).contains("[0/1]"));
    assert!(count("* [/]\n- [X] item", false).contains("[1/1]"));
    assert!(count("* [%]\n- [X] item", false).contains("[100%]"));
    assert!(count("- [/]\n  - [ ] item", false).contains("[0/1]"));
    assert!(count("- [/]\n  - [X] item", false).contains("[1/1]"));
    assert!(count("- [%]\n  - [X] item", false).contains("[100%]"));
    assert!(count("- [ ] item 1\n- [ ] item 2 [/]\n  - [X] sub 1", false).contains("[1/1]"));
    let mut e = ed("- [/]\n  - item\n    - [X] sub-item");
    opt("org-checkbox-children-only-statistics", false.into());
    run(&mut e, "org-update-checkbox-count").unwrap();
    assert!(text(&e).starts_with("- [1/1]"));
    let mut e = ed("- [/]\n  - item\n    - [X] sub-item");
    opt("org-checkbox-hierarchical-statistics", false.into());
    run(&mut e, "org-update-checkbox-count").unwrap();
    assert!(text(&e).starts_with("- [1/1]"), "obsolete alias");
    assert!(count("\n<point>* H\n:PROPERTIES:\n:COOKIE_DATA: recursive\n:END:\n- [/]\n  - item\n    - [X] sub-item", false).contains("[1/1]"));
    assert!(count("- [/]\n  - item\n    - [ ] sub-item", false).contains("[0/0]"));
    assert_eq!(
        count("* [/]\n- [X] item\n* [/]\n- [X] item", true)
            .matches("[1/1]")
            .count(),
        2
    );
    assert_eq!(
        count("* [/]\n- [X] item\n* [/]\n- [X] item", false)
            .matches("[1/1]")
            .count(),
        1
    );
    let mut e =
        ed("\n- [/]\n  - [X] item1\n    :DRAWER:\n    - [X] item\n    :END:\n  - [X] item2");
    opt("org-checkbox-children-only-statistics", false.into());
    run(&mut e, "org-update-checkbox-count").unwrap();
    assert!(text(&e).contains("[2/2]"));
    let list =
        |first: &str, n: usize, rest: &str| format!("- [%]\n{first}{}", vec![rest; n].join("\n"));
    assert!(count(&list("", 101, "  - [ ]"), false).contains("[0%]"));
    assert!(count(&list("  - [X]\n", 100, "  - [ ]"), false).contains("[1%]"));
    assert!(count(&list("  - [ ]\n", 200, "  - [X]"), false).contains("[99%]"));
    // TODO cookies are left to the TODO statistics.
    assert!(
        count(
            "* H [/]\n:PROPERTIES:\n:COOKIE_DATA: todo\n:END:\n- [X] a",
            false
        )
        .contains("H [/]")
    );
    // Cookies in verbatim contexts are not statistics cookies.
    assert!(count("* H\n#+begin_src\n[/]\n#+end_src\n- [X] a", false).contains("\n[/]\n"));
    assert_eq!(percent_cookie(1, 3), "[33%]");
}

fn radio(t: &str) -> bool {
    let mut e = ed(t);
    let mut b = load(&e);
    let r = at_radio_list(&mut b);
    store(&mut e, b);
    r
}

#[test]
fn at_radio_list_p() {
    assert!(radio("#+attr_org: :radio t\n<point>- foo"));
    assert!(radio("#+attr_org: :radio t\n- foo\n<point>- bar"));
    assert!(radio("#+ATTR_ORG: :radio t\n<point>- foo"));
    assert!(radio("#+attr_org: :radio bar\n<point>- foo"));
    assert!(!radio("#+attr_org: :radio nil\n<point>- foo"));
    assert!(!radio("<point>- foo"));
    assert!(!radio("#+attr_org: :radio t\n- foo\n  <point>bar"));
    assert!(!radio(
        "#+attr_org: :radio t\n#+begin_example\n<point>- foo\n#+end_example"
    ));
    // Toggling a radio list checks one box only.
    let mut e = ed("#+attr_org: :radio t\n- [X] a\n- [ ] b\n- c");
    goto_line(&mut e, 3);
    run(&mut e, "org-toggle-checkbox").unwrap();
    assert_eq!(text(&e), "#+attr_org: :radio t\n- [ ] a\n- [X] b\n- [ ] c");
}

fn toggle_item_region(t: &str, arg: bool, to: Option<&str>) -> String {
    let mut e = ed(t);
    match to {
        Some(s) => {
            let lo = e.cur.line;
            search(&mut e, s);
            e.org.as_mut().unwrap().region = Some((lo, e.cur.line));
            // Emacs' region ends at point: the end of `s` here.
        }
        None => region_to_end(&mut e),
    }
    run_arg(
        &mut e,
        "org-toggle-item",
        if arg { Prefix::U(1) } else { Prefix::None },
    )
    .unwrap();
    text(&e)
}

#[test]
fn toggle_item_spec() {
    assert_eq!(after("line", "org-toggle-item"), "- line");
    assert_eq!(after("- line", "org-toggle-item"), "line");
    assert_eq!(after("* line", "org-toggle-item"), "- line");
    assert_eq!(after("* DONE line", "org-toggle-item"), "- [X] line");
    assert_eq!(after("* TODO line", "org-toggle-item"), "- [ ] line");
    assert_eq!(
        after("* H\nSCHEDULED: <2012-03-29 Thu>", "org-toggle-item"),
        "- H\n"
    );
    assert_eq!(
        after("* H\n:PROPERTIES:\n:A: 1\n:END:", "org-toggle-item"),
        "- H\n"
    );
    assert_eq!(
        after(
            "* H\n:PROPERTIES:\n:A: 1\n:END:\n\n\nText",
            "org-toggle-item"
        ),
        "- H\nText"
    );
    assert_eq!(
        after("<point> \n* H :tag:", "org-toggle-item"),
        " \n* H :tag:"
    );
    assert_eq!(
        toggle_item_region("* H1\n** H2", false, None),
        "- H1\n  - H2"
    );
    assert_eq!(
        toggle_item_region("* TODO H1\n** TODO H2", false, None),
        "- [ ] H1\n  - [ ] H2"
    );
    assert_eq!(
        toggle_item_region("* H1\nText", false, None),
        "- H1\n  Text"
    );
    assert_eq!(toggle_item_region("- 1\n  - 2", false, None), "1\n  2");
    assert_eq!(toggle_item_region("- 1\n2", false, None), "1\n2");
    assert_eq!(
        toggle_item_region("line 1\nline 2", false, None),
        "- line 1\n- line 2"
    );
    assert_eq!(
        toggle_item_region("line 1\n- line 2", false, None),
        "- line 1\n- line 2"
    );
    assert_eq!(
        toggle_item_region(
            "* Main headline\n** Headline 1\nbbbbbbbb [fn:1]\n\n[fn:1] cccccccccccccccc\n* Headline 2",
            true,
            None
        ),
        "- Main headline\n  - Headline 1\n    bbbbbbbb [fn:1]\n\n- Headline 2\n[fn:1] cccccccccccccccc\n\n\n"
    );
    assert_eq!(
        toggle_item_region(
            "* Head 1\n[fn:1] cccccccccccccccc\n* Head 2\n\n\nParagraph outside footnote definitions.",
            true,
            Some("Head 2")
        ),
        "- Head 1\n- Head 2\n[fn:1] cccccccccccccccc\n\n\nParagraph outside footnote definitions."
    );
    assert_eq!(
        toggle_item_region(
            "Line 1\nLine 2\n[fn:1] definition\n\n\n- next item",
            true,
            Some("definition")
        ),
        "- Line 1\n  Line 2\n- next item\n[fn:1] definition\n\n\n"
    );
    assert_eq!(
        toggle_item_region("line 1\nline 2", true, None),
        "- line 1\n  line 2"
    );
}

fn sorted(t: &str, with_case: bool, kind: char) -> String {
    let mut e = ed(t);
    with_buf(&mut e, |b| sort_list(b, with_case, kind, None, None)).unwrap();
    text(&e)
}

#[test]
fn sort() {
    assert_eq!(
        sorted("- def\n- XYZ\n- abc\n", false, 'a'),
        "- abc\n- def\n- XYZ\n"
    );
    assert_eq!(
        sorted("- def\n- XYZ\n- abc\n", false, 'A'),
        "- XYZ\n- def\n- abc\n"
    );
    assert_eq!(sorted("- b\n- C\n- a\n", true, 'a'), "- C\n- a\n- b\n");
    assert_eq!(sorted("- b\n- C\n- a\n", true, 'A'), "- b\n- a\n- C\n");
    assert_eq!(sorted("- 10\n- 1\n- 2\n", false, 'n'), "- 1\n- 2\n- 10\n");
    assert_eq!(sorted("- 10\n- 1\n- 2\n", false, 'N'), "- 10\n- 2\n- 1\n");
    assert_eq!(
        sorted("- [X] abc\n- [ ] xyz\n- [ ] def\n", false, 'x'),
        "- [ ] xyz\n- [ ] def\n- [X] abc\n"
    );
    assert_eq!(
        sorted("- [X] abc\n- [ ] xyz\n- [ ] def\n", false, 'X'),
        "- [X] abc\n- [ ] xyz\n- [ ] def\n"
    );
    let ts = "- <2018-05-09 Wed>\n- <2017-05-09 Tue>\n- <2017-05-08 Mon>\n";
    assert_eq!(
        sorted(ts, false, 't'),
        "- <2017-05-08 Mon>\n- <2017-05-09 Tue>\n- <2018-05-09 Wed>\n"
    );
    assert_eq!(sorted(ts, false, 'T'), ts);
    let len = |s: &str| Key::Num(s.lines().next().unwrap_or("").len() as f64);
    let lt = |a: &Key, b: &Key| a < b;
    let mut e = ed("- ccc\n- b\n- aa\n");
    with_buf(&mut e, |b| sort_list(b, false, 'f', Some(&len), Some(&lt))).unwrap();
    assert_eq!(text(&e), "- b\n- aa\n- ccc\n");
    let mut e = ed("- ccc\n- b\n- aa\n");
    with_buf(&mut e, |b| sort_list(b, false, 'F', Some(&len), Some(&lt))).unwrap();
    assert_eq!(text(&e), "- ccc\n- aa\n- b\n");
    // Interactively: a key menu, sorting one sub-list, renumbering.
    let mut e = ed("1. c\n2. a\n   - z\n   - y\n3. b\n");
    crate::org::run(&mut e, "org-sort-list", Prefix::None);
    e.handle_key(crate::key::Key::ch('a'));
    assert_eq!(text(&e), "1. a\n   - z\n   - y\n2. b\n3. c\n");
}

fn lisp(t: &str) -> export::List {
    let mut e = ed(t);
    let mut b = load(&e);
    let l = export::to_lisp(&mut b, false);
    store(&mut e, b);
    l
}

fn generic(t: &str, p: Params) -> String {
    export::to_generic(&lisp(t), &p)
}

#[test]
fn to_generic() {
    let p = |f: fn(&mut Params)| {
        let mut p = Params::default();
        f(&mut p);
        p
    };
    assert_eq!(
        generic("- a", p(|p| p.ustart = Gen::s("begin"))),
        "begin\na"
    );
    assert_ne!(
        generic("1. a", p(|p| p.ustart = Gen::s("begin"))),
        "begin\na"
    );
    assert_eq!(generic("- a", p(|p| p.uend = Gen::s("end"))), "a\nend");
    assert_ne!(generic("1. a", p(|p| p.uend = Gen::s("end"))), "a\nend");
    let lv = |p: &mut Params| {
        p.ustart = Gen::f(|a| Some(format!("begin l{}", a.depth)));
        p.uend = Gen::f(|a| Some(format!("end l{}", a.depth)));
    };
    assert_eq!(
        generic("- a\n  - b", p(lv)),
        "begin l1\na\nbegin l2\nb\nend l2\nend l1"
    );
    assert_eq!(
        generic("1. a", p(|p| p.ostart = Gen::s("begin"))),
        "begin\na"
    );
    assert_ne!(
        generic("- a", p(|p| p.ostart = Gen::s("begin"))),
        "begin\na"
    );
    assert_eq!(generic("1. a", p(|p| p.oend = Gen::s("end"))), "a\nend");
    let lo = |p: &mut Params| {
        p.ostart = Gen::f(|a| Some(format!("begin l{}", a.depth)));
        p.oend = Gen::f(|a| Some(format!("end l{}", a.depth)));
    };
    assert_eq!(
        generic("1. a\n  1. b", p(lo)),
        "begin l1\na\nbegin l2\nb\nend l2\nend l1"
    );
    assert_eq!(
        generic("- tag :: a", p(|p| p.dstart = Gen::s("begin"))),
        "begin\ntaga"
    );
    assert_ne!(
        generic("- a", p(|p| p.dstart = Gen::s("begin"))),
        "begin\na"
    );
    assert_eq!(
        generic("- tag :: a", p(|p| p.dend = Gen::s("end"))),
        "taga\nend"
    );
    let ld = |p: &mut Params| {
        p.dstart = Gen::f(|a| Some(format!("begin l{}", a.depth)));
        p.dend = Gen::f(|a| Some(format!("end l{}", a.depth)));
    };
    assert_eq!(
        generic("- tag1 :: a\n  - tag2 :: b", p(ld)),
        "begin l1\ntag1a\nbegin l2\ntag2b\nend l2\nend l1"
    );
    assert_eq!(
        generic(
            "- tag :: a",
            p(|p| {
                p.dtstart = Some(">".into());
                p.dtend = Some("<".into());
            })
        ),
        ">tag<a"
    );
    assert_eq!(
        generic(
            "- tag :: a",
            p(|p| {
                p.ddstart = Some(">".into());
                p.ddend = Some("<".into());
            })
        ),
        "tag>a<"
    );
    assert_eq!(generic("- a", p(|p| p.istart = Gen::s("start"))), "starta");
    assert_eq!(
        generic(
            "- a\n  - b",
            p(|p| p.istart = Gen::f(|a| Some(format!("level{} ", a.depth))))
        ),
        "level1 a\nlevel2 b"
    );
    assert_eq!(
        generic(
            "- a\n  - b",
            p(|p| p.iend = Gen::f(|a| Some(format!("level{}", a.depth))))
        ),
        "a\nblevel2level1"
    );
    assert_eq!(
        generic("1. [@3] a", p(|p| p.icount = Gen::s("count"))),
        "counta"
    );
    assert_ne!(generic("1. a", p(|p| p.icount = Gen::s("count"))), "counta");
    assert_eq!(
        generic(
            "1. [@3] a",
            p(|p| {
                p.icount = Gen::s("count");
                p.istart = Gen::s("start");
            })
        ),
        "counta"
    );
    assert_eq!(
        generic(
            "1. [@3] a",
            p(|p| p.icount = Gen::f(|a| Some(format!(
                "level:{}, counter:{} ",
                a.depth,
                a.counter.unwrap()
            ))))
        ),
        "level:1, counter:3 a"
    );
    assert_eq!(
        generic("- a\n- b", p(|p| p.isep = Gen::s("--"))),
        "a\n--\nb"
    );
    assert_ne!(
        generic("- a\n  - b", p(|p| p.isep = Gen::s("--"))),
        "a\n--\nb"
    );
    assert_eq!(
        generic(
            "- a\n- b",
            p(|p| p.isep = Gen::f(|a| Some(format!("- {} -", a.depth))))
        ),
        "a\n- 1 -\nb"
    );
    assert_eq!(
        generic(
            "1. [@3] a",
            p(|p| p.ifmt = Gen::f(|a| Some(format!(">> {} <<", a.contents))))
        ),
        ">> a <<"
    );
    assert_eq!(generic("- [X] a", p(|p| p.cbon = Some("!".into()))), "!a");
    assert_ne!(
        generic(
            "- [X] a",
            p(|p| {
                p.cboff = Some("!".into());
                p.cbtrans = Some("!".into());
            })
        ),
        "!a"
    );
    assert_eq!(generic("- [ ] a", p(|p| p.cboff = Some("!".into()))), "!a");
    assert_eq!(
        generic("- [-] a", p(|p| p.cbtrans = Some("!".into()))),
        "!a"
    );
    assert_eq!(
        generic(
            "- a",
            p(|p| {
                p.ustart = Gen::s("begin");
                p.uend = Gen::s("end");
                p.splice = true;
            })
        ),
        "a"
    );
    assert_eq!(generic("-", Params::default()), "");
}

#[test]
fn to_backends_and_org() {
    assert_eq!(
        export::to_html(&lisp("- a"), &Params::default()),
        "<ul class=\"org-ul\">\n<li>a</li>\n</ul>"
    );
    assert_eq!(
        export::to_latex(&lisp("- a"), &Params::default()),
        "\\begin{itemize}\n\\item a\n\\end{itemize}"
    );
    assert_eq!(
        export::to_texinfo(&lisp("- a"), &Params::default()),
        "@itemize\n@item\na\n@end itemize"
    );
    let o = |t: &str| export::to_org(&lisp(t), &Params::default());
    assert_eq!(o("- a"), "- a");
    assert_eq!(o("1. a"), "1. a");
    assert_eq!(o("- a :: b"), "- a :: b");
    assert_eq!(o("- a\n  - b"), "- a\n  - b");
    assert_eq!(o("- a\n  b"), "- a\n  b");
    assert_eq!(o("- a\n  - b\n  c"), "- a\n  - b\n  c");
    // Checked against upstream Emacs.
    assert_eq!(
        o("- a\n  - b\n- [X] c :: d\n- [@3] e\n\n  more\n  - x\n\n  tail"),
        "- a\n  - b\n- [X] c :: d\n- [@3] e\n  \n  more\n  - x\n  \n  tail"
    );
    let l = lisp("- a\n  - b\n- c");
    assert_eq!(
        export::to_subtree(&l, 1, false, false, &Params::default()),
        "* a\n** b\n* c"
    );
    let l = lisp("- a\n  - b\n- [X] c :: d\n- [@3] e\n\n  more");
    assert_eq!(
        export::to_subtree(&l, 1, true, false, &Params::default()),
        "* a\n** b\n* DONE  c d\n* e\n\nmore"
    );
    assert_eq!(
        export::to_subtree(&lisp("- a\n  - b"), 2, false, true, &Params::default()),
        "*** a\n***** b"
    );
}

#[test]
fn make_subtree() {
    let mut e = ed("* H\n- a\n  - [ ] b\n- c\nafter");
    goto_line(&mut e, 3);
    run(&mut e, "org-list-make-subtree").unwrap();
    assert_eq!(text(&e), "* H\n** a\n*** TODO b\n** c\nafter");
}

fn toggle(t: &str, arg: Prefix) -> Result<String, String> {
    let mut e = ed(t);
    run_arg(&mut e, "org-toggle-checkbox", arg).map(|()| text(&e))
}

#[test]
fn toggle_checkbox_spec() {
    assert_eq!(toggle("- [ ] a", Prefix::None).unwrap(), "- [X] a");
    assert_eq!(toggle("- [X] a", Prefix::None).unwrap(), "- [ ] a");
    assert_eq!(toggle("- a", Prefix::None).unwrap(), "- a");
    assert_eq!(toggle("- a", Prefix::U(1)).unwrap(), "- [ ] a");
    assert_eq!(toggle("- [X] a", Prefix::U(1)).unwrap(), "- a");
    assert_eq!(toggle("- [X] a", Prefix::U(2)).unwrap(), "- [-] a");
    assert_eq!(toggle("1. [@3] a", Prefix::U(1)).unwrap(), "1. [@3][ ] a");
    assert_eq!(
        toggle("- [ ] a\n  - [ ] b\n  - [X] c", Prefix::None).unwrap(),
        "- [-] a\n  - [ ] b\n  - [X] c"
    );
    let mut e = ed("- [ ] a\n  - [<point> ] b\n  - [X] c");
    run(&mut e, "org-toggle-checkbox").unwrap();
    assert_eq!(text(&e), "- [X] a\n  - [X] b\n  - [X] c");
    // Parent statistics follow.
    assert_eq!(
        toggle("* H [/]\n- [ ] a\n- [ ] b", Prefix::None).unwrap(),
        "* H [2/2]\n- [X] a\n- [X] b"
    );
    let mut e = ed("* T [%]\n- [ ] a\n- [ ] b");
    goto_line(&mut e, 2);
    run(&mut e, "org-toggle-checkbox").unwrap();
    assert_eq!(text(&e), "* T [50%]\n- [X] a\n- [ ] b");
    assert!(toggle("text", Prefix::None).is_err());
    assert_eq!(
        toggle("* H\n:PROPERTIES:\n:A: 1\n:END:\ntext", Prefix::None),
        Err("No item in subtree".into())
    );
    // A region.
    let mut e = ed("- [ ] a\n- [X] b\n- c\n- [ ] d");
    e.org.as_mut().unwrap().region = Some((0, 2));
    run(&mut e, "org-toggle-checkbox").unwrap();
    assert_eq!(text(&e), "- [X] a\n- [X] b\n- c\n- [ ] d");
    // ORDERED: a box after an unchecked one cannot be checked.
    let t = "* H\n:PROPERTIES:\n:ORDERED: t\n:END:\n- [ ] a\n- [ ] b";
    let mut e = ed(t);
    goto_line(&mut e, 6);
    assert_eq!(
        run(&mut e, "org-toggle-checkbox"),
        Err("Checkbox blocked because of unchecked box at line 5".into())
    );
}

#[test]
fn ctrl_c_ctrl_c_on_items() {
    let cc = |t: &str, arg: Prefix| {
        let mut e = ed(t);
        let r = run_arg(&mut e, "org-list-ctrl-c-ctrl-c", arg);
        (r, text(&e), e.msg.clone().map(|m| m.0))
    };
    assert_eq!(
        cc("- [ ] a\n- [ ] <point>b", Prefix::None).1,
        "- [ ] a\n- [X] b"
    );
    assert_eq!(cc("- a\n- <point>b", Prefix::U(1)).1, "- a\n- [ ] b");
    assert_eq!(cc("- a\n- [X] <point>b", Prefix::U(2)).1, "- a\n- [-] b");
    // Without a box, the list is repaired.
    assert_eq!(cc("1. a\n1. <point>b", Prefix::None).1, "1. a\n2. b");
    // At the beginning of the first item: the plain list.
    assert_eq!(cc("- [ ] a\n- [ ] b", Prefix::None).1, "- [X] a\n- [ ] b");
    assert_eq!(cc("- a\n- b", Prefix::U(1)).1, "- [ ] a\n- [ ] b");
    assert_eq!(cc("- a\n+ b", Prefix::None).1, "- a\n- b");
    assert_eq!(
        cc("- a\n- b", Prefix::None).2.as_deref(),
        Some("Cannot update this checkbox")
    );
    let (r, ..) = cc("- [X] a\n  - [X] <point>b", Prefix::None);
    assert!(r.is_ok());
    let (r, ..) = cc("- [X] <point>a\n  - [X] b", Prefix::None);
    assert_eq!(
        r,
        Err("Cannot toggle this checkbox: all subitems checked".into())
    );
}

#[test]
fn reset_checkbox_state() {
    let mut e = ed("* H [1/2]\n- [X] a\n- [ ] b\n  - [X] c\n* Other\n- [X] d");
    goto_line(&mut e, 2);
    run(&mut e, "org-reset-checkbox-state-subtree").unwrap();
    assert_eq!(
        text(&e),
        "* H [0/2]\n- [ ] a\n- [ ] b\n  - [ ] c\n* Other\n- [X] d"
    );
    assert!(after_err("- [X] a", "org-reset-checkbox-state-subtree"));
}

#[test]
fn tab_cycles_item_visibility() {
    let t = "- a\n  - b\n    text\n  - c\n- d";
    let mut e = ed(t);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(shown(&e), vec!["- a...", "- d"]);
    assert_eq!(e.msg.as_ref().unwrap().0, "FOLDED");
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(shown(&e), vec!["- a", "  - b...", "  - c", "- d"]);
    assert_eq!(e.msg.as_ref().unwrap().0, "CHILDREN");
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(shown(&e).len(), 5);
    assert_eq!(e.msg.as_ref().unwrap().0, "SUBTREE");
    let mut e = ed("- a\n  text\n- b");
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(e.msg.as_ref().unwrap().0, "SUBTREE (NO CHILDREN)");
    let mut e = ed("- a\n- b");
    crate::org::run(&mut e, "org-cycle", Prefix::None);
    assert_eq!(e.msg.as_ref().unwrap().0, "EMPTY ENTRY");
}

#[test]
fn structure_alists() {
    let t = "- [X] first item\n  1. sub-item 1\n  5. [@5] sub-item 2\n  some other text belonging to first item\n- last item\n  + tag :: description\n";
    let mut b = Buf::new(t, 0);
    let st = list_struct(&mut b);
    let rows: Vec<_> =
        st.0.iter()
            .map(|i| {
                (
                    i.pos,
                    i.ind,
                    i.bullet.as_str(),
                    i.counter.as_deref(),
                    i.checkbox.as_deref(),
                    i.tag.as_deref(),
                    i.end,
                )
            })
            .collect();
    assert_eq!(
        rows,
        vec![
            (0, 0, "- ", None, Some("[X]"), None, 96),
            (17, 2, "1. ", None, None, None, 33),
            (33, 2, "5. ", Some("5"), None, None, 54),
            (96, 0, "- ", None, None, None, 131),
            (108, 2, "+ ", None, None, Some("tag"), 131),
        ]
    );
    assert_eq!(
        st.parents(),
        vec![
            (0, None),
            (17, Some(0)),
            (33, Some(0)),
            (96, None),
            (108, Some(96))
        ]
    );
    assert_eq!(
        st.prevs(),
        vec![
            (0, None),
            (17, None),
            (33, Some(17)),
            (96, Some(0)),
            (108, None)
        ]
    );
    assert_eq!(st.item_number(33, &st.prevs(), &st.parents()), vec![1, 5]);
    assert_eq!(st.list_type(108, &st.prevs()), ListType::Descriptive);
    let mut b = Buf::new("- ab :: b", 0);
    assert!(at_item_description(&mut b));
    let mut b = Buf::new("1. [@3] 10:00:00 :: x", 0);
    assert_eq!(at_item_counter(&mut b).as_deref(), Some("3"));
    assert!(at_item_timer(&mut b));
    let mut b = Buf::new("  - x", 2);
    assert!(at_item_bullet(&mut b));
    b.goto(4);
    assert!(!at_item_bullet(&mut b));
    assert_eq!(bullet_string("1."), "1. ");
    assert_eq!(inc_bullet("9) "), "10) ");
    assert_eq!(inc_bullet("b. "), "c. ");
    // org-apply-on-list counts the items of the sub-list; body columns.
    let mut b = Buf::new("- a\n  - x\n- b\n- c", 0);
    assert_eq!(apply_on_list(&mut b, 0, |_, n| n + 1), 3);
    assert_eq!(b.pt, 0);
    assert_eq!(item_body_column(&b, 4), 4);
    assert_eq!(item_body_column(&Buf::new("10. x", 0), 0), 4);
}

#[test]
fn visual_line_range_is_the_region() {
    options::set(toml::Table::new());
    let e = org("- a\n- b\n- c", "jVj:org-indent-item<Enter>");
    assert_eq!(text(&e), "- a\n  - b\n  - c");
    assert_eq!(crate::org::region(&e), None);
    let e = org("- [ ] a\n- [ ] b\n- [ ] c", "Vj:org-toggle-checkbox<Enter>");
    assert_eq!(text(&e), "- [X] a\n- [X] b\n- [ ] c");
    // Org keys in Visual-line mode act on the selected lines.
    let e = org("- [ ] a\n- [ ] b\n- [ ] c", "jVj<C-c><C-x><C-b>");
    assert_eq!(text(&e), "- [ ] a\n- [X] b\n- [X] c");
    assert_eq!(e.mode, crate::editor::Mode::Normal);
}
