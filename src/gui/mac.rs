//! Cmd keys from the app's menu bar. What each does depends on the mode.

use crate::editor::Mode;
use crate::session::Session;

fn keys(s: &mut Session, k: &str) {
    for key in crate::key::parse_keys(k) {
        s.handle_key(key);
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
    // Save and undo/redo keep the mode; the rest start from Normal.
    match key {
        's' if insert || visual => return ex(s, "w"),
        'z' if insert => {
            let p = if shift {
                s.ed.undo.redo(&mut s.ed.buf)
            } else {
                s.ed.undo.undo(&mut s.ed.buf)
            };
            if let Some(p) = p {
                s.ed.set_cursor(p.0, p.1);
            }
            return;
        }
        'c' | 'x' if insert => return,
        'c' if visual => return keys(s, "y"),
        'x' if visual => return keys(s, "d"),
        'a' if visual => return keys(s, "ggVG"),
        _ => {}
    }
    if s.ed.mode != Mode::Normal {
        keys(s, "<Esc>");
    }
    keys(
        s,
        match (key, shift) {
            ('s', _) => ":w<Enter>",
            ('z', false) => "u",
            ('z', true) => "<C-r>",
            ('c', _) => "yy",
            ('x', _) => "dd",
            ('a', _) => "ggVG",
            ('w', _) => ":q<Enter>",
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
}
