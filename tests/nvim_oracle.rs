//! Differential tests: the same keys typed into Fred and into a clean
//! headless Neovim (`tools/nvim-oracle.lua`) must leave the same text,
//! cursor and mode. Skipped when `nvim` isn't installed.
//!
//! A row of `docs/nvim-parity.csv` moves to `done` only once cases here
//! cover it.

use fred::buffer::Buffer;
use fred::editor::{Editor, Mode};
use fred::key::parse_keys;
use std::process::Command;

/// (text, keys) pairs, grouped by what they cover.
const CASES: &[(&str, &str)] = &[
    // Motions.
    ("abc def ghi", "w"),
    ("abc def ghi", "2w"),
    ("abc def ghi", "$b"),
    ("abc def ghi", "e"),
    ("foo.bar baz", "W"),
    ("foo.bar baz", "$B"),
    ("foo.bar baz", "E"),
    ("  indented", "$^"),
    ("abc def", "$0"),
    ("a\nb\nc", "G"),
    ("a\nb\nc", "G2G"),
    ("a\nb\nc", "Ggg"),
    ("a\n\nb\nc\n\nd", "}"),
    ("a\n\nb\nc\n\nd", "G{"),
    ("abcabc", "fc"),
    ("abcabc", "2fc"),
    ("abcabc", "tc"),
    ("abcabc", "$Fa"),
    ("abcabc", "$Ta"),
    ("abcabc", "fb;"),
    ("abcabc", "fb;,"),
    ("one\ntwo\nthree", "j+"),
    ("one\n  two\nthree", "G-"),
    ("abc\nd\nabcdef", "$jj"),
    // Operators.
    ("abc def ghi", "dw"),
    ("abc def ghi", "d2w"),
    ("abc def ghi", "2dw"),
    ("abc def ghi", "de"),
    ("abc def ghi", "wd$"),
    ("abc def ghi", "wd0"),
    ("a\nb\nc\nd", "dd"),
    ("a\nb\nc\nd", "2dd"),
    ("a\nb\nc\nd", "dj"),
    ("a\nb\nc\nd", "jdk"),
    ("a\nb\nc\nd", "dG"),
    ("abc def", "cwxyz<Esc>"),
    ("abc def", "ccnew<Esc>"),
    ("abc def", "yyp"),
    ("abc def", "ywP"),
    ("abc def", "yw$p"),
    ("abc\ndef", "ddp"),
    ("abcdef", "x"),
    ("abcdef", "3x"),
    ("abcdef", "$X"),
    ("abcdef", "lD"),
    ("abcdef", "lCxy<Esc>"),
    ("abcdef", "sX<Esc>"),
    ("abc\ndef", "SX<Esc>"),
    ("abc\ndef", "J"),
    ("abc\ndef\nghi", "3J"),
    ("abcdef", "rx"),
    ("abcdef", "3rx"),
    // Insert mode.
    ("abc", "ixy<Esc>"),
    ("abc", "axy<Esc>"),
    ("  abc", "Ixy<Esc>"),
    ("abc", "Axy<Esc>"),
    ("abc", "oxy<Esc>"),
    ("abc", "Oxy<Esc>"),
    ("abc", "Axy<BS><Esc>"),
    ("abc def", "Ax<C-w><Esc>"),
    ("abc", "ixy"),
    ("abc def", "Axy<C-w><C-w><Esc>"),
    ("abc def", "A<C-w><Esc>"),
    ("abc def", "Axy<C-u><Esc>"),
    ("abc def", "Axy<C-u><C-u><Esc>"),
    ("  abc", "A<C-u><Esc>"),
    ("  abc", "A<C-u><C-u><Esc>"),
    // Undo, redo and repeat.
    ("abc def", "dwu"),
    ("abc def", "dwu<C-r>"),
    ("a b c d", "dw.."),
    ("abc\ndef", "x.j."),
    // Visual mode.
    ("abc def", "vld"),
    ("abc def", "vey$p"),
    ("a\nb\nc", "Vjd"),
    ("a\nb\nc", "vjd"),
    ("abc def", "wvbd"),
    ("abcdef", "lvlohd"),
    ("abc def", "vl"),
    ("a\nb", "Vj"),
    // Select mode: typing replaces, <BS>/<Del> delete, CTRL-G toggles.
    ("abc def", "gh"),
    ("abc def", "gh<Right>x"),
    ("abc def", "ghl"),
    ("abc def", "gh<Right>xy<Esc>"),
    ("abc def", "gh<Right><CR>"),
    ("abc def", "gh<Right><Left>x"),
    ("abc\ndef\nghi", "lgh<Down>x"),
    ("a\nb\nc", "gHx"),
    ("a\nb\nc", "GgHx"),
    ("a\nb\nc", "jgH<Down>x"),
    ("abc def", "gh<Right><BS>"),
    ("abc def", "gh<Right><Del>"),
    ("abc def", "gh<Right><C-h>"),
    ("abc def", "wgh<Right><BS>"),
    ("a\nb\nc", "gH<BS>"),
    ("a\nb\nc", "jgH<Down><BS>"),
    ("ab\ncd\nef", "lgh<Down><Del>"),
    ("abc def", "gh<Right><Esc>"),
    ("abc def", "vl<C-g>"),
    ("abc def", "vl<C-g>x"),
    ("abc def", "gh<Right><C-g>"),
    ("abc def", "gh<Right><C-g>d"),
    ("a\nb", "Vj<C-g>"),
    ("a\nb", "gH<C-g>"),
    // 'keymodel' and 'selectmode'.
    ("abc def", ":set km=startsel slm=key<CR><S-Right>x"),
    ("abc def", ":set km=startsel slm=key<CR><S-Right><S-Right>x"),
    ("abc def", ":set km=startsel slm=key<CR><S-End>x"),
    ("abc def", ":set km=startsel slm=key<CR>l<S-Home>x"),
    ("abc\ndef", ":set km=startsel slm=key<CR><S-Down>x"),
    ("abc def", ":set km=startsel<CR><S-Right>x"),
    (
        "abc def",
        ":set km=startsel,stopsel slm=key<CR><S-Right><Right>x",
    ),
    (
        "abc def",
        ":set km=startsel,stopsel slm=key<CR><S-Right><S-Right><Left>x",
    ),
    (
        "abc def",
        ":set km=startsel,stopsel slm=key<CR><S-Right><Esc>",
    ),
    ("abc def", ":set km=stopsel<CR>vl<Right>x"),
    ("abc def", ":set km=stopsel<CR>gh<Right><Left>x"),
    ("abc def", ":set km=startsel,stopsel slm=key<CR>i<S-Right>"),
    ("abc def", ":set km=startsel,stopsel slm=key<CR>i<S-Right>x"),
    (
        "abc def",
        ":set km=startsel,stopsel slm=key<CR>i<S-Right><Esc>",
    ),
    (
        "abc def",
        ":set km=startsel,stopsel slm=key<CR>i<S-Right><Right>",
    ),
    (
        "abc def",
        ":set km=startsel,stopsel slm=key<CR>i<S-Right><BS>",
    ),
    ("abc def", ":set km=startsel,stopsel slm=key<CR>A<S-Left>"),
    ("abc def", ":set km=startsel,stopsel slm=key<CR>A<S-Left>x"),
    (
        "abc def",
        ":set km=startsel,stopsel slm=key<CR>A<S-Left><Esc>",
    ),
    ("abc def", "gh<Right>xy<Esc>u"),
    ("abc def", ":set slm=cmd<CR>vlx"),
    ("a\nb", ":set slm=cmd<CR>Vx"),
    ("abc def", ":set slm=mouse<CR>vlx"),
    ("abc def", "$gh<BS>"),
    ("abc\ndef", ":set km=startsel slm=key<CR>j<S-Up><BS>"),
    // Marks.
    ("a\nb\nc", "majjd'a"),
    // Search.
    ("foo bar foo", "/foo<CR>"),
    ("foo bar foo", "/foo<CR>n"),
    ("foo bar foo", "$?bar<CR>"),
    ("foo bar foo", "/foo<CR>N"),
    // Ex commands.
    ("a\nb\nc", ":2d<CR>"),
    ("a\nb\nc", ":%s/./X/<CR>"),
    ("a\nb\nc", ":g/b/d<CR>"),
    ("a\nb\nc", ":1m$<CR>"),
    ("a\nb\nc", ":1t.<CR>"),
    // Vim patterns: a bare ( | + { is literal, \( \| \+ \{ are not.
    ("f(x) || g", ":s/(x) ||/[y]/<CR>"),
    (
        "foo bar baz",
        ":s/\\(foo\\|baz\\) \\(\\w\\+\\)/\\2 \\1/<CR>",
    ),
    ("aaa+b", ":s/a\\{2}+/X/<CR>"),
    ("is this is", ":s/\\<is\\>/X/g<CR>"),
    ("a.b axb", ":s/\\Va.b/X/g<CR>"),
    ("ab ab", ":s/\\v(a)(b)/\\2\\1/g<CR>"),
    ("x^y$z", ":s/^x^y$z$/ok/<CR>"),
    ("one\nTwo\nthree", "/\\ctwo<CR>x"),
    ("one\ntwo (2)\nthree", "/(2)<CR>x"),
    // Options: 'tabstop' decides the column `j` keeps below a tab.
    ("\tx\n12345678y", "$j"),
    ("\tx\n12345678y", ":set ts=4<CR>$j"),
    ("\tx\n12345678y", ":se ts=4 ts+=2<CR>$j"),
    ("\tx\n12345678y", ":setl ts=2<CR>$j"),
    ("\tx\n12345678y", ":set ts=2<CR>:set ts&<CR>$j"),
    ("abc", ":set nosuch<CR>x"),
    ("a\nb\nc", ":1,2j<CR>"),
    ("a\n  b\nc", ":1,2j!<CR>"),
    ("a\n  b\n)c", ":%j<CR>"),
    ("a \n  b", "J"),
    // Motions: G and gg keep the column ('nostartofline').
    ("abc\nabc", "lG"),
    ("abc\nabc", "Glgg"),
    ("abc\nabc\nabc", "l2G"),
    // ge gE.
    ("abc def", "wge"),
    ("abc def", "$gE"),
    ("abc def", "wdge"),
    ("a.b c", "$gE"),
    ("abc\n\ndef", "Gge"),
    ("abc def ghi", "$2ge"),
    ("abc", "ge"),
    // H M L.
    ("abc\nabc\nabc", "lL"),
    ("abc\nabc\nabc", "GlH"),
    ("a\nb\nc\nd", "M"),
    ("a\nb\nc", "M"),
    ("a\nb\nc\nd\ne", "M"),
    ("a\nb\nc\nd", "2H"),
    ("a\nb\nc\nd", "2L"),
    ("a\nb\nc\nd", "dL"),
    // | _ g_.
    ("abcdef", "3|"),
    ("abcdef", "d3|"),
    ("abcdef", "$d3|"),
    ("abc\n  abc", "l_"),
    ("abc\n  abc", "l2_"),
    ("abc\n  abc", "d2_"),
    ("abc  ", "g_"),
    ("abc  ", "dg_"),
    ("abc  \nde  ", "2g_"),
    // %.
    ("a (b) c", "%"),
    ("a (b) c", "f)%"),
    ("a (b) c", "d%"),
    ("if (a[1] == b) {", "%"),
    ("f(a, (b), c)", "%"),
    ("f(a, (b), c)", "$%"),
    ("x = \")\" + (y)", "%"),
    ("(\n  a\n)", "%"),
    ("(\n  a\n)", "G%"),
    ("abc", "%"),
    ("a\nb\nc\nd", "50%"),
    ("a\nb\nc\nd", "100%"),
    ("a\nb\nc\nd", "d50%"),
    // Sentences.
    ("One. Two. Three.", ")"),
    ("One. Two. Three.", "2)"),
    ("One. Two. Three.", "$("),
    ("One. Two. Three.", "$2("),
    ("One.  Two!  Three?", "))"),
    ("One.\nTwo.", ")"),
    ("One\n\nTwo.", ")"),
    ("One\n\nTwo.", "))"),
    ("One. Two. Three.", "d)"),
    ("One. Two. Three.", "wwd("),
    ("(One.) Two.", ")"),
    // Sections.
    ("a\n{\nb\n}\n{\nc", "]]"),
    ("a\n{\nb\n}\n{\nc", "2]]"),
    ("a\n{\nb\n}\n{\nc", "G[["),
    ("a\n{\nb\n}\n{\nc", "]["),
    ("a\n{\nb\n}\n{\nc", "G[]"),
    // Unmatched brackets.
    ("f(a, (b), c)", "fb[("),
    ("f(a, (b), c)", "fc[("),
    ("f(a, (b), c)", "fa])"),
    ("{ a { b } c }", "fb[{"),
    ("{ a { b } c }", "fb2[{"),
    ("{ a { b } c }", "fc]}"),
    ("f(a, b)", "fad])"),
    ("f(a, b)", "fbd[("),
    // * # g* g#.
    ("foo bar foo", "*"),
    ("foo bar foo", "$#"),
    ("foo foobar foo", "g*"),
    ("foo foobar foo", "*"),
    ("foo bar foo", "*n"),
    ("foo bar foo", "*N"),
    ("  foo bar foo", "*"),
    ("a.b x a.b", "*"),
    // go.
    ("abc\ndef", "5go"),
    ("abc\ndef", "go"),
    ("abc\ndef", "99go"),
    // Text objects: words.
    ("abc def ghi", "wdiw"),
    ("abc def ghi", "wdaw"),
    ("abc def ghi", "$daw"),
    ("abc def ghi", "wd2aw"),
    ("abc def ghi", "wd3iw"),
    ("abc   def", "llldiw"),
    ("abc   def", "llldaw"),
    ("foo.bar baz", "diW"),
    ("foo.bar baz", "daW"),
    ("foo.bar baz", "ciwX<Esc>"),
    ("abc def", "yiwP"),
    ("  abc", "wdaw"),
    ("abc def.", "wdaw"),
    ("a\nb", "daw"),
    ("abc def ghi", "wviwd"),
    ("abc def ghi", "wvawd"),
    ("abc def ghi", "wviwiwd"),
    ("abc def ghi", "wviwiwiwd"),
    ("abc def ghi", "wvaawd"),
    ("abc def ghi", "wv2iwd"),
    // Sentences.
    ("One. Two. Three.", "wwdis"),
    ("One. Two. Three.", "wwdas"),
    ("One. Two. Three.", "$das"),
    ("One. Two. Three.", "dis"),
    ("One. Two. Three.", "d2as"),
    ("One.  Two.", "4ldis"),
    ("One.  Two.", "4ldas"),
    ("One. Two. Three.", "wwvisd"),
    ("One. Two. Three.", "wwvasd"),
    ("One. Two. Three.", "visisd"),
    // Paragraphs.
    ("a\nb\n\nc\nd", "dip"),
    ("a\nb\n\nc\nd", "dap"),
    ("a\nb\n\nc\nd", "Gdap"),
    ("a\nb\n\nc\nd", "jjdip"),
    ("a\nb\n\nc\nd", "jjdap"),
    ("a\nb\n\nc\nd", "d2ap"),
    ("a\nb\n\nc\nd", "d2ip"),
    ("a\nb\n\nc\nd", "yipP"),
    ("a\nb\n\nc\nd", "vipd"),
    ("a\nb\n\nc\nd", "vapd"),
    ("a\nb\n\nc\nd\n\ne", "vipipd"),
    ("a\nb\n\nc\nd\n\ne", "Vapapd"),
    ("a\n  \nb", "jdip"),
    // Brackets.
    ("f(a, b)", "fadi("),
    ("f(a, b)", "fada("),
    ("f(a, b)", "fadib"),
    ("f(a, b)", "fadi)"),
    ("f(a, (b), c)", "fbdi("),
    ("f(a, (b), c)", "fbd2i("),
    ("f(a, (b), c)", "fbda("),
    ("f()", "di("),
    ("f()", "f(di("),
    ("f() x", "$di("),
    ("x (a) y", "di("),
    ("{\n  a\n}", "jdiB"),
    ("{\n  a\n}", "jdaB"),
    ("{\n  a\n}", "di{"),
    ("if x {\n  a\n  b\n}", "jdi{"),
    ("[1, [2], 3]", "f2di["),
    ("[1, [2], 3]", "f2da]"),
    ("<a <b> c>", "fbdi<"),
    ("<a <b> c>", "fbda>"),
    ("f(a, b)", "faci(x<Esc>"),
    ("f(a, b)", "fayi(P"),
    ("f(a, (b), c)", "fbvi(d"),
    ("f(a, (b), c)", "fbvi(i(d"),
    ("f(a, (b), c)", "fbva(d"),
    ("f(a, (b), c)", "fbva(a(d"),
    ("f(\\(a), b)", "fadi("),
    // Quotes.
    ("x = \"abc\" y", "fbdi\""),
    ("x = \"abc\" y", "fbda\""),
    ("x = \"abc\"", "fbda\""),
    ("x = 'abc' y", "fbdi'"),
    ("x = `abc` y", "fbdi`"),
    ("x = \"abc\" y", "di\""),
    ("x = \"abc\" \"def\"", "fedi\""),
    ("x = \"abc\" \"def\"", "f\"di\""),
    ("x = \"abc\" \"def\"", "2f\"di\""),
    ("x = \"a\\\"bc\" y", "fbdi\""),
    ("x = \"\" y", "f\"di\""),
    ("x = \"abc\" y", "fbci\"z<Esc>"),
    ("x = \"abc\" y", "fbd2i\""),
    ("x = \"abc\" y", "fbvi\"d"),
    ("x = \"abc\" y", "fbva\"d"),
    ("x = \"abc\" y", "fbvi\"i\"d"),
    ("no quotes", "di\""),
    // Operators: shift.
    ("abc\ndef", ">>"),
    ("abc\ndef", "2>>"),
    ("abc\ndef", ">j"),
    ("\tabc", "<<"),
    ("  abc", "<<"),
    ("abc\n\ndef", ">2j"),
    ("abc", ":set sw=4 et<CR>>>"),
    ("abc", ":set sw=4<CR>>>>>"),
    ("abc", ":set sw=0 ts=4<CR>>>"),
    ("abc\ndef", "Vj>"),
    ("abc\ndef", "vj>"),
    ("  abc\n  def", "Vj<"),
    ("abc\ndef", ">>j."),
    ("a\nb\nc\nd", ">ip"),
    // Operators: case.
    ("abc def", "g~w"),
    ("abc def", "g~~"),
    ("abc def", "g~g~"),
    ("abc def", "gUw"),
    ("abc def", "gUU"),
    ("abc def", "wgUU"),
    ("abc def", "gUgU"),
    ("ABC DEF", "guw"),
    ("ABC DEF", "guu"),
    ("ABC DEF", "gugu"),
    ("abc def", "g?w"),
    ("abc def", "g??"),
    ("abc def", "g?g?"),
    ("abc\ndef", "gUj"),
    ("abc\ndef", "jlgUk"),
    ("abc def", "gUiw"),
    ("abc def", "wgUaw"),
    ("abc def", "gUe."),
    ("abc def", "vegU"),
    ("abc def", "veU"),
    ("ABC def", "veu"),
    ("abc def", "ve~"),
    ("abc\ndef", "VjU"),
    ("abc def", "veg?"),
    ("abc def", "~"),
    ("abc def", "3~"),
    ("abc def", "$~"),
    ("abc", "10~"),
    ("abc def", "~~."),
    ("", "~"),
    // gJ.
    ("abc\n  def", "gJ"),
    ("abc\ndef\nghi", "3gJ"),
    ("abc\n  def", "VjgJ"),
    // CTRL-H CTRL-J.
    ("abc", "$<C-h>"),
    ("a\nb", "<C-j>"),
];

