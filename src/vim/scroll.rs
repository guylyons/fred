//! Scrolling the window: CTRL-E, CTRL-Y, CTRL-D, CTRL-U, CTRL-F, CTRL-B
//! and the `z` commands, after Neovim's `move.c` (for lines that don't
//! wrap or fold). The window's view follows the editor's request
//! (`Editor::view_req`).

use crate::editor::Editor;

/// The window's top line and height.
fn view(ed: &Editor) -> (usize, usize) {
    let (top, rows) = ed
        .viewport
        .map_or((0, ed.win_height), |v| (v.view.top, v.rows));
    (top.min(ed.line_count() - 1), rows.max(1))
}

fn scrolloff(ed: &Editor, rows: usize) -> usize {
    (ed.opts.num(crate::options::Opt::Scrolloff).max(0) as usize).min((rows - 1) / 2)
}

/// Show `top` at the top of the window.
fn set_top(ed: &mut Editor, top: usize) {
    let top = top.min(ed.line_count() - 1);
    ed.view_req.set(Some(top));
    if let Some(v) = ed.viewport.as_mut() {
        v.view.top = top;
    }
}

/// Move to `line`, keeping the remembered column.
fn to_line(ed: &mut Editor, line: usize) {
    if line != ed.cur.line {
        ed.set_line_keep_col(line.min(ed.line_count() - 1));
    }
}

/// Neovim's `cursor_correct()`: keep 'scrolloff' lines around the cursor
/// (none needed at the buffer's ends).
fn cursor_correct(ed: &mut Editor, top: usize, rows: usize) {
    let n = ed.line_count();
    let so = scrolloff(ed, rows);
    let above = if top == 0 { 0 } else { so };
    let below = if top + rows >= n { 0 } else { so };
    let c = ed.cur.line;
    if c < top + above {
        to_line(ed, top + above);
    } else if c + below + 1 > top + rows {
        to_line(ed, (top + rows).saturating_sub(below + 1));
    }
}

/// CTRL-E (`down`: the text moves up) and CTRL-Y, `count` lines.
pub fn lines(ed: &mut Editor, down: bool, count: usize) {
    let (top, rows) = view(ed);
    let n = ed.line_count();
    let prev = ed.cur.line;
    let new_top = if down {
        (top + count).min(n - 1)
    } else {
        top.saturating_sub(count)
    };
    set_top(ed, new_top);
    // The cursor stays in the window.
    if ed.cur.line < new_top {
        to_line(ed, new_top);
    } else if ed.cur.line >= new_top + rows {
        to_line(ed, new_top + rows - 1);
    }
    if scrolloff(ed, rows) > 0 {
        cursor_correct(ed, new_top, rows);
        // At an end, move the cursor at least, not to get stuck.
        if new_top == top {
            if down && ed.cur.line <= prev && prev + 1 < n {
                to_line(ed, prev + 1);
            } else if !down && ed.cur.line >= prev && top > 0 && prev > 0 {
                to_line(ed, prev - 1);
            }
        }
    }
    if new_top == top && ed.cur.line == prev {
        ed.vim.failed = true;
    }
}

/// CTRL-D and CTRL-U (`half`, a count sets 'scroll'), CTRL-F and CTRL-B:
/// Neovim's `pagescroll()`.
pub fn page(ed: &mut Editor, down: bool, count: Option<usize>, half: bool) {
    let (top, rows) = view(ed);
    let n = ed.line_count();
    let prev = ed.cur.pos();
    let mut new_top = top;
    if half {
        if let Some(c) = count {
            ed.vim.scroll = Some(c.min(rows));
        }
        let amount = ed.vim.scroll.unwrap_or(rows / 2).clamp(1, rows);
        let mut shift = amount as isize;
        // Don't scroll past the end of the buffer.
        if down && top + rows + amount > n {
            shift = (n - top) as isize - rows as isize;
        }
        if shift > 0 {
            new_top = if down {
                (top + shift as usize).min(n - 1)
            } else {
                top.saturating_sub(shift as usize)
            };
        }
        let line = if down {
            (prev.0 + amount).min(n - 1)
        } else {
            prev.0.saturating_sub(amount)
        };
        set_top(ed, new_top);
        to_line(ed, line);
    } else {
        let count = count.unwrap_or(1).max(1);
        // A page less two lines of overlap (none at the ends).
        let at_end = if down { top + rows > n - 1 } else { top == 0 };
        let step = if at_end {
            rows
        } else {
            rows.saturating_sub(2).max(1)
        };
        new_top = if down {
            (top + step * count).min(n - 1)
        } else {
            top.saturating_sub(step * count)
        };
        set_top(ed, new_top);
        if new_top != top {
            let line = if down {
                new_top
            } else {
                (new_top + rows - 1).min(n - 1)
            };
            to_line(ed, line);
        }
    }
    cursor_correct(ed, new_top, rows);
    if new_top == top && ed.cur.pos() == prev {
        ed.vim.failed = true;
    }
}

/// The `z` commands that put the cursor's line at the top (`t`, `<CR>`,
/// `+`), middle (`z`, `.`) or bottom (`b`, `-`, `^`) of the window.
pub fn z(ed: &mut Editor, c: char, count: Option<usize>) {
    let (top, rows) = view(ed);
    let n = ed.line_count();
    let so = scrolloff(ed, rows);
    if let Some(l) = count.filter(|_| c != '^') {
        to_line(ed, l.clamp(1, n) - 1);
    }
    match c {
        '+' if count.is_none() => to_line(ed, (top + rows).min(n - 1)),
        '^' => {
            if let Some(l) = count {
                // Line N at the bottom, then the cursor at its top line.
                let l = l.clamp(1, n) - 1;
                let below = so.min(n - 1 - l);
                let t = (l + below + 1).saturating_sub(rows);
                to_line(ed, t);
            } else {
                to_line(ed, top.saturating_sub(1));
            }
        }
        _ => {}
    }
    if matches!(c, '\n' | '.' | '-' | '+' | '^') {
        let l = ed.cur.line;
        ed.set_cursor(l, ed.first_nonblank(l));
    }
    let l = ed.cur.line;
    let new_top = match c {
        't' | '\n' | '+' => l.saturating_sub(so),
        'b' | '-' | '^' => {
            let below = so.min(n - 1 - l);
            (l + below + 1).saturating_sub(rows)
        }
        // Neovim's scroll_cursor_halfway(): as many lines above as below.
        _ => l.saturating_sub((rows - 1) / 2),
    };
    set_top(ed, new_top);
}
