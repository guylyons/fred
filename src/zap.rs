//! Flash-style jumps to words wholly visible in the editor's text area.

use crate::editor::Editor;
use crate::key::{Key, KeyCode};
use crate::ui::View;
use crate::ui::layout::{Layout, Placed, wrap_cursor};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Viewport {
    pub view: View,
    pub rows: usize,
    pub cols: usize,
    pub wrap: bool,
}

#[derive(Debug)]
pub(crate) struct Word {
    pub text: String,
    pub point: (usize, usize),
    /// Text-area coordinates and width of each grapheme.
    pub cells: Vec<(usize, usize, usize)>,
}

/// Scan only the visible byte slice of each on-screen line. The same Layout
/// and right-edge cut-marker rule as rendering determine that slice.
fn words(ed: &Editor, vp: Viewport) -> Vec<Word> {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let re = WORD.get_or_init(|| Regex::new(r"\w+").unwrap());
    let word_char = |c: char| re.is_match(c.encode_utf8(&mut [0; 4]));
    let mut out = vec![];
    let mut y = 0;
    for line in vp.view.top..ed.line_count() {
        if y >= vp.rows {
            break;
        }
        let text = ed.buf.line(line);
        let skip = if line == vp.view.top {
            vp.view.top_row
        } else {
            0
        };
        let iter: Box<dyn Iterator<Item = Placed<'_>>> = if vp.wrap {
            Box::new(Layout::new(&text, ed.tabstop, Some(vp.cols)))
        } else {
            Box::new(Layout::from_col(&text, ed.tabstop, vp.view.left))
        };
        let mut iter = iter.filter(|p| p.width > 0).peekable();
        let mut cells = vec![];
        let mut last_row = 0;
        while let Some(p) = iter.next() {
            last_row = p.row;
            if p.row < skip {
                continue;
            }
            if y + p.row - skip >= vp.rows {
                break;
            }
            if !vp.wrap {
                if p.col < vp.view.left {
                    continue;
                }
                let end = p.col - vp.view.left + p.width;
                if end > vp.cols || (end == vp.cols && iter.peek().is_some()) {
                    break;
                }
            } else if p.x + p.width > vp.cols {
                continue;
            }
            cells.push(p);
        }
        if let (Some(first), Some(last)) = (cells.first(), cells.last()) {
            let lo = first.byte;
            let hi = last.byte + last.text.len();
            for m in re.find_iter(&text[lo..hi]) {
                let start = lo + m.start();
                let end = lo + m.end();
                // A clipped fragment must never masquerade as a whole word.
                if text[..start].chars().next_back().is_some_and(word_char)
                    || text[end..].chars().next().is_some_and(word_char)
                {
                    continue;
                }
                let placed: Vec<_> = cells
                    .iter()
                    .filter(|p| p.byte >= start && p.byte < end)
                    .collect();
                if placed.first().is_none_or(|p| p.byte != start)
                    || placed.last().is_none_or(|p| p.byte + p.text.len() != end)
                    || placed.iter().map(|p| p.text.len()).sum::<usize>() != end - start
                {
                    continue;
                }
                out.push(Word {
                    text: m.as_str().to_string(),
                    point: (line, start),
                    cells: placed
                        .into_iter()
                        .map(|p| {
                            (
                                if vp.wrap { p.x } else { p.col - vp.view.left },
                                y + p.row - skip,
                                p.width,
                            )
                        })
                        .collect(),
                });
            }
        }
        if vp.wrap && line == ed.cur.line {
            last_row = last_row.max(wrap_cursor(&text, ed.cur.byte, ed.tabstop, vp.cols).0);
        }
        y += if vp.wrap {
            (last_row + 1).saturating_sub(skip)
        } else {
            1
        };
    }
    out
}

#[derive(Debug, Default)]
pub(crate) struct Zap {
    pub query: String,
    pub label_prefix: String,
    selecting: bool,
    force_labels: bool,
    words: Vec<Word>,
}

impl Zap {
    fn matching(&self) -> Vec<&Word> {
        let sensitive = self.query.chars().any(char::is_uppercase);
        let query = self.query.to_lowercase();
        self.words
            .iter()
            .filter(|w| {
                if sensitive {
                    w.text.starts_with(&self.query)
                } else {
                    w.text.to_lowercase().starts_with(&query)
                }
            })
            .collect()
    }

