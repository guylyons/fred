//! Org buffer tests through Fred's key handling.

use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::key::parse_keys;

/// An Org buffer with `text`, after `keys`.
pub(crate) fn org(text: &str, keys: &str) -> Editor {
    let mut e = Editor::new(Buffer::from_text(text));
    e.path = Some("t.org".into());
    super::attach(&mut e);
    for k in parse_keys(keys) {
        e.handle_key(k);
    }
    e
}

/// The visible lines, folded ones ending in "...".
pub(crate) fn shown(e: &Editor) -> Vec<String> {
    (0..e.line_count())
        .filter(|&l| !e.folds.hidden(l))
        .map(|l| {
            let mut s = e.buf.line(l);
            if e.folds.folded_after(l) {
                s.push_str("...");
            }
            s
        })
        .collect()
}

const DOC: &str = "#+TITLE: t\n* A\ntext a\n** A1\nbody\n** A2\n* B\nb\n";

#[test]
fn tab_cycles_subtree_folded_children_subtree() {
    let mut e = org(DOC, "j");
    assert_eq!(e.cur.line, 1);
    assert_eq!(shown(&e).len(), 8, "showeverything by default");
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Tab));
    assert_eq!(shown(&e), vec!["#+TITLE: t", "* A...", "* B", "b"]);
    assert_eq!(e.msg.as_ref().unwrap().0, "FOLDED");
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Tab));
    assert_eq!(shown(&e), vec!["#+TITLE: t", "* A", "text a", "** A1...", "** A2", "* B", "b"]);
    assert_eq!(e.msg.as_ref().unwrap().0, "CHILDREN");
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Tab));
    assert_eq!(shown(&e).len(), 8);
    assert_eq!(e.msg.as_ref().unwrap().0, "SUBTREE");
    // j/k step over folded lines.
    let mut e = org(DOC, "j<Tab>j");
    assert_eq!(e.cur.line, 6);
    e.handle_key(crate::key::Key::ch('k'));
    assert_eq!(e.cur.line, 1);
}

#[test]
fn shift_tab_cycles_globally_and_startup_options_apply() {
    let e = org(DOC, "<S-Tab>");
    assert_eq!(e.msg.as_ref().unwrap().0, "OVERVIEW");
    assert_eq!(shown(&e), vec!["#+TITLE: t", "* A...", "* B..."]);
    let e = org(DOC, "<S-Tab><S-Tab>");
    assert_eq!(shown(&e), vec!["#+TITLE: t", "* A...", "** A1...", "** A2", "* B..."]);
    let e = org(DOC, "<S-Tab><S-Tab><S-Tab>");
    assert_eq!(shown(&e).len(), 8);
    let e = org(&format!("#+STARTUP: content\n{DOC}"), "");
    assert_eq!(shown(&e), vec!["#+STARTUP: content", "#+TITLE: t", "* A...", "** A1...", "** A2", "* B..."]);
}

#[test]
fn search_into_a_fold_reveals_it_and_edits_keep_folds() {
    let mut e = org(DOC, "<S-Tab>/body<Enter>");
    assert_eq!(e.cur.line, 4);
    assert!(!e.folds.hidden(4));
    assert!(e.folds.hidden(6) == false && e.folds.hidden(7));
    // Typing on a visible line above keeps the folds where they were.
    let mut e2 = org(DOC, "<S-Tab>ggOnew<Esc>");
    assert_eq!(shown(&e2), vec!["new", "#+TITLE: t", "* A...", "* B..."]);
    e2.handle_key(crate::key::Key::ch('u'));
    assert_eq!(shown(&e2), vec!["#+TITLE: t", "* A...", "* B..."]);
    e.handle_key(crate::key::Key::ch('u'));
}

#[test]
fn drawers_and_blocks_fold_on_tab() {
    let text = "* H\n:PROPERTIES:\n:ID: x\n:END:\n#+begin_src sh\necho\n#+end_src\n";
    let e = org(text, "");
    assert_eq!(shown(&e).len(), 7, "showeverything leaves drawers open");
    let text = &format!("#+STARTUP: showall\n{text}");
    let e = org(text, "");
    assert_eq!(shown(&e), vec!["#+STARTUP: showall", "* H", ":PROPERTIES:...", "#+begin_src sh", "echo", "#+end_src"]);
    let e = org(text, "jjj<Tab>");
    assert_eq!(e.cur.line, 5);
    assert_eq!(shown(&e), vec!["#+STARTUP: showall", "* H", ":PROPERTIES:...", "#+begin_src sh..."]);
    // TAB on the drawer line opens it.
    let e = org(text, "jj<Tab>");
    assert_eq!(shown(&e).len(), 8);
}

