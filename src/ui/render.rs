//! Rendering the editor into a ratatui frame.

use super::layout::{Layout, Placed, wrap_cursor};
use super::view::View;
use crate::config::Config;
use crate::editor::{CmdLine, Editor, Mode};
use crate::git::FileState;
use crate::git::Mark;
use crate::highlight::{Highlighter, LineStyles};
use crate::pick::{Kind, Picker, Row};
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
    let mut lay = Layout::from_col(line, tabstop, left)
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
    let numbers = if cfg.numbers || cfg.relative_numbers {
        digits(ed.line_count()).max(3) + 1
    } else {
        0
    };
    numbers + git_column(ed) + blame_column(ed)
}

/// magit-blame's margin, when the current style draws one.
fn blame_column(ed: &Editor) -> usize {
    ed.blame
        .as_ref()
        .filter(|b| b.version == ed.buf.version)
        .map_or(0, |b| b.width())
}

/// One column for git marks, if the file is in git.
fn git_column(ed: &Editor) -> usize {
    usize::from(ed.git.active())
}

/// A git mark's bar and color.
fn git_sign(m: Mark) -> (&'static str, Color) {
    match m {
        Mark::Added => ("▎", Color::Green),
        Mark::Changed => ("▎", Color::Yellow),
        Mark::RemovedAbove => ("▔", Color::Red),
        Mark::RemovedBelow => ("▁", Color::Red),
    }
}

fn mode_name(m: &Mode) -> &'static str {
    match m {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::VisualLine { .. } => "V-LINE",
        Mode::Command(_) => "COMMAND",
        Mode::Pick(p) => match p.kind {
            Kind::Files => "FIND",
            Kind::Grep => "GREP",
            Kind::Lines | Kind::AllLines => "LINES",
            Kind::Browse => "FILES",
            Kind::Recent => "RECENT",
            Kind::Buffers => "BUFFERS",
            Kind::Def => "DEFINITION",
            Kind::Branches => "BRANCHES",
            Kind::MagitMenu => "MAGIT MENU",
        },
    }
}

