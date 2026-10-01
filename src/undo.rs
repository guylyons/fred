//! Linear undo/redo of edit groups.

use crate::buffer::{Buffer, Edit};

type Pos = (usize, usize);

#[derive(Debug)]
struct Group {
    id: u64,
    /// Inverse edits, in the order they were recorded.
    edits: Vec<Edit>,
    before: Pos,
    after: Pos,
}

#[derive(Debug, Default)]
pub struct Undo {
    undo: Vec<Group>,
    redo: Vec<Group>,
    open: Option<Group>,
    depth: usize,
    next_id: u64,
}

impl Undo {
    /// Start a group. Nested calls join the outer group.
    pub fn begin(&mut self, cursor: Pos) {
        if self.depth == 0 {
            self.next_id += 1;
            self.open = Some(Group { id: self.next_id, edits: vec![], before: cursor, after: cursor });
        }
        self.depth += 1;
    }

    /// Record the inverse of an applied edit.
    pub fn record(&mut self, inverse: Edit) {
        match &mut self.open {
            Some(g) => g.edits.push(inverse),
            None => {
                self.begin((0, 0));
                self.record(inverse);
                self.depth = 1;
                self.end((0, 0));
            }
        }
    }

    pub fn end(&mut self, cursor: Pos) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth > 0 {
            return;
        }
        if let Some(mut g) = self.open.take()
            && !g.edits.is_empty()
        {
            g.after = cursor;
            self.undo.push(g);
            self.redo.clear();
        }
    }

    pub fn in_group(&self) -> bool {
        self.depth > 0
    }

    /// Undo the last group; returns the cursor to restore.
    pub fn undo(&mut self, buf: &mut Buffer) -> Option<Pos> {
        let g = self.undo.pop()?;
        let redo = Self::replay(buf, &g.edits);
        let before = g.before;
        self.redo.push(Group { id: g.id, edits: redo, before: g.before, after: g.after });
        Some(before)
    }

    pub fn redo(&mut self, buf: &mut Buffer) -> Option<Pos> {
        let g = self.redo.pop()?;
        let undo = Self::replay(buf, &g.edits);
        let after = g.after;
        self.undo.push(Group { id: g.id, edits: undo, before: g.before, after: g.after });
        Some(after)
    }

    /// Apply inverses newest-first and return their inverses in replay order.
    fn replay(buf: &mut Buffer, edits: &[Edit]) -> Vec<Edit> {
        edits.iter().rev().map(|e| buf.apply(e.clone())).collect()
    }

    /// Identifies the current position in history; equal ids mean equal text.
    pub fn state_id(&self) -> u64 {
        self.undo.last().map_or(0, |g| g.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Edit};

    fn ed(start: usize, end: usize, t: &str) -> Edit {
        Edit { start, end, text: t.into() }
    }

    #[test]
    fn undo_redo_group() {
        let mut b = Buffer::from_str("abc");
        let mut u = Undo::default();
        u.begin((0, 0));
        u.record(b.apply(ed(0, 1, "X")));
        u.record(b.apply(ed(1, 2, "Y")));
        u.end((0, 1));
        assert_eq!(b.text(), "XYc");
        assert_eq!(u.undo(&mut b), Some((0, 0)));
        assert_eq!(b.text(), "abc");
        assert_eq!(u.redo(&mut b), Some((0, 1)));
        assert_eq!(b.text(), "XYc");
        assert_eq!(u.redo(&mut b), None);
    }

    #[test]
    fn empty_group_dropped() {
        let mut b = Buffer::from_str("a");
        let mut u = Undo::default();
        u.begin((0, 0));
        u.end((0, 0));
        assert_eq!(u.undo(&mut b), None);
    }

    #[test]
    fn nested_begin_is_one_group() {
        let mut b = Buffer::from_str("abc");
        let mut u = Undo::default();
        u.begin((0, 0));
        u.begin((0, 0));
        u.record(b.apply(ed(0, 1, "X")));
        u.end((0, 0));
        u.record(b.apply(ed(1, 2, "Y")));
        u.end((0, 0));
        u.undo(&mut b);
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut b = Buffer::from_str("abc");
        let mut u = Undo::default();
        u.begin((0, 0));
        u.record(b.apply(ed(0, 1, "X")));
        u.end((0, 0));
        u.undo(&mut b);
        u.begin((0, 0));
        u.record(b.apply(ed(0, 1, "Z")));
        u.end((0, 0));
        assert_eq!(u.redo(&mut b), None);
        assert_eq!(b.text(), "Zbc");
    }

    #[test]
    fn state_id_returns_after_undo_redo() {
        let mut b = Buffer::from_str("abc");
        let mut u = Undo::default();
        let saved = u.state_id();
        u.begin((0, 0));
        u.record(b.apply(ed(0, 1, "X")));
        u.end((0, 0));
        let changed = u.state_id();
        assert_ne!(saved, changed);
        u.undo(&mut b);
        assert_eq!(u.state_id(), saved);
        u.redo(&mut b);
        assert_eq!(u.state_id(), changed);
    }
}
