//! org-keys.el speed commands: with org-use-speed-commands, single keys
//! at the beginning of a headline (Normal mode, column 0) run
//! org-speed-commands, and at a `#+begin_src` line the
//! org-babel-key-bindings; `?` (org-speed-command-help) lists them.
//!
//! Adaptation: SPC keeps Fred's leader, so the `" "` speed command
//! (org-display-outline-path) is not offered; a function value of
//! org-use-speed-commands counts as t.

use super::sexp::{self, Sexp};
use super::{Prefix, syntax};
use crate::editor::{Editor, Mode};
use crate::key::Key;

/// The default org-speed-commands, as upstream writes it.
const DEFAULT: &str = r##"(("Outline Navigation")
    ("n" . (org-speed-move-safe 'org-next-visible-heading))
    ("p" . (org-speed-move-safe 'org-previous-visible-heading))
    ("f" . (org-speed-move-safe 'org-forward-heading-same-level))
    ("b" . (org-speed-move-safe 'org-backward-heading-same-level))
    ("F" . org-next-block)
    ("B" . org-previous-block)
    ("u" . (org-speed-move-safe 'outline-up-heading))
    ("j" . org-goto)
    ("g" . (org-refile '(4)))
    ("Outline Visibility")
    ("c" . org-cycle)
    ("C" . org-shifttab)
    (" " . org-display-outline-path)
    ("s" . org-toggle-narrow-to-subtree)
    ("k" . org-cut-subtree)
    ("=" . org-columns)
    ("Outline Structure Editing")
    ("U" . org-metaup)
    ("D" . org-metadown)
    ("r" . org-metaright)
    ("l" . org-metaleft)
    ("R" . org-shiftmetaright)
    ("L" . org-shiftmetaleft)
    ("i" . (progn (forward-char 1) (call-interactively 'org-insert-heading-respect-content)))
    ("^" . org-sort)
    ("w" . org-refile)
    ("a" . org-archive-subtree-default-with-confirmation)
    ("@" . org-mark-subtree)
    ("#" . org-toggle-comment)
    ("Clock Commands")
    ("I" . org-clock-in)
    ("O" . org-clock-out)
    ("Meta Data Editing")
    ("t" . org-todo)
    ("," . (org-priority))
    ("0" . (org-priority 'remove))
    ("1" . (org-priority org-priority-highest))
    ("2" . (org-priority org-priority-default))
    ("3" . (org-priority org-priority-lowest))
    (":" . org-set-tags-command)
    ("e" . org-set-effort)
    ("E" . org-inc-effort)
    ("W" . (lambda (m) (interactive "sMinutes before warning: ") (org-entry-put (point) "APPT_WARNTIME" m)))
    ("Agenda Views etc")
    ("v" . org-agenda)
    ("/" . org-sparse-tree)
    ("Misc")
    ("o" . org-open-at-point)
    ("?" . org-speed-command-help)
    ("<" . (org-agenda-set-restriction-lock 'subtree))
    (">" . (org-agenda-remove-restriction-lock)))"##;

/// org-babel-key-bindings (single keys; control keys need C-c C-v).
const BABEL: &[(&str, &str)] = &[
    ("p", "org-babel-previous-src-block"),
    ("n", "org-babel-next-src-block"),
    ("e", "org-babel-execute-maybe"),
    ("o", "org-babel-open-src-block-result"),
    ("v", "org-babel-expand-src-block"),
    ("u", "org-babel-goto-src-block-head"),
    ("g", "org-babel-goto-named-src-block"),
    ("r", "org-babel-goto-named-result"),
    ("b", "org-babel-execute-buffer"),
    ("s", "org-babel-execute-subtree"),
    ("d", "org-babel-demarcate-block"),
    ("t", "org-babel-tangle"),
    ("f", "org-babel-tangle-file"),
    ("c", "org-babel-check-src-block"),
    ("j", "org-babel-insert-header-arg"),
    ("l", "org-babel-load-in-session"),
    ("i", "org-babel-lob-ingest"),
    ("I", "org-babel-view-src-block-info"),
    ("z", "org-babel-switch-to-session-with-code"),
    ("a", "org-babel-sha1-hash"),
    ("h", "org-babel-describe-bindings"),
    ("x", "org-babel-do-key-sequence-in-edit-buffer"),
    ("k", "org-babel-remove-result-one-or-many"),
];

/// One org-speed-commands entry: a heading, or a key and its action.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Head(String),
    Key(String, Sexp),
}

/// org-speed-commands (the option, else upstream's default).
pub fn speed_commands() -> Vec<Entry> {
    let v = sexp::option("org-speed-commands")
        .filter(|v| v.list().is_some_and(|l| !l.is_empty()))
        .unwrap_or_else(|| sexp::read(DEFAULT).expect("the default speed commands"));
    v.list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let key = e.car()?.str()?.to_owned();
            match e {
                Sexp::List(items) if items.len() == 1 => Some(Entry::Head(key)),
                _ => Some(Entry::Key(key, e.cdr())),
            }
        })
        .collect()
}

/// org-use-speed-commands.
pub fn enabled() -> bool {
    sexp::option("org-use-speed-commands").is_some_and(|v| v.truthy())
}

/// prin1 with Emacs's print-quoted (`'x`).
fn prin1(v: &Sexp) -> String {
    match v {
        Sexp::List(items) if items.len() == 2 && items[0].sym() == Some("quote") => {
            format!("'{}", prin1(&items[1]))
        }
        Sexp::List(items) => format!(
            "({})",
            items.iter().map(prin1).collect::<Vec<_>>().join(" ")
        ),
        v => v.to_string(),
    }
}

/// The `*Help*` text of org-speed-command-help.
pub fn help_text() -> String {
    let mut out = String::from("Speed commands\n==============\n");
    for e in speed_commands() {
        match e {
            Entry::Head(h) => {
                out.push_str(&format!("\n{h}\n{}\n", "-".repeat(h.chars().count())));
            }
            Entry::Key(k, v) => {
                let action = match &v {
                    Sexp::Sym(s) => s.clone(),
                    v => prin1(v),
                };
                out.push_str(&format!("{k}   {action}\n"));
            }
        }
    }
    out
}

/// The symbol in `'x` or `x`.
fn quoted(v: &Sexp) -> Option<&str> {
    match v {
        Sexp::List(items) if items.len() == 2 && items[0].sym() == Some("quote") => items[1].sym(),
        v => v.sym(),
    }
}

/// Run a speed command's action (a command, or one of the forms of the
/// default table).
fn exec(ed: &mut Editor, action: &Sexp) -> Result<(), String> {
    if let Some(name) = action.sym() {
        super::run(ed, name, Prefix::None);
        return Ok(());
    }
    let items = action.list().unwrap_or(&[]);
    let Some(f) = items.first().and_then(Sexp::sym) else {
        return Err(format!("Invalid speed command: {}", prin1(action)));
    };
    let args = &items[1..];
    match f {
        "org-speed-move-safe" => {
            let cmd = args.first().and_then(quoted).unwrap_or("").to_owned();
            let pos = ed.cur.pos();
            super::call(ed, &cmd, Prefix::None)?;
            if !(ed.cur.byte == 0 && syntax::level(&ed.buf.line(ed.cur.line)).is_some()) {
                ed.set_cursor(pos.0, pos.1);
                return Err(format!("Boundary reached while executing {cmd}"));
            }
            Ok(())
        }
        "progn" => args.iter().try_for_each(|a| exec(ed, a)),
        "forward-char" => {
            let n = args.first().and_then(Sexp::int).unwrap_or(1).max(0) as usize;
            let len = ed.buf.line(ed.cur.line).len();
            ed.cur.byte = (ed.cur.byte + n).min(len);
            Ok(())
        }
        "call-interactively" => {
            let cmd = args.first().and_then(quoted).unwrap_or("");
            super::call(ed, cmd, Prefix::None)
        }
        "org-priority" => match args.first() {
            None => super::call(ed, "org-priority", Prefix::None),
            Some(a) if quoted(a) == Some("remove") => super::todo::set_priority(ed, None),
            Some(a) => {
                let (high, low, default) = super::settings(ed).priorities;
                let p = match a.sym() {
                    Some("org-priority-highest") => high,
                    Some("org-priority-lowest") => low,
                    Some("org-priority-default") => default,
                    _ => a.int().map_or(default, |i| i as u32),
                };
                super::todo::set_priority(ed, Some(p))
            }
        },
        "lambda" => {
            // (lambda (m) (interactive "sPROMPT") (org-entry-put (point) PROP m))
            let prompt = args
                .iter()
                .find_map(|x| {
                    let l = x.list()?;
                    (l.first()?.sym()? == "interactive").then(|| l.get(1)?.str())?
                })
                .and_then(|s| s.strip_prefix('s'))
                .unwrap_or("Value: ")
                .to_owned();
            let prop = args
                .iter()
                .find_map(|x| {
                    let l = x.list()?;
                    (l.first()?.sym()? == "org-entry-put").then(|| l.get(2)?.str())?
                })
                .map(str::to_owned)
                .ok_or("Speed command lambda not supported")?;
            super::read(ed, &prompt, "", move |ed, v| {
                let h = super::props::entry(ed, ed.cur.line);
                ed.undo.begin(ed.cur.pos());
                if let Err(e) = super::props::put(ed, h, &prop, &v) {
                    ed.set_err(e);
                }
                ed.undo.end(ed.cur.pos());
            });
            Ok(())
        }
        "org-refile" => {
            let arg = match args.first().and_then(|a| match a {
                Sexp::List(l) if l.len() == 2 && l[0].sym() == Some("quote") => l[1].list(),
                _ => None,
            }) {
                Some([Sexp::Int(4)]) => Prefix::U(1),
                Some([Sexp::Int(16)]) => Prefix::U(2),
                _ => Prefix::None,
            };
            super::call(ed, "org-refile", arg)
        }
        name if args.iter().all(|a| quoted(a).is_some()) => super::call(ed, name, Prefix::None),
        _ => Err(format!("Invalid speed command: {}", prin1(action))),
    }
}

/// A speed key in Normal mode: true when it ran a speed command.
pub fn speed_key(ed: &mut Editor, k: Key) -> bool {
    if ed.mode != Mode::Normal
        || ed.org.is_none()
        || ed.org_view.is_some()
        || !ed.vim.pending.is_empty()
        || !ed.org_keys.is_empty()
        || k.ctrl
        || k.alt
        || ed.cur.byte != 0
        || !enabled()
    {
        return false;
    }
    let Some(c) = k.char() else { return false };
    if c == ' ' {
        return false;
    }
    let key = c.to_string();
    let line = ed.buf.line(ed.cur.line);
    if syntax::level(&line).is_some() {
        let found = speed_commands().into_iter().find_map(|e| match e {
            Entry::Key(x, v) if x == key => Some(v),
            _ => None,
        });
        let Some(action) = found else { return false };
        let before = ed.cur.pos();
        ed.undo.begin(before);
        let r = exec(ed, &action);
        ed.undo.end(ed.cur.pos());
        if let Err(e) = r {
            ed.set_err(e);
        }
        ed.clamp_cursor();
        return true;
    }
    if crate::org_re!(r"(?i)^[ \t]*#\+begin_src").is_match(&line)
        && super::ctx::src_block(ed, ed.cur.line).is_some()
        && let Some((_, cmd)) = BABEL.iter().find(|(x, _)| *x == key)
    {
        super::run(ed, cmd, Prefix::None);
        return true;
    }
    false
}

/// org-babel-describe-bindings: the `C-c C-v` keys.
fn babel_bindings() -> String {
    let mut out = String::from("Key             Binding\n---             -------\n");
    for (k, c) in super::keymap::ORG_MODE_MAP {
        if k.starts_with("C-c C-v ") {
            out.push_str(&format!("{k:<16}{c}\n"));
        }
    }
    out
}

/// The org-keys.el commands.
pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-speed-command-help" => {
            if !enabled() {
                return Some(Err(
                    "Speed commands are not activated, customize `org-use-speed-commands'".into(),
                ));
            }
            let text = help_text();
            super::effect(ed, move |s| {
                s.org_temp_buffer("*Help*", &text, false);
            });
            Ok(())
        }
        "org-babel-describe-bindings" => {
            let text = babel_bindings();
            super::effect(ed, move |s| {
                s.org_temp_buffer("*Help*", &text, false);
            });
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::tests::org;

    #[test]
    fn default_table_and_help() {
        let t = speed_commands();
        assert_eq!(t[0], Entry::Head("Outline Navigation".into()));
        assert!(t.contains(&Entry::Key("c".into(), Sexp::Sym("org-cycle".into()))));
        let h = help_text();
        assert!(h.starts_with("Speed commands\n==============\n\nOutline Navigation\n------------------\nn   (org-speed-move-safe 'org-next-visible-heading)\n"), "{h}");
        assert!(h.contains("\n0   (org-priority 'remove)\n"), "{h}");
        assert!(h.contains("\nt   org-todo\n"), "{h}");
    }

    #[test]
    fn speed_keys_at_heading_start() {
        let text = "* A\nbody\n* B\n** B1\n* C";
        // Off by default: `n` is Vim's.
        let e = org(text, "n");
        assert_eq!(e.cur.line, 0);
        super::super::options::put("org-use-speed-commands", toml::Value::Boolean(true));
        let e = org(text, "n");
        assert_eq!(e.cur.line, 2);
        let e = org(text, "nnn");
        assert_eq!(e.cur.line, 4);
        // Boundary reached: back where it was, with an error.
        let e = org(text, "Gn");
        assert_eq!(e.cur.line, 4);
        assert!(
            e.msg
                .as_ref()
                .unwrap()
                .0
                .ends_with("Boundary reached while executing org-next-visible-heading")
        );
        // t sets TODO; 1 sets the highest priority; 0 removes it.
        let e = org(text, "t");
        assert_eq!(e.buf.line(0), "* TODO A");
        let e = org(text, "1");
        assert_eq!(e.buf.line(0), "* [#A] A");
        let e = org(text, "10");
        assert_eq!(e.buf.line(0), "* A");
        // Not at column 0: Vim keys.
        let e = org(text, "<Right>n");
        assert_eq!(e.cur.line, 0);
        // On a body line: Vim keys.
        let e = org(text, "<Down>t");
        assert_eq!(e.buf.line(0), "* A");
        // W asks for APPT_WARNTIME.
        let e = org(text, "W15<Enter>");
        assert_eq!(e.buf.line(2), ":APPT_WARNTIME: 15");
        // Babel keys on a src block line.
        let e = org(
            "#+begin_src sh\necho\n#+end_src\n#+begin_src sh\nx\n#+end_src",
            "n",
        );
        assert_eq!(e.cur.line, 3);
        super::super::options::remove("org-use-speed-commands");
    }
}
