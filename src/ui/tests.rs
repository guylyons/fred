use super::*;
use crate::buffer::Buffer;
use crate::complete::Popup;
use crate::config::Config;
use crate::editor::{Editor, Mode};
use crate::highlight::Highlighter;
use crate::key::parse_keys;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use std::time::Duration;

struct Screen {
    term: Terminal<TestBackend>,
    view: View,
    hl: Highlighter,
    cfg: Config,
}

impl Screen {
    fn new(w: u16, h: u16) -> Screen {
        Screen {
            term: Terminal::new(TestBackend::new(w, h)).unwrap(),
            view: View::default(),
            hl: Highlighter::new("ansi", false).unwrap(),
            cfg: Config::default(),
        }
    }
    fn draw(&mut self, ed: &Editor) {
        let Screen {
            term,
            view,
            hl,
            cfg,
        } = self;
        term.draw(|f| draw(f, ed, view, hl, cfg, Duration::from_secs(1)))
            .unwrap();
    }
    fn row(&self, y: u16) -> String {
        let b = self.term.backend().buffer();
        let mut s = String::new();
        let mut x = 0;
        while x < b.area.width {
            let c = &b[(x, y)];
            s.push_str(c.symbol());
            // The cell after a wide character is a placeholder.
            x += unicode_width::UnicodeWidthStr::width(c.symbol()).max(1) as u16;
        }
        s.trim_end().to_string()
    }
    fn cursor(&mut self) -> (u16, u16) {
        let p = self.term.get_cursor_position().unwrap();
        (p.x, p.y)
    }
}

fn editor(text: &str, keys: &str) -> Editor {
    let mut e = Editor::new(Buffer::from_text(text));
    for k in parse_keys(keys) {
        e.handle_key(k);
    }
    e
}

#[test]
fn renders_gutter_status_and_cursor() {
    let mut s = Screen::new(30, 5);
    let e = editor("a\nb\nc", "");
    s.draw(&e);
    assert_eq!(s.row(0), "  1 a");
    assert_eq!(s.row(2), "  3 c");
    assert!(s.row(3).contains("NORMAL"), "{}", s.row(3));
    assert!(s.row(3).contains("[No Name]"), "{}", s.row(3));
    assert!(s.row(3).contains("1:1"), "{}", s.row(3));
    assert_eq!(s.row(4), "");
    assert_eq!(s.cursor(), (4, 0));
}

#[test]
fn zap_highlights_only_the_typed_prefix() {
    for (text, query, last_match, prefix_width) in [
        ("reload", "re", 1, 2),
        ("漢字語", "漢字", 2, 4),
        ("e\u{301}lan", "e\u{301}l", 1, 2),
    ] {
        let mut s = Screen::new(20, 3);
        let mut e = editor(text, "");
        e.run_ex("set nonumber");
        s.draw(&e);
        e.viewport = Some(crate::zap::Viewport {
            view: s.view,
            rows: 1,
            cols: 20,
            wrap: false,
        });
        for k in parse_keys(&format!(" s{query}")) {
            e.handle_key(k);
        }
        s.draw(&e);
        let b = s.term.backend().buffer();
        assert_eq!(b[(0, 0)].bg, ratatui::style::Color::Magenta);
        assert_eq!(b[(last_match, 0)].bg, ratatui::style::Color::Yellow);
        assert_ne!(
            b[(prefix_width, 0)].bg,
            ratatui::style::Color::Yellow,
            "{text}"
        );
    }
}

#[test]
fn zap_labels_at_a_narrow_edge_reveal_the_next_selection_key() {
    let mut s = Screen::new(8, 62);
    let mut e = editor(&vec!["       x"; 60].join("\n"), "");
    e.run_ex("set nonumber");
    s.draw(&e);
    e.viewport = Some(crate::zap::Viewport {
        view: s.view,
        rows: 60,
        cols: 8,
        wrap: false,
    });
    for k in parse_keys(" sx") {
        e.handle_key(k);
    }
    s.draw(&e);
    assert_eq!(&s.row(0)[7..], "a");
    e.handle_key(crate::key::Key::ch('a'));
    s.draw(&e);
    assert_eq!(
        &s.row(1)[7..],
        "s",
        "the remaining label key must be visible"
    );
}

#[test]
fn modeline_shows_metadata_and_keeps_the_filename_when_narrow() {
    let mut e = editor("fn main() {}\r\n", "");
    e.path = Some("/a/very/long/project/src/main.rs".into());
    let mut s = Screen::new(100, 5);
    s.hl.set_file(e.path.as_deref(), &e.buf);
    s.draw(&e);
    let row = s.row(3);
    for want in ["NORMAL", "main.rs", "1:1", "100%", "CRLF", "UTF-8", "Rust"] {
        assert!(row.contains(want), "{want} missing from {row}");
    }
    let mut narrow = Screen::new(30, 5);
    narrow.hl.set_file(e.path.as_deref(), &e.buf);
    narrow.draw(&e);
    let row = narrow.row(3);
    assert!(row.contains("main.rs") && row.contains("1:1"), "{row}");
    assert!(
        !row.contains("UTF-8"),
        "secondary details should yield to the filename: {row}"
    );
}

#[test]
fn modeline_metadata_yields_to_filename_and_unsaved_flags() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q", "-b", "a-very-long-feature-branch"])
            .status()
            .unwrap()
            .success()
    );
    let path = dir.path().join("important-file.rs");
    let mut e = editor(&"a".repeat(10_000), "");
    e.path = Some(path.clone());
    e.cur.byte = 9_999;
    e.buf.modified = true;
    e.readonly = true;
    e.git = crate::git::Gutter::load(&path);
    let started = std::time::Instant::now();
    while !e.git.refresh(&e.buf) {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut s = Screen::new(80, 5);
    s.hl.set_file(e.path.as_deref(), &e.buf);
    s.draw(&e);
    let row = s.row(3);
    for want in ["NORMAL", "important-file.rs", "[+]", "[RO]", "1:10000"] {
        assert!(row.contains(want), "{want} missing from {row}");
    }
}

#[test]
fn ex_completion_popup_cycles_and_esc_dismisses_it() {
    let mut s = Screen::new(40, 12);
    let mut e = editor("text", ":<Tab>");
    s.draw(&e);
    let rows: Vec<_> = (0..11).map(|y| s.row(y)).collect();
    // Each name, then what it does.
    let name = |r: &str| r.split_whitespace().next().unwrap_or("").to_string();
    assert!(
        rows.iter()
            .any(|r| name(r) == "ai" && r.contains("have Claude")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| name(r) == "buffer"), "{rows:?}");
    let ai_y = (0..11).find(|&y| name(&s.row(y)) == "ai").unwrap();
    assert_eq!(
        s.term.backend().buffer()[(0, ai_y)].bg,
        ratatui::style::Color::Cyan
    );
    // `:ai` needs an argument, so it comes with the space before it.
    assert_eq!(s.cursor(), (4, 11));
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Tab));
    assert!(matches!(&e.mode, Mode::Command(cl) if cl.text == "bdelete"));
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::BackTab));
    assert!(matches!(&e.mode, Mode::Command(cl) if cl.text == "ai "));
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Esc));
    assert!(
        matches!(&e.mode, Mode::Command(_)),
        "first Esc only closes completion"
    );
    s.draw(&e);
    assert!(!(0..11).any(|y| name(&s.row(y)) == "buffer"));
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Esc));
    assert_eq!(e.mode, Mode::Normal);
}

#[test]
fn pasting_into_ex_dismisses_old_completion_candidates() {
    let mut e = editor("text", ":wr<Tab>");
    e.paste(" notes.txt");
    assert!(
        matches!(&e.mode, Mode::Command(cl) if cl.text == "write notes.txt" && cl.comp.is_none())
    );
}

#[test]
fn modified_and_mode_in_status() {
    let mut s = Screen::new(40, 5);
    let mut e = editor("a\nb\nc", "xi");
    e.path = Some("notes.txt".into());
    s.draw(&e);
    assert!(s.row(3).contains("INSERT"), "{}", s.row(3));
    assert!(s.row(3).contains("notes.txt [+]"), "{}", s.row(3));
}

#[test]
fn long_line_cut_marker() {
    let mut s = Screen::new(20, 3);
    let e = editor(&"x".repeat(40), "");
    s.draw(&e);
    assert_eq!(s.row(0), format!("  1 {}›", "x".repeat(15)));
}

#[test]
fn horizontal_scroll_follows_cursor() {
    let mut s = Screen::new(20, 3);
    let text: String = ('a'..='z').cycle().take(40).collect();
    let e = editor(&text, "$");
    s.draw(&e);
    assert_eq!(s.view.left, 24);
    assert_eq!(s.cursor(), (19, 0));
}

#[test]
fn wide_char_cursor_column() {
    let mut s = Screen::new(20, 3);
    let e = editor("漢x", "l");
    s.draw(&e);
    assert_eq!(s.cursor(), (6, 0));
    assert_eq!(s.row(0), "  1 漢x");
}

#[test]
fn tabs_and_control_chars() {
    let mut s = Screen::new(30, 4);
    let e = editor("\tx\na\u{1}b", "");
    s.draw(&e);
    assert_eq!(s.row(0), "  1         x");
    assert_eq!(s.row(1), "  2 a^Ab");
}

