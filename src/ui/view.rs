//! Which part of the buffer is on screen.

use super::layout::{wrap_cursor, wrap_rows};
use crate::editor::Editor;
use crate::text::{col_of_byte, display_width, grapheme_width, next_grapheme};

/// Top line and left column of the text area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub top: usize,
    /// With wrapping: screen rows of the top line scrolled off above.
    pub top_row: usize,
    pub left: usize,
    /// Wheel scrolling leaves the editing position alone until keyboard input.
    pub detached: bool,
    pub(crate) picker_offset: usize,
    pub(crate) last_click: Option<(std::time::Instant, bool, usize)>,
    /// Where the `:explain` box was last drawn, for clicks.
    pub(crate) explain_box: Option<ratatui::layout::Rect>,
}

/// Total window rows (text + status + command) for a file of `file_lines`.
pub fn window_height(cfg_height: usize, file_lines: usize, term_rows: u16) -> u16 {
    let max_text = (term_rows as usize).saturating_sub(4);
    let text = cfg_height.min(file_lines.max(1)).min(max_text).max(1);
    ((text + 2) as u16).min(term_rows.max(3))
}

impl View {
    pub(crate) fn double_click(&mut self, picker: bool, row: usize) -> bool {
        let now = std::time::Instant::now();
        let double = self
            .last_click
            .take()
            .is_some_and(|(then, was_picker, previous)| {
                was_picker == picker
                    && previous == row
                    && now.duration_since(then) <= std::time::Duration::from_millis(400)
            });
        if !double {
            self.last_click = Some((now, picker, row));
        }
        double
    }

    pub fn wheel(&mut self, ed: &Editor, rows: usize, cols: usize, wrap: bool, down: bool) {
        self.detached = true;
        if !wrap && ed.folds.is_empty() {
            self.top = if down {
                self.top.saturating_add(3)
            } else {
                self.top.saturating_sub(3)
            }
            .min(ed.line_count().saturating_sub(rows));
            self.top_row = 0;
            return;
        }
        let rows_of = |l| {
            if ed.folds.hidden(l) {
                return 0;
            }
            if !wrap {
                return 1;
            }
            let line = ed.buf.line(l);
            let height = wrap_rows(&line, ed.tabstop, cols);
            if l == ed.cur.line {
                height.max(wrap_cursor(&line, ed.cur.byte, ed.tabstop, cols).0 + 1)
            } else {
                height
            }
        };
        let mut top = (self.top, self.top_row);
        if down {
            for _ in 0..3 {
                if top.1 + 1 < rows_of(top.0) {
                    top.1 += 1;
                } else {
                    let next = ed.folds.down(top.0, 1, ed.line_count());
                    if next != top.0 {
                        top = (next, 0);
                    }
                }
            }
        } else {
            top = up(top, 3, &rows_of);
        }
        let last = ed.folds.prev_visible(ed.line_count() - 1);
        let max_top = up((last, rows_of(last) - 1), rows.saturating_sub(1), &rows_of);
        (self.top, self.top_row) = top.min(max_top);
        if wrap {
            self.left = 0;
        }
    }

    /// Scroll so the cursor is visible with up to 2 lines of context.
    pub fn scroll(&mut self, ed: &Editor, rows: usize, cols: usize, wrap: bool) {
        if self.detached {
            return;
        }
        if wrap {
            self.scroll_wrapped(ed, rows.max(1), cols.max(1));
            return;
        }
        self.top_row = 0;
        let n = ed.line_count();
        let c = ed.cur.line;
        let rows = rows.max(1);
        if !ed.folds.is_empty() {
            // Hidden lines take no rows.
            let rows_of = |l: usize| usize::from(!ed.folds.hidden(l));
            self.scroll_rows(ed, rows, (c, 0), &rows_of);
        } else {
            let so = 2.min((rows - 1) / 2);
            if c < self.top + so {
                self.top = c.saturating_sub(so);
            }
            if c + so >= self.top + rows {
                self.top = c + so + 1 - rows;
            }
            self.top = self.top.min(n.saturating_sub(rows)).min(c);
        }
        let line = ed.buf.line(c);
        let cc = col_of_byte(&line, ed.cur.byte, ed.tabstop);
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
        let room = |left: usize| {
            if line_w > left + cols {
                cols.saturating_sub(1).max(1)
            } else {
                cols
            }
        };
        if cc + w > self.left + room(self.left) {
            let full = (cc + w).saturating_sub(cols);
            self.left = if line_w > full + cols {
                (cc + w).saturating_sub(room(full))
            } else {
                full
            };
        }
    }

