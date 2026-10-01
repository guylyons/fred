//! Which part of the buffer is on screen.

use crate::editor::Editor;
use crate::text::{col_of_byte, display_width, grapheme_width, next_grapheme};

/// Top line and left column of the text area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub top: usize,
    pub left: usize,
}

/// Total window rows (text + status + command) for a file of `file_lines`.
pub fn window_height(cfg_height: usize, file_lines: usize, term_rows: u16) -> u16 {
    let max_text = (term_rows as usize).saturating_sub(4);
    let text = cfg_height.min(file_lines.max(1)).min(max_text).max(1);
    ((text + 2) as u16).min(term_rows.max(3))
}

/// Screen rows a line takes when wrapped at `cols`.
pub fn line_rows(width: usize, cols: usize) -> usize {
    width.div_ceil(cols.max(1)).max(1)
}

impl View {
    /// Scroll so the cursor is visible with up to 2 lines of context.
    pub fn scroll(&mut self, ed: &Editor, rows: usize, cols: usize, wrap: bool) {
        let n = ed.line_count();
        let c = ed.cur.line;
        let rows = rows.max(1);
        let so = 2.min((rows - 1) / 2);
        if c < self.top + so {
            self.top = c.saturating_sub(so);
        }
        if c + so >= self.top + rows {
            self.top = c + so + 1 - rows;
        }
        self.top = self.top.min(n.saturating_sub(rows)).min(c);
        let line = ed.buf.line(c);
        let cc = col_of_byte(&line, ed.cur.byte, ed.tabstop);
        if wrap {
            self.left = 0;
            // Wrapped lines above the cursor can push it off screen.
            loop {
                let mut row = 0;
                for l in self.top..c {
                    row += line_rows(display_width(&ed.buf.line(l), ed.tabstop, 0), cols);
                }
                row += cc / cols.max(1);
                if row < rows || self.top >= c {
                    break;
                }
                self.top += 1;
            }
            return;
        }
        let w = if ed.cur.byte < line.len() {
            grapheme_width(
                &line[ed.cur.byte..next_grapheme(&line, ed.cur.byte)],
                cc,
                ed.tabstop,
            )
            .max(1)
        } else {
            1
        };
        if cc < self.left {
            self.left = cc;
        }
        // When text continues past the right edge, the last column shows the
        // `›` marker, so the cursor must stay left of it.
        let line_w = display_width(&line, ed.tabstop, 0);
        let room = |left: usize| if line_w > left + cols { cols.saturating_sub(1).max(1) } else { cols };
        if cc + w > self.left + room(self.left) {
            let full = (cc + w).saturating_sub(cols);
            self.left = if line_w > full + cols { (cc + w).saturating_sub(room(full)) } else { full };
        }
    }
}