#[test]
fn scrolloff_keeps_context() {
    let mut s = Screen::new(20, 7);
    let text = (1..=20)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let e = editor(&text, "10G");
    s.draw(&e);
    assert_eq!(s.view.top, 7);
    assert_eq!(s.row(0), "  8 8");
    assert_eq!(s.cursor(), (4, 2));
}

#[test]
fn zz_centers_the_cursor_line() {
    // 10 text rows: zz puts the cursor on the fifth.
    let mut s = Screen::new(20, 12);
    let text = (1..=40)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut e = editor(&text, "10G");
    s.draw(&e);
    assert_eq!(
        s.view.top, 2,
        "scrolloff alone leaves line 10 near the bottom"
    );
    e.viewport = Some(crate::zap::Viewport {
        view: s.view,
        rows: 10,
        cols: 20,
        wrap: false,
    });
    for k in crate::key::parse_keys("zz") {
        e.handle_key(k);
    }
    s.draw(&e);
    assert_eq!(s.view.top, 5);
    assert_eq!(s.cursor().1, 4);
    for k in crate::key::parse_keys("2Gzz") {
        e.handle_key(k);
    }
    s.draw(&e);
    assert_eq!(s.view.top, 0, "no scrolling above the first line");
}

#[test]
fn wrap_mode() {
    let mut s = Screen::new(20, 5);
    let mut e = editor(&format!("{}\nb", "x".repeat(40)), "");
    e.run_ex("set wrap");
    s.draw(&e);
    assert_eq!(s.row(0), format!("  1 {}", "x".repeat(16)));
    assert_eq!(s.row(1), format!("    {}", "x".repeat(16)));
    assert_eq!(s.row(2), format!("    {}", "x".repeat(8)));
}

#[test]
fn wrap_mode_cursor_on_continuation_row() {
    let mut s = Screen::new(20, 5);
    let mut e = editor(&"x".repeat(40), "$");
    e.run_ex("set wrap");
    s.draw(&e);
    assert_eq!(s.cursor(), (4 + 7, 2));
}

#[test]
fn command_line_and_messages() {
    let mut s = Screen::new(20, 4);
    let e = editor("a", ":w");
    s.draw(&e);
    assert_eq!(s.row(3), ":w");
    assert_eq!(s.cursor(), (2, 3));
    let e = editor("a", "/zz<Enter>");
    s.draw(&e);
    assert_eq!(s.row(3), "? pattern not found:");
}

#[test]
fn popup_is_drawn() {
    let mut s = Screen::new(30, 8);
    let mut e = editor("hello\nworld\n", "ohe");
    e.popup = Some(Popup {
        items: vec!["hello".into(), "help".into()],
        sel: Some(1),
        start: 0,
        typed: "he".into(),
    });
    assert_eq!(e.mode, Mode::Insert);
    s.draw(&e);
    // Popup opens below the cursor (row 1), starting one column left of the word.
    assert_eq!(s.row(2), "  3 hello");
    assert_eq!(s.row(3), "~   help");
    let b = s.term.backend().buffer();
    assert_eq!(
        b[(4, 3)].bg,
        ratatui::style::Color::White,
        "selected item is highlighted"
    );
}

#[test]
fn visual_selection_stays_visible_while_typing_ex() {
    let mut s = Screen::new(40, 6);
    let mut e = editor("one\ntwo\nthree\nfour", "Vj:ai explain");
    s.draw(&e);
    for y in [0, 1] {
        assert!(
            s.term.backend().buffer()[(4, y)]
                .modifier
                .contains(Modifier::REVERSED)
        );
    }
    assert!(
        !s.term.backend().buffer()[(4, 2)]
            .modifier
            .contains(Modifier::REVERSED)
    );
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Esc));
    s.draw(&e);
    assert!(
        !s.term.backend().buffer()[(4, 0)]
            .modifier
            .contains(Modifier::REVERSED)
    );
    assert!(!e.buf.modified);
}

#[test]
fn visual_selection_is_reversed() {
    let mut s = Screen::new(20, 5);
    let e = editor("ab\ncd\nef", "Vj");
    s.draw(&e);
    let b = s.term.backend().buffer();
    assert!(b[(4, 0)].modifier.contains(Modifier::REVERSED));
    // The cursor's cell (c) is left to the terminal's cursor to invert.
    assert!(!b[(4, 1)].modifier.contains(Modifier::REVERSED));
    assert!(b[(5, 1)].modifier.contains(Modifier::REVERSED));
    assert!(!b[(4, 2)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn charwise_visual_reverses_just_the_selection() {
    let mut s = Screen::new(20, 5);
    let e = editor("abcd\nefgh\nij", "lvj");
    s.draw(&e);
    let b = s.term.backend().buffer();
    let rev = |x, y| b[(x, y)].modifier.contains(Modifier::REVERSED);
    // Text starts at column 4: b-d of the first line, e-f of the second,
    // except the cursor's cell (f), left to the terminal's cursor to invert.
    assert_eq!(
        (4..8).map(|x| rev(x, 0)).collect::<Vec<_>>(),
        [false, true, true, true]
    );
    assert_eq!(
        (4..8).map(|x| rev(x, 1)).collect::<Vec<_>>(),
        [true, false, false, false]
    );
    assert!(!rev(4, 2));
    assert!(s.row(4).contains("VISUAL") || s.row(3).contains("VISUAL"));
}

#[test]
fn relative_numbers() {
    let mut s = Screen::new(20, 5);
    let mut e = editor("a\nb\nc", "j");
    e.run_ex("set relativenumber");
    s.draw(&e);
    assert_eq!(s.row(0), "  1 a");
    assert_eq!(s.row(1), "  2 b");
    assert_eq!(s.row(2), "  1 c");
}

#[test]
fn window_height_clamps() {
    assert_eq!(window_height(12, 3, 40), 5);
    assert_eq!(window_height(12, 100, 10), 8);
    assert_eq!(window_height(12, 0, 40), 3);
    assert_eq!(window_height(12, 100, 3), 3);
}

#[test]
fn tiny_windows_dont_panic() {
    for (w, h) in [(20, 1), (20, 2), (1, 3), (2, 1), (5, 4), (1, 1)] {
        let mut s = Screen::new(w, h);
        let mut e = editor("hello world\nsecond line", "ohe");
        e.popup = Some(Popup {
            items: vec!["hello".into()],
            sel: Some(0),
            start: 0,
            typed: "he".into(),
        });
        s.draw(&e);
        let e = editor("abc", ":w");
        s.draw(&e);
    }
}

#[test]
fn cursor_is_never_under_the_cut_marker() {
    let text: String = ('a'..='z').cycle().take(100).collect();
    for keys in ["050l", "020l", "0fz", "016l", "015l"] {
        let mut s = Screen::new(20, 3);
        let e = editor(&text, keys);
        s.draw(&e);
        let (x, y) = s.cursor();
        let under = s.term.backend().buffer()[(x, y)].symbol().to_string();
        let want = text[e.cur.byte..].chars().next().unwrap().to_string();
        assert_eq!(
            under, want,
            "keys {keys}: cursor at ({x},{y}) shows {under:?}"
        );
    }
}

fn cell_at_cursor(s: &mut Screen) -> String {
    let (x, y) = s.cursor();
    s.term.backend().buffer()[(x, y)].symbol().to_string()
}

#[test]
fn wrap_moves_a_wide_char_that_does_not_fit_and_the_cursor_follows() {
    // 36 text columns: "a" + 17 漢 = 35, so the 18th 漢 starts row 2.
    let text = format!("a{}\nend", "漢".repeat(30));
    for (keys, want) in [
        ("17l", (37, 0)),
        ("18l", (4, 1)),
        ("19l", (6, 1)),
        ("$", (28, 1)),
    ] {
        let mut s = Screen::new(40, 8);
        let mut e = editor(&text, keys);
        e.run_ex("set wrap");
        s.draw(&e);
        assert_eq!(s.row(1), format!("    {}", "漢".repeat(13)));
        assert_eq!(s.cursor(), want, "{keys}");
        assert_eq!(cell_at_cursor(&mut s), "漢", "{keys}");
    }
}

#[test]
fn wrap_scrolls_inside_a_line_taller_than_the_window() {
    let long: String = ('a'..='y').cycle().take(300).collect::<String>() + "Z";
    let text = format!("{long}\nb\nc\nd");
    let mut s = Screen::new(40, 6); // 4 text rows of 36 columns
    let e = editor(&text, ":set wrap<CR>$");
    s.draw(&e);
    assert_eq!(cell_at_cursor(&mut s), "Z");
    let e = editor(&text, ":set wrap<CR>$0");
    s.draw(&e);
    assert_eq!(s.cursor(), (4, 0), "back at the start of the line");
    assert_eq!(s.row(0), format!("  1 {}", &long[..36]));
    let e = editor(&text, ":set wrap<CR>$j");
    s.draw(&e);
    assert_eq!(cell_at_cursor(&mut s), "b");
}

#[test]
fn sideways_scrolled_line_with_tabs_and_wide_chars() {
    let text = format!("ab\t{}xyz", "漢".repeat(40));
    let mut s = Screen::new(30, 3); // 26 text columns
    let e = editor(&text, "$");
    s.draw(&e);
    assert_eq!(cell_at_cursor(&mut s), "z");
    assert!(s.row(0).ends_with("xyz"), "{}", s.row(0));
    let e = editor(&text, "$0");
    s.draw(&e);
    // 26 columns: "ab" + tab (8) + 8 漢 (16) = 24; a 9th would reach the
    // last column, which belongs to the › marker.
    assert_eq!(s.row(0), format!("  1 ab      {} ›", "漢".repeat(8)));
}

#[test]
fn status_line_colors_the_mode() {
    use ratatui::style::Color;
    let mut s = Screen::new(40, 5);
    let cell = |s: &Screen, x: u16| s.term.backend().buffer()[(x, 3)].clone();
    let mut e = editor("a\nb", "");
    s.draw(&e);
    assert_eq!(cell(&s, 2).fg, Color::Blue);
    assert_eq!(cell(&s, 30).bg, Color::Indexed(235));
    e.handle_key(crate::key::Key::ch('i'));
    s.draw(&e);
    assert_eq!(cell(&s, 2).fg, Color::Green);
    e.handle_key(crate::key::Key::ch('x'));
    s.draw(&e);
    let row = s.row(3);
    let plus = row.find("[+]").unwrap() as u16;
    assert_eq!(cell(&s, plus).fg, Color::LightYellow, "{row}");
}

#[test]
fn picker_is_a_panel_under_the_file() {
    let mut s = Screen::new(40, 20);
    let text: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    let mut e = editor(&text, "");
    s.draw(&e);
    let before: Vec<String> = (0..18).map(|y| s.row(y)).collect();
    for k in parse_keys(" k") {
        e.handle_key(k);
    }
    s.draw(&e);
    // 18 text rows, 30 results: the panel stops at half (9 rows); the top
    // 9 still show the file exactly as before.
    for y in 0..9 {
        assert_eq!(s.row(y), before[y as usize], "row {y}");
    }
    assert!(s.row(9).contains("LINES"), "{}", s.row(9));
    assert!(s.row(10).starts_with("lines>"), "{}", s.row(10));
    assert_eq!(s.cursor(), (7, 10));
    assert!(s.row(11).starts_with(">  1: line 1"), "{}", s.row(11));
    assert!(s.row(19).starts_with("   9: line 9"), "{}", s.row(19));
    for k in parse_keys("<Down>") {
        e.handle_key(k);
    }
    s.draw(&e);
    assert!(s.row(12).starts_with(">  2: line 2"), "{}", s.row(12));
    // Fewer results: only as tall as needed (3 rows), the file above.
    for k in parse_keys("<C-u>1$") {
        e.handle_key(k);
    }
    s.draw(&e);
    assert_eq!(s.row(14), before[14]);
    assert_eq!(
        (17..20).map(|y| s.row(y)).collect::<Vec<_>>(),
        [">  1: line 1", "  11: line 11", "  21: line 21"]
    );
}

#[test]
fn line_picker_rows_are_syntax_highlighted() {
    let mut s = Screen::new(40, 12);
    let text = "fn main() {\n    let x = 1;\n}\n";
    let mut e = editor(text, "");
    e.path = Some("main.rs".into());
    s.hl.set_file(e.path.as_deref(), &e.buf);
    s.draw(&e);
    // `fn` on the file's first row, after the gutter.
    let fg = |s: &Screen, x: u16, y: u16| s.term.backend().buffer()[(x, y)].fg;
    let kw = fg(&s, 4, 0);
    assert_ne!(kw, ratatui::style::Color::Reset, "editor highlights `fn`");
    for k in parse_keys(" kmain") {
        e.handle_key(k);
    }
    s.draw(&e);
    let row = (0..12)
        .find(|&y| s.row(y).contains("1: fn main"))
        .expect("picker row");
    let x = s.row(row).find("fn").unwrap() as u16;
    // Unselected rows aren't reversed; this one is selected, so compare
    // the color whichever way it's applied.
    let cell = s.term.backend().buffer()[(x, row)].clone();
    assert!(
        cell.fg == kw || cell.bg == kw,
        "picker `fn`: {cell:?}, editor: {kw:?}"
    );
}

#[test]
fn file_pickers_show_type_icons_when_configured() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("main.rs"), "").unwrap();
    std::fs::write(dir.path().join("notes.zzz"), "").unwrap();
    let mut e = editor("", "");
    crate::pick::browse(&mut e, dir.path());
    let mut s = Screen::new(40, 12);
    let rows = |s: &Screen| (0..12).map(|y| s.row(y)).collect::<Vec<_>>().join("\n");
    s.draw(&e);
    assert!(!rows(&s).contains('\u{f1617}'), "{}", rows(&s));
    s.cfg.icons = true;
    s.draw(&e);
    let all = rows(&s);
    for want in ["\u{f024b} src/", "\u{f1617} main.rs", "\u{f0214} notes.zzz"] {
        assert!(all.contains(want), "{want:?} in\n{all}");
    }
}

#[test]
fn mouse_click_selects_picker_row_and_double_click_opens_it() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut s = Screen::new(40, 10);
    let mut e = editor("one\ntwo\nthree\nfour\nfive\nsix", " k");
    let area = ratatui::layout::Rect::new(2, 3, 40, 10);
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 9,
        row: 11,
        modifiers: KeyModifiers::NONE,
    };
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert!(matches!(&e.mode, Mode::Pick(p) if p.sel == 2));
    assert_eq!(e.cur.pos(), (0, 0));
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert_eq!(e.mode, Mode::Normal);
    assert_eq!(e.cur.pos(), (2, 0));
}

