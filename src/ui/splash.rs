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
    if area.is_empty() {
        return;
    }
    // The dashboard owns its area, including cells previously used by the editor.
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buf[(x, y)].reset();
        }
    }
    let recent = recent(ed);
    let columns = actions
        .len()
        .div_ceil(area.height.saturating_sub(1).max(1) as usize)
        .min((area.width as usize / 19).max(1));
    let action_rows = actions.len().div_ceil(columns) as u16;
    let spare = area.height.saturating_sub(action_rows);
    let compact_logo = LOGO.lines().count().div_ceil(4) as u16 + 2;
    let minimum_logo = if area.width >= 12 && spare >= compact_logo {
        compact_logo
    } else {
        spare.min(1)
    };
    let recent_rows = recent
        .len()
        .max(1)
        .min(spare.saturating_sub(minimum_logo + 2) as usize);
    let recent_height = if recent_rows > 0 {
        recent_rows as u16 + 2
    } else {
        0
    };
    let logo_height = spare - recent_height;
    draw(buf, Rect::new(area.x, area.y, area.width, logo_height));
    let width = area.width.min(44 * columns as u16);
    let x = area.x + (area.width - width) / 2;
    let column_width = width / columns as u16;
    let label_style = Style::default().fg(Color::Indexed(250));
    let key_style = Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::BOLD);
    for (i, &(icon, fallback, label, key)) in actions.iter().enumerate() {
        let y = area.y + logo_height + (i / columns) as u16;
        if y >= area.bottom() {
            break;
        }
        let cx = x + (i % columns) as u16 * column_width;
        // Drop icons before sacrificing labels on narrow terminals.
        let inset = if column_width >= 19 { 3 } else { 0 };
        if inset > 0 {
            let icon = if icons { icon } else { fallback };
            buf.set_stringn(cx, y, icon.to_string(), 2, label_style);
        }
        buf.set_stringn(
            cx + inset,
            y,
            label,
            column_width.saturating_sub(inset + 2) as usize,
            label_style,
        );
        buf.set_stringn(cx + column_width - 1, y, key.to_string(), 1, key_style);
    }
    let heading_y = area.y + logo_height + action_rows + 1;
    if recent_rows > 0 {
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
        if area.height > 0 {
            let width = area.width.min(TITLE.len() as u16);
            buf.set_stringn(
                area.x + (area.width - width) / 2,
                area.y,
                TITLE,
                width as usize,
                Style::default()
                    .fg(Color::Rgb(140, 235, 235))
                    .add_modifier(Modifier::BOLD),
            );
        }
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
    fn dashboard_stays_inside_small_offset_areas() {
        let ed = crate::editor::Editor::new(crate::buffer::Buffer::from_text(""));
        for width in 0..50 {
            for height in 0..25 {
                let mut buf = Screen::empty(Rect::new(0, 0, 55, 30));
                buf.set_string(0, 0, "outside", Style::default());
                dashboard(&mut buf, Rect::new(3, 2, width, height), &ed, true);
                assert_eq!(buf[(0, 0)].symbol(), "o");
                for y in 0..30 {
                    for x in 0..55 {
                        if (x >= 3 + width || y >= 2 + height) && (x > 6 || y > 0) {
                            assert_eq!(
                                buf[(x, y)].symbol(),
                                " ",
                                "{width}x{height} wrote outside at {x},{y}"
                            );
                        }
                    }
                }
            }
        }
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
        // Too small for artwork: keep the title.
        let mut tiny = Screen::empty(Rect::new(0, 0, 10, 8));
        let a = tiny.area;
        draw(&mut tiny, a);
        assert!(text(&tiny).contains(TITLE));
    }
}
