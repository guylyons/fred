//! Cmd keys from the app's menu bar. What each does depends on the mode.

use crate::editor::Mode;
use crate::session::Session;

fn keys(s: &mut Session, k: &str) {
    for key in crate::key::parse_keys(k) {
        s.handle_key(key);
    }
}

/// Yank or delete, and always reach the system clipboard (even with
/// `clipboard = false`, where plain y/d leave it alone).
fn copy(s: &mut Session, k: &str) {
    keys(s, k);
    if !s.ed.clipboard() {
        crate::clipboard::set(&s.ed.reg);
    }
}

/// An ex command that leaves the mode alone (so it works from Insert).
fn ex(s: &mut Session, cmd: &str) {
    s.ed.run_ex(cmd);
    if let Some(eff) = s.ed.pending_effect.take() {
        s.perform(eff);
    }
}

pub fn mac_key(s: &mut Session, key: char, shift: bool) {
    if !"szcxawf".contains(key) {
        return;
    }
    let insert = s.ed.mode == Mode::Insert;
    let visual = matches!(s.ed.mode, Mode::Visual { .. } | Mode::VisualLine { .. });
    // Copy, cut and select-all type Visual keys, which Select would type over.
    if matches!(key, 'c' | 'x' | 'a') {
        s.ed.vim.select = false;
    }
    // Save and undo/redo keep the mode; the rest start from Normal.
    match key {
        's' if visual => return ex(s, "w"),
        // Saving inside the open undo group would leave the buffer marked
        // modified when Esc closes it, so close it, save, and come back.
        's' if insert => {
            let before = s.ed.cur.byte;
            keys(s, "<Esc>");
            let moved = s.ed.cur.byte < before;
            ex(s, "w");
            return keys(s, if moved { "a" } else { "i" });
        }
        // Esc closes the open undo group; then undo and go back to Insert.
        'z' if insert => {
            keys(s, if shift { "<Esc><C-r>i" } else { "<Esc>ui" });
            return;
        }
        'c' | 'x' if insert => return,
        'c' if visual => return copy(s, "y"),
        'x' if visual => return copy(s, "d"),
        'a' if visual => return keys(s, "ggVG"),
        _ => {}
    }
    if s.ed.mode != Mode::Normal {
        // A selection begun in Insert would go back there on Esc, and the
        // keys below would be typed as text.
        s.ed.vim.restart_insert = false;
        keys(s, "<Esc>");
    }
    // Cmd-W closes the tab, as in Sublime.
    if key == 'w' {
        let (id, b) = (s.active, s.win().buf);
        let i = s.win().tabs.iter().position(|&t| t == b).unwrap_or(0);
        if let Err(e) = s.close_tab(id, i) {
            s.ed.set_err(e);
        }
        return;
    }
    keys(
        s,
        match (key, shift) {
            ('s', _) => ":w<Enter>",
            ('z', false) => "u",
            ('z', true) => "<C-r>",
            ('c', _) => return copy(s, "yy"),
            ('x', _) => return copy(s, "dd"),
            ('a', _) => "ggVG",
            _ => "/",
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn open(text: &str) -> (Session, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("f"), text).unwrap();
        let (s, _) = Session::open(
            Some(d.path().join("f")),
            &Config::default(),
            &d.path().join("swap"),
        )
        .unwrap();
        (s, d)
    }

    #[test]
    fn cmd_s_saves_without_leaving_insert() {
        let (mut s, d) = open("one\n");
        s.handle_keys("Ahi");
        mac_key(&mut s, 's', false);
        assert_eq!(s.ed.mode, Mode::Insert);
        assert_eq!(
            std::fs::read_to_string(d.path().join("f")).unwrap(),
            "onehi\n"
        );
    }

    #[test]
    fn cmd_z_and_cmd_shift_z_undo_and_redo_in_normal() {
        let (mut s, _d) = open("one\n");
        s.handle_keys("xx");
        mac_key(&mut s, 'z', false);
        assert_eq!(s.ed.buf.line(0).trim_end(), "ne");
        mac_key(&mut s, 'z', true);
        assert_eq!(s.ed.buf.line(0).trim_end(), "e");
    }

    #[test]
    fn cmd_z_in_insert_undoes_and_stays() {
        let (mut s, _d) = open("one\n");
        s.handle_keys("xi");
        mac_key(&mut s, 'z', false);
        assert_eq!(s.ed.mode, Mode::Insert);
        assert_eq!(s.ed.buf.line(0).trim_end(), "one");
    }

    #[test]
    fn cmd_z_in_insert_undoes_typed_text_and_redo_restores_it() {
        let (mut s, _d) = open("one\n");
        s.handle_keys("x");
        s.handle_keys("Ahi");
        mac_key(&mut s, 'z', false);
        assert_eq!(s.ed.mode, Mode::Insert);
        assert_eq!(s.ed.buf.line(0).trim_end(), "ne");
        mac_key(&mut s, 'z', true);
        assert_eq!(s.ed.mode, Mode::Insert);
        assert_eq!(s.ed.buf.line(0).trim_end(), "nehi");
    }

    #[test]
    fn cmd_s_in_insert_then_esc_stays_unmodified() {
        let (mut s, _d) = open("one\n");
        s.handle_keys("Ahi");
        mac_key(&mut s, 's', false);
        s.handle_keys("<Esc>");
        assert!(!s.ed.buf.modified);
    }

    #[test]
    fn cmd_c_and_x_reach_the_clipboard_with_clipboard_off() {
        let (mut s, _d) = open("a\nb\n");
        s.handle_keys(":set clipboard=<Enter>");
        assert!(!s.ed.clipboard());
        mac_key(&mut s, 'c', false);
        assert_eq!(crate::clipboard::get().as_deref(), Some("a\n"));
        mac_key(&mut s, 'x', false);
        assert_eq!(s.ed.buf.line(0).trim_end(), "b");
    }

    #[test]
    fn cmd_c_in_visual_copies_the_selection_and_leaves_visual() {
        let (mut s, _d) = open("hello world\n");
        s.handle_keys("vee");
        mac_key(&mut s, 'c', false);
        assert_eq!(s.ed.mode, Mode::Normal);
        assert_eq!(crate::clipboard::get().as_deref(), Some("hello world"));
    }

    #[test]
    fn cmd_x_in_normal_cuts_the_line() {
        let (mut s, _d) = open("a\nb\n");
        mac_key(&mut s, 'x', false);
        assert_eq!(s.ed.buf.line(0).trim_end(), "b");
        assert_eq!(crate::clipboard::get().as_deref(), Some("a\n"));
    }

    #[test]
    fn cmd_a_selects_everything() {
        let (mut s, _d) = open("a\nb\nc\n");
        mac_key(&mut s, 'a', false);
        assert!(matches!(s.ed.mode, Mode::VisualLine { .. }));
        assert_eq!(s.ed.visual_lines(), Some((0, 2)));
    }

    #[test]
    fn cmd_f_opens_search() {
        let (mut s, _d) = open("a\n");
        s.handle_keys("i");
        mac_key(&mut s, 'f', false);
        assert!(matches!(s.ed.mode, Mode::Command(ref cl) if cl.kind == '/'));
    }

    #[test]
    fn cmd_c_x_and_v_work_in_select_mode() {
        let (mut s, _d) = open("hello world\n");
        s.handle_keys("gh<Right><Right><Right><Right>");
        mac_key(&mut s, 'c', false);
        assert_eq!(s.ed.mode, Mode::Normal);
        assert_eq!(crate::clipboard::get().as_deref(), Some("hello"));
        s.handle_keys("wgh<Right>");
        mac_key(&mut s, 'x', false);
        assert_eq!(s.ed.buf.line(0).trim_end(), "hello rld");
        // Cmd-V replaces the selection and leaves the registers alone.
        s.handle_keys("0gh<Right>");
        s.ed.paste("HE");
        assert_eq!(s.ed.buf.line(0).trim_end(), "HEllo rld");
        assert_eq!(s.ed.mode, Mode::Normal);
        assert_eq!(s.ed.reg.text, "wo");
        // Cmd-A from Select selects everything, without typing over it.
        s.handle_keys("gh");
        mac_key(&mut s, 'a', false);
        assert_eq!(s.ed.buf.line(0).trim_end(), "HEllo rld");
        assert_eq!(s.ed.visual_lines(), Some((0, 0)));
    }

    /// A selection begun in Insert: by Shift+arrow, or (`drag`) as the
    /// mouse starts one (`select::start` from Insert, what a drag calls).
    fn insert_selection(drag: bool) -> (Session, tempfile::TempDir) {
        let (mut s, d) = open("hello world\n");
        s.handle_keys(":set km=startsel,stopsel slm=key,mouse<Enter>i");
        if drag {
            crate::vim::select::start(&mut s.ed, (0, 0), false, true);
            s.ed.set_cursor(0, 1);
        } else {
            s.handle_keys("<S-Right>");
        }
        assert!(s.ed.select_mode());
        (s, d)
    }

    #[test]
    fn cmd_keys_after_a_selection_begun_in_insert_type_nothing() {
        for drag in [false, true] {
            let (mut s, _d) = insert_selection(drag);
            mac_key(&mut s, 'z', false);
            assert_eq!(
                s.ed.buf.line(0).trim_end(),
                "hello world",
                "Cmd-Z, drag {drag}"
            );
            assert_ne!(s.ed.mode, Mode::Insert);
            let (mut s, _d) = insert_selection(drag);
            mac_key(&mut s, 'z', true);
            assert_eq!(
                s.ed.buf.line(0).trim_end(),
                "hello world",
                "Cmd-Shift-Z, drag {drag}"
            );
            let (mut s, _d) = insert_selection(drag);
            mac_key(&mut s, 'f', false);
            assert!(
                matches!(s.ed.mode, Mode::Command(ref cl) if cl.kind == '/'),
                "Cmd-F, drag {drag}"
            );
            assert_eq!(s.ed.buf.line(0).trim_end(), "hello world");
            // Cmd-W closes the tab: the buffer, hidden, is untouched.
            let (mut s, _d) = insert_selection(drag);
            mac_key(&mut s, 'w', false);
            assert_eq!(
                s.ed_at(0).buf.line(0).trim_end(),
                "hello world",
                "Cmd-W, drag {drag}"
            );
            assert_ne!(s.ed.mode, Mode::Insert);
        }
    }

    #[test]
    fn select_replace_and_delete_leave_the_system_clipboard_alone() {
        for keys in ["x", "<BS>", "<Del>"] {
            let (mut s, _d) = open("hello world\n");
            s.handle_keys(":set clipboard=unnamedplus<Enter>");
            crate::clipboard::set(&crate::editor::Register {
                text: "copied".into(),
                linewise: false,
                block: false,
            });
            s.handle_keys(&format!("gh<Right>{keys}"));
            assert_eq!(crate::clipboard::get().as_deref(), Some("copied"), "{keys}");
            // The unnamed register still gets the text, as in Neovim.
            assert_eq!(s.ed.reg.text, "he", "{keys}");
        }
    }
}
