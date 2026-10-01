//! One editing session: the editor plus its file, swap file and file effects.

use crate::buffer::{Buffer, LineEnding};
use crate::config::Config;
use crate::editor::Editor;
use crate::ex::ExEffect;
use crate::ex::addr::Range;
use crate::fileio::{self, FileStamp};
use crate::key::Key;
use crate::swap::{self, SwapInfo};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SWAP_IDLE: Duration = Duration::from_secs(1);
const SWAP_EDITS: usize = 200;

pub struct Session {
    pub ed: Editor,
    pub stamp: Option<FileStamp>,
    pub swap_dir: PathBuf,
    pub swap_path: PathBuf,
    /// Summary of the last successful write of the buffer's own file.
    pub written: Option<String>,
    pub quit: bool,
    /// A different file was loaded (`:e`); the caller resets per-file state.
    pub reloaded: bool,
    /// Another fred owns this file's swap; never write it.
    pub no_swap: bool,
    lossy: bool,
    cfg: Config,
    seen_version: u64,
    last_change: Option<Instant>,
    swapped_version: Option<u64>,
    swap_on_disk: bool,
    swap_error_shown: bool,
}

fn make_editor(buf: Buffer, cfg: &Config) -> Editor {
    let mut ed = Editor::new(buf);
    ed.tabstop = cfg.tabstop;
    ed.autocomplete = cfg.autocomplete;
    ed.win_height = cfg.height;
    ed
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(p),
    }
}

fn human_size(n: usize) -> String {
    match n {
        n if n < 1024 => format!("{n}B"),
        n if n < 1024 * 1024 => format!("{:.1}K", n as f64 / 1024.0),
        n => format!("{:.1}M", n as f64 / (1024.0 * 1024.0)),
    }
}

/// `"name" 12L, 340B written`
pub fn summary(name: &Path, data: &[u8]) -> String {
    let mut lines = data.iter().filter(|&&b| b == b'\n').count();
    if !data.is_empty() && !data.ends_with(b"\n") {
        lines += 1;
    }
    format!(
        "\"{}\" {lines}L, {} written",
        name.display(),
        human_size(data.len())
    )
}

struct Opened {
    ed: Editor,
    stamp: Option<FileStamp>,
    lossy: bool,
}

fn open_file(path: Option<&Path>, cfg: &Config) -> Result<Opened, String> {
    let Some(p) = path else {
        let buf = Buffer::from_text("");
        return Ok(Opened {
            ed: make_editor(buf, cfg),
            stamp: None,
            lossy: false,
        });
    };
    let l = fileio::load(p)?;
    let lossy = l.notice.as_deref() == Some("not valid UTF-8");
    let (lines, bytes) = (l.buf.len_lines(), l.buf.len_bytes());
    let mut ed = make_editor(l.buf, cfg);
    ed.path = Some(p.to_path_buf());
    ed.readonly = l.readonly;
    let name = p.display();
    match l.notice.as_deref() {
        Some("[new]") => ed.set_msg(format!("\"{name}\" [new]")),
        Some(n) => ed.set_msg(format!("\"{name}\" {n}")),
        None => ed.set_msg(format!("\"{name}\" {lines}L, {}", human_size(bytes))),
    }
    Ok(Opened {
        ed,
        stamp: l.stamp,
        lossy,
    })
}

impl Session {
    /// Open `path` (or an unnamed buffer). Returns a leftover swap file, if any.
    pub fn open(
        path: Option<PathBuf>,
        cfg: &Config,
        swap_dir: &Path,
    ) -> Result<(Session, Option<SwapInfo>), String> {
        let o = open_file(path.as_deref(), cfg)?;
        let swap_path = swap::swap_path_in(swap_dir, path.as_deref());
        let mut s = Session {
            seen_version: o.ed.buf.version,
            ed: o.ed,
            stamp: o.stamp,
            swap_dir: swap_dir.to_path_buf(),
            swap_path,
            written: None,
            quit: false,
            reloaded: false,
            no_swap: false,
            lossy: o.lossy,
            cfg: cfg.clone(),
            last_change: None,
            swapped_version: None,
            swap_on_disk: false,
            swap_error_shown: false,
        };
        let info = s.leftover_swap();
        Ok((s, info))
    }

