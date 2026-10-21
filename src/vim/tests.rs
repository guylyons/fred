//! Behavior tests for vim editing commands.

use crate::editor::Mode;
use crate::editor::tests::ed;

fn txt(t: &str, keys: &str) -> String {
    ed(t, keys).buf.text()
}

#[test]
fn operators() {
    assert_eq!(txt("abc def ghi", "dw"), "def ghi");
    assert_eq!(txt("abc def ghi", "2dw"), "ghi");
    assert_eq!(txt("abc def ghi", "d2w"), "ghi");
    assert_eq!(txt("abc def", "wdw"), "abc ");
    assert_eq!(txt("abc def\nghi", "wdw"), "abc \nghi");
    assert_eq!(txt("abc def", "cwxy<Esc>"), "xy def");
    assert_eq!(txt("abc def", "ce<Esc>"), " def");
    assert_eq!(txt("abc def", "wdb"), "def");
    assert_eq!(txt("a\nb\nc", "dd"), "b\nc");
    assert_eq!(txt("a\nb\nc", "jdd"), "a\nc");
    assert_eq!(txt("a\nb\nc", "Gdd"), "a\nb");
    assert_eq!(txt("a\nb\nc", "2ddp"), "c\na\nb");
    assert_eq!(txt("a\nb\nc", "yyjp"), "a\nb\na\nc");
    assert_eq!(txt("a\nb\nc", "yyP"), "a\na\nb\nc");
    assert_eq!(txt("a\nb\nc", "dj"), "c");
    assert_eq!(txt("a\nb\nc", "Gdk"), "a");
    assert_eq!(txt("a\nb\nc", "dG"), "");
    assert_eq!(txt("a\nb\nc", "jdgg"), "c");
    assert_eq!(txt("abc", "x"), "bc");
    assert_eq!(txt("abc", "2x"), "c");
    assert_eq!(txt("abc", "$x"), "ab");
    assert_eq!(txt("abc", "$X"), "ac");
    assert_eq!(txt("abc", "rZ"), "Zbc");
    assert_eq!(txt("abc", "2rZ"), "ZZc");
    assert_eq!(txt("abc", "5rZ"), "abc");
    assert_eq!(txt("a\nb", "J"), "a b");
    assert_eq!(txt("a\n   b", "J"), "a b");
    assert_eq!(txt("a\nb\nc", "3J"), "a b c");
    assert_eq!(txt("abc def", "D"), "");
    assert_eq!(txt("abc def", "wD"), "abc ");
    assert_eq!(txt("abc def", "wCx<Esc>"), "abc x");
    assert_eq!(txt("a,b,c", "dt,"), ",b,c");
    assert_eq!(txt("a,b,c", "df,"), "b,c");
    assert_eq!(txt("a,b,c", "$dF,"), "a,bc");
    assert_eq!(txt("abc", "$d0"), "c");
    assert_eq!(txt("  abc", "$d^"), "  c");
    assert_eq!(txt("abc def", "d$"), "");
    assert_eq!(txt("abc def", "y$P"), "abc defabc def");
    assert_eq!(txt("abc def", "ywP"), "abc abc def");
    assert_eq!(txt("abc", "xp"), "bac");
    assert_eq!(txt("x", "dd"), "");
    assert_eq!(txt("e\u{301}漢x", "x"), "漢x");
    assert_eq!(txt("abc", "sX<Esc>"), "Xbc");
    assert_eq!(txt("  abc\nd", "Sx<Esc>"), "  x\nd");
    assert_eq!(txt("  abc\nd", "ccx<Esc>"), "  x\nd");
    assert_eq!(txt("a\nb\nc", "cjx<Esc>"), "x\nc");
    assert_eq!(txt("p1\n\np2", "d}"), "\np2");
}

#[test]
fn register_put() {
    let e = ed("one two", "dwP");
    assert_eq!(e.buf.text(), "one two");
    let e = ed("a\nb", "yy2p");
    assert_eq!(e.buf.text(), "a\na\na\nb");
    assert_eq!(e.cur.pos(), (1, 0));
    let e = ed("ab", "yl3p");
    assert_eq!(e.buf.text(), "aaaab");
}

