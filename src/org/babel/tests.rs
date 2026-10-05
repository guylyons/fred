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
    (0..e.line_count())
        .map(|l| e.buf.line(l))
        .collect::<Vec<_>>()
        .join("\n")
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
        assert_eq!(
            text(&e),
            want.trim_end_matches('\n'),
            "{input:?} with {cmd}: {:?}",
            e.msg
        );
    }
}

#[test]
fn insert_result_formats() {
    if !have("emacs") {
        return;
    }
    options::put(
        "org-babel-min-lines-for-block-output",
        toml::Value::Integer(1),
    );
    check(&[(
        "<point>#+begin_src emacs-lisp\n\"* Not an headline\"\n#+end_src\n",
        "org-babel-execute-maybe",
        "#+begin_src emacs-lisp\n\"* Not an headline\"\n#+end_src\n\n#+RESULTS:\n#+begin_example\n,* Not an headline\n#+end_example",
    )]);
    options::put(
        "org-babel-min-lines-for-block-output",
        toml::Value::Integer(10),
    );
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
        (
            "text <point>src_sh{echo 1} more\n",
            "org-babel-execute-maybe",
            "text src_sh{echo 1} {{{results(=1=)}}} more\n",
        ),
        (
            "text <point>src_sh[:results raw]{echo 1} more\n",
            "org-babel-execute-maybe",
            "text src_sh[:results raw]{echo 1} 1\n more\n",
        ),
        (
            "#+name: inner\n#+begin_src sh\necho in\n#+end_src\n\n<point>#+begin_src sh :noweb yes :results output\n# <<inner>>\necho out\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+name: inner\n#+begin_src sh\necho in\n#+end_src\n\n#+begin_src sh :noweb yes :results output\n# <<inner>>\necho out\n#+end_src\n\n#+RESULTS:\n: out\n",
        ),
        (
            "#+begin_src sh\necho a\n#+end_src\n\n#+RESULTS:\n: a\n\nafter\n",
            "org-babel-remove-result",
            "#+begin_src sh\necho a\n#+end_src\n\nafter\n",
        ),
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
        (
            "<point>#+begin_src emacs-lisp :eval never\n1\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+begin_src emacs-lisp :eval never\n1\n#+end_src\n",
        ),
    ]);
    let mut e = buf(
        "x\n#+name: x\n#+begin_src emacs-lisp\n1\n#+end_src\n\n#+RESULTS: x\n: 1\n\n<point>#+begin_src emacs-lisp\n2\n#+end_src\n\n#+RESULTS:\n: 2\n",
    );
    crate::org::run(&mut e, "org-babel-remove-result-one-or-many", Prefix::U(1));
    assert_eq!(
        text(&e),
        "x\n#+name: x\n#+begin_src emacs-lisp\n1\n#+end_src\n\n#+begin_src emacs-lisp\n2\n#+end_src"
    );
}

#[test]
fn file_results_and_inline_calls() {
    if !have("sh") || !have("emacs") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.org");
    let mut e =
        buf("<point>#+begin_src sh :results output file :file out.txt\necho a\n#+end_src\n");
    e.path = Some(path.clone());
    run(&mut e, "org-babel-execute-maybe");
    assert_eq!(
        text(&e),
        "#+begin_src sh :results output file :file out.txt\necho a\n#+end_src\n\n#+RESULTS:\n[[file:out.txt]]"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
        "a\n"
    );
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
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "Tangled 3 code blocks from t.org"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.el")).unwrap(),
        ";; [[file:t.org::*Top][Top:1]]\n(message \"hi\")\n;; Top:1 ends here\n\n\n\n;; #+name: two\n\n;; [[file:t.org::two][two]]\n2\n;; two ends here\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.sh")).unwrap(),
        "#!/bin/sh\necho x\n"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(dir.path().join("out.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    // :tangle yes with noweb references.
    let mut e = buf(
        "#+begin_src emacs-lisp :tangle yes :noweb yes\n(list <<x>>)\n#+end_src\n#+name: x\n#+begin_src emacs-lisp\n1\n#+end_src\n",
    );
    e.path = Some(dir.path().join("t2.org"));
    run(&mut e, "org-babel-tangle");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("t2.el")).unwrap(),
        "(list 1)\n"
    );
}