#[test]
fn double_click_after_scrolling_opens_the_row_that_was_clicked() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut s = Screen::new(40, 10);
    let text: String = (1..=20).map(|i| format!("line{i}\n")).collect();
    let mut e = editor(
        &text,
        " k<Down><Down><Down><Down><Down><Down><Down><Down><Down><Down>",
    );
    s.draw(&e);
    let before = s.row(7);
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 9,
        row: 7,
        modifiers: KeyModifiers::NONE,
    };
    let area = ratatui::layout::Rect::new(0, 0, 40, 10);
    super::render::mouse(&mut e, &mut s.view, area, click);
    s.draw(&e);
    assert_eq!(
        &s.row(7)[1..],
        &before[1..],
        "selection must not move the clicked row"
    );
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert_eq!(e.mode, Mode::Normal);
    assert_eq!(e.cur.line, 8);
}

#[test]
fn dired_colors_file_kinds_fields_and_marks() {
    use ratatui::style::Color;
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("folder")).unwrap();
    std::fs::write(dir.path().join("plain.txt"), "hello").unwrap();
    std::fs::write(dir.path().join("archive.tar.gz"), "compressed").unwrap();
    std::fs::write(dir.path().join(".hidden"), "hidden").unwrap();
    std::fs::write(dir.path().join("run"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(
        dir.path().join("run"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("plain.txt", dir.path().join("link")).unwrap();
    let mut e = editor("", "");
    crate::dired::visit(&mut e, dir.path(), None).unwrap();
    let mut s = Screen::new(100, 12);
    e.run_ex("set nonumber");
    s.draw(&e);
    // Names start where the `.` entry's does.
    let col = e.buf.line(1).len() - 1;
    let position = |name: &str| {
        let y = (1..e.line_count())
            .find(|&y| e.buf.line(y)[col..].starts_with(name))
            .unwrap();
        (col as u16, y as u16)
    };
    let folder = position("folder");
    let link = position("link");
    let run = position("run");
    let plain = position("plain.txt");
    let archive = position("archive.tar.gz");
    let hidden = position(".hidden");
    let b = s.term.backend().buffer();
    let directory_color = b[folder].fg;
    for pos in [folder, link, run, plain] {
        assert_ne!(
            b[pos].fg,
            Color::Reset,
            "file kind at {pos:?} must be styled"
        );
    }
    assert_ne!(b[folder].fg, b[plain].fg);
    assert_ne!(b[link].fg, b[folder].fg);
    assert_ne!(b[run].fg, b[plain].fg);
    assert_ne!(
        b[(plain.0 + 5, plain.1)].fg,
        b[plain].fg,
        "suffix differs from basename"
    );
    assert_ne!(
        b[(archive.0 + 11, archive.1)].fg,
        b[(plain.0 + 5, plain.1)].fg,
        "compressed suffix differs from ordinary extension"
    );
    assert_ne!(b[hidden].fg, b[plain].fg, "hidden names are subdued");
    assert_ne!(
        b[(2, run.1)].fg,
        b[(3, run.1)].fg,
        "absent and read permissions differ"
    );
    assert_ne!(
        b[(3, run.1)].fg,
        b[(4, run.1)].fg,
        "read and write permissions differ"
    );
    assert_ne!(
        b[(5, run.1)].fg,
        b[(3, run.1)].fg,
        "execute permissions stand out"
    );
    let (size, date) = (plain.0 - 15, plain.0 - 12);
    assert_ne!(b[(size, plain.1)].fg, Color::Reset, "size is styled");
    assert_ne!(
        b[(date, plain.1)].fg,
        b[(size, plain.1)].fg,
        "date differs from size"
    );
    e.set_cursor(plain.1 as usize, plain.0 as usize);
    e.handle_key(crate::key::Key::ch('m'));
    s.draw(&e);
    let b = s.term.backend().buffer();
    assert!(b[(0, plain.1)].modifier.contains(Modifier::BOLD));
    let marked = b[(0, plain.1)].fg;
    e.set_cursor(folder.1 as usize, folder.0 as usize);
    e.handle_key(crate::key::Key::ch('d'));
    s.draw(&e);
    let b = s.term.backend().buffer();
    assert_ne!(b[(0, folder.1)].fg, marked);
    assert_eq!(
        b[(0, folder.1)].fg,
        b[folder].fg,
        "delete flag overrides directory color"
    );
    e.set_cursor(folder.1 as usize, folder.0 as usize);
    e.handle_key(crate::key::Key::ch('u'));
    s.draw(&e);
    assert_eq!(s.term.backend().buffer()[folder].fg, directory_color);
}

#[test]
fn dired_colors_survive_compact_wrap_and_yield_to_name_editing() {
    use ratatui::style::Color;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("漢字folder")).unwrap();
    let mut e = editor("", "");
    crate::dired::visit(&mut e, dir.path(), Some("漢字folder")).unwrap();
    e.handle_key(crate::key::Key::ch('('));
    let mut s = Screen::new(8, 16);
    e.run_ex("set nonumber wrap cursorline");
    s.draw(&e);
    let b = s.term.backend().buffer();
    let cells: Vec<_> = b
        .content
        .iter()
        .filter(|c| matches!(c.symbol(), "漢" | "字" | "f" | "o" | "l" | "d" | "e" | "r"))
        .collect();
    assert!(cells.iter().any(|c| c.symbol() == "漢"));
    let color = cells.iter().find(|c| c.symbol() == "漢").unwrap().fg;
    assert_ne!(color, Color::Reset);
    // The tail wraps onto the next row, retaining the directory color.
    let start = b.content.iter().position(|c| c.symbol() == "漢").unwrap();
    let tail = b.content[start..]
        .iter()
        .find(|c| c.symbol() == "r")
        .unwrap();
    assert_eq!(tail.fg, color);
    e.handle_key(crate::key::Key::ch('i'));
    s.draw(&e);
    let b = s.term.backend().buffer();
    assert_eq!(
        b.content.iter().find(|c| c.symbol() == "漢").unwrap().fg,
        Color::Reset
    );
}

#[test]
fn dired_newline_names_do_not_break_coloring() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a\nvery-long-name.txt"), "hello").unwrap();
    let mut e = editor("", "");
    crate::dired::visit(&mut e, dir.path(), None).unwrap();
    let mut s = Screen::new(80, 12);
    s.draw(&e);
    e.handle_key(crate::key::Key::ch('('));
    s.draw(&e);
    assert!(!e.buf.modified);
}

#[test]
fn dired_double_click_opens_an_entry_and_slow_clicks_only_select() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.txt"), "hello").unwrap();
    let mut e = editor("", "");
    crate::dired::visit(&mut e, dir.path(), None).unwrap();
    let line = (0..e.line_count())
        .find(|&l| e.buf.line(l).ends_with("file.txt"))
        .unwrap();
    let path = e.dired.as_ref().unwrap().dir.join("file.txt");
    let mut s = Screen::new(80, 12);
    let area = ratatui::layout::Rect::new(2, 3, 80, 12);
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 9,
        row: 3 + line as u16,
        modifiers: KeyModifiers::NONE,
    };
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            ..click
        },
    );
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert_eq!(e.cur.line, line);
    assert!(e.pending_effect.is_none());
    s.view.last_click = Some((
        std::time::Instant::now() - Duration::from_millis(500),
        false,
        line,
    ));
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert!(
        e.pending_effect.is_none(),
        "slow clicks must not open an entry"
    );
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert!(
        matches!(&e.pending_effect, Some(crate::ex::ExEffect::Open { path: target, .. }) if *target == path)
    );
    assert!(!e.buf.modified);
    assert!(
        !s.view.detached,
        "opening an entry should follow its cursor again"
    );
}