#[test]
fn insert_and_undo() {
    let e = ed("abc", "ifoo<Esc>");
    assert_eq!(e.buf.text(), "fooabc");
    assert_eq!(e.cur.pos(), (0, 2));
    assert_eq!(e.mode, Mode::Normal);
    assert_eq!(txt("abc", "ifoo<Esc>u"), "abc");
    assert_eq!(txt("abc", "ifoo<Esc>u<C-r>"), "fooabc");
    assert_eq!(txt("abc", "ax<Esc>"), "axbc");
    assert_eq!(txt("abc", "Ax<Esc>"), "abcx");
    assert_eq!(txt("  abc", "Ix<Esc>"), "  xabc");
    assert_eq!(txt("  ab", "A<Enter>x<Esc>"), "  ab\n  x");
    assert_eq!(txt("ab\ncd", "jI<BS><Esc>"), "abcd");
    assert_eq!(txt("abc", "A<BS><BS><Esc>"), "a");
    assert_eq!(txt("ab", "ox<Esc>Oy<Esc>"), "ab\ny\nx");
    assert_eq!(txt("  ab", "ox<Esc>"), "  ab\n  x");
    assert_eq!(txt("ab", "i<Del><Esc>"), "b");
    assert_eq!(txt("foo bar", "A<C-w><Esc>"), "foo ");
    assert_eq!(
        txt("x", "i<Tab><Esc>"),
        "\tx",
        "unindented files get real tabs"
    );
    assert_eq!(txt("\tx", "A<Enter><Tab>y<Esc>"), "\tx\n\t\ty");
    assert_eq!(txt("e\u{301}x", "A<BS><BS><Esc>"), "");
    // one undo step for a change with its insert session
    assert_eq!(txt("abc def", "cwxy<Esc>u"), "abc def");
    assert_eq!(txt("abc", "ix<Esc>iy<Esc>u"), "xabc");
    assert_eq!(txt("a", "ix<Left>y<Esc>"), "yxa");
}

#[test]
fn modified_flag_follows_undo() {
    let e = ed("abc", "x");
    assert!(e.buf.modified);
    let e = ed("abc", "xu");
    assert!(!e.buf.modified);
}

#[test]
fn dot_repeat() {
    assert_eq!(txt("a b c d", "dw."), "c d");
    assert_eq!(txt("a\nb\nc", "Ax<Esc>j."), "ax\nbx\nc");
    assert_eq!(txt("aaaa", "x3."), "");
    assert_eq!(txt("ab cd ef", "cwX<Esc>w."), "X X ef");
    assert_eq!(txt("a\nb\nc\nd", "dd."), "c\nd");
    assert_eq!(txt("a\nb", "ox<Esc>."), "a\nx\nx\nb");
    assert_eq!(txt("abc", "x.u"), "bc");
}

#[test]
fn visual_line() {
    assert_eq!(txt("a\nb\nc", "Vjd"), "c");
    assert_eq!(txt("a\nb\nc", "jVkd"), "c");
    assert_eq!(txt("a\nb\nc", "Vjyjjp"), "a\nb\nc\na\nb");
    assert_eq!(txt("a\nb\nc", "Vj:s/$/!/<Enter>"), "a!\nb!\nc");
    assert_eq!(txt("a\nb\nc", "VjJ"), "a b\nc");
    assert_eq!(txt("a\nb\nc", "Vjcx<Esc>"), "x\nc");
    assert_eq!(ed("a\nb", "V<Esc>").mode, Mode::Normal);
    assert_eq!(txt("a\nb\nc", "VGx"), "");
}

