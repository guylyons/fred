//! `:terminal`, `Space t`: your shell in Ghostty's own terminal emulator
//! (libghostty-vt), as a buffer any window can show. In terminal mode
//! (Insert, in that buffer) keys go to the shell; `Ctrl-\ Ctrl-n` or `jj`
//! is Normal mode again, and the shell keeps running.
//!
//! Built with `--features ghostty` (it needs Zig 0.15 to build Ghostty).

#[cfg(feature = "ghostty")]
pub use imp::Ghost;

/// Without the `ghostty` feature there's never a terminal.
#[cfg(not(feature = "ghostty"))]
pub enum Ghost {}

#[cfg(not(feature = "ghostty"))]
impl Ghost {
    pub fn spawn(_: u16, _: u16) -> Result<Ghost, String> {
        Err("fred was built without Ghostty: cargo install --path . --features ghostty (needs Zig 0.15)".into())
    }
    pub fn pump(&mut self) -> Option<bool> {
        match *self {}
    }
    pub fn input(&mut self, _: ratatui::crossterm::event::KeyEvent) -> bool {
        match *self {}
    }
    pub fn flush(&mut self) -> bool {
        match *self {}
    }
    pub fn paste(&mut self, _: &str) {
        match *self {}
    }
    pub fn scroll(&mut self, _: isize) {
        match *self {}
    }
    pub fn title(&self) -> String {
        match *self {}
    }
    pub fn draw(
        &mut self,
        _: &mut ratatui::buffer::Buffer,
        _: ratatui::layout::Rect,
    ) -> Option<(u16, u16, ratatui::crossterm::cursor::SetCursorStyle)> {
        match *self {}
    }
}

#[cfg(feature = "ghostty")]
mod imp {
    use libghostty_vt::key::{self, Action, Key, Mods, OptionAsAlt};
    use libghostty_vt::render::{CellIterator, CursorVisualStyle, RenderState, RowIterator};
    use libghostty_vt::style::{StyleColor, Underline};
    use libghostty_vt::terminal::{
        ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode,
        PrimaryDeviceAttributes, ScrollViewport, SecondaryDeviceAttributes, SizeReportSize,
        TertiaryDeviceAttributes,
    };
    use libghostty_vt::{Terminal, TerminalOptions};
    use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::cursor::SetCursorStyle;
    use ratatui::crossterm::event::{KeyCode as C, KeyEvent, KeyEventKind, KeyModifiers as M};
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use std::cell::{Cell, RefCell};
    use std::io::{Read, Write};
    use std::rc::Rc;
    use std::sync::mpsc::{self, Receiver, TryRecvError};
    use std::time::{Duration, Instant};

    /// How long a lone `j` waits for a second one before it's typed.
    const JJ: Duration = Duration::from_millis(300);

    type Pty = Rc<RefCell<Box<dyn Write + Send>>>;

    pub struct Ghost {
        vt: Terminal<'static, 'static>,
        render: RenderState<'static>,
        rows: RowIterator<'static>,
        cells: CellIterator<'static>,
        keys: key::Encoder<'static>,
        event: key::Event<'static>,
        pty: Pty,
        master: Box<dyn MasterPty + Send>,
        child: Box<dyn Child + Send + Sync>,
        out: Receiver<Vec<u8>>,
        size: Rc<Cell<(u16, u16)>>,
        /// `Ctrl-\` came: `Ctrl-n` next leaves.
        leader: bool,
        /// A `j` held back: a second one leaves.
        held: Option<(KeyEvent, Instant)>,
    }

    fn plain(k: &KeyEvent, c: char) -> bool {
        k.code == C::Char(c) && (k.modifiers - M::SHIFT).is_empty()
    }

    fn err(e: impl std::fmt::Display) -> String {
        format!("terminal: {e}")
    }

