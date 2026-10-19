//! Session services for occur: the `*Occur*` buffer, and the buffer it
//! lists (searched again by `gr`, written to by occur-edit's `:w`).

use super::*;

impl Session {
    /// `:occur`: list this buffer's lines matching `pat` in `*Occur*`
    /// (reused, as Emacs's).
    pub(super) fn occur(&mut self, pat: &str, range: Option<(usize, usize)>) {
        let Some(src) = self
            .ed
            .path
            .as_deref()
            .and_then(|p| std::path::absolute(p).ok())
        else {
            return self.ed.set_err("occur: this buffer has no file");
        };
        let name = file_name(&src);
        let (o, lines) = match crate::occur::search(&self.ed.buf, src, &name, pat, range) {
            Ok(r) => r,
            Err(e) => return self.ed.set_err(e),
        };
        let at = (0..self.bufs.len()).find(|&i| self.ed_at(i).occur.is_some());
        match at {
            Some(i) => self.show(i),
            None => {
                let ed = make_editor(
                    Buffer::from_text(""),
                    self.ed.opts.for_new_buffer(),
                    &self.cfg,
                );
                let swap = self.swap_dir.join(format!("occur-{}", self.bufs.len()));
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
        }
        crate::occur::show(&mut self.ed, o, &lines, None);
    }

    /// `gr`: search the listed buffer again (dropping any edits).
    pub(super) fn occur_revert(&mut self) {
        let Some(o) = self.ed.occur.as_deref().cloned() else {
            return;
        };
        let i = match self.org_buffer(&o.source) {
            Ok(i) => i,
            Err(e) => return self.ed.set_err(e),
        };
        let name = file_name(&o.source);
        let found = self.org_with_buffer(i, |ed| {
            crate::occur::search(&ed.buf, o.source.clone(), &name, &o.pat, o.range)
        });
        match found {
            Ok((o, lines)) => {
                let line = self.ed.cur.line;
                crate::occur::show(&mut self.ed, o, &lines, Some(line));
            }
            Err(e) => self.ed.set_err(e),
        }
    }

    /// `:w` in occur-edit: put the changed lines into the listed buffer,
    /// as one undo step there (not saved, as Emacs).
    pub(super) fn occur_write(&mut self) {
        let plan = match crate::occur::changes(&self.ed) {
            Ok(p) => p,
            Err(e) => return self.ed.set_err(e),
        };
        let src = self
            .ed
            .occur
            .as_ref()
            .map(|o| o.source.clone())
            .unwrap_or_default();
        let i = match self.org_buffer(&src) {
            Ok(i) => i,
            Err(e) => return self.ed.set_err(e),
        };
        let r = self.org_with_buffer(i, |ed| {
            // All or nothing: the lines must still be what was listed.
            if let Some((l, ..)) = plan
                .iter()
                .find(|(l, old, _)| *l >= ed.line_count() || ed.buf.line(*l) != *old)
            {
                return Err(format!(
                    "occur: line {} changed since the search (gr searches again)",
                    l + 1
                ));
            }
            if plan.is_empty() {
                return Ok(());
            }
            ed.undo.begin((ed.cur.line, ed.cur.byte));
            for (l, _, new) in &plan {
                crate::vim::ops::splice_lines(ed, *l, 1, std::slice::from_ref(new));
            }
            ed.undo.end((plan[0].0, 0));
            Ok(())
        });
        if let Err(e) = r {
            return self.ed.set_err(e);
        }
        crate::occur::written(&mut self.ed);
        let name = file_name(&src);
        self.ed.set_msg(format!(
            "changed {} in {name}",
            crate::occur::plural(plan.len(), "line", "lines")
        ));
    }
}

/// The buffer's name, as Emacs's: the file's name.
fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