#[test]
fn export_evaluates_per_exports() {
    if !have("sh") || !have("emacs") {
        return;
    }
    setup();
    let text = "#+begin_src emacs-lisp :exports both\n(+ 1 2)\n#+end_src\n\nText src_emacs-lisp[:exports results]{(* 2 3)} end.\n\n#+begin_src sh :exports none\necho hi\n#+end_src\n\n#+begin_src sh :exports results\necho hi\n#+end_src\n";
    let out =
        super::export_process_text(text, None, &Default::default(), &Default::default()).unwrap();
    assert_eq!(
        out,
        "#+begin_src emacs-lisp :exports both\n(+ 1 2)\n#+end_src\n\n#+RESULTS:\n: 3\n\nText {{{results(=6=)}}} end.\n\n#+RESULTS:\n: hi\n"
    );
}

/// One block per language, results checked against upstream.
fn lang_case(tool: &str, src: &str, results: &str) {
    if !have(tool) {
        return;
    }
    let input = format!("<point>{src}");
    let want = format!("{}\n#+RESULTS:\n{results}", src);
    check(&[(&input, "org-babel-execute-maybe", &want)]);
}

#[test]
fn languages_like_upstream() {
    lang_case(
        "perl",
        "#+begin_src perl :var x=3\n$x * 2\n#+end_src\n",
        ": 6",
    );
    lang_case(
        "perl",
        "#+begin_src perl :results output\nprint \"a\\tb\\n\";\n#+end_src\n",
        ": a\tb",
    );
    lang_case(
        "lua",
        "#+begin_src lua :var x=2\nreturn x * 3\n#+end_src\n",
        ": 6",
    );
    lang_case(
        "gcc",
        "#+begin_src C :includes <stdio.h> :var n=4\nprintf(\"%d\\n\", n*n);\n#+end_src\n",
        ": 16",
    );
    lang_case(
        "g++",
        "#+begin_src C++ :includes <iostream>\nstd::cout << \"hi\" << std::endl;\n#+end_src\n",
        ": hi",
    );
    lang_case(
        "gfortran",
        "#+begin_src fortran :var n=5\nwrite(*,*) n*2\n#+end_src\n",
        ": 10",
    );
    lang_case(
        "node",
        "#+begin_src js :var a=[1,2]\nreturn a.map(x => x * 2)\n#+end_src\n",
        "| 2 | 4 |",
    );
    lang_case(
        "sh",
        "#+begin_src css\nbody { color: red }\n#+end_src\n",
        ": body { color: red }",
    );
    lang_case(
        "sh",
        "#+begin_src latex\n\\LaTeX{}\n#+end_src\n",
        "#+begin_export latex\n\\LaTeX{}\n#+end_export",
    );
    lang_case(
        "sh",
        "#+begin_src makefile\nall:\n\techo hi\n#+end_src\n",
        ": all:\n: \techo hi",
    );
    lang_case(
        "sh",
        "#+begin_src processing :var x=1\nellipse(x, 2, 3, 4);\n#+end_src\n",
        "#+begin_export html\n<script src=\"processing.js\"></script>\n <script type=\"text/processing\" data-processing-target=\"ob-cd1470ca4fe6eb94bcebf27dffe469f6c536bd83\">\nint x=1;\nellipse(x, 2, 3, 4);\n</script> <canvas id=\"ob-cd1470ca4fe6eb94bcebf27dffe469f6c536bd83\"></canvas>\n#+end_export",
    );
    lang_case(
        "bash",
        "#+begin_src bash :var arr='(\"a\" \"b\") :results output\necho ${arr[1]}\n#+end_src\n",
        ": b",
    );
    lang_case(
        "sh",
        "#+begin_src sh :cmdline \"x y\" :results output\necho $2\n#+end_src\n",
        ": y",
    );
    lang_case(
        "sh",
        "#+begin_src sh :results value\nexit 3\n#+end_src\n",
        "",
    );
    if have("awk") {
        check(&[(
            "#+name: t\n| 1 | 2 |\n| 3 | 4 |\n\n<point>#+begin_src awk :stdin t\n{print $1 + $2}\n#+end_src\n",
            "org-babel-execute-maybe",
            "#+name: t\n| 1 | 2 |\n| 3 | 4 |\n\n#+begin_src awk :stdin t\n{print $1 + $2}\n#+end_src\n\n#+RESULTS:\n| 3 |\n| 7 |",
        )]);
    }
    if have("sed") {
        check(&[(
            "<point>#+begin_src sed :stdin x :results output\ns/a/b/\n#+end_src\n\n#+name: x\n: aaa\n",
            "org-babel-execute-maybe",
            "#+begin_src sed :stdin x :results output\ns/a/b/\n#+end_src\n\n#+RESULTS:\n: baa\n\n#+name: x\n: aaa",
        )]);
    }
    check(&[(
        "<point>#+begin_src sh :stdin t :results output\ncat\n#+end_src\n\n#+name: t\n| a | b |\n",
        "org-babel-execute-maybe",
        "#+begin_src sh :stdin t :results output\ncat\n#+end_src\n\n#+RESULTS:\n: a\tb\n\n#+name: t\n| a | b |",
    )]);
}

