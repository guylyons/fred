//! Splitting windows and moving between them (`:split`, `CTRL-W`).

use super::{Parked, Session, SwapState, make_editor};
use crate::buffer::Buffer;
use crate::ex::{BufCmd, ExEffect, SplitWhat};
use crate::window::Dir;

impl Session {
    /// `:split` and friends: the active window divides, the new part (above,
    /// or left when `vertical`) becomes active showing the same buffer, then
    /// `what`. `size` is in text rows or columns.
    pub fn split(&mut self, vertical: bool, what: SplitWhat, size: Option<usize>) {
        let dir = if vertical { Dir::Row } else { Dir::Col };
        // Plus the new window's status line or divider.
        let size = size.map(|n| n.saturating_add(1).min(u16::MAX as usize) as u16);
        let id = self.next_win_id;
        if let Err(e) = self.tree.split(self.area, self.active, id, dir, false, size) {
            return self.ed.set_err(e);
        }
        self.next_win_id += 1;
        let cur = self.ed.cur;
        let mut w = self.win().clone();
        w.id = id;
        self.win_mut().cur = cur;
        self.windows.push(w);
        self.prev_win = Some(self.active);
        self.active = id;
        match what {
            SplitWhat::Same => {}
            SplitWhat::File(p) => self.perform(ExEffect::Edit {
                path: Some(p),
                force: false,
            }),
            SplitWhat::Buffer(arg) => self.perform(ExEffect::Buffer {
                cmd: BufCmd::Go,
                arg,
                force: false,
            }),
            SplitWhat::Empty => self.new_buffer(),
        }
    }

    /// A new unnamed buffer in the active window.
    fn new_buffer(&mut self) {
        let opts = self.ed.opts.for_new_buffer();
        let ed = make_editor(Buffer::from_text(""), opts, &self.cfg);
        self.clock += 1;
        let swap = self
            .swap_dir
            .join(format!("new-{}-{}.swp", std::process::id(), self.clock));
        self.bufs.push(Some(Parked {
            ed,
            stamp: None,
            swap_path: swap,
            lossy: false,
            no_swap: false,
            swap_state: SwapState::None,
            used: 0,
            hl: None,
            view: Default::default(),
        }));
        self.show(self.bufs.len() - 1);
    }

    /// Close window `id`; the window that takes its space (above or left,
    /// else below or right) becomes active if `id` was. Its buffer stays.
    pub fn close_window(&mut self, id: usize) -> Result<(), &'static str> {
        let old = self.tree.layout(self.area).into_iter().find(|p| p.id == id);
        self.tree.close(self.area, id)?;
        if id == self.active {
            let placed = self.tree.layout(self.area);
            let heir = old
                .and_then(|o| placed.iter().find(|p| p.rect.intersects(o.rect)))
                .map_or(self.tree.order()[0], |p| p.id);
            self.focus(heir);
        }
        self.windows.retain(|w| w.id != id);
        if self.prev_win == Some(id) {
            self.prev_win = None;
        }
        Ok(())
    }

    /// `:close`, `CTRL-W c`, `:q` in a split.
    pub(super) fn close_active(&mut self) {
        if let Err(e) = self.close_window(self.active) {
            self.ed.set_err(e);
        }
    }

    /// `:only`, `CTRL-W o`: every other window closes.
    pub fn only(&mut self) {
        let keep = self.active;
        self.tree.only(keep);
        self.windows.retain(|w| w.id == keep);
        self.prev_win = None;
    }

    /// `:wqall`: write every modified buffer that has a file, stopping at
    /// the first that fails (its error shown). True when all were written.
    pub(super) fn write_all(&mut self) -> bool {
        let home = self.cur;
        let mut ok = true;
        for i in 0..self.bufs.len() {
            let ed = self.ed_at(i);
            if !ed.buf.modified || ed.path.is_none() || ed.generated() {
                continue;
            }
            if i != self.cur {
                self.make_live(i);
            }
            if !self.write(None, false, None) {
                ok = false;
                break;
            }
        }
        if self.cur != home {
            let msg = self.ed.msg.take();
            self.make_live(home);
            self.ed.msg = msg;
        }
        ok
    }

    /// Make window `id` the active one, with its own cursor.
    pub fn focus(&mut self, id: usize) {
        if id == self.active || !self.windows.iter().any(|w| w.id == id) {
            return;
        }
        let cur = self.ed.cur;
        self.win_mut().cur = cur;
        self.prev_win = Some(self.active);
        self.active = id;
        let (buf, c) = (self.win().buf, self.win().cur);
        if buf != self.cur {
            self.make_live(buf);
        }
        self.ed.set_cursor(c.line, c.byte);
        self.ed.cur.want_col = c.want_col;
        // The text may have shrunk under it, from another window.
        self.ed.clamp_cursor();
    }

    /// The active window's cursor cell on screen, near enough to choose
    /// between neighbours.
    fn cursor_cell(&self) -> (u16, u16) {
        let Some(p) = self.tree.layout(self.area).into_iter().find(|p| p.id == self.active) else {
            return (0, 0);
        };
        let view = self.win().view;
        let line = self.ed.buf.line(self.ed.cur.line);
        let col = crate::text::col_of_byte(&line, self.ed.cur.byte, self.ed.tabstop());
        let x = crate::ui::render::gutter_width(&self.ed) + col.saturating_sub(view.left);
        let y = self.ed.cur.line.saturating_sub(view.top);
        let last = |len: u16| len.saturating_sub(1) as usize;
        (
            p.rect.x + x.min(last(p.rect.width)) as u16,
            p.rect.y + y.min(last(p.rect.height)) as u16,
        )
    }

    /// A `CTRL-W` command.
    pub fn wincmd(&mut self, c: char, count: Option<usize>) {
        let order = self.tree.order();
        let at = order.iter().position(|&id| id == self.active).unwrap_or(0);
        let nth = |n: usize| order[n.clamp(1, order.len()) - 1];
        match c {
            's' | 'S' => self.split(false, SplitWhat::Same, count),
            'v' => self.split(true, SplitWhat::Same, count),
            'n' => self.split(false, SplitWhat::Empty, count),
            // With a count, that buffer; else the alternate one.
            '^' => {
                let arg = count.map_or("#".into(), |n| n.to_string());
                self.split(false, SplitWhat::Buffer(arg), None)
            }
            'h' | 'j' | 'k' | 'l' => {
                let n = count.unwrap_or(1);
                if let Some(id) = self.tree.neighbor(self.area, self.active, c, n, self.cursor_cell()) {
                    self.focus(id);
                }
            }
            'w' => self.focus(count.map_or(order[(at + 1) % order.len()], nth)),
            'W' => self.focus(count.map_or(order[(at + order.len() - 1) % order.len()], nth)),
            't' => self.focus(order[0]),
            'b' => self.focus(order[order.len() - 1]),
            'c' => self.close_active(),
            'q' => self.perform(ExEffect::Quit { force: false }),
            'o' => self.only(),
            'p' => {
                if let Some(id) = self.prev_win {
                    self.focus(id);
                }
            }
            _ => {}
        }
    }
}
