//! Regression tests for bugs found by randomized testing (`tests/fuzz.rs`),
//! shrunk to minimal reproductions. Each one failed before its fix.

use fred::buffer::Buffer;
use fred::editor::{Editor, Mode};
use fred::key::parse_keys;
use fred::text::floor_grapheme;
use std::sync::mpsc;
use std::time::Duration;

fn ed(text: &str, keys: &str) -> Editor {
    let mut e = Editor::new(Buffer::from_text(text));
    feed(&mut e, keys);
    e
}

fn feed(e: &mut Editor, keys: &str) {
    for k in parse_keys(keys) {
        e.handle_key(k);
    }
}

fn cmdline(e: &Editor) -> &str {
    match &e.mode {
        Mode::Command(cl) => &cl.text,
        m => panic!("not on the command line: {m:?}"),
    }
}

fn is_err(e: &Editor) -> bool {
    matches!(e.msg, Some((_, true)))
}

/// Ctrl-W on the `:` `/` `?` line panics when the char before the deleted
/// word is a multi-byte non-word char: the word start is computed as `i + 1`
/// after that char's first byte, which is not a char boundary, so
/// `String::replace_range` panics ("start of range should be a character
/// boundary", src/editor.rs:309).
/// Fuzz signature: `panic src/editor.rs:309`.
#[test]
fn cmdline_ctrl_w_after_multibyte_char_panics() {
    let e = ed("", "?→<C-w>");
    assert_eq!(cmdline(&e), "", "Ctrl-W deletes the lone punctuation char");
    let e = ed("", ":a→b<C-w>");
    assert_eq!(cmdline(&e), "a→", "Ctrl-W deletes the word after the arrow");
}

/// A huge address offset overflows `i64` in `parse_addr` (src/ex/addr.rs:100):
/// "attempt to add with overflow" (a panic in debug builds; in release the
/// sum silently wraps). Expected: `? invalid address`.
/// Fuzz signature: `panic src/ex/addr.rs:100`.
#[test]
fn huge_address_offset_overflows() {
    let e = ed("a\nb", ":+9223372036854775807<Enter>");
    assert!(is_err(&e), "expected an error, got {:?}", e.msg);
    assert_eq!(e.buf.text(), "a\nb");
}

/// `<C-r>` is allowed in Visual-line mode; when the redo deletes lines, the
/// selection's anchor is left past the last line. The next Visual operator
/// then reads that line and panics in ropey (`Rope::line` out of bounds via
/// `vim::ops::lines_text`).
/// Fuzz signature: `visual-anchor-out-of-range` (shrunk from seed 882:
/// text "\nΩ", keys "eO<Esc>d+uV<C-r>").
#[test]
fn redo_in_visual_line_leaves_anchor_past_end() {
    // Delete the last line, undo, select it, redo the delete.
    let mut e = ed("a\nb\nc", "GdduV<C-r>");
    assert_eq!(e.buf.text(), "a\nb");
    if let Mode::VisualLine { anchor } = e.mode {
        assert!(
            anchor < e.buf.len_lines(),
            "anchor {anchor} but only {} lines",
            e.buf.len_lines()
        );
    }
    feed(&mut e, "d"); // currently panics: line index 2 out of bounds
}

/// When an ex command fails after it already edited (a `g` whose command
/// fails on a later line), `ex::run` rolls back by pushing the partial edits
/// as an undo group and undoing it. That leaves the failed command's partial
/// edits on the redo stack (and wipes what was there): `<C-r>` then applies
/// half of a command that reported an error.
/// Fuzz signature: `redo-reached-unseen-state` (shrunk: text "a\nb",
/// keys ":g/./s/a<Enter>").
#[test]
fn failed_ex_command_partial_edits_are_redoable() {
    // `j` joins line 1 and 2, then fails on the last line ("invalid address").
    let e = ed("a\nb\nc", ":g/./j<Enter>");
    assert!(is_err(&e));
    assert_eq!(e.buf.text(), "a\nb\nc", "the failed command is rolled back");
    let e = ed("a\nb\nc", ":g/./j<Enter><C-r>");
    assert_eq!(e.buf.text(), "a\nb\nc", "nothing to redo");
    // The redo history from before the failed command survives it.
    let e = ed("a\nb\nc", "xu:g/./j<Enter><C-r>");
    assert_eq!(e.buf.text(), "\nb\nc", "<C-r> redoes the `x`");
}

