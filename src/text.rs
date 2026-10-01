//! Grapheme and display-width helpers. Byte offsets are always into a single line.

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

/// Byte offset of the grapheme boundary after `byte` (or `s.len()`).
pub fn next_grapheme(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    let mut c = GraphemeCursor::new(byte, s.len(), true);
    c.next_boundary(s, 0).ok().flatten().unwrap_or(s.len())
}

/// Byte offset of the grapheme boundary before `byte` (or 0).
pub fn prev_grapheme(s: &str, byte: usize) -> usize {
    if byte == 0 {
        return 0;
    }
    let mut c = GraphemeCursor::new(byte.min(s.len()), s.len(), true);
    c.prev_boundary(s, 0).ok().flatten().unwrap_or(0)
}

/// Snap `byte` down to the nearest grapheme boundary. Looks only at the
/// text around `byte`, so it is cheap even on a very long line.
pub fn floor_grapheme(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    let mut byte = byte;
    while !s.is_char_boundary(byte) {
        byte -= 1;
    }
    let mut c = GraphemeCursor::new(byte, s.len(), true);
    match c.is_boundary(s, 0) {
        Ok(true) => byte,
        _ => c.prev_boundary(s, 0).ok().flatten().unwrap_or(0),
    }
}

/// Printable ASCII only: every byte is one grapheme one column wide, so
/// columns are byte offsets (fast paths for very long lines).
pub fn is_plain(s: &str) -> bool {
    s.bytes().all(|b| (0x20..0x7f).contains(&b))
}

/// Where Ctrl-W deletes back to from `end`: trailing blanks, then either a
/// run of word characters or a run of other non-blank characters.
pub fn word_start_before(s: &str, end: usize) -> usize {
    let before = s[..end].trim_end_matches([' ', '\t']);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    match before.chars().next_back() {
        None => 0,
        Some(c) if word(c) => before.trim_end_matches(word).len(),
        Some(_) => before
            .trim_end_matches(|c: char| !word(c) && !c.is_whitespace())
            .len(),
    }
}

/// Display width of one grapheme when it starts at column `col`.
pub fn grapheme_width(g: &str, col: usize, tabstop: usize) -> usize {
    match g {
        "\t" => tabstop - col % tabstop,
        _ if g.chars().next().is_some_and(is_control) => 2,
        _ => g.width(),
    }
}

/// Characters drawn as `^X`.
pub fn is_control(c: char) -> bool {
    (c as u32) < 0x20 || c == '\u{7f}'
}

/// Width of `s` on screen when it starts at column `start_col`.
pub fn display_width(s: &str, tabstop: usize, start_col: usize) -> usize {
    if is_plain(s) {
        return s.len();
    }
    let mut col = start_col;
    for g in s.graphemes(true) {
        col += grapheme_width(g, col, tabstop);
    }
    col - start_col
}

/// Screen column where the grapheme at `byte` starts.
pub fn col_of_byte(line: &str, byte: usize, tabstop: usize) -> usize {
    display_width(&line[..byte.min(line.len())], tabstop, 0)
}

/// Byte offset of the grapheme covering screen column `col` (or `line.len()`).
pub fn byte_of_col(line: &str, col: usize, tabstop: usize) -> usize {
    if is_plain(line) {
        return col.min(line.len());
    }
    let mut c = 0;
    for (i, g) in line.grapheme_indices(true) {
        let w = grapheme_width(g, c, tabstop);
        if col < c + w.max(1) {
            return i;
        }
        c += w;
    }
    line.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes_and_width() {
        let s = "e\u{301}漢x";
        assert_eq!(next_grapheme(s, 0), 3);
        assert_eq!(next_grapheme(s, 3), 6);
        assert_eq!(prev_grapheme(s, 6), 3);
        assert_eq!(display_width(s, 8, 0), 4);
        assert_eq!(display_width("\tx", 8, 0), 9);
    }

    #[test]
    fn plain_ascii_is_one_column_per_byte() {
        assert!(is_plain("ab c{}[]~"));
        assert!(!is_plain("a\tb") && !is_plain("é") && !is_plain("a\u{1}"));
        let s = "x".repeat(50);
        assert_eq!(display_width(&s, 8, 3), 50);
        assert_eq!(byte_of_col(&s, 7, 8), 7);
        assert_eq!(byte_of_col(&s, 70, 8), 50);
        assert_eq!(col_of_byte(&s, 20, 8), 20);
    }

    #[test]
    fn floor_grapheme_looks_only_nearby() {
        let s = "ae\u{301}漢";
        assert_eq!(floor_grapheme(s, 0), 0);
        assert_eq!(floor_grapheme(s, 1), 1);
        assert_eq!(floor_grapheme(s, 2), 1, "inside e + combining accent");
        assert_eq!(floor_grapheme(s, 3), 1);
        assert_eq!(floor_grapheme(s, 4), 4);
        assert_eq!(floor_grapheme(s, 5), 4, "inside 漢");
        assert_eq!(floor_grapheme(s, 7), 7);
        assert_eq!(floor_grapheme(s, 99), 7);
    }

    #[test]
    fn col_byte_mapping() {
        let s = "a\t漢b";
        assert_eq!(col_of_byte(s, 2, 8), 8);
        assert_eq!(col_of_byte(s, 5, 8), 10);
        assert_eq!(byte_of_col(s, 9, 8), 2); // inside 漢 → its start
        assert_eq!(byte_of_col(s, 100, 8), s.len());
    }
}
