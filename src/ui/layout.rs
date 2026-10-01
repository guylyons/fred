//! Where each grapheme of a line goes on screen. Scrolling and drawing both
//! use this, so the cursor is always placed where its character is drawn.

use crate::text::{grapheme_width, is_plain};
use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};

/// One grapheme placed on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed<'a> {
    pub byte: usize,
    pub text: &'a str,
    /// Column in the unwrapped line (tabs expanded).
    pub col: usize,
    pub width: usize,
    /// With wrapping: the screen row within the line, and the column in it.
    pub row: usize,
    pub x: usize,
}

/// Lays out a line's graphemes. With `wrap = Some(cols)`, a grapheme that
/// doesn't fit in the rest of a row starts the next one.
pub struct Layout<'a> {
    iter: Graphemes<'a>,
    tabstop: usize,
    wrap: Option<usize>,
    col: usize,
    row: usize,
    x: usize,
}

/// The graphemes of a line; printable ASCII is walked byte by byte.
enum Graphemes<'a> {
    General(GraphemeIndices<'a>),
    Plain { line: &'a str, pos: usize },
}

impl<'a> Layout<'a> {
    pub fn new(line: &'a str, tabstop: usize, wrap: Option<usize>) -> Layout<'a> {
        let iter = if is_plain(line) {
            Graphemes::Plain { line, pos: 0 }
        } else {
            Graphemes::General(line.grapheme_indices(true))
        };
        Layout {
            iter,
            tabstop,
            wrap,
            col: 0,
            row: 0,
            x: 0,
        }
    }

    /// Without the ASCII fast path (tests compare the two).
    #[cfg(test)]
    pub fn general(line: &'a str, tabstop: usize, wrap: Option<usize>) -> Layout<'a> {
        let mut l = Layout::new(line, tabstop, wrap);
        l.iter = Graphemes::General(line.grapheme_indices(true));
        l
    }

    /// Unwrapped, starting at the grapheme covering column `col` (or the
    /// first one after it). On printable ASCII this skips straight there.
    pub fn from_col(line: &'a str, tabstop: usize, col: usize) -> impl Iterator<Item = Placed<'a>> {
        let mut l = Layout::new(line, tabstop, None);
        if let Graphemes::Plain { pos, .. } = &mut l.iter {
            *pos = col.min(line.len());
            l.col = *pos;
            l.x = *pos;
        }
        l.filter(move |p| p.col + p.width > col)
    }
}

impl<'a> Iterator for Layout<'a> {
    type Item = Placed<'a>;

    fn next(&mut self) -> Option<Placed<'a>> {
        let (byte, text) = match &mut self.iter {
            Graphemes::General(it) => it.next()?,
            Graphemes::Plain { line, pos } => {
                let b = *pos;
                let t = line.get(b..b + 1)?;
                *pos += 1;
                (b, t)
            }
        };
        let width = grapheme_width(text, self.col, self.tabstop);
        if let Some(cols) = self.wrap
            && self.x > 0
            && self.x + width > cols
        {
            self.row += 1;
            self.x = 0;
        }
        let p = Placed {
            byte,
            text,
            col: self.col,
            width,
            row: self.row,
            x: self.x,
        };
        self.col += width;
        self.x += width;
        Some(p)
    }
}

/// With wrapping at `cols`: the (row, x) of the cursor at `byte`, which may
/// be the end of the line.
pub fn wrap_cursor(line: &str, byte: usize, tabstop: usize, cols: usize) -> (usize, usize) {
    if is_plain(line) {
        let b = byte.min(line.len());
        return (b / cols.max(1), b % cols.max(1));
    }
    let mut lay = Layout::new(line, tabstop, Some(cols));
    for p in lay.by_ref() {
        if p.byte >= byte {
            return (p.row, p.x);
        }
    }
    if lay.x >= cols && lay.x > 0 {
        (lay.row + 1, 0)
    } else {
        (lay.row, lay.x)
    }
}

/// With wrapping at `cols`: how many screen rows the line takes.
pub fn wrap_rows(line: &str, tabstop: usize, cols: usize) -> usize {
    if is_plain(line) {
        return line.len().div_ceil(cols.max(1)).max(1);
    }
    Layout::new(line, tabstop, Some(cols))
        .last()
        .map_or(1, |p| p.row + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_char_moves_to_the_next_row() {
        // 5 columns: "abcd" then 漢 (2 wide) doesn't fit after it.
        let line = "abcd漢x";
        let placed: Vec<(usize, usize)> = Layout::new(line, 8, Some(5))
            .map(|p| (p.row, p.x))
            .collect();
        assert_eq!(placed, [(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 2)]);
        assert_eq!(wrap_cursor(line, 4, 8, 5), (1, 0));
        assert_eq!(wrap_cursor(line, line.len(), 8, 5), (1, 3));
        assert_eq!(wrap_rows(line, 8, 5), 2);
    }

    #[test]
    fn plain_lines_match_the_general_layout() {
        // The ASCII fast path must place everything exactly where the
        // grapheme-by-grapheme path does.
        let plain = "fn main() { let x = 1; }".repeat(5);
        for cols in [1, 3, 7, 16, 200] {
            for byte in (0..=plain.len()).step_by(3) {
                let slow = {
                    let mut lay = Layout::general(&plain, 8, Some(cols));
                    let mut at = None;
                    for p in lay.by_ref() {
                        if p.byte >= byte {
                            at = Some((p.row, p.x));
                            break;
                        }
                    }
                    at.unwrap_or(if lay.x >= cols && lay.x > 0 {
                        (lay.row + 1, 0)
                    } else {
                        (lay.row, lay.x)
                    })
                };
                assert_eq!(
                    wrap_cursor(&plain, byte, 8, cols),
                    slow,
                    "cols {cols} byte {byte}"
                );
            }
            assert_eq!(
                wrap_rows(&plain, 8, cols),
                Layout::general(&plain, 8, Some(cols)).last().unwrap().row + 1
            );
        }
        let from: Vec<_> = Layout::from_col(&plain, 8, 10).take(3).collect();
        let all: Vec<_> = Layout::new(&plain, 8, None).skip(10).take(3).collect();
        assert_eq!(from, all);
    }

    #[test]
    fn end_of_a_full_row_is_the_next_row() {
        assert_eq!(wrap_cursor("abcde", 5, 8, 5), (1, 0));
        assert_eq!(wrap_rows("abcde", 8, 5), 1);
        assert_eq!(wrap_rows("", 8, 5), 1);
        assert_eq!(wrap_cursor("", 0, 8, 5), (0, 0));
    }
}