    fn leftover_swap(&mut self) -> Option<SwapInfo> {
        if !self.swap_path.exists() {
            return None;
        }
        let Ok(info) = swap::read(&self.swap_path) else {
            return None;
        };
        // A swap identical to the file holds nothing to recover.
        if !swap::owner_alive(&info) && info.text.as_bytes() == self.ed.buf.to_bytes() {
            swap::remove(&self.swap_path);
            return None;
        }
        Some(info)
    }

    pub fn handle_key(&mut self, k: Key) {
        self.ed.handle_key(k);
        if let Some(eff) = self.ed.pending_effect.take() {
            self.perform(eff);
        }
    }

    pub fn perform(&mut self, eff: ExEffect) {
        match eff {
            ExEffect::None => {}
            ExEffect::Write {
                path,
                force,
                range,
                then_quit,
            } => {
                if self.write(path, force, range) && then_quit {
                    if self.ed.buf.modified {
                        // Wrote a copy; the buffer itself is still unsaved.
                        self.ed.set_err("unsaved changes (q! to discard, wq to save)");
                    } else {
                        self.quit = true;
                    }
                }
            }
            ExEffect::WriteIfModifiedQuit => {
                if !self.ed.buf.modified || self.write(None, false, None) {
                    self.quit = true;
                }
            }
            ExEffect::Quit { force } => {
                if self.ed.buf.modified && !force {
                    self.ed
                        .set_err("unsaved changes (q! to discard, wq to save)");
                } else {
                    self.quit = true;
                }
            }
            ExEffect::Edit { path, force } => self.edit(&path, force),
        }
    }

    fn write(&mut self, path: Option<String>, force: bool, range: Option<Range>) -> bool {
        if self.lossy {
            self.ed
                .set_err("file is not valid UTF-8; writing it would change it");
            return false;
        }
        let target = path.as_deref().map(expand_tilde);
        match (target, range) {
            (None, Some(_)) => {
                self.ed.set_err("partial write needs a file name");
                false
            }
            (None, None) => match self.ed.path.clone() {
                Some(p) => self.write_own(&p, force),
                None => {
                    self.ed.set_err("no file name (use :w name)");
                    false
                }
            },
            (Some(t), None) if self.ed.path.is_none() => {
                self.ed.path = Some(t.clone());
                self.stamp = None;
                let ok = self.write_own(&t, force);
                if ok {
                    swap::remove(&self.swap_path);
                    self.swap_path = swap::swap_path_in(&self.swap_dir, Some(&t));
                } else {
                    self.ed.path = None;
                }
                ok
            }
            (Some(t), None) if self.ed.path.as_deref() == Some(t.as_path()) => {
                self.write_own(&t, force)
            }
            (Some(t), range) => {
                let data = match range {
                    Some(r) => self.range_bytes(r),
                    None => self.ed.buf.to_bytes(),
                };
                match fileio::write(&t, &data, None, force) {
                    Ok(_) => {
                        self.ed.set_msg(summary(&t, &data));
                        true
                    }
                    Err(e) => {
                        self.ed.set_err(e);
                        false
                    }
                }
            }
        }
    }

    fn range_bytes(&self, r: Range) -> Vec<u8> {
        let nl = self.ed.buf.line_ending.as_str();
        let mut s = String::new();
        for l in r.start..=r.end {
            s.push_str(&self.ed.buf.line(l));
            s.push_str(nl);
        }
        s.into_bytes()
    }

    fn write_own(&mut self, p: &Path, force: bool) -> bool {
        if self.ed.readonly && !force {
            self.ed.set_err("read-only (w! to override)");
            return false;
        }
        let data = self.ed.buf.to_bytes();
        match fileio::write(p, &data, self.stamp.as_ref(), force) {
            Ok(st) => {
                self.stamp = Some(st);
                self.ed.mark_saved();
                let msg = summary(p, &data);
                self.ed.set_msg(msg.clone());
                self.written = Some(msg);
                self.remove_swap();
                true
            }
            Err(e) => {
                self.ed.set_err(e);
                false
            }
        }
    }