/// Mouse coordinates use the same layout as rendering, including inline offsets.
pub fn mouse(
    ed: &mut Editor,
    view: &mut View,
    cfg: &Config,
    area: Rect,
    event: ratatui::crossterm::event::MouseEvent,
) {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    if !area.contains((event.column, event.row).into()) {
        view.last_click = None;
        return;
    }
    if let Mode::Pick(p) = &mut ed.mode {
        let code = match event.kind {
            MouseEventKind::ScrollDown => crate::key::KeyCode::Down,
            MouseEventKind::ScrollUp => crate::key::KeyCode::Up,
            MouseEventKind::Down(MouseButton::Left) => {
                let text_rows = area.height.saturating_sub(2) as usize;
                let panel = panel_rows(text_rows, p.rows.len());
                let top = area.bottom().saturating_sub(panel as u16);
                if event.row < top {
                    view.last_click = None;
                    return;
                }
                let selected = view.picker_offset + (event.row - top) as usize;
                if selected >= p.rows.len() {
                    view.last_click = None;
                    return;
                }
                p.sel = selected;
                if view.double_click(true, selected) {
                    view.detached = false;
                    ed.handle_key(crate::key::Key::new(crate::key::KeyCode::Enter));
                }
                return;
            }
            _ => return,
        };
        view.last_click = None;
        ed.handle_key(crate::key::Key::new(code));
        return;
    }
    if ed.explain.is_some() && matches!(event.kind, MouseEventKind::Down(_)) {
        if let Some(b) = view.explain_box
            && b.contains((event.column, event.row).into())
        {
            // Its ✕ closes it; other clicks inside leave it be.
            if event.row == b.y && event.column + 4 >= b.right() {
                ed.explain = None;
            }
            return;
        }
        // A click elsewhere closes it, and still does what it would.
        ed.explain = None;
    }
    if event.row >= area.bottom().saturating_sub(2) || matches!(ed.mode, Mode::Command(_)) {
        view.last_click = None;
        return;
    }
    ed.zap = None;
    let gutter = gutter_width(ed, cfg).min(area.width as usize / 2);
    let cols = (area.width as usize).saturating_sub(gutter).max(1);
    let rows = (area.height as usize).saturating_sub(2).max(1);
    match event.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            view.last_click = None;
            view.wheel(
                ed,
                rows,
                cols,
                cfg.wrap,
                event.kind == MouseEventKind::ScrollDown,
            );
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let mut y = (event.row - area.y) as usize + view.top_row;
            let x = (event.column - area.x) as usize;
            let x = x.saturating_sub(gutter);
            for line in view.top..ed.line_count() {
                let text = ed.buf.line(line);
                let height = if cfg.wrap {
                    let height = super::layout::wrap_rows(&text, ed.tabstop, cols);
                    if line == ed.cur.line {
                        height.max(wrap_cursor(&text, ed.cur.byte, ed.tabstop, cols).0 + 1)
                    } else {
                        height
                    }
                } else {
                    1
                };
                if y >= height {
                    y -= height;
                    continue;
                }
                let target = if cfg.wrap { x } else { x + view.left };
                let byte = Layout::new(&text, ed.tabstop, cfg.wrap.then_some(cols))
                    .find(|p| p.row == y && p.x + p.width > target)
                    .map_or_else(
                        || {
                            Layout::new(&text, ed.tabstop, cfg.wrap.then_some(cols))
                                .filter(|p| p.row == y)
                                .last()
                                .map_or(text.len(), |p| p.byte + p.text.len())
                        },
                        |p| p.byte,
                    );
                ed.set_cursor(line, byte);
                ed.popup = None;
                ed.vim.pending.clear();
                if ed.dired.as_ref().is_some_and(|d| !d.editing) {
                    view.detached = true;
                    if view.double_click(false, line) {
                        view.detached = false;
                        ed.handle_key(crate::key::Key::new(crate::key::KeyCode::Enter));
                    }
                } else {
                    view.last_click = None;
                }
                return;
            }
        }
        _ => {}
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
    let text_rows = (area.height as usize).saturating_sub(2);
    // A picker takes a panel at the bottom; the file stays as it was above.
    let panel = match ed.mode {
        // A Magit menu is a transient popup sized to its layout.
        Mode::Pick(ref p) if p.kind == Kind::MagitMenu => {
            let lines = transient_lines(ed, p.sel, area.width as usize).len();
            lines.clamp(1, text_rows.saturating_sub(1).max(1))
        }
        Mode::Pick(ref p) => panel_rows(text_rows, p.rows.len()),
        _ => 0,
    };
    let rows = text_rows - panel;
    if let Mode::Pick(p) = &ed.mode {
        view.picker_offset = view.picker_offset.min(p.rows.len().saturating_sub(panel));
        if p.sel < view.picker_offset {
            view.picker_offset = p.sel;
        } else if p.sel >= view.picker_offset + panel {
            view.picker_offset = (p.sel + 1).saturating_sub(panel);
        }
    } else {
        view.picker_offset = 0;
    }
    let gutter = gutter_width(ed, cfg).min(area.width as usize / 2);
    let blame_w = blame_column(ed).min(gutter);
    let sign = git_column(ed).min(gutter - blame_w);
    let blame = ed.blame.as_ref().filter(|b| b.version == ed.buf.version);
    let cols = (area.width as usize).saturating_sub(gutter).max(1);
    if panel == 0 && ed.zap.is_none() {
        view.scroll(ed, rows.max(1), cols, cfg.wrap);
    }
    let n = ed.line_count();
    let last = (view.top + rows).min(n);
    let styles = if ed.magit.is_some() {
        vec![]
    } else {
        hl.styles(&ed.buf, view.top..last, budget)
    };
    let sel = match ed.mode {
        Mode::VisualLine { anchor } => Some((anchor.min(ed.cur.line), anchor.max(ed.cur.line))),
        Mode::Command(ref cl) if cl.kind == ':' && cl.text.starts_with("'<,'>") => ed
            .marks
            .get(&'<')
            .zip(ed.marks.get(&'>'))
            .map(|(&a, &b)| (a.min(b), a.max(b))),
        _ => None,
    };
    let splash = ed.path.is_none()
        && ed.buf.len_bytes() == 0
        && matches!(ed.mode, Mode::Normal | Mode::Command(_));
    let num_style = Style::default().fg(Color::DarkGray);
    let cur_num_style = Style::default().add_modifier(Modifier::BOLD);
    let buf = f.buffer_mut();
    let (ox, oy) = (area.x, area.y);
    let mut cursor = None;
    // The explained lines' first row and the row after them, if on screen.
    let mut ex_top = ed
        .explain
        .as_ref()
        .and_then(|(range, _)| (range.start <= view.top && view.top <= range.end).then_some(0));
    let mut ex_end = None;
    let mut y = 0usize;
    let mut l = view.top;
    while y < rows && l < n {
        let line = ed.buf.line(l);
        let st = if ed.magit.is_some() {
            let text = line.trim_start();
            let color = if text.starts_with('+') {
                Color::Green
            } else if text.starts_with('-') {
                Color::Red
            } else if text.starts_with("@@") {
                Color::Cyan
            } else if text.starts_with("Head:") || text.starts_with("v ") || text.starts_with("> ")
            {
                Color::Yellow
            } else {
                Color::Reset
            };
            Some(vec![(Style::default().fg(color), 0..line.len())])
        } else if ed.dired.is_some() {
            Some(crate::dired::styles(ed, l))
        } else {
            styles.get(l - view.top).cloned().flatten()
        };
        let selected = sel.is_some_and(|(a, b)| l >= a && l <= b);
        // Rows of this line already scrolled off the top.
        let skip = if l == view.top { view.top_row } else { 0 };
        let first_y = y;
        let mut emit = |y: &mut usize, row: usize, mut spans: Vec<Span<'static>>| {
            if selected {
                reversed(&mut spans);
            }
            if blame_w > 0
                && row == 0
                && let Some((_, text)) = blame.and_then(|b| b.margin(l))
            {
                // Keep one blank column between a heading and the source text.
                let keep = if blame_w > 1 { blame_w - 1 } else { blame_w };
                let text: String = text.chars().take(keep).collect();
                let text = format!("{text:<blame_w$}");
                buf.set_stringn(
                    ox,
                    oy + *y as u16,
                    text,
                    blame_w,
                    Style::default().fg(Color::Indexed(244)),
                );
            }
            if gutter > 0 && row == 0 {
                let mark = ed.git.mark(l).map(git_sign);
                let num = if cfg.relative_numbers && l != ed.cur.line {
                    l.abs_diff(ed.cur.line)
                } else {
                    l + 1
                };
                let mut style = if l == ed.cur.line {
                    cur_num_style
                } else {
                    num_style
                };
                if let Some((bar, color)) = mark.filter(|_| sign > 0) {
                    buf.set_stringn(
                        ox + blame_w as u16,
                        oy + *y as u16,
                        bar,
                        1,
                        Style::default().fg(color),
                    );
                    style = style.fg(color);
                }
                let numbers = gutter - sign - blame_w;
                if numbers > 0 {
                    let text = format!("{num:>w$} ", w = numbers - 1);
                    buf.set_stringn(
                        ox + (blame_w + sign) as u16,
                        oy + *y as u16,
                        text,
                        numbers,
                        style,
                    );
                }
            }
            buf.set_line(
                ox + gutter as u16,
                oy + *y as u16,
                &Line::from(spans),
                cols as u16,
            );
            // magit-blame highlight style: the first line of each chunk.
            if row == 0
                && blame.is_some_and(|b| {
                    b.style() == "highlight" && b.chunk_at(l).is_some_and(|c| c.line == l)
                })
            {
                buf.set_style(
                    Rect::new(ox + gutter as u16, oy + *y as u16, cols as u16, 1),
                    Style::default().bg(Color::Indexed(237)),
                );
            }
            if cfg.hl_line && l == ed.cur.line && !selected && !splash {
                buf.set_style(
                    Rect::new(ox + gutter as u16, oy + *y as u16, cols as u16, 1),
                    Style::default().bg(Color::Indexed(235)),
                );
            }
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
                if cr >= skip && first_y + cr - skip < rows {
                    cursor = Some((gutter + cx, first_y + cr - skip));
                }
            }
        } else {
            emit(&mut y, 0, visible(&line, &st, ed.tabstop, view.left, cols));
            if l == ed.cur.line {
                let cc = col_of_byte(&line, ed.cur.byte, ed.tabstop);
                if cc >= view.left && cc - view.left < cols {
                    cursor = Some((gutter + cc - view.left, first_y));
                }
            }
        }
        if let Some((r, _)) = &ed.explain {
            if l == r.start && skip == 0 {
                ex_top = Some(first_y);
            }
            if l == r.end {
                ex_end = Some(y);
            }
        }
        l += 1;
    }
    while y < rows {
        buf.set_stringn(ox, oy + y as u16, "~", 1, num_style);
        y += 1;
    }
    if let Some(zap) = &ed.zap {
        for (word, label) in zap.choices() {
            if !label.starts_with(&zap.label_prefix) {
                continue;
            }
            let highlight = Style::default().bg(Color::Yellow).fg(Color::Black);
            for &(x, y, width) in word.cells.iter().take(zap.query.graphemes(true).count()) {
                for dx in 0..width {
                    if x + dx < cols && y < rows {
                        buf[(ox + (gutter + x + dx) as u16, oy + y as u16)].set_style(highlight);
                    }
                }
            }
            if let Some(&(x, y, _)) = word.cells.first()
                && x < cols
                && y < rows
            {
                buf.set_stringn(
                    ox + (gutter + x) as u16,
                    oy + y as u16,
                    &label[zap.label_prefix.len()..],
                    cols - x,
                    Style::default()
                        .bg(Color::Magenta)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
    }
    // `fred` with no file: the start screen, until there's text.
    if splash {
        super::splash::dashboard(
            buf,
            Rect::new(ox, oy, area.width, rows as u16),
            ed,
            cfg.icons,
        );
    }
    view.explain_box = ed.explain.as_ref().and_then(|(_, text)| {
        let text_area = Rect::new(ox, oy, area.width, rows as u16);
        draw_explain(buf, text_area, gutter, text, ex_top, ex_end)
    });
    if let Mode::Pick(p) = &ed.mode {
        let list = Rect::new(ox, oy + rows as u16 + 2, area.width, panel as u16);
        if p.kind == Kind::MagitMenu {
            for (i, line) in transient_lines(ed, p.sel, area.width as usize)
                .into_iter()
                .take(panel)
                .enumerate()
            {
                buf.set_line(list.x, list.y + i as u16, &line, list.width);
            }
        } else {
            draw_picker(buf, list, p, view.picker_offset, ed, hl, cfg, budget);
        }
    }
    let input_area = Rect::new(ox, oy, area.width, area.height - panel as u16);
    if area.height >= 2 {
        draw_status(buf, input_area, ed, hl);
    }
    let cmd_cursor = draw_command_row(buf, input_area, ed);
    if let Some((cx, cy)) = cursor
        && cy < rows
    {
        let geom = Geom {
            gutter,
            rows,
            wrap_cols: if cfg.wrap { cols } else { 0 },
        };
        draw_popup(buf, area, ed, view, geom, (cx, cy));
    }
    if let Mode::Command(cl) = &ed.mode {
        draw_ex_completion(buf, input_area, cl);
    }
    match (cmd_cursor, cursor) {
        (Some(x), _) => f.set_cursor_position((ox + x as u16, oy + input_area.height - 1)),
        (None, Some((x, y))) if rows > 0 => {
            let x = x.min(area.width as usize - 1) as u16;
            let y = y.min(rows - 1) as u16;
            f.set_cursor_position((ox + x, oy + y));
        }
        _ => {}
    }
}

/// Ex candidates grow upward from the command line, best first.
fn draw_ex_completion(buf: &mut Screen, area: Rect, cl: &CmdLine) {
    let Some((items, selected, _)) = &cl.comp else {
        return;
    };
    let height = items.len().min(area.height.saturating_sub(1) as usize);
    if height == 0 || area.width == 0 {
        return;
    }
    let width = (items
        .iter()
        .map(|s| display_width(s, 1, 0))
        .max()
        .unwrap_or(0)
        + 2)
    .min(area.width as usize);
    let off = (selected + 1)
        .saturating_sub(height)
        .min(items.len() - height);
    let top = area.bottom() - 1 - height as u16;
    for (row, item) in items.iter().skip(off).take(height).enumerate() {
        let style = if row + off == *selected {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            Style::default()
                .bg(Color::Indexed(235))
                .fg(Color::Indexed(250))
        };
        let y = top + row as u16;
        buf.set_stringn(area.x, y, " ".repeat(width), width, style);
        buf.set_stringn(area.x + 1, y, item, width.saturating_sub(2), style);
    }
}

/// The mode's colour on the status line (the terminal's own palette).
fn mode_color(m: &Mode) -> Color {
    match m {
        Mode::Normal => Color::Blue,
        Mode::Insert => Color::Green,
        Mode::VisualLine { .. } => Color::Magenta,
        Mode::Command(_) => Color::Yellow,
        Mode::Pick(_) => Color::Cyan,
    }
}

fn draw_status(buf: &mut Screen, area: Rect, ed: &Editor, hl: &Highlighter) {
    let y = area.y + area.height - 2;
    let w = area.width as usize;
    let bar = Style::default()
        .bg(Color::Indexed(235))
        .fg(Color::Indexed(250));
    let bold = bar.add_modifier(Modifier::BOLD);
    let mode = bar.fg(mode_color(&ed.mode)).add_modifier(Modifier::BOLD);
    let label = match (&ed.mode, &ed.dired) {
        _ if ed.zap.is_some() => "ZAP".to_string(),
        _ if ed.magit.is_some() => "MAGIT".to_string(),
        (Mode::Normal, Some(d)) if !d.editing => "DIRED".to_string(),
        (m, _) => mode_name(m).to_string(),
    };
    buf.set_stringn(area.x, y, " ".repeat(w), w, bar);
    let mut left = vec![
        Span::styled("▎", mode),
        Span::styled(format!(" {label} "), mode),
    ];
    if let Mode::Pick(p) = &ed.mode {
        let (text, err) = match &ed.msg {
            Some((m, true)) => (m.as_str(), true),
            _ => (p.status.as_str(), p.err),
        };
        left.push(Span::styled(
            text,
            if err { bar.fg(Color::LightRed) } else { bar },
        ));
        buf.set_line(area.x, y, &Line::from(left), area.width);
        return;
    }
    let line = ed.buf.line(ed.cur.line);
    let col = col_of_byte(&line, ed.cur.byte, ed.tabstop) + 1;
    let mut right = vec![Span::styled(format!(" {}:{col} ", ed.cur.line + 1), bold)];
    if w >= 50 {
        let percent = (ed.cur.line + 1) * 100 / ed.line_count().max(1);
        right.push(Span::styled(format!("{percent}% "), bar));
    }
    if w >= 80 {
        let ending = match ed.buf.line_ending {
            crate::buffer::LineEnding::Lf => "LF",
            crate::buffer::LineEnding::CrLf => "CRLF",
        };
        right.push(Span::styled(
            format!(" {ending} UTF-8{} ", if ed.buf.bom { "+BOM" } else { "" }),
            bar,
        ));
    }
    if w >= 60 && hl.syntax_name() != "Plain Text" {
        right.push(Span::styled(
            format!(" {} ", hl.syntax_name()),
            bold.fg(Color::LightBlue),
        ));
    }
    if w >= 80
        && let Some(branch) = ed.git.branch()
    {
        right.push(Span::styled(
            format!(" ⎇ {} ", status_tail(&branch, 20)),
            bold.fg(Color::LightGreen),
        ));
    }
    let filename = ed.magit.as_ref().map_or_else(
        || {
            ed.path.as_ref().and_then(|p| p.file_name()).map_or_else(
                || "[No Name]".to_string(),
                |s| s.to_string_lossy().into_owned(),
            )
        },
        |v| v.title(),
    );
    let flags_width = usize::from(ed.buf.modified) * 4 + usize::from(ed.readonly) * 5;
    let core_width =
        display_width(&label, 1, 0) + 4 + display_width(&filename, 1, 0).min(20) + flags_width + 1;
    while right.len() > 1
        && right
            .iter()
            .map(|s| display_width(&s.content, 1, 0))
            .sum::<usize>()
            + core_width
            > w
    {
        right.pop();
    }
    let rw: usize = right.iter().map(|s| display_width(&s.content, 1, 0)).sum();
    let lw = w.saturating_sub(rw);
    let bytes = ed.buf.len_bytes();
    if w >= 60 {
        let size = if bytes >= 1_000_000 {
            format!("{:.1}M", bytes as f64 / 1_000_000.0)
        } else if bytes >= 1_000 {
            format!("{}k", bytes / 1_000)
        } else {
            format!("{bytes}B")
        };
        let detail = format!(" {size}  F ");
        if lw >= core_width + display_width(&detail, 1, 0) {
            left.push(Span::styled(detail, bar));
        }
    }
    let name = ed.magit.as_ref().map_or_else(
        || {
            ed.path.as_ref().map_or_else(
                || "[No Name]".to_string(),
                |p| crate::pick::browse::tilde(p),
            )
        },
        |v| v.title(),
    );
    let flags = format!(
        "{}{}",
        if ed.buf.modified { " [+]" } else { "" },
        if ed.readonly { " [RO]" } else { "" }
    );
    let used: usize = left.iter().map(|s| display_width(&s.content, 1, 0)).sum();
    let name = status_tail(
        &name,
        lw.saturating_sub(used + display_width(&flags, 1, 0) + 1),
    );
    if let Some((dir, file)) = name.rsplit_once('/') {
        left.push(Span::styled(format!("{dir}/"), bar.fg(Color::DarkGray)));
        left.push(Span::styled(file.to_string(), bold));
    } else {
        left.push(Span::styled(name, bold));
    }
    if ed.buf.modified {
        left.push(Span::styled(" [+]", bar.fg(Color::LightYellow)));
    }
    if ed.readonly {
        left.push(Span::styled(" [RO]", bar.fg(Color::LightRed)));
    }
    buf.set_line(area.x, y, &Line::from(left), lw as u16);
    if rw <= w {
        buf.set_line(area.x + lw as u16, y, &Line::from(right), rw as u16);
    }
}

/// Keep the filename end of a path when the modeline runs out of space.
pub(super) fn status_tail(text: &str, width: usize) -> String {
    if display_width(text, 1, 0) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 1;
    let mut start = text.len();
    for (byte, g) in text.grapheme_indices(true).rev() {
        used += display_width(g, 1, 0);
        if used > width {
            break;
        }
        start = byte;
    }
    format!("…{}", &text[start..])
}

/// Draws the bottom row; returns the cursor column when typing a command.
fn draw_command_row(buf: &mut Screen, area: Rect, ed: &Editor) -> Option<usize> {
    let y = area.y + area.height - 1;
    let w = area.width as usize;
    if let Some(zap) = &ed.zap {
        let text = format!(
            "zap> {}{}",
            zap.query,
            if zap.label_prefix.is_empty() {
                String::new()
            } else {
                format!(" [{}]", zap.label_prefix)
            }
        );
        buf.set_stringn(area.x, y, &text, w, Style::default());
        return Some(display_width(&text, 1, 0).min(w.saturating_sub(1)));
    }
    match &ed.mode {
        Mode::Pick(p) => Some(draw_input(buf, area, p.prompt(), &p.query)),
        Mode::Command(cl) if !cl.prompt.is_empty() => Some(draw_input(buf, area, &cl.prompt, cl)),
        Mode::Command(cl) => Some(draw_input(buf, area, cl.kind.encode_utf8(&mut [0; 4]), cl)),
        _ => {
            if let Some((m, err)) = &ed.msg {
                let style = if *err {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                buf.set_stringn(area.x, y, m, w, style);
            } else if let Some(m) = ed
                .blame
                .as_ref()
                .filter(|b| b.version == ed.buf.version)
                .and_then(|b| b.message(ed.cur.line))
            {
                buf.set_stringn(area.x, y, m, w, Style::default());
            }
            None
        }
    }
}

/// Draws `prompt` and the line being typed on the bottom row, scrolled so
/// the cursor shows; returns the cursor column.
fn draw_input(buf: &mut Screen, area: Rect, prompt: &str, cl: &CmdLine) -> usize {
    let y = area.y + area.height - 1;
    let w = area.width as usize;
    let full = format!("{prompt}{}", cl.text);
    let ccol = display_width(prompt, 1, 0) + display_width(&cl.text[..cl.cursor], 1, 0);
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
    ccol - skip
}

/// Picker rows: as many as there are results (at least one), up to half
/// the text rows.
/// One transient entry's styled cells.
type Cells = Vec<Span<'static>>;
/// A transient group: heading and its (entry index, cells).
type Group<'a> = (&'a str, Vec<(usize, Cells)>);

/// A Magit menu drawn like Emacs' transient: argument groups first, one entry
/// per line, then action groups side by side in columns. Switches that are on
/// are highlighted and show their argument; choices show their value.
pub fn transient_lines(ed: &Editor, selected: usize, width: usize) -> Vec<Line<'static>> {
    use crate::magit::{Action, current_choice, menu_entries};
    let menu = ed.magit_menu.unwrap_or('*');
    let entries = menu_entries(menu);
    let heading = Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD);
    let on = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let off = Style::default().fg(Color::DarkGray);
    // Each entry as styled cells, grouped in order of first appearance.
    let mut groups: Vec<Group> = vec![];
    for (i, (key, group, label, action)) in entries.iter().enumerate() {
        let mut spans = vec![
            Span::styled(format!("{key:>3} "), key_style),
            Span::raw(label.to_string()),
        ];
        match action {
            Action::ToggleOption(o) => {
                let set = ed.magit_options.contains(o);
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("({})", o.argument()),
                    if set { on } else { off },
                ));
            }
            Action::ReadOption(prefix) => {
                spans.push(Span::raw(" "));
                spans.push(
                    match ed
                        .magit_values
                        .get(&(crate::magit::arg_menu(menu), *prefix))
                    {
                        Some(v) => Span::styled(format!("({prefix}{v})"), on),
                        None => Span::styled(format!("({prefix})"), off),
                    },
                );
            }
            Action::CycleOption(prefix) => {
                spans.push(Span::raw(" "));
                spans.push(
                    match current_choice(ed, crate::magit::arg_menu(menu), prefix) {
                        Some(v) => Span::styled(format!("({prefix}{v})"), on),
                        None => Span::styled(format!("({prefix})"), off),
                    },
                );
            }
            _ => {}
        }
        if i == selected {
            reversed(&mut spans);
        }
        match groups.iter_mut().find(|(g, _)| g == group) {
            Some((_, list)) => list.push((i, spans)),
            None => groups.push((group, vec![(i, spans)])),
        }
    }
    let span_width = |s: &[Span]| {
        s.iter()
            .map(|x| display_width(&x.content, 1, 0))
            .sum::<usize>()
    };
    let mut lines: Vec<Line<'static>> = vec![];
    // Argument groups: full width, one entry per line.
    for (group, list) in groups.iter().filter(|(g, _)| g.starts_with("Arguments")) {
        lines.push(Line::from(Span::styled(group.to_string(), heading)));
        lines.extend(list.iter().map(|(_, s)| Line::from(s.clone())));
    }
    // Action groups: columns, wrapping to a new band when the width runs out.
    let actions: Vec<_> = groups
        .iter()
        .filter(|(g, _)| !g.starts_with("Arguments"))
        .collect();
    let mut band: Vec<(usize, &Group)> = vec![];
    let mut used = 0;
    let flush = |band: &mut Vec<(usize, &Group)>, lines: &mut Vec<Line<'static>>| {
        if band.is_empty() {
            return;
        }
        let height = band
            .iter()
            .map(|(_, (_, l))| l.len() + 1)
            .max()
            .unwrap_or(0);
        for row in 0..height {
            let mut spans: Vec<Span<'static>> = vec![];
            for (w, (group, list)) in band.iter() {
                let cell: Vec<Span<'static>> = if row == 0 {
                    vec![Span::styled(group.to_string(), heading)]
                } else {
                    list.get(row - 1)
                        .map(|(_, s)| s.clone())
                        .unwrap_or_default()
                };
                let pad = w.saturating_sub(span_width(&cell));
                spans.extend(cell);
                spans.push(Span::raw(" ".repeat(pad)));
            }
            lines.push(Line::from(spans));
        }
        band.clear();
    };
    for g in actions {
        let w =
            g.1.iter()
                .map(|(_, s)| span_width(s))
                .chain([display_width(g.0, 1, 0)])
                .max()
                .unwrap_or(0)
                + 3;
        if used + w > width && !band.is_empty() {
            flush(&mut band, &mut lines);
            used = 0;
        }
        band.push((w, g));
        used += w;
    }
    flush(&mut band, &mut lines);
    lines
}

