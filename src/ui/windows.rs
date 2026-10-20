//! The whole screen: every window, then what belongs to fred.

use super::render::{chrome_rows, draw_chrome, draw_window};
use crate::config::Config;
use crate::session::Session;
use ratatui::Frame;
use ratatui::layout::Rect;
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
    let mut view = s.win().view;
    draw_chrome(f, area, &s.ed, &mut view, &mut s.hl, cfg, budget, cursor);
    s.win_mut().view = view;
}
