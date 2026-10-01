//! The text buffer. Every change goes through `Buffer::apply`.

use ropey::Rope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// Replace chars `start..end` with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Text stored with `\n` line endings and no trailing terminator; the file's
/// own line ending, final newline and BOM are remembered and restored on write.
#[derive(Clone, Debug)]
pub struct Buffer {
    rope: Rope,
    pub line_ending: LineEnding,
    pub mixed_endings: bool,
    pub final_newline: bool,
    pub bom: bool,
    pub modified: bool,
    pub edits_since_swap: usize,
    /// Increments on every edit.
    pub version: u64,
    dirty_from: Option<usize>,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::from_text("")
    }
}

impl Buffer {
    pub fn from_text(s: &str) -> Buffer {
        let (bom, s) = match s.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let crlf = s.matches("\r\n").count();
        let bare = s.matches('\n').count() - crlf;
        let line_ending = if crlf > bare {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        let mut text = if crlf > 0 {
            s.replace("\r\n", "\n")
        } else {
            s.to_string()
        };
        let final_newline = text.ends_with('\n');
        if final_newline {
            text.pop();
        }
        Buffer {
            rope: Rope::from_str(&text),
            line_ending,
            mixed_endings: crlf > 0 && bare > 0,
            final_newline,
            bom,
            modified: false,
            edits_since_swap: 0,
            version: 0,
            dirty_from: None,
        }
    }

    /// The file contents as they should be written to disk.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(self.rope.len_bytes() + 16);
        if self.bom {
            out.push('\u{feff}');
        }
        let nl = self.line_ending.as_str();
        for (i, chunk) in self.text().split('\n').enumerate() {
            if i > 0 {
                out.push_str(nl);
            }
            out.push_str(chunk);
        }
        if self.final_newline {
            out.push_str(nl);
        }
        out.into_bytes()
    }

    /// Apply an edit and return its inverse.
    pub fn apply(&mut self, e: Edit) -> Edit {
        let was_empty = self.rope.len_chars() == 0;
        let removed = self.rope.slice(e.start..e.end).to_string();
        self.rope.remove(e.start..e.end);
        self.rope.insert(e.start, &e.text);
        if was_empty && self.rope.len_chars() > 0 {
            // A zero-byte file that gains text gets a final newline, like vim.
            self.final_newline = true;
        }
        let line = self.rope.char_to_line(e.start);
        self.dirty_from = Some(self.dirty_from.map_or(line, |d| d.min(line)));
        self.modified = true;
        self.edits_since_swap += 1;
        self.version += 1;
        Edit {
            start: e.start,
            end: e.start + e.text.chars().count(),
            text: removed,
        }
    }