#[test]
fn calc_and_org_values() {
    if !have("emacs") {
        return;
    }
    setup();
    let mut doc = super::doc::Doc::new(
        "#+name: ob-calc-table-2\n| 1 | 2 | 3 | 4 | 5 |\n\n#+begin_src calc :results silent verbatim :var a=ob-calc-table-2\n\ta*2 - 2\n#+end_src\n",
        None,
    );
    doc.point = (3, 0);
    let mut cx = super::core::Cx::default();
    let r = super::core::execute_src_block(
        &mut doc,
        &mut cx,
        false,
        None,
        &[],
        super::core::ExecKind::Src,
    )
    .unwrap();
    assert_eq!(
        r,
        Some(crate::org::sexp::Sexp::Str("[0, 2, 4, 6, 8]".into()))
    );
    let mut doc = super::doc::Doc::new(
        "#+begin_src org :var x=\"world\"\nHello $x\n#+end_src\n",
        None,
    );
    let r = super::core::execute_src_block(
        &mut doc,
        &mut cx,
        false,
        None,
        &[],
        super::core::ExecKind::Src,
    )
    .unwrap();
    assert_eq!(r, Some(crate::org::sexp::Sexp::Str("Hello world".into())));
    assert_eq!(doc.len(), 3, "org blocks are raw silent");
}

#[test]
fn ctrl_c_ctrl_c_runs_blocks_and_calls() {
    if !have("sh") {
        return;
    }
    let mut e = buf("#+name: f\n#+begin_src sh\necho hi\n#+end_src\n\n<point>#+call: f()\n");
    e.handle_key(crate::key::Key::ctrl('c'));
    e.handle_key(crate::key::Key::ctrl('c'));
    assert_eq!(
        text(&e),
        "#+name: f\n#+begin_src sh\necho hi\n#+end_src\n\n#+call: f()\n\n#+RESULTS:\n: hi"
    );
}

#[test]
fn confirmation_asks_and_remembers() {
    if !have("sh") {
        return;
    }
    setup();
    options::put("org-confirm-babel-evaluate", toml::Value::Boolean(true));
    let src = "<point>#+begin_src sh\necho 1\n#+end_src\n";
    let mut e = org(&src.replace("<point>", ""), "");
    run(&mut e, "org-babel-execute-maybe");
    assert_eq!(
        text(&e),
        "#+begin_src sh\necho 1\n#+end_src",
        "waits for an answer"
    );
    e.handle_key(crate::key::Key::ch('n'));
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "Evaluation of this sh code block is aborted."
    );
    run(&mut e, "org-babel-execute-maybe");
    e.handle_key(crate::key::Key::ch('y'));
    assert_eq!(
        text(&e),
        "#+begin_src sh\necho 1\n#+end_src\n\n#+RESULTS:\n: 1"
    );
    options::put("org-confirm-babel-evaluate", toml::Value::Boolean(false));
}