fn panel_rows(text_rows: usize, results: usize) -> usize {
    results.clamp(1, (text_rows / 2).max(1)).min(text_rows)
}

/// Picker results in `area` (the panel), best at the top: code in its
/// syntax colors, matches in bold underline, and for files a dot when git
/// has them changed (yellow) or new (green), then its type's icon.
fn draw_picker(
    buf: &mut Screen,
    area: Rect,
    p: &Picker,
    off: usize,
    ed: &Editor,
    hl: &mut Highlighter,
    cfg: &Config,
    budget: Duration,
) {
    let rows = area.height as usize;
    let files = matches!(
        p.kind,
        Kind::Files | Kind::Browse | Kind::Recent | Kind::Buffers
    );
    // The text area's highlighting ran first; don't lose its "not done".
    let mut incomplete = hl.incomplete();
    let started = std::time::Instant::now();
    for (i, r) in p.rows.iter().enumerate().skip(off).take(rows) {
        let y = area.y + (i - off) as u16;
        let selected = i == p.sel;
        let mut spans = vec![Span::raw(if selected { ">" } else { " " })];
        if files {
            let state = ed.project.file_state(&r.path, r.text.ends_with('/'));
            spans.push(match state {
                Some(FileState::Modified) => Span::styled("●", Style::default().fg(Color::Yellow)),
                Some(FileState::New) => Span::styled("●", Style::default().fg(Color::Green)),
                None => Span::raw(" "),
            });
            if cfg.icons {
                // By path: a buffer row's text has its number in front.
                let name = match r.path.file_name() {
                    Some(n) if !r.text.ends_with('/') => n.to_string_lossy(),
                    _ => r.text.as_str().into(),
                };
                let (icon, color) = file_icon(&name);
                spans.push(Span::raw(" "));
                spans.push(Span::styled(icon.to_string(), Style::default().fg(color)));
            }
        }
        spans.push(Span::raw(" "));
        let code = r.code.map(|at| {
            let styles = if p.kind == Kind::Lines {
                // This file's own highlighting, exactly as in the editor.
                let left = budget.saturating_sub(started.elapsed());
                let st = hl.styles(&ed.buf, r.line..r.line + 1, left);
                incomplete |= hl.incomplete();
                st.into_iter().next().flatten()
            } else {
                hl.one_line(&r.path, &r.text[at..])
            };
            (at, styles)
        });
        spans.extend(row_spans(r, code.as_ref()));
        if selected {
            reversed(&mut spans);
        }
        buf.set_line(area.x, y, &Line::from(spans), area.width);
    }
    if incomplete {
        hl.set_incomplete();
    }
}

