//! Tests of the export helpers ported so far (expectations from ox.el
//! and Emacs).

use super::*;
use crate::org::sexp::Sexp;

#[test]
fn option_items_like_ox_parse_option_keyword() {
    let items = env::option_items("H:2 num:nil toc:3 ':t ::nil \\n:t d:(not \"LOGBOOK\") ^:{}");
    let get = |k: &str| items.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    assert_eq!(get("H"), Some(Sexp::Int(2)));
    assert_eq!(get("num"), Some(Sexp::Nil));
    assert_eq!(get("toc"), Some(Sexp::Int(3)));
    assert_eq!(get("'"), Some(Sexp::T));
    assert_eq!(get(":"), Some(Sexp::Nil));
    assert_eq!(get("\\n"), Some(Sexp::T));
    assert_eq!(get("^"), Some(Sexp::Sym("{}".into())));
    assert_eq!(
        get("d").map(|v| v.to_string()),
        Some("(not \"LOGBOOK\")".to_owned())
    );
}

#[test]
fn roman_numbers_and_references() {
    assert_eq!(tools::number_to_roman(1994), "MCMXCIV");
    assert_eq!(tools::number_to_roman(0), "0");
    assert_eq!(tools::format_reference(0x1a), "org000001a");
    assert_eq!(
        tools::coderef_format("x", Some("see (x) here")),
        "see %s here"
    );
    assert_eq!(tools::coderef_format("x", None), "%s");
}

#[test]
fn search_cells_of_strings() {
    assert_eq!(
        tools::string_to_search_cell("*Some  title"),
        vec![Cell::Headline(vec!["SOME".into(), "TITLE".into()])]
    );
    assert_eq!(
        tools::string_to_search_cell("#id"),
        vec![Cell::CustomId("id".into())]
    );
    assert_eq!(tools::string_to_search_cell("A").len(), 2);
}

#[test]
fn translations_use_the_dictionary() {
    assert_eq!(
        quotes::translate("Table of Contents", ":ascii", "fr"),
        "Sommaire"
    );
    assert_eq!(quotes::translate("Footnotes", ":utf-8", "en"), "Footnotes");
    assert_eq!(
        quotes::translate("Unknown reference", ":ascii", "fr"),
        "Destination inconnue"
    );
}

#[test]
fn macro_seam_expands_builtin_and_buffer_macros() {
    let text = "#+TITLE: My doc\n#+MACRO: greet Hello, $1!\n{{{title}}} {{{greet(you)}}} {{{n}}} {{{n}}}\n";
    let out = macros::expand_macros(text, None, &[], &["TITLE"], None).unwrap();
    assert!(out.ends_with("My doc Hello, you! 1 2\n"), "{out}");
    let err = macros::expand_macros("{{{nope}}}\n", None, &[], &[], None).unwrap_err();
    assert_eq!(err, "Undefined Org macro: nope; aborting");
    // {{{results}}} waits for the Babel phase.
    let kept = macros::expand_macros("{{{results(x)}}}\n", None, &[], &[], None).unwrap();
    assert_eq!(kept, "{{{results(x)}}}\n");
}

#[test]
fn include_values_parse_like_upstream() {
    let p = include::parse_include_value(
        "\"file.org::*Head\" :lines \"3-5\" :minlevel 2",
        Some(std::path::Path::new("/d")),
        None,
    );
    assert_eq!(p.file.as_deref(), Some("/d/file.org"));
    assert_eq!(p.location.as_deref(), Some("*Head"));
    assert_eq!(p.lines.as_deref(), Some("3-5"));
    assert_eq!(p.minlevel, Some(2));
    let p = include::parse_include_value(
        "x.py src python :lines \"1-\"",
        Some(std::path::Path::new("/d")),
        Some(1),
    );
    assert!(p.literal);
    assert_eq!(p.block.as_deref(), Some("src"));
    assert!(p.args.is_some());
    assert_eq!(p.minlevel, None);
}
