//! The whole screen: every window, then what belongs to fred.

use super::render::{chrome_rows, draw_chrome, draw_window};
use crate::config::Config;
use crate::session::Session;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
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
    s.each_window(|ed, view, hl, p, active| {
        let left = budget.saturating_sub(start.elapsed());
        let at = draw_window(buf, p.rect, ed, view, hl, cfg, left, active, p.divider);
        if active {
            cursor = at;
        }
        // Magit draws without the highlighter: its flag means nothing.
        incomplete |= ed.magit.is_none() && hl.incomplete();
    });
    s.hl_incomplete = incomplete;
    if s.picking_window {
        let order = s.tree.order();
        for p in s.tree.layout(s.area) {
            if let Some(i) = order.iter().position(|&id| id == p.id) {
                number(buf, p.rect, i + 1);
            }
        }
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
    let (x, y) = (rect.x + (rect.width - w) / 2, rect.y + (rect.height - 3) / 2);
    let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD).remove_modifier(Modifier::DIM);
    let bar = "─".repeat(label.len() + 2);
    buf.set_string(x, y, format!("╭{bar}╮"), style);
    buf.set_string(x, y + 1, format!("│ {label} │"), style);
    buf.set_string(x, y + 2, format!("╰{bar}╯"), style);
}
