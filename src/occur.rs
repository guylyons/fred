//! Occur: the lines of a buffer that match a regexp, listed in an
//! `*Occur*` buffer, as Emacs's occur-mode (lisp/replace.el), with
//! evil-collection's keys.
//!
//! `:[range]occur {regexp}` (or `M-s o`) lists the matching lines;
//! a Visual selection limits it to the selected lines. In the listing,
//! `Enter` goes to the match (and `n` finds the next one), `gr` runs the
//! search again (revert-buffer), and `i` edits the listed lines;
//! `:w` writes the edits back to the buffer they came from
//! (occur-edit-mode), `gr` discards them.
//!
//! ponytail: Emacs's other-window commands (`o`, `C-o`, `n`/`p` with the
//! match shown beside, `C-c C-f` follow mode) wait on fred having windows;
//! `multi-occur`, context lines (NLINES) and `r`/`c` (rename, clone) are
//! not done yet.

use crate::buffer::Buffer;
use crate::editor::{Editor, Mode};
use crate::ex::ExEffect;
use crate::key::{Key, KeyCode};
use regex::Regex;
use std::path::PathBuf;

/// Lines before the first match: the summary.
const HEADER: usize = 1;

#[derive(Clone, Debug)]
pub struct Occur {
    /// The file whose buffer was searched.
    pub source: PathBuf,
    pub pat: String,
    /// Searched lines, 0-based and inclusive (a region); None = all.
    pub range: Option<(usize, usize)>,
    re: Regex,
    hits: Vec<Hit>,
    /// `i`: the listed lines are being edited; `:w` writes them back.
    pub editing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hit {
    /// The source line, 0-based.
    line: usize,
    /// Byte of its first match.
    col: usize,
    text: String,
}

/// The `%7d:` before each listed line.
fn prefix(line: usize) -> String {
    format!("{:>7}:", line + 1)
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Search `buf` for `pat`: the listing, or why there is none.
pub fn search(
    buf: &Buffer,
    source: PathBuf,
    name: &str,
    pat: &str,
    range: Option<(usize, usize)>,
) -> Result<(Occur, Vec<String>), String> {
    let re = crate::search::compile(pat)?;
    let last = buf.len_lines().saturating_sub(1);
    let (lo, hi) = range.map_or((0, last), |(a, b)| (a.min(last), b.min(last)));
    let mut hits = vec![];
    let mut matches = 0;
    for line in lo..=hi {
        let text = buf.line(line);
        let n = re.find_iter(&text).count();
        if let Some(m) = re.find(&text) {
            matches += n;
            hits.push(Hit {
                line,
                col: m.start(),
                text,
            });
        }
    }
    if hits.is_empty() {
        return Err(format!("no matches for \"{pat}\""));
    }
    // As Emacs: "3 matches in 2 lines for "re" in buffer: name".
    let in_lines = if hits.len() == matches {
        String::new()
    } else {
        format!(" in {}", plural(hits.len(), "line", "lines"))
    };
    let region = range.map_or(String::new(), |_| {
        format!(" within region: {}-{}", lo + 1, hi + 1)
    });
    let mut lines = vec![format!(
        "{}{in_lines} for \"{pat}\" in buffer: {name}{region}",
        plural(matches, "match", "matches")
    )];
    lines.extend(hits.iter().map(|h| prefix(h.line) + &h.text));
    let o = Occur {
        source,
        pat: pat.to_owned(),
        range,
        re,
        hits,
        editing: false,
    };
    Ok((o, lines))
}

/// Make `ed` show the listing `o`, the cursor on `line` (else the first match).
pub fn show(ed: &mut Editor, o: Occur, lines: &[String], line: Option<usize>) {
    let n = ed.buf.len_lines();
    crate::vim::ops::splice_lines(ed, 0, n, lines);
    ed.undo = crate::undo::Undo::default();
    ed.mark_saved();
    ed.occur = Some(Box::new(o));
    ed.readonly = true;
    ed.mode = Mode::Normal;
    let l = line.unwrap_or(HEADER).clamp(HEADER, ed.line_count() - 1);
    ed.set_cursor(l, prefix(0).len());
    ed.clamp_cursor();
}

fn hit_at(ed: &Editor, line: usize) -> Option<&Hit> {
    let o = ed.occur.as_ref()?;
    line.checked_sub(HEADER).and_then(|i| o.hits.get(i))
}

const ALT_S: Key = Key {
    code: KeyCode::Char('s'),
    ctrl: false,
    alt: true,
    shift: false,
};

/// `M-s` reaches fred whole (not as Esc, s) in Normal and Visual mode.
pub fn wants_alt(ed: &Editor, k: Key) -> bool {
    k == ALT_S
        && matches!(
            ed.mode,
            Mode::Normal | Mode::Visual { .. } | Mode::VisualLine { .. }
        )
}

/// `M-s o` starts occur in any buffer, as Emacs; from Visual, on the
/// selected lines. Runs before the other modes' keys.
pub fn start_key(ed: &mut Editor, k: Key) -> bool {
    if ed.vim.pending == [ALT_S] {
        ed.vim.pending.clear();
        if k == Key::ch('o') {
            match ed.visual_lines() {
                Some((lo, hi)) => {
                    ed.marks.insert('<', lo);
                    ed.marks.insert('>', hi);
                    ed.open_cmdline(':', "'<,'>occur ");
                }
                None => ed.open_cmdline(':', "occur "),
            }
        }
        return true;
    }
    if wants_alt(ed, k) && ed.vim.pending.is_empty() {
        ed.vim.pending.push(k);
        return true;
    }
    false
}

/// Occur's handling of `k` in Normal or Visual mode; false leaves it to vim.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    let Some(o) = ed.occur.as_ref() else {
        return false;
    };
    // `gr` re-runs the search; while editing it discards the edits.
    if ed.vim.pending.len() == 1 && ed.vim.pending[0].char() == Some('g') {
        if k.char() == Some('r') {
            ed.vim.pending.clear();
            ed.pending_effect = Some(ExEffect::OccurRevert);
            return true;
        }
        return false;
    }
    if o.editing || !ed.vim.pending.is_empty() || k.ctrl || k.alt {
        return false;
    }
    if ed.mode != Mode::Normal {
        // Visual: only the operators that would change the listing.
        if let Some(
            'd' | 'x' | 'c' | 's' | 'r' | 'p' | 'P' | 'J' | 'C' | 'S' | 'R' | 'D' | 'X' | '>' | '<'
            | '=' | '~' | 'u' | 'U',
        ) = k.char()
        {
            ed.mode = Mode::Normal;
            ed.set_err("occur is read-only (i edits the lines)");
            return true;
        }
        return false;
    }
    match k.code {
        // occur-mode-goto-occurrence.
        KeyCode::Enter => {
            if let Some(h) = hit_at(ed, ed.cur.line) {
                // On the listed text: that spot; else the line's first match.
                let at = ed.cur.byte.checked_sub(prefix(h.line).len());
                let col = at.filter(|&c| c <= h.text.len()).unwrap_or(h.col);
                ed.pending_effect = Some(ExEffect::Open {
                    path: o.source.clone(),
                    line: h.line,
                    col,
                    pattern: Some(o.pat.clone()),
                });
            }
        }
        // occur-edit-mode.
        KeyCode::Char('i') => {
            if let Some(o) = ed.occur.as_mut() {
                o.editing = true;
            }
            ed.readonly = false;
            ed.set_msg("editing: :w writes the lines back, gr discards");
        }
        // Vim's editing keys: the listing is read-only.
        KeyCode::Char(
            'a' | 'A' | 'I' | 'o' | 'O' | 'c' | 'C' | 's' | 'S' | 'r' | 'R' | 'p' | 'P' | 'J' | 'x'
            | 'X' | 'd' | 'D' | '.' | '>' | '<' | '=' | '~',
        ) => ed.set_err("occur is read-only (i edits the lines)"),
        _ => return false,
    }
    true
}

