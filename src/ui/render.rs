//! Rendering the editor into a ratatui frame.

use super::layout::{Layout, Placed, wrap_cursor};
use super::view::View;
use crate::config::Config;
use crate::editor::{CmdLine, Editor, Mode};
use crate::git::FileState;
use crate::git::Mark;
use crate::highlight::{Highlighter, LineStyles};
use crate::options::Opt;
use crate::pick::{Kind, Picker, Row};
use crate::text::{col_of_byte, display_width, is_control};
use ratatui::Frame;
use ratatui::buffer::Buffer as Screen;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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

/// `st` with the bytes in `r` reversed.
fn reverse_bytes(st: &Option<LineStyles>, r: std::ops::Range<usize>) -> Option<LineStyles> {
    let mut cuts: Vec<usize> = st
        .iter()
        .flatten()
        .flat_map(|(_, s)| [s.start, s.end])
        .chain([r.start, r.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut styler = Styler::new(st);
    let spans = cuts.windows(2).map(|w| {
        let s = styler.at(w[0]);
        let s = if r.contains(&w[0]) {
            s.add_modifier(Modifier::REVERSED)
        } else {
            s
        };
        (s, w[0]..w[1])
    });
    Some(spans.collect())
}

fn digits(n: usize) -> usize {
    n.to_string().len()
}

pub fn gutter_width(ed: &Editor) -> usize {
    // magit-section-disable-line-numbers.
    let magit_off = ed.magit.is_some()
        && crate::magit::options::flag("magit-section-disable-line-numbers", true);
    let numbers = if (ed.opts.bool(Opt::Number) || ed.opts.bool(Opt::Relativenumber)) && !magit_off
    {
        digits(ed.line_count()).max(3) + 1
    } else {
        0
    };
    numbers + git_column(ed) + blame_column(ed)
}

/// Rows drawn above buffer line L that are not text: magit-blame's
/// headings style puts each chunk's heading on its own line (as upstream
/// does in a terminal).
pub(crate) fn virtual_rows(ed: &Editor, l: usize) -> usize {
    ed.blame
        .as_ref()
        .filter(|b| b.version == ed.buf.version && b.heading_above(l).is_some())
        .map_or(0, |_| 1)
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

pub(crate) fn mode_name(m: &Mode) -> &'static str {
    match m {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::VisualLine { .. } => "V-LINE",
        Mode::Visual { .. } => "VISUAL",
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
            // `:help` borrows Org's chooser.
            Kind::OrgChoice if p.title == "commands" => "HELP",
            Kind::OrgChoice if p.title == "symbols" => "IMENU",
            Kind::OrgMenu | Kind::OrgChoice => "ORG",
        },
    }
}

/// Mouse coordinates use the same layout as rendering, including inline offsets.
pub fn mouse(
    ed: &mut Editor,
    view: &mut View,
    area: Rect,
    event: ratatui::crossterm::event::MouseEvent,
) {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
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
                p.prompt = false;
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
    // The open right-click menu has the mouse: hovering highlights an
    // item, a click on one chooses it and a click elsewhere closes it.
    if let Some(m) = ed.menu.as_mut() {
        let row = view
            .menu_box
            .filter(|b| b.contains((event.column, event.row).into()))
            .map(|b| (event.row - b.y) as usize);
        let item = row
            .and_then(|r| m.items.get(r))
            .map(|i| i.0)
            .filter(|&i| i != crate::menu::Item::Separator);
        match event.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                m.sel = row.filter(|_| item.is_some());
                return;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                match item {
                    Some(i) => crate::menu::choose(ed, i),
                    None if row.is_some() => {}
                    None => ed.menu = None,
                }
                return;
            }
            // Another right-click opens it again where that one was.
            MouseEventKind::Down(MouseButton::Right) => ed.menu = None,
            MouseEventKind::Up(_) => return,
            _ => {
                ed.menu = None;
                return;
            }
        }
    }
    // A click elsewhere leaves the explain box open and does what it would.
    if ed.explain.is_some()
        && matches!(event.kind, MouseEventKind::Down(_))
        && let Some(b) = view.explain_box
        && b.contains((event.column, event.row).into())
    {
        // Its ✕ closes it; other clicks inside leave it be.
        if event.row == b.y && event.column + 4 >= b.right() {
            ed.explain = None;
        }
        return;
    }
    if ed.draw.is_some()
        && !matches!(ed.mode, Mode::Command(_))
        && crate::draw::mouse(
            ed,
            view,
            area,
            event,
            gutter_width(ed).min(area.width as usize / 2),
        )
    {
        view.last_click = None;
        return;
    }
    if event.row >= area.bottom().saturating_sub(2) || matches!(ed.mode, Mode::Command(_)) {
        view.last_click = None;
        return;
    }
    ed.zap = None;
    let gutter = gutter_width(ed).min(area.width as usize / 2);
    let cols = (area.width as usize).saturating_sub(gutter).max(1);
    let rows = (area.height as usize).saturating_sub(2).max(1);
    // 'selectmode' mouse: the mouse selects in Select mode.
    let select = ed.opts.has(Opt::Selectmode, "mouse");
    match event.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            view.last_click = None;
            view.wheel(
                ed,
                rows,
                cols,
                ed.opts.bool(Opt::Wrap),
                event.kind == MouseEventKind::ScrollDown,
            );
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let Some((line, byte)) = text_pos(ed, view, area, event, gutter, cols) else {
                return;
            };
            ed.popup = None;
            ed.vim.pending.clear();
            let triple = view.triple.take().is_some_and(|(then, l)| {
                l == line && then.elapsed() <= std::time::Duration::from_millis(400)
            });
            // Shift-click extends the selection (or starts one) to the click.
            if event.modifiers.contains(KeyModifiers::SHIFT) && ed.dired.is_none() {
                let anchor = match ed.mode {
                    Mode::Visual { anchor } => anchor,
                    _ => {
                        let from = ed.cur.pos();
                        crate::vim::select::start(ed, from, false, select);
                        from
                    }
                };
                ed.set_cursor(line, byte);
                ed.mode = Mode::Visual { anchor };
                view.drag_from = Some(anchor);
                return;
            }
            if ed.visual_lines().is_some() {
                crate::vim::select::end(ed);
            }
            ed.set_cursor(line, byte);
            view.drag_from = Some((line, ed.cur.byte));
            let text = ed.buf.line(line);
            // magit-mouse-toggle-section: a click on a section's v/>.
            if ed.magit.is_some() {
                let indicator = text.len() - text.trim_start().len();
                if byte == indicator
                    && matches!(text[indicator..].chars().next(), Some('v' | '>'))
                    && text[indicator..].chars().nth(1) == Some(' ')
                {
                    ed.handle_key(crate::key::Key::new(crate::key::KeyCode::Tab));
                    return;
                }
            }
            if ed.dired.as_ref().is_some_and(|d| !d.editing) {
                view.drag_from = None;
                view.detached = true;
                if view.double_click(false, line) {
                    view.detached = false;
                    ed.handle_key(crate::key::Key::new(crate::key::KeyCode::Enter));
                }
            } else if triple {
                // Triple-click selects the line.
                view.last_click = None;
                crate::vim::select::start(ed, (line, 0), true, select);
            } else if view.double_click(false, line) {
                // Double-click selects the word (or run of punctuation) under it.
                view.triple = Some((std::time::Instant::now(), line));
                let (lo, hi) = word_at(&text, ed.cur.byte);
                if hi > lo {
                    ed.set_cursor(line, crate::text::prev_grapheme(&text, hi));
                    crate::vim::select::start(ed, (line, lo), false, select);
                }
            }
        }
        // Dragging selects charwise from where the button went down, as in vim.
        MouseEventKind::Drag(MouseButton::Left) => {
            let Some(anchor) = view.drag_from else {
                return;
            };
            // ponytail: no autoscroll when dragging past the window edge.
            let Some((line, byte)) = text_pos(ed, view, area, event, gutter, cols) else {
                return;
            };
            view.last_click = None;
            ed.set_cursor(line, byte);
            if (line, ed.cur.byte) != anchor && ed.visual_lines().is_none() {
                crate::vim::select::start(ed, anchor, false, select);
            }
            if matches!(ed.mode, Mode::Visual { .. }) {
                ed.mode = Mode::Visual { anchor };
            }
        }
        MouseEventKind::Up(MouseButton::Left) => view.drag_from = None,
        // 'mousemodel' popup_setpos: the click moves the cursor (ending a
        // selection) unless it is inside the selection, then the menu opens.
        MouseEventKind::Down(MouseButton::Right) => {
            let Some((line, byte)) = text_pos(ed, view, area, event, gutter, cols) else {
                return;
            };
            ed.popup = None;
            ed.vim.pending.clear();
            view.last_click = None;
            let inside = match crate::vim::selection(ed, &ed.mode) {
                Some(crate::vim::ops::Span::Lines(lo, hi)) => (lo..=hi).contains(&line),
                Some(crate::vim::ops::Span::Chars(from, to)) => (from..to).contains(&(line, byte)),
                None => false,
            };
            if !inside {
                if matches!(ed.mode, Mode::Visual { .. } | Mode::VisualLine { .. }) {
                    ed.mode = Mode::Normal;
                }
                ed.set_cursor(line, byte);
            }
            crate::menu::open(ed, (event.column, event.row));
        }
        _ => {}
    }
}

