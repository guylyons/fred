//! Sublime-style tabs: each window keeps the buffers it has shown, in
//! display order, and at most one preview tab that the next preview
//! replaces. Closing a tab hides its buffer; `:bd` still deletes it.

use super::Session;
use crate::window::Window;
use std::path::Path;

const NO_TAB: &str = "no such tab";

impl Session {
    fn window(&mut self, id: usize) -> &mut Window {
        self.windows
            .iter_mut()
            .find(|w| w.id == id)
            .expect("a window")
    }

    /// The buffer in window `win`'s tab `index`.
    fn tab(&self, win: usize, index: usize) -> Result<usize, String> {
        self.windows
            .iter()
            .find(|w| w.id == win)
            .and_then(|w| w.tabs.get(index).copied())
            .ok_or_else(|| NO_TAB.into())
    }

    /// A click on a tab: its window becomes active, showing it.
    pub fn select_tab(&mut self, win: usize, index: usize) -> Result<(), String> {
        let b = self.tab(win, index)?;
        self.focus(win);
        self.show(b);
        self.check_tabs();
        Ok(())
    }

    /// Close a tab, in that window only; its buffer stays open, hidden,
    /// even with unsaved changes (as `:hide`). Closing the shown one shows
    /// its right neighbour, else its left; closing the last closes the
    /// window, or in the last window leaves an empty `[No Name]`.
    pub fn close_tab(&mut self, win: usize, index: usize) -> Result<(), String> {
        let b = self.tab(win, index)?;
        let w = self.window(win);
        let (shown, tabs) = (w.buf == b, w.tabs.clone());
        if shown && tabs.len() == 1 && self.windows.len() > 1 {
            self.close_window(win).map_err(str::to_string)?;
            self.check_tabs();
            return Ok(());
        }
        if shown {
            // Focus moves there and back: CTRL-W p's target stays.
            let (back, prev_win) = (self.active, self.prev_win);
            self.focus(win);
            let blank =
                self.ed.path.is_none() && !self.ed.buf.modified && self.ed.buf.len_bytes() == 0;
            if tabs.len() > 1 {
                self.show(
                    tabs[if index + 1 < tabs.len() {
                        index + 1
                    } else {
                        index - 1
                    }],
                );
            } else if !blank {
                self.new_buffer();
            }
            self.focus(back);
            self.prev_win = prev_win;
            if tabs.len() == 1 && blank {
                // Already an empty `[No Name]`: nothing to close.
                return Ok(());
            }
        }
        let w = self.window(win);
        w.tabs.retain(|&t| t != b);
        w.preview = w.preview.filter(|&p| p != b);
        w.prev = w.prev.filter(|&p| p != b);
        self.check_tabs();
        Ok(())
    }

    /// Drag tab `index` to `to` (past the end means last).
    pub fn move_tab(&mut self, win: usize, index: usize, to: usize) -> Result<(), String> {
        let b = self.tab(win, index)?;
        let tabs = &mut self.window(win).tabs;
        tabs.remove(index);
        tabs.insert(to.min(tabs.len()), b);
        self.check_tabs();
        Ok(())
    }

    /// A double-click on a preview tab: it stays.
    pub fn keep_tab(&mut self, win: usize, index: usize) -> Result<(), String> {
        let b = self.tab(win, index)?;
        let w = self.window(win);
        w.preview = w.preview.filter(|&p| p != b);
        Ok(())
    }

    /// The sidebar's `open`: as the picker opens a file, then, with
    /// `preview`, a new tab is the window's preview, in place of the old
    /// one. The old one's buffer stays open unless this preview opened it
    /// and it is unmodified and in no other tab. Without `preview`, the
    /// file's preview tab becomes permanent.
    pub fn open_preview(&mut self, path: &Path, preview: bool) {
        let (before, n) = (self.win().tabs.clone(), self.bufs.len());
        self.open_pick(path.to_path_buf(), None);
        self.keep_edited_previews();
        let i = self.cur;
        let id = self.active;
        let w = self.window(id);
        if !preview {
            w.preview = w.preview.filter(|&p| p != i);
        }
        if !preview || before.contains(&i) {
            return self.check_tabs();
        }
        if let Some(old) = w.preview.take() {
            w.tabs.retain(|&t| t != i);
            let at = w
                .tabs
                .iter()
                .position(|&t| t == old)
                .expect("preview in tabs");
            w.tabs[at] = i;
            // No longer a tab here: CTRL-^ must not bring it back.
            w.prev = w.prev.filter(|&p| p != old);
            let fresh = w.preview_mark.1;
            if fresh
                && !self.ed_at(old).buf.modified
                && !self.windows.iter().any(|w| w.tabs.contains(&old))
            {
                self.drop_buffer(old);
            }
        }
        let (version, cur) = (self.ed.buf.version, self.cur);
        let w = self.window(id);
        w.preview = Some(cur);
        w.preview_mark = (version, i >= n);
        self.check_tabs();
    }

