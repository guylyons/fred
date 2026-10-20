//! Terminal buffers (`:terminal`, `Space t`): a Ghostty terminal in a
//! window, like any other buffer.

use super::{Parked, Session, SwapState, make_editor};
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
            .tree
            .layout(self.area)
            .into_iter()
            .find(|p| p.id == self.active)
            .map_or((80, 24), |p| {
                (p.rect.width - u16::from(p.divider), p.rect.height - 1)
            });
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