/// The bytes of the word, or run of other non-blank characters, at `byte`.
fn word_at(text: &str, byte: usize) -> (usize, usize) {
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            1
        } else if c.is_whitespace() {
            0
        } else {
            2
        }
    };
    let Some(here) = text[byte..].chars().next().map(class) else {
        return (byte, byte);
    };
    let lo = text[..byte]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| class(c) == here)
        .last()
        .map_or(byte, |(i, _)| i);
    let hi = text[byte..]
        .char_indices()
        .find(|&(_, c)| class(c) != here)
        .map_or(text.len(), |(i, _)| byte + i);
    (lo, hi)
}

/// The buffer (line, byte) under a mouse event in the text area.
fn text_pos(
    ed: &Editor,
    view: &View,
    area: Rect,
    event: ratatui::crossterm::event::MouseEvent,
    gutter: usize,
    cols: usize,
) -> Option<(usize, usize)> {
    let mut y = (event.row - area.y) as usize + view.top_row;
    let x = ((event.column - area.x) as usize).saturating_sub(gutter);
    let wrap = ed.opts.bool(Opt::Wrap);
    for line in view.top..ed.line_count() {
        let text = ed.buf.line(line);
        let height = if line != ed.cur.line && ed.folds.hidden(line) {
            0
        } else if wrap {
            let height = super::layout::wrap_rows(&text, ed.tabstop(), cols);
            if line == ed.cur.line {
                height.max(wrap_cursor(&text, ed.cur.byte, ed.tabstop(), cols).0 + 1)
            } else {
                height
            }
        } else {
            1
        };
        // A blame heading row above the line counts as the line.
        let virt = if height > 0 {
            virtual_rows(ed, line)
        } else {
            0
        };
        if y >= height + virt {
            y -= height + virt;
            continue;
        }
        let y = y.saturating_sub(virt);
        let target = if wrap { x } else { x + view.left };
        let layout = || Layout::new(&text, ed.tabstop(), wrap.then_some(cols));
        let byte = layout()
            .find(|p| p.row == y && p.x + p.width > target)
            .map_or_else(
                || {
                    layout()
                        .filter(|p| p.row == y)
                        .last()
                        .map_or(text.len(), |p| p.byte + p.text.len())
                },
                |p| p.byte,
            );
        return Some((line, byte));
    }
    // Below the last line: its end, so drags past the text still select.
    let last = ed.line_count().checked_sub(1)?;
    Some((last, ed.buf.line(last).len()))
}

