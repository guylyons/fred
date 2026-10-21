//! Words in the current buffer.

use crate::buffer::Buffer;

/// Buffers larger than this are only re-indexed when forced (entering Insert).
const AUTO_REBUILD_MAX: usize = 1024 * 1024;
/// Buffers larger than this are never indexed: a 2 GB dump took a minute,
/// on the first key typed. Completion still offers nearby files' words.
const INDEX_MAX: usize = 10 * 1024 * 1024;

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Words of at least 3 characters in `text`, in order.
pub fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !is_word_char(c))
        .filter(|w| w.chars().count() >= 3)
}

#[derive(Debug, Default)]
pub struct WordIndex {
    lines: Vec<Vec<String>>,
    /// The buffer (id, version) indexed.
    built: Option<(u64, u64)>,
    /// The last completion's prefix and every indexed word it matched
    /// (`super::candidates`), and words of lines re-read since: what a
    /// longer prefix can match.
    pub(super) memo: Option<(String, Vec<String>)>,
    pub(super) fresh: Vec<String>,
}

fn line_words(buf: &Buffer, l: usize) -> Vec<String> {
    words(&buf.line(l)).map(String::from).collect()
}

impl WordIndex {
    /// Catch up with the buffer: just the edited lines when it remembers
    /// them, else re-index (big buffers only when `force`, huge ones never).
    pub fn ensure(&mut self, buf: &Buffer, force: bool) {
        if buf.len_bytes() > INDEX_MAX {
            *self = WordIndex::default();
            return;
        }
        if let Some((id, v)) = self.built
            && id == buf.id
            && let Some(splices) = buf.splices_since(v)
        {
            // The lines to re-read, in the latest text so far.
            let mut dirty: Option<(usize, usize)> = None;
            for &(first, removed, inserted) in splices {
                let end = (first + removed + 1).min(self.lines.len());
                let new = std::iter::repeat_with(Vec::new).take(inserted + 1);
                self.lines.splice(first.min(end)..end, new);
                let (lo, hi) = dirty.unwrap_or((first, first));
                let hi = if hi > first + removed {
                    hi + inserted - removed
                } else {
                    hi.max(first + inserted)
                };
                dirty = Some((lo.min(first), hi));
            }
            if let Some((lo, hi)) = dirty {
                for l in lo..=hi.min(self.lines.len().saturating_sub(1)) {
                    self.lines[l] = line_words(buf, l);
                    // Only a memo can use them.
                    if self.memo.is_some() {
                        self.fresh.extend_from_slice(&self.lines[l]);
                    }
                }
            }
            self.built = Some((id, buf.version));
            return;
        }
        if !force && self.built.is_some() && buf.len_bytes() > AUTO_REBUILD_MAX {
            return;
        }
        self.lines = (0..buf.len_lines()).map(|l| line_words(buf, l)).collect();
        self.built = Some((buf.id, buf.version));
        self.memo = None;
        self.fresh.clear();
    }

    pub fn line_words(&self, line: usize) -> &[String] {
        self.lines.get(line).map_or(&[], Vec::as_slice)
    }

    pub fn len_lines(&self) -> usize {
        self.lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Edit};

    #[test]
    fn words_and_rebuild() {
        let mut b = Buffer::from_text("foo bar_baz x9 ab\nnaïve 漢字漢字");
        let mut i = WordIndex::default();
        i.ensure(&b, true);
        assert_eq!(i.line_words(0), ["foo", "bar_baz"]);
        assert_eq!(i.line_words(1), ["naïve", "漢字漢字"]);
        b.apply(Edit {
            start: 0,
            end: 0,
            text: "zebra ".into(),
        });
        i.ensure(&b, false);
        assert_eq!(i.line_words(0), ["zebra", "foo", "bar_baz"]);
        // Huge buffers are never indexed, forced or not.
        let huge = Buffer::from_text(&"word ".repeat(INDEX_MAX / 4));
        i.ensure(&huge, true);
        assert_eq!(i.len_lines(), 0);
    }

    #[test]
    fn edits_update_just_their_lines() {
        // Big: a re-index would wait for `force`; the edit's lines don't.
        let mut b = Buffer::from_text(&"word ".repeat(300_000));
        let mut i = WordIndex::default();
        i.ensure(&b, true);
        b.apply(Edit {
            start: 0,
            end: 0,
            text: "fresh ".into(),
        });
        i.ensure(&b, false);
        assert_eq!(i.line_words(0)[0], "fresh");
        // Several edits between looks, lines in and out: as a re-index.
        let mut b = Buffer::from_text(
            &(0..40)
                .map(|n| format!("w{n:03}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let mut i = WordIndex::default();
        i.ensure(&b, true);
        let mut seed = 7usize;
        for round in 0..200 {
            for _ in 0..round % 4 + 1 {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let len = b.len_chars();
                let start = (seed >> 33) % (len + 1);
                let end = (start + (seed >> 20) % 9).min(len);
                let text = ["", "\n", "abc", "xyz\nqq", "\nnew\n", "zz"][(seed >> 8) % 6];
                b.apply(Edit {
                    start,
                    end,
                    text: text.into(),
                });
            }
            i.ensure(&b, false);
            let mut full = WordIndex::default();
            full.ensure(&b, true);
            assert_eq!(i.lines, full.lines, "round {round}");
        }
        // Too far behind to catch up: re-indexed.
        for _ in 0..2000 {
            b.apply(Edit {
                start: 0,
                end: 0,
                text: "x".into(),
            });
        }
        i.ensure(&b, true);
        let mut full = WordIndex::default();
        full.ensure(&b, true);
        assert_eq!(i.lines, full.lines);
    }
}