#[test]
fn dired_edge_click_keeps_the_entry_under_the_pointer() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().unwrap();
    for i in 0..24 {
        std::fs::write(dir.path().join(format!("file{i:02}.txt")), "hello").unwrap();
    }
    let mut e = editor("", "");
    crate::dired::visit(&mut e, dir.path(), None).unwrap();
    let mut s = Screen::new(80, 12);
    e.run_ex("set nowrap");
    s.draw(&e);
    let before = s.row(9);
    let name = (0..24)
        .map(|i| format!("file{i:02}.txt"))
        .find(|name| before.ends_with(name))
        .unwrap();
    let path = e.dired.as_ref().unwrap().dir.join(name);
    let area = ratatui::layout::Rect::new(0, 0, 80, 12);
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 9,
        row: 9,
        modifiers: KeyModifiers::NONE,
    };
    super::render::mouse(&mut e, &mut s.view, area, click);
    s.draw(&e);
    assert_eq!(
        s.row(9),
        before,
        "clicking an edge entry must not shift the view"
    );
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert!(
        matches!(&e.pending_effect, Some(crate::ex::ExEffect::Open { path: target, .. }) if *target == path)
    );
}

#[test]
fn mouse_wheel_scrolls_picker_matches() {
    use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    let mut s = Screen::new(40, 8);
    let mut e = editor(&"match\n".repeat(20), " k");
    let area = ratatui::layout::Rect::new(2, 3, 40, 8);
    let mut wheel = |e: &mut Editor, kind| {
        super::render::mouse(
            e,
            &mut s.view,
            area,
            MouseEvent {
                kind,
                column: 8,
                row: 10,
                modifiers: KeyModifiers::NONE,
            },
        );
    };
    wheel(&mut e, MouseEventKind::ScrollDown);
    let Mode::Pick(p) = &e.mode else {
        panic!("picker closed")
    };
    assert_eq!(p.sel, 1);
    for _ in 0..30 {
        wheel(&mut e, MouseEventKind::ScrollDown);
    }
    let Mode::Pick(p) = &e.mode else {
        panic!("picker closed")
    };
    assert_eq!(p.sel, p.rows.len() - 1);
    for _ in 0..30 {
        wheel(&mut e, MouseEventKind::ScrollUp);
    }
    let Mode::Pick(p) = &e.mode else {
        panic!("picker closed")
    };
    assert_eq!(p.sel, 0);
    assert_eq!(e.cur.pos(), (0, 0));
    assert!(!e.buf.modified);
}

#[test]
fn mouse_scroll_and_click_use_screen_coordinates() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut s = Screen::new(20, 6);
    let mut e = editor("a\nb\nc\nd\ne\nf\ng\nh", "");
    let area = ratatui::layout::Rect::new(2, 3, 20, 6);
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    e.run_ex("set nowrap");
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::ScrollDown, 8, 3),
    );
    assert_eq!(e.cur.line, 0);
    assert_eq!(s.view.top, 3);
    s.draw(&e);
    assert_eq!(
        s.view.top, 3,
        "redrawing must not follow the cursor after a wheel event"
    );
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 6, 4),
    );
    assert_eq!(e.cur.line, 4);
    e = editor("ab漢x\n\tz", "");
    s.view = View::default();
    let area = ratatui::layout::Rect::new(0, 0, 9, 6);
    e.run_ex("set wrap");
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 7, 0),
    );
    assert_eq!(
        e.cur.byte, 2,
        "either cell of a wide character selects that character"
    );
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 4, 1),
    );
    assert_eq!(e.cur.pos(), (1, 0), "a tab selects its source byte");
    e = editor(&"abcdefghij\n".repeat(10), ":set wrap<CR>");
    s.view = View::default();
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::ScrollDown, 5, 0),
    );
    assert_eq!((s.view.top, s.view.top_row), (1, 1));
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 6, 0),
    );
    assert_eq!(e.cur.pos(), (1, 7));
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::ScrollUp, 5, 0),
    );
    assert_eq!((s.view.top, s.view.top_row), (0, 0));
    e = editor("abcdefghijklmnopqrstuvwxyz", "");
    s.view.left = 10;
    e.run_ex("set nowrap");
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 6, 0),
    );
    assert_eq!(e.cur.byte, 12);
    e = editor("abcde", "A");
    s.view = View::default();
    s.view.wheel(&e, 1, 5, true, true);
    assert_eq!(
        s.view.top_row, 1,
        "the full-row end cursor occupies a screen row"
    );
}

