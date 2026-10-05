//! Babel tests, ported from testing/lisp/test-ob*.el (expected values
//! checked against upstream in `emacs --batch`). Tests that need a
//! missing interpreter are skipped with a note.

use super::super::tests::org;
use crate::editor::Editor;
use crate::org::{Prefix, options};

fn setup() {
    options::put("org-confirm-babel-evaluate", toml::Value::Boolean(false));
    options::put(
        "org-babel-load-languages",
        toml::Value::String(
            "'((emacs-lisp . t) (shell . t) (python . t) (ruby . t) (sqlite . t) (perl . t) (js . t) (lua . t) (awk . t) (sed . t) (C . t) (org . t) (css . t) (makefile . t) (processing . t) (latex . t) (calc . t) (dot . t) (R . t) (fortran . t))".into(),
        ),
    );
}

/// An Org buffer with TEXT, point at `<point>` (or the start).
fn buf(text: &str) -> Editor {
    setup();
    let (before, after) = text.split_once("<point>").unwrap_or(("", text));
    let mut e = org(&format!("{before}{after}"), "");
    let line = before.matches('\n').count();
    let col = before.rsplit('\n').next().unwrap_or("").len();
    e.set_cursor(line, col);
    e
}

fn run(e: &mut Editor, cmd: &str) {
    crate::org::run(e, cmd, Prefix::None);
}

fn text(e: &Editor) -> String {
    (0..e.line_count()).map(|l| e.buf.line(l)).collect::<Vec<_>>().join("\n")
}

fn have(cmd: &str) -> bool {
    let ok = super::lang::which(cmd);
    if !ok {
        eprintln!("skipped: `{cmd}' is not installed");
    }
    ok
}

/// Execute the block at point and return the buffer.
fn exec(src: &str) -> String {
    let mut e = buf(src);
    run(&mut e, "org-babel-execute-src-block");
    text(&e)
}

#[test]
fn shell_output_and_table_results() {
    if !have("sh") {
        return;
    }
    assert_eq!(
        exec("<point>#+begin_src sh\necho 1\necho 2\n#+end_src\n"),
        "#+begin_src sh\necho 1\necho 2\n#+end_src\n\n#+RESULTS:\n| 1 |\n| 2 |"
    );
    assert_eq!(
        exec("<point>#+begin_src sh :results output\necho hello\n#+end_src\n"),
        "#+begin_src sh :results output\necho hello\n#+end_src\n\n#+RESULTS:\n: hello"
    );
}

/// (buffer with <point>, command, expected buffer) checked against
/// upstream (test-ob.el behaviors).
fn check(cases: &[(&str, &str, &str)]) {
    for (input, cmd, want) in cases {
        let mut e = buf(input);
        run(&mut e, cmd);
        assert_eq!(text(&e), want.trim_end_matches('\n'), "{input:?} with {cmd}: {:?}", e.msg);
    }
}

