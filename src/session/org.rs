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
        let swap = self
            .swap_dir
            .join(format!("org-special-{}", self.bufs.len()));
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
    pub fn org_with_file<R>(
        &mut self,
        path: &Path,
        f: impl FnOnce(&mut Editor) -> R,
    ) -> Result<R, String> {
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
        if ok {
            Ok(())
        } else {
            Err(self.ed.msg.clone().map(|m| m.0).unwrap_or_default())
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::parse_keys;

    struct T {
        dir: tempfile::TempDir,
        s: Session,
    }

    impl T {
        fn new(files: &[(&str, &str)], open: &str) -> T {
            let dir = tempfile::tempdir().unwrap();
            for (n, c) in files {
                std::fs::write(dir.path().join(n), c).unwrap();
            }
            let mut cfg = Config::default();
            cfg.org.0.insert(
                "org-directory".into(),
                toml::Value::String(dir.path().display().to_string()),
            );
            let (s, _) =
                Session::open(Some(dir.path().join(open)), &cfg, &dir.path().join("swap")).unwrap();
            T { dir, s }
        }
        fn keys(&mut self, k: &str) {
            for key in parse_keys(k) {
                self.s.handle_key(key);
            }
        }
        fn file(&self, n: &str) -> String {
            std::fs::read_to_string(self.dir.path().join(n)).unwrap_or_default()
        }
        fn text_of(&mut self, n: &str) -> String {
            let p = self.dir.path().join(n);
            self.s.org_text(&p).unwrap()
        }
    }

    #[test]
    fn refile_between_files() {
        let mut t = T::new(
            &[
                ("a.org", "* Move me\nbody\n* Stay"),
                ("b.org", "* Target\n** Old"),
            ],
            "a.org",
        );
        let b = t.dir.path().join("b.org").display().to_string();
        crate::org::options::put(
            "org-refile-targets",
            toml::Value::String(format!("'((\"{b}\" :maxlevel . 2))")),
        );
        t.keys("<C-c><C-w>Target<Enter>");
        assert_eq!(t.s.ed.buf.text(), "* Stay", "{:?}", t.s.ed.msg);
        assert_eq!(t.text_of("b.org"), "* Target\n** Old\n** Move me\nbody");
        crate::org::options::put("org-refile-targets", toml::Value::Boolean(false));
    }

    #[test]
    fn refile_within_file_default_targets() {
        crate::org::options::put("org-refile-targets", toml::Value::Boolean(false));
        let mut t = T::new(&[("a.org", "* A\n** x\n* B")], "a.org");
        t.keys("j<C-c><C-w>B<Enter>");
        assert_eq!(t.s.ed.buf.text(), "* A\n* B\n** x");
    }

    #[test]
    fn archive_to_file_with_context() {
        crate::org::set_now(Some(1_780_000_000));
        let mut t = T::new(&[("a.org", "* P :p:\n** DONE Old\n* Q")], "a.org");
        t.keys("j<C-c><C-x><C-a>");
        assert_eq!(t.s.ed.buf.text(), "* P :p:\n* Q", "{:?}", t.s.ed.msg);
        let arch = t.file("a.org_archive");
        assert!(arch.starts_with("\nArchived entries from file "), "{arch}");
        assert!(
            arch.contains("* DONE Old\n:PROPERTIES:\n:ARCHIVE_TIME:"),
            "{arch}"
        );
        assert!(arch.contains(":ARCHIVE_OLPATH: P\n"), "{arch}");
        assert!(arch.contains(":ARCHIVE_ITAGS: p\n"), "{arch}");
        assert!(arch.contains(":ARCHIVE_TODO: DONE\n"), "{arch}");
        crate::org::set_now(None);
    }

    #[test]
    fn capture_into_notes_headline_and_finish() {
        crate::org::set_now(Some(1_780_000_000));
        let mut t = T::new(
            &[("notes.org", "* Tasks\n* Other"), ("src.txt", "hello")],
            "src.txt",
        );
        crate::org::options::put(
            "org-capture-templates",
            toml::Value::String("'((\"w\" \"Work todo\" entry (file+headline org-default-notes-file \"Tasks\") \"* TICKET %?\\nEntered on %U\\n** Description\\n** Notes\\n** Resolution\"))".into()),
        );
        let notes = t.dir.path().join("notes.org").display().to_string();
        crate::org::options::put("org-default-notes-file", toml::Value::String(notes));
        t.keys(" ocw");
        assert_eq!(t.s.ed.cur.line, 1, "{:?}", t.s.ed.msg);
        t.keys("Fix it<C-c><C-c>");
        let text = t.file("notes.org");
        let ts = crate::org::timestamp(1_780_000_000, true, true);
        assert_eq!(
            text,
            format!(
                "* Tasks\n** TICKET Fix it\nEntered on {ts}\n*** Description\n*** Notes\n*** Resolution\n* Other"
            )
        );
        assert_eq!(
            t.s.ed.path.as_ref().unwrap().file_name().unwrap(),
            "src.txt"
        );
        crate::org::options::put("org-capture-templates", toml::Value::Boolean(false));
        crate::org::set_now(None);
    }

    #[test]
    fn edit_src_block_in_a_dedicated_buffer() {
        let mut t = T::new(
            &[(
                "a.org",
                "* H\n#+begin_src python\n  print(1)\n  * star\n#+end_src\n",
            )],
            "a.org",
        );
        t.keys("jj<C-c>'");
        assert_eq!(
            t.s.ed.org_buffer_name.as_deref(),
            Some("*Org Src a.org[ python ]*")
        );
        assert_eq!(t.s.ed.buf.text(), "print(1)\n* star");
        assert_eq!(t.s.ed.cur.line, 0);
        assert!(t.s.ed.org.is_none());
        t.keys("A + 1<Esc><C-c>'");
        assert_eq!(
            t.s.ed.buf.text(),
            "* H\n#+begin_src python\n  print(1) + 1\n  ,* star\n#+end_src"
        );
        assert!(t.s.ed.org_buffer_name.is_none());
        // Abort leaves the block alone.
        t.keys("jj<C-c>'Ox<Esc><C-c><C-k>");
        assert_eq!(t.s.ed.buf.line(2), "  print(1) + 1");
    }
}