#[test]
fn splash_shows_actions_and_hotkeys() {
    let dir = tempfile::tempdir().unwrap();
    let recent = dir.path().join("recent");
    crate::pick::recent::record(&recent, std::path::Path::new("/tmp/notes.txt"), None);
    let mut e = editor("", "");
    e.project = std::sync::Arc::new(crate::pick::Project::new(None, Some(recent)));
    let mut s = Screen::new(80, 40);
    s.draw(&e);
    let text = (0..38).map(|y| s.row(y)).collect::<Vec<_>>().join("\n");
    for label in [
        "Dr. Fred",
        "New file",
        "Find file",
        "Recent files",
        "Live grep",
        "Claude",
        "Config",
        "Quit",
        "notes.txt",
    ] {
        assert!(text.contains(label), "missing {label}");
    }
    let mut previous = None;
    let mut key_column = None;
    for (label, key) in [
        ("New file", 'e'),
        ("Find file", 'f'),
        ("Recent files", 'r'),
        ("Live grep", 'g'),
        ("Claude", 'a'),
        ("Config", 'c'),
        ("Quit", 'q'),
    ] {
        let y = (0..38).find(|&y| s.row(y).contains(label)).unwrap();
        assert!(
            previous.is_none_or(|prev| y == prev + 1),
            "keep the original compact spacing"
        );
        previous = Some(y);
        let row = s.row(y);
        assert!(row.ends_with(key), "{row}");
        let column = row.chars().count() - 1;
        assert!(key_column.is_none_or(|c| c == column));
        key_column = Some(column);
        assert_eq!(
            s.term.backend().buffer()[(column as u16, y)].fg,
            ratatui::style::Color::LightMagenta
        );
    }
    let mut standard = Screen::new(80, 24);
    standard.draw(&e);
    assert!(
        (0..22).any(|y| standard.row(y).contains("Dr. Fred")),
        "keep the logo and menu on a standard terminal"
    );
    assert!(
        (0..22).any(|y| standard.row(y).contains("notes.txt")),
        "show recent files on a standard terminal"
    );
    e.handle_key(crate::key::Key::ch('r'));
    assert!(matches!(e.mode, Mode::Pick(ref p) if p.kind == crate::pick::Kind::Recent));
    for (key, kind) in [
        ('f', crate::pick::Kind::Files),
        ('g', crate::pick::Kind::Grep),
    ] {
        e.mode = Mode::Normal;
        e.handle_key(crate::key::Key::ch(key));
        assert!(matches!(e.mode, Mode::Pick(ref p) if p.kind == kind));
    }
    e.mode = Mode::Normal;
    e.handle_key(crate::key::Key::ch('v'));
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        concat!("fred ", env!("CARGO_PKG_VERSION"))
    );
    e.handle_key(crate::key::Key::ch('1'));
    assert!(
        matches!(e.pending_effect, Some(crate::ex::ExEffect::Open { ref path, .. }) if path == std::path::Path::new("/tmp/notes.txt"))
    );
}

#[test]
fn splash_new_file_ai_config_and_quit_shortcuts() {
    let mut e = editor("", "e");
    assert_eq!(e.mode, Mode::Insert);
    for k in parse_keys("hello<Esc>u") {
        e.handle_key(k);
    }
    assert_eq!(
        e.buf.len_bytes(),
        0,
        "new-file typing is one undoable insert"
    );
    e = editor("", "a");
    assert!(matches!(&e.mode, Mode::Command(cl) if cl.text == "ai "));
    e = editor("", "c");
    assert!(
        matches!(&e.pending_effect, Some(crate::ex::ExEffect::Open { path, .. }) if *path == crate::config::config_path())
    );
    e = editor("", "q");
    assert!(matches!(
        e.pending_effect,
        Some(crate::ex::ExEffect::Quit { force: false })
    ));
}

#[test]
fn hl_line_follows_cursor_and_preserves_syntax() {
    let mut s = Screen::new(30, 6);
    let mut e = editor("fn main() {}\nlet x = 1;\n", "");
    e.path = Some("main.rs".into());
    s.hl.set_file(e.path.as_deref(), &e.buf);
    s.draw(&e);
    let plain = s.term.backend().buffer().clone();
    e.run_ex("set cursorline");
    s.draw(&e);
    let highlighted = s.term.backend().buffer();
    let bg = highlighted[(29, 0)].bg;
    assert_ne!(bg, plain[(29, 0)].bg);
    for x in 4..30 {
        assert_eq!(highlighted[(x, 0)].bg, bg);
        assert_eq!(highlighted[(x, 0)].fg, plain[(x, 0)].fg);
        assert_eq!(highlighted[(x, 0)].modifier, plain[(x, 0)].modifier);
    }
    assert_eq!(highlighted[(0, 0)], plain[(0, 0)], "gutter unchanged");
    assert_eq!(highlighted[(4, 1)], plain[(4, 1)]);
    e.handle_key(crate::key::Key::ch('j'));
    s.draw(&e);
    let moved = s.term.backend().buffer();
    assert_eq!(moved[(29, 0)].bg, plain[(29, 0)].bg);
    assert_eq!(moved[(29, 1)].bg, bg);
}

#[test]
fn hl_line_covers_wrapped_rows_and_empty_lines() {
    let mut s = Screen::new(8, 6);
    let mut e = editor("漢字abcde\n\nlast", ":set cul wrap nonu<CR>");
    s.draw(&e);
    let b = s.term.backend().buffer();
    let bg = b[(7, 1)].bg;
    assert_ne!(bg, b[(7, 2)].bg);
    for y in 0..2 {
        let mut x = 0;
        while x < 8 {
            assert_eq!(b[(x, y)].bg, bg, "wrapped cell {x},{y}");
            // The backend skips hidden continuation cells of wide glyphs.
            x += unicode_width::UnicodeWidthStr::width(b[(x, y)].symbol()).max(1) as u16;
        }
    }
    e.handle_key(crate::key::Key::ch('j'));
    s.draw(&e);
    let b = s.term.backend().buffer();
    for x in 0..8 {
        assert_eq!(b[(x, 2)].bg, bg, "empty line cell {x}");
    }
    assert_ne!(b[(7, 0)].bg, bg);
}

#[test]
fn hl_line_defers_to_visual_selection() {
    let e = editor("one\ntwo\nthree", "Vj");
    let mut s = Screen::new(20, 5);
    s.draw(&e);
    let selection = s.term.backend().buffer().clone();
    s.cfg = Config::parse("hl_line = true").0;
    s.draw(&e);
    assert_eq!(s.term.backend().buffer(), &selection);
}

#[test]
fn hl_line_does_not_change_splash() {
    let e = editor("", "");
    let mut s = Screen::new(60, 20);
    s.draw(&e);
    let splash = s.term.backend().buffer().clone();
    s.cfg = Config::parse("hl_line = true").0;
    s.draw(&e);
    assert_eq!(s.term.backend().buffer(), &splash);
}

#[test]
fn splash_reflows_in_small_windows() {
    let e = editor("", "");
    for (w, h) in [(80, 20), (44, 12), (40, 8), (20, 10)] {
        let mut s = Screen::new(w, h);
        s.draw(&e);
        let text = (0..h - 2).map(|y| s.row(y)).collect::<Vec<_>>().join("\n");
        for label in [
            "Dr. Fred",
            "New file",
            "Find file",
            "Recent files",
            "Live grep",
            "Claude",
            "Config",
            "Quit",
        ] {
            assert!(text.contains(label), "{w}x{h} missing {label}:\n{text}");
        }
        assert!(
            !text.contains('~'),
            "splash should clear editor filler: {text}"
        );
        assert!(s.row(h - 2).contains("NORMAL"));
    }
}

#[test]
fn magit_status_uses_title_and_diff_colors() {
    use crate::magit::repo::{Repo, Snapshot};
    use crate::magit::{Kind, Row, View};
    use ratatui::style::Color;
    let mut e = editor("Head: main\n+added\n-removed\n@@ hunk", "");
    let mut v = View::status(
        Repo {
            root: "/repo".into(),
        },
        Snapshot::default(),
    );
    v.kind = Kind::Patch("123abc".into());
    v.rows = e
        .buf
        .to_bytes()
        .split(|b| *b == b'\n')
        .map(|b| Row {
            text: String::from_utf8_lossy(b).into(),
            action: None,
        })
        .collect();
    e.path = Some("/state/magit-views/opaque-hash".into());
    e.magit = Some(Box::new(v));
    e.readonly = true;
    let mut screen = Screen::new(70, 6);
    e.run_ex("set nonumber");
    screen.draw(&e);
    assert_eq!(screen.term.backend().buffer()[(0, 1)].fg, Color::Green);
    assert_eq!(screen.term.backend().buffer()[(0, 2)].fg, Color::Red);
    assert!(screen.row(4).contains("Magit 123abc"), "{}", screen.row(4));
    assert!(!screen.row(4).contains("opaque-hash"));
}

#[test]
fn explain_box_sits_above_the_selection_until_closed() {
    use crate::ex::ExEffect;
    use crate::key::{Key, KeyCode};
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let text = "a\nb\nc\nd\ne\nf\ng\nh";
    // Visual `K` asks for an explanation of the selected lines.
    let mut e = editor(text, "5GVjK");
    assert!(matches!(
        &e.pending_effect,
        Some(ExEffect::Ai { range, explain: true, .. }) if (range.start, range.end) == (4, 5)
    ));
    assert_eq!(e.mode, Mode::Normal);
    e.explain = Some((
        crate::ex::addr::Range { start: 4, end: 5 },
        "Two letters.".into(),
    ));
    let mut s = Screen::new(40, 10);
    s.draw(&e);
    // Lines 5-6 are rows 4-5; the box takes the three rows above them.
    assert!(s.row(1).contains("Claude explains") && s.row(1).contains("✕"));
    assert!(s.row(2).contains("Two letters."), "{}", s.row(2));
    assert!(s.row(4).contains('e'));
    let click = |column, row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let area = ratatui::layout::Rect::new(0, 0, 40, 10);
    // Inside the box: stays.
    super::render::mouse(&mut e, &mut s.view, area, click(10, 2));
    assert!(e.explain.is_some());
    // The ✕ closes it.
    let b = s.view.explain_box.unwrap();
    super::render::mouse(&mut e, &mut s.view, area, click(b.right() - 2, b.y));
    assert!(e.explain.is_none());
    // A click away moves the cursor but leaves it open; only Esc closes it.
    e.explain = Some((crate::ex::addr::Range { start: 4, end: 5 }, "x".into()));
    s.draw(&e);
    super::render::mouse(&mut e, &mut s.view, area, click(6, 7));
    assert!(e.explain.is_some(), "clicks away leave it open");
    assert_eq!(e.cur.line, 7);
    e.handle_key(Key::ch('j'));
    assert!(e.explain.is_some(), "other keys leave it open");
    e.handle_key(Key::new(KeyCode::Esc));
    assert!(e.explain.is_none());
}