    /// Prefix-free labels; their first key cannot extend any current match.
    pub fn choices(&self) -> Vec<(&Word, String)> {
        if self.query.is_empty() && !self.force_labels {
            return vec![];
        }
        let matches = self.matching();
        let sensitive = self.query.chars().any(char::is_uppercase);
        let query = if sensitive {
            self.query.clone()
        } else {
            self.query.to_lowercase()
        };
        let alphabet: Vec<char> = "asdfghjklqwertyuiopzxcvbnmASDFGHJKLQWERTYUIOPZXCVBNM"
            .chars()
            .filter(|&c| {
                self.force_labels
                    || !matches.iter().any(|w| {
                        let text = if sensitive {
                            w.text.clone()
                        } else {
                            w.text.to_lowercase()
                        };
                        text.strip_prefix(&query)
                            .and_then(|s| s.chars().next())
                            .is_some_and(|next| {
                                if sensitive {
                                    next == c
                                } else {
                                    next.eq_ignore_ascii_case(&c)
                                }
                            })
                    })
            })
            .collect();
        let base = alphabet.len();
        if base < 2 {
            return matches.into_iter().map(|w| (w, String::new())).collect();
        }
        let mut digits = 1;
        let mut capacity = base;
        while capacity < matches.len() {
            digits += 1;
            capacity *= base;
        }
        matches
            .into_iter()
            .enumerate()
            .map(|(mut i, w)| {
                let mut label = vec![alphabet[0]; digits];
                for c in label.iter_mut().rev() {
                    *c = alphabet[i % base];
                    i /= base;
                }
                (w, label.into_iter().collect())
            })
            .collect()
    }

    fn exact(&self) -> Option<(usize, usize)> {
        if self.query.is_empty() {
            return None;
        }
        let exact: Vec<_> = self
            .matching()
            .into_iter()
            .filter(|w| {
                if self.query.chars().any(char::is_uppercase) {
                    w.text == self.query
                } else {
                    w.text.to_lowercase() == self.query.to_lowercase()
                }
            })
            .collect();
        (exact.len() == 1).then(|| exact[0].point)
    }
}

pub(crate) fn open(ed: &mut Editor) {
    ed.zap = Some(Zap {
        words: ed.viewport.map_or_else(Vec::new, |vp| words(ed, vp)),
        ..Zap::default()
    });
}

pub(crate) fn key(ed: &mut Editor, k: Key) {
    let mut zap = ed.zap.take().unwrap();
    if k.is(KeyCode::Esc) || k == Key::ctrl('c') || k == Key::ctrl('g') {
        return;
    }
    let mut target = None;
    if k.is(KeyCode::Backspace) {
        if zap.selecting {
            if zap.label_prefix.pop().is_none() {
                zap.selecting = false;
                zap.force_labels = false;
            }
        } else {
            zap.query.pop();
            target = zap.exact();
        }
    } else if k.is(KeyCode::Enter) {
        zap.selecting = true;
        zap.force_labels = true;
        zap.label_prefix.clear();
    } else if let Some(c) = k.char() {
        let prefix = format!("{}{c}", zap.label_prefix);
        let choices = zap.choices();
        if zap.selecting || choices.iter().any(|(_, label)| label.starts_with(c)) {
            if choices.iter().any(|(_, label)| label.starts_with(&prefix)) {
                target = choices
                    .iter()
                    .find(|(_, label)| *label == prefix)
                    .map(|(w, _)| w.point);
                zap.selecting = true;
                zap.label_prefix = prefix;
            }
        } else {
            zap.query.push(c);
            target = zap.exact();
        }
    }
    if let Some((line, byte)) = target {
        ed.set_cursor(line, byte);
    } else {
        ed.zap = Some(zap);
    }
}