/// A Magit buffer line's colors: headings, and diff lines with
/// magit-diff-fontify-hunk's syntax colors over an added/removed tint and
/// magit-diff-refine-hunk's changed words.
fn magit_styles(ed: &Editor, hl: &Highlighter, l: usize, line: &str) -> LineStyles {
    let Some(view) = ed.magit.as_deref() else {
        return vec![];
    };
    let text = line.trim_start();
    let color = if text.starts_with('+') {
        Color::Green
    } else if text.starts_with('-') {
        Color::Red
    } else if text.starts_with("@@") {
        Color::Cyan
    } else if text.starts_with("Head:")
        || matches!(view.action_at(l), Some(crate::magit::RowAction::Section(_)))
    {
        Color::Yellow
    } else {
        Color::Reset
    };
    let base = vec![(Style::default().fg(color), 0..line.len())];
    let body = line;
    let sign = body.chars().next();
    if !matches!(sign, Some('+' | '-' | ' ')) || body.starts_with("+++") || body.starts_with("---")
    {
        return base;
    }
    // Inside a hunk: its header, then the file (diff header or status row).
    // Diff, commit and stash buffers keep the raw diff, headers included.
    let at = view.raw_at(l);
    let mut in_hunk = false;
    let mut path = None;
    let above: Box<dyn Iterator<Item = (usize, String)>> = match at {
        Some(r) => Box::new(
            (0..r)
                .rev()
                .map(|k| (k, view.raw_line(k).unwrap_or_default().into_owned())),
        ),
        None => Box::new((0..l).rev().map(|k| (k, ed.buf.line(k).to_string()))),
    };
    for (k, t) in above.take(5000) {
        let t = t.as_str();
        if t.starts_with("@@") {
            in_hunk = true;
        } else if let Some(f) = t.strip_prefix("+++ b/") {
            path = Some(std::path::PathBuf::from(f));
            break;
        } else if let Some(rest) = t.strip_prefix("diff --git a/") {
            path = rest
                .rsplit_once(" b/")
                .map(|(_, b)| std::path::PathBuf::from(b));
            break;
        } else if t.starts_with("diff ")
            || !(t.starts_with(['+', '-', ' ', '\\']) || (in_hunk && !t.is_empty()))
        {
            // A status file row (raw lines have no rows).
            if at.is_none()
                && let Some(crate::magit::RowAction::File(p, _)) = view.action_at(k)
            {
                path = Some(p);
            }
            break;
        }
    }
    if !in_hunk {
        return base;
    }
    let start = 1;
    let tint = match sign {
        Some('+') => Some(Color::Indexed(22)),
        Some('-') => Some(Color::Indexed(52)),
        _ => None,
    };
    let content_style = |st: Style| match tint {
        Some(bg) => st.bg(bg),
        None => st,
    };
    let mut spans: LineStyles = vec![(Style::default().fg(color), 0..start.min(line.len()))];
    let syntax = view
        .fontify
        .then(|| {
            path.as_deref()
                .and_then(|p| hl.one_line(p, &line[start.min(line.len())..]))
        })
        .flatten();
    match syntax {
        Some(st) => {
            let mut at = start;
            for (s, r) in st {
                let r = r.start + start..r.end + start;
                if r.start > at {
                    spans.push((content_style(Style::default()), at..r.start));
                }
                at = r.end;
                spans.push((content_style(s), r));
            }
            if at < line.len() {
                spans.push((content_style(Style::default()), at..line.len()));
            }
        }
        None => spans.push((
            Style::default().fg(color),
            start.min(line.len())..line.len(),
        )),
    }
    // magit-diff-paint-whitespace (t, or "uncommitted"), on the lines
    // magit-diff-paint-whitespace-lines picks (t added, "both", "all"),
    // with magit-diff-highlight-trailing.
    let opts = crate::magit::options::value;
    let paint = match opts("magit-diff-paint-whitespace") {
        Some(toml::Value::Boolean(false)) => false,
        Some(toml::Value::String(s)) if s == "uncommitted" => {
            !crate::magit::committed_diff(&view.kind)
        }
        _ => true,
    };
    let lines_ok = match opts("magit-diff-paint-whitespace-lines") {
        Some(toml::Value::String(s)) if s == "both" => sign != Some(' '),
        Some(toml::Value::String(s)) if s == "all" => true,
        _ => sign == Some('+'),
    };
    // magit-diff-highlight-indentation: [[DIR-REGEXP, "tabs" or N]], the
    // last matching the repository path wins.
    if paint
        && lines_ok
        && let Some(toml::Value::Array(rules)) = opts("magit-diff-highlight-indentation")
    {
        let root = view.repo.root.to_string_lossy().into_owned();
        let indent = rules
            .iter()
            .rev()
            .filter_map(|r| r.as_array())
            .find_map(|r| {
                let re = r.first()?.as_str()?;
                regex::Regex::new(&crate::magit::options::emacs_regex(re))
                    .ok()?
                    .is_match(&root)
                    .then(|| r.get(1).cloned())
                    .flatten()
            });
        let content = &line[start.min(line.len())..];
        let lead = content.len() - content.trim_start_matches([' ', '\t']).len();
        let bad = match indent {
            Some(toml::Value::String(t)) if t == "tabs" => content[..lead]
                .find('\t')
                .map(|i| i..content[..lead].rfind('\t').map_or(i, |j| j + 1)),
            Some(toml::Value::Integer(n)) if n > 0 => content[..lead]
                .find(&" ".repeat(n as usize))
                .map(|i| i..lead),
            _ => None,
        };
        if let Some(r) = bad {
            spans = paint_range(
                spans,
                start + r.start..start + r.end,
                Style::default().bg(Color::Red),
            );
        }
    }
    if paint && lines_ok && crate::magit::options::flag("magit-diff-highlight-trailing", true) {
        let content = &line[start.min(line.len())..];
        let trimmed = content.trim_end_matches([' ', '\t']).len();
        if trimmed < content.len() {
            let r = start + trimmed..line.len();
            spans = paint_range(spans, r, Style::default().bg(Color::Red));
        }
    }
    if view.refine && tint.is_some() {
        let rows: Vec<String> = ((l.saturating_sub(200))..(l + 200).min(ed.line_count()))
            .map(|k| ed.buf.line(k).to_string())
            .collect();
        let lines: Vec<&str> = rows.iter().map(String::as_str).collect();
        let me = l - l.saturating_sub(200);
        if let Some(other) = crate::magit::diff::refine_partner(&lines, me) {
            let (old, new) = if sign == Some('-') {
                (&lines[me][1..], &lines[other][1..])
            } else {
                (&lines[other][1..], &lines[me][1..])
            };
            let (a, b) = crate::magit::diff::refine(old, new);
            let changed = if sign == Some('-') { a } else { b };
            spans = emphasize(spans, &changed, start);
        }
    }
    spans
}

