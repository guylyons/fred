//! magit-smerge-keep-* (smerge-mode's C-c ^ keys) on the conflict block
//! around point in a file buffer.
use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};

/// Which side of a conflict to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    Upper,
    Base,
    Lower,
    All,
    /// The side point is in.
    Current,
}

/// Line ranges of a conflict around LINE: (start, base, sep, end), where
/// base is the ||||||| line if present.
fn conflict(lines: &[String], line: usize) -> Option<(usize, Option<usize>, usize, usize)> {
    let start = (0..=line.min(lines.len().saturating_sub(1)))
        .rev()
        .find(|&i| lines[i].starts_with("<<<<<<<"))?;
    let end = (start..lines.len()).find(|&i| lines[i].starts_with(">>>>>>>"))?;
    if end < line {
        return None;
    }
    let sep = (start..end).find(|&i| lines[i].starts_with("======="))?;
    let base = (start..sep).find(|&i| lines[i].starts_with("|||||||"));
    Some((start, base, sep, end))
}

/// Point is inside a <<<<<<< ... >>>>>>> block (scanning up from point).
fn in_conflict(ed: &Editor) -> bool {
    for i in (0..=ed.cur.line).rev() {
        let l = ed.buf.line(i);
        if l.starts_with(">>>>>>>") && i != ed.cur.line {
            return false;
        }
        if l.starts_with("<<<<<<<") {
            return true;
        }
    }
    false
}

/// Resolve the conflict around point; false if point is not in one.
pub fn keep(ed: &mut Editor, how: Keep) -> bool {
    let lines: Vec<String> = (0..ed.buf.len_lines()).map(|i| ed.buf.line(i)).collect();
    let line = ed.cur.line;
    let Some((start, base, sep, end)) = conflict(&lines, line) else {
        return false;
    };
    let upper_end = base.unwrap_or(sep);
    let how = match how {
        Keep::Current if line < upper_end => Keep::Upper,
        Keep::Current if base.is_some_and(|b| line < sep && line > b) => Keep::Base,
        Keep::Current if line > sep => Keep::Lower,
        Keep::Current => return false,
        h => h,
    };
    let keep: Vec<String> = match how {
        Keep::Upper => lines[start + 1..upper_end].to_vec(),
        Keep::Base => match base {
            Some(b) => lines[b + 1..sep].to_vec(),
            None => return false,
        },
        Keep::Lower => lines[sep + 1..end].to_vec(),
        _ => {
            let mut all = lines[start + 1..upper_end].to_vec();
            all.extend_from_slice(&lines[sep + 1..end]);
            all
        }
    };
    let pos = (ed.cur.line, ed.cur.byte);
    ed.undo.begin(pos);
    if let Some((edit, _)) = ed.buf.splice_edit(start, end + 1 - start, &keep) {
        let inverse = ed.buf.apply(edit);
        ed.undo.record(inverse);
    }
    ed.undo.end(pos);
    ed.set_cursor(start.min(ed.buf.len_lines().saturating_sub(1)), 0);
    true
}

/// C-c ^ u/b/l/a/RET in a file buffer with a conflict at point.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    if ed.magit.is_some() || ed.commit_repo.is_some() || ed.mode != Mode::Normal || ed.readonly {
        return false;
    }
    match ed.vim.pending.as_slice() {
        // Only on a conflict: elsewhere C-c keeps Vim's meaning.
        [] if k == Key::ctrl('c') && in_conflict(ed) => {
            ed.vim.pending = vec![k];
            true
        }
        [c] if *c == Key::ctrl('c') => {
            if k.char() == Some('^') {
                ed.vim.pending.push(k);
                return true;
            }
            ed.vim.pending.clear();
            false
        }
        [c, h] if *c == Key::ctrl('c') && *h == Key::ch('^') => {
            ed.vim.pending.clear();
            let how = match (k.char(), k.is(KeyCode::Enter)) {
                (_, true) => Keep::Current,
                (Some('u'), _) => Keep::Upper,
                (Some('b'), _) => Keep::Base,
                (Some('l'), _) => Keep::Lower,
                (Some('a'), _) => Keep::All,
                _ => return false,
            };
            if !keep(ed, how) {
                ed.set_err("No conflict here");
            }
            true
        }
        _ => false,
    }
}