    /// Text typed into a preview makes it permanent.
    pub(crate) fn keep_edited_previews(&mut self) {
        for k in 0..self.windows.len() {
            let w = &self.windows[k];
            let edited = w.preview.is_some_and(|p| {
                let buf = &self.ed_at(p).buf;
                buf.modified || buf.version != w.preview_mark.0
            });
            if edited {
                self.windows[k].preview = None;
            }
        }
    }

    /// Every window's tabs hold its shown buffer, real buffers, no
    /// duplicates, its preview and its alternate (CTRL-^). Debug builds only.
    pub(crate) fn check_tabs(&self) {
        if !cfg!(debug_assertions) {
            return;
        }
        for w in &self.windows {
            let mut seen = w.tabs.clone();
            seen.sort_unstable();
            seen.dedup();
            assert!(
                w.tabs.contains(&w.buf)
                    && seen.len() == w.tabs.len()
                    && seen.last().is_none_or(|&b| b < self.bufs.len())
                    && w.preview.is_none_or(|p| w.tabs.contains(&p))
                    && w.prev.is_none_or(|p| w.tabs.contains(&p)),
                "bad tabs: {w:?} with {} buffers",
                self.bufs.len()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Session;
    use crate::config::Config;
    use crate::editor::Mode;
    use std::fs;
    use std::path::PathBuf;

    struct T {
        dir: tempfile::TempDir,
        s: Session,
    }

    impl T {
        /// Files `names` (each holding its name), the first one open.
        fn new(names: &[&str]) -> T {
            let dir = tempfile::tempdir().unwrap();
            for n in names {
                fs::write(dir.path().join(n), format!("{n}\n")).unwrap();
            }
            let first = names.first().map(|n| dir.path().join(n));
            let (s, _) =
                Session::open(first, &Config::default(), &dir.path().join("swap")).unwrap();
            T { dir, s }
        }
        fn path(&self, n: &str) -> PathBuf {
            self.dir.path().join(n)
        }
        fn keys(&mut self, k: &str) {
            self.s.handle_keys(k);
        }
        fn e(&mut self, n: &str) {
            let p = self.path(n);
            self.keys(&format!(":e {}<Enter>", p.display()));
        }
        fn name(&self, b: usize) -> String {
            let ed = self.s.ed_at(b);
            ed.path
                .as_deref()
                .and_then(|p| p.file_name())
                .map_or("[No Name]".into(), |n| n.to_string_lossy().into_owned())
        }
        /// The active window's tabs by file name.
        fn tabs(&self) -> Vec<String> {
            self.s.win().tabs.iter().map(|&b| self.name(b)).collect()
        }
        fn current(&self) -> String {
            self.name(self.s.cur)
        }
        fn preview(&self) -> Option<String> {
            self.s.win().preview.map(|b| self.name(b))
        }
        fn id(&self) -> usize {
            self.s.active
        }
    }

    #[test]
    fn opening_files_adds_tabs_after_the_current_one() {
        let mut t = T::new(&["a", "b", "c"]);
        t.e("b");
        t.keys(":b1<Enter>");
        t.e("c");
        assert_eq!(t.tabs(), ["a", "c", "b"]);
        // Going to one already there just makes it current.
        t.e("b");
        assert_eq!(t.tabs(), ["a", "c", "b"]);
        assert_eq!(t.current(), "b");
        t.s.check_tabs();
    }

    #[test]
    fn closing_the_current_tab_shows_its_right_neighbour_then_left() {
        let mut t = T::new(&["a", "b", "c"]);
        t.e("b");
        t.e("c");
        let id = t.id();
        t.s.select_tab(id, 1).unwrap();
        assert_eq!(t.current(), "b");
        t.s.close_tab(id, 1).unwrap();
        assert_eq!(
            (t.tabs(), t.current()),
            (vec!["a".into(), "c".into()], "c".into())
        );
        t.s.close_tab(id, 1).unwrap();
        assert_eq!((t.tabs(), t.current()), (vec!["a".into()], "a".into()));
        assert_eq!(t.s.close_tab(id, 5), Err("no such tab".into()));
        assert_eq!(t.s.close_tab(id + 9, 0), Err("no such tab".into()));
        t.s.check_tabs();
    }

    #[test]
    fn closing_a_modified_tab_keeps_the_buffer_hidden() {
        let mut t = T::new(&["a", "b"]);
        t.e("b");
        t.keys("ix<Esc>");
        let id = t.id();
        t.s.close_tab(id, 1).unwrap();
        assert_eq!((t.tabs(), t.current()), (vec!["a".into()], "a".into()));
        t.keys(":ls<Enter>");
        let Mode::Pick(p) = &t.s.ed.mode else {
            panic!("no picker")
        };
        assert!(
            p.rows.iter().any(|r| r.text.ends_with("/b [+]")),
            "{:?}",
            p.rows.iter().map(|r| &r.text).collect::<Vec<_>>()
        );
        t.keys("<Esc>");
        t.s.check_tabs();
    }

    #[test]
    fn closing_the_last_tab_closes_the_window_or_leaves_no_name() {
        let mut t = T::new(&["a", "b"]);
        let p = t.path("b");
        t.keys(&format!(":split {}<Enter>", p.display()));
        let top = t.id();
        // The split carries a's tab over; close both of its tabs.
        let n = t.s.win().tabs.len();
        for _ in 0..n {
            t.s.close_tab(top, 0).unwrap();
        }
        assert_eq!(t.s.windows.len(), 1);
        assert_eq!(t.current(), "a");
        // The last window's last tab: an empty [No Name] in its place.
        let id = t.id();
        t.s.close_tab(id, 0).unwrap();
        assert_eq!(t.s.windows.len(), 1);
        assert_eq!(t.tabs(), ["[No Name]"]);
        assert_eq!(t.s.ed.buf.text(), "");
        // a and b are still open, hidden.
        assert_eq!(t.s.bufs.len(), 3);
        t.s.check_tabs();
    }

    #[test]
    fn bd_removes_the_buffer_from_every_windows_tabs_and_shifts_indices() {
        let mut t = T::new(&["a", "b", "c"]);
        t.e("b");
        t.e("c");
        // Two windows, each with tabs a, b, c; the bottom one shows a.
        t.keys(":split<Enter><C-w>j:b1<Enter><C-w>k");
        let other = t.s.windows.iter().find(|w| w.id != t.s.active).unwrap().id;
        t.keys(":bd 2<Enter>");
        assert_eq!(t.tabs(), ["a", "c"]);
        let w = t.s.windows.iter().find(|w| w.id == other).unwrap();
        let names: Vec<String> = w.tabs.iter().map(|&b| t.name(b)).collect();
        assert_eq!(names, ["a", "c"]);
        assert_eq!(t.name(w.buf), "a");
        t.s.check_tabs();
    }

    #[test]
    fn a_preview_is_replaced_by_the_next_preview_in_place() {
        let mut t = T::new(&["a", "b", "c", "d"]);
        t.s.open_preview(&t.path("b"), true);
        assert_eq!(
            (t.tabs(), t.preview()),
            (vec!["a".into(), "b".into()], Some("b".into()))
        );
        t.keys(":b1<Enter>");
        t.s.open_preview(&t.path("c"), true);
        // In b's place, not after a.
        assert_eq!(t.tabs(), ["a", "c"]);
        assert_eq!(t.preview(), Some("c".into()));
        // Opened without `preview` it becomes permanent; so does `keep`.
        t.s.open_preview(&t.path("c"), false);
        assert_eq!(t.preview(), None);
        t.s.open_preview(&t.path("d"), true);
        let id = t.id();
        t.s.keep_tab(id, 2).unwrap();
        assert_eq!(
            (t.tabs(), t.preview()),
            (vec!["a".into(), "c".into(), "d".into()], None)
        );
        // A preview of a tab already there just selects it.
        t.s.open_preview(&t.path("a"), true);
        assert_eq!((t.current(), t.preview()), ("a".into(), None));
        t.s.check_tabs();
    }

    #[test]
    fn editing_a_preview_keeps_it() {
        let mut t = T::new(&["a", "b", "c"]);
        t.s.open_preview(&t.path("b"), true);
        t.keys("ix<Esc>");
        assert_eq!(t.preview(), None);
        t.s.open_preview(&t.path("c"), true);
        assert_eq!(t.tabs(), ["a", "b", "c"]);
        assert_eq!(t.preview(), Some("c".into()));
        assert!((0..t.s.bufs.len()).any(|i| t.s.ed_at(i).buf.text() == "xb"));
        t.s.check_tabs();
    }

    #[test]
    fn an_unmodified_preview_only_buffer_is_deleted_when_replaced() {
        let mut t = T::new(&["a", "b", "c"]);
        t.e("b");
        // b was open before its preview: it stays. c was opened by it: it goes.
        t.keys(":b1<Enter>");
        let id = t.id();
        t.s.close_tab(id, 1).unwrap();
        t.s.open_preview(&t.path("b"), true);
        t.s.open_preview(&t.path("c"), true);
        assert_eq!(t.s.bufs.len(), 3);
        t.s.open_preview(&t.path("a"), true);
        t.s.open_preview(&t.path("b"), true);
        assert_eq!(t.s.bufs.len(), 2);
        assert_eq!(t.tabs(), ["a", "b"]);
        t.s.check_tabs();
    }

    #[test]
    fn a_replaced_preview_kept_open_is_not_the_alternate() {
        let mut t = T::new(&["a", "b", "c"]);
        // b is open before its preview, so replacing it keeps it, hidden.
        t.e("b");
        t.keys(":b1<Enter>");
        let id = t.id();
        t.s.close_tab(id, 1).unwrap();
        t.s.open_preview(&t.path("b"), true);
        t.s.open_preview(&t.path("c"), true);
        assert_eq!(t.tabs(), ["a", "c"]);
        assert_eq!(t.s.bufs.len(), 3);
        // Ctrl-^ never brings back a buffer that is not a tab: in the
        // GUI it goes back to a. (The terminal's is every buffer's.)
        assert_eq!(t.s.win().prev, None);
        t.s.gui = true;
        t.keys("<C-^>");
        assert_eq!(t.current(), "a");
        t.s.check_tabs();
    }

    #[test]
    fn closing_a_tab_in_another_window_keeps_ctrl_w_p() {
        let mut t = T::new(&["a", "b"]);
        t.e("b");
        let first = t.id();
        t.keys(":split<Enter>");
        let second = t.id();
        t.keys(":split<Enter>");
        // CTRL-W p goes back to the second window.
        assert_ne!(t.id(), second);
        t.s.close_tab(first, 1).unwrap();
        t.keys("<C-w>p");
        assert_eq!(t.id(), second);
        t.s.check_tabs();
    }

    #[test]
    fn bn_and_bp_follow_tabs_only_in_gui() {
        let mut t = T::new(&["a", "b", "c"]);
        t.e("b");
        t.e("c");
        let id = t.id();
        t.s.close_tab(id, 1).unwrap(); // b hidden; tabs a, c
        t.keys(":b1<Enter>");
        t.s.gui = true;
        t.keys(":bn<Enter>");
        assert_eq!(t.current(), "c");
        t.keys(":bn<Enter>");
        assert_eq!(t.current(), "a");
        t.keys(":bp<Enter>");
        assert_eq!(t.current(), "c");
        // The terminal: every buffer, as before.
        t.s.gui = false;
        t.keys(":b1<Enter>:bn<Enter>");
        assert_eq!(t.current(), "b");
        t.s.check_tabs();
    }

    #[test]
    fn move_tab_reorders() {
        let mut t = T::new(&["a", "b", "c"]);
        t.e("b");
        t.e("c");
        let id = t.id();
        t.s.move_tab(id, 2, 0).unwrap();
        assert_eq!(t.tabs(), ["c", "a", "b"]);
        t.s.move_tab(id, 0, 9).unwrap();
        assert_eq!(t.tabs(), ["a", "b", "c"]);
        assert_eq!(t.s.move_tab(id, 3, 0), Err("no such tab".into()));
        assert_eq!(t.current(), "c");
        t.s.check_tabs();
    }
}
