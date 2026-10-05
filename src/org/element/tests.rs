use super::*;
use crate::org::syntax;

fn st(text: &str) -> Settings {
    syntax::settings(text.lines(), None)
}

fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn unquote(s: &str) -> String {
    let s = &s[1..s.len() - 1];
    let mut o = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => o.push('\n'),
                Some('t') => o.push('\t'),
                Some('r') => o.push('\r'),
                Some(c) => o.push(c),
                None => {}
            }
        } else {
            o.push(c);
        }
    }
    o
}

const SKIP: &[&str] = &["structure", "path", "true-level"];

fn dump_value(t: &Tree, v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::T => "t".into(),
        Value::Int(n) => n.to_string(),
        Value::Str(s) => quote(s),
        Value::Sym(s) => s.clone(),
        Value::Node(n) => dump(t, *n),
        Value::Secondary(s) if s.is_empty() => "nil".into(),
        Value::List(l) if l.is_empty() => "nil".into(),
        Value::Secondary(s) => format!(
            "({})",
            s.iter().map(|&n| dump(t, n)).collect::<Vec<_>>().join(" ")
        ),
        Value::List(l) => format!(
            "({})",
            l.iter()
                .map(|x| dump_value(t, x))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Value::Cons(a, b) => match &**b {
            Value::Nil => format!("({})", dump_value(t, a)),
            Value::List(_) | Value::Secondary(_) if !b.is_nil() => {
                let inner = dump_value(t, b);
                format!("({} {}", dump_value(t, a), &inner[1..])
            }
            Value::List(_) | Value::Secondary(_) => format!("({})", dump_value(t, a)),
            b => format!("({} . {})", dump_value(t, a), dump_value(t, b)),
        },
    }
}

/// The golden dump of a node (tools/org-element-golden.el).
pub fn dump(t: &Tree, id: NodeId) -> String {
    if let Some(s) = t.plain_text(id) {
        return quote(s);
    }
    let mut props: Vec<(&str, &Value)> = t
        .props(id)
        .filter(|(k, v)| !v.is_nil() && !SKIP.contains(k))
        .collect();
    props.sort_by(|a, b| a.0.cmp(b.0));
    let mut o = format!("({}", t.kind(id).name());
    for (k, v) in props {
        o.push_str(&format!(" :{k} {}", dump_value(t, v)));
    }
    for &c in t.contents(id) {
        o.push(' ');
        o.push_str(&dump(t, c));
    }
    o.push(')');
    o
}

fn parse(text: &str) -> Tree {
    parse_buffer(text, &st(text))
}

fn golden(data: &str) -> Vec<(String, String, String)> {
    data.split("\n\n")
        .filter(|r| r.starts_with("#text "))
        .map(|r| {
            let mut lines = r.lines();
            let text = unquote(lines.next().unwrap().strip_prefix("#text ").unwrap());
            let tree = lines
                .next()
                .unwrap()
                .strip_prefix("#tree ")
                .unwrap()
                .to_owned();
            let interp = lines
                .next()
                .unwrap()
                .strip_prefix("#interp ")
                .unwrap()
                .to_owned();
            (text, tree, interp)
        })
        .collect()
}

fn excerpt(s: &str) -> String {
    s.chars().take(300).collect()
}