/// A Material Design icon (Nerd Font `nf-md-*`) and color for a file
/// picker row, by name: directories end in `/`.
fn file_icon(name: &str) -> (char, Color) {
    if name.ends_with('/') {
        return ('\u{f024b}', Color::Blue);
    }
    let base = name.rsplit('/').next().unwrap_or(name);
    let ext = base
        .rsplit_once('.')
        .map_or("", |(_, e)| e)
        .to_ascii_lowercase();
    match base {
        "Dockerfile" | "Containerfile" => return ('\u{f0868}', Color::Blue),
        "Makefile" | "justfile" => return ('\u{f0493}', Color::DarkGray),
        "LICENSE" | "LICENSE.md" | "COPYING" => return ('\u{f0fc3}', Color::Yellow),
        ".gitignore" | ".gitattributes" | ".gitmodules" => return ('\u{f02a2}', Color::Red),
        _ => {}
    }
    match ext.as_str() {
        "rs" => ('\u{f1617}', Color::Red),
        "py" | "pyi" => ('\u{f0320}', Color::Yellow),
        "js" | "mjs" | "cjs" => ('\u{f031e}', Color::Yellow),
        "ts" | "mts" | "cts" => ('\u{f06e6}', Color::Blue),
        "jsx" | "tsx" => ('\u{f0708}', Color::Cyan),
        "vue" => ('\u{f0844}', Color::Green),
        "html" | "htm" | "twig" => ('\u{f031d}', Color::Red),
        "css" => ('\u{f031c}', Color::Blue),
        "scss" | "sass" => ('\u{f07ec}', Color::Magenta),
        "md" | "markdown" => ('\u{f0354}', Color::White),
        "go" => ('\u{f07d3}', Color::Cyan),
        "c" | "h" => ('\u{f0671}', Color::Blue),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => ('\u{f0672}', Color::Blue),
        "java" => ('\u{f0b37}', Color::Red),
        "kt" | "kts" => ('\u{f1219}', Color::Magenta),
        "rb" => ('\u{f0d2d}', Color::Red),
        "php" | "module" | "theme" | "inc" | "install" => ('\u{f031f}', Color::Magenta),
        "swift" => ('\u{f06e5}', Color::Red),
        "lua" => ('\u{f08b1}', Color::Blue),
        "cs" => ('\u{f031b}', Color::Magenta),
        "hs" => ('\u{f0c92}', Color::Magenta),
        "r" => ('\u{f07d4}', Color::Blue),
        "nix" => ('\u{f1105}', Color::Blue),
        "json" | "jsonc" => ('\u{f0626}', Color::Yellow),
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" | "env" => ('\u{f0493}', Color::DarkGray),
        "xml" | "svg" => ('\u{f05c0}', Color::Yellow),
        "sh" | "bash" | "zsh" | "fish" => ('\u{f018d}', Color::Green),
        "sql" | "db" | "sqlite" => ('\u{f01bc}', Color::Cyan),
        "lock" => ('\u{f033e}', Color::DarkGray),
        "csv" | "tsv" => ('\u{f0c7e}', Color::Green),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" | "avif" => {
            ('\u{f021f}', Color::Magenta)
        }
        "pdf" => ('\u{f0226}', Color::Red),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" => ('\u{f0223}', Color::Cyan),
        "mp4" | "mov" | "mkv" | "webm" | "avi" => ('\u{f022b}', Color::Magenta),
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" => ('\u{f05c4}', Color::Yellow),
        "txt" | "log" => ('\u{f0219}', Color::White),
        _ => ('\u{f0214}', Color::DarkGray),
    }
}