#[test]
fn visual_chars() {
    assert_eq!(txt("hello world", "lvlld"), "ho world");
    assert_eq!(txt("hello world", "wvbd"), "orld");
    assert_eq!(txt("ab\ncd\nef", "lvjd"), "a\nef");
    assert_eq!(txt("hello", "vlyP"), "hehello");
    assert_eq!(txt("hello", "lvlcX<Esc>"), "hXlo");
    assert_eq!(txt("hello", "lvlohd"), "lo");
    assert_eq!(txt("a\n\nb", "jvd"), "a\nb");
    assert_eq!(txt("a\nb\nc", "vjD"), "c");
    assert_eq!(txt("a\nb\nc", "vjVd"), "c");
    assert_eq!(txt("abc\nd", "lVvd"), "c\nd");
    assert_eq!(ed("ab", "vl<Esc>").mode, Mode::Normal);
    assert_eq!(ed("ab", "vlv").mode, Mode::Normal);
    assert_eq!(txt("abc", "vlud"), "c");
}

#[test]
fn marks_and_ex() {
    assert_eq!(txt("a\nb\nc\nd", "majjmb:'a,'bd<Enter>"), "d");
    assert_eq!(ed("a\nb", ":2<Enter>").cur.pos(), (1, 0));
    assert!(ed("a", ":zz<Enter>").msg.unwrap().1);
    assert_eq!(txt("a\nb", ":,d<Enter>u"), "a\nb");
    let e = ed("a", ":w<Enter>");
    assert!(e.pending_effect.is_some());
}

#[test]
fn counts_dont_hang() {
    assert_eq!(txt("abc", "999999x"), "");
    assert_eq!(txt("a\nb", "99999dd"), "");
    assert_eq!(txt("ab", "99999p"), "ab");
}

#[test]
fn paste_inserts_text_verbatim() {
    // Insert mode: no autoindent, no completion popup, CRLF normalized.
    let mut e = ed("  ab", "A");
    e.paste("x\r\n  y");
    assert_eq!(e.buf.text(), "  abx\n  y");
    assert!(e.popup.is_none());
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Esc));
    e.handle_key(crate::key::Key::ch('u'));
    assert_eq!(
        e.buf.text(),
        "  ab",
        "the paste and the insert session are one undo step"
    );
    // Normal mode: inserted at the cursor as one undo step, stays in Normal.
    let mut e = ed("ab", "l");
    e.paste("XY");
    assert_eq!(e.buf.text(), "aXYb");
    assert_eq!(e.mode, Mode::Normal);
    e.handle_key(crate::key::Key::ch('u'));
    assert_eq!(e.buf.text(), "ab");
    // Command line: first line only.
    let mut e = ed("ab", ":");
    e.paste("s/a/b/\nignored");
    match &e.mode {
        Mode::Command(cl) => assert_eq!(cl.text, "s/a/b/"),
        m => panic!("{m:?}"),
    }
}

fn cmdline(e: &crate::editor::Editor) -> String {
    match &e.mode {
        Mode::Command(cl) => cl.text.clone(),
        m => panic!("not on the command line: {m:?}"),
    }
}

#[test]
fn cmdline_ctrl_w_handles_multibyte() {
    assert_eq!(
        cmdline(&ed("a", ":s/a/→<C-w>")),
        "s/a",
        "a punctuation run is one word"
    );
    assert_eq!(cmdline(&ed("a", "/“<C-w>")), "");
    assert_eq!(cmdline(&ed("a", ":foo bar<C-w>")), "foo ");
    assert_eq!(cmdline(&ed("a", ":foo  <C-w>")), "");
    assert_eq!(cmdline(&ed("a", ":a+-<C-w>")), "a");
    assert_eq!(txt("a →→", "A<C-w><Esc>"), "a ");
}

#[test]
fn form_feed_and_lone_cr_lines() {
    assert_eq!(txt("a\x0cb\nc\nd", ":2d<Enter>"), "a\x0cb\nd");
    assert_eq!(txt("a\x0cb\nc", "ccX<Esc>"), "X\nc");
    assert_eq!(txt("a\nhello\r", "yyGp"), "a\nhello\r\na");
}

fn bytes(t: &str, keys: &str) -> String {
    String::from_utf8(ed(t, keys).buf.to_bytes()).unwrap()
}