    impl Ghost {
        /// Your login shell in a `cols`×`rows` terminal.
        pub fn spawn(cols: u16, rows: u16) -> Result<Ghost, String> {
            let (cols, rows) = (cols.max(2), rows.max(2));
            let pair = native_pty_system()
                .openpty(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(err)?;
            let mut cmd = CommandBuilder::new_default_prog();
            cmd.env("TERM", "xterm-256color");
            cmd.env("COLORTERM", "truecolor");
            if let Ok(dir) = std::env::current_dir() {
                cmd.cwd(dir);
            }
            let child = pair.slave.spawn_command(cmd).map_err(err)?;
            // Only the shell holds the slave now: its exit ends our reads.
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().map_err(err)?;
            let pty: Pty = Rc::new(RefCell::new(pair.master.take_writer().map_err(err)?));
            let (tx, out) = mpsc::channel();
            std::thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                while let Ok(n @ 1..) = reader.read(&mut buf) {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            });
            let mut vt = Terminal::new(TerminalOptions {
                cols,
                rows,
                max_scrollback: 10_000,
            })
            .map_err(err)?;
            let size = Rc::new(Cell::new((cols, rows)));
            let (w, sz) = (pty.clone(), size.clone());
            vt.on_pty_write(move |_, data| {
                let _ = w.borrow_mut().write_all(data);
            })
            .map_err(err)?
            // Shells and editors ask what the terminal is, and some wait
            // for the answer.
            .on_device_attributes(|_| {
                Some(DeviceAttributes {
                    primary: PrimaryDeviceAttributes::new(
                        ConformanceLevel::VT220,
                        &[DeviceAttributeFeature::ANSI_COLOR],
                    ),
                    secondary: SecondaryDeviceAttributes {
                        device_type: DeviceType::VT220,
                        firmware_version: 1,
                        rom_cartridge: 0,
                    },
                    tertiary: TertiaryDeviceAttributes { unit_id: 0 },
                })
            })
            .map_err(err)?
            .on_size(move |_| {
                let (columns, rows) = sz.get();
                Some(SizeReportSize {
                    rows,
                    columns,
                    cell_width: 0,
                    cell_height: 0,
                })
            })
            .map_err(err)?
            .on_xtversion(|_| Some("fred (libghostty-vt)"))
            .map_err(err)?;
            Ok(Ghost {
                vt,
                render: RenderState::new().map_err(err)?,
                rows: RowIterator::new().map_err(err)?,
                cells: CellIterator::new().map_err(err)?,
                keys: key::Encoder::new().map_err(err)?,
                event: key::Event::new().map_err(err)?,
                pty,
                master: pair.master,
                child,
                out,
                size,
                leader: false,
                held: None,
            })
        }

        /// The shell's output so far, into the terminal: whether any came,
        /// or None once the shell has exited.
        pub fn pump(&mut self) -> Option<bool> {
            let mut any = false;
            loop {
                match self.out.try_recv() {
                    Ok(bytes) => {
                        self.vt.vt_write(&bytes);
                        any = true;
                    }
                    Err(TryRecvError::Empty) => return Some(any),
                    Err(TryRecvError::Disconnected) => {
                        let _ = self.child.try_wait();
                        return None;
                    }
                }
            }
        }

        fn send(&self, bytes: &[u8]) {
            let mut w = self.pty.borrow_mut();
            let _ = w.write_all(bytes).and_then(|()| w.flush());
        }

        fn resize(&mut self, cols: u16, rows: u16) {
            let (cols, rows) = (cols.max(2), rows.max(2));
            if self.size.get() == (cols, rows) {
                return;
            }
            self.size.set((cols, rows));
            let _ = self.vt.resize(cols, rows, 0, 0);
            let _ = self.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }

        /// A key typed in terminal mode: true when it leaves terminal mode
        /// (`Ctrl-\ Ctrl-n`, `jj`) rather than going to the shell.
        pub fn input(&mut self, k: KeyEvent) -> bool {
            if k.kind == KeyEventKind::Release {
                return false;
            }
            if let Some((j, _)) = self.held.take() {
                if plain(&k, 'j') {
                    return true;
                }
                self.key(j);
            }
            let ctrl = k.modifiers.contains(M::CONTROL);
            if std::mem::take(&mut self.leader) {
                if k.code == C::Char('n') {
                    return true;
                }
                self.key(KeyEvent::new(C::Char('\\'), M::CONTROL));
            } else if ctrl && matches!(k.code, C::Char('\\' | '4')) {
                // Ctrl-\ is Ctrl-4 to a terminal without the kitty protocol.
                self.leader = true;
                return false;
            } else if plain(&k, 'j') {
                self.held = Some((k, Instant::now()));
                return false;
            }
            self.key(k);
            false
        }

        /// A held `j` that waited long enough is typed: whether it was.
        pub fn flush(&mut self) -> bool {
            match self.held {
                Some((j, at)) if at.elapsed() >= JJ => {
                    self.held = None;
                    self.key(j);
                    true
                }
                _ => false,
            }
        }

        /// A key, as Ghostty would send it: the encoder follows the
        /// program's modes (application cursor keys, kitty keyboard...).
        fn key(&mut self, k: KeyEvent) {
            let action = match k.kind {
                KeyEventKind::Release => return,
                KeyEventKind::Repeat => Action::Repeat,
                KeyEventKind::Press => Action::Press,
            };
            let m = k.modifiers;
            let mut mods = Mods::empty();
            for (from, to) in [
                (M::SHIFT, Mods::SHIFT),
                (M::ALT, Mods::ALT),
                (M::CONTROL, Mods::CTRL),
                (M::SUPER, Mods::SUPER),
            ] {
                if m.contains(from) {
                    mods |= to;
                }
            }
            let (key, text, unshifted) = match k.code {
                C::Char(c) => (
                    char_key(c),
                    (!m.contains(M::CONTROL)).then(|| c.to_string()),
                    c.to_ascii_lowercase(),
                ),
                C::BackTab => {
                    mods |= Mods::SHIFT;
                    (Key::Tab, None, '\0')
                }
                C::F(n @ 1..=12) => {
                    let f = Key::try_from(Key::F1 as u32 + u32::from(n) - 1);
                    (f.unwrap_or(Key::Unidentified), None, '\0')
                }
                code => {
                    let key = match code {
                        C::Enter => Key::Enter,
                        C::Tab => Key::Tab,
                        C::Backspace => Key::Backspace,
                        C::Esc => Key::Escape,
                        C::Left => Key::ArrowLeft,
                        C::Right => Key::ArrowRight,
                        C::Up => Key::ArrowUp,
                        C::Down => Key::ArrowDown,
                        C::Home => Key::Home,
                        C::End => Key::End,
                        C::PageUp => Key::PageUp,
                        C::PageDown => Key::PageDown,
                        C::Delete => Key::Delete,
                        C::Insert => Key::Insert,
                        _ => return,
                    };
                    (key, None, '\0')
                }
            };
            let consumed = if text.is_some() {
                mods & Mods::SHIFT
            } else {
                Mods::empty()
            };
            self.event
                .set_action(action)
                .set_key(key)
                .set_mods(mods)
                .set_consumed_mods(consumed)
                .set_unshifted_codepoint(unshifted)
                .set_utf8(text.clone());
            let mut out = vec![];
            let _ = self
                .keys
                .set_options_from_terminal(&self.vt)
                // The host terminal already decided Option is Alt.
                .set_macos_option_as_alt(OptionAsAlt::True)
                .encode_to_vec(&self.event, &mut out);
            if out.is_empty()
                && let Some(t) = text
            {
                out = t.into_bytes();
            }
            if !out.is_empty() {
                self.vt.scroll_viewport(ScrollViewport::Bottom);
                self.send(&out);
            }
        }

        /// Pasted text: bracketed if the program asked for it.
        pub fn paste(&mut self, text: &str) {
            let bracketed = self.vt.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
            let mut data = text.as_bytes().to_vec();
            let mut out = vec![0; data.len() + 16];
            if let Ok(n) = libghostty_vt::paste::encode(&mut data, bracketed, &mut out) {
                self.vt.scroll_viewport(ScrollViewport::Bottom);
                self.send(&out[..n]);
            }
        }