/// Give RANGE of the spans STYLE's background (spans stay sorted).
fn paint_range(spans: LineStyles, range: std::ops::Range<usize>, style: Style) -> LineStyles {
    let mut out = vec![];
    for (st, r) in spans {
        let a = r.start.max(range.start).min(r.end);
        let b = r.end.min(range.end).max(a);
        if r.start < a {
            out.push((st, r.start..a));
        }
        if a < b {
            out.push((st.patch(style), a..b));
        }
        if b < r.end {
            out.push((st, b..r.end));
        }
    }
    out
}
/// Split styled spans so the byte ranges in CHANGED (offset by SHIFT) gain
/// reverse video, keeping the spans sorted and non-overlapping.
fn emphasize(spans: LineStyles, changed: &[std::ops::Range<usize>], shift: usize) -> LineStyles {
    let mut out = vec![];
    for (st, r) in spans {
        let mut at = r.start;
        for c in changed {
            let c = (c.start + shift).max(r.start)..(c.end + shift).min(r.end);
            if c.start >= c.end || c.start < at {
                continue;
            }
            if c.start > at {
                out.push((st, at..c.start));
            }
            out.push((st.add_modifier(Modifier::REVERSED), c.clone()));
            at = c.end;
        }
        if at < r.end {
            out.push((st, at..r.end));
        }
    }
    out
}

/// Draw the editor; the frame area is the whole inline window.
/// The picker or Magit menu panel's height, below the command row.
fn panel_height(ed: &Editor, area: Rect) -> usize {
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
    panel.min(text_rows)
}

/// Rows below the windows: the command row and any picker panel.
pub fn chrome_rows(ed: &Editor, area: Rect) -> u16 {
    1 + panel_height(ed, area) as u16
}

/// One window, the whole screen but the command row and panel.
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
    let win = Rect {
        height: area.height.saturating_sub(chrome_rows(ed, area)),
        ..area
    };
    let win = crate::window::Placed {
        rect: win,
        ..Default::default()
    };
    let cursor = draw_window(f.buffer_mut(), win, ed, view, hl, cfg, budget, true);
    draw_chrome(f, area, ed, view, hl, cfg, budget, cursor);
}

