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

/// Replace the whole buffer (one undo step), keeping the cursor at the top.
pub fn replace_text(ed: &mut Editor, text: &str) {
    replace_all(ed, text);
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

/// log-edit-comment-ring, kept per repository (git-commit-save-message).
fn ring_path(repo: &Repo) -> Option<std::path::PathBuf> {
    let p = repo
        .read(&["rev-parse", "--git-path", "fred-message-ring"])
        .ok()?;
    Some(repo.root.join(String::from_utf8_lossy(&p).trim()))
}
fn saved_messages(repo: &Repo) -> Vec<String> {
    ring_path(repo)
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| {
            String::from_utf8_lossy(&b)
                .split('\0')
                .filter(|m| !m.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
/// git-commit-buffer-message: without comment lines and the scissors
/// section, outer blank lines trimmed; None when nothing is left.
pub fn buffer_message(text: &str) -> Option<String> {
    let mut out = vec![];
    for line in text.lines() {
        if line.starts_with('#') && line.contains(" >8 ") {
            break;
        }
        if !line.starts_with('#') {
            out.push(line);
        }
    }
    let s = out.join("\n");
    let s = s.trim_matches('\n');
    (!s.trim().is_empty()).then(|| format!("{s}\n"))
}
/// git-commit-save-message: newest first, without duplicates, at most
/// log-edit-maximum-comment-ring-size (32) entries.
pub fn save_message(ed: &mut Editor) {
    let Some(repo) = ed.commit_repo.clone() else {
        return ed.set_err("Not in a commit message draft");
    };
    let Some(message) = buffer_message(&ed.buf.text()) else {
        return ed.set_msg("Only whitespace and/or comments; message not saved");
    };
    let mut ring = saved_messages(&repo);
    ring.retain(|m| *m != message);
    ring.insert(0, message);
    ring.truncate(32);
    let written = ring_path(&repo)
        .ok_or("No git directory".to_owned())
        .and_then(|p| std::fs::write(p, ring.join("\0")).map_err(|e| e.to_string()));
    match written {
        Ok(()) => ed.set_msg("Message saved"),
        Err(e) => ed.set_err(e),
    }
}

/// git-commit-prev-message / -next-message: saved messages, then recent
/// commit messages (upstream's ring holds only what this Emacs saved).
fn cycle(ed: &mut Editor, older: bool) {
    let Some(repo) = ed.commit_repo.clone() else {
        return;
    };
    let mut ring = saved_messages(&repo)
        .into_iter()
        .map(|m| m.trim().to_owned())
        .collect::<Vec<_>>();
    let log: Vec<String> = repo
        .read(&["log", "-n", "50", "--format=%B%x00"])
        .map(|o| {
            String::from_utf8_lossy(&o)
                .split('\0')
                .map(|m| m.trim().to_owned())
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default();
    for m in log {
        if !ring.contains(&m) {
            ring.push(m);
        }
    }
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
        } else if k.alt && k.code == KeyCode::Char('s') {
            save_message(ed);
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

/// magit-diff--modified-defuns, adapted: Git's hunk-header function context
/// (its xfuncname, from a -U0 diff) stands in for Emacs's which-function.
pub fn modified_defuns(diff: &[u8]) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = vec![];
    let mut old = String::new();
    for line in String::from_utf8_lossy(diff).lines() {
        if line.starts_with("diff --git ") {
            old.clear();
        } else if let Some(f) = line.strip_prefix("--- a/") {
            old = f.to_owned();
        } else if let Some(f) = line.strip_prefix("+++ ") {
            let file = f.strip_prefix("b/").map_or(old.clone(), str::to_owned);
            out.push((file, vec![]));
        } else if line.starts_with("@@ ")
            && let Some((_, ctx)) = line[3..].split_once(" @@")
            && let Some(name) = defun_name(ctx.trim())
            && let Some((_, defs)) = out.last_mut()
            && !defs.contains(&name)
        {
            defs.push(name);
        }
    }
    out
}
/// The function a hunk changes: the nearest context line above its first
/// change that starts a definition by Git's default funcname rule (a
/// letter, `_` or `$` in column 0), else the header's own context.
pub fn hunk_defun<'a>(hunk: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut lines = hunk.into_iter();
    let header = lines.next()?;
    let mut context = vec![];
    for l in lines {
        if l.starts_with(['+', '-']) {
            break;
        }
        context.push(l.strip_prefix(' ').unwrap_or(l));
    }
    let starts_def = |l: &&&str| l.starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '$');
    if let Some(def) = context.iter().rev().find(starts_def) {
        return defun_name(def.trim());
    }
    let (_, ctx) = header.get(3..)?.split_once(" @@")?;
    defun_name(ctx.trim())
}
/// The name in a function header line: the identifier before its first
/// parameter list, else the line itself.
pub fn defun_name(header: &str) -> Option<String> {
    if header.is_empty() {
        return None;
    }
    let ident = |c: char| c.is_alphanumeric() || "_-:.$!?*<>".contains(c);
    for (i, _) in header.match_indices('(').filter(|(i, _)| *i > 0) {
        let before = header[..i].trim_end();
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, c)| ident(*c))
            .last()
            .map(|(j, _)| j);
        if let Some(j) = start {
            return Some(before[j..].to_owned());
        }
    }
    Some(header.trim_end_matches(['{', ':', ' ']).to_owned())
}
/// git-commit-insert-changelog-gnu (true) and -plain.
pub fn changelog(entries: &[(String, Vec<String>)], gnu: bool) -> Vec<String> {
    let mut out = vec![];
    for (file, defs) in entries {
        if gnu {
            if defs.is_empty() {
                out.push(format!("* {file}:"));
            }
            for (i, d) in defs.iter().enumerate() {
                out.push(if i == 0 {
                    format!("* {file} ({d}):")
                } else {
                    format!("({d}):")
                });
            }
        } else {
            out.push(format!("{file}:"));
            out.extend(defs.iter().map(|d| format!("  `{d}'")));
        }
    }
    out
}
/// The diff being committed (magit-commit-diff--args): staged changes, or
/// since HEAD^ when amending.
pub fn committing_diff(repo: &Repo, amend: bool) -> Result<Vec<u8>, String> {
    let base = if amend && repo.read(&["rev-parse", "--verify", "-q", "HEAD^"]).is_ok() {
        "HEAD^"
    } else {
        "HEAD"
    };
    let mut args = vec![
        "diff",
        "-U0",
        "--no-color",
        "--no-ext-diff",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    if repo.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        args.extend(["--cached", base]);
    } else {
        args.push("--cached");
    }
    repo.read(&args)
}
/// Insert lines below the cursor line in a draft.
fn insert_below(ed: &mut Editor, lines: Vec<String>) {
    if lines.is_empty() {
        return ed.set_msg("No changes");
    }
    let pos = (ed.cur.line, ed.cur.byte);
    let at = ed.cur.line + 1;
    let n = lines.len();
    ed.undo.begin(pos);
    if let Some((edit, _)) = ed.buf.splice_edit(
        at.min(ed.buf.len_lines()),
        at.min(ed.buf.len_lines()),
        &lines,
    ) {
        let inverse = ed.buf.apply(edit);
        ed.undo.record(inverse);
    }
    ed.undo.end(pos);
    ed.set_cursor(at + n - 1, 0);
}
/// git-commit-insert-changelog-gnu / -plain and magit-generate-changelog.
pub fn insert_changelog(ed: &mut Editor, gnu: bool) {
    let Some(repo) = ed.commit_repo.clone() else {
        return ed.set_err("Not in a commit message buffer");
    };
    let amend = matches!(ed.commit_mode, super::CommitMode::Amend(_));
    match committing_diff(&repo, amend) {
        Ok(diff) => insert_below(ed, changelog(&modified_defuns(&diff), gnu)),
        Err(e) => ed.set_err(e),
    }
}
/// magit-commit-add-log-insert: add "* FILE (DEFUN): " to the message's
/// changelog entries (before comments and trailers). Returns the new text
/// and the line to put the cursor on.
pub fn add_log_insert(text: &str, file: &str, defun: Option<&str>) -> (String, usize) {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    // The message body ends before comments and the trailer block.
    let mut end = lines
        .iter()
        .position(|l| l.starts_with('#'))
        .unwrap_or(lines.len());
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let trailer = |l: &str| {
        l.split_once(": ").is_some_and(|(k, _)| {
            !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
    };
    let mut start = end;
    while start > 0 && trailer(&lines[start - 1]) {
        start -= 1;
    }
    if start < end && start > 0 && lines[start - 1].trim().is_empty() {
        end = start - 1;
    }
    let head = format!("* {file}");
    let entry = (0..end).find(|&i| {
        lines[i] == format!("{head}:")
            || lines[i].starts_with(&format!("{head}: "))
            || lines[i].starts_with(&format!("{head} ("))
    });
    if let Some(i) = entry {
        let same = defun.is_none_or(|d| lines[i].starts_with(&format!("{head} ({d}):")));
        // The entry's continuation lines run to the next "* " entry.
        let mut j = i + 1;
        while j < end && !lines[j].starts_with("* ") && !lines[j].trim().is_empty() {
            if defun.is_some_and(|d| lines[j].starts_with(&format!("({d}):"))) {
                return (lines.join("\n") + "\n", j);
            }
            j += 1;
        }
        if same {
            return (lines.join("\n") + "\n", i);
        }
        lines.insert(j, format!("({}): ", defun.unwrap_or_default()));
        return (lines.join("\n") + "\n", j);
    }
    let new = match defun {
        Some(d) => format!("{head} ({d}): "),
        None => format!("{head}: "),
    };
    // After the last entry block, else after the summary and a blank line.
    let last = (0..end)
        .rev()
        .find(|&i| lines[i].starts_with("* ") || lines[i].starts_with('('));
    let at = match last {
        Some(i) => {
            let mut j = i + 1;
            while j < end && !lines[j].trim().is_empty() && !lines[j].starts_with("* ") {
                j += 1;
            }
            j
        }
        None if end == 0 => 0,
        None => {
            lines.insert(end, String::new());
            end + 1
        }
    };
    lines.insert(at, new);
    (lines.join("\n") + "\n", at)
}

/// Today's local date, YYYY-MM-DD (add-log-time-format).
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as libc::time_t);
    // SAFETY: `tm` is plain data that localtime_r fills in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call.
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return "1970-01-01".into();
    }
    format!(
        "{}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}
/// add-change-log-entry: under today's heading for this author (added at
/// the top when the first heading differs), an item "* FILE (DEFUN): ";
/// an existing item for the file gains the defun.
pub fn change_log_add(old: &str, heading: &str, file: &str, defun: Option<&str>) -> String {
    let mut lines: Vec<String> = old.lines().map(str::to_owned).collect();
    if lines.first().map(String::as_str) != Some(heading) {
        let mut top = vec![heading.to_owned(), String::new()];
        if !lines.is_empty() {
            top.push(String::new());
        }
        lines.splice(0..0, top);
        // The new item goes between the heading's blank line and the old text.
        let item = match defun {
            Some(d) => format!("\t* {file} ({d}): "),
            None => format!("\t* {file}: "),
        };
        lines.insert(2, item);
        return lines.join("\n") + "\n";
    }
    let end = lines[1..]
        .iter()
        .position(|l| !l.is_empty() && !l.starts_with('\t') && !l.starts_with(' '))
        .map_or(lines.len(), |i| i + 1);
    let head = format!("\t* {file}");
    if let Some(i) = (1..end).find(|&i| lines[i].starts_with(&head)) {
        if let Some(d) = defun
            && !lines[i..end].iter().any(|l| l.contains(&format!("({d})")))
        {
            lines.insert(i + 1, format!("\t({d}): "));
        }
        return lines.join("\n") + "\n";
    }
    let item = match defun {
        Some(d) => format!("{head} ({d}): "),
        None => format!("{head}: "),
    };
    lines.insert(2.min(lines.len()), item);
    lines.join("\n") + "\n"
}
