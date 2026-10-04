//! Session services for Org: special buffers (notes, capture), and
//! editing other files' buffers (agenda, refile, capture targets).

use super::*;

/// What runs when a special buffer is finished (C-c C-c) or aborted (C-c C-k):
/// its text and whether it was aborted.
pub type Finish = Box<dyn FnOnce(&mut Session, String, bool) + Send>;

impl Session {
    /// Open a special Org buffer named `name` (`*Org Note*`) holding
    /// `text`, the cursor at `cursor`. `finish` runs on C-c C-c / C-c C-k.
    pub fn org_special(&mut self, name: &str, text: &str, cursor: (usize, usize), finish: Finish) {
        let mut ed = make_editor(Buffer::from_text(text), &self.cfg);
        ed.org = Some(Box::default());
        ed.org_buffer_name = Some(name.to_owned());
        ed.org_finish = Some(crate::org::FinishSlot::new(finish));
        ed.org_return = Some(self.cur);
        ed.set_cursor(cursor.0, cursor.1);
        let swap = self.swap_dir.join(format!("org-special-{}", self.bufs.len()));
        self.bufs.push(Some(Parked {
            ed,
            stamp: None,
            swap_path: swap,
            lossy: false,
            no_swap: true,
            swap_state: SwapState::None,
            used: 0,
        }));
        self.show(self.bufs.len() - 1);
    }

    /// Finish the special buffer being edited (`abort` for C-c C-k).
    pub fn org_finish(&mut self, abort: bool) {
        let Some(slot) = self.ed.org_finish.take() else {
            return;
        };
        let text = self.ed.buf.text();
        let back = self.ed.org_return.take();
        let me = self.cur;
        if let Some(b) = back.filter(|&b| b < self.bufs.len() && b != me) {
            self.show(b);
        }
        // Drop a special buffer (a capture's file buffer stays).
        if self.cur != me && self.ed_at(me).path.is_none() {
            self.bufs.remove(me);
            if me < self.cur {
                self.cur -= 1;
            }
            self.clock += 1;
        }
        if let Some(f) = slot.take() {
            f(self, text, abort);
        }
    }

    /// The index of a buffer visiting `path`, opening it (not shown) if needed.
    pub fn org_buffer(&mut self, path: &Path) -> Result<usize, String> {
        if self.is_own_file(path) {
            return Ok(self.cur);
        }
        if let Some(i) = self.find(path) {
            return Ok(i);
        }
        let o = open_file(Some(path), &self.cfg)?;
        let swap = swap::swap_path_in(&self.swap_dir, Some(path));
        self.bufs.push(Some(Parked {
            ed: o.ed,
            stamp: o.stamp,
            swap_path: swap,
            lossy: o.lossy,
            no_swap: false,
            swap_state: SwapState::None,
            used: 0,
        }));
        Ok(self.bufs.len() - 1)
    }

    /// Run `f` on the editor of buffer `i`.
    pub fn org_with_buffer<R>(&mut self, i: usize, f: impl FnOnce(&mut Editor) -> R) -> R {
        if i == self.cur {
            return f(&mut self.ed);
        }
        let ed = &mut self.bufs[i].as_mut().expect("a parked buffer").ed;
        f(ed)
    }

    /// Run `f` on the buffer visiting `path` (opened if needed).
    pub fn org_with_file<R>(&mut self, path: &Path, f: impl FnOnce(&mut Editor) -> R) -> Result<R, String> {
        let i = self.org_buffer(path)?;
        Ok(self.org_with_buffer(i, f))
    }

    /// The text of `path`: its buffer if open, else the file.
    pub fn org_text(&self, path: &Path) -> Result<String, String> {
        if self.is_own_file(path) {
            return Ok(self.ed.buf.text());
        }
        if let Some(i) = self.find(path) {
            return Ok(self.ed_at(i).buf.text());
        }
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Show buffer `i`.
    pub fn org_show(&mut self, i: usize) {
        self.show(i);
    }

    /// Visit `path` at line `l` (opening it).
    pub fn org_visit(&mut self, path: &Path, l: usize) -> Result<(), String> {
        let i = self.org_buffer(path)?;
        self.show(i);
        let l = l.min(self.ed.line_count() - 1);
        self.ed.set_cursor(l, 0);
        if self.ed.folds.hidden(l) {
            self.ed.reveal_cursor();
        }
        Ok(())
    }

    /// The current buffer's index.
    pub fn org_current(&self) -> usize {
        self.cur
    }

    /// Every open buffer's editor (for saving Org buffers).
    pub fn org_editors(&mut self) -> Vec<&mut Editor> {
        self.editors_mut().collect()
    }

    /// Write buffer `i` to its file (org-save-all-org-buffers).
    pub fn org_save(&mut self, i: usize) -> Result<(), String> {
        let before = self.cur;
        if i != self.cur {
            self.show(i);
        }
        let ok = self.write(None, false, None);
        if before != self.cur {
            self.show(before);
        }
        if ok { Ok(()) } else { Err(self.ed.msg.clone().map(|m| m.0).unwrap_or_default()) }
    }

    /// Number of buffers.
    pub fn org_buffer_count(&self) -> usize {
        self.bufs.len()
    }

    /// The path of buffer `i`.
    pub fn org_buffer_path(&self, i: usize) -> Option<PathBuf> {
        self.ed_at(i).path.clone()
    }
}