        /// The wheel: through the scrollback (up is negative).
        // ponytail: programs that track the mouse don't get it; add a
        // mouse::Encoder when one needs clicks.
        pub fn scroll(&mut self, lines: isize) {
            self.vt.scroll_viewport(ScrollViewport::Delta(lines));
        }

        /// The window title the shell set, if any.
        pub fn title(&self) -> String {
            self.vt.title().unwrap_or_default().to_string()
        }

        /// The screen into `area` (the terminal takes its size); where the
        /// cursor is, and its shape.
        pub fn draw(&mut self, buf: &mut Buffer, area: Rect) -> Option<(u16, u16, SetCursorStyle)> {
            self.resize(area.width, area.height);
            let snap = self.render.update(&self.vt).ok()?;
            let mut rows = self.rows.update(&snap).ok()?;
            let mut text = String::new();
            let mut y = 0;
            while let Some(row) = rows.next() {
                if y >= area.height {
                    break;
                }
                let Ok(mut cells) = self.cells.update(row) else {
                    break;
                };
                let mut x = 0;
                while let Some(cell) = cells.next() {
                    if x >= area.width {
                        break;
                    }
                    let mut style = Style::default();
                    if cell.has_styling().unwrap_or(false)
                        && let Ok(s) = cell.style()
                    {
                        style.fg = color(s.fg_color);
                        style.bg = color(s.bg_color);
                        for (on, m) in [
                            (s.bold, Modifier::BOLD),
                            (s.italic, Modifier::ITALIC),
                            (s.faint, Modifier::DIM),
                            (s.blink, Modifier::SLOW_BLINK),
                            (s.inverse, Modifier::REVERSED),
                            (s.invisible, Modifier::HIDDEN),
                            (s.strikethrough, Modifier::CROSSED_OUT),
                            (s.underline != Underline::None, Modifier::UNDERLINED),
                        ] {
                            if on {
                                style = style.add_modifier(m);
                            }
                        }
                    } else if let Ok(Some(bg)) = cell.bg_color() {
                        // An erase in a color leaves color and no text.
                        style.bg = Some(Color::Rgb(bg.r, bg.g, bg.b));
                    }
                    let c = &mut buf[(area.x + x, area.y + y)];
                    text.clear();
                    match cell.graphemes_len() {
                        Ok(1..) if cell.graphemes_utf8(&mut text).is_ok() => c.set_symbol(&text),
                        _ => c.set_symbol(" "),
                    };
                    c.set_style(style);
                    x += 1;
                }
                y += 1;
            }
            if !snap.cursor_visible().unwrap_or(false) {
                return None;
            }
            let at = snap.cursor_viewport().ok()??;
            let blink = snap.cursor_blinking().unwrap_or(false);
            let shape = match (snap.cursor_visual_style(), blink) {
                (Ok(CursorVisualStyle::Bar), true) => SetCursorStyle::BlinkingBar,
                (Ok(CursorVisualStyle::Bar), false) => SetCursorStyle::SteadyBar,
                (Ok(CursorVisualStyle::Underline), true) => SetCursorStyle::BlinkingUnderScore,
                (Ok(CursorVisualStyle::Underline), false) => SetCursorStyle::SteadyUnderScore,
                (_, true) => SetCursorStyle::BlinkingBlock,
                (_, false) => SetCursorStyle::SteadyBlock,
            };
            Some((area.x + at.x, area.y + at.y, shape))
        }
    }

    impl Drop for Ghost {
        fn drop(&mut self) {
            let _ = self.child.kill();
        }
    }

    /// Palette colors stay palette colors, so the host terminal's theme
    /// shows through; unset is the host's own foreground or background.
    fn color(c: StyleColor) -> Option<Color> {
        match c {
            StyleColor::None => None,
            StyleColor::Palette(i) => Some(Color::Indexed(i.0)),
            StyleColor::Rgb(c) => Some(Color::Rgb(c.r, c.g, c.b)),
        }
    }