/// One window in `rect`: its text, then its status line on the last row,
/// with the `│` divider in the last column when `divider`. An inactive
/// window shows no selection, popup, zap labels or explain box, and a dim
/// status line. Returns the cursor's cell when `active`.
#[allow(clippy::too_many_arguments)]
pub fn draw_window(
    buf: &mut Screen,
    placed: crate::window::Placed,
    ed: &Editor,
    view: &mut View,
    hl: &mut Highlighter,
    cfg: &Config,
    budget: Duration,
    active: bool,
) -> Option<(u16, u16)> {
    let (rect, divider) = (placed.rect, placed.divider);
    // The GUI's tab row and minimap margin are left out of the text area.
    let area = placed.text();
    if rect.height == 0 || rect.width <= u16::from(divider) || area.width == 0 {
        return None;
    }
    let status = Rect::new(
        rect.x,
        rect.bottom() - 1,
        rect.width - u16::from(divider),
        1,
    );
    if let Some(t) = &ed.term {
        let cursor = t.borrow_mut().draw(buf, area);
        draw_status(buf, status, ed, hl, active);
        draw_divider(buf, rect, divider);
        return cursor
            .filter(|_| active && ed.mode == Mode::Insert)
            .map(|(x, y, _)| (x, y));
    }
    let rows = area.height as usize;
    ed.drawn_sel.borrow_mut().clear();
    let normal = Mode::Normal;
    let mode = if active { &ed.mode } else { &normal };
    let picking = matches!(mode, Mode::Pick(_));
    let zap = ed.zap.as_ref().filter(|_| active);
    let gutter = gutter_width(ed).min(area.width as usize / 2);
    let blame_w = blame_column(ed).min(gutter);
    let sign = git_column(ed).min(gutter - blame_w);
    let blame = ed.blame.as_ref().filter(|b| b.version == ed.buf.version);
    let cols = (area.width as usize).saturating_sub(gutter).max(1);
    // magit-margin: author and date right-aligned beside commit rows, when
    // the window leaves the text at least as wide as the margin.
    let margin_w = ed.magit.as_ref().map_or(0, |v| v.margin_width() + 1);
    let margin_w = if cols >= margin_w * 2 { margin_w } else { 0 };
    let cols = cols - margin_w;
    let now = crate::magit::margin::now();
    if !picking && zap.is_none() {
        view.scroll(ed, rows.max(1), cols, ed.opts.bool(Opt::Wrap));
    }
    let n = ed.line_count();
    // Hidden lines take no rows, so the screen can reach further.
    let last = if ed.folds.is_empty() {
        (view.top + rows).min(n)
    } else {
        (ed.folds.down(view.top, rows, n) + 1).min(n)
    };
    let org_settings = if ed.org.is_some() {
        crate::org::settings(ed)
    } else {
        std::rc::Rc::default()
    };
    let styles = if ed.magit.is_some() || ed.org.is_some() {
        vec![]
    } else {
        hl.styles(&ed.buf, view.top..last, budget)
    };
    // Charwise Visual: its first and last positions, both included.
    let chars = match *mode {
        Mode::Visual { anchor } => Some((anchor.min(ed.cur.pos()), anchor.max(ed.cur.pos()))),
        _ => None,
    };
    let sel = match *mode {
        Mode::VisualLine { .. } | Mode::Visual { .. } => ed.visual_lines(),
        Mode::Command(ref cl) if cl.kind == ':' && cl.text.starts_with("'<,'>") => ed
            .marks
            .get(&'<')
            .zip(ed.marks.get(&'>'))
            .map(|(&a, &b)| (a.min(b), a.max(b))),
        _ => None,
    };
    let splash = ed.path.is_none()
        && ed.buf.len_bytes() == 0
        && matches!(mode, Mode::Normal | Mode::Command(_));
    let num_style = Style::default().fg(Color::DarkGray);
    let cur_num_style = Style::default().add_modifier(Modifier::BOLD);
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
        if l != ed.cur.line && ed.folds.hidden(l) {
            l += 1;
            continue;
        }
        let mut line = ed.buf.line(l);
        // org-ellipsis after a line whose following lines are folded.
        if ed.folds.folded_after(l) {
            line.push_str("...");
        }
        let st = if ed.magit.is_some() {
            Some(magit_styles(ed, hl, l, &line))
        } else if ed.dired.is_some() {
            Some(crate::dired::styles(ed, l))
        } else if ed.occur.is_some() {
            Some(crate::occur::styles(ed, l))
        } else if ed.org.is_some() {
            Some(crate::org::face::styles(ed, l, &org_settings))
        } else if let Some(st) = ed
            .claude
            .as_ref()
            .and_then(|_| crate::claude::styles(&line))
        {
            Some(st)
        } else {
            styles.get(l - view.top).cloned().flatten()
        };
        let in_sel = sel.is_some_and(|(a, b)| l >= a && l <= b);
        // The selection takes this line's line break.
        let with_break = in_sel
            && chars.is_none_or(|(_, b)| l < b.0 || b.1 >= ed.buf.line_len(l))
            && ed.buf.line_len(l) > 0;
        let sel_before = ed.drawn_sel.borrow().len();
        let mut selected = in_sel;
        // A charwise selection's partial first or last line reverses just its bytes.
        let st = match chars {
            Some((a, b)) if in_sel => {
                let lo = if l == a.0 { a.1 } else { 0 };
                let hi = if l == b.0 && b.1 < line.len() {
                    crate::text::next_grapheme(&line, b.1)
                } else {
                    line.len() + 1
                };
                if lo == 0 && hi > line.len() {
                    st
                } else {
                    selected = false;
                    reverse_bytes(&st, lo..hi)
                }
            }
            _ => st,
        };
        // Org: descriptive links and hidden markers, except on the cursor line.
        let (line, st) = if ed.org.is_some() && l != ed.cur.line {
            crate::org::face::conceal(line, st, &org_settings)
        } else {
            (line, st)
        };
        // Rows of this line already scrolled off the top: a blame heading
        // above it first, then its own.
        let top_skip = if l == view.top { view.top_row } else { 0 };
        let virt = virtual_rows(ed, l);
        if virt > 0 && top_skip == 0 {
            if let Some(text) = blame.and_then(|b| b.heading_above(l)) {
                let width = cols + gutter;
                let text: String = text.chars().take(width).collect();
                buf.set_stringn(
                    ox,
                    oy + y as u16,
                    format!("{text:<width$}"),
                    width,
                    Style::default().bg(Color::DarkGray).fg(Color::White),
                );
            }
            y += 1;
            if y >= rows {
                break;
            }
        }
        let skip = top_skip.saturating_sub(virt);
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
                let num = if ed.opts.bool(Opt::Relativenumber) && l != ed.cur.line {
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
            // In a selected line, the selection is its reversed cells.
            if in_sel {
                let x0 = ox + gutter as u16;
                let on: Vec<u16> = (x0..x0 + cols as u16)
                    .filter(|&x| {
                        buf[(x, oy + *y as u16)]
                            .modifier
                            .contains(Modifier::REVERSED)
                    })
                    .collect();
                if let (Some(a), Some(b)) = (on.first(), on.last()) {
                    // A wide character's second cell isn't marked: count it.
                    let wide = buf[(*b, oy + *y as u16)].symbol().width().max(1) as u16;
                    let end = (b + wide).min(x0 + cols as u16);
                    ed.drawn_sel.borrow_mut().push((*y as u16, a - ox, end - a));
                }
            }
            if margin_w > 0
                && row == 0
                && let Some(text) = ed.magit.as_ref().and_then(|v| v.margin_at(l, now))
            {
                let text = format!(" {text}");
                buf.set_stringn(
                    ox + (gutter + cols) as u16,
                    oy + *y as u16,
                    text,
                    margin_w,
                    Style::default().fg(Color::Indexed(244)),
                );
            }
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
            if ed.opts.bool(Opt::Cursorline) && l == ed.cur.line && !in_sel && !splash {
                buf.set_style(
                    Rect::new(ox + gutter as u16, oy + *y as u16, cols as u16, 1),
                    Style::default().bg(Color::Indexed(235)),
                );
            }
            *y += 1;
        };
        if ed.opts.bool(Opt::Wrap) {
            let mut styler = Styler::new(&st);
            let mut row = 0;
            let mut spans = vec![];
            for p in Layout::new(&line, ed.tabstop(), Some(cols)) {
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
                let (cr, cx) = wrap_cursor(&line, ed.cur.byte, ed.tabstop(), cols);
                // The end of a full row puts the cursor on a row of its own.
                if cr > row && y < rows {
                    emit(&mut y, cr, vec![]);
                }
                if cr >= skip && first_y + cr - skip < rows {
                    cursor = Some((gutter + cx, first_y + cr - skip));
                }
            }
        } else {
            emit(
                &mut y,
                0,
                visible(&line, &st, ed.tabstop(), view.left, cols),
            );
            if l == ed.cur.line {
                let cc = col_of_byte(&line, ed.cur.byte, ed.tabstop());
                if cc >= view.left && cc - view.left < cols {
                    cursor = Some((gutter + cc - view.left, first_y));
                }
            }
        }
        // One more cell for the line break, on the line's last row.
        if with_break {
            let mut d = ed.drawn_sel.borrow_mut();
            if d.len() > sel_before {
                if let Some(last) = d.last_mut()
                    && last.1 + last.2 < (gutter + cols) as u16
                {
                    last.2 += 1;
                }
            } else {
                // Only the line break is selected: its cell after the text.
                let text = ed.buf.line(l);
                let at = if ed.opts.bool(Opt::Wrap) {
                    let (cr, cx) = wrap_cursor(&text, text.len(), ed.tabstop(), cols);
                    (cr >= skip).then(|| (first_y + cr - skip, cx))
                } else {
                    let cc = col_of_byte(&text, text.len(), ed.tabstop());
                    (cc >= view.left && cc - view.left < cols).then(|| (first_y, cc - view.left))
                };
                if let Some((row, x)) = at.filter(|&(row, _)| row < rows) {
                    d.push((row as u16, (gutter + x) as u16, 1));
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
    // The terminal's block cursor inverts its cell: a selected one would go unreadable.
    if active
        && ed.visual_lines().is_some()
        && let Some((x, y)) = cursor
    {
        buf[(ox + x as u16, oy + y as u16)]
            .modifier
            .remove(Modifier::REVERSED);
    }
    if let Some(zap) = zap {
        draw_zap(buf, zap, None, (ox, oy), gutter, cols, rows);
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
    view.explain_box = ed
        .explain
        .as_ref()
        .filter(|_| active)
        .and_then(|(_, text)| {
            let text_area = Rect::new(ox, oy, area.width, rows as u16);
            draw_explain(buf, text_area, gutter, text, ex_top, ex_end)
        });
    draw_status(buf, status, ed, hl, active);
    draw_divider(buf, rect, divider);
    let (cx, cy) = cursor.filter(|_| active && rows > 0)?;
    if cy < rows {
        let geom = Geom {
            gutter,
            rows,
            wrap_cols: if ed.opts.bool(Opt::Wrap) { cols } else { 0 },
        };
        draw_popup(buf, area, ed, view, geom, (cx, cy));
    }
    let x = cx.min(area.width as usize - 1) as u16;
    let y = cy.min(rows - 1) as u16;
    Some((ox + x, oy + y))
}

/// A terminal's name in the status line: the title its shell set.
pub(crate) fn term_title(ed: &Editor) -> Option<String> {
    let title = ed.term.as_ref()?.borrow().title();
    Some(match title.as_str() {
        "" => "terminal".into(),
        t => format!("terminal · {t}"),
    })
}

/// The `│` between this window and the one to its right.
fn draw_divider(buf: &mut Screen, rect: Rect, divider: bool) {
    if divider {
        let dim = Style::default().fg(Color::Indexed(240));
        for y in rect.y..rect.bottom() {
            buf.set_stringn(rect.right() - 1, y, "│", 1, dim);
        }
    }
}

/// What belongs to fred rather than a window: the command row and the
/// picker or Magit menu panel at the bottom of `area`, the approval dialog,
/// the right-click menu and ex completion; then the terminal cursor
/// (`cursor`, the active window's, unless a command is being typed).
#[allow(clippy::too_many_arguments)]
pub fn draw_chrome(
    f: &mut Frame,
    area: Rect,
    ed: &Editor,
    view: &mut View,
    hl: &mut Highlighter,
    cfg: &Config,
    budget: Duration,
    cursor: Option<(u16, u16)>,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let panel = panel_height(ed, area);
    let rows = (area.height as usize).saturating_sub(2) - panel;
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
    let buf = f.buffer_mut();
    let (ox, oy) = (area.x, area.y);
    // Claude asking to use a tool: a dialog over the text until y or n.
    if let Some(q) = ed.msg.as_ref().and_then(|(m, _)| crate::claude::asked(m)) {
        // `y` and `n` answer in Normal mode only (see `Session::handle_key`).
        let typing = ed.mode != Mode::Normal || !ed.vim.pending.is_empty();
        draw_approval(buf, Rect::new(ox, oy, area.width, rows as u16), q, typing);
    }
    if let Mode::Pick(p) = &ed.mode {
        let list = Rect::new(
            ox,
            oy + area.height - panel as u16,
            area.width,
            panel as u16,
        );
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
    let cmd_cursor = draw_command_row(buf, input_area, ed);
    view.menu_box = ed.menu.as_ref().and_then(|m| draw_menu(buf, area, m));
    if let Mode::Command(cl) = &ed.mode {
        draw_ex_completion(buf, input_area, cl);
    }
    match (cmd_cursor, cursor) {
        (Some(x), _) => f.set_cursor_position((ox + x as u16, oy + input_area.height - 1)),
        (None, Some(at)) => f.set_cursor_position(at),
        _ => {}
    }
}

/// Ex candidates grow upward from the command line, best first.
fn draw_ex_completion(buf: &mut Screen, area: Rect, cl: &CmdLine) {
    // What the command being typed does, beside the completions (if any).
    let mut room = area;
    if let Some((items, selected, _)) = &cl.comp {
        room.height -= draw_completions(buf, area, items, *selected, cl.kind == ':');
    }
    if cl.kind == ':'
        && let Some(c) = crate::ex::cmd::typed(&cl.text)
    {
        draw_command_help(buf, room, c);
    }
}

/// The Tab completions, above the command line; command names say what
/// they do. Returns the rows taken.
fn draw_completions(
    buf: &mut Screen,
    area: Rect,
    items: &[String],
    selected: usize,
    ex: bool,
) -> u16 {
    let height = items.len().min(area.height.saturating_sub(1) as usize);
    if height == 0 || area.width == 0 {
        return 0;
    }
    let about = |item: &String| {
        let name = item.trim_end();
        ex.then(|| crate::ex::cmd::COMMANDS.iter().find(|c| c.name == name))
            .flatten()
            .map(|c| c.about)
    };
    let name_w = items
        .iter()
        .map(|s| display_width(s, 1, 0))
        .max()
        .unwrap_or(0);
    let about_w = items
        .iter()
        .filter_map(about)
        .map(str::len)
        .max()
        .map_or(0, |w| w + 2);
    let width = (name_w + about_w + 2).min(area.width as usize);
    let off = (selected + 1)
        .saturating_sub(height)
        .min(items.len() - height);
    let top = area.bottom() - 1 - height as u16;
    for (row, item) in items.iter().enumerate().skip(off).take(height) {
        let style = if row == selected {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            Style::default()
                .bg(Color::Indexed(235))
                .fg(Color::Indexed(250))
        };
        let y = top + (row - off) as u16;
        buf.set_stringn(area.x, y, " ".repeat(width), width, style);
        buf.set_stringn(area.x + 1, y, item, width.saturating_sub(2), style);
        if let Some(a) = about(item)
            && name_w + 3 < width
        {
            let dim = if row == selected {
                style
            } else {
                style.fg(Color::Indexed(244))
            };
            buf.set_stringn(area.x + 3 + name_w as u16, y, a, width - name_w - 3, dim);
        }
    }
    height as u16
}

/// Command `c`'s usage and what it does, in a box at the bottom of `area`,
/// above the command line.
fn draw_command_help(buf: &mut Screen, area: Rect, c: &crate::ex::cmd::Command) {
    use ratatui::widgets::Widget;
    let usage = c.synopsis();
    let inner = usage.chars().count().max(c.about.len()) as u16;
    let width = (inner + 4).min(area.right().saturating_sub(area.x));
    // On a short screen it would hide most of the text.
    if width < 8 || area.height < 10 {
        return;
    }
    let rect = Rect::new(area.x, area.bottom() - 5, width, 4);
    ratatui::widgets::Clear.render(rect, buf);
    let block = ratatui::widgets::Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));
    let text = block.inner(rect);
    block.render(rect, buf);
    let w = text.width as usize;
    buf.set_stringn(
        text.x + 1,
        text.y,
        &usage,
        w - 1,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    buf.set_stringn(text.x + 1, text.y + 1, c.about, w - 1, Style::default());
}

/// The mode's colour on the status line (the terminal's own palette).
fn mode_color(m: &Mode) -> Color {
    match m {
        Mode::Normal => Color::Blue,
        Mode::Insert => Color::Green,
        Mode::VisualLine { .. } | Mode::Visual { .. } => Color::Magenta,
        Mode::Command(_) => Color::Yellow,
        Mode::Pick(_) => Color::Cyan,
    }
}

/// The mode label the status line shows.
pub(crate) fn status_label(ed: &Editor) -> String {
    match (&ed.mode, &ed.dired) {
        _ if ed.zap.is_some() => "ZAP".to_string(),
        _ if ed.magit.is_some() => "MAGIT".to_string(),
        (Mode::Insert, _) if ed.term.is_some() => "TERMINAL".to_string(),
        (Mode::Normal, Some(d)) if !d.editing => "DIRED".to_string(),
        (Mode::Visual { .. }, _) if ed.vim.select => "SELECT".to_string(),
        (Mode::VisualLine { .. }, _) if ed.vim.select => "SELECT LINE".to_string(),
        (m, _) => mode_name(m).to_string(),
    }
}

/// The status line in `area`'s first row; dim and without the mode for an
/// inactive window.
fn draw_status(buf: &mut Screen, area: Rect, ed: &Editor, hl: &Highlighter, active: bool) {
    let y = area.y;
    let w = area.width as usize;
    let bar = Style::default()
        .bg(Color::Indexed(235))
        .fg(Color::Indexed(250));
    let bold = bar.add_modifier(Modifier::BOLD);
    let mode = bar.fg(mode_color(&ed.mode)).add_modifier(Modifier::BOLD);
    let label = status_label(ed);
    buf.set_stringn(area.x, y, " ".repeat(w), w, bar);
    // Drawing: the toolbar takes the status line.
    if let Some(d) = &ed.draw {
        let mut x = area.x;
        for (label, _, on) in crate::draw::toolbar(d) {
            let style = if on {
                mode.add_modifier(Modifier::REVERSED)
            } else {
                bar
            };
            let left = (area.right() - x) as usize;
            x = buf.set_stringn(x, y, &label, left, style).0;
        }
        return;
    }
    let mut left = if active {
        vec![
            Span::styled("▎", mode),
            Span::styled(format!(" {label} "), mode),
        ]
    } else {
        vec![Span::styled(" ", bar)]
    };
    // Claude at work, in whichever buffer you're watching it in.
    if let Some(w) = &ed.claude_working {
        left.push(Span::styled(format!(" {w} "), bold.fg(Color::LightMagenta)));
    }
    if let Mode::Pick(p) = &ed.mode
        && active
    {
        let (text, err) = match &ed.msg {
            Some((m, true)) => (m.as_str().into(), true),
            _ => (p.status_text(), p.err),
        };
        left.push(Span::styled(
            text,
            if err { bar.fg(Color::LightRed) } else { bar },
        ));
        buf.set_line(area.x, y, &Line::from(left), area.width);
        return;
    }
    let line = ed.buf.line(ed.cur.line);
    let col = col_of_byte(&line, ed.cur.byte, ed.tabstop()) + 1;
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
    let filename = ed.claude.as_ref().map_or(filename, |c| c.title());
    let filename = term_title(ed).unwrap_or(filename);
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
    let name = term_title(ed).unwrap_or(name);
    let flags = format!(
        "{}{}",
        if ed.buf.modified { " [+]" } else { "" },
        if ed.readonly && ed.term.is_none() {
            " [RO]"
        } else {
            ""
        }
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
    if !active {
        // Neovim's StatusLineNC.
        let dim = Style::default()
            .bg(Color::Indexed(236))
            .fg(Color::Indexed(244));
        buf.set_style(Rect::new(area.x, y, area.width, 1), dim);
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
            if let Some(st) = ed.msg.as_ref().and_then(|(m, _)| crate::claude::styles(m)) {
                let m = &ed.msg.as_ref().unwrap().0;
                let spans: Vec<_> = st
                    .into_iter()
                    .map(|(s, r)| Span::styled(&m[r], s))
                    .collect();
                buf.set_line(area.x, y, &Line::from(spans), area.width);
            } else if let Some((m, err)) = &ed.msg {
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
    use crate::magit::{Action, current_choice, shown_entries};
    let menu = ed.magit_menu.unwrap_or('*');
    let entries = shown_entries(ed, menu);
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
        let selected = !p.prompt && i == p.sel;
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

/// Claude's question as a dialog in the middle of the text, like Claude
/// Code's: the tool, all of what it would run, and the keys that answer.
fn draw_approval(buf: &mut Screen, area: Rect, (name, arg): (&str, &str), typing: bool) {
    use ratatui::widgets::{Block, BorderType, Clear, Widget};
    let width = area.width.min(72);
    if width < 24 || area.height < 6 {
        return;
    }
    let w = width as usize - 4;
    let arg = if arg.is_empty() {
        "(no arguments)"
    } else {
        arg
    };
    // Broken anywhere, not at spaces: no character of what is being allowed
    // may go unseen.
    let mut lines = vec![String::new()];
    for c in arg.chars().map(|c| if c == '\t' { ' ' } else { c }) {
        let cw = display_width(c.encode_utf8(&mut [0; 4]), 1, 0);
        if display_width(lines.last().unwrap(), 1, 0) + cw > w {
            lines.push(String::new());
        }
        lines.last_mut().unwrap().push(c);
    }
    let room = area.height as usize - 5;
    if lines.len() > room {
        let more = lines.len() - room + 1;
        lines.truncate(room - 1);
        lines.push(format!("… {more} more lines: read all of it in *claude*"));
    }
    let h = lines.len() as u16 + 5;
    let x0 = area.x + (area.width - width) / 2;
    let rect = Rect::new(x0, area.y + (area.height - h) / 2, width, h);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    Clear.render(rect, buf);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Line::from(vec![
            Span::styled(" ⏺ Claude wants to use ", bold.fg(Color::Yellow)),
            Span::styled(format!("{name} "), bold.fg(Color::Cyan)),
        ]))
        .render(rect, buf);
    for (i, l) in lines.iter().enumerate() {
        buf.set_stringn(
            x0 + 2,
            rect.y + 2 + i as u16,
            l,
            w,
            Style::default().fg(Color::White),
        );
    }
    let keys = Line::from(vec![
        Span::styled(
            if typing { "Esc, then" } else { "Allow?   " },
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" y ", bold.fg(Color::Black).bg(Color::Green)),
        Span::raw(" Yes    "),
        Span::styled(" n ", bold.fg(Color::Black).bg(Color::Red)),
        Span::raw(" No"),
    ]);
    buf.set_line(x0 + 2, rect.y + h - 2, &keys, width - 4);
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

/// The right-click menu, below and right of the click where it fits.
fn draw_menu(buf: &mut Screen, area: Rect, m: &crate::menu::Menu) -> Option<Rect> {
    let w = m.items.iter().map(|i| display_width(i.1, 1, 0)).max()? + 2;
    let (w, h) = (
        (w as u16).min(area.width),
        (m.items.len() as u16).min(area.height),
    );
    let (cx, cy) = m.at;
    let x = cx.min(area.right().saturating_sub(w)).max(area.x);
    let y = if cy + 1 + h <= area.bottom() {
        cy + 1
    } else {
        cy.saturating_sub(h).max(area.y)
    };
    let normal = Style::default().bg(Color::DarkGray).fg(Color::White);
    let selected = Style::default().bg(Color::White).fg(Color::Black);
    for (i, &(item, label)) in m.items.iter().take(h as usize).enumerate() {
        let (text, style) = match item {
            crate::menu::Item::Separator => ("─".repeat(w as usize), normal.fg(Color::Gray)),
            _ if m.sel == Some(i) => (format!(" {label:<0$}", w as usize - 1), selected),
            _ => (format!(" {label:<0$}", w as usize - 1), normal),
        };
        buf.set_stringn(x, y + i as u16, text, w as usize, style);
    }
    Some(Rect::new(x, y, w, h))
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
        let (sr, sx) = wrap_cursor(&line, start, ed.tabstop(), wrap_cols);
        let (cr, _) = wrap_cursor(&line, ed.cur.byte, ed.tabstop(), wrap_cols);
        gutter + if sr == cr { sx } else { 0 }
    } else {
        gutter + col_of_byte(&line, start, ed.tabstop()).saturating_sub(view.left)
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

/// Zap's labels on the words in window `win` (`None`: the zapping editor's
/// own), whose text starts at `origin` + `gutter`.
pub(crate) fn draw_zap(
    buf: &mut Screen,
    zap: &crate::zap::Zap,
    win: Option<usize>,
    (ox, oy): (u16, u16),
    gutter: usize,
    cols: usize,
    rows: usize,
) {
    for (word, label) in zap.choices() {
        if word.win != win || !label.starts_with(&zap.label_prefix) {
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
