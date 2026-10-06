//! org-goto.el: look up a location in the current file, keeping its
//! visibility. The `outline` interface browses a read-only copy of the
//! buffer (`*org-goto*`, in OVERVIEW) with org-goto-map's keys; the
//! `outline-path-completion` interface completes outline paths.
//!
//! Adaptation: with org-goto-auto-isearch (the default) typed letters stay
//! Vim keys there, so `/` searches (Emacs starts an isearch); with it nil
//! the n/p/f/b/u/q keys of org-goto-map apply. The help text of
//! `*Org Help*` is shown as a message.

use super::{Prefix, fold, links, options, syntax};
use crate::editor::Editor;
use crate::key::Key;
use std::cell::Cell;

/// org-goto-map's keys (always).
pub const MAP: &[(&str, &str)] = &[
    ("RET", "org-goto-ret"),
    ("<left>", "org-goto-left"),
    ("<right>", "org-goto-right"),
    ("C-g", "org-goto-quit"),
    ("TAB", "org-cycle"),
    ("<down>", "outline-next-visible-heading"),
    ("<up>", "outline-previous-visible-heading"),
    ("C-c C-n", "outline-next-visible-heading"),
    ("C-c C-p", "outline-previous-visible-heading"),
    ("C-c C-f", "outline-forward-same-level"),
    ("C-c C-b", "outline-backward-same-level"),
    ("C-c C-u", "outline-up-heading"),
];

/// org-goto-map's letters when org-goto-auto-isearch is nil.
pub const LETTERS: &[(&str, &str)] = &[
    ("q", "org-goto-quit"),
    ("n", "outline-next-visible-heading"),
    ("p", "outline-previous-visible-heading"),
    ("f", "outline-forward-same-level"),
    ("b", "outline-backward-same-level"),
    ("u", "outline-up-heading"),
];

const BUFFER: &str = "*org-goto*";

/// org-goto-help.
fn help() -> String {
    let isearch = options::bool("org-goto-auto-isearch", true);
    format!(
        "Browse buffer copy, to find location or copy text.{} RET=jump to location  C-g=quit and return to previous location  [Up]/[Down]=next/prev headline  TAB=cycle visibility  [/] search",
        if isearch {
            "  Just type for auto-isearch."
        } else {
            "  n/p/f/b/u to navigate, q to quit."
        }
    )
}

/// The `*org-goto*` buffer is current.
pub fn active(ed: &Editor) -> bool {
    ed.org_buffer_name.as_deref() == Some(BUFFER)
}

/// The goto buffer's keymaps.
pub fn maps() -> Vec<&'static [(&'static str, &'static str)]> {
    if options::bool("org-goto-auto-isearch", true) {
        vec![MAP]
    } else {
        vec![MAP, LETTERS]
    }
}

/// A key org-goto-map binds (so Vim does not keep it).
pub fn binds(k: Key) -> bool {
    maps()
        .iter()
        .flat_map(|m| m.iter())
        .any(|(d, _)| super::parse_keys(d).is_some_and(|ks| ks.first() == Some(&k)))
}

/// org-goto-selected-point and org-goto-exit-command.
type Selected = Option<(usize, usize, &'static str)>;

thread_local! {
    static SELECTED: Cell<Selected> = const { Cell::new(None) };
}

/// org-goto-location: browse a copy of the buffer; `then` gets the
/// selected (line, byte, exit command), or None on quit.
pub fn location(ed: &mut Editor, then: impl FnOnce(&mut Editor, Selected) + Send + 'static) {
    let text = ed.buf.text();
    let start = ed.cur.line;
    SELECTED.with(|s| s.set(None));
    super::effect(ed, move |s| {
        s.org_special(
            BUFFER,
            &text,
            (start, 0),
            Box::new(move |s, _text, abort| {
                let sel = SELECTED.with(Cell::take);
                then(&mut s.ed, if abort { None } else { sel });
            }),
        );
        let e = &mut s.ed;
        e.readonly = true;
        fold::overview(e);
        if fold::hidden(e, start) {
            fold::show_set_visibility(e, start, "lineage");
        }
        e.set_cursor(start, 0);
        e.set_msg(format!("Select location and press RET. {}", help()));
    });
}

/// After a selection: org-mark-ring-push, move, reveal (org-goto).
fn arrive(ed: &mut Editor, l: usize, b: usize) {
    links::mark_ring_push(ed);
    let l = l.min(ed.line_count() - 1);
    ed.set_cursor(l, b);
    if fold::hidden(ed, l) {
        fold::show_context_for(ed, l, "org-goto");
    }
}

/// The outline-path-completion targets: headings up to org-goto-max-level.
fn targets(ed: &Editor) -> Vec<(String, usize)> {
    let st = super::settings(ed);
    let max = options::int("org-goto-max-level", 5).max(1) as usize;
    let odd = st.opt_bool("org-odd-levels-only", false);
    let lines: Vec<String> = (0..ed.line_count()).map(|l| ed.buf.line(l)).collect();
    let task = super::inlinetask::min_level_stars(&st);
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let n = syntax::level(l)?;
            let lvl = if odd { n.div_ceil(2) } else { n };
            (lvl <= max && n < task).then(|| {
                let p = super::refile::outline_path(&lines, &st, i, true).join("/");
                (p, i)
            })
        })
        .collect()
}

