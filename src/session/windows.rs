//! Splitting windows and moving between them (`:split`, `CTRL-W`).

use super::{Parked, Session, SwapState, make_editor};
use crate::buffer::Buffer;
use crate::editor::Mode;
use crate::ex::{BufCmd, ExEffect, SplitWhat};
use crate::window::Dir;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

impl Session {
    /// `:split` and friends: the active window divides, the new part (above,
    /// or left when `vertical`) becomes active showing the same buffer, then
    /// `what`. `size` is in text rows or columns.
    pub fn split(&mut self, vertical: bool, what: SplitWhat, size: Option<usize>) {
        self.split_at(vertical, false, what, size)
    }

    /// `split`, the new window below (or right of) the old one when `after`.
    pub(super) fn split_at(
        &mut self,
        vertical: bool,
        after: bool,
        what: SplitWhat,
        size: Option<usize>,
    ) {
        let dir = if vertical { Dir::Row } else { Dir::Col };
        // Plus the new window's status line or divider.
        let size = size.map(|n| n.saturating_add(1).min(u16::MAX as usize) as u16);
        let id = self.next_win_id;
        if let Err(e) = self
            .tree
            .split(self.area, self.active, id, dir, after, size)
        {
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

    /// `Space w c N`: start over with `n` windows in as even a grid as fits,
    /// each showing this buffer: rows of `ceil(sqrt(n))`, the extra ones in
    /// the top rows (3 is two above one).
    pub fn grid(&mut self, n: usize) {
        self.only();
        let cols = (1..=n).find(|c| c * c >= n).unwrap_or(1);
        let rows = n.div_ceil(cols);
        let per_row: Vec<usize> = (0..rows)
            .map(|r| n / rows + usize::from(r < n % rows))
            .collect();
        // Each split leaves the window its share and gives the new one the rest.
        let size = |s: &Session, vertical: bool, left: usize| {
            let r = s.placed().into_iter().find(|p| p.id == s.active)?.rect;
            let len = usize::from(if vertical { r.width } else { r.height });
            Some((len - len / left).saturating_sub(1))
        };
        let mut heads = vec![self.active];
        for left in (2..=rows).rev() {
            let Some(rest) = size(self, false, left) else {
                break;
            };
            self.split_at(false, true, SplitWhat::Same, Some(rest));
            heads.push(self.active);
        }
        for (head, count) in heads.iter().zip(&per_row) {
            self.focus(*head);
            for left in (2..=*count).rev() {
                let Some(rest) = size(self, true, left) else {
                    break;
                };
                self.split_at(true, true, SplitWhat::Same, Some(rest));
            }
        }
        self.focus(heads[0]);
        match self.windows.len() {
            made if made < n => self
                .ed
                .set_err(format!("E36: room for {made} windows, not {n}")),
            _ => self.ed.msg = None,
        }
    }

    /// A new unnamed buffer in the active window.
    pub(super) fn new_buffer(&mut self) {
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
        let old = self.placed().into_iter().find(|p| p.id == id);
        self.tree.close(self.area, id)?;
        if id == self.active {
            let placed = self.placed();
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

    /// The mouse, on a `screen` this size. With windows split, clicking one
    /// makes it active, and the click lands in it as with one window.
    pub fn mouse(&mut self, screen: Rect, m: MouseEvent) {
        let mut area = screen;
        // The picker and the right-click menu are drawn over the whole screen.
        let windows = self.ed.menu.is_none() && !matches!(self.ed.mode, Mode::Pick(_));
        if windows && self.windows.len() > 1 {
            // A press on a status line or divider drags it to resize.
            match (m.kind, self.grab) {
                (MouseEventKind::Down(MouseButton::Left), _) => {
                    let at = (m.column, m.row).into();
                    let placed = self.placed();
                    let p = placed.iter().find(|p| p.rect.contains(at));
                    let status = p
                        .filter(|p| m.row + 1 == p.rect.bottom())
                        .map(|p| (Dir::Col, p.rect.bottom(), m.column));
                    let divider = p
                        .filter(|p| p.divider && m.column + 1 == p.rect.right())
                        .map(|p| (Dir::Row, p.rect.right(), m.row));
                    self.grab = [status, divider]
                        .into_iter()
                        .flatten()
                        .find(|&(d, e, c)| self.tree.drag(self.area, d, e, c, e).is_some());
                    if self.grab.is_some() {
                        return;
                    }
                }
                (MouseEventKind::Drag(MouseButton::Left), Some((d, edge, cross))) => {
                    let to = if d == Dir::Col { m.row } else { m.column } + 1;
                    let edge = self
                        .tree
                        .drag(self.area, d, edge, cross, to)
                        .unwrap_or(edge);
                    self.grab = Some((d, edge, cross));
                    return;
                }
                (MouseEventKind::Up(_), Some(_)) => {
                    self.grab = None;
                    return;
                }
                _ => {}
            }
            let placed = self.placed();
            if m.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(p) = placed
                    .iter()
                    .find(|p| p.rect.contains((m.column, m.row).into()))
                && p.id != self.active
            {
                // A click half-typed into the old window doesn't carry over.
                self.ed.vim.pending.clear();
                self.focus(p.id);
                self.win_mut().view.last_click = None;
            }
        }
        // Its text and status line, as `mouse` expects the whole screen
        // to be (it leaves out the last two rows): outside it, and on the
        // GUI's tab row and minimap, it is ignored.
        if windows
            && (self.windows.len() > 1 || self.chrome != (0, 0))
            && let Some(p) = self.placed().iter().find(|p| p.id == self.active)
        {
            let text = p.text();
            area = Rect {
                height: text.height + 2,
                ..text
            };
        }
        // A terminal: the wheel scrolls back through its output, and a
        // click is for typing in it.
        if let Some(t) = &self.ed.term {
            match m.kind {
                MouseEventKind::ScrollUp => t.borrow_mut().scroll(-3),
                MouseEventKind::ScrollDown => t.borrow_mut().scroll(3),
                MouseEventKind::Down(MouseButton::Left) => self.ed.mode = Mode::Insert,
                _ => {}
            }
            return;
        }
        let mut view = self.win().view;
        crate::ui::render::mouse(&mut self.ed, &mut view, area, m);
        self.win_mut().view = view;
    }

    /// Zap just opened on the active window: add the words in every other.
    pub(super) fn zap_windows(&mut self) {
        let placed = self.placed();
        let mut more = vec![];
        for w in &self.windows {
            let Some(p) = placed.iter().find(|p| p.id == w.id && p.id != self.active) else {
                continue;
            };
            let ed = match self.bufs[w.buf].as_ref() {
                _ if w.buf == self.cur => &self.ed,
                Some(park) => &park.ed,
                None => continue,
            };
            // ponytail: a wrapped line under the live cursor counts that
            // cursor, not this window's; off by a row at worst, near it.
            let vp = crate::zap::Viewport::of(p.text(), ed, w.view);
            more.extend(crate::zap::words(ed, vp).into_iter().map(|mut x| {
                x.win = Some(w.id);
                x
            }));
        }
        if let Some(z) = &mut self.ed.zap {
            z.words.extend(more);
        }
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
        // Going to a terminal is going to type in it.
        if self.ed.term.is_some() {
            self.ed.mode = Mode::Insert;
        }
    }

    /// The active window's cursor cell on screen, near enough to choose
    /// between neighbours.
    fn cursor_cell(&self) -> (u16, u16) {
        let Some(p) = self.placed().into_iter().find(|p| p.id == self.active) else {
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
            p.rect.y + p.header + y.min(last(p.rect.height - p.header)) as u16,
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
                if let Some(id) =
                    self.tree
                        .neighbor(self.area, self.active, c, n, self.cursor_cell())
                {
                    self.focus(id);
                }
            }
            'w' => self.focus(count.map_or(order[(at + 1) % order.len()], nth)),
            'W' => self.focus(count.map_or(order[(at + order.len() - 1) % order.len()], nth)),
            // Fred's own: `Space w` numbers the windows to pick one.
            '#' => {
                self.picking_window = true;
                self.ed
                    .set_msg("window number, or c then how many for a grid");
            }
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