#[test]
fn explain_reply_remains_visible_for_full_viewport_and_multiscreen_selection() {
    let text = (0..40).map(|i| format!("line {i}\n")).collect::<String>();
    for (range, top) in [
        (crate::ex::addr::Range { start: 0, end: 7 }, 0),
        (crate::ex::addr::Range { start: 0, end: 39 }, 12),
    ] {
        let mut e = editor(&text, "");
        let before = e.buf.to_bytes();
        e.set_cursor(top, 0);
        e.explain = Some((range, "Visible explanation.".into()));
        let mut s = Screen::new(40, 10);
        s.view.top = top;
        s.draw(&e);
        assert!((0..8).any(|row| s.row(row).contains("Visible explanation.")));
        assert!(s.view.explain_box.is_some());
        assert!(s.row(8).contains("NORMAL"));
        assert_eq!(e.buf.to_bytes(), before);
    }
}

#[test]
fn blame_margin_headings_and_lines_shift_text_and_show_message() {
    use crate::magit::blame::{Blame, Chunk, Info};
    use crate::magit::repo::Repo;
    let mut e = editor("alpha\nbeta\ngamma", "");
    let rev = "a".repeat(40);
    e.blame = Some(Blame {
        repo: Repo {
            root: "/repo".into(),
        },
        file: "f".into(),
        args: vec![],
        chunks: vec![Chunk {
            rev: rev.clone(),
            line: 0,
            lines: 3,
            orig_line: 1,
            orig_file: "f".into(),
            prev: None,
        }],
        info: [(
            rev,
            Info {
                summary: "the summary".into(),
                author: "Ada".into(),
                committer_time: 0,
                committer_tz: "+0000".into(),
            },
        )]
        .into(),
        style: 0,
        kind: crate::magit::blame::Kind::Addition,
        rev: None,
        version: e.buf.version,
        was_readonly: false,
    });
    let mut screen = Screen::new(100, 6);
    e.run_ex("set nonumber");
    screen.draw(&e);
    // headings: the chunk's heading on its own row above it (as upstream
    // draws it in a terminal), then the text without a gutter.
    assert_eq!(
        screen.row(0).trim_end(),
        format!(
            "Ada                  {} the summary",
            crate::magit::margin::strftime("%F %H:%M", 0)
        ),
    );
    assert!(screen.row(1).starts_with("alpha"), "{}", screen.row(1));
    assert!(screen.row(2).starts_with("beta"), "{}", screen.row(2));
    e.blame.as_mut().unwrap().style = 2;
    screen.draw(&e);
    assert!(screen.row(0).starts_with("┌alpha"), "{}", screen.row(0));
    assert!(screen.row(5).contains("the summary"), "{}", screen.row(5));
}

#[test]
fn blame_heading_rows_count_when_scrolling() {
    use crate::magit::blame::{Blame, Chunk, Info};
    use crate::magit::repo::Repo;
    let text: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
    let mut e = editor(&text.join("\n"), "");
    let revs: Vec<String> = (0..10).map(|i| format!("{i}").repeat(40)).collect();
    e.blame = Some(Blame {
        repo: Repo {
            root: "/repo".into(),
        },
        file: "f".into(),
        args: vec![],
        chunks: (0..10)
            .map(|i| Chunk {
                rev: revs[i].clone(),
                line: i,
                lines: 1,
                orig_line: i + 1,
                orig_file: "f".into(),
                prev: None,
            })
            .collect(),
        info: revs
            .iter()
            .map(|r| {
                let i = Info {
                    summary: format!("c{}", &r[..1]),
                    author: "Ada".into(),
                    committer_time: 0,
                    committer_tz: "+0000".into(),
                };
                (r.clone(), i)
            })
            .collect(),
        style: 0,
        kind: crate::magit::blame::Kind::Addition,
        rev: None,
        version: e.buf.version,
        was_readonly: false,
    });
    e.run_ex("set nonumber");
    // Every line has a heading above it: the last line still shows.
    e.set_cursor(9, 0);
    let mut screen = Screen::new(60, 8);
    screen.draw(&e);
    let rows: Vec<String> = (0..6).map(|y| screen.row(y)).collect();
    assert!(rows.iter().any(|r| r.starts_with("line9")), "{rows:?}");
    let at = rows.iter().position(|r| r.starts_with("line9")).unwrap();
    assert!(rows[at - 1].contains("c9"), "{rows:?}");
}

#[test]
fn magit_menus_render_as_transient_popups() {
    use ratatui::style::Color;
    let mut e = editor("source line", "");
    crate::magit::open_menu(&mut e, 'p');
    e.magit_options
        .insert(crate::magit::MenuOption::PushForceWithLease);
    let mut screen = Screen::new(120, 30);
    e.run_ex("set nonumber");
    screen.draw(&e);
    let rows: Vec<String> = (0..30).map(|y| screen.row(y)).collect();
    let find = |needle: &str| rows.iter().position(|r| r.contains(needle));
    let args = find("Arguments").expect("argument heading");
    let lease = find("-f Force with lease (--force-with-lease)").expect("switch row");
    assert!(lease > args);
    // Action groups sit side by side in one band.
    let band = find("Push current to").expect("action heading");
    assert!(
        rows[band].contains("Push") && rows[band].matches("Push").count() >= 2,
        "{}",
        rows[band]
    );
    // An enabled switch is highlighted; a disabled one is dim.
    let x = rows[lease].find("(--force-with-lease)").unwrap() as u16 + 1;
    assert_eq!(
        screen.term.backend().buffer()[(x, lease as u16)].fg,
        Color::Cyan
    );
    let dry = find("(--dry-run)").unwrap();
    let x = rows[dry].find("(--dry-run)").unwrap() as u16 + 1;
    assert_eq!(
        screen.term.backend().buffer()[(x, dry as u16)].fg,
        Color::DarkGray
    );
    // The source text stays visible above the popup.
    assert!(rows[0].contains("source line"));
}

#[test]
fn org_buffers_fold_conceal_links_and_style_headings() {
    let mut e = Editor::new(Buffer::from_text(
        "* TODO Task :work:\nbody [[https://x.org][site]]\n** Child\n* Other [[https://y.org][y]]",
    ));
    e.path = Some("t.org".into());
    crate::org::attach(&mut e);
    let mut s = Screen::new(60, 8);
    e.run_ex("set nonumber");
    s.draw(&e);
    assert_eq!(
        s.row(0).split_whitespace().collect::<Vec<_>>(),
        vec!["*", "TODO", "Task", ":work:"]
    );
    assert_eq!(s.row(1), "body site");
    for k in parse_keys("j") {
        e.handle_key(k);
    }
    s.draw(&e);
    assert_eq!(
        s.row(1),
        "body [[https://x.org][site]]",
        "the cursor line is literal"
    );
    assert_eq!(s.row(3), "* Other y");
    for k in parse_keys("k<Tab>") {
        e.handle_key(k);
    }
    s.draw(&e);
    assert!(s.row(0).starts_with("* TODO Task"));
    assert!(s.row(0).ends_with(":work:..."), "{:?}", s.row(0));
    assert_eq!(s.row(1), "* Other y");
    // TODO keyword face: org-todo (red, bold).
    let cell = &s.term.backend().buffer()[(2, 0)];
    assert!(cell.modifier.contains(Modifier::BOLD));
}

