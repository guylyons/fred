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
    assert!(s.row(3).ends_with("1:1"), "{}", s.row(3));
    assert_eq!(s.row(4), "");
    assert_eq!(s.cursor(), (4, 0));
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
        e.popup = Some(Popup { items: vec!["hello".into()], sel: Some(0), start: 0, typed: "he".into() });
        s.draw(&e);
        let e = editor("abc", ":w");
        s.draw(&e);
    }
}
