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
        s.cfg.numbers = false;
        let mut e = editor(text, "");
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
    s.cfg.numbers = false;
    let mut e = editor(&vec!["       x"; 60].join("\n"), "");
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
fn wrap_mode() {
    let mut s = Screen::new(20, 5);
    s.cfg.wrap = true;
    let e = editor(&format!("{}\nb", "x".repeat(40)), "");
    s.draw(&e);
    assert_eq!(s.row(0), format!("  1 {}", "x".repeat(16)));
    assert_eq!(s.row(1), format!("    {}", "x".repeat(16)));
    assert_eq!(s.row(2), format!("    {}", "x".repeat(8)));
}

#[test]
fn wrap_mode_cursor_on_continuation_row() {
    let mut s = Screen::new(20, 5);
    s.cfg.wrap = true;
    let e = editor(&"x".repeat(40), "$");
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
fn visual_selection_is_reversed() {
    let mut s = Screen::new(20, 5);
    let e = editor("ab\ncd\nef", "Vj");
    s.draw(&e);
    let b = s.term.backend().buffer();
    assert!(b[(4, 0)].modifier.contains(Modifier::REVERSED));
    assert!(b[(4, 1)].modifier.contains(Modifier::REVERSED));
    assert!(!b[(4, 2)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn relative_numbers() {
    let mut s = Screen::new(20, 5);
    s.cfg.relative_numbers = true;
    let e = editor("a\nb\nc", "j");
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
        s.cfg.wrap = true;
        let e = editor(&text, keys);
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
    s.cfg.wrap = true;
    let e = editor(&text, "$");
    s.draw(&e);
    assert_eq!(cell_at_cursor(&mut s), "Z");
    let e = editor(&text, "$0");
    s.draw(&e);
    assert_eq!(s.cursor(), (4, 0), "back at the start of the line");
    assert_eq!(s.row(0), format!("  1 {}", &long[..36]));
    let e = editor(&text, "$j");
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
fn mouse_wheel_scrolls_picker_matches() {
    use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    let mut s = Screen::new(40, 8);
    let mut e = editor(&"match\n".repeat(20), " k");
    let area = ratatui::layout::Rect::new(2, 3, 40, 8);
    let mut wheel = |e: &mut Editor, kind| {
        super::render::mouse(
            e,
            &mut s.view,
            &s.cfg,
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
    s.cfg.wrap = false;
    let mut e = editor("a\nb\nc\nd\ne\nf\ng\nh", "");
    let area = ratatui::layout::Rect::new(2, 3, 20, 6);
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    super::render::mouse(
        &mut e,
        &mut s.view,
        &s.cfg,
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
        &s.cfg,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 6, 4),
    );
    assert_eq!(e.cur.line, 4);
    e = editor("ab漢x\n\tz", "");
    s.view = View::default();
    s.cfg.wrap = true;
    let area = ratatui::layout::Rect::new(0, 0, 9, 6);
    super::render::mouse(
        &mut e,
        &mut s.view,
        &s.cfg,
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
        &s.cfg,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 4, 1),
    );
    assert_eq!(e.cur.pos(), (1, 0), "a tab selects its source byte");
    e = editor(&"abcdefghij\n".repeat(10), "");
    s.view = View::default();
    super::render::mouse(
        &mut e,
        &mut s.view,
        &s.cfg,
        area,
        mouse(MouseEventKind::ScrollDown, 5, 0),
    );
    assert_eq!((s.view.top, s.view.top_row), (1, 1));
    super::render::mouse(
        &mut e,
        &mut s.view,
        &s.cfg,
        area,
        mouse(MouseEventKind::Down(MouseButton::Left), 6, 0),
    );
    assert_eq!(e.cur.pos(), (1, 7));
    super::render::mouse(
        &mut e,
        &mut s.view,
        &s.cfg,
        area,
        mouse(MouseEventKind::ScrollUp, 5, 0),
    );
    assert_eq!((s.view.top, s.view.top_row), (0, 0));
    e = editor("abcdefghijklmnopqrstuvwxyz", "");
    s.cfg.wrap = false;
    s.view.left = 10;
    super::render::mouse(
        &mut e,
        &mut s.view,
        &s.cfg,
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
fn splash_shows_actions_and_recent_files() {
    let dir = tempfile::tempdir().unwrap();
    let recent = dir.path().join("recent");
    crate::pick::recent::record(&recent, std::path::Path::new("/tmp/notes.txt"), None);
    let mut e = editor("", "");
    e.project = std::sync::Arc::new(crate::pick::Project::new(None, Some(recent)));
    let mut s = Screen::new(80, 40);
    s.draw(&e);
    let text = (0..38).map(|y| s.row(y)).collect::<Vec<_>>().join("\n");
    for label in ["Dr. Fred", "Recent files", "Find file", "Grep", "notes.txt"] {
        assert!(text.contains(label), "missing {label}");
    }
    for i in 0..5 {
        crate::pick::recent::record(
            e.project.recent_file.as_deref().unwrap(),
            std::path::Path::new(&format!("/tmp/file{i}.txt")),
            None,
        );
    }
    let mut standard = Screen::new(80, 24);
    standard.draw(&e);
    assert!(
        (0..22).any(|y| standard.row(y).contains("Dr. Fred")),
        "keep the logo on a standard terminal with a full recent list"
    );
    crate::pick::recent::record(
        e.project.recent_file.as_deref().unwrap(),
        std::path::Path::new("/tmp/notes.txt"),
        None,
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