#[test]
fn insert_result_formats() {
    if !have("emacs") {
        return;
    }
    options::put("org-babel-min-lines-for-block-output", toml::Value::Integer(1));
    check(&[(
        "<point>#+begin_src emacs-lisp\n\"* Not an headline\"\n#+end_src\n",
        "org-babel-execute-maybe",
        "#+begin_src emacs-lisp\n\"* Not an headline\"\n#+end_src\n\n#+RESULTS:\n#+begin_example\n,* Not an headline\n#+end_example",
    )]);
    options::put("org-babel-min-lines-for-block-output", toml::Value::Integer(10));
    check(&[
        (
            "<point>#+begin_src emacs-lisp :results raw\n\"* Not an headline\"\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :results raw\n\"* Not an headline\"\n#+end_src\n\n#+RESULTS:\n* Not an headline\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :results drawer\n\"a\\nb\"\n#+end_src\n\nnext\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :results drawer\n\"a\\nb\"\n#+end_src\n\n#+RESULTS:\n:results:\na\nb\n:end:\n\nnext\n",
        ),
        (
            "#+name: tbl\n| 1 | 2 |\n| 3 | 4 |\n\n#+name: blk\n<point>#+begin_src emacs-lisp :var x=tbl[1,0]\nx\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+name: tbl\n| 1 | 2 |\n| 3 | 4 |\n\n#+name: blk\n#+begin_src emacs-lisp :var x=tbl[1,0]\nx\n#+end_src\n\n#+RESULTS: blk\n: 3\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :cache yes\n'(1 2)\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :cache yes\n'(1 2)\n#+end_src\n\n#+RESULTS[dbea93064141ee93caceff6ebfe00116246c4918]:\n| 1 | 2 |\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :wrap example\n\"x\"\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :wrap example\n\"x\"\n#+end_src\n\n#+RESULTS:\n#+begin_example\nx\n#+end_example\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :results html\n\"<b>x</b>\"\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :results html\n\"<b>x</b>\"\n#+end_src\n\n#+RESULTS:\n#+begin_export html\n<b>x</b>\n#+end_export\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :results list\n'(\"a\" \"b\" 3)\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :results list\n'(\"a\" \"b\" 3)\n#+end_src\n\n#+RESULTS:\n- a\n- b\n- 3\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :results code\n'(1 2)\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :results code\n'(1 2)\n#+end_src\n\n#+RESULTS:\n#+begin_src emacs-lisp\n(1 2)\n#+end_src\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :results value scalar\n'(1 2)\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :results value scalar\n'(1 2)\n#+end_src\n\n#+RESULTS:\n: (1 2)\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :results value :var x=\"a b\"\nx\n#+end_src\n\n#+RESULTS:\n: old\n\nafter\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :results value :var x=\"a b\"\nx\n#+end_src\n\n#+RESULTS:\n: a b\n\nafter\n",
        ),
        (
            "#+name: add\n#+begin_src emacs-lisp :var a=1 b=2\n(+ a b)\n#+end_src\n\n<point>#+call: add(a=5)\n",
            "org-babel-execute-maybe",
            "#+name: add\n#+begin_src emacs-lisp :var a=1 b=2\n(+ a b)\n#+end_src\n\n#+call: add(a=5)\n\n#+RESULTS:\n: 7\n",
        ),
        (
            "<point>#+begin_src emacs-lisp :post wrap(x=*this*)\n\"y\"\n#+end_src\n\n#+name: wrap\n#+begin_src emacs-lisp :var x=\"\"\n(concat \"[\" x \"]\")\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :post wrap(x=*this*)\n\"y\"\n#+end_src\n\n#+RESULTS:\n: [y]\n\n#+name: wrap\n#+begin_src emacs-lisp :var x=\"\"\n(concat \"[\" x \"]\")\n#+end_src\n",
        ),
    ]);
}

#[test]
fn shell_blocks_like_upstream() {
    if !have("sh") {
        return;
    }
    check(&[
        ("text <point>src_sh{echo 1} more\n", "org-babel-execute-maybe", "text src_sh{echo 1} {{{results(=1=)}}} more\n"),
        ("text <point>src_sh[:results raw]{echo 1} more\n", "org-babel-execute-maybe", "text src_sh[:results raw]{echo 1} 1\n more\n"),
        (
            "#+name: inner\n#+begin_src sh\necho in\n#+end_src\n\n<point>#+begin_src sh :noweb yes :results output\n# <<inner>>\necho out\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+name: inner\n#+begin_src sh\necho in\n#+end_src\n\n#+begin_src sh :noweb yes :results output\n# <<inner>>\necho out\n#+end_src\n\n#+RESULTS:\n: out\n",
        ),
        ("#+begin_src sh\necho a\n#+end_src\n\n#+RESULTS:\n: a\n\nafter\n", "org-babel-remove-result", "#+begin_src sh\necho a\n#+end_src\n\nafter\n"),
        (
            "* H\n  <point>#+begin_src sh :results output\n  echo a\n  echo b\n  #+end_src\n",
            "org-babel-execute-maybe",
            "* H\n  #+begin_src sh :results output\n  echo a\n  echo b\n  #+end_src\n\n  #+RESULTS:\n  : a\n  : b\n",
        ),
        (
            "<point>#+begin_src sh :results output\nfor i in 1 2 3 4 5 6 7 8 9 10; do echo $i; done\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src sh :results output\nfor i in 1 2 3 4 5 6 7 8 9 10; do echo $i; done\n#+end_src\n\n#+RESULTS:\n#+begin_example\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n#+end_example",
        ),
    ]);
}