#[test]
fn navigation_and_editing_commands() {
    let src = "* H\n#+name: a\n#+begin_src sh\necho a\n#+end_src\n\n#+begin_src sh :results output\necho b\n#+end_src\n";
    let mut e = buf(src);
    run(&mut e, "org-babel-next-src-block");
    assert_eq!(e.cur.line, 2);
    run(&mut e, "org-babel-next-src-block");
    assert_eq!(e.cur.line, 6);
    run(&mut e, "org-babel-next-src-block");
    assert!(
        e.msg
            .as_ref()
            .unwrap()
            .0
            .ends_with("No further code blocks")
    );
    run(&mut e, "org-babel-previous-src-block");
    assert_eq!(e.cur.line, 2);
    e.set_cursor(3, 0);
    run(&mut e, "org-babel-goto-src-block-head");
    assert_eq!(e.cur.line, 2);
    // Demarcate: split at point.
    let mut e = buf("#+begin_src emacs-lisp :var x=1\n;; one\n<point>;; two\n#+end_src\n");
    run(&mut e, "org-babel-demarcate-block");
    assert_eq!(
        text(&e),
        "#+begin_src emacs-lisp :var x=1\n  ;; one\n#+end_src\n\n#+begin_src emacs-lisp :var x=1\n  ;; two\n#+end_src"
    );
    // Check: misspelled header.
    let mut e = buf("<point>#+begin_src sh :cahce yes\necho\n#+end_src\n");
    run(&mut e, "org-babel-check-src-block");
    assert!(
        e.msg
            .as_ref()
            .unwrap()
            .0
            .ends_with("Supplied header \"cahce\" is suspiciously close to \"cache\"")
    );
    // Insert header argument through completion.
    let mut e = buf("<point>#+begin_src sh\necho\n#+end_src\n");
    run(&mut e, "org-babel-insert-header-arg");
    for k in crate::key::parse_keys("dir<Enter>/tmp<Enter>") {
        e.handle_key(k);
    }
    assert_eq!(e.buf.line(0), "#+begin_src sh :dir /tmp");
}

#[test]
fn shell_sessions_persist() {
    if !have("sh") {
        return;
    }
    let mut e = buf(
        "<point>#+begin_src sh :session s1\nX=42\n#+end_src\n\n#+begin_src sh :session s1 :results output\necho $X\n#+end_src\n",
    );
    run(&mut e, "org-babel-execute-maybe");
    let l = (0..e.line_count())
        .rfind(|&l| e.buf.line(l).starts_with("#+begin_src"))
        .unwrap();
    e.set_cursor(l, 0);
    run(&mut e, "org-babel-execute-maybe");
    assert!(text(&e).ends_with("#+RESULTS:\n: 42"), "{}", text(&e));
    if have("python3") {
        let mut e = buf(
            "<point>#+begin_src python :session py :results output\nx = 6\nprint(x * 7)\n#+end_src\n",
        );
        run(&mut e, "org-babel-execute-maybe");
        assert!(text(&e).ends_with("#+RESULTS:\n: 42"), "{}", text(&e));
        let mut e = buf("<point>#+begin_src python :session py\nx + 1\n#+end_src\n");
        run(&mut e, "org-babel-execute-maybe");
        assert!(text(&e).ends_with("#+RESULTS:\n: 7"), "{}", text(&e));
    }
}

#[test]
fn library_of_babel_and_sbe() {
    if !have("emacs") {
        return;
    }
    setup();
    let dir = tempfile::tempdir().unwrap();
    let lib = dir.path().join("lib.org");
    std::fs::write(
        &lib,
        "#+name: lib-double\n#+begin_src emacs-lisp :var n=1\n(* 2 n)\n#+end_src\n",
    )
    .unwrap();
    let mut e = buf("x\n");
    run(&mut e, "org-babel-lob-ingest");
    for c in lib.display().to_string().chars() {
        e.handle_key(crate::key::Key::ch(c));
    }
    // The prompt holds the buffer's file name first: replace it.
    let mut e2 = buf("<point>#+call: lib-double(n=21)\n");
    let mut d = super::doc::Doc::new(&std::fs::read_to_string(&lib).unwrap(), Some(lib.clone()));
    super::core::lob_ingest(&mut d, &mut super::core::Cx::default()).unwrap();
    run(&mut e2, "org-babel-execute-maybe");
    assert_eq!(text(&e2), "#+call: lib-double(n=21)\n\n#+RESULTS:\n: 42");
    let e3 = buf("#+name: sq\n#+begin_src emacs-lisp :var x=0\n(* x x)\n#+end_src\n");
    let r = super::sbe(
        &e3,
        "sq",
        "",
        &[("x".into(), vec!["7".into()])],
        &Default::default(),
    )
    .unwrap();
    assert_eq!(r, "49");
    drop(e);
}

#[test]
fn sha1_hash_matches_upstream() {
    let mut e = buf("<point>#+begin_src emacs-lisp :cache yes :var x=1\nx\n#+end_src\n");
    run(&mut e, "org-babel-sha1-hash");
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "d97aa4375142853b7e06244c008f2c899afdc48a"
    );
}