    fn edit(&mut self, path: &str, force: bool) {
        if self.ed.buf.modified && !force {
            self.ed.set_err("unsaved changes (e! to discard)");
            return;
        }
        let p = expand_tilde(path);
        match open_file(Some(&p), &self.cfg) {
            Ok(o) => {
                self.remove_swap();
                self.ed = o.ed;
                self.stamp = o.stamp;
                self.lossy = o.lossy;
                self.no_swap = false;
                self.seen_version = self.ed.buf.version;
                self.swapped_version = None;
                self.swap_path = swap::swap_path_in(&self.swap_dir, Some(&p));
                self.reloaded = true;
                if self.swap_path.exists() {
                    self.no_swap = true;
                    self.ed.set_err(
                        "a swap file exists for this file; reopen it with fred to recover",
                    );
                }
            }
            Err(e) => self.ed.set_err(format!("{}: {e}", p.display())),
        }
    }

    /// Replace the buffer with a swap file's text.
    pub fn recover(&mut self, info: SwapInfo) {
        let mut ed = make_editor(Buffer::from_text(&info.text), &self.cfg);
        ed.path = self.ed.path.clone();
        ed.readonly = self.ed.readonly;
        ed.saved_state = u64::MAX;
        ed.buf.modified = true;
        ed.set_msg("recovered unsaved changes; :w to save them");
        self.ed = ed;
        self.swap_on_disk = true;
    }

    /// Delete a leftover swap file the user chose not to recover.
    pub fn discard_swap(&mut self) {
        swap::remove(&self.swap_path);
    }

    /// Write the swap file when there are unsaved edits and the user paused
    /// (or made many edits); remove it once the buffer is saved.
    pub fn maybe_swap(&mut self, now: Instant) {
        let v = self.ed.buf.version;
        if v != self.seen_version {
            self.seen_version = v;
            self.last_change = Some(now);
        }
        if self.no_swap {
            return;
        }
        if !self.ed.buf.modified {
            self.remove_swap();
            return;
        }
        if self.swapped_version == Some(v) {
            return;
        }
        let idle = self
            .last_change
            .is_some_and(|t| now.duration_since(t) >= SWAP_IDLE);
        if idle || self.ed.buf.edits_since_swap >= SWAP_EDITS || self.last_change.is_none() {
            self.write_swap();
        }
    }

    /// Write the swap now if there is anything unsaved (crash, signal).
    pub fn write_swap(&mut self) {
        if self.no_swap || !self.ed.buf.modified {
            return;
        }
        let text = String::from_utf8_lossy(&self.ed.buf.to_bytes()).into_owned();
        match swap::write(&self.swap_path, self.ed.path.as_deref(), &text) {
            Ok(()) => {
                self.swap_on_disk = true;
                self.swapped_version = Some(self.ed.buf.version);
                self.ed.buf.edits_since_swap = 0;
            }
            Err(e) => {
                if !self.swap_error_shown {
                    self.swap_error_shown = true;
                    self.ed.set_err(e);
                }
            }
        }
    }

    fn remove_swap(&mut self) {
        if self.swap_on_disk {
            swap::remove(&self.swap_path);
            self.swap_on_disk = false;
        }
        self.swapped_version = None;
    }

    /// Normal exit: the swap file is no longer needed.
    pub fn cleanup(&mut self) {
        if !self.no_swap {
            self.remove_swap();
            swap::remove(&self.swap_path);
        }
    }

    /// `+LINE` on the command line.
    pub fn goto_line(&mut self, line: Option<usize>) {
        let n = self.ed.line_count();
        let l = line.map_or(n - 1, |l| l.clamp(1, n) - 1);
        let b = self.ed.first_nonblank(l);
        self.ed.set_cursor(l, b);
    }