/// org-goto.
pub fn goto(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let interface = options::string("org-goto-interface", "outline");
    let outline = interface.trim_start_matches('\'') != "outline-path-completion";
    let outline = outline != !arg.is_none();
    if outline {
        location(ed, |ed, sel| match sel {
            Some((l, b, _)) => arrive(ed, l, b),
            None => ed.set_msg("Quit"),
        });
        return Ok(());
    }
    let t = targets(ed);
    let names = t.iter().map(|(n, _)| format!("{n}/")).collect();
    super::complete(ed, "Goto: ", names, true, move |ed, pick| {
        match t
            .iter()
            .find(|(n, _)| format!("{n}/") == pick || *n == pick)
        {
            Some(&(_, l)) => arrive(ed, l, 0),
            None => ed.set_msg("Quit"),
        }
    });
    Ok(())
}

/// The goto buffer's exit commands.
fn exit(ed: &mut Editor, sel: Selected) {
    SELECTED.with(|s| s.set(sel));
    let abort = sel.is_none();
    super::effect(ed, move |s| s.org_finish(abort));
}

/// The org-goto commands.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let here = (ed.cur.line, ed.cur.byte);
    let at_heading = syntax::level(&ed.buf.line(here.0)).is_some();
    Some(match name {
        "org-goto" => goto(ed, arg),
        "org-goto-ret" if active(ed) => {
            exit(ed, Some((here.0, here.1, "return")));
            Ok(())
        }
        "org-goto-left" if active(ed) => {
            if !at_heading {
                return Some(Err("Not on a heading".into()));
            }
            exit(ed, Some((here.0, 0, "left")));
            Ok(())
        }
        "org-goto-right" if active(ed) => {
            if !at_heading {
                return Some(Err("Not on a heading".into()));
            }
            exit(ed, Some((here.0, here.1, "right")));
            Ok(())
        }
        "org-goto-quit" if active(ed) => {
            exit(ed, None);
            Ok(())
        }
        "org-goto-local-auto-isearch" if active(ed) => {
            ed.open_cmdline('/', "");
            Ok(())
        }
        "org-goto-ret" | "org-goto-left" | "org-goto-right" | "org-goto-quit" => {
            Err(format!("{name}: not in an org-goto buffer"))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::key::parse_keys;
    use crate::session::Session;

    fn session(text: &str) -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.org");
        std::fs::write(&f, text).unwrap();
        let (s, _) = Session::open(Some(f), &Config::default(), &dir.path().join("swap")).unwrap();
        (dir, s)
    }

    fn keys(s: &mut Session, k: &str) {
        for key in parse_keys(k) {
            s.handle_key(key);
        }
    }

    const DOC: &str = "* A\na body\n* B\n** B1\nb1 body\n* C";

    #[test]
    fn outline_interface() {
        let (_d, mut s) = session(DOC);
        keys(&mut s, "<C-c><C-j>");
        assert_eq!(s.ed.org_buffer_name.as_deref(), Some("*org-goto*"));
        assert!(s.ed.readonly);
        assert!(s.ed.folds.hidden(1) && s.ed.folds.hidden(3));
        // Down to B, TAB shows B1, Down, RET.
        keys(&mut s, "<Down><Tab><Down><Enter>");
        assert!(s.ed.org_buffer_name.is_none());
        assert_eq!(s.ed.cur.line, 3);
        // The mark ring goes back.
        keys(&mut s, "<C-c>&");
        assert_eq!(s.ed.cur.line, 0);
        // C-g quits.
        keys(&mut s, "<C-c><C-j><Down><C-g>");
        assert_eq!(s.ed.cur.line, 0);
        assert_eq!(s.ed.msg.as_ref().unwrap().0, "Quit");
        // <left> on a heading selects it.
        keys(&mut s, "<C-c><C-j>");
        assert!(s.ed.org_buffer_name.is_some());
        keys(&mut s, "<Down><Down><Left>");
        assert_eq!(s.ed.cur.line, 5);
    }

    #[test]
    fn letters_without_auto_isearch_and_path_completion() {
        let (_d, mut s) = session(DOC);
        crate::org::options::put("org-goto-auto-isearch", toml::Value::Boolean(false));
        keys(&mut s, "<C-c><C-j>nn<Enter>");
        assert_eq!(s.ed.cur.line, 5);
        keys(&mut s, "<C-c><C-j>q");
        assert!(s.ed.org_buffer_name.is_none());
        crate::org::options::remove("org-goto-auto-isearch");
        // With C-u: outline-path completion.
        let (_d, mut s) = session(DOC);
        keys(&mut s, " u<C-c><C-j>B/B1<Enter>");
        assert_eq!(s.ed.cur.line, 3, "{:?}", s.ed.msg);
    }
}