#[test]
fn magit_log_margin_and_hunk_styles() {
    use crate::magit::margin::Stamp;
    use crate::magit::repo::{Repo, Snapshot};
    use crate::magit::{Kind, Row, RowAction, View};
    use ratatui::style::{Color, Modifier};
    let text = "abc12345 subject\ndiff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-let a = 1;\n+let b = 1;";
    let mut e = editor(text, "");
    let mut v = View::status(
        Repo {
            root: "/repo".into(),
        },
        Snapshot::default(),
    );
    v.kind = Kind::Log(vec!["HEAD".into()], vec![]);
    v.rows = text
        .lines()
        .map(|t| Row {
            text: t.into(),
            action: None,
        })
        .collect();
    v.rows[0].action = Some(RowAction::Commit("abc12345".into()));
    v.margin = crate::magit::margin::Margin::for_kind(&v.kind);
    v.stamps.insert(
        "abc12345".into(),
        Stamp {
            author: "Ann".into(),
            time: crate::magit::margin::now() - 3 * 86_400,
            stat: None,
        },
    );
    v.refine = true;
    v.fontify = true;
    e.path = Some("/state/magit-views/opaque-hash".into());
    e.magit = Some(Box::new(v));
    e.readonly = true;
    let mut screen = Screen::new(80, 9);
    e.run_ex("set nonumber");
    screen.draw(&e);
    let row = screen.row(0);
    assert!(row.starts_with("abc12345 subject"), "{row}");
    assert!(row.contains("Ann") && row.contains(" 3 days"), "{row}");
    assert!(row.trim_end().ends_with("3 days"), "{row}");
    // Removed and added lines: tinted, with the changed word reversed.
    let buf = screen.term.backend().buffer();
    assert_eq!(buf[(0, 5)].fg, Color::Red);
    assert_eq!(buf[(1, 5)].bg, Color::Indexed(52));
    assert!(buf[(5, 5)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(1, 5)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(5, 6)].modifier.contains(Modifier::REVERSED));
    assert_eq!(buf[(1, 6)].bg, Color::Indexed(22));
}
#[test]
fn magit_diff_paints_trailing_whitespace_on_added_lines() {
    use crate::magit::repo::{Repo, Snapshot};
    use crate::magit::{Kind, Row, View};
    use ratatui::style::Color;
    let text = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old  \n+new  ";
    let mut e = editor(text, "");
    let mut v = View::status(
        Repo {
            root: "/repo".into(),
        },
        Snapshot::default(),
    );
    v.kind = Kind::Patch("abc".into());
    v.rows = text
        .lines()
        .map(|t| Row {
            text: t.into(),
            action: None,
        })
        .collect();
    e.path = Some("/state/magit-views/opaque".into());
    e.magit = Some(Box::new(v));
    e.readonly = true;
    let mut screen = Screen::new(40, 8);
    e.run_ex("set nonumber");
    screen.draw(&e);
    let buf = screen.term.backend().buffer();
    // Added line: trailing spaces painted; removed line: not (t = added only).
    assert_eq!(buf[(4, 5)].bg, Color::Red);
    assert_ne!(buf[(4, 4)].bg, Color::Red);
    assert_ne!(buf[(2, 5)].bg, Color::Red);
}

#[test]
fn mouse_drag_selects_and_double_click_selects_a_word() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut e = editor("hello world\nsecond line\n", "");
    let mut s = Screen::new(80, 12);
    s.draw(&e);
    let area = ratatui::layout::Rect::new(0, 0, 80, 12);
    let gutter = super::render::gutter_width(&e) as u16;
    let at = |kind, col: u16, row| MouseEvent {
        kind,
        column: gutter + col,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let down = MouseEventKind::Down(MouseButton::Left);
    let drag = MouseEventKind::Drag(MouseButton::Left);
    super::render::mouse(&mut e, &mut s.view, area, at(down, 2, 0));
    assert_eq!(e.mode, Mode::Normal);
    super::render::mouse(&mut e, &mut s.view, area, at(drag, 3, 1));
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        at(MouseEventKind::Up(MouseButton::Left), 3, 1),
    );
    assert_eq!(e.mode, Mode::Visual { anchor: (0, 2) });
    assert_eq!((e.cur.line, e.cur.byte), (1, 3));
    // A plain click ends the selection.
    super::render::mouse(&mut e, &mut s.view, area, at(down, 7, 0));
    assert_eq!(e.mode, Mode::Normal);
    // A second click there selects "world".
    super::render::mouse(&mut e, &mut s.view, area, at(down, 7, 0));
    assert_eq!(e.mode, Mode::Visual { anchor: (0, 6) });
    assert_eq!((e.cur.line, e.cur.byte), (0, 10));
}

#[test]
fn right_click_opens_neovims_popup_menu() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut e = editor("hello world\nsecond line\n", "");
    let mut s = Screen::new(40, 12);
    s.draw(&e);
    let area = ratatui::layout::Rect::new(0, 0, 40, 12);
    let gutter = super::render::gutter_width(&e) as u16;
    let at = |kind, col: u16, row| MouseEvent {
        kind,
        column: gutter + col,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let right = MouseEventKind::Down(MouseButton::Right);
    // popup_setpos: the click moves the cursor, then the menu opens below it.
    super::render::mouse(&mut e, &mut s.view, area, at(right, 7, 1));
    assert_eq!(e.cur.pos(), (1, 7));
    s.draw(&e);
    assert!(s.row(2).contains("Go to definition"), "{}", s.row(2));
    assert!(s.row(4).contains("Paste") && s.row(5).contains("Select All"));
    // Hovering highlights; a click chooses.
    let b = s.view.menu_box.unwrap();
    let on = |row| MouseEvent {
        column: b.x + 1,
        ..at(MouseEventKind::Moved, 0, row)
    };
    super::render::mouse(&mut e, &mut s.view, area, on(b.y + 3));
    assert_eq!(e.menu.as_ref().unwrap().sel, Some(3));
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        ..on(b.y + 3)
    };
    super::render::mouse(&mut e, &mut s.view, area, click);
    assert_eq!(e.mode, Mode::VisualLine { anchor: 0 }, "Select All");
    assert!(e.menu.is_none());
    // Inside a selection the cursor stays and the menu acts on it.
    super::render::mouse(&mut e, &mut s.view, area, at(right, 2, 0));
    assert_eq!(e.mode, Mode::VisualLine { anchor: 0 });
    s.draw(&e);
    assert!((0..12).any(|y| s.row(y).contains("Copy")));
    // A click elsewhere only closes it.
    let away = at(MouseEventKind::Down(MouseButton::Left), 30, 9);
    super::render::mouse(&mut e, &mut s.view, area, away);
    assert!(e.menu.is_none());
    assert_eq!(e.mode, Mode::VisualLine { anchor: 0 });
}

#[test]
fn occur_shades_line_numbers_and_highlights_matches() {
    use ratatui::style::Color;
    let src = crate::buffer::Buffer::from_text("a foo b\nnone\n");
    let (o, lines) = crate::occur::search(&src, "f".into(), "f", "foo", None).unwrap();
    let mut e = editor("", "");
    crate::occur::show(&mut e, o, &lines, None);
    let mut s = Screen::new(60, 6);
    e.run_ex("set nonumber");
    s.draw(&e);
    let b = s.term.backend().buffer();
    let row: String = (0..15).map(|x| b[(x, 1)].symbol().to_string()).collect();
    assert_eq!(row, "      1:a foo b");
    assert_eq!(b[(3, 1)].fg, Color::DarkGray);
    assert_eq!(b[(10, 1)].bg, Color::Yellow, "the match");
    assert_ne!(b[(8, 1)].bg, Color::Yellow, "not the text around it");
}

/// A session on the first of `files` (written to a temp dir), numbers off.
fn session_screen(
    w: u16,
    h: u16,
    files: &[(&str, &str)],
) -> (crate::session::Session, Screen, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    for (name, text) in files {
        std::fs::write(d.path().join(name), text).unwrap();
    }
    let first = files.first().map(|f| d.path().join(f.0));
    let (mut s, _) =
        crate::session::Session::open(first, &Config::default(), &d.path().join("swap")).unwrap();
    s.handle_keys(":set nonumber<Enter>");
    let mut sc = Screen::new(w, h);
    // As the app does: a draw sizes the windows before any keys.
    sc.draw_session(&mut s);
    (s, sc, d)
}

impl Screen {
    fn draw_session(&mut self, s: &mut crate::session::Session) {
        let cfg = &self.cfg;
        self.term
            .draw(|f| super::windows::draw_session(f, s, cfg, Duration::from_secs(1)))
            .unwrap();
    }
}

#[test]
fn two_windows_two_status_lines() {
    let (mut s, mut sc, d) = session_screen(40, 10, &[("a", "alpha\n"), ("b", "beta\n")]);
    s.handle_keys(&format!(":split {}<Enter>", d.path().join("b").display()));
    sc.draw_session(&mut s);
    assert_eq!(sc.row(0), "beta");
    assert!(sc.row(3).contains("b"), "{}", sc.row(3)); // status of the top window
    assert_eq!(sc.row(4), "alpha");
    assert!(sc.row(8).contains("a"), "{}", sc.row(8)); // status of the bottom window
    assert_eq!(sc.cursor(), (0, 0)); // in the new, top window
}

#[test]
fn vsplit_draws_divider() {
    let (mut s, mut sc, _d) = session_screen(41, 6, &[("a", "alpha\n")]);
    s.handle_keys(":vsplit<Enter>");
    sc.draw_session(&mut s);
    assert!(sc.row(0).starts_with("alpha"), "{}", sc.row(0));
    assert_eq!(sc.row(0).chars().nth(19), Some('│')); // new window: 20 cols incl. divider
}

#[test]
fn inactive_same_buffer_no_visual() {
    let (mut s, mut sc, _d) = session_screen(40, 10, &[("a", "alpha\n")]);
    s.handle_keys(":split<Enter>vl");
    sc.draw_session(&mut s);
    let b = sc.term.backend().buffer();
    // Visual is reverse video; the cursor's own cell is left plain.
    assert!(b[(0, 0)].modifier.contains(Modifier::REVERSED)); // top (active) shows Visual
    assert!(!b[(0, 4)].modifier.contains(Modifier::REVERSED)); // bottom does not
}

#[test]
fn other_window_cursor_clamped_after_delete() {
    let (mut s, mut sc, _d) = session_screen(40, 10, &[("a", &"x\n".repeat(50))]);
    s.handle_keys("50G:split<Enter>ggdG");
    sc.draw_session(&mut s); // must not panic
    s.handle_keys("<C-w>j");
    assert_eq!(s.ed.cur.line, 0);
}