    /// The key that types `c` on a US keyboard, for the encoder's
    /// Ctrl and kitty sequences.
    fn char_key(c: char) -> Key {
        let from = |base: Key, first: char| {
            Key::try_from(base as u32 + (c as u32 - first as u32)).unwrap_or(Key::Unidentified)
        };
        match c.to_ascii_lowercase() {
            'a'..='z' => from(Key::A, if c.is_ascii_uppercase() { 'A' } else { 'a' }),
            '0'..='9' => from(Key::Digit0, '0'),
            ' ' => Key::Space,
            '-' => Key::Minus,
            '=' => Key::Equal,
            '[' => Key::BracketLeft,
            ']' => Key::BracketRight,
            '\\' => Key::Backslash,
            ';' => Key::Semicolon,
            '\'' => Key::Quote,
            ',' => Key::Comma,
            '.' => Key::Period,
            '/' => Key::Slash,
            '`' => Key::Backquote,
            _ => Key::Unidentified,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::{Duration, Instant};

        fn screen(g: &mut Ghost) -> String {
            let area = Rect::new(0, 0, 40, 6);
            let mut buf = Buffer::empty(area);
            g.draw(&mut buf, area);
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        #[test]
        fn a_shell_runs_and_answers_keys() {
            let mut g = Ghost::spawn(40, 6).unwrap();
            for c in "echo $((6*7))\r".chars() {
                let code = if c == '\r' { C::Enter } else { C::Char(c) };
                assert!(!g.input(KeyEvent::new(code, M::NONE)));
            }
            let start = Instant::now();
            while !screen(&mut g).lines().any(|l| l.trim() == "42") {
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "{}",
                    screen(&mut g)
                );
                assert!(g.pump().is_some(), "the shell exited");
                std::thread::sleep(Duration::from_millis(20));
            }
            g.input(KeyEvent::new(C::Char('d'), M::CONTROL));
            while g.pump().is_some() {
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "Ctrl-D didn't end it"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        #[test]
        fn colors_keep_their_palette() {
            let mut g = Ghost::spawn(40, 6).unwrap();
            g.vt.vt_write(b"\x1b[2J\x1b[H\x1b[1;31mred\x1b[0m \x1b[38;2;1;2;3mrgb");
            let area = Rect::new(0, 0, 40, 6);
            let mut buf = Buffer::empty(area);
            g.draw(&mut buf, area);
            assert_eq!(buf[(0, 0)].fg, Color::Indexed(1));
            assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(buf[(3, 0)].fg, Color::Reset);
            assert_eq!(buf[(4, 0)].fg, Color::Rgb(1, 2, 3));
        }

        #[test]
        fn jj_and_ctrl_backslash_ctrl_n_leave() {
            let mut g = Ghost::spawn(40, 6).unwrap();
            let k = |c| KeyEvent::new(C::Char(c), M::NONE);
            assert!(!g.input(k('j')));
            assert!(g.input(k('j')));
            // A `j` then something else: both go to the shell.
            assert!(!g.input(k('j')));
            assert!(!g.input(k('x')));
            assert!(g.held.is_none());
            assert!(!g.input(k('j')));
            g.held.as_mut().unwrap().1 -= JJ;
            assert!(g.flush() && g.held.is_none());
            assert!(!g.input(KeyEvent::new(C::Char('\\'), M::CONTROL)));
            assert!(g.input(KeyEvent::new(C::Char('n'), M::CONTROL)));
        }

        #[test]
        fn keys_map_to_ghostty_keys() {
            assert_eq!(char_key('a'), Key::A);
            assert_eq!(char_key('Z'), Key::Z);
            assert_eq!(char_key('7'), Key::Digit7);
            assert_eq!(char_key('/'), Key::Slash);
            assert_eq!(char_key('é'), Key::Unidentified);
        }
    }
}
