//! The start screen (`fred` with no file): Dr. Fred, in half-block pixels.

use ratatui::buffer::Buffer as Screen;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

/// One character per pixel: White, Cyan, Teal, blacK, Gray; `.` is clear.
const LOGO: &str = include_str!("logo.txt");
const TITLE: &str = "Dr. Fred";

pub fn active(ed: &crate::editor::Editor) -> bool {
    ed.path.is_none() && ed.buf.len_bytes() == 0
}

pub fn recent(ed: &crate::editor::Editor) -> Vec<std::path::PathBuf> {
    ed.project
        .recent_file
        .as_deref()
        .map(crate::pick::recent::load)
        .unwrap_or_default()
        .into_iter()
        .take(5)
        .collect()
}

pub fn dashboard(buf: &mut Screen, area: Rect, ed: &crate::editor::Editor) {
    let recent = recent(ed);
    let mut lines = vec![
        "[f] Find file    Space p".into(),
        "[g] Grep         Space g".into(),
        "[r] Recent files Space r".into(),
    ];
    if recent.is_empty() {
        lines.push("    No recent files".into());
    }
    for (i, path) in recent.iter().enumerate() {
        lines.push(format!("[{}] {}", i + 1, path.display()));
    }
    let logo_min = LOGO.lines().count().div_ceil(2) as u16 + 2;
    let height = (lines.len() as u16 + 1)
        .min(area.height.saturating_sub(logo_min).max(4))
        .min(area.height);
    let logo_height = area.height - height;
    draw(buf, Rect::new(area.x, area.y, area.width, logo_height));
    let width = lines
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.as_str()))
        .max()
        .unwrap_or(0)
        .min(area.width as usize);
    let x = area.x + (area.width - width as u16) / 2;
    for (i, line) in lines.iter().take(height as usize).enumerate() {
        buf.set_stringn(
            x,
            area.y + logo_height + i as u16,
            line,
            (area.right() - x) as usize,
            Style::default().fg(Color::Rgb(140, 235, 235)),
        );
    }
}

fn color(px: u8) -> Option<Color> {
    Some(match px {
        b'W' => Color::Rgb(240, 240, 240),
        b'C' => Color::Rgb(140, 235, 235),
        b'T' => Color::Rgb(0, 110, 110),
        b'K' => Color::Rgb(0, 0, 0),
        b'G' => Color::Rgb(170, 190, 190),
        _ => return None,
    })
}

/// Draw the logo and its title centered in `area`: pixels twice the size
/// when there's room, nothing when even one size doesn't fit.
pub fn draw(buf: &mut Screen, area: Rect) {
    let rows: Vec<&[u8]> = LOGO.lines().map(str::as_bytes).collect();
    let (w, h) = (rows[0].len(), rows.len());
    // Cells for the logo at `scale`, plus a gap and the title below.
    let size = |scale: usize| (w * scale, (h * scale).div_ceil(2) + 2);
    let fits = |(cw, ch): (usize, usize)| cw <= area.width as usize && ch <= area.height as usize;
    let Some(scale) = [2, 1].into_iter().find(|&s| fits(size(s))) else {
        return;
    };
    let (cw, ch) = size(scale);
    let x0 = area.x + (area.width - cw as u16) / 2;
    let y0 = area.y + (area.height - ch as u16) / 2;
    let px = |x: usize, y: usize| {
        rows.get(y / scale)
            .and_then(|r| r.get(x / scale))
            .and_then(|&p| color(p))
    };
    for cy in 0..ch - 2 {
        for cx in 0..cw {
            let cell = &mut buf[(x0 + cx as u16, y0 + cy as u16)];
            match (px(cx, 2 * cy), px(cx, 2 * cy + 1)) {
                (None, None) => {}
                (Some(top), bottom) => {
                    cell.set_symbol("▀")
                        .set_fg(top)
                        .set_bg(bottom.unwrap_or(Color::Reset));
                }
                (None, Some(bottom)) => {
                    cell.set_symbol("▄").set_fg(bottom).set_bg(Color::Reset);
                }
            }
        }
    }
    let title = Style::default()
        .fg(Color::Rgb(140, 235, 235))
        .add_modifier(Modifier::BOLD);
    let tx = area.x + (area.width - TITLE.len() as u16) / 2;
    buf.set_string(tx, y0 + ch as u16 - 1, TITLE, title);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(buf: &Screen) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn centered_and_scaled_to_fit() {
        // Room for double size: 48 cells wide, 28 tall, title below.
        let mut big = Screen::empty(Rect::new(0, 0, 100, 40));
        let a = big.area;
        draw(&mut big, a);
        let t = text(&big);
        assert!(t.contains(TITLE));
        let width = |t: &str| t.lines().map(|l| l.trim().chars().count()).max().unwrap();
        assert_eq!(width(&t), 48);
        // The head mirror's white is a pixel: top half, bottom half or both.
        assert!(
            big.content()
                .iter()
                .any(|c| c.fg == Color::Rgb(240, 240, 240))
        );
        // Too small for double size: single.
        let mut small = Screen::empty(Rect::new(0, 0, 40, 20));
        let a = small.area;
        draw(&mut small, a);
        assert_eq!(width(&text(&small)), 24);
        // Too small for anything: nothing.
        let mut tiny = Screen::empty(Rect::new(0, 0, 20, 10));
        let a = tiny.area;
        draw(&mut tiny, a);
        assert!(text(&tiny).trim().is_empty());
    }
}