#[test]
fn python_values_tables_and_names() {
    if !have("python") {
        return;
    }
    check(&[
        (
            "<point>#+begin_src python\nreturn [[1, 'a'], [2, None]]\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src python\nreturn [[1, 'a'], [2, None]]\n#+end_src\n\n#+RESULTS:\n| 1 | a    |\n| 2 | None |\n",
        ),
        (
            "<point>#+begin_src python :results output\nprint('hi')\nprint(1+1)\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src python :results output\nprint('hi')\nprint(1+1)\n#+end_src\n\n#+RESULTS:\n: hi\n: 2\n",
        ),
        (
            "#+name: t\n| a | b |\n|---+---|\n| 1 | 2 |\n| 3 | 4 |\n\n<point>#+begin_src python :var t=t\nreturn [r[0]*10 for r in t]\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+name: t\n| a | b |\n|---+---|\n| 1 | 2 |\n| 3 | 4 |\n\n#+begin_src python :var t=t\nreturn [r[0]*10 for r in t]\n#+end_src\n\n#+RESULTS:\n| 10 | 30 |\n",
        ),
        (
            "#+name: t\n| a | b |\n|---+---|\n| 1 | 2 |\n| 3 | 4 |\n\n<point>#+begin_src python :var t=t :colnames yes\nreturn [[r[0]*10, r[1]] for r in t]\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+name: t\n| a | b |\n|---+---|\n| 1 | 2 |\n| 3 | 4 |\n\n#+begin_src python :var t=t :colnames yes\nreturn [[r[0]*10, r[1]] for r in t]\n#+end_src\n\n#+RESULTS:\n|  a | b |\n|----+---|\n| 10 | 2 |\n| 30 | 4 |\n",
        ),
        (
            "#+name: t\n| r1 | 1 |\n| r2 | 2 |\n\n<point>#+begin_src python :var t=t :rownames yes\nreturn [[x[0]+1] for x in t]\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+name: t\n| r1 | 1 |\n| r2 | 2 |\n\n#+begin_src python :var t=t :rownames yes\nreturn [[x[0]+1] for x in t]\n#+end_src\n\n#+RESULTS:\n| r1 | 2 |\n| r2 | 3 |\n",
        ),
    ]);
}

#[test]
fn ruby_and_sqlite() {
    if have("ruby") {
        check(&[(
            "<point>#+begin_src ruby\n[1, 2, nil]\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src ruby\n[1, 2, nil]\n#+end_src\n\n#+RESULTS:\n| 1 | 2 | hline |\n",
        )]);
    }
    if have("sqlite3") {
        check(&[(
            "<point>#+begin_src sqlite :colnames yes\ncreate table x(a,b); insert into x values (1,'p'),(2,'q'); select * from x;\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src sqlite :colnames yes\ncreate table x(a,b); insert into x values (1,'p'),(2,'q'); select * from x;\n#+end_src\n\n#+RESULTS:\n| 1 | p |\n|---+---|\n| 2 | q |\n",
        )]);
    }
}

#[test]
fn header_args_from_properties_and_result_handling() {
    if !have("sh") {
        return;
    }
    check(&[
        (
            "#+PROPERTY: header-args :results output\n* H\n:PROPERTIES:\n:header-args+: :var y=2\n:END:\n<point>#+begin_src sh\necho $y\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+PROPERTY: header-args :results output\n* H\n:PROPERTIES:\n:header-args+: :var y=2\n:END:\n#+begin_src sh\necho $y\n#+end_src\n\n#+RESULTS:\n: 2\n",
        ),
        (
            "<point>#+begin_src sh :results append\necho a\n#+end_src\n\n#+RESULTS:\n| x |\n",
            "org-babel-execute-maybe",
            "#+begin_src sh :results append\necho a\n#+end_src\n\n#+RESULTS:\n| x |\n: a\n",
        ),
        (
            "* A\n#+begin_src sh\necho 1\n#+end_src\n* B\n#+begin_src sh\necho 2\n#+end_src\n#+call: x()\n",
            "org-babel-execute-buffer",
            "* A\n#+begin_src sh\necho 1\n#+end_src\n\n#+RESULTS:\n: 1\n\n* B\n#+begin_src sh\necho 2\n#+end_src\n\n#+RESULTS:\n: 2\n\n#+call: x()\n",
        ),
        ("<point>#+begin_src emacs-lisp :eval never\n1\n#+end_src\n", "org-babel-execute-maybe", "#+begin_src emacs-lisp :eval never\n1\n#+end_src\n"),
    ]);
    let mut e = buf("x\n#+name: x\n#+begin_src emacs-lisp\n1\n#+end_src\n\n#+RESULTS: x\n: 1\n\n<point>#+begin_src emacs-lisp\n2\n#+end_src\n\n#+RESULTS:\n: 2\n");
    crate::org::run(&mut e, "org-babel-remove-result-one-or-many", Prefix::U(1));
    assert_eq!(text(&e), "x\n#+name: x\n#+begin_src emacs-lisp\n1\n#+end_src\n\n#+begin_src emacs-lisp\n2\n#+end_src");
}

