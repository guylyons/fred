//! Rendering the editor into a ratatui frame.

use super::layout::{Layout, Placed, wrap_cursor};
use super::view::View;
use crate::config::Config;
use crate::editor::{Editor, Mode};
use crate::highlight::{Highlighter, LineStyles};
use crate::text::{col_of_byte, display_width, is_control};
use ratatui::Frame;
use ratatui::buffer::Buffer as Screen;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

/// Styles for increasing byte offsets of one line.
struct Styler<'a> {
    spans: &'a [(Style, std::ops::Range<usize>)],
    ix: usize,
}

impl<'a> Styler<'a> {
    fn new(styles: &'a Option<LineStyles>) -> Styler<'a> {
        Styler {
            spans: styles.as_deref().unwrap_or(&[]),
            ix: 0,
        }
    }

    fn at(&mut self, byte: usize) -> Style {
        while self.ix < self.spans.len() && self.spans[self.ix].1.end <= byte {
            self.ix += 1;
        }
        match self.spans.get(self.ix) {
            Some((st, r)) if r.contains(&byte) => *st,
            _ => Style::default(),
        }
    }
}

/// Draw one grapheme: tabs as spaces, control characters as `^X`.
fn push_cell(spans: &mut Vec<Span<'static>>, p: &Placed, style: Style) {
    if p.text == "\t" {
        spans.push(Span::styled(" ".repeat(p.width), style));
    } else if let Some(c) = p.text.chars().next().filter(|&c| is_control(c)) {
        let shown = char::from_u32((c as u32 + 64) & 0x7f).unwrap_or('?');
        spans.push(Span::styled(
            format!("^{shown}"),
            Style::default().fg(Color::Blue),
        ));
    } else if p.width > 0 {
        spans.push(Span::styled(p.text.to_string(), style));
    }
}

/// No wrapping: spans for columns `left..left + cols` only (a long line is
/// never laid out in full), with `›` in the last column when it continues.
fn visible(
    line: &str,
    styles: &Option<LineStyles>,
    tabstop: usize,
    left: usize,
    cols: usize,
) -> Vec<Span<'static>> {
    let mut styler = Styler::new(styles);
    let mut lay = Layout::new(line, tabstop, None)
        .filter(|p| p.width > 0)
        .peekable();
    let mut spans = vec![];
    let mut used = 0;
    while let Some(p) = lay.next() {
        let end = p.col + p.width;
        if end <= left {
            continue;
        }
        let style = styler.at(p.byte);
        if p.col < left {
            // A wide char cut by the left edge: show its visible half as blank.
            spans.push(Span::styled(" ".repeat(end - left), style));
            used += end - left;
            continue;
        }
        let more = lay.peek().is_some();
        if used + p.width > cols || (used + p.width == cols && more) {
            // Leave the last column for the cut marker.
            while used + 1 < cols {
                spans.push(Span::raw(" "));
                used += 1;
            }
            spans.push(Span::styled("›", Style::default().fg(Color::DarkGray)));
            return spans;
        }
        push_cell(&mut spans, &p, style);
        used += p.width;
    }
    spans
}

fn reversed(spans: &mut Vec<Span<'static>>) {
    if spans.is_empty() {
        spans.push(Span::raw(" "));
    }
    for s in spans.iter_mut() {
        s.style = s.style.add_modifier(Modifier::REVERSED);
    }
}

fn digits(n: usize) -> usize {
    n.to_string().len()
}

pub fn gutter_width(ed: &Editor, cfg: &Config) -> usize {
    if cfg.numbers || cfg.relative_numbers {
        digits(ed.line_count()).max(3) + 1
    } else {
        0
    }
}

fn mode_name(m: &Mode) -> &'static str {
    match m {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::VisualLine { .. } => "V-LINE",
        Mode::Command(_) => "COMMAND",
    }
}

