//! Where each grapheme of a line goes on screen. Scrolling and drawing both
//! use this, so the cursor is always placed where its character is drawn.

use crate::text::grapheme_width;
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
    iter: GraphemeIndices<'a>,
    tabstop: usize,
    wrap: Option<usize>,
    col: usize,
    row: usize,
    x: usize,
}

impl<'a> Layout<'a> {
    pub fn new(line: &'a str, tabstop: usize, wrap: Option<usize>) -> Layout<'a> {
        Layout {
            iter: line.grapheme_indices(true),
            tabstop,
            wrap,
            col: 0,
            row: 0,
            x: 0,
        }
    }
}

impl<'a> Iterator for Layout<'a> {
    type Item = Placed<'a>;

    fn next(&mut self) -> Option<Placed<'a>> {
        let (byte, text) = self.iter.next()?;
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
    fn end_of_a_full_row_is_the_next_row() {
        assert_eq!(wrap_cursor("abcde", 5, 8, 5), (1, 0));
        assert_eq!(wrap_rows("abcde", 8, 5), 1);
        assert_eq!(wrap_rows("", 8, 5), 1);
        assert_eq!(wrap_cursor("", 0, 8, 5), (0, 0));
    }
}