    /// First line changed since the last call, if any.
    pub fn take_dirty_from(&mut self) -> Option<usize> {
        self.dirty_from.take()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Line `i` without its newline.
    pub fn line(&self, i: usize) -> String {
        let mut s = self.rope.line(i).to_string();
        if s.ends_with('\n') {
            s.pop();
        }
        s
    }

    /// Byte length of line `i` without its newline.
    pub fn line_len(&self, i: usize) -> usize {
        let l = self.rope.line(i);
        let n = l.len_bytes();
        if n > 0 && l.byte(n - 1) == b'\n' {
            n - 1
        } else {
            n
        }
    }

    pub fn line_to_char(&self, i: usize) -> usize {
        self.rope.line_to_char(i)
    }

    pub fn char_to_line(&self, c: usize) -> usize {
        self.rope.char_to_line(c)
    }

    pub fn pos_to_char(&self, line: usize, byte: usize) -> usize {
        self.rope.byte_to_char(self.rope.line_to_byte(line) + byte)
    }

    pub fn char_to_pos(&self, c: usize) -> (usize, usize) {
        let line = self.rope.char_to_line(c);
        (
            line,
            self.rope.char_to_byte(c) - self.rope.line_to_byte(line),
        )
    }

    /// Char index just past the end of line `i` (before its newline).
    pub fn line_end_char(&self, i: usize) -> usize {
        self.pos_to_char(i, self.line_len(i))
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        self.rope.slice(start..end).to_string()
    }

    /// The edit replacing `remove` lines at `at` with `insert` lines, and the
    /// number of lines that take their place (a buffer never has zero lines).
    pub fn splice_edit(
        &self,
        at: usize,
        remove: usize,
        insert: &[String],
    ) -> Option<(Edit, usize)> {
        let n = self.len_lines();
        let joined = insert.join("\n");
        let edit = if remove > 0 && !insert.is_empty() {
            Edit {
                start: self.line_to_char(at),
                end: self.line_end_char(at + remove - 1),
                text: joined,
            }
        } else if remove > 0 {
            if at + remove < n {
                Edit {
                    start: self.line_to_char(at),
                    end: self.line_to_char(at + remove),
                    text: String::new(),
                }
            } else if at > 0 {
                Edit {
                    start: self.line_end_char(at - 1),
                    end: self.len_chars(),
                    text: String::new(),
                }
            } else {
                Edit {
                    start: 0,
                    end: self.len_chars(),
                    text: String::new(),
                }
            }
        } else if insert.is_empty() {
            return None;
        } else if at < n {
            Edit {
                start: self.line_to_char(at),
                end: self.line_to_char(at),
                text: joined + "\n",
            }
        } else {
            let end = self.len_chars();
            Edit {
                start: end,
                end,
                text: format!("\n{joined}"),
            }
        };
        let inserted = if remove == n && insert.is_empty() {
            1
        } else {
            insert.len()
        };
        Some((edit, inserted))
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_final_newline_roundtrip() {
        for s in [
            "a\r\nb\r\n",
            "a\nb",
            "\u{feff}x\n",
            "",
            "\n",
            "a\r\nb",
            "x\n\n",
        ] {
            assert_eq!(Buffer::from_text(s).to_bytes(), s.as_bytes(), "{s:?}");
        }
    }

    #[test]
    fn apply_returns_inverse() {
        let mut b = Buffer::from_text("hello\nworld\n");
        let inv = b.apply(Edit {
            start: 0,
            end: 5,
            text: "bye".into(),
        });
        assert_eq!(b.line(0), "bye");
        assert!(b.modified);
        b.apply(inv);
        assert_eq!(b.text(), "hello\nworld");
    }

    #[test]
    fn empty_buffer_has_one_line() {
        assert_eq!(Buffer::from_text("").len_lines(), 1);
        assert_eq!(Buffer::from_text("a\nb\n").len_lines(), 2);
    }

    #[test]
    fn positions() {
        let b = Buffer::from_text("ab\n漢x");
        assert_eq!(b.pos_to_char(1, 3), 4);
        assert_eq!(b.char_to_pos(4), (1, 3));
        assert_eq!(b.line_len(1), 4);
    }

    #[test]
    fn only_newline_splits_lines() {
        // Form feed, lone CR, VT, NEL and U+2028/9 are ordinary characters.
        for s in ["a\x0cb\nc", "a\rb\nc", "a\x0bb\nc", "a\u{85}b\nc", "a\u{2028}b\nc", "a\u{2029}b\nc"] {
            let b = Buffer::from_text(s);
            assert_eq!(b.len_lines(), 2, "{s:?}");
            assert_eq!(b.line(1), "c", "{s:?}");
        }
    }

    #[test]
    fn dirty_from_tracks_first_changed_line() {
        let mut b = Buffer::from_text("a\nb\nc");
        b.apply(Edit {
            start: 4,
            end: 4,
            text: "x".into(),
        });
        b.apply(Edit {
            start: 2,
            end: 2,
            text: "y".into(),
        });
        assert_eq!(b.take_dirty_from(), Some(1));
        assert_eq!(b.take_dirty_from(), None);
    }
}
