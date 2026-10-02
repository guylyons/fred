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
    numbers + git_column(ed)
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
            Kind::Lines => "LINES",
            Kind::Browse => "FILES",
            Kind::Recent => "RECENT",
        },
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
        Mode::Pick(ref p) => panel_rows(text_rows, p.rows.len()),
        _ => 0,
    };
    let rows = text_rows - panel;
    let gutter = gutter_width(ed, cfg).min(area.width as usize / 2);
    let sign = git_column(ed).min(gutter);
    let cols = (area.width as usize).saturating_sub(gutter).max(1);
    if panel == 0 {
        view.scroll(ed, rows.max(1), cols, cfg.wrap);
    }
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
                if let Some((bar, color)) = mark {
                    buf.set_stringn(ox, oy + *y as u16, bar, 1, Style::default().fg(color));
                    style = style.fg(color);
                }
                let numbers = gutter - sign;
                if numbers > 0 {
                    let text = format!("{num:>w$} ", w = numbers - 1);
                    buf.set_stringn(ox + sign as u16, oy + *y as u16, text, numbers, style);
                }
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
    if let Mode::Pick(p) = &ed.mode {
        let list = Rect::new(ox, oy + rows as u16, area.width, panel as u16);
        draw_picker(buf, list, p, ed, hl, cfg, budget);
    }
    if area.height >= 2 {
        draw_status(buf, area, ed, hl);
    }
    let cmd_cursor = draw_command_row(buf, area, ed);
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
    let bar = Style::default().bg(Color::DarkGray).fg(Color::White);
    let bold = bar.add_modifier(Modifier::BOLD);
    let mode = Style::default()
        .bg(mode_color(&ed.mode))
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);
    let mut left = vec![
        Span::styled(format!(" {} ", mode_name(&ed.mode)), mode),
        Span::styled(" ", bar),
    ];
    let mut right = vec![];
    match &ed.mode {
        // An error from opening the pick (unsaved changes) replaces the count.
        Mode::Pick(p) => match &ed.msg {
            Some((m, true)) => left.push(Span::styled(m.clone(), bar.fg(Color::LightRed))),
            _ if p.err => left.push(Span::styled(p.status.clone(), bar.fg(Color::LightRed))),
            _ => left.push(Span::styled(p.status.clone(), bar)),
        },
        _ => {
            let name = ed
                .path
                .as_ref()
                .map_or_else(|| "[No Name]".to_string(), |p| p.display().to_string());
            left.push(Span::styled(name, bold));
            if ed.buf.modified {
                left.push(Span::styled(" [+]", bar.fg(Color::LightYellow)));
            }
            if ed.readonly {
                left.push(Span::styled(" [RO]", bar.fg(Color::LightRed)));
            }
            let line = ed.buf.line(ed.cur.line);
            let col = col_of_byte(&line, ed.cur.byte, ed.tabstop) + 1;
            if hl.syntax_name() != "Plain Text" {
                let ft = format!("{}  ", hl.syntax_name().to_lowercase());
                right.push(Span::styled(ft, bar.fg(Color::Gray)));
            }
            right.push(Span::styled(format!("{}:{col} ", ed.cur.line + 1), bold));
        }
    }
    let w = area.width as usize;
    let rw: usize = right.iter().map(|s| display_width(&s.content, 1, 0)).sum();
    buf.set_stringn(area.x, y, " ".repeat(w), w, bar);
    let lw = w.saturating_sub(rw + 1).max(1);
    buf.set_line(area.x, y, &Line::from(left), lw as u16);
    if rw < w {
        buf.set_line(area.x + (w - rw) as u16, y, &Line::from(right), rw as u16);
    }
}

/// Draws the bottom row; returns the cursor column when typing a command.
fn draw_command_row(buf: &mut Screen, area: Rect, ed: &Editor) -> Option<usize> {
    let y = area.y + area.height - 1;
    let w = area.width as usize;
    match &ed.mode {
        Mode::Pick(p) => Some(draw_input(buf, area, p.prompt(), &p.query)),
        Mode::Command(cl) => Some(draw_input(buf, area, cl.kind.encode_utf8(&mut [0; 4]), cl)),
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
fn panel_rows(text_rows: usize, results: usize) -> usize {
    results.clamp(1, (text_rows / 2).max(1)).min(text_rows)
}

/// Picker results in `area` (the panel), best at the bottom: code in its
/// syntax colors, matches in bold underline, and for files a dot when git
/// has them changed (yellow) or new (green), then its type's icon.
fn draw_picker(
    buf: &mut Screen,
    area: Rect,
    p: &Picker,
    ed: &Editor,
    hl: &mut Highlighter,
    cfg: &Config,
    budget: Duration,
) {
    let rows = area.height as usize;
    let off = (p.sel + 1).saturating_sub(rows);
    let files = matches!(p.kind, Kind::Files | Kind::Browse | Kind::Recent);
    // The text area's highlighting ran first; don't lose its "not done".
    let mut incomplete = hl.incomplete();
    let started = std::time::Instant::now();
    for (i, r) in p.rows.iter().enumerate().skip(off).take(rows) {
        let y = area.y + (rows - 1 - (i - off)) as u16;
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
                let (icon, color) = file_icon(&r.text);
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
