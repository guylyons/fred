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
    s.each_window(|ed, view, _, p, _| {
        let name = render::term_title(ed).unwrap_or_else(|| match &ed.path {
            Some(path) => cwd
                .as_deref()
                .and_then(|c| path.strip_prefix(c).ok())
                .unwrap_or(path)
                .display()
                .to_string(),
            None => "[No Name]".into(),
        });
        let file = file_of(ed);
        let line = ed.buf.line(ed.cur.line);
        let col = crate::text::col_of_byte(&line, ed.cur.byte, ed.tabstop()) + 1;
        status.push((
            p.id,
            file,
            (view.top, ed.line_count()),
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
        .placed()
        .into_iter()
        .filter_map(|p| {
            let i = status.iter().position(|(id, ..)| *id == p.id)?;
            let (r, text) = (p.rect, p.text());
            let sel: Vec<proto::Sel> = if p.id == s.active {
                s.ed.drawn_sel
                    .borrow()
                    .iter()
                    .map(|&(y, x, w)| proto::Sel { y, x, w })
                    .collect()
            } else {
                vec![]
            };
            let row_sel = |y: u16| {
                sel.iter()
                    .find(|p| p.y == y - text.y)
                    .map(|p| r.x + p.x..r.x + p.x + p.w)
            };
            Some(Win {
                x: r.x,
                y: r.y,
                w: r.width,
                h: r.height,
                divider: p.divider,
                active: p.id == s.active,
                // Not the tab row, the minimap or the divider column:
                // the app draws its own.
                rows: (text.y..text.bottom())
                    .map(|y| runs(buf, y, r.x, text.right(), row_sel(y)))
                    .collect(),
                sel,
                path: status[i].1.take(),
                id: p.id,
                header: p.header,
                margin: p.margin,
                top: status[i].2.0,
                lines: status[i].2.1,
                tabs: tabs(s, p.id),
                status: status.swap_remove(i).3,
            })
        })
        .collect();
    let bottom = (s.area.bottom()..buf.area.bottom())
        .map(|y| runs(buf, y, 0, buf.area.right(), None))
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

/// A file's absolute path; none for no-name and special buffers.
fn file_of(ed: &crate::editor::Editor) -> Option<String> {
    let special =
        ed.dired.is_some() || ed.occur.is_some() || ed.term.is_some() || ed.magit.is_some();
    (!special)
        .then(|| ed.path.as_deref().and_then(|p| std::path::absolute(p).ok()))
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Window `id`'s tabs, in order.
fn tabs(s: &Session, id: usize) -> Vec<proto::Tab> {
    let Some(w) = s.windows.iter().find(|w| w.id == id) else {
        return vec![];
    };
    w.tabs
        .iter()
        .map(|&b| {
            let ed = s.ed_at(b);
            let name = render::term_title(ed)
                .or_else(|| {
                    ed.path
                        .as_ref()?
                        .file_name()
                        .map(|n| n.to_string_lossy().into())
                })
                .unwrap_or_else(|| "[No Name]".into());
            proto::Tab {
                name,
                path: file_of(ed),
                modified: ed.buf.modified,
                preview: w.preview == Some(b),
                current: w.buf == b,
            }
        })
        .collect()
}

const FLAGS: [Modifier; 6] = [
    Modifier::BOLD,
    Modifier::ITALIC,
    Modifier::UNDERLINED,
    Modifier::REVERSED,
    Modifier::DIM,
    Modifier::CROSSED_OUT,
];

/// Row `y`, columns `from..to`, as runs (x relative to `from`). Cells in
/// `sel` are the selection, which the app draws: not reversed.
fn runs(buf: &Buffer, y: u16, from: u16, to: u16, sel: Option<std::ops::Range<u16>>) -> Vec<Run> {
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
        let mut modifier = cell.modifier;
        if sel.as_ref().is_some_and(|r| r.contains(&x)) {
            modifier.remove(Modifier::REVERSED);
        }
        let f = FLAGS
            .iter()
            .enumerate()
            .filter(|(_, m)| modifier.contains(**m))
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
    fn a_selection_is_sel_spans_not_reversed_cells() {
        let (mut s, mut term, _d) = setup(30, 8, "abc\n\ndef ghi\n");
        let sel = |s: &mut Session, term: &mut Terminal<TestBackend>| {
            let f = build(s, &Config::default(), term, 0);
            let w = &f.windows[0];
            for row in &w.rows {
                assert!(row.iter().all(|r| r.f & 8 == 0), "reversed: {row:?}");
            }
            w.sel.iter().map(|p| (p.y, p.x, p.w)).collect::<Vec<_>>()
        };
        assert_eq!(sel(&mut s, &mut term), vec![]);
        // Charwise: a full line runs one cell past its text (the line
        // break); an empty one is one cell; the last stops at the cursor.
        s.handle_keys("lvjj");
        assert_eq!(
            sel(&mut s, &mut term),
            vec![(0, 1, 3), (1, 0, 1), (2, 0, 2)]
        );
        s.handle_keys("<Esc>ggVj");
        assert_eq!(sel(&mut s, &mut term), vec![(0, 0, 4), (1, 0, 1)]);
        s.handle_keys("<Esc>Ggh<End>");
        assert_eq!(sel(&mut s, &mut term), vec![(2, 0, 7)]);
    }

    #[test]
    fn sel_spans_cover_wide_chars_and_a_lone_line_break() {
        let (mut s, mut term, _d) = setup(30, 8, "a日b\nabc\ndef\n");
        let sel = |s: &mut Session, term: &mut Terminal<TestBackend>| {
            let f = build(s, &Config::default(), term, 0);
            f.windows[0]
                .sel
                .iter()
                .map(|p| (p.y, p.x, p.w))
                .collect::<Vec<_>>()
        };
        // Ending on a wide character takes both its cells.
        s.handle_keys("gh<Right>");
        assert_eq!(sel(&mut s, &mut term), vec![(0, 0, 3)]);
        // Only line 2's line break is selected: it still gets its cell.
        s.handle_keys("<Esc>:set km=startsel slm=key<Enter>jA<S-Down>");
        assert_eq!(sel(&mut s, &mut term), vec![(1, 3, 1), (2, 0, 3)]);
    }

    #[test]
    fn a_file_window_has_its_absolute_path_and_no_name_has_none() {
        let (mut s, mut term, d) = setup(20, 6, "hello\n");
        let f = build(&mut s, &Config::default(), &mut term, 0);
        let want = std::path::absolute(d.path().join("f.txt")).unwrap();
        assert_eq!(f.windows[0].path.as_deref(), Some(want.to_str().unwrap()));
        let (mut s, _) = Session::open(None, &Config::default(), &d.path().join("swap2")).unwrap();
        let f = build(&mut s, &Config::default(), &mut term, 0);
        assert_eq!(f.windows[0].path, None);
        assert!(
            !serde_json::to_string(&f.windows[0])
                .unwrap()
                .contains("\"path\"")
        );
    }

    #[test]
    fn gui_windows_reserve_a_tab_row_and_a_minimap_margin() {
        let long = format!("{}\n", "x".repeat(60));
        let (mut s, mut term, _d) = setup(40, 12, &format!("hello\n{long}"));
        s.handle_keys(":set nowrap<Enter>j");
        s.chrome = (1, 8);
        let f = build(&mut s, &Config::default(), &mut term, 0);
        let w = &f.windows[0];
        assert_eq!((w.header, w.margin), (1, 8));
        assert_eq!(w.rows.len(), w.h as usize - 2);
        let text: String = w.rows[0].iter().map(|r| r.s.as_str()).collect();
        assert_eq!(text, "hello");
        for row in &w.rows {
            for r in row {
                let end = r.x as usize + UnicodeWidthStr::width(r.s.as_str());
                assert!(end <= w.w as usize - 8, "{r:?}");
            }
        }
        // The cursor (line 2) is drawn under the tab row.
        assert_eq!(f.cursor.map(|c| c.y), Some(2));
    }

    #[test]
    fn the_terminal_layout_is_unchanged() {
        let (mut s, mut term, _d) = setup(40, 12, "hello\n");
        let f = build(&mut s, &Config::default(), &mut term, 0);
        let w = &f.windows[0];
        assert_eq!((w.header, w.margin), (0, 0));
        assert_eq!(w.rows.len(), w.h as usize - 1);
        assert_eq!(f.cursor.map(|c| c.y), Some(0));
    }

    #[test]
    fn tabs_and_top_and_lines_are_in_the_frame() {
        let text: String = (1..=30).map(|n| format!("{n}\n")).collect();
        let (mut s, mut term, d) = setup(40, 12, &text);
        std::fs::write(d.path().join("g.txt"), "g\n").unwrap();
        std::fs::write(d.path().join("h.txt"), "h\n").unwrap();
        s.handle_keys(&format!(":e {}<Enter>", d.path().join("g.txt").display()));
        s.open_preview(&d.path().join("h.txt"), true);
        s.handle_keys(":b1<Enter>G");
        s.chrome = (1, 0);
        let f = build(&mut s, &Config::default(), &mut term, 0);
        let w = &f.windows[0];
        assert_eq!(w.id, s.active);
        let tabs: Vec<_> = w
            .tabs
            .iter()
            .map(|t| (t.name.as_str(), t.preview, t.current))
            .collect();
        assert_eq!(
            tabs,
            [
                ("f.txt", false, true),
                ("g.txt", false, false),
                ("h.txt", true, false)
            ]
        );
        let want = std::path::absolute(d.path().join("g.txt")).unwrap();
        assert_eq!(w.tabs[1].path.as_deref(), Some(want.to_str().unwrap()));
        assert!(!w.tabs[0].modified);
        assert_eq!(w.lines, 30);
        // 12 rows: the command row, the tab row and the status row leave 9.
        assert_eq!(w.top, 30 - 9);
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