    /// Wrap mode: scroll by screen rows (a line can be taller than the
    /// window), keeping up to 2 rows of context where there is any.
    fn scroll_wrapped(&mut self, ed: &Editor, rows: usize, cols: usize) {
        self.left = 0;
        let c = ed.cur.line;
        let (cr, _) = wrap_cursor(&ed.buf.line(c), ed.cur.byte, ed.tabstop, cols);
        let rows_of = |l: usize| {
            if l != c && ed.folds.hidden(l) {
                return 0;
            }
            let r = wrap_rows(&ed.buf.line(l), ed.tabstop, cols);
            if l == c { r.max(cr + 1) } else { r }
        };
        self.scroll_rows(ed, rows, (c, cr), &rows_of);
    }

    /// Scroll by screen rows so `cursor` (line, row) shows with up to 2
    /// rows of context; `rows_of` is 0 for hidden lines.
    fn scroll_rows(
        &mut self,
        ed: &Editor,
        rows: usize,
        cursor: (usize, usize),
        rows_of: &impl Fn(usize) -> usize,
    ) {
        let n = ed.line_count();
        let (c, cr) = cursor;
        self.top = ed.folds.prev_visible(self.top.min(n - 1));
        self.top_row = self.top_row.min(rows_of(self.top).max(1) - 1);
        let so = 2.min((rows - 1) / 2);
        let cursor = (c, cr);
        let above = so.min(rows_before(cursor, so, rows_of));
        let below = so.min(rows_after(cursor, n, so, rows_of));
        let top = (self.top, self.top_row);
        let new_top = if top > cursor {
            Some(up(cursor, above, rows_of))
        } else {
            let d = rows_between(top, cursor, rows, rows_of);
            if d < above {
                Some(up(cursor, above, rows_of))
            } else if d + below >= rows {
                Some(up(cursor, rows - 1 - below, rows_of))
            } else {
                None
            }
        };
        if let Some((l, r)) = new_top {
            self.top = l;
            self.top_row = r;
        }
    }
}

/// Screen rows from `a` to `b` (`a` <= `b`), counting at most past `cap`.
pub fn rows_between(
    a: (usize, usize),
    b: (usize, usize),
    cap: usize,
    rows_of: &impl Fn(usize) -> usize,
) -> usize {
    if a.0 == b.0 {
        return b.1 - a.1;
    }
    let mut d = rows_of(a.0) - a.1;
    for l in a.0 + 1..b.0 {
        if d > cap {
            return d;
        }
        d += rows_of(l);
    }
    d + b.1
}

/// The position `k` screen rows above `pos` (or the very top).
fn up(pos: (usize, usize), k: usize, rows_of: &impl Fn(usize) -> usize) -> (usize, usize) {
    let (mut l, mut r) = pos;
    let mut k = k;
    while k > 0 {
        if r >= k {
            return (l, r - k);
        }
        k -= r + 1;
        // Hidden lines above take no rows.
        let mut prev = l;
        loop {
            if prev == 0 {
                return (l, 0);
            }
            prev -= 1;
            if rows_of(prev) > 0 {
                break;
            }
        }
        l = prev;
        r = rows_of(l) - 1;
    }
    (l, r)
}

/// Screen rows above `pos` in the file, counting up to `cap`.
fn rows_before(pos: (usize, usize), cap: usize, rows_of: &impl Fn(usize) -> usize) -> usize {
    let mut d = pos.1;
    let mut l = pos.0;
    while d < cap && l > 0 {
        l -= 1;
        d += rows_of(l);
    }
    d
}

/// Screen rows below `pos` in the file, counting up to `cap`.
fn rows_after(
    pos: (usize, usize),
    n: usize,
    cap: usize,
    rows_of: &impl Fn(usize) -> usize,
) -> usize {
    let mut d = rows_of(pos.0) - 1 - pos.1;
    let mut l = pos.0 + 1;
    while d < cap && l < n {
        d += rows_of(l);
        l += 1;
    }
    d
}