    pub fn line_ending_name(&self) -> &'static str {
        match self.ed.buf.line_ending {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::key::parse_keys;
    use std::fs;
    use std::time::{Duration, Instant};

    struct T {
        dir: tempfile::TempDir,
        s: Session,
    }

    impl T {
        fn open(name: Option<&str>, content: Option<&str>) -> T {
            let dir = tempfile::tempdir().unwrap();
            let path = name.map(|n| dir.path().join(n));
            if let (Some(p), Some(c)) = (&path, content) {
                fs::write(p, c).unwrap();
            }
            let (s, _) = Session::open(path, &Config::default(), &dir.path().join("swap")).unwrap();
            T { dir, s }
        }
        fn keys(&mut self, k: &str) {
            for key in parse_keys(k) {
                self.s.handle_key(key);
            }
        }
        fn file(&self, name: &str) -> String {
            fs::read_to_string(self.dir.path().join(name)).unwrap()
        }
        fn msg(&self) -> String {
            self.s.ed.msg.clone().map(|m| m.0).unwrap_or_default()
        }
    }

    #[test]
    fn new_file_notice_and_write() {
        let mut t = T::open(Some("f.txt"), None);
        assert!(t.msg().contains("[new]"), "{}", t.msg());
        t.keys("ihi<Esc>:w<Enter>");
        assert_eq!(t.file("f.txt"), "hi\n");
        assert!(t.msg().ends_with("1L, 3B written"), "{}", t.msg());
        assert!(!t.s.ed.buf.modified);
        assert!(t.s.written.is_some());
    }

    #[test]
    fn write_detects_external_change() {
        let mut t = T::open(Some("f"), Some("a\n"));
        fs::write(t.dir.path().join("f"), "theirs\n").unwrap();
        t.keys("x:w<Enter>");
        assert!(t.msg().contains("changed on disk"), "{}", t.msg());
        assert_eq!(t.file("f"), "theirs\n");
        t.keys(":w!<Enter>");
        assert_eq!(t.file("f"), "\n");
    }

    #[test]
    fn quit_rules() {
        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys("x:q<Enter>");
        assert!(!t.s.quit);
        assert!(t.msg().contains("unsaved changes"));
        t.keys(":q!<Enter>");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "abc\n");

        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys("x:wq<Enter>");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "bc\n");