pub(crate) fn paste(ed: &mut Editor, text: &str) {
    let zap = ed.zap.as_mut().unwrap();
    zap.query.push_str(text.lines().next().unwrap_or(""));
    zap.label_prefix.clear();
    zap.selecting = false;
    zap.force_labels = false;
    if let Some((line, byte)) = zap.exact() {
        ed.set_cursor(line, byte);
        ed.zap = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::key::parse_keys;

    fn editor(text: &str, rows: usize, cols: usize, wrap: bool) -> Editor {
        let mut ed = Editor::new(Buffer::from_text(text));
        ed.viewport = Some(Viewport {
            view: View::default(),
            rows,
            cols,
            wrap,
        });
        ed
    }

    fn keys(ed: &mut Editor, text: &str) {
        for key in parse_keys(text) {
            ed.handle_key(key);
        }
    }

    #[test]
    fn visible_words_respect_clipping_wrap_and_unicode() {
        for (text, left, top_row, rows, cols, wrap, expected) in [
            ("cat dog\nhidden", 0, 0, 1, 7, false, vec!["cat", "dog"]),
            ("cat dog", 1, 0, 1, 7, false, vec!["dog"]),
            ("catX more", 0, 0, 1, 4, false, vec![]),
            ("cat more", 0, 0, 1, 4, false, vec!["cat"]),
            ("hidden zebra", 7, 0, 1, 8, false, vec!["zebra"]),
            ("alpha beta gamma", 0, 0, 2, 6, true, vec!["alpha", "beta"]),
            ("alpha beta gamma", 0, 1, 1, 6, true, vec!["beta"]),
            (
                "a\t漢字 e\u{301}",
                0,
                0,
                1,
                20,
                false,
                vec!["a", "漢字", "e\u{301}"],
            ),
            ("漢字 dog", 1, 0, 1, 10, false, vec!["dog"]),
            ("cat", 0, 0, 0, 8, false, vec![]),
            ("a漢b z", 0, 0, 8, 1, true, vec!["z"]),
        ] {
            let mut ed = editor(text, rows, cols, wrap);
            let vp = ed.viewport.as_mut().unwrap();
            vp.view.left = left;
            vp.view.top_row = top_row;
            let actual: Vec<_> = words(&ed, ed.viewport.unwrap())
                .into_iter()
                .map(|w| w.text)
                .collect();
            assert_eq!(actual, expected, "{text:?} left={left} top_row={top_row}");
        }
    }

    #[test]
    fn exact_jump_and_labels_leave_the_buffer_unchanged() {
        let mut ed = editor("first zebra zebras\nzebra", 2, 40, false);
        keys(&mut ed, " szebr");
        assert!(ed.zap.is_some()); // A unique prefix is not yet an exact word.
        assert_eq!(ed.cur.pos(), (0, 0));
        let labels: Vec<_> = ed
            .zap
            .as_ref()
            .unwrap()
            .choices()
            .into_iter()
            .map(|(_, l)| l)
            .collect();
        assert_eq!(labels, ["s", "d", "f"]); // 'a' continues the search.
        keys(&mut ed, "a");
        assert!(ed.zap.is_some()); // Two exact occurrences still need selection.
        keys(&mut ed, "d");
        assert_eq!(ed.cur.pos(), (0, 12));
        assert!(ed.zap.is_none());
        assert!(!ed.buf.modified);
        assert_eq!(ed.undo.state_id(), 0);

        let mut ed = editor("first zebra zebras\nzebra", 1, 40, false);
        keys(&mut ed, " szebra");
        assert_eq!(ed.cur.pos(), (0, 6)); // Off-screen exact occurrence is ignored.
        assert!(ed.zap.is_none());
    }

    #[test]
    fn cancel_backspace_and_paste_do_not_run_normal_commands() {
        let mut ed = editor("first naïve_naïve", 1, 40, false);
        keys(&mut ed, " sx<BS>na");
        assert_eq!(ed.zap.as_ref().unwrap().query, "na");
        keys(&mut ed, "<C-g>");
        assert!(ed.zap.is_none());
        assert_eq!(ed.cur.pos(), (0, 0));
        keys(&mut ed, " s");
        ed.paste("naïve_naïve\nignored");
        assert_eq!(ed.cur.pos(), (0, 6));
        assert!(ed.zap.is_none());
        assert!(!ed.buf.modified);
    }

    #[test]
    fn backspace_to_an_exact_pasted_query_jumps() {
        let mut ed = editor("first zebra", 1, 40, false);
        keys(&mut ed, " s");
        ed.paste("zebraa");
        keys(&mut ed, "<BS>");
        assert_eq!(ed.cur.pos(), (0, 6));
        assert!(ed.zap.is_none());
    }

    #[test]
    fn label_selection_handles_more_than_one_alphabet() {
        let mut ed = editor(&vec!["x"; 60].join("\n"), 60, 10, false);
        keys(&mut ed, " sx");
        assert_eq!(ed.zap.as_ref().unwrap().choices()[59].1, "sk");
        keys(&mut ed, "s");
        assert!(ed.zap.is_some());
        keys(&mut ed, "k");
        assert_eq!(ed.cur.pos(), (59, 0));
        assert!(ed.zap.is_none());
    }

    #[test]
    fn enter_provides_labels_when_every_letter_continues_a_word() {
        let text = ('a'..='z')
            .map(|c| format!("x{c}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut ed = editor(&text, 1, 100, false);
        keys(&mut ed, " sx");
        assert!(
            ed.zap
                .as_ref()
                .unwrap()
                .choices()
                .iter()
                .all(|(_, l)| l.is_empty())
        );
        keys(&mut ed, "<Enter>s");
        assert_eq!(ed.cur.pos(), (0, 3));
        assert!(ed.zap.is_none());
    }
}
