//! Terminal buffers (`:terminal`, `Space t`): a Ghostty terminal in a
//! window, like any other buffer.

use super::{Panel, Parked, Session, SwapState, make_editor};
use crate::buffer::Buffer;
use crate::editor::Mode;
use crate::ex::{BufCmd, ExEffect, SplitWhat};
use crate::term::Ghost;
use ratatui::crossterm::event::KeyEvent;
use std::cell::RefCell;

impl Session {
    /// A new terminal: in the active window, or (`split`) a new one below.
    pub(super) fn terminal(&mut self, split: bool) {
        if split {
            self.split_at(false, true, SplitWhat::Same, None);
        }
        let (cols, rows) = self
            .placed()
            .into_iter()
            .find(|p| p.id == self.active)
            .map_or((80, 24), |p| (p.text().width, p.text().height));
        let ghost = match Ghost::spawn(cols, rows) {
            Ok(g) => g,
            Err(e) => {
                if split {
                    self.close_active();
                }
                return self.ed.set_err(e);
            }
        };
        let mut ed = make_editor(
            Buffer::from_text(""),
            self.ed.opts.for_new_buffer(),
            &self.cfg,
        );
        ed.term = Some(Box::new(RefCell::new(ghost)));
        // Normal mode in it moves between windows, not over hidden text.
        ed.readonly = true;
        let swap = self.swap_dir.join(format!("terminal-{}", self.bufs.len()));
        self.bufs.push(Some(Parked {
            ed,
            stamp: None,
            swap_path: swap,
            lossy: false,
            no_swap: true,
            swap_state: SwapState::None,
            used: 0,
            hl: None,
            view: Default::default(),
        }));
        self.show(self.bufs.len() - 1);
        self.ed.mode = Mode::Insert;
    }

    /// The GUI's terminal panel, shown and focused: a full-width window
    /// across the bottom, 40% of the height, with the hidden panel's
    /// shell if it is still running, else a new one.
    pub fn show_panel(&mut self) {
        if let Some(w) = self
            .panel
            .win
            .filter(|w| self.windows.iter().any(|x| x.id == *w))
        {
            self.focus(w);
            if self.ed.term.is_some() {
                self.ed.mode = Mode::Insert;
            }
            return;
        }
        let hidden = self.panel_buf();
        let id = self.next_win_id;
        let n = (self.area.height * 2 / 5).max(6);
        if let Err(e) = self.tree.split_bottom(self.area, id, n) {
            return self.ed.set_err(e);
        }
        self.next_win_id += 1;
        let mut w = self.win().clone();
        let from = w.buf;
        w.id = id;
        w.tabs = vec![from];
        w.preview = None;
        w.prev = None;
        w.stand_in = Some(from); // the terminal's tab replaces it
        let cur = self.ed.cur;
        self.win_mut().cur = cur;
        self.windows.push(w);
        self.prev_win = Some(self.active);
        self.active = id;
        self.panel = Panel {
            win: Some(id),
            ..Panel::default()
        };
        match hidden {
            Some(b) => {
                self.show(b);
                self.ed.mode = Mode::Insert;
            }
            None => self.terminal(false),
        }
        self.win_mut().stand_in = None;
        if self.ed.term.is_none() {
            // No shell: no panel.
            self.panel.win = None;
            self.close_active();
        }
    }

    /// The hidden panel's terminal, if its shell is still there.
    fn panel_buf(&self) -> Option<usize> {
        let b = self.panel.buf?;
        (b < self.bufs.len()
            && (b == self.cur || self.bufs[b].is_some())
            && self.ed_at(b).term.is_some())
        .then_some(b)
    }

    /// Ctrl-`: show the panel, focus it, or (focused) hide it with its shell running.
    pub fn toggle_panel(&mut self) {
        match self
            .panel
            .win
            .filter(|w| self.windows.iter().any(|x| x.id == *w))
        {
            Some(w) if w == self.active => {
                self.restore_panel();
                let buf = self.win().buf;
                if self.close_window(w).is_ok() {
                    self.panel = Panel {
                        buf: Some(buf),
                        ..Panel::default()
                    };
                }
            }
            _ => self.show_panel(),
        }
    }

    /// The panel takes the full height, or goes back to its size.
    pub fn maximize_panel(&mut self) {
        if self.panel.restore.is_some() {
            return self.restore_panel();
        }
        let Some(w) = self
            .panel
            .win
            .filter(|w| self.windows.iter().any(|x| x.id == *w))
        else {
            return self.show_panel();
        };
        let Some(sizes) = self.tree.bottom_sizes(self.area, w) else {
            return;
        };
        let mut full = vec![0; sizes.len()];
        *full.last_mut().expect("a split") = self.area.height;
        self.tree.set_sizes(&full);
        self.panel.restore = Some(sizes);
        self.focus(w);
    }

    fn restore_panel(&mut self) {
        if let Some(sizes) = self.panel.restore.take() {
            self.tree.set_sizes(&sizes);
        }
    }

    /// In terminal mode: the active window's terminal has the keys.
    pub fn in_terminal(&self) -> bool {
        self.ed.term.is_some() && self.ed.mode == Mode::Insert
    }

    /// A key in terminal mode, for the shell; `jj` or `Ctrl-\ Ctrl-n` is
    /// Normal mode again.
    pub fn term_key(&mut self, k: KeyEvent) {
        let Some(t) = &self.ed.term else { return };
        if t.borrow_mut().input(k) {
            self.ed.mode = Mode::Normal;
        }
    }

    pub fn term_paste(&mut self, text: &str) {
        if let Some(t) = &self.ed.term {
            t.borrow_mut().paste(text);
        }
    }

    /// Every terminal's output, and a held `j` typed: whether any screen
    /// changed. A terminal whose shell exited goes, with its windows.
    pub fn tick_terms(&mut self) -> bool {
        let mut changed = false;
        let mut done = vec![];
        for i in 0..self.bufs.len() {
            let ed = if i == self.cur {
                &self.ed
            } else {
                match &self.bufs[i] {
                    Some(p) => &p.ed,
                    None => continue,
                }
            };
            let Some(t) = &ed.term else { continue };
            let mut t = t.borrow_mut();
            changed |= t.flush();
            match t.pump() {
                Some(any) => changed |= any,
                None => done.push(i),
            }
        }
        // Highest first: deleting one renumbers those above it.
        for i in done.into_iter().rev() {
            self.panel.buf = match self.panel.buf {
                Some(b) if b == i => None,
                Some(b) if b > i => Some(b - 1),
                b => b,
            };
            let mode = (i == self.cur).then_some(Mode::Normal);
            self.perform(ExEffect::Buffer {
                cmd: BufCmd::Delete,
                arg: (i + 1).to_string(),
                force: true,
            });
            if let Some(m) = mode {
                self.ed.mode = m;
            }
            self.ed.set_msg("the shell exited");
            changed = true;
        }
        changed
    }
}
