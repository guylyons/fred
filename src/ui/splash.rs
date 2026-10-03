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

pub fn dashboard(buf: &mut Screen, area: Rect, ed: &crate::editor::Editor, icons: bool) {
    let actions = [
        ('\u{f0214}', '+', "New file", 'e'),
        ('\u{f0219}', '?', "Find file", 'f'),
        ('\u{f025a}', '*', "Recent files", 'r'),
        ('\u{f021a}', '/', "Live grep", 'g'),
        ('\u{f06a9}', '@', "Claude", 'a'),
        ('\u{f0493}', '#', "Config", 'c'),
        ('\u{f0343}', 'x', "Quit", 'q'),
    ];
    let recent = recent(ed);
    let compact_logo = LOGO.lines().count().div_ceil(4) as u16 + 2;
    let recent_rows = recent.len().max(1).min(
        area.height
            .saturating_sub(compact_logo + actions.len() as u16 + 2)
            .max(1) as usize,
    );
    let height = (actions.len() as u16 + 2 + recent_rows as u16).min(area.height);
    let logo_height = area.height - height;
    draw(buf, Rect::new(area.x, area.y, area.width, logo_height));
    let width = area.width.min(44);
    let x = area.x + (area.width - width) / 2;
    let label_style = Style::default().fg(Color::Indexed(250));
    let key_style = Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::BOLD);
    for (i, &(icon, fallback, label, key)) in actions.iter().enumerate() {
        let y = area.y + logo_height + i as u16;
        if y >= area.bottom() || width < 5 {
            break;
        }
        let icon = if icons { icon } else { fallback };
        buf.set_stringn(x, y, icon.to_string(), 2, label_style);
        buf.set_stringn(x + 3, y, label, (width - 5) as usize, label_style);
        buf.set_stringn(x + width - 1, y, key.to_string(), 1, key_style);
    }
    let heading_y = area.y + logo_height + actions.len() as u16 + 1;
    if heading_y < area.bottom() {
        buf.set_stringn(
            x,
            heading_y,
            "Recent files",
            width as usize,
            label_style.add_modifier(Modifier::BOLD),
        );
    }
    for i in 0..recent_rows {
        let y = heading_y + 1 + i as u16;
        if y >= area.bottom() {
            break;
        }
        let line = recent.get(i).map_or_else(
            || "No recent files".to_string(),
            |path| {
                format!(
                    "[{}] {}",
                    i + 1,
                    super::render::status_tail(
                        &crate::pick::browse::tilde(path),
                        width.saturating_sub(4) as usize
                    )
                )
            },
        );
        buf.set_stringn(x, y, line, width as usize, label_style);
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
/// when there's room, half size when the menu needs more space.
pub fn draw(buf: &mut Screen, area: Rect) {
    let rows: Vec<&[u8]> = LOGO.lines().map(str::as_bytes).collect();
    let (w, h) = (rows[0].len(), rows.len());
    // Pixel dimensions, plus the original gap and title below.
    let Some((cw, ph)) = [(w * 2, h * 2), (w, h), (w / 2, h.div_ceil(2))]
        .into_iter()
        .find(|&(cw, ph)| cw <= area.width as usize && ph.div_ceil(2) + 2 <= area.height as usize)
    else {
        return;
    };
    let ch = ph.div_ceil(2) + 2;
    let x0 = area.x + (area.width - cw as u16) / 2;
    let y0 = area.y + (area.height - ch as u16) / 2;
    let px = |x: usize, y: usize| {
        if y >= ph {
            return None;
        }
        rows.get(y * h / ph)
            .and_then(|r| r.get(x * w / cw))
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
        let mut tiny = Screen::empty(Rect::new(0, 0, 10, 8));
        let a = tiny.area;
        draw(&mut tiny, a);
        assert!(text(&tiny).trim().is_empty());
    }
}
