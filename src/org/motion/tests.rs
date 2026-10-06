//! Expectations from Emacs (point as a 0-based offset; buffers end in a
//! newline, which Fred keeps as the buffer's final-newline flag).

use super::super::tests::org;
use super::*;

fn run(text: &str, at: usize, cmd: &str) -> (String, usize) {
    let mut e = org(text, "");
    let b = EBuf::new(&e.buf.text(), at);
    let (l, c) = b.line_col(at);
    e.set_cursor(l, c);
    super::super::call(&mut e, cmd, Prefix::None).unwrap();
    let b = EBuf::load(&e);
    (e.buf.text(), b.pt)
}

#[test]
fn paragraphs_match_emacs() {
    let t = "Para one\n\nPara two\n\n- a\n- b\n";
    assert_eq!(run(t, 0, "org-forward-paragraph").1, 9);
    assert_eq!(run(t, 10, "org-forward-paragraph").1, 19);
    assert_eq!(run(t, 15, "org-backward-paragraph").1, 9);
}

#[test]
fn elements_match_emacs() {
    let t = "A\n\nB\n\nC\n";
    assert_eq!(run(t, 3, "org-forward-element").1, 6);
    assert_eq!(run(t, 3, "org-backward-element").1, 0);
    assert_eq!(
        run(t, 3, "org-drag-element-backward"),
        ("B\n\nA\n\nC".into(), 0)
    );
    assert_eq!(
        run(t, 0, "org-drag-element-forward"),
        ("B\n\nA\n\nC".into(), 3)
    );
    assert_eq!(
        run(t, 3, "org-transpose-element"),
        ("B\n\nA\n\nC".into(), 6)
    );
    assert_eq!(
        run("x\ny\nz\n", 2, "org-drag-line-forward"),
        ("x\nz\ny".into(), 4)
    );
    assert_eq!(run("* H\n- a\n- b\n", 6, "org-up-element").1, 4);
}