/// Draw the editor; the frame area is the whole inline window.
pub fn draw(
    f: &mut Frame,
    ed: &Editor,
    view: &mut View,
    hl: &mut Highlighter,
    cfg: &Config,
    budget: Duration,
) {
    let area = f.area();
    if area.height == 0 || area.width == 0 {
        return;
    }
    // Text rows; a terminal under 3 rows tall keeps only the bottom rows.
    let rows = (area.height as usize).saturating_sub(2);
    let gutter = gutter_width(ed, cfg).min(area.width as usize / 2);
    let cols = (area.width as usize).saturating_sub(gutter).max(1);
    view.scroll(ed, rows.max(1), cols, cfg.wrap);
    let n = ed.line_count();
    let last = (view.top + rows).min(n);
    let styles = hl.styles(&ed.buf, view.top..last, budget);
    let sel = match ed.mode {
        Mode::VisualLine { anchor } => Some((anchor.min(ed.cur.line), anchor.max(ed.cur.line))),
        _ => None,
    };
    let num_style = Style::default().fg(Color::DarkGray);
    let cur_num_style = Style::default().add_modifier(Modifier::BOLD);
    let buf = f.buffer_mut();
    let (ox, oy) = (area.x, area.y);
    let mut cursor = None;
    let mut y = 0usize;
    let mut l = view.top;
    while y < rows && l < n {
        let line = ed.buf.line(l);
        let st = styles.get(l - view.top).cloned().flatten();
        let selected = sel.is_some_and(|(a, b)| l >= a && l <= b);
        // Rows of this line already scrolled off the top.
        let skip = if l == view.top { view.top_row } else { 0 };
        let first_y = y;
        let mut emit = |y: &mut usize, row: usize, mut spans: Vec<Span<'static>>| {
            if selected {
                reversed(&mut spans);
            }
            if gutter > 0 && row == 0 {
                let num = if cfg.relative_numbers && l != ed.cur.line {
                    l.abs_diff(ed.cur.line)
                } else {
                    l + 1
                };
                let style = if l == ed.cur.line {
                    cur_num_style
                } else {
                    num_style
                };
                let text = format!("{num:>w$} ", w = gutter - 1);
                buf.set_stringn(ox, oy + *y as u16, text, gutter, style);
            }
            buf.set_line(
                ox + gutter as u16,
                oy + *y as u16,
                &Line::from(spans),
                cols as u16,
            );
            *y += 1;
        };
        if cfg.wrap {
            let mut styler = Styler::new(&st);
            let mut row = 0;
            let mut spans = vec![];
            for p in Layout::new(&line, ed.tabstop, Some(cols)) {
                if p.row != row {
                    if row >= skip {
                        emit(&mut y, row, std::mem::take(&mut spans));
                        if y >= rows {
                            break;
                        }
                    }
                    spans.clear();
                    row = p.row;
                }
                let style = styler.at(p.byte);
                if p.row >= skip {
                    push_cell(&mut spans, &p, style);
                }
            }
            if y < rows && row >= skip {
                emit(&mut y, row, spans);
            }
            if l == ed.cur.line {
                let (cr, cx) = wrap_cursor(&line, ed.cur.byte, ed.tabstop, cols);
                // The end of a full row puts the cursor on a row of its own.
                if cr > row && y < rows {
                    emit(&mut y, cr, vec![]);
                }
                if cr >= skip {
                    cursor = Some((gutter + cx, first_y + cr - skip));
                }
            }
        } else {
            emit(&mut y, 0, visible(&line, &st, ed.tabstop, view.left, cols));
            if l == ed.cur.line {
                let cc = col_of_byte(&line, ed.cur.byte, ed.tabstop);
                cursor = Some((gutter + cc.saturating_sub(view.left), first_y));
            }
        }
        l += 1;
    }
    while y < rows {
        buf.set_stringn(ox, oy + y as u16, "~", 1, num_style);
        y += 1;
    }
    if area.height >= 2 {
        draw_status(buf, area, ed, hl);
    }
    let cmd_cursor = draw_command_row(buf, area, ed);
    if let Some((cx, cy)) = cursor
        && cy < rows
    {
        let wrap_cols = if cfg.wrap { cols } else { 0 };
        draw_popup(buf, area, ed, view, gutter, rows, (cx, cy), wrap_cols);
    }
    match (cmd_cursor, cursor) {
        (Some(x), _) => f.set_cursor_position((ox + x as u16, oy + area.height - 1)),
        (None, Some((x, y))) if rows > 0 => {
            let x = x.min(area.width as usize - 1) as u16;
            let y = y.min(rows - 1) as u16;
            f.set_cursor_position((ox + x, oy + y));
        }
        _ => {}
    }
}

