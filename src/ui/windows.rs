//! The whole screen: every window, then what belongs to fred.

use super::render::{chrome_rows, draw_chrome, draw_window, draw_zap, gutter_width};
use crate::config::Config;
use crate::session::Session;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use std::time::{Duration, Instant};

/// Lay the windows out above the command row, draw each, then the chrome.
/// The highlighting `budget` is shared: the active window draws first.
pub fn draw_session(f: &mut Frame, s: &mut Session, cfg: &Config, budget: Duration) {
    let area = f.area();
    if area.height == 0 || area.width == 0 {
        return;
    }
    s.area = Rect {
        height: area.height.saturating_sub(chrome_rows(&s.ed, area)),
        ..area
    };
    let start = Instant::now();
    let mut cursor = None;
    let mut incomplete = false;
    let buf = f.buffer_mut();
    // Other windows' text areas, for zap's labels on their words.
    let mut zap_areas = vec![];
    let zapping = s.ed.zap.is_some();
    s.each_window(|ed, view, hl, p, active| {
        let left = budget.saturating_sub(start.elapsed());
        let at = draw_window(buf, p, ed, view, hl, cfg, left, active);
        if active {
            cursor = at;
        } else if zapping {
            let text = p.text();
            let vp = crate::zap::Viewport::of(text, ed, *view);
            let gutter = gutter_width(ed).min(text.width as usize / 2);
            zap_areas.push((p.id, (text.x, text.y), gutter, vp.cols, vp.rows));
        }
        // Magit draws without the highlighter: its flag means nothing.
        incomplete |= ed.magit.is_none() && hl.incomplete();
    });
    s.hl_incomplete = incomplete;
    if let Some(zap) = &s.ed.zap {
        for (id, origin, gutter, cols, rows) in zap_areas {
            draw_zap(buf, zap, Some(id), origin, gutter, cols, rows);
        }
    }
    if s.picking_window {
        let order = s.tree.order();
        for p in s.placed() {
            if let Some(i) = order.iter().position(|&id| id == p.id) {
                // Below the GUI's tab row and clear of its minimap.
                let rect = Rect {
                    y: p.text().y,
                    height: p.rect.height.saturating_sub(p.header),
                    width: p.rect.width.saturating_sub(p.margin),
                    ..p.rect
                };
                number(buf, rect, i + 1);
            }
        }
    }
    if s.which_key
        && let Some((title, keys)) = crate::vim::which_key(&s.ed)
    {
        // Clear of the GUI's minimap.
        let area = Rect {
            width: s.area.width.saturating_sub(s.chrome.1),
            ..s.area
        };
        which_key(buf, area, title, &keys);
    }
    let mut view = s.win().view;
    draw_chrome(f, area, &s.ed, &mut view, &mut s.hl, cfg, budget, cursor);
    s.win_mut().view = view;
}

/// `Space w`: dim window `rect` and put its number `n` in the middle.
fn number(buf: &mut Buffer, rect: Rect, n: usize) {
    buf.set_style(rect, Style::default().add_modifier(Modifier::DIM));
    let label = n.to_string();
    let w = label.len() as u16 + 4;
    if rect.width < w || rect.height < 3 {
        return;
    }
    let (x, y) = (
        rect.x + (rect.width - w) / 2,
        rect.y + (rect.height - 3) / 2,
    );
    let style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
        .remove_modifier(Modifier::DIM);
    let bar = "─".repeat(label.len() + 2);
    buf.set_string(x, y, format!("╭{bar}╮"), style);
    buf.set_string(x, y + 1, format!("│ {label} │"), style);
    buf.set_string(x, y + 2, format!("╰{bar}╯"), style);
}

/// The keys that can follow `title`, in columns, along the bottom of `area`.
fn which_key(buf: &mut Buffer, area: Rect, title: &str, keys: &[(String, &str)]) {
    let width = keys
        .iter()
        .map(|(k, what)| k.len() + what.len() + 5)
        .max()
        .unwrap_or(1);
    let inner = area.width.saturating_sub(2) as usize;
    let cols = (inner / width).max(1);
    let rows = keys.len().div_ceil(cols);
    let height = (rows as u16 + 2).min(area.height);
    let rect = Rect {
        y: area.bottom() - height,
        height,
        ..area
    };
    Clear.render(rect, buf);
    let block = Block::default()
        .borders(Borders::TOP)
        .title(format!(" {title} "))
        .border_style(Style::default().fg(Color::DarkGray));
    let text = block.inner(rect);
    block.render(rect, buf);
    for (i, (key, what)) in keys.iter().enumerate() {
        let (col, row) = (i / rows, i % rows);
        let (x, y) = (text.x + 1 + (col * width) as u16, text.y + row as u16);
        if y >= text.bottom() || x >= text.right() {
            continue;
        }
        let room = (text.right() - x) as usize;
        let key_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        buf.set_stringn(x, y, key, room, key_style);
        let x = x + key.len() as u16;
        if x < text.right() {
            let line = format!(" → {what}");
            let what_style = if what.starts_with('+') {
                Style::default().fg(Color::Magenta)
            } else {
                Style::default()
            };
            buf.set_stringn(x, y, line, (text.right() - x) as usize, what_style);
        }
    }
}