#[test]
fn missing_final_newline_is_kept() {
    assert_eq!(bytes("abc", "ddu"), "abc");
    assert_eq!(bytes("s3cr3t", "ccnew<Esc>"), "new");
    assert_eq!(bytes("x", "rZ"), "Z");
    assert_eq!(bytes("a\nb", "Gdd"), "a");
    // A zero-byte file that gains text gets a final newline, like vim;
    // undoing back to nothing writes nothing again.
    assert_eq!(bytes("", "ihi<Esc>"), "hi\n");
    assert_eq!(bytes("", "ihi<Esc>u"), "");
    assert_eq!(bytes("", ""), "");
}

#[test]
fn marks_follow_their_lines() {
    // A line inserted above shifts the mark down.
    assert_eq!(
        txt("a\nb\nc\nd\ne", "jjjmaggOnew<Esc>:'a,$d<Enter>"),
        "new\na\nb\nc"
    );
    // A line deleted above shifts it up.
    assert_eq!(ed("a\nb\nc\nd", "GmaggddG'a").cur.line, 2);
    // Deleting the marked line deletes the mark.
    let e = ed("a\nb\nc", "jmadd:'a<Enter>");
    assert!(
        e.msg
            .as_ref()
            .is_some_and(|m| m.1 && m.0.contains("mark not set")),
        "{:?}",
        e.msg
    );
    // Splitting a line below the mark doesn't move it; o above it does.
    assert_eq!(ed("a\nb\nc", "jmaggA<Enter>x<Esc>G'a").cur.line, 2);
    assert_eq!(ed("a\nb\nc", "jmaggox<Esc>G'a").cur.line, 2);
    // ex commands move marks too.
    assert_eq!(ed("a\nb\nc\nd", "Gma:1d<Enter>gg'a").cur.line, 2);
}

#[test]
fn tab_follows_the_file_indentation() {
    assert_eq!(
        txt("all:\n", "o<Tab>echo<Esc>"),
        "all:\n\techo",
        "Makefiles need tabs"
    );
    assert_eq!(txt("\ta", "o<Tab>x<Esc>"), "\ta\n\t\tx");
    assert_eq!(txt("a\n    b", "o<Tab>x<Esc>"), "a\n    x\n    b");
    assert_eq!(
        txt("a\n  b\n    c", "o<Tab>x<Esc>"),
        "a\n  x\n  b\n    c",
        "2-space files indent by 2"
    );
    assert_eq!(
        txt("a\n  b", "A<Tab>x<Esc>"),
        "a x\n  b",
        "Tab mid-line pads to the next indent stop"
    );
}

#[test]
fn operators_with_search_motions() {
    assert_eq!(txt("abc foo bar\nx", "d/foo<Enter>"), "foo bar\nx");
    assert_eq!(txt("abc foo bar", "c/bar<Enter>X<Esc>"), "Xbar");
    assert_eq!(txt("one two three", "$d?two<Enter>"), "one e");
    assert_eq!(txt("abc foo", "y/foo<Enter>P"), "abc abc foo");
    assert_eq!(txt("abc foo", "d/foo<Enter>u"), "abc foo");
    let e = ed("abc foo", "d/foo<Esc>");
    assert_eq!(e.buf.text(), "abc foo");
    assert_eq!(e.mode, Mode::Normal);
    assert_eq!(txt("abc foo", "d/zzz<Enter>"), "abc foo");
    assert_eq!(
        txt("abc foo", "x"),
        "bc foo",
        "a cancelled operator leaves no state behind"
    );
}

#[test]
fn counted_search_lands_on_the_right_match() {
    // Matches on lines 0, 1, 2; after /a the cursor is on line 1, so the
    // k-th next match is on line (1 + k) % 3.
    for (count, line) in [(1, 2), (3, 1), (5, 0), (1_000_000, 2)] {
        assert_eq!(
            ed("a\na\na", &format!("/a<Enter>{count}n")).cur.line,
            line,
            "{count}n"
        );
    }
    // G, then /a wraps to line 0; four matches back: 2, 1, 0, 2.
    assert_eq!(
        ed("a\na\na", "G/a<Enter>4N").cur.line,
        2,
        "N counts backwards"
    );
    assert_eq!(
        txt("x a b a c a", "d2/a<Enter>"),
        "a c a",
        "operator with a counted search"
    );
}