fn draw_status(buf: &mut Screen, area: Rect, ed: &Editor, hl: &Highlighter) {
    let y = area.y + area.height - 2;
    let bar = Style::default().add_modifier(Modifier::REVERSED);
    let name = ed
        .path
        .as_ref()
        .map_or_else(|| "[No Name]".to_string(), |p| p.display().to_string());
    let modified = if ed.buf.modified { " [+]" } else { "" };
    let ro = if ed.readonly { " [RO]" } else { "" };
    let left = format!(" {}  {name}{modified}{ro}", mode_name(&ed.mode));
    let line = ed.buf.line(ed.cur.line);
    let col = col_of_byte(&line, ed.cur.byte, ed.tabstop) + 1;
    let ft = match hl.syntax_name() {
        "Plain Text" => String::new(),
        s => format!("{}  ", s.to_lowercase()),
    };
    let right = format!("{ft}{}:{col} ", ed.cur.line + 1);
    let w = area.width as usize;
    let rw = display_width(&right, 1, 0);
    buf.set_stringn(area.x, y, " ".repeat(w), w, bar);
    buf.set_stringn(area.x, y, &left, w.saturating_sub(rw + 1).max(1), bar);
    if rw < w {
        buf.set_stringn(area.x + (w - rw) as u16, y, &right, rw, bar);
    }
}

/// Draws the bottom row; returns the cursor column when typing a command.
fn draw_command_row(buf: &mut Screen, area: Rect, ed: &Editor) -> Option<usize> {
    let y = area.y + area.height - 1;
    let w = area.width as usize;
    match &ed.mode {
        Mode::Command(cl) => {
            let full = format!("{}{}", cl.kind, cl.text);
            let ccol = 1 + display_width(&cl.text[..cl.cursor], 1, 0);
            let skip = (ccol + 1).saturating_sub(w);
            let mut col = 0;
            let mut shown = String::new();
            for g in full.graphemes(true) {
                if col >= skip {
                    shown.push_str(g);
                }
                col += display_width(g, 1, 0);
            }
            buf.set_stringn(area.x, y, shown, w, Style::default());
            Some(ccol - skip)
        }
        _ => {
            if let Some((m, err)) = &ed.msg {
                let style = if *err {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                buf.set_stringn(area.x, y, m, w, style);
            }
            None
        }
    }
}

fn draw_popup(
    buf: &mut Screen,
    area: Rect,
    ed: &Editor,
    view: &View,
    gutter: usize,
    rows: usize,
    cursor: (usize, usize),
    wrap_cols: usize,
) {
    let Some(p) = &ed.popup else { return };
    if rows < 3 || p.items.is_empty() {
        return;
    }
    let (_, cy) = cursor;
    let below = rows - cy - 1;
    let above = cy;
    let want = p.items.len();
    let (y0, h) = if below >= want || below >= above {
        (cy + 1, want.min(below))
    } else {
        (cy - want.min(above), want.min(above))
    };
    if h == 0 {
        return;
    }
    let width = p
        .items
        .iter()
        .map(|s| display_width(s, 1, 0))
        .max()
        .unwrap_or(0)
        + 2;
    let width = width.min(area.width as usize);
    let line = ed.buf.line(ed.cur.line);
    let start = p.start.min(line.len());
    let start_col = if wrap_cols > 0 {
        let (sr, sx) = wrap_cursor(&line, start, ed.tabstop, wrap_cols);
        let (cr, _) = wrap_cursor(&line, ed.cur.byte, ed.tabstop, wrap_cols);
        gutter + if sr == cr { sx } else { 0 }
    } else {
        gutter + col_of_byte(&line, start, ed.tabstop).saturating_sub(view.left)
    };
    let x0 = start_col.saturating_sub(1).min(area.width as usize - width);
    let normal = Style::default().bg(Color::DarkGray).fg(Color::White);
    let selected = Style::default().bg(Color::White).fg(Color::Black);
    // Keep the selected item visible when the list is taller than the space.
    let first = p.sel.map_or(0, |s| (s + 1).saturating_sub(h));
    for (i, item) in p.items.iter().enumerate().skip(first).take(h) {
        let style = if p.sel == Some(i) { selected } else { normal };
        let text = format!(" {item:<w$}", w = width - 1);
        buf.set_stringn(
            area.x + x0 as u16,
            area.y + (y0 + i - first) as u16,
            text,
            width,
            style,
        );
    }
}