/// ropey's default `unicode_lines` feature makes the rope treat CR, VT, FF,
/// NEL, U+2028 and U+2029 as line breaks, but the rest of fred (the file
/// format, `Buffer::line`, the spec's "`\n` line endings only") treats them
/// as ordinary characters. Line numbers then disagree with the file and
/// commands edit the wrong text.
/// Fuzz signature: `line-count-disagrees-with-newlines` (2 of every 3 cases
/// that contain such a character).
#[test]
fn non_lf_line_breaks_split_lines() {
    // A 2-line file whose first line contains U+2028: `:2d` deletes the "b"
    // after the separator instead of line 2 ("c").
    let e = ed("a\u{2028}b\nc\n", ":2d<Enter>");
    assert_eq!(String::from_utf8(e.buf.to_bytes()).unwrap(), "a\u{2028}b\n");
    // A form-feed line (^L page break) gets a phantom empty line after it,
    // and deleting that "line" glues the ^L onto the next one.
    let e = ed("foo\n\u{c}\nbar\n", "");
    assert_eq!(e.buf.len_lines(), 3);
    let e = ed("foo\n\u{c}\nbar\n", "jjdd");
    assert_eq!(String::from_utf8(e.buf.to_bytes()).unwrap(), "foo\n\u{c}\n");
    // A stray CR inside a line: `dd` deletes only up to the CR.
    let e = ed("a\rb\nc\n", "dd");
    assert_eq!(String::from_utf8(e.buf.to_bytes()).unwrap(), "c\n");
}

/// `n`/`N` with a count recompiles the regex (and re-searches) once per
/// step: ~250 µs per step in a debug build, ~21 µs in release, so a count
/// of 1000000 (the cap) freezes the editor for ~20 s in release even on a
/// two-line buffer (and the fuzz harness hung on it).
/// Fuzz signature: hang (watchdog), seed 369 of the first harness version.
#[test]
fn counted_search_freezes() {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let e = ed("foo bar\nbaz foo\n", "/foo<Enter>100000n");
        let _ = tx.send(e.cur.pos());
    });
    // ~25 s in a debug build today.
    let pos = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("100000n did not finish within 5 s");
    assert_eq!(pos, (1, 4));
}

/// `Editor::paste` in Insert mode inserts the text but skips
/// `clamp_cursor`, so when the pasted text joins the grapheme cluster after
/// the cursor (a base letter before a combining mark, one regional
/// indicator before another, an emoji ending in ZWJ before another emoji),
/// the cursor ends up inside that cluster. The next typed key then splits
/// the cluster: here the accent moves from the pasted `e` onto the `x`.
/// Fuzz signature: `cursor-not-on-grapheme-boundary` after a paste step.
#[test]
fn paste_in_insert_mode_leaves_cursor_inside_grapheme() {
    for (text, paste) in [("\u{301}", "e"), ("🇸", "🇺"), ("👩", "👨\u{200d}")] {
        let mut e = ed(text, "i");
        e.paste(paste);
        let line = e.buf.line(e.cur.line);
        assert_eq!(
            floor_grapheme(&line, e.cur.byte),
            e.cur.byte,
            "pasting {paste:?} before {text:?}: cursor {:?} is inside a grapheme of {line:?}",
            e.cur.pos()
        );
    }
}

/// `.` replayed the recorded keys literally: when the repeated change's
/// motion failed (`cff` with no `f` left), the change never entered Insert
/// mode, so the recorded insert text ran as Normal-mode commands, including
/// a `u` that undid inside the open undo group and later made ropey panic
/// ("index out of bounds" at rope.rs:952).
/// Fuzz signature: `panic ropey rope.rs:952`, seed 112461.
#[test]
fn dot_repeat_whose_motion_fails_runs_insert_text_as_commands() {
    let e = ed("—f", "cffdwu<Esc>.");
    assert_eq!(
        e.buf.text(),
        "dwu",
        "a repeat whose motion fails does nothing"
    );
    assert_eq!(e.mode, Mode::Normal);
    let e = ed("—f", "cffdwu<Esc>.uu");
    assert_eq!(e.buf.text(), "—f");
}
