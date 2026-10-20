//! Hidden line ranges (Org visibility), kept in step with edits.

/// Sorted, disjoint, non-adjacent inclusive ranges of hidden lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folds(Vec<(usize, usize)>);

impl Folds {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn ranges(&self) -> &[(usize, usize)] {
        &self.0
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// The hidden range containing `l`.
    pub fn range_at(&self, l: usize) -> Option<(usize, usize)> {
        let i = self.0.partition_point(|&(_, e)| e < l);
        self.0.get(i).copied().filter(|&(s, _)| s <= l)
    }

    pub fn hidden(&self, l: usize) -> bool {
        self.range_at(l).is_some()
    }

    /// The line after `l` starts a hidden range: `l` shows an ellipsis.
    pub fn folded_after(&self, l: usize) -> bool {
        !self.hidden(l) && self.hidden(l + 1)
    }

    /// Hide lines `s..=e`.
    pub fn hide(&mut self, s: usize, e: usize) {
        if s > e {
            return;
        }
        let (mut s, mut e) = (s, e);
        self.0.retain(|&(a, b)| {
            // Overlapping or adjacent ranges merge into the new one.
            if a <= e.saturating_add(1) && s <= b.saturating_add(1) {
                s = s.min(a);
                e = e.max(b);
                false
            } else {
                true
            }
        });
        let i = self.0.partition_point(|&(a, _)| a < s);
        self.0.insert(i, (s, e));
    }

    /// Show lines `s..=e`.
    pub fn show(&mut self, s: usize, e: usize) {
        if s > e {
            return;
        }
        let mut out = Vec::with_capacity(self.0.len() + 1);
        for &(a, b) in &self.0 {
            if b < s || a > e {
                out.push((a, b));
                continue;
            }
            if a < s {
                out.push((a, s - 1));
            }
            if b > e {
                out.push((e + 1, b));
            }
        }
        self.0 = out;
    }

    /// The first visible line at or after `l` (`l` itself when none is).
    pub fn next_visible(&self, l: usize, n: usize) -> usize {
        match self.range_at(l) {
            Some((_, e)) if e + 1 < n => e + 1,
            Some((s, _)) => s.saturating_sub(1),
            None => l,
        }
    }

    /// The first visible line at or before `l`.
    pub fn prev_visible(&self, l: usize) -> usize {
        match self.range_at(l) {
            Some((s, _)) if s > 0 => s - 1,
            Some((_, e)) => e + 1,
            None => l,
        }
    }

    /// `count` visible lines down from `l`, stopping at the last one.
    pub fn down(&self, l: usize, count: usize, n: usize) -> usize {
        let mut l = l;
        for _ in 0..count {
            let next = self.next_visible(l + 1, n);
            if l + 1 >= n || next <= l {
                break;
            }
            l = next;
        }
        l
    }

    /// `count` visible lines up from `l`, stopping at the first one.
    pub fn up(&self, l: usize, count: usize) -> usize {
        let mut l = l;
        for _ in 0..count {
            if l == 0 {
                break;
            }
            let prev = self.prev_visible(l - 1);
            if prev >= l {
                break;
            }
            l = prev;
        }
        l
    }

    /// Lines `at..at+removed` were replaced by `inserted` lines. Lines
    /// inserted strictly inside a hidden range stay hidden.
    pub fn line_change(&mut self, at: usize, removed: usize, inserted: usize) {
        let end = at + removed;
        let old = std::mem::take(&mut self.0);
        for (s, e) in old {
            let ns = if s < at {
                s
            } else if s < end {
                at + inserted
            } else {
                s + inserted - removed
            };
            let ne = if e < at {
                Some(e)
            } else if e < end {
                at.checked_sub(1)
            } else {
                Some(e + inserted - removed)
            };
            if let Some(ne) = ne.filter(|&ne| ns <= ne) {
                self.hide(ns, ne);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_show_merge_split() {
        let mut f = Folds::default();
        f.hide(3, 5);
        f.hide(6, 8);
        assert_eq!(f.ranges(), &[(3, 8)]);
        f.show(5, 6);
        assert_eq!(f.ranges(), &[(3, 4), (7, 8)]);
        assert!(f.hidden(4) && !f.hidden(5) && f.folded_after(2));
        assert_eq!(f.down(2, 1, 20), 5);
        assert_eq!(f.down(5, 1, 20), 6);
        assert_eq!(f.down(6, 1, 20), 9);
        assert_eq!(f.up(9, 2), 5);
        assert_eq!(f.down(9, 1, 9), 9, "the last line stays");
    }

    #[test]
    fn edits_shift_and_clip() {
        let mut f = Folds::default();
        f.hide(3, 5);
        f.line_change(0, 0, 2);
        assert_eq!(f.ranges(), &[(5, 7)]);
        f.line_change(6, 1, 0);
        assert_eq!(f.ranges(), &[(5, 6)]);
        // Insertion inside the hidden range stays hidden.
        f.line_change(6, 0, 3);
        assert_eq!(f.ranges(), &[(5, 9)]);
        // Removing the whole range drops it.
        f.line_change(5, 5, 0);
        assert!(f.is_empty());
        // Lines inserted where the range starts are visible.
        f.hide(3, 4);
        f.line_change(3, 0, 1);
        assert_eq!(f.ranges(), &[(4, 5)]);
    }
}