#[test]
fn typing_before_a_combining_mark_keeps_the_cursor_on_a_boundary() {
    // "e" typed before a lone combining accent becomes "é"; the next key
    // goes after it instead of splitting it.
    assert_eq!(txt("\u{301}", "iex<Esc>"), "e\u{301}x");
}

#[test]
fn jj_leaves_insert() {
    let e = ed("abc", "ifoojj");
    assert_eq!(
        (e.buf.text().as_str(), e.mode, e.cur.pos()),
        ("fooabc", Mode::Normal, (0, 2))
    );
    assert_eq!(txt("abc", "ixjju"), "abc");
    assert_eq!(txt("abc", "ixjj."), "xxabc");
    assert_eq!(txt("abc", "ijkj<Esc>"), "jkjabc");
    // A `j` from an earlier insert doesn't pair with the next one.
    assert_eq!(ed("abc", "ij<Esc>ij").mode, Mode::Insert);
}

#[test]
fn space_enter_saves_and_space_semicolon_closes() {
    use crate::ex::ExEffect;
    let e = ed("abc", " <Enter>");
    assert!(
        matches!(
            e.pending_effect,
            Some(ExEffect::Write {
                path: None,
                force: false,
                range: None,
                then_quit: false
            })
        ),
        "{:?}",
        e.pending_effect
    );
    assert_eq!(
        ed("abc", " ;").pending_effect,
        Some(ExEffect::Quit { force: false })
    );
    assert_eq!(ed("abc", " x").pending_effect, None);
}

/// Text, cursor and Neovim's mode letter (`s`/`S` for Select).
fn sel(t: &str, keys: &str) -> (String, (usize, usize), &'static str) {
    let e = ed(t, keys);
    let mode = match (&e.mode, e.vim.select) {
        (Mode::Visual { .. }, true) => "s",
        (Mode::VisualLine { .. }, true) => "S",
        (Mode::Visual { .. }, false) => "v",
        (Mode::VisualLine { .. }, false) => "V",
        (Mode::Insert, _) => "i",
        (Mode::Normal, _) => "n",
        _ => "?",
    };
    (e.buf.text(), e.cur.pos(), mode)
}

#[test]
fn select_mode_typing_replaces() {
    assert_eq!(sel("abc def", "gh"), ("abc def".into(), (0, 0), "s"));
    assert_eq!(sel("abc def", "gh<Right>x"), ("xc def".into(), (0, 1), "i"));
    assert_eq!(
        sel("abc def", "gh<Right>xy<Esc>"),
        ("xyc def".into(), (0, 1), "n")
    );
    // `l` is printable: it replaces, it doesn't move.
    assert_eq!(sel("abc def", "ghl"), ("lbc def".into(), (0, 1), "i"));
    assert_eq!(
        sel("abc def", "gh<Right><CR>"),
        ("\nc def".into(), (1, 0), "i")
    );
    // Linewise Select replaces like a charwise one (the line break goes).
    assert_eq!(sel("a\nb\nc", "gHx"), ("xb\nc".into(), (0, 1), "i"));
    assert_eq!(sel("a\nb\nc", "GgHx"), ("a\nb\nx".into(), (2, 1), "i"));
    assert_eq!(sel("a\nb\nc", "jgH<Down>x"), ("a\nx".into(), (1, 1), "i"));
}

#[test]
fn select_mode_bs_and_del_delete() {
    assert_eq!(
        sel("abc def", "gh<Right><BS>"),
        ("c def".into(), (0, 0), "n")
    );
    assert_eq!(
        sel("abc def", "gh<Right><Del>"),
        ("c def".into(), (0, 0), "n")
    );
    assert_eq!(
        sel("abc def", "wgh<Right><BS>"),
        ("abc f".into(), (0, 4), "n")
    );
    assert_eq!(sel("a\nb\nc", "gH<BS>"), ("b\nc".into(), (0, 0), "n"));
    assert_eq!(sel("a\nb\nc", "jgH<Down><BS>"), ("a".into(), (0, 0), "n"));
    assert_eq!(
        sel("ab\ncd\nef", "lgh<Down><Del>"),
        ("a\nef".into(), (0, 0), "n")
    );
}