#[test]
fn space_w_numbers_and_dims_the_windows() {
    let (mut s, mut sc, _d) = session_screen(40, 12, &[("a", "alpha\n")]);
    s.handle_keys(":vsplit<Enter>:split<Enter> w");
    sc.draw_session(&mut s);
    let screen: Vec<String> = (0..12).map(|r| sc.row(r)).collect();
    let text = screen.join("\n");
    for n in ["│ 1 │", "│ 2 │", "│ 3 │"] {
        assert!(text.contains(n), "{text}");
    }
    let b = sc.term.backend().buffer();
    assert!(b[(0, 0)].modifier.contains(Modifier::DIM));
    s.handle_keys("1");
    sc.draw_session(&mut s);
    assert!(
        !sc.term.backend().buffer()[(0, 0)]
            .modifier
            .contains(Modifier::DIM)
    );
}

#[test]
fn clicking_a_window_focuses_it() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let click = |column, row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let (mut s, mut sc, d) =
        session_screen(41, 12, &[("a", "alpha\nalps\n"), ("b", "beta\nbest\n")]);
    let screen = ratatui::layout::Rect::new(0, 0, 41, 12);
    // Left: b (active). Right: a.
    s.handle_keys(&format!(":vsplit {}<Enter>", d.path().join("b").display()));
    sc.draw_session(&mut s);
    let left = s.active;
    s.mouse(screen, click(23, 1));
    assert_ne!(s.active, left);
    assert_eq!(
        (s.ed.buf.text().as_str(), s.ed.cur.pos()),
        ("alpha\nalps", (1, 3))
    );
    // And back, onto b's second line.
    s.mouse(screen, click(1, 1));
    assert_eq!(
        (s.active, s.ed.buf.text().as_str(), s.ed.cur.pos()),
        (left, "beta\nbest", (1, 1))
    );
    // The divider moves nothing; a status line focuses its window, as in Vim,
    // leaving the cursor where it was.
    s.mouse(screen, click(19, 1));
    assert_eq!((s.active, s.ed.cur.pos()), (left, (1, 1)));
    s.mouse(screen, click(25, 10));
    assert_eq!(
        (s.ed.buf.text().as_str(), s.ed.cur.pos()),
        ("alpha\nalps", (1, 3))
    );
}

#[test]
fn zap_reaches_every_window() {
    let (mut s, mut sc, d) = session_screen(41, 8, &[("a", "apple zed\n"), ("b", "zoo plum\n")]);
    s.handle_keys(&format!(":vsplit {}<Enter>", d.path().join("b").display()));
    sc.draw_session(&mut s);
    // As the app does after a draw.
    let p = s.placed().into_iter().find(|p| p.id == s.active).unwrap();
    s.ed.viewport = Some(crate::zap::Viewport::of(p.text(), &s.ed, s.win().view));
    let left = s.active;
    s.handle_keys(" sz");
    sc.draw_session(&mut s);
    // `zoo` here and `zed` in the right window (from column 20) are labelled.
    let b = sc.term.backend().buffer();
    assert_eq!(b[(0, 0)].bg, ratatui::style::Color::Magenta);
    assert_eq!(
        b[(26, 0)].bg,
        ratatui::style::Color::Magenta,
        "{}",
        sc.row(0)
    );
    let label = b[(26, 0)].symbol().to_string();
    s.handle_keys(&label);
    assert_ne!(s.active, left);
    assert_eq!(
        (s.ed.buf.text().as_str(), s.ed.cur.pos()),
        ("apple zed", (0, 6))
    );
    // A word only the other window has jumps straight there.
    s.handle_keys(" splum");
    assert_eq!((s.active, s.ed.cur.pos()), (left, (0, 4)));
}

#[test]
fn which_key_lists_the_leader_keys() {
    let (mut s, mut sc, _d) = session_screen(80, 20, &[("a", "alpha\n")]);
    s.handle_keys(" ");
    s.which_key = true; // The app sets it after a short pause.
    sc.draw_session(&mut s);
    let text: String = (0..20).map(|r| sc.row(r) + "\n").collect();
    assert!(text.contains(" SPC "), "{text}");
    assert!(text.contains("s → zap to a word"), "{text}");
    assert!(text.contains("o → +org"), "{text}");
    assert!(text.contains("h → help: search commands"), "{text}");
    // `Space o` shows Org's.
    s.handle_keys("o");
    assert!(!s.which_key);
    s.which_key = true;
    sc.draw_session(&mut s);
    let text: String = (0..20).map(|r| sc.row(r) + "\n").collect();
    assert!(text.contains("a → org-agenda"), "{text}");
    // A key ends it.
    s.handle_keys("<Esc>");
    sc.draw_session(&mut s);
    assert!(!(0..20).any(|r| sc.row(r).contains("org-agenda")));
}

#[test]
fn colon_line_shows_help_for_the_command() {
    let mut s = Screen::new(80, 12);
    let e = editor("a", ":sp");
    s.draw(&e);
    let text: String = (0..12).map(|r| s.row(r) + "\n").collect();
    assert!(text.contains(":[N]sp[lit] [file]"), "{text}");
    assert!(
        text.contains("split the window, optionally opening a file"),
        "{text}"
    );
    // Tab's names say what they do too.
    let e = editor("a", ":clo<Tab>");
    s.draw(&e);
    let text: String = (0..12).map(|r| s.row(r) + "\n").collect();
    assert!(text.contains("close this window"), "{text}");
    // No command, no box.
    let e = editor("a", ":%");
    s.draw(&e);
    assert!(!(0..12).any(|r| s.row(r).contains('╭')));
}

#[test]
fn help_lists_commands_and_puts_the_pick_on_the_colon_line() {
    let mut e = editor("a", "");
    e.run_ex("help vsplit");
    let Mode::Pick(p) = &e.mode else {
        panic!("{:?}", e.mode)
    };
    assert!(
        p.rows[0].text.starts_with(":[N]vs[plit] [file]"),
        "{}",
        p.rows[0].text
    );
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Enter));
    let Mode::Command(cl) = &e.mode else {
        panic!("{:?}", e.mode)
    };
    assert_eq!(cl.text, "vsplit");
    // One that needs an argument comes with the space for it.
    let mut e = editor("a", "");
    e.run_ex("help move lines");
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Enter));
    let Mode::Command(cl) = &e.mode else {
        panic!("{:?}", e.mode)
    };
    assert_eq!(cl.text, "move ");
}

#[test]
fn selectmode_mouse_drags_double_and_triple_clicks_select() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut e = editor("hello world\nsecond line\n", ":set slm=mouse<CR>");
    let mut s = Screen::new(80, 12);
    s.draw(&e);
    let area = ratatui::layout::Rect::new(0, 0, 80, 12);
    let gutter = super::render::gutter_width(&e) as u16;
    let at = |kind, col: u16, row, modifiers| MouseEvent {
        kind,
        column: gutter + col,
        row,
        modifiers,
    };
    let none = KeyModifiers::NONE;
    let down = MouseEventKind::Down(MouseButton::Left);
    let drag = MouseEventKind::Drag(MouseButton::Left);
    let mut mouse =
        |e: &mut crate::editor::Editor, ev| super::render::mouse(e, &mut s.view, area, ev);
    mouse(&mut e, at(down, 2, 0, none));
    mouse(&mut e, at(drag, 3, 1, none));
    assert_eq!(e.mode, Mode::Visual { anchor: (0, 2) });
    assert!(e.select_mode(), "a drag starts Select");
    assert_eq!(super::render::status_label(&e), "SELECT");
    // Typing replaces it.
    e.handle_key(crate::key::Key::ch('x'));
    assert_eq!(e.buf.text(), "hexnd line");
    assert_eq!(e.mode, Mode::Insert);
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Esc));
    // Double-click: the word, in Select; triple-click: the line.
    mouse(&mut e, at(down, 1, 0, none));
    mouse(&mut e, at(down, 1, 0, none));
    assert_eq!(e.mode, Mode::Visual { anchor: (0, 0) });
    assert!(e.select_mode());
    mouse(&mut e, at(down, 1, 0, none));
    assert_eq!(e.mode, Mode::VisualLine { anchor: 0 });
    assert!(e.select_mode());
    // A click ends it; Shift-click extends from the cursor, in Select.
    mouse(&mut e, at(down, 0, 0, none));
    assert_eq!(e.mode, Mode::Normal);
    assert!(!e.select_mode());
    mouse(&mut e, at(down, 4, 0, KeyModifiers::SHIFT));
    assert_eq!(e.mode, Mode::Visual { anchor: (0, 0) });
    assert!(e.select_mode());
}

#[test]
fn a_mouse_selection_from_insert_goes_back_to_insert() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut e = editor("hello world\n", ":set slm=mouse<CR>i");
    let mut s = Screen::new(80, 12);
    s.draw(&e);
    let area = ratatui::layout::Rect::new(0, 0, 80, 12);
    let gutter = super::render::gutter_width(&e) as u16;
    let at = |kind, col: u16| MouseEvent {
        kind,
        column: gutter + col,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        at(MouseEventKind::Down(MouseButton::Left), 0),
    );
    super::render::mouse(
        &mut e,
        &mut s.view,
        area,
        at(MouseEventKind::Drag(MouseButton::Left), 4),
    );
    assert!(e.select_mode());
    e.handle_key(crate::key::Key::new(crate::key::KeyCode::Backspace));
    assert_eq!(e.buf.text(), " world");
    assert_eq!(e.mode, Mode::Insert);
}