        let mut t = T::open(Some("f"), Some("abc\n"));
        t.keys(":x<Enter>");
        assert!(t.s.quit);
        assert!(
            t.s.written.is_none(),
            ":x on an unmodified buffer doesn't write"
        );
    }

    #[test]
    fn wq_to_another_file_keeps_unsaved_work() {
        let mut t = T::open(Some("f"), Some("one\ntwo\n"));
        let part = t.dir.path().join("part");
        let other = t.dir.path().join("other");
        t.keys(&format!("GoNEW WORK<Esc>:1,2wq {}<Enter>", part.display()));
        assert!(!t.s.quit, "the buffer still has unsaved changes");
        assert!(t.msg().contains("unsaved changes"), "{}", t.msg());
        assert_eq!(t.file("part"), "one\ntwo\n");
        t.keys(&format!(":wq {}<Enter>", other.display()));
        assert!(!t.s.quit);
        assert_eq!(t.file("other"), "one\ntwo\nNEW WORK\n");
        t.keys(":wq<Enter>");
        assert!(t.s.quit);
        assert_eq!(t.file("f"), "one\ntwo\nNEW WORK\n");
    }

    #[test]
    fn write_copy_and_partial() {
        let mut t = T::open(Some("f"), Some("one\ntwo\nthree\n"));
        let copy = t.dir.path().join("copy");
        let part = t.dir.path().join("part");
        t.keys(&format!("x:w {}<Enter>", copy.display()));
        assert_eq!(t.file("copy"), "ne\ntwo\nthree\n");
        assert!(
            t.s.ed.buf.modified,
            "writing a copy doesn't save the buffer"
        );
        t.keys(&format!(":w {}<Enter>", copy.display()));
        assert!(t.msg().contains("file exists"), "{}", t.msg());
        t.keys(&format!(":2,3w {}<Enter>", part.display()));
        assert_eq!(t.file("part"), "two\nthree\n");
        t.keys(":2,3w<Enter>");
        assert!(t.msg().contains("file name"), "{}", t.msg());
    }

    #[test]
    fn unnamed_buffer() {
        let mut t = T::open(None, None);
        t.keys("ihello<Esc>:w<Enter>");
        assert!(t.msg().contains("no file name"), "{}", t.msg());
        let p = t.dir.path().join("named.txt");
        t.keys(&format!(":w {}<Enter>", p.display()));
        assert_eq!(fs::read_to_string(&p).unwrap(), "hello\n");
        assert_eq!(t.s.ed.path.as_deref(), Some(p.as_path()));
        assert!(!t.s.ed.buf.modified);
    }

    #[test]
    fn edit_other_file() {
        let mut t = T::open(Some("a"), Some("aaa\n"));
        fs::write(t.dir.path().join("b"), "bbb\n").unwrap();
        let b = t.dir.path().join("b");
        t.keys(&format!("x:e {}<Enter>", b.display()));
        assert!(t.msg().contains("unsaved changes"), "{}", t.msg());
        t.keys(&format!(":e! {}<Enter>", b.display()));
        assert_eq!(t.s.ed.buf.text(), "bbb");
        assert!(t.s.reloaded);
    }

    #[test]
    fn invalid_utf8_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bin");
        fs::write(&p, [b'a', 0xff, b'\n']).unwrap();
        let (mut s, _) = Session::open(
            Some(p.clone()),
            &Config::default(),
            &dir.path().join("swap"),
        )
        .unwrap();
        assert!(s.ed.readonly);
        for k in parse_keys(":w!<Enter>") {
            s.handle_key(k);
        }
        assert_eq!(fs::read(&p).unwrap(), [b'a', 0xff, b'\n']);
        assert!(s.ed.msg.unwrap().1);
    }

    #[test]
    fn swap_written_after_idle_and_removed_on_save() {
        let mut t = T::open(Some("f"), Some("a\n"));
        let sp = t.s.swap_path.clone();
        let now = Instant::now();
        t.keys("ixyz<Esc>");
        t.s.maybe_swap(now);
        assert!(!sp.exists(), "not yet idle");
        t.s.maybe_swap(now + Duration::from_millis(1100));
        let info = crate::swap::read(&sp).unwrap();
        assert_eq!(info.text, "xyza\n");
        t.keys(":w<Enter>");
        t.s.maybe_swap(now + Duration::from_secs(3));
        assert!(!sp.exists());
    }

    #[test]
    fn swap_written_after_many_edits() {
        let mut t = T::open(Some("f"), Some("a\n"));
        t.keys(&"ix<Esc>".repeat(200));
        t.s.maybe_swap(Instant::now());
        assert!(t.s.swap_path.exists());
    }

    #[test]
    fn stale_swap_is_offered_and_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, "old\n").unwrap();
        let swap_dir = dir.path().join("swap");
        let sp = crate::swap::swap_path_in(&swap_dir, Some(&p));
        crate::swap::write(&sp, Some(&p), "recovered\r\n").unwrap();
        // Pretend the owner was a dead process.
        let text = fs::read_to_string(&sp).unwrap().replacen(
            &format!("\"pid\":{}", std::process::id()),
            "\"pid\":999999",
            1,
        );
        fs::write(&sp, text).unwrap();
        let (mut s, info) = Session::open(Some(p.clone()), &Config::default(), &swap_dir).unwrap();
        let info = info.expect("swap offered");
        assert!(!crate::swap::owner_alive(&info));
        s.recover(info);
        assert!(s.ed.buf.modified);
        for k in parse_keys(":w<Enter>") {
            s.handle_key(k);
        }
        assert_eq!(fs::read_to_string(&p).unwrap(), "recovered\r\n");
    }
}