#[test]
fn ctrl_g_toggles_visual_and_select() {
    assert_eq!(sel("abc def", "vl<C-g>"), ("abc def".into(), (0, 1), "s"));
    assert_eq!(sel("abc def", "vl<C-g>x"), ("xc def".into(), (0, 1), "i"));
    assert_eq!(
        sel("abc def", "gh<Right><C-g>"),
        ("abc def".into(), (0, 1), "v")
    );
    assert_eq!(
        sel("abc def", "gh<Right><C-g>d"),
        ("c def".into(), (0, 0), "n")
    );
    assert_eq!(sel("a\nb", "Vj<C-g>"), ("a\nb".into(), (1, 0), "S"));
    assert_eq!(sel("a\nb", "gH<C-g>"), ("a\nb".into(), (0, 0), "V"));
    assert_eq!(
        sel("abc def", "gh<Right><Esc>"),
        ("abc def".into(), (0, 1), "n")
    );
    // Arrows without keymodel move and extend.
    assert_eq!(
        sel("abc def", "gh<Right><Left>x"),
        ("xbc def".into(), (0, 1), "i")
    );
}

#[test]
fn keymodel_startsel_and_stopsel() {
    let km = ":set km=startsel,stopsel slm=key<CR>";
    assert_eq!(
        sel("abc def", &format!("{km}<S-Right>x")),
        ("xc def".into(), (0, 1), "i")
    );
    assert_eq!(
        sel("abc def", &format!("{km}<S-Right><S-Right>x")),
        ("x def".into(), (0, 1), "i")
    );
    assert_eq!(
        sel("abc def", &format!("{km}<S-End>x")),
        ("x".into(), (0, 1), "i")
    );
    assert_eq!(
        sel("abc\ndef", &format!("{km}<S-Down>x")),
        ("xef".into(), (0, 1), "i")
    );
    assert_eq!(
        sel("abc def", &format!("{km}l<S-Home>x")),
        ("xc def".into(), (0, 1), "i")
    );
    // stopsel: an unshifted arrow ends the selection, then moves.
    assert_eq!(
        sel("abc def", &format!("{km}<S-Right><Right>x")),
        ("ab def".into(), (0, 2), "n")
    );
    assert_eq!(
        sel("abc def", &format!("{km}<S-Right><S-Right><Left>x")),
        ("ac def".into(), (0, 1), "n")
    );
    // ... and Visual mode too.
    assert_eq!(
        sel("abc def", ":set km=stopsel<CR>vl<Right>x"),
        ("ab def".into(), (0, 2), "n")
    );
    // startsel without `key` in 'selectmode' starts Visual.
    assert_eq!(
        sel("abc def", ":set km=startsel<CR><S-Right>x"),
        ("c def".into(), (0, 0), "n")
    );
    // From Insert mode, the selection returns to Insert when it ends.
    assert_eq!(
        sel("abc def", &format!("{km}i<S-Right>")),
        ("abc def".into(), (0, 1), "s")
    );
    assert_eq!(
        sel("abc def", &format!("{km}i<S-Right>x")),
        ("xc def".into(), (0, 1), "i")
    );
    assert_eq!(
        sel("abc def", &format!("{km}i<S-Right><Esc>")),
        ("abc def".into(), (0, 1), "i")
    );
    assert_eq!(
        sel("abc def", &format!("{km}i<S-Right><Right>")),
        ("abc def".into(), (0, 2), "i")
    );
    assert_eq!(
        sel("abc def", &format!("{km}i<S-Right><BS>")),
        ("c def".into(), (0, 0), "i")
    );
}

#[test]
fn select_mode_label() {
    let e = ed("a\nb", "gh");
    assert_eq!(crate::ui::render::status_label(&e), "SELECT");
    let e = ed("a\nb", "gH");
    assert_eq!(crate::ui::render::status_label(&e), "SELECT LINE");
}
