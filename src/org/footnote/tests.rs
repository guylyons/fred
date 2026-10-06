//! Ported from testing/lisp/test-org-footnote.el.

use super::*;
use crate::org::options;
use toml::Value;

/// An Org buffer holding `text` with point at `<point>` (the
/// org-test-with-temp-text convention), Emacs-style point (may be at eol).
pub(crate) fn at(text: &str) -> Editor {
    let pos = text.find("<point>").unwrap_or(0);
    let t = text.replace("<point>", "");
    let mut e = crate::org::tests::org(&t, "");
    let line = t[..pos].matches('\n').count();
    let bol = t[..pos].rfind('\n').map_or(0, |i| i + 1);
    e.cur.line = line;
    e.cur.byte = pos - bol;
    e
}

/// buffer-string.
pub(crate) fn text(e: &Editor) -> String {
    let mut s = e.buf.text();
    if e.buf.final_newline {
        s.push('\n');
    }
    s
}

/// Point as an offset into [`text`].
fn point(e: &Editor) -> usize {
    EBuf::load_emacs(e).pt
}

struct Opts(Vec<&'static str>);

impl Drop for Opts {
    fn drop(&mut self) {
        for n in &self.0 {
            options::remove(n);
        }
    }
}

fn opts(v: &[(&'static str, Value)]) -> Opts {
    for (k, x) in v {
        options::put(k, x.clone());
    }
    Opts(v.iter().map(|(k, _)| *k).collect())
}

fn sym(s: &str) -> Value {
    Value::String(format!("'{s}"))
}

fn nil() -> Value {
    Value::Boolean(false)
}

fn new_in(t: &str) -> Result<String, String> {
    let mut e = at(t);
    new(&mut e)?;
    Ok(text(&e))
}

#[test]
fn new_anon_and_auto_labels() {
    let _o = opts(&[
        ("org-footnote-auto-label", sym("anonymous")),
        ("org-footnote-section", nil()),
    ]);
    assert_eq!(new_in("Test<point>").unwrap(), "Test[fn::]");
    drop(_o);
    let _o = opts(&[("org-footnote-section", nil())]);
    let t = new_in("Test<point>").unwrap();
    assert!(
        crate::org_re!(r"^Test\[fn:1\]\n+\[fn:1\]").is_match(&t),
        "{t:?}"
    );
    options::put("org-footnote-auto-label", sym("random"));
    let t = new_in("Test<point>").unwrap();
    let c = crate::org_re!(r"^Test\[fn:(.+?)\]\n+\[fn:(.+?)\]")
        .captures(&t)
        .unwrap();
    assert_eq!(c[1], c[2]);
    options::remove("org-footnote-auto-label");
}

#[test]
fn new_contexts() {
    // Error at beginning of line, but not for inline or anonymous notes.
    assert!(new_in("<point>Test").is_err());
    {
        let _o = opts(&[("org-footnote-define-inline", Value::Boolean(true))]);
        assert_eq!(new_in("<point>Test").unwrap(), "[fn:1:]Test");
    }
    {
        let _o = opts(&[("org-footnote-auto-label", sym("anonymous"))]);
        assert!(new_in("<point>Test").is_ok());
    }
    // Error at keywords.
    assert!(new_in("#+TIT<point>LE: value").is_err());
    assert!(new_in("#+CAPTION: <point>\nParagraph").is_err());
    // Blank lines at the beginning of the document.
    assert!(new_in(" <point>").unwrap().contains(" [fn:1]"));
    // Headlines: on the heading text or the blank lines below.
    assert!(new_in("* H<point>").is_ok());
    assert!(new_in("* H\n <point>\nParagraph").is_ok());
    assert!(new_in("*<point> H").is_err());
    assert!(new_in("* H <point>:tag:").is_err());
    // Recursive objects, but not links.
    assert!(new_in(" *bold<point>*").unwrap().contains(" *bold[fn:1]*"));
    assert!(new_in(" [[https://orgmode.org][Org mode<point>]]").is_err());
    // After an element or an object.
    assert!(
        new_in("#+BEGIN_EXAMPLE\nA\n#+END_EXAMPLE\n <point>")
            .unwrap()
            .contains(" [fn:1]")
    );
    assert!(new_in(" *bold*<point>").unwrap().contains(" *bold*[fn:1]"));
    // At the start of a footnote definition.
    assert!(new_in("[fn:1]<point>").unwrap().contains("[fn:1][fn:2]"));
    assert!(new_in("[fn:1] <point>").unwrap().contains("[fn:1] [fn:2]"));
    assert!(
        new_in("[fn:1]<point> \nParagraph")
            .unwrap()
            .contains("[fn:1][fn:2]")
    );
    assert!(new_in("[fn:<point>1]").is_err());
    assert!(new_in("<point>[fn:1]").is_err());
    // Table cells.
    assert!(new_in("| <point> |").unwrap().contains(" [fn:1]"));
    assert!(new_in("|<point> |").unwrap().contains("|[fn:1]"));
    assert!(new_in("| <point>|").unwrap().contains(" [fn:1]"));
    assert!(new_in("| contents   <point>|").unwrap().contains(" [fn:1]"));
}

#[test]
fn new_moves_to_the_definition_and_adjusts() {
    let mut e = at("Text<point>");
    new(&mut e).unwrap();
    let t = text(&e);
    let p = point(&e);
    let bol = t[..p].rfind('\n').map_or(0, |i| i + 1);
    assert_eq!(&t[bol..p], "[fn:1]");
    // org-footnote-auto-adjust.
    let _o = opts(&[
        ("org-footnote-auto-adjust", Value::Boolean(true)),
        ("org-footnote-section", nil()),
    ]);
    let mut e = at("Text[fn:1]Text<point>Text[fn:2]\n\n[fn:1] 1\n\n[fn:2] 2");
    new(&mut e).unwrap();
    let t = text(&e);
    assert!(t.ends_with("[fn:1] 1\n\n[fn:2] \n\n[fn:3] 2\n"), "{t:?}");
    assert!(t.starts_with("Text[fn:1]Text[fn:2]Text[fn:3]"), "{t:?}");
}

#[test]
fn new_keeps_local_variables_and_uses_the_section() {
    {
        let _o = opts(&[("org-footnote-section", nil())]);
        assert_eq!(
            new_in("Paragraph<point>\n# Local Variables:\n# foo: t\n# End:").unwrap(),
            "Paragraph[fn:1]\n\n[fn:1] \n# Local Variables:\n# foo: t\n# End:"
        );
    }
    assert_eq!(
        new_in("Paragraph<point>\n# Local Variables:\n# foo: t\n# End:").unwrap(),
        "Paragraph[fn:1]\n\n* Footnotes\n\n[fn:1] \n# Local Variables:\n# foo: t\n# End:"
    );
    assert_eq!(
        new_in("Para<point>\n* Footnotes\n:properties:\n:custom_id: id\n:end:")
            .unwrap()
            .trim(),
        "Para[fn:1]\n* Footnotes\n:properties:\n:custom_id: id\n:end:\n\n[fn:1]"
    );
}

fn deleted(t: &str, label: Option<&str>) -> Result<String, String> {
    let mut e = at(t);
    delete_cmd(&mut e, label.map(str::to_owned))?;
    Ok(text(&e))
}

#[test]
fn delete() {
    assert_eq!(
        deleted("Paragraph<point>[fn:1]\n\n[fn:1] Definition", None)
            .unwrap()
            .trim(),
        "Paragraph"
    );
    assert_eq!(
        deleted(
            "Paragraph<point>[fn:1] and another[fn:1]\n\n[fn:1] def\n\n[fn:1] def",
            None
        )
        .unwrap()
        .trim(),
        "Paragraph and another"
    );
    assert_eq!(
        deleted("Para<point>[fn:label:def] and[fn:label]", None)
            .unwrap()
            .trim(),
        "Para and"
    );
    let _o = opts(&[("org-footnote-section", nil())]);
    assert_eq!(
        deleted("Para<point>[fn::def]", None).unwrap().trim(),
        "Para"
    );
    assert_eq!(
        deleted(
            "Paragraph[fn:1] and another[fn:2]\n\n[fn:1] def\n\n[fn:2] def2",
            Some("2")
        )
        .unwrap()
        .trim(),
        "Paragraph[fn:1] and another\n\n[fn:1] def"
    );
    assert!(deleted("Para[fn:1]\n\n[fn:1] Def", None).is_err());
    assert_eq!(
        deleted(
            "Para[fn:1]\n\n[fn:1] para1\n\npara2\n\n\nOutside footnote.",
            Some("1")
        )
        .unwrap()
        .trim(),
        "Para\n\n\nOutside footnote."
    );
    assert_eq!(
        deleted(
            "Text[fn:1]\n\n[fn:1] Definition.\n\n\nOther text.",
            Some("1")
        )
        .unwrap(),
        "Text\n\n\nOther text."
    );
    assert_eq!(
        deleted(
            "Paragraph[fn:1]\n[fn:1] Def 1\n# Local Variables:\n# foo: t\n# End:",
            Some("1")
        )
        .unwrap(),
        "Paragraph\n# Local Variables:\n# foo: t\n# End:"
    );
}

#[test]
fn goto_definition_and_previous_reference() {
    let mut e = at("No footnote definition");
    assert!(goto_definition(&mut e, "1", None).is_err());
    // Outside the narrowed part.
    let mut e = at("Some text<point>\n[fn:1] Definition.");
    fold::narrow(&mut e, 0, 0);
    assert_eq!(
        goto_definition(&mut e, "1", None).unwrap_err(),
        "Definition is outside narrowed part of buffer"
    );
    let mut e = at("Some text\n[fn:1] Definition.");
    goto_definition(&mut e, "1", None).unwrap();
    assert_eq!(&text(&e)[point(&e)..], " Definition.");
    let mut e = at("Some text[fn:label:definition]");
    goto_definition(&mut e, "label", None).unwrap();
    assert_eq!(&text(&e)[point(&e)..], "definition]");
    // C-c & comes back.
    crate::org::run(&mut e, "org-mark-ring-goto", Prefix::None);
    assert_eq!(e.cur.line, 0);

    let mut e = at("No footnote reference");
    assert_eq!(
        goto_previous_reference(&mut e, "1").unwrap_err(),
        "Cannot find reference of footnote \"1\""
    );
    let mut e = at("Some text<point>\nReference[fn:1].");
    fold::narrow(&mut e, 0, 0);
    assert!(goto_previous_reference(&mut e, "1").is_err());
    let mut e = at("First reference[fn:1]\nReference[fn:1].<point>");
    goto_previous_reference(&mut e, "1").unwrap();
    assert_eq!(e.cur.line, 1);
    let mut e = at("First reference[fn:1]\nReference[fn:1].");
    goto_previous_reference(&mut e, "1").unwrap();
    assert_eq!(e.cur.line, 0);
}

fn run_f(t: &str, f: impl FnOnce(&mut F)) -> String {
    let mut e = at(t);
    with(&mut e, f);
    text(&e)
}

#[test]
fn sort() {
    let _o = opts(&[("org-footnote-section", nil())]);
    assert_eq!(
        run_f("\nText[fn:1][fn:2]\n\n[fn:2] Def 2\n\n[fn:1] Def 1", |x| x
            .sort()),
        "\nText[fn:1][fn:2]\n\n[fn:1] Def 1\n\n[fn:2] Def 2\n"
    );
    assert_eq!(
        run_f(
            "\n* H1\nText[fn:1]\n* H2\nText[fn:2]\n\n[fn:1] Def 1\n\n[fn:2] Def 2\n",
            |x| x.sort()
        ),
        "\n* H1\nText[fn:1]\n\n[fn:1] Def 1\n* H2\nText[fn:2]\n\n[fn:2] Def 2\n"
    );
    options::put("org-footnote-section", Value::String("Footnotes".into()));
    assert_eq!(
        run_f("\nText[fn:1][fn:2]\n\n[fn:2] Def 2\n\n[fn:1] Def 1", |x| x
            .sort()),
        "\nText[fn:1][fn:2]\n\n* Footnotes\n\n[fn:1] Def 1\n\n[fn:2] Def 2\n"
    );
    assert_eq!(
        run_f(
            "\nText[fn:1]\n\n* Footnotes\n\n[fn:1] Def 1\n\n* Headline\n\n** Footnotes\n\n* Other headline",
            |x| x.sort()
        ),
        "\nText[fn:1]\n\n* Headline\n\n* Other headline\n\n* Footnotes\n\n[fn:1] Def 1\n"
    );
}

#[test]
fn renumber() {
    let r = |t| run_f(t, |x| x.renumber());
    assert_eq!(r("Test[fn:99]"), "Test[fn:1]");
    assert_eq!(r("Test[fn:99]\n\n[fn:99] 99"), "Test[fn:1]\n\n[fn:1] 99");
    assert_eq!(r("Test[fn:99:99]"), "Test[fn:1:99]");
    assert_eq!(
        r("Test[fn:label]\n\n[fn:label] Def"),
        "Test[fn:label]\n\n[fn:label] Def"
    );
    assert_eq!(
        r("Test[fn:1]\n[fn:1] 1\n[fn:99] 99"),
        "Test[fn:1]\n[fn:1] 1\n[fn:2] 99"
    );
    assert_eq!(
        r("Test[fn:4][fn:3:def][fn:2]"),
        "Test[fn:1][fn:2:def][fn:3]"
    );
    assert_eq!(r("Test[fn:4][fn::def][fn:2]"), "Test[fn:1][fn::def][fn:2]");
}

#[test]
fn normalize() {
    let _o = opts(&[("org-footnote-section", nil())]);
    let n = |t| run_f(t, |x| x.normalize());
    assert_eq!(
        n("Test[fn:label]\n[fn:label] def"),
        "Test[fn:1]\n\n[fn:1] def\n"
    );
    assert_eq!(n("Test[fn:label:def]"), "Test[fn:1]\n\n[fn:1] def\n");
    assert_eq!(n("Test[fn::def]"), "Test[fn:1]\n\n[fn:1] def\n");
    assert_eq!(
        n("Test[fn:2][fn:1]\n\n[fn:2] def2\n[fn:1] def"),
        "Test[fn:1][fn:2]\n\n[fn:1] def2\n\n[fn:2] def\n"
    );
    assert_eq!(
        n("Test[fn:2][fn::inline]\n[fn:2] def\n"),
        "Test[fn:1][fn:2]\n\n[fn:1] def\n\n[fn:2] inline\n"
    );
    assert_eq!(
        n("Test[fn:lab1][fn:lab2]\n[fn:lab1] def[fn::inline]\n[fn:lab2] last"),
        "Test[fn:1][fn:3]\n\n[fn:1] def[fn:2]\n\n[fn:2] inline\n\n[fn:3] last\n"
    );
    assert_eq!(
        n("Test[fn::def]\nNext\n[fn:unref] A"),
        "Test[fn:1]\nNext\n\n[fn:1] def\n\n[fn:2] A\n"
    );
    assert_eq!(
        n(
            "\nParagraph[fn:foo][fn:bar]\n\n[fn:bar] Def 2\n\n[fn:foo] Def 1\n\n# Local Variables:\n# foo: t\n# End:"
        ),
        "\nParagraph[fn:1][fn:2]\n\n[fn:1] Def 1\n\n[fn:2] Def 2\n\n# Local Variables:\n# foo: t\n# End:"
    );
}

#[test]
fn action_jumps_and_keys() {
    // C-c C-c on a reference goes to the definition, and back.
    let mut e = at("Text[fn:<point>1] more\n\n[fn:1] Def");
    crate::org::run(&mut e, "org-footnote-action", Prefix::None);
    assert_eq!(e.cur.line, 2, "{:?}", e.msg);
    crate::org::run(&mut e, "org-footnote-action", Prefix::None);
    assert_eq!((e.cur.line, e.cur.byte), (0, 4));
    // C-c C-x f creates a new footnote at point.
    let _o = opts(&[("org-footnote-section", nil())]);
    // (Evil point is before the character under the cursor.)
    let e = crate::org::tests::org("Text", "A<Esc><C-c><C-x>f");
    assert_eq!(text(&e), "Tex[fn:1]t\n\n[fn:1] \n", "{:?}", e.msg);
    // With C-u: the menu.
    let mut e = at("Test[fn:99]\n\n[fn:99] 99");
    crate::org::run(&mut e, "org-footnote-action", Prefix::U(1));
    e.handle_key(crate::key::Key::ch('r'));
    assert_eq!(text(&e), "Test[fn:1]\n\n[fn:1] 99");
    // Missing definition: offer to create it.
    let mut e = at("Text[fn:<point>7] more");
    crate::org::run(&mut e, "org-footnote-action", Prefix::None);
    e.handle_key(crate::key::Key::ch('y'));
    assert_eq!(text(&e), "Text[fn:7] more\n\n[fn:7] \n");
}

#[test]
fn prompt_for_label() {
    let _o = opts(&[
        ("org-footnote-auto-label", nil()),
        ("org-footnote-section", nil()),
    ]);
    let mut e = at("Text<point> end");
    new(&mut e).unwrap();
    for k in crate::key::parse_keys("note<Enter>") {
        e.handle_key(k);
    }
    assert_eq!(text(&e), "Text[fn:note] end\n\n[fn:note] \n");
    // An empty label makes an anonymous footnote.
    let mut e = at("Text<point> end");
    new(&mut e).unwrap();
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Enter));
    assert_eq!(text(&e), "Text[fn::] end");
}
