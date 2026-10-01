//! Words in the current buffer.

use crate::buffer::Buffer;

/// Buffers larger than this are only re-indexed when forced (entering Insert).
const AUTO_REBUILD_MAX: usize = 1024 * 1024;

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
    built: Option<u64>,
}

impl WordIndex {
    /// Re-index if the buffer changed (big buffers only when `force`).
    pub fn ensure(&mut self, buf: &Buffer, force: bool) {
        if self.built == Some(buf.version)
            || (!force && self.built.is_some() && buf.len_bytes() > AUTO_REBUILD_MAX)
        {
            return;
        }
        self.lines = (0..buf.len_lines())
            .map(|l| words(&buf.line(l)).map(String::from).collect())
            .collect();
        self.built = Some(buf.version);
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
    }

    #[test]
    fn big_buffers_rebuild_only_when_forced() {
        let mut b = Buffer::from_text(&"word ".repeat(300_000));
        let mut i = WordIndex::default();
        i.ensure(&b, true);
        b.apply(Edit {
            start: 0,
            end: 0,
            text: "fresh ".into(),
        });
        i.ensure(&b, false);
        assert_eq!(i.line_words(0)[0], "word");
        i.ensure(&b, true);
        assert_eq!(i.line_words(0)[0], "fresh");
    }
}
