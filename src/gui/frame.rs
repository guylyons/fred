//! One redraw as a `Frame`: the ratatui renderer's output, sliced per window.
use super::proto::{self, Cursor, Frame, Run, Status, Win};
use crate::app;
use crate::config::Config;
use crate::editor::Mode;
use crate::session::Session;
use crate::ui::{render, windows};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

/// Render `s` into `term` (sized by the caller) and slice it into a frame.
pub fn build(s: &mut Session, cfg: &Config, term: &mut Terminal<TestBackend>, seq: u64) -> Frame {
    term.draw(|f| windows::draw_session(f, s, cfg, app::HIGHLIGHT_BUDGET))
        .expect("in-memory terminal");
    app::drawn(s);
    let mut status = vec![];
    let cwd = std::env::current_dir().ok();
    s.each_window(|ed, _, _, p, _| {
        let name = render::term_title(ed).unwrap_or_else(|| match &ed.path {
            Some(path) => cwd
                .as_deref()
                .and_then(|c| path.strip_prefix(c).ok())
                .unwrap_or(path)
                .display()
                .to_string(),
            None => "[No Name]".into(),
        });
        let line = ed.buf.line(ed.cur.line);
        let col = crate::text::col_of_byte(&line, ed.cur.byte, ed.tabstop()) + 1;
        status.push((
            p.id,
            Status {
                mode: render::status_label(ed),
                name,
                modified: ed.buf.modified,
                pos: format!("{}:{col}", ed.cur.line + 1),
                branch: ed.git.branch().map(|b| b.to_string()),
            },
        ));
    });
    let be = term.backend();
    let buf = be.buffer();
    let windows = s
        .tree
        .layout(s.area)
        .into_iter()
        .filter_map(|p| {
            let i = status.iter().position(|(id, _)| *id == p.id)?;
            let r = p.rect;
            Some(Win {
                x: r.x,
                y: r.y,
                w: r.width,
                h: r.height,
                divider: p.divider,
                active: p.id == s.active,
                rows: (r.y..r.bottom().saturating_sub(1))
                    // Not the divider column: the app draws its own line.
                    .map(|y| runs(buf, y, r.x, r.right() - u16::from(p.divider)))
                    .collect(),
                status: status.swap_remove(i).1,
            })
        })
        .collect();
    let bottom = (s.area.bottom()..buf.area.bottom())
        .map(|y| runs(buf, y, 0, buf.area.right()))
        .collect();
    let cursor = be.cursor_visible().then(|| {
        let p = be.cursor_position();
        let bar = s.ed.mode == Mode::Insert && s.ed.term.is_none();
        // ponytail: no "underline" (Replace) shape; Fred has no Replace mode.
        Cursor {
            x: p.x,
            y: p.y,
            shape: if bar { "bar" } else { "block" }.into(),
        }
    });
    Frame {
        t: "frame".into(),
        seq,
        cols: buf.area.width,
        rows: buf.area.height,
        title: format!(
            "fred — {}",
            s.ed.path
                .as_ref()
                .map_or("[No Name]".into(), |p| p.display().to_string())
        ),
        windows,
        bottom,
        cursor,
    }
}

const FLAGS: [Modifier; 6] = [
    Modifier::BOLD,
    Modifier::ITALIC,
    Modifier::UNDERLINED,
    Modifier::REVERSED,
    Modifier::DIM,
    Modifier::CROSSED_OUT,
];