#[derive(Debug, PartialEq)]
struct State {
    text: String,
    line: usize,
    col: usize,
    mode: String,
}

fn fred(text: &str, keys: &str) -> State {
    let mut ed = Editor::new(Buffer::from_text(text));
    for k in parse_keys(keys) {
        ed.handle_key(k);
    }
    let mode = match ed.mode {
        Mode::Normal => "n",
        Mode::Insert => "i",
        Mode::Visual { .. } if ed.select_mode() => "s",
        Mode::VisualLine { .. } if ed.select_mode() => "S",
        Mode::Visual { .. } => "v",
        Mode::VisualLine { .. } => "V",
        Mode::Command(_) => "c",
        Mode::Pick(_) => "pick",
    };
    State {
        text: ed.buf.text(),
        line: ed.cur.line,
        col: ed.cur.byte,
        mode: mode.into(),
    }
}

/// None when `nvim` isn't installed.
fn nvim(text: &str, keys: &str) -> Option<State> {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/nvim-oracle.lua");
    let out = Command::new("nvim")
        .args(["--clean", "--headless", "-l", script, text, keys])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let last = stdout.lines().last().unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(last)
        .unwrap_or_else(|e| panic!("nvim oracle for {keys:?}: {e}: {stdout}"));
    Some(State {
        text: v["text"].as_str().unwrap_or_default().into(),
        line: v["line"].as_u64().unwrap_or_default() as usize,
        col: v["col"].as_u64().unwrap_or_default() as usize,
        mode: v["mode"].as_str().unwrap_or_default().into(),
    })
}

#[test]
fn fred_matches_neovim() {
    let results: Vec<_> = std::thread::scope(|s| {
        let jobs: Vec<_> = CASES
            .iter()
            .map(|&(text, keys)| s.spawn(move || (text, keys, nvim(text, keys))))
            .collect();
        jobs.into_iter().map(|j| j.join().unwrap()).collect()
    });
    let mut wrong = vec![];
    for (text, keys, want) in results {
        let Some(want) = want else {
            eprintln!("nvim not found; skipping the Neovim oracle");
            return;
        };
        let got = fred(text, keys);
        if got != want {
            wrong.push(format!(
                "{text:?} + {keys:?}\n  fred: {got:?}\n  nvim: {want:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ from Neovim:\n{}",
        wrong.len(),
        CASES.len(),
        wrong.join("\n")
    );
}
