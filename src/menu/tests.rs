use super::*;
use crate::editor::tests::ed;

fn labels(e: &Editor) -> Vec<&'static str> {
    e.menu
        .as_ref()
        .map_or(vec![], |m| m.items.iter().map(|i| i.1).collect())
}

fn clip(text: &str) {
    clipboard::set(&clipboard::from_clip(text));
}

/// `keys`, then the menu's `item`.
fn after(text: &str, keys: &str, item: Item) -> Editor {
    let mut e = ed(text, keys);
    open(&mut e, (0, 0));
    choose(&mut e, item);
    e
}

#[test]
fn items_follow_the_mode() {
    let mut e = ed("hello", "");
    open(&mut e, (0, 0));
    assert_eq!(labels(&e), ["Go to definition", "", "Paste", "Select All"]);
    let mut e = ed("hello", "vl");
    open(&mut e, (0, 0));
    assert_eq!(
        labels(&e),
        [
            "Go to definition",
            "",
            "Cut",
            "Copy",
            "Paste",
            "Delete",
            "Select All"
        ]
    );
    let mut e = ed("  ", "i");
    open(&mut e, (0, 0));
    assert_eq!(labels(&e), ["Paste", "Select All"]);
    let mut e = ed("see https://x.org/a. ok", "6l");
    open(&mut e, (0, 0));
    assert_eq!(labels(&e)[0], "Open in web browser");
    assert_eq!(e.menu.unwrap().url.as_deref(), Some("https://x.org/a"));
}

#[test]
fn cut_copy_and_delete() {
    let e = after("hello", "vl", Item::Copy);
    assert_eq!(
        (e.buf.text(), e.mode.clone()),
        ("hello".into(), Mode::Normal)
    );
    assert_eq!(clipboard::get().as_deref(), Some("he"));
    let e = after("hello", "lvl", Item::Cut);
    assert_eq!(e.buf.text(), "hlo");
    assert_eq!(
        (clipboard::get().as_deref(), e.reg.text.as_str()),
        (Some("el"), "el")
    );
    // Delete goes to the black hole: neither register changes.
    clip("kept");
    let mut e = ed("hello", "yl");
    e.handle_key(Key::ch('v'));
    open(&mut e, (0, 0));
    choose(&mut e, Item::Delete);
    assert_eq!(e.buf.text(), "ello");
    assert_eq!(
        (clipboard::get().as_deref(), e.reg.text.as_str()),
        (Some("kept"), "h")
    );
    // Lines.
    let e = after("a\nb\nc", "Vj", Item::Cut);
    assert_eq!(
        (e.buf.text(), clipboard::get().as_deref()),
        ("c".into(), Some("a\nb\n"))
    );
    // One undo step each.
    let mut e = after("hello", "lvl", Item::Cut);
    e.handle_key(Key::ch('u'));
    assert_eq!(e.buf.text(), "hello");
}

#[test]
fn paste() {
    clip("XY");
    // `gP`: before the cursor, which ends up after the text.
    let e = after("hello", "l", Item::Paste);
    assert_eq!((e.buf.text(), e.cur.byte), ("hXYello".into(), 3));
    // Over a selection, even one ending the line; the unnamed register stays.
    let e = after("abc", "yl$v", Item::Paste);
    assert_eq!((e.buf.text(), e.reg.text.as_str()), ("abXY".into(), "a"));
    // In Insert mode, still inserting.
    let e = after("abc", "a", Item::Paste);
    assert_eq!(
        (e.buf.text(), e.mode.clone()),
        ("aXYbc".into(), Mode::Insert)
    );
    // Lines: above the cursor's line, in place of lines, or splitting a line.
    clip("x\ny\n");
    assert_eq!(after("a\nb", "j", Item::Paste).buf.text(), "a\nx\ny\nb");
    assert_eq!(after("a\nb\nc", "GV", Item::Paste).buf.text(), "a\nb\nx\ny");
    assert_eq!(after("a\nb", "ggVG", Item::Paste).buf.text(), "x\ny");
    assert_eq!(after("abc", "lv", Item::Paste).buf.text(), "a\nx\ny\nc");
    // Characters in place of lines go in as a line.
    clip("z");
    assert_eq!(after("a\nb\nc", "jV", Item::Paste).buf.text(), "a\nz\nc");
}

#[test]
fn select_all() {
    let e = after("a\nb\nc", "j", Item::SelectAll);
    assert_eq!(
        (e.mode.clone(), e.cur.line),
        (Mode::VisualLine { anchor: 0 }, 2)
    );
    let e = after("ab\ncd", "lv", Item::SelectAll);
    assert_eq!(
        (e.mode.clone(), e.cur.pos()),
        (Mode::Visual { anchor: (0, 0) }, (1, 1))
    );
    let e = after("ab\ncd", "i", Item::SelectAll);
    assert_eq!(e.mode, Mode::VisualLine { anchor: 0 });
}

#[test]
fn keys_choose_or_close() {
    clip("Z");
    let mut e = ed("hello", "");
    open(&mut e, (0, 0));
    // Down skips the separator; Enter chooses.
    for k in ["<Down>", "<Down>", "<CR>"] {
        e.handle_key(crate::key::parse_keys(k)[0]);
    }
    assert_eq!((e.buf.text(), e.menu.is_none()), ("Zhello".into(), true));
    let mut e = ed("hello", "");
    open(&mut e, (0, 0));
    e.handle_key(Key::new(KeyCode::Esc));
    assert!(e.menu.is_none());
    // Any other key closes it and does what it does.
    open(&mut e, (0, 0));
    e.handle_key(Key::ch('x'));
    assert_eq!((e.buf.text(), e.menu.is_none()), ("ello".into(), true));
}

#[test]
fn urls() {
    assert_eq!(
        url_at("(see https://a.b/c?d=1).", 9),
        Some("https://a.b/c?d=1")
    );
    assert_eq!(url_at("http://x.y", 0), Some("http://x.y"));
    assert_eq!(url_at("ftp://x.y https://z", 2), None);
    assert_eq!(url_at("", 0), None);
}