/// Where two dumps first differ, with context.
fn diff(want: &str, got: &str) -> String {
    let i = want
        .bytes()
        .zip(got.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let from = want[..i]
        .char_indices()
        .rev()
        .nth(400)
        .map_or(0, |(k, _)| k);
    let w: String = want[from..].chars().take(700).collect();
    let g: String = got[from..].chars().take(700).collect();
    format!(" want ...{w}\n  got ...{g}")
}

/// Compare Fred's parse trees and interpretations with golden records.
fn check_golden(data: &str) {
    let all = golden(data);
    let mut fails = vec![];
    for (text, tree, interp) in &all {
        let t = parse(text);
        let got = dump(&t, t.root);
        if !tree.starts_with("error") && &got != tree {
            fails.push(format!(
                "TEXT {}\n{}",
                excerpt(&quote(text)),
                diff(tree, &got)
            ));
        }
        let got = quote(&interpret(&t, t.root));
        if interp != "error" && &got != interp {
            fails.push(format!(
                "TEXT {} (interp)\n{}",
                excerpt(&quote(text)),
                diff(interp, &got)
            ));
        }
    }
    if !fails.is_empty() {
        let n = fails.len();
        let shown: Vec<String> = fails.into_iter().take(12).collect();
        panic!(
            "{n} differences in {} texts:\n{}",
            all.len(),
            shown.join("\n\n")
        );
    }
}

/// Fred's parser and interpreter against upstream org-element on every
/// text of test-org-element.el and extra cases
/// (tools/org-element-golden.el).
#[test]
fn golden_upstream() {
    check_golden(include_str!("testdata/golden.txt"));
}

/// The same check on a golden file generated from other upstream test
/// files: FRED_ORG_GOLDEN=FILE cargo test --lib golden_file -- --ignored.
#[test]
#[ignore]
fn golden_file() {
    let path = std::env::var("FRED_ORG_GOLDEN").expect("FRED_ORG_GOLDEN");
    check_golden(&std::fs::read_to_string(path).unwrap());
}

/// org-element-at-point and org-element-context at every position,
/// against records from upstream: "POS AT-TYPE AT-BEGIN CTX-TYPE
/// CTX-BEGIN" lines after each `#text`.
/// FRED_ORG_ATPOINT=FILE cargo test --lib golden_at_point -- --ignored.
#[test]
#[ignore]
fn golden_at_point() {
    let path = std::env::var("FRED_ORG_ATPOINT").expect("FRED_ORG_ATPOINT");
    let data = std::fs::read_to_string(path).unwrap();
    let mut fails = vec![];
    let mut total = 0;
    for rec in data.split("\n\n").filter(|r| r.starts_with("#text ")) {
        let mut lines = rec.lines();
        let text = unquote(lines.next().unwrap().strip_prefix("#text ").unwrap());
        let t = parse(&text);
        for l in lines {
            let f: Vec<&str> = l.split(' ').collect();
            let pos: usize = f[0].parse().unwrap();
            total += 1;
            let show = |n: Option<NodeId>| match n {
                Some(n) => format!(
                    "{} {}",
                    t.kind(n).name(),
                    t.begin(n).map_or("-".into(), |b| b.to_string())
                ),
                None => "nil -".into(),
            };
            let at = show(super::at_point(&text, &t, pos));
            let ctx = show(super::context(&text, &t, pos));
            let want_at = format!("{} {}", f[1], f[2]);
            let want_ctx = format!("{} {}", f[3], f[4]);
            if f[1] != "err" && at != want_at {
                fails.push(format!(
                    "{} @{pos}: at-point want {want_at} got {at}",
                    excerpt(&quote(&text))
                ));
            }
            if f[3] != "err" && ctx != want_ctx {
                fails.push(format!(
                    "{} @{pos}: context want {want_ctx} got {ctx}",
                    excerpt(&quote(&text))
                ));
            }
        }
    }
    if !fails.is_empty() {
        let n = fails.len();
        panic!(
            "{n}/{total} differ:\n{}",
            fails.into_iter().take(25).collect::<Vec<_>>().join("\n")
        );
    }
}

// ---- synthetic trees, as upstream tests write them: (TYPE PROPS . CHILDREN) ----

fn value_of(t: &mut Tree, key: &str, s: &crate::org::sexp::Sexp) -> Value {
    use crate::org::sexp::Sexp as S;
    match s {
        S::Nil => Value::Nil,
        S::T => Value::T,
        S::Int(n) => Value::Int(*n),
        S::Str(x) => Value::Str(x.clone()),
        S::Sym(x) => Value::Sym(x.clone()),
        S::List(l)
            if l.first()
                .and_then(|x| x.sym())
                .and_then(Type::from_name)
                .is_some() =>
        {
            Value::Node(build(t, s))
        }
        S::List(l) if ["title", "tag", "prefix", "suffix"].contains(&key) => {
            Value::Secondary(l.iter().map(|x| build(t, x)).collect())
        }
        S::List(l) if key == "caption" => Value::List(
            l.iter()
                .map(|e| match e.list() {
                    Some([main @ S::List(_), rest @ ..]) => Value::cons(
                        Value::Secondary(
                            main.list().unwrap().iter().map(|x| build(t, x)).collect(),
                        ),
                        Value::Secondary(rest.iter().map(|x| build(t, x)).collect()),
                    ),
                    Some(l) => Value::Secondary(l.iter().map(|x| build(t, x)).collect()),
                    None => value_of(t, "", e),
                })
                .collect(),
        ),
        S::List(l) => Value::List(l.iter().map(|x| value_of(t, "", x)).collect()),
        S::Dotted(l, d) if l.len() == 1 => Value::cons(value_of(t, "", &l[0]), value_of(t, "", d)),
        other => panic!("unsupported {other:?}"),
    }
}

fn build(t: &mut Tree, s: &crate::org::sexp::Sexp) -> NodeId {
    if let Some(x) = s
        .str()
        .filter(|_| matches!(s, crate::org::sexp::Sexp::Str(_)))
    {
        return t.text(x);
    }
    let l = s.list().expect("node");
    let name = l[0].sym().unwrap();
    let kind = Type::from_name(name)
        .unwrap_or_else(|| Type::Pseudo(Box::leak(name.to_owned().into_boxed_str())));
    let id = t.create(kind, vec![], vec![]);
    if let Some(props) = l.get(1).and_then(|p| p.list()) {
        for kv in props.chunks(2) {
            let k = kv[0].sym().unwrap().trim_start_matches(':').to_owned();
            let v = value_of(t, &k, &kv[1]);
            t.adopt_value_parent(id, &v);
            t.put(id, &k, v);
        }
    }
    let children: Vec<NodeId> = l.iter().skip(2).map(|c| build(t, c)).collect();
    t.set_contents(id, children);
    id
}

/// Interpret a synthetic tree given as Lisp data.
fn interp(data: &str) -> String {
    let mut t = Tree::new();
    let id = build(&mut t, &crate::org::sexp::read(data).unwrap());
    interpret(&t, id)
}

impl Tree {
    fn adopt_value_parent(&mut self, id: NodeId, v: &Value) {
        match v {
            Value::Node(n) => self.set_parent(*n, Some(id)),
            Value::Secondary(s) => s.iter().for_each(|&n| self.set_parent(n, Some(id))),
            Value::List(l) => l.iter().for_each(|x| self.adopt_value_parent(id, x)),
            Value::Cons(a, b) => {
                self.adopt_value_parent(id, a);
                self.adopt_value_parent(id, b);
            }
            _ => {}
        }
    }
}

#[test]
fn interpret_data() {
    assert_eq!(
        interp(r#"(org-data nil (paragraph (:name "para") "Paragraph"))"#),
        "#+name: para\nParagraph\n"
    );
    assert_eq!(
        interp(r#"(org-data nil (paragraph (:attr_ascii ("line1" "line2")) "Paragraph"))"#),
        "#+attr_ascii: line1\n#+attr_ascii: line2\nParagraph\n"
    );
    assert_eq!(
        interp(r#"(org-data nil (paragraph (:caption (("caption"))) "Paragraph"))"#),
        "#+caption: caption\nParagraph\n"
    );
    assert_eq!(
        interp(r#"(org-data nil (paragraph (:caption ((("long") "short"))) "Paragraph"))"#),
        "#+caption[short]: long\nParagraph\n"
    );
    assert_eq!(
        interp(
            r#"(org-data nil (paragraph (:caption ((("l1") "s1") (("l2") "s2"))) "Paragraph"))"#
        ),
        "#+caption[s1]: l1\n#+caption[s2]: l2\nParagraph\n"
    );
    // Pseudo objects and elements are transparent.
    assert_eq!(
        interp(r#"(paragraph nil (pseudo-object (:post-blank 1) "A") "B")"#).trim(),
        "A B"
    );
    assert_eq!(
        interp(
            r#"(center nil (pseudo-element (:post-blank 1) (paragraph nil "A")) (paragraph nil "B"))"#
        ),
        "A\n\nB\n"
    );
    // Obey post-blank in strings and elements.
    let mut t = Tree::new();
    let a = t.text("A");
    t.put(a, "post-blank", 1usize);
    assert_eq!(interpret(&t, a), "A ");
    assert_eq!(interp(r#"(paragraph (:post-blank 1) "Foo")"#), "Foo\n\n");
    assert_eq!(
        interp(r#"(section nil (paragraph (:post-blank 1) "Foo"))"#),
        "Foo\n\n"
    );
    assert_eq!(
        interp(r#"(section (:post-blank 1) (paragraph (:post-blank 1) "Foo"))"#),
        "Foo\n\n\n"
    );
}

#[test]
fn normalize_contents() {
    let cases = [
        (
            r#"(paragraph nil "  Two spaces\n   Three spaces")"#,
            "Two spaces\n Three spaces\n",
        ),
        (
            r#"(paragraph nil "  Two spaces\nNo space")"#,
            "  Two spaces\nNo space\n",
        ),
        (
            r#"(paragraph nil " One " (bold nil "space") "\n  Two spaces")"#,
            "One *space*\n Two spaces\n",
        ),
        (
            r#"(paragraph nil (verbatim (:value "V")) "No space\n  Two\n   Three")"#,
            "=V=No space\n  Two\n   Three\n",
        ),
        (
            r#"(paragraph nil "  Two spaces\n\n \n  Two spaces")"#,
            "Two spaces\n\n\nTwo spaces\n",
        ),
        (
            r#"(verse-block nil "  line 1\n\n  line 2")"#,
            "#+begin_verse\nline 1\n\nline 2#+end_verse\n",
        ),
        (
            r#"(paragraph nil "  Two spaces " (bold nil " and\n One space"))"#,
            " Two spaces * and\nOne space*\n",
        ),
        (
            r#"(paragraph nil " 1 space" (line-break) "  2 spaces")"#,
            "1 space\\\\\n 2 spaces\n",
        ),
    ];
    for (data, want) in cases {
        assert_eq!(interp(data), want, "{data}");
    }
    assert_eq!(normalize_string("Test", false), "Test\n");
    assert_eq!(normalize_string("Test\n \n\n", false), "Test\n");
    assert_eq!(normalize_string("Test\n\n", true), "Test\n\n");
    assert_eq!(normalize_string("", false), "");
}

#[test]
fn timestamp_interpreter() {
    let ts = |props: &str| interp(&format!("(timestamp ({props}))"));
    assert_eq!(
        ts(
            "type active year-start 2012 month-start 3 day-start 29 hour-start 16 minute-start 40"
                .split(' ')
                .enumerate()
                .map(|(i, w)| if i % 2 == 0 {
                    format!(":{w}")
                } else {
                    w.to_owned()
                })
                .collect::<Vec<_>>()
                .join(" ")
                .as_str()
        ),
        "<2012-03-29 Thu 16:40>"
    );
    assert_eq!(
        ts(
            ":type inactive :year-start 2012 :month-start 3 :day-start 29 :hour-start 16 :minute-start 40"
        ),
        "[2012-03-29 Thu 16:40]"
    );
    assert_eq!(
        ts(
            ":type active-range :year-start 2012 :month-start 3 :day-start 29 :hour-start 16 :minute-start 40 :year-end 2012 :month-end 3 :day-end 30"
        ),
        "<2012-03-29 Thu 16:40>--<2012-03-30 Fri>"
    );
    assert_eq!(
        ts(
            ":type active-range :year-start 2012 :month-start 3 :day-start 29 :hour-end 16 :minute-end 40 :year-end 2012 :month-end 3 :day-end 30"
        ),
        "<2012-03-29 Thu>--<2012-03-30 Fri 16:40>"
    );
    assert_eq!(
        ts(
            ":type active-range :range-type timerange :year-start 2012 :month-start 3 :day-start 29 :hour-start 16 :minute-start 40 :hour-end 16 :minute-end 41"
        ),
        "<2012-03-29 Thu 16:40-16:41>"
    );
    assert_eq!(
        ts(
            ":type active :year-start 2012 :month-start 3 :day-start 29 :repeater-type cumulate :repeater-value 1 :repeater-unit year :repeater-deadline-value 2 :repeater-deadline-unit year"
        ),
        "<2012-03-29 Thu +1y/2y>"
    );
    assert_eq!(
        ts(
            ":type active :year-start 2012 :month-start 3 :day-start 29 :warning-type first :warning-value 3 :warning-unit day :repeater-type restart :repeater-value 1 :repeater-unit week"
        ),
        "<2012-03-29 Thu .+1w --3d>"
    );
    assert_eq!(
        ts(":type diary :diary-sexp \"(diary-float t 4 2)\""),
        "<%%(diary-float t 4 2)>"
    );
    // Dates normalize like encode-time.
    assert_eq!(format_date(2012, 2, 30, None, None), "2012-03-01 Thu");
    assert_eq!(
        format_date(2012, 13, 1, Some(24), Some(30)),
        "2013-01-02 Wed 00:30"
    );
}

/// TEXT with `<point>` → (text, position).
fn at(s: &str) -> (String, usize) {
    let p = s.find("<point>").unwrap_or(0);
    (s.replacen("<point>", "", 1), p)
}

fn at_point_type(s: &str) -> Option<&'static str> {
    let (text, p) = at(s);
    let t = parse(&text);
    super::at_point(&text, &t, p).map(|e| t.kind(e).name())
}

fn context_type(s: &str) -> Option<&'static str> {
    let (text, p) = at(s);
    let t = parse(&text);
    super::context(&text, &t, p).map(|e| t.kind(e).name())
}

#[test]
fn at_point() {
    let cases = [
        ("#+BEGIN_CENTER\n<point>A\n#+END_CENTER", "paragraph"),
        ("| a | b |", "table"),
        ("- item", "plain-list"),
        (
            "#+BEGIN_CENTER\nParagraph\n<point>#+END_CENTER",
            "center-block",
        ),
        ("- Para1\n<point>\n- Para2", "item"),
        ("- Para1\n- Para2\n<point>\nPara3", "plain-list"),
        (
            "* Headline\n- Para1\n- Para2\n<point>\nPara3\n* Another headline",
            "plain-list",
        ),
        ("- a<point>", "paragraph"),
        (
            "- outer\n  #+begin_center\n<point>  - inner\n  #+end_center",
            "plain-list",
        ),
        (":DRAWER:\ntest\n:EN<point>D:\n", "drawer"),
        (":DRA<point>WER:\ntest\n:END:\n", "drawer"),
        (":DRAWER:\ntest\n<point>:END:\n", "drawer"),
        ("* H1\n<point>  \n* H2\n", "headline"),
    ];
    for (s, want) in cases {
        assert_eq!(at_point_type(s), Some(want), "{s:?}");
    }
    // The parent of a paragraph in a block is the block.
    let (text, p) = at("#+BEGIN_CENTER\n<point>A\n#+END_CENTER");
    let t = parse(&text);
    let e = super::at_point(&text, &t, p).unwrap();
    assert_eq!(t.kind(t.parent(e).unwrap()), Type::CenterBlock);
    // Blank lines at the beginning of the buffer: nothing.
    assert_eq!(at_point_type("\n<point>\n* H"), None);
    assert!(at_point_type("* H\n<point>").is_some());
}

#[test]
fn context() {
    let cases: &[(&str, Option<&str>)] = &[
        (
            "Some *text with _under<point>line_ text*",
            Some("underline"),
        ),
        ("* Headline _<point>with_ underlining", Some("underline")),
        ("| a | {<point>{{macro}}} |", Some("macro")),
        ("| a | b<point> {{{macro}}} |", Some("table-cell")),
        ("- *bo<point>ld* ::", Some("bold")),
        ("| a | b |\n|-<point>--|---|\n| c | d |", Some("table-row")),
        ("#+CAPTION: {<point>{{macro}}}\n| a | b |", Some("macro")),
        ("#+caption: *<point>bold*\nParagraph", Some("bold")),
        ("*bold*<point>", Some("bold")),
        ("<<target>><point>{{{test}}}", Some("macro")),
        ("* *bo<point>ld*", Some("bold")),
        ("|a|b|c<point>", Some("table-cell")),
        ("[fn::[[<point>https://orgmode.org]]]", Some("link")),
        ("* Headline :file:tags: :real:tag:", Some("headline")),
        ("* Headline :file<point>:tags: :real:tag:", Some("link")),
    ];
    for (s, want) in cases {
        assert_eq!(context_type(s), *want, "{s:?}");
    }
    assert_ne!(
        context_type("* Headline <2020-04-01>        :tag1<point>:"),
        Some("timestamp")
    );
    assert_ne!(context_type("- *bold* ::<point>"), Some("bold"));
    assert_ne!(context_type("- *bold* ::\n<point>"), Some("bold"));
    assert_ne!(context_type("* Headline :file<point>:tags:"), Some("link"));
    assert_ne!(
        context_type("<point>@@latex:\n\nparagraph\n\n@@"),
        Some("export-snippet")
    );
    let (text, p) = at("Some *bold<point>* text");
    let t = parse(&text);
    let o = super::context(&text, &t, p).unwrap();
    assert_eq!(t.kind(t.parent(o).unwrap()), Type::Paragraph);
}

#[test]
fn lineage_and_map() {
    let text = "* H1\n** H2\n#+BEGIN_CENTER\n*bold*\n#+END_CENTER";
    let t = parse(text);
    let bold = t
        .map_first(t.root, Some(&[Type::Bold]), |_, n| Some(n))
        .unwrap();
    let names = |v: Vec<NodeId>| v.into_iter().map(|n| t.kind(n).name()).collect::<Vec<_>>();
    assert_eq!(
        names(t.lineage(bold, false)),
        [
            "paragraph",
            "center-block",
            "section",
            "headline",
            "headline",
            "org-data"
        ]
    );
    assert_eq!(names(t.lineage(bold, true))[0], "bold");
    assert_eq!(
        t.lineage_of(bold, &[Type::CenterBlock], false)
            .map(|n| t.kind(n)),
        Some(Type::CenterBlock)
    );
    assert_eq!(t.lineage_of(bold, &[Type::ExampleBlock], false), None);
    assert_eq!(t.lineage_of(bold, &[Type::Bold], true), Some(bold));
    let raw = t.lineage_map(bold, None, false, false, |t, n| {
        t.str(n, "raw-value").map(str::to_owned)
    });
    assert_eq!(raw, ["H2", "H1"]);
    let first = t.lineage_map(bold, None, false, true, |t, n| {
        t.str(n, "raw-value").map(str::to_owned)
    });
    assert_eq!(first, ["H2"]);
    assert_eq!(
        t.parent_element(bold).map(|n| t.kind(n)),
        Some(Type::Paragraph)
    );
    // org-element-map: types, first match, no-recursion, affiliated.
    let t = parse("Paragraph.\n\n#+CAPTION: *cap*\n- a\n  - b\n");
    assert_eq!(
        t.map(t.root, Some(&[Type::PlainList]), |_, n| Some(n))
            .len(),
        2
    );
    let top = t.map_with(
        t.root,
        Some(&[Type::PlainList]),
        false,
        &[Type::PlainList],
        false,
        |_, n| Visit::Value(n),
    );
    assert_eq!(top.len(), 1);
    assert!(
        t.map(t.root, Some(&[Type::Bold]), |_, n| Some(n))
            .is_empty()
    );
    assert_eq!(
        t.map_with(t.root, Some(&[Type::Bold]), false, &[], true, |_, n| {
            Visit::Value(n)
        })
        .len(),
        1
    );
    let skipped = t.map_with(
        t.root,
        Some(&[Type::PlainList, Type::Item]),
        false,
        &[],
        false,
        |t, n| {
            if t.kind(n) == Type::PlainList {
                Visit::Skip
            } else {
                Visit::Value(n)
            }
        },
    );
    assert!(skipped.is_empty());
    // Plain text is mappable.
    let texts = t.map(t.root, Some(&[Type::PlainText]), |t, n| {
        t.plain_text(n).map(str::to_owned)
    });
    assert_eq!(texts[0], "Paragraph.\n");
}

#[test]
fn granularity_and_secondary_strings() {
    let text = "* Head 1\n** Head 2\n#+BEGIN_CENTER\nCentered paragraph.\n#+END_CENTER\nParagraph \\alpha.";
    let s = st(text);
    let count = |g: Granularity, ty: Type| {
        let t = parse_buffer_with(
            text,
            &s,
            &Opts {
                granularity: g,
                ..Opts::default()
            },
        );
        t.map(t.root, Some(&[ty]), |_, n| Some(n)).len()
    };
    assert_eq!(count(Granularity::Headline, Type::Headline), 2);
    assert_eq!(count(Granularity::Headline, Type::Paragraph), 0);
    assert_eq!(count(Granularity::GreaterElement, Type::CenterBlock), 1);
    assert_eq!(count(Granularity::GreaterElement, Type::Paragraph), 1);
    assert_eq!(count(Granularity::GreaterElement, Type::Entity), 0);
    assert_eq!(count(Granularity::Element, Type::Paragraph), 2);
    assert_eq!(count(Granularity::Element, Type::Entity), 0);
    assert_eq!(count(Granularity::Object, Type::Entity), 1);
    // Coarse granularity keeps titles and tags as strings.
    let t = parse_buffer_with(
        "* Headline\n- tag :: item",
        &st(""),
        &Opts {
            granularity: Granularity::Element,
            ..Opts::default()
        },
    );
    let h = t
        .map_first(t.root, Some(&[Type::Headline]), |_, n| Some(n))
        .unwrap();
    assert_eq!(t.str(h, "title"), Some("Headline"));
    let i = t
        .map_first(t.root, Some(&[Type::Item]), |_, n| Some(n))
        .unwrap();
    assert_eq!(t.str(i, "tag"), Some("tag"));
    let t = parse("* Headline");
    let h = t
        .map_first(t.root, Some(&[Type::Headline]), |_, n| Some(n))
        .unwrap();
    assert!(t.prop(h, "title").as_secondary().is_some());
    // org-element-parse-secondary-string.
    let t = parse_secondary_string("a *b* c", restriction(Type::Paragraph), &st(""));
    let kinds: Vec<&str> = t
        .contents(t.root)
        .iter()
        .map(|&n| t.kind(n).name())
        .collect();
    assert_eq!(kinds, ["plain-text", "bold", "plain-text"]);
    assert_eq!(interpret(&t, t.root), "a *b* c");
    // Restrictions: no links in link descriptions, only cells in rows.
    assert!(!restriction(Type::Link).contains(&Type::Link));
    assert_eq!(restriction(Type::TableRow), [Type::TableCell]);
    assert!(!restriction(Type::Headline).contains(&Type::LineBreak));
}

#[test]
fn parse_buffer_visible_and_parents() {
    let text = "* H1\n** H2\n** H3 :visible:\n** H4\n** H5 :visible:";
    // Fold H1's body except the visible headlines, as org-occur would.
    let hidden = |a: &str, b: &str| text.find(a).unwrap()..text.find(b).unwrap();
    let opts = Opts {
        invisible: vec![hidden("** H2", "** H3"), hidden("** H4", "** H5")],
        ..Opts::default()
    };
    let t = parse_buffer_with(text, &st(text), &opts);
    let raw = t.map(t.root, Some(&[Type::Headline]), |t, n| {
        t.str(n, "raw-value").map(str::to_owned)
    });
    assert_eq!(raw, ["H1", "H3", "H5"]);
    // :parent of elements, objects, and secondary strings.
    let t = parse("#+BEGIN_CENTER\nText\n#+END_CENTER");
    let p = t
        .map_first(t.root, Some(&[Type::Paragraph]), |_, n| Some(n))
        .unwrap();
    assert_eq!(t.kind(t.parent(p).unwrap()), Type::CenterBlock);
    let t = parse("a_{/b/}");
    let i = t
        .map_first(t.root, Some(&[Type::Italic]), |_, n| Some(n))
        .unwrap();
    assert_eq!(t.kind(t.parent(i).unwrap()), Type::Subscript);
    let t = parse("* /italic/");
    let i = t
        .map_first(t.root, Some(&[Type::Italic]), |_, n| Some(n))
        .unwrap();
    assert_eq!(t.kind(t.parent(i).unwrap()), Type::Headline);
    assert_eq!(t.secondary_p(i), Some("title"));
    assert_eq!(t.class(i), Class::Object);
}

#[test]
fn ast_surgery() {
    // org-element-adopt, extract, insert-before, set, copy.
    let mut t = Tree::new();
    let s = t.text("text");
    let para = t.create(Type::Paragraph, vec![], vec![s]);
    let b = t.create(Type::Bold, vec![], vec![]);
    t.adopt(para, &[b]);
    assert_eq!(t.contents(para), [s, b]);
    assert_eq!(t.parent(b), Some(para));
    let i = t.create(Type::Italic, vec![], vec![]);
    t.insert_before(i, b);
    assert_eq!(t.contents(para), [s, i, b]);
    t.extract(i);
    assert_eq!(t.contents(para), [s, b]);
    assert_eq!(t.parent(i), None);
    let inner = t.text("x");
    let u = t.create(
        Type::Underline,
        vec![("post-blank", Value::Int(1))],
        vec![inner],
    );
    t.set(b, u, &[]);
    assert_eq!(t.kind(b), Type::Underline);
    assert_eq!(t.parent(inner), Some(b));
    assert_eq!(interpret(&t, para), "text_x_ \n");
    let c = t.copy(para, true);
    assert_eq!(t.parent(c), None);
    assert_eq!(interpret(&t, c), interpret(&t, para));
    assert_ne!(t.contents(c)[0], t.contents(para)[0]);
    let shallow = t.copy(para, false);
    assert!(t.contents(shallow).is_empty());
    // Secondary strings are copied with their parent reset.
    let t0 = parse("* /a/ b");
    let mut t = t0.clone();
    let h = t
        .map_first(t.root, Some(&[Type::Headline]), |_, n| Some(n))
        .unwrap();
    let h2 = t.copy(h, false);
    let title = t.secondary(h2, "title").to_vec();
    assert!(title.iter().all(|&o| t.parent(o) == Some(h2)));
    assert!(!t.secondary(h, "title").contains(&title[0]));
    // Extracting from a secondary string.
    let it = title[0];
    t.extract(it);
    assert_eq!(t.secondary(h2, "title").len(), 1);
}

#[test]
fn property_inherited() {
    let mut t = Tree::new();
    let child = t.create(
        Type::Pseudo("child"),
        vec![("foo", Value::sym("baz"))],
        vec![],
    );
    let parent = t.create(
        Type::Pseudo("parent"),
        vec![
            ("foo", Value::sym("bar")),
            (
                "extra",
                Value::List(vec![Value::sym("value"), Value::sym("value2")]),
            ),
        ],
        vec![child],
    );
    let _ = parent;
    assert_eq!(
        t.property_inherited(&["foo"], child, false, false, false, false),
        [Value::sym("bar")]
    );
    assert_eq!(
        t.property_inherited(&["foo"], child, true, false, false, false),
        [Value::sym("baz")]
    );
    assert_eq!(
        t.property_inherited(&["foo"], child, true, true, false, false),
        [Value::sym("bar"), Value::sym("baz")]
    );
    assert_eq!(
        t.property_inherited(&["foo", "extra"], child, false, true, false, false),
        [Value::sym("bar"), Value::sym("value"), Value::sym("value2")]
    );
    let lone = t.create(
        Type::Pseudo("child"),
        vec![("foo", Value::from("nil"))],
        vec![],
    );
    assert!(
        t.property_inherited(&["foo"], lone, true, false, true, false)
            .is_empty()
    );
    assert!(
        !t.property_inherited(&["foo"], lone, true, false, false, false)
            .is_empty()
    );
    // INCLUDE-NIL.
    let g = t.create(
        Type::Pseudo("grandchild"),
        vec![("foo", Value::sym("baz"))],
        vec![],
    );
    let c = t.create(Type::Pseudo("child"), vec![], vec![g]);
    t.put(c, "foo", Value::Nil);
    t.create(
        Type::Pseudo("parent"),
        vec![("foo", Value::sym("bar"))],
        vec![c],
    );
    assert!(t.property_inherited(&["foo"], g, false, false, false, true)[0].is_nil());
    assert_eq!(
        t.property_inherited(&["foo"], g, false, false, false, false),
        [Value::sym("bar")]
    );
    assert_eq!(
        t.property_inherited(&["foo"], g, true, true, false, true),
        [Value::sym("bar"), Value::Nil, Value::sym("baz")]
    );
}

#[test]
fn headline_properties_and_planning() {
    let text = "* TODO [#B] COMMENT Title :a:ARCHIVE:\nSCHEDULED: <2012-03-29 Thu> DEADLINE: <2012-03-30 Fri>\n:PROPERTIES:\n:CUSTOM_ID: x\n:Foo: bar baz\n:END:\nBody";
    let t = parse(text);
    let h = t
        .map_first(t.root, Some(&[Type::Headline]), |_, n| Some(n))
        .unwrap();
    assert_eq!(t.str(h, "todo-keyword"), Some("TODO"));
    assert_eq!(t.str(h, "todo-type"), Some("todo"));
    assert_eq!(t.int(h, "priority"), Some('B' as i64));
    assert!(t.flag(h, "commentedp") && t.flag(h, "archivedp"));
    assert_eq!(t.str(h, "raw-value"), Some("Title"));
    assert_eq!(t.str(h, "CUSTOM_ID"), Some("x"));
    assert_eq!(t.str(h, "FOO"), Some("bar baz"));
    let sched = t.prop(h, "scheduled").as_node().unwrap();
    assert_eq!(t.int(sched, "day-start"), Some(29));
    assert_eq!(
        timestamp_string(&t, t.prop(h, "deadline").as_node().unwrap()),
        "<2012-03-30 Fri>"
    );
    // Custom TODO keywords come from the buffer settings.
    let text = "#+TODO: NEXT | DONE\n* NEXT a\n* TODO b";
    let t = parse(text);
    let kws = t.map(t.root, Some(&[Type::Headline]), |t, n| {
        Some(t.str(n, "todo-keyword").map(str::to_owned))
    });
    assert_eq!(kws, [Some("NEXT".to_owned()), None]);
    // Link abbreviations from #+LINK.
    let text = "#+LINK: gh https://github.com/%s\n[[gh:bzg/org-mode]]";
    let t = parse(text);
    let l = t
        .map_first(t.root, Some(&[Type::Link]), |_, n| Some(n))
        .unwrap();
    assert_eq!(
        (t.str(l, "type"), t.str(l, "path")),
        (Some("https"), Some("//github.com/bzg/org-mode"))
    );
    assert_eq!(
        t.str(l, "raw-link"),
        Some("https://github.com/bzg/org-mode")
    );
}

#[test]
fn byte_offsets_and_helpers() {
    let text = "* Ü\nä *b*\n";
    let t = parse(text);
    let b = t
        .map_first(t.root, Some(&[Type::Bold]), |_, n| Some(n))
        .unwrap();
    assert_eq!(&text[t.begin(b).unwrap()..t.end(b).unwrap()], "*b*");
    assert_eq!(offset(text, 1, 2), text.find(" *b*").unwrap());
    assert_eq!(macro_extract_arguments("a,b\\,c,"), ["a", "b,c", ""]);
    assert_eq!(
        macro_escape_arguments(&["a".into(), "b,c".into()]),
        "a,b\\,c"
    );
    assert_eq!(link_unescape("a\\]b"), "a]b");
    assert_eq!(link_escape("a]b"), "a\\]b");
    assert_eq!(unescape_code(",* a\n,#+b"), "* a\n#+b");
    assert_eq!(escape_code("* a\n#+b"), ",* a\n,#+b");
    assert_eq!(remove_indentation("  a\n    b"), "a\n  b");
    assert_eq!(
        table_align("| a | 1 |\n|-\n| bb | 22 |\n"),
        "| a  |  1 |\n|----+----|\n| bb | 22 |\n"
    );
    assert_eq!(entity("alpha").map(|e| e.6), Some("α"));
    let t = parse("A\n\nB");
    let ps = t.map(t.root, Some(&[Type::Paragraph]), |_, n| Some(n));
    assert!(!nested_p(&t, ps[0], ps[1]));
    let (out, _) = swap_a_b("A\n\nB", &t, ps[0], ps[1]).unwrap();
    assert_eq!(out, "B\n\nA");
}