/// A picker row's text: the head (`12: `) dim, code in `code`'s styles
/// (from byte `at`), matched ranges bold and underlined.
fn row_spans(r: &Row, code: Option<&(usize, Option<LineStyles>)>) -> Vec<Span<'static>> {
    let mut styler = code.map(|(at, st)| (*at, Styler::new(st)));
    let matched = Modifier::BOLD | Modifier::UNDERLINED;
    let mut out = vec![];
    let mut run = String::new();
    let mut style = Style::default();
    for (b, ch) in r.text.char_indices() {
        let mut st = match &mut styler {
            Some((at, s)) if b >= *at => s.at(b - *at),
            Some(_) => Style::default().fg(Color::DarkGray),
            None => Style::default(),
        };
        if r.hl.iter().any(|&(a, e)| (a..e).contains(&b)) {
            st = st.add_modifier(matched);
            if code.is_none() {
                st = st.fg(Color::Yellow);
            }
        }
        if st != style && !run.is_empty() {
            out.push(Span::styled(std::mem::take(&mut run), style));
        }
        style = st;
        // Control characters (tabs in grep lines) would upset the layout.
        run.push(if ch.is_control() { ' ' } else { ch });
    }
    if !run.is_empty() {
        out.push(Span::styled(run, style));
    }
    out
}