#[test]
fn file_results_and_inline_calls() {
    if !have("sh") || !have("emacs") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.org");
    let mut e = buf("<point>#+begin_src sh :results output file :file out.txt\necho a\n#+end_src\n");
    e.path = Some(path.clone());
    run(&mut e, "org-babel-execute-maybe");
    assert_eq!(text(&e), "#+begin_src sh :results output file :file out.txt\necho a\n#+end_src\n\n#+RESULTS:\n[[file:out.txt]]");
    assert_eq!(std::fs::read_to_string(dir.path().join("out.txt")).unwrap(), "a\n");
    check(&[(
        "#+name: square\n#+begin_src emacs-lisp :var n=2\n(* n n)\n#+end_src\n\nResult: <point>call_square(n=3)\n",
        "org-babel-execute-maybe",
        "#+name: square\n#+begin_src emacs-lisp :var n=2\n(* n n)\n#+end_src\n\nResult: call_square(n=3) {{{results(=9=)}}}\n",
    )]);
}

#[test]
fn tangle_writes_files_with_link_comments() {
    let dir = tempfile::tempdir().unwrap();
    let src = "* Top\n#+begin_src emacs-lisp :tangle out.el :comments link\n(message \"hi\")\n#+end_src\n\n#+name: two\n#+begin_src emacs-lisp :tangle out.el :comments both\n2\n#+end_src\n#+begin_src sh :tangle out.sh :shebang #!/bin/sh\necho x\n#+end_src\n";
    let mut e = buf(src);
    e.path = Some(dir.path().join("t.org"));
    run(&mut e, "org-babel-tangle");
    assert_eq!(e.msg.as_ref().unwrap().0, "Tangled 3 code blocks from t.org");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.el")).unwrap(),
        ";; [[file:t.org::*Top][Top:1]]\n(message \"hi\")\n;; Top:1 ends here\n\n\n\n;; #+name: two\n\n;; [[file:t.org::two][two]]\n2\n;; two ends here\n"
    );
    assert_eq!(std::fs::read_to_string(dir.path().join("out.sh")).unwrap(), "#!/bin/sh\necho x\n");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(dir.path().join("out.sh")).unwrap().permissions().mode() & 0o777, 0o755);
    // :tangle yes with noweb references.
    let mut e = buf("#+begin_src emacs-lisp :tangle yes :noweb yes\n(list <<x>>)\n#+end_src\n#+name: x\n#+begin_src emacs-lisp\n1\n#+end_src\n");
    e.path = Some(dir.path().join("t2.org"));
    run(&mut e, "org-babel-tangle");
    assert_eq!(std::fs::read_to_string(dir.path().join("t2.el")).unwrap(), "(list 1)\n");
}

#[test]
fn export_evaluates_per_exports() {
    if !have("sh") || !have("emacs") {
        return;
    }
    setup();
    let text = "#+begin_src emacs-lisp :exports both\n(+ 1 2)\n#+end_src\n\nText src_emacs-lisp[:exports results]{(* 2 3)} end.\n\n#+begin_src sh :exports none\necho hi\n#+end_src\n\n#+begin_src sh :exports results\necho hi\n#+end_src\n";
    let out = super::export_process_text(text, None, &Default::default(), &Default::default()).unwrap();
    assert_eq!(out, "#+begin_src emacs-lisp :exports both\n(+ 1 2)\n#+end_src\n\n#+RESULTS:\n: 3\n\nText {{{results(=6=)}}} end.\n\n#+RESULTS:\n: hi\n");
}

#[test]
fn sha1_hash_matches_upstream() {
    let mut e = buf("<point>#+begin_src emacs-lisp :cache yes :var x=1\nx\n#+end_src\n");
    run(&mut e, "org-babel-sha1-hash");
    assert_eq!(e.msg.as_ref().unwrap().0, "d97aa4375142853b7e06244c008f2c899afdc48a");
}