/// Row `y`, columns `from..to`, as runs (x relative to `from`).
fn runs(buf: &Buffer, y: u16, from: u16, to: u16) -> Vec<Run> {
    let mut out: Vec<Run> = vec![];
    let mut covered = 0;
    let mut end = 0; // columns up to the last cell that isn't a default blank
    let mut last_style = None;
    for x in from..to {
        let cell = &buf[(x, y)];
        if covered > 0 {
            covered -= 1;
            continue;
        }
        covered = cell.symbol().width().max(1) - 1;
        let fg = proto::color(cell.fg);
        let bg = proto::color(cell.bg);
        let f = FLAGS
            .iter()
            .enumerate()
            .filter(|(_, m)| cell.modifier.contains(**m))
            .fold(0, |a, (i, _)| a | 1 << i);
        let style = (fg, bg, f);
        let sym = if cell.symbol().is_empty() {
            " "
        } else {
            cell.symbol()
        };
        if last_style == Some(style) {
            out.last_mut().unwrap().s.push_str(sym);
        } else {
            out.push(Run {
                x: x - from,
                s: sym.into(),
                fg,
                bg,
                f,
            });
            last_style = Some(style);
        }
        if style != (None, None, 0) || sym != " " {
            end = out.len();
        }
    }
    // Trailing default-styled blanks: drop whole runs, trim the last.
    out.truncate(end);
    if let Some(r) = out.last_mut()
        && r.fg.is_none()
        && r.bg.is_none()
        && r.f == 0
    {
        r.s.truncate(r.s.trim_end().len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(w: u16, h: u16, text: &str) -> (Session, Terminal<TestBackend>, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("f.txt");
        std::fs::write(&path, text).unwrap();
        let (mut s, _) =
            Session::open(Some(path), &Config::default(), &d.path().join("swap")).unwrap();
        s.handle_keys(":set nonumber<Enter>");
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| windows::draw_session(f, &mut s, &Config::default(), app::HIGHLIGHT_BUDGET))
            .unwrap();
        (s, term, d)
    }

    #[test]
    fn a_window_row_is_styled_runs() {
        let (mut s, mut term, _d) = setup(20, 6, "hello\n");
        let f = build(&mut s, &Config::default(), &mut term, 3);
        assert_eq!(f.seq, 3);
        assert_eq!((f.cols, f.rows), (20, 6));
        assert_eq!(f.windows.len(), 1);
        let w = &f.windows[0];
        assert!(w.active);
        assert_eq!(w.rows.len(), w.h as usize - 1);
        let text: String = w.rows[0].iter().map(|r| r.s.as_str()).collect();
        assert_eq!(text, "hello");
        assert_eq!(w.status.mode, "NORMAL");
        assert_eq!(w.status.pos, "1:1");
        assert_eq!(
            f.cursor.as_ref().map(|c| (c.x, c.y, c.shape.as_str())),
            Some((0, 0, "block"))
        );
    }

    #[test]
    fn wide_chars_and_tabs_keep_columns() {
        let (mut s, mut term, _d) = setup(30, 6, "中x\ty\n");
        let f = build(&mut s, &Config::default(), &mut term, 0);
        let row = &f.windows[0].rows[0];
        let at = |col: u16| {
            row.iter()
                .find(|r| r.x <= col && col < r.x + UnicodeWidthStr::width(r.s.as_str()) as u16)
                .map(|r| r.s.clone())
        };
        assert!(at(0).unwrap().starts_with('中'));
        assert!(at(8).unwrap().contains('y'));
    }

    #[test]
    fn insert_mode_is_a_bar_and_command_line_is_in_bottom() {
        let (mut s, mut term, _d) = setup(20, 6, "hello\n");
        s.handle_keys("i");
        let f = build(&mut s, &Config::default(), &mut term, 0);
        assert_eq!(f.cursor.unwrap().shape, "bar");
        assert_eq!(f.windows[0].status.mode, "INSERT");
        s.handle_keys("<Esc>:w");
        let f = build(&mut s, &Config::default(), &mut term, 0);
        let bottom: String = f.bottom.concat().iter().map(|r| r.s.as_str()).collect();
        assert!(bottom.starts_with(":w"), "{bottom:?}");
    }

    #[test]
    fn split_windows_have_their_own_rects_and_status() {
        let (mut s, mut term, _d) = setup(40, 12, "hello\n");
        s.handle_keys("<C-w>v");
        let f = build(&mut s, &Config::default(), &mut term, 0);
        assert_eq!(f.windows.len(), 2);
        assert_eq!(f.windows.iter().filter(|w| w.active).count(), 1);
        assert!(f.windows.iter().any(|w| w.divider));
        // The app draws the divider natively; no box-drawing char in rows.
        for w in &f.windows {
            for row in &w.rows {
                assert!(row.iter().all(|r| !r.s.contains('│')), "{row:?}");
            }
        }
    }

    #[test]
    fn colors_encode_as_palette_or_rgb() {
        use ratatui::style::Color;
        assert_eq!(proto::color(Color::Reset), None);
        assert_eq!(proto::color(Color::DarkGray), Some(8));
        assert_eq!(proto::color(Color::Indexed(244)), Some(244));
        assert_eq!(proto::color(Color::Rgb(0xff, 0x66, 0xb2)), Some(0x1ff66b2));
    }

    #[test]
    fn frame_json_omits_defaults() {
        let run = proto::Run {
            x: 0,
            s: "a".into(),
            fg: None,
            bg: None,
            f: 0,
        };
        assert_eq!(serde_json::to_string(&run).unwrap(), r#"{"x":0,"s":"a"}"#);
    }
}
