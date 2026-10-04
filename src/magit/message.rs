//! git-commit.el in Fred's commit draft: message history, trailers and
//! magit-diff-while-committing.
use super::Action;
use super::repo::Repo;
use crate::{
    editor::{Editor, Mode},
    ex::ExEffect,
    key::{Key, KeyCode},
};

/// The trailer keys git-commit binds under C-c (git-commit-redundant-bindings).
fn trailer_for(k: Key) -> Option<&'static str> {
    if k.alt && k.code == KeyCode::Char('i') {
        return Some("Suggested-by");
    }
    if !k.ctrl && !k.is(KeyCode::Enter) {
        return None;
    }
    let c = match k.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    };
    Some(match (c, k.is(KeyCode::Enter)) {
        (_, true) | (Some('m'), _) => "Modified-by",
        (Some('a'), _) => "Acked-by",
        (Some('o'), _) => "Cc",
        (Some('p'), _) => "Reported-by",
        (Some('r'), _) => "Reviewed-by",
        (Some('s'), _) => "Signed-off-by",
        (Some('t'), _) => "Tested-by",
        _ => return None,
    })
}

/// git-commit-self-ident: "Name <email>".
pub fn ident(repo: &Repo) -> String {
    repo.read(&["var", "GIT_COMMITTER_IDENT"])
        .map(|o| {
            let s = String::from_utf8_lossy(&o).trim().to_owned();
            // Drop the trailing "<seconds> <zone>".
            let mut parts: Vec<&str> = s.rsplitn(3, ' ').collect();
            parts.pop().unwrap_or("").to_owned()
        })
        .unwrap_or_default()
}

fn replace_all(ed: &mut Editor, text: &str) {
    let lines: Vec<String> = text
        .trim_end_matches('\n')
        .split('\n')
        .map(str::to_owned)
        .collect();
    let pos = (ed.cur.line, ed.cur.byte);
    ed.undo.begin(pos);
    if let Some((edit, _)) = ed.buf.splice_edit(0, ed.buf.len_lines(), &lines) {
        let inverse = ed.buf.apply(edit);
        ed.undo.record(inverse);
    }
    ed.undo.end(pos);
    ed.set_cursor(0, 0);
}

/// git-commit-insert-trailer: append "KEY: VALUE" with git interpret-trailers.
pub fn insert_trailer(ed: &mut Editor, key: &str, value: &str) {
    let Some(repo) = ed.commit_repo.clone() else {
        return;
    };
    let value = match value.trim() {
        "" => ident(&repo),
        v => v.to_owned(),
    };
    if value.is_empty() || value.contains('\n') {
        return ed.set_err("A trailer value is required");
    }
    let text = ed.buf.text();
    // ponytail: synchronous git; local and fast.
    match repo.run(
        &[
            "interpret-trailers".into(),
            "--trailer".into(),
            format!("{key}: {value}").into(),
        ],
        Some(text.as_bytes()),
    ) {
        Ok(out) => replace_all(ed, &String::from_utf8_lossy(&out)),
        Err(e) => ed.set_err(e),
    }
}

/// git-commit-prev-message / -next-message over recent commit messages
/// (upstream cycles log-edit-comment-ring; Fred uses the repository's log).
fn cycle(ed: &mut Editor, older: bool) {
    let Some(repo) = ed.commit_repo.clone() else {
        return;
    };
    let ring: Vec<String> = repo
        .read(&["log", "-n", "50", "--format=%B%x00"])
        .map(|o| {
            String::from_utf8_lossy(&o)
                .split('\0')
                .map(|m| m.trim().to_owned())
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let next = match (ed.commit_history.as_ref().map(|h| h.0), older) {
        (None, true) => Some(0),
        (None, false) => return ed.set_err("No next message"),
        (Some(i), true) if i + 1 < ring.len() => Some(i + 1),
        (Some(_), true) => return ed.set_err("No previous message"),
        (Some(0), false) => None,
        (Some(i), false) => Some(i - 1),
    };
    let original = match ed.commit_history.take() {
        Some((_, o)) => o,
        None => ed.buf.text(),
    };
    match next {
        Some(i) => {
            let Some(m) = ring.get(i).cloned() else {
                return ed.set_err("No previous message");
            };
            replace_all(ed, &m);
            ed.commit_history = Some((i, original));
        }
        None => replace_all(ed, &original),
    }
}

/// Keys in a commit draft (git-commit-mode-map under evil-collection).
pub fn key(ed: &mut Editor, k: Key) -> bool {
    if ed.commit_repo.is_none() || ed.magit.is_some() || ed.mode != Mode::Normal {
        return false;
    }
    if ed.vim.pending == [Key::ctrl('c')] {
        ed.vim.pending.clear();
        if k == Key::ctrl('d') {
            ed.pending_effect = Some(ExEffect::Magit(Action::DiffWhileCommitting));
        } else if k == Key::ctrl('i') || k.is(KeyCode::Tab) {
            ed.magit_prompt = Some(super::Prompt::Trailer(None));
            ed.open_cmdline('=', "");
            if let Mode::Command(cl) = &mut ed.mode {
                cl.prompt = "Insert trailer (Key: value): ".into();
            }
        } else if k == Key::ctrl('s') {
            insert_trailer(ed, "Signed-off-by", "");
        } else if let Some(key) = trailer_for(k) {
            let me = ed.commit_repo.as_ref().map(ident).unwrap_or_default();
            ed.magit_prompt = Some(super::Prompt::Trailer(Some(key)));
            ed.open_cmdline('=', "");
            if let Mode::Command(cl) = &mut ed.mode {
                cl.prompt = format!("{key} (default {me}): ");
            }
        } else if k.alt && matches!(k.code, KeyCode::Char('p' | 'n')) {
            cycle(ed, k.code == KeyCode::Char('p'));
        } else {
            // Not a git-commit key: it keeps its usual meaning.
            return false;
        }
        return true;
    }
    if ed.vim.pending.is_empty() && k == Key::ctrl('c') {
        ed.vim.pending = vec![k];
        return true;
    }
    // evil-collection: M-k / gk previous message, M-j / gj next.
    let c = match k.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    };
    let older = match (ed.vim.pending.as_slice(), c, k.alt) {
        ([], Some('k'), true) => Some(true),
        ([], Some('j'), true) => Some(false),
        ([g], Some('k'), false) if *g == Key::ch('g') => Some(true),
        ([g], Some('j'), false) if *g == Key::ch('g') => Some(false),
        _ => None,
    };
    if let Some(older) = older {
        ed.vim.pending.clear();
        cycle(ed, older);
        return true;
    }
    false
}