/// Where the text area is: gutter width, text rows, wrap width (0 = off).
/// `:explain`'s box: above the lines (`top` is their first row), or below
/// them (`end` is the row after) when that has more room. Returns where.
fn draw_explain(
    buf: &mut Screen,
    area: Rect,
    gutter: usize,
    text: &str,
    top: Option<usize>,
    end: Option<usize>,
) -> Option<Rect> {
    use ratatui::widgets::{Block, Clear, Widget};
    let width = (area.width as usize).min(80);
    if width < 10 || (top.is_none() && end.is_none()) {
        return None;
    }
    let lines = word_wrap(text, width - 4);
    let want = lines.len() + 2;
    let above = top.unwrap_or(0);
    let below = end.map_or(0, |e| (area.height as usize).saturating_sub(e));
    let (mut h, mut y0) = if top.is_some() && (above >= want || above >= below) {
        let h = want.min(above);
        (h, above - h)
    } else {
        (want.min(below), end.unwrap_or(0))
    };
    if h < 3 {
        // A full-viewport selection has no spare rows. Overlay its bottom
        // instead of silently losing the successful reply.
        h = want.min(area.height as usize);
        y0 = area.height as usize - h;
    }
    if h < 3 {
        return None;
    }
    let x0 = gutter.min(area.width as usize - width);
    let rect = Rect::new(
        area.x + x0 as u16,
        area.y + y0 as u16,
        width as u16,
        h as u16,
    );
    Clear.render(rect, buf);
    Block::bordered()
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Claude explains ")
        .title(Line::from(" ✕ ").right_aligned())
        .render(rect, buf);
    let inner = h - 2;
    for (i, line) in lines.iter().take(inner).enumerate() {
        let cut = i + 1 == inner && lines.len() > inner;
        let shown = if cut {
            format!("{line}…")
        } else {
            line.clone()
        };
        buf.set_stringn(
            rect.x + 2,
            rect.y + 1 + i as u16,
            shown,
            width - 4,
            Style::default(),
        );
    }
    Some(rect)
}

/// `text` broken at spaces into lines at most `width` columns wide.
// ponytail: a word wider than the box is cut off, not split.
fn word_wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = vec![];
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let w = display_width(word, 1, 0);
            if !line.is_empty() && display_width(&line, 1, 0) + 1 + w > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

#[derive(Clone, Copy)]
struct Geom {
    gutter: usize,
    rows: usize,
    wrap_cols: usize,
}

fn draw_popup(
    buf: &mut Screen,
    area: Rect,
    ed: &Editor,
    view: &View,
    geom: Geom,
    cursor: (usize, usize),
) {
    let Geom {
        gutter,
        rows,
        wrap_cols,
    } = geom;
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