/// `:w` while editing: the changed lines, as (source line, old, new).
pub fn changes(ed: &Editor) -> Result<Vec<(usize, String, String)>, String> {
    let Some(o) = ed.occur.as_ref().filter(|o| o.editing) else {
        return Err("occur: nothing to write (i edits the lines)".into());
    };
    if ed.line_count() != HEADER + o.hits.len() {
        return Err("occur: lines can't be added or removed, only changed".into());
    }
    let mut out = vec![];
    for (i, h) in o.hits.iter().enumerate() {
        let line = ed.buf.line(HEADER + i);
        let Some(new) = line.strip_prefix(&prefix(h.line)) else {
            return Err(format!(
                "occur: the line number of line {} changed",
                h.line + 1
            ));
        };
        if new != h.text {
            out.push((h.line, h.text.clone(), new.to_owned()));
        }
    }
    Ok(out)
}

/// The edits were written back: the listing shows them, read-only again.
pub fn written(ed: &mut Editor) {
    let lines: Vec<String> = (0..ed.line_count()).map(|l| ed.buf.line(l)).collect();
    let Some(mut o) = ed.occur.take().map(|o| *o) else {
        return;
    };
    for (i, h) in o.hits.iter_mut().enumerate() {
        h.text = lines[HEADER + i][prefix(h.line).len()..].to_owned();
    }
    o.editing = false;
    let line = ed.cur.line;
    show(ed, o, &lines, Some(line));
}

/// Faces: the summary, the line-number prefix and each match.
pub(crate) fn styles(ed: &Editor, line: usize) -> crate::highlight::LineStyles {
    use ratatui::style::{Color, Modifier, Style};
    let Some(o) = ed.occur.as_ref().filter(|o| !o.editing) else {
        return vec![];
    };
    let text = ed.buf.line(line);
    if line < HEADER {
        return vec![(Style::default().add_modifier(Modifier::BOLD), 0..text.len())];
    }
    let Some(h) = hit_at(ed, line) else {
        return vec![];
    };
    let p = prefix(h.line).len().min(text.len());
    let mut spans = vec![(Style::default().fg(Color::DarkGray), 0..p)];
    spans.extend(
        o.re.find_iter(&text[p..])
            .filter(|m| !m.is_empty())
            .map(|m| {
                (
                    Style::default().fg(Color::Black).bg(Color::Yellow),
                    p + m.start()..p + m.end(),
                )
            }),
    );
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_matching_lines_as_emacs() {
        let buf = Buffer::from_text("foo\nbar\nfoo foo\nBaz\n");
        let (o, lines) = search(&buf, "a.txt".into(), "a.txt", "foo", None).unwrap();
        assert_eq!(
            lines,
            [
                "3 matches in 2 lines for \"foo\" in buffer: a.txt",
                "      1:foo",
                "      3:foo foo",
            ]
        );
        assert_eq!(o.hits[1].line, 2);
        // Smart case, a region, one match per line.
        let (_, lines) = search(&buf, "a".into(), "a", "baz", Some((1, 3))).unwrap();
        assert_eq!(
            lines[0],
            "1 match for \"baz\" in buffer: a within region: 2-4"
        );
        assert!(search(&buf, "a".into(), "a", "Foo", None).is_err());
    }
}
