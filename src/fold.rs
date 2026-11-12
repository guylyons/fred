//! Hidden line ranges (Org visibility, closed folds) and Vim's folds for code.

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

/// One line's fold level, as a 'foldexpr' gives it: `start` is Vim's `>N`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub n: usize,
    pub start: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fold {
    start: usize,
    end: usize,
    /// 1 for an outermost fold.
    depth: usize,
    /// Opened or closed by hand; None follows 'foldlevel'.
    open: Option<bool>,
}

/// 'foldenable', 'foldlevel' and 'foldminlines'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldOpts {
    pub enable: bool,
    pub level: usize,
    pub minlines: usize,
}

/// Vim's folds for a code buffer ('foldmethod' expr or indent): nested line
/// ranges computed from line levels, each open or closed.
#[derive(Clone, Debug, Default)]
pub struct CodeFolds {
    /// Sorted by start, an outer fold before the ones inside it.
    folds: Vec<Fold>,
    /// The buffer (id, version) the folds were computed from.
    pub synced: Option<(u64, u64)>,
}

impl CodeFolds {
    /// Rebuild the folds from line levels. A fold where one of the same depth
    /// was stays opened or closed by hand.
    pub fn set_levels(&mut self, levels: &[Level]) {
        let old = std::mem::take(&mut self.folds);
        let mut stack: Vec<usize> = vec![];
        for (i, lv) in levels.iter().enumerate() {
            // `>N` ends a fold of level N that is still running.
            let keep = if lv.start && lv.n > 0 && stack.len() >= lv.n {
                lv.n - 1
            } else {
                lv.n
            };
            while stack.len() > keep {
                self.folds[stack.pop().unwrap()].end = i - 1;
            }
            while stack.len() < lv.n {
                let depth = stack.len() + 1;
                // As Vim, the fold that was here keeps its state.
                let open = old
                    .iter()
                    .find(|f| f.start <= i && i <= f.end && f.depth == depth)
                    .and_then(|f| f.open);
                stack.push(self.folds.len());
                self.folds.push(Fold {
                    start: i,
                    end: i,
                    depth,
                    open,
                });
            }
        }
        for f in stack {
            self.folds[f].end = levels.len() - 1;
        }
    }

    fn closed(f: &Fold, o: FoldOpts) -> bool {
        // A fold of 'foldminlines' lines or fewer never shows closed.
        o.enable && f.end - f.start + 1 > o.minlines && !f.open.unwrap_or(f.depth <= o.level)
    }

    /// The closed folds that show, outermost only: (first line, last line).
    pub fn closed_ranges(&self, o: FoldOpts) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = vec![];
        for f in self.folds.iter().filter(|f| Self::closed(f, o)) {
            if out.last().is_none_or(|&(_, e)| f.start > e) {
                out.push((f.start, f.end));
            }
        }
        out
    }

    /// The folds holding line `l`, outermost first.
    fn at(&self, l: usize) -> Vec<usize> {
        (0..self.folds.len())
            .filter(|&i| self.folds[i].start <= l && l <= self.folds[i].end)
            .collect()
    }

    /// `zo`: open the outermost closed fold at `l`, `count` times; `zO`
    /// (`recurse`) opens everything inside it too. False when `l` has no fold.
    pub fn open(&mut self, l: usize, count: usize, recurse: bool, o: FoldOpts) -> bool {
        let at = self.at(l);
        for _ in 0..count.max(1) {
            let Some(&i) = at.iter().find(|&&i| Self::closed(&self.folds[i], o)) else {
                break;
            };
            self.folds[i].open = Some(true);
            if recurse {
                let (s, e) = (self.folds[i].start, self.folds[i].end);
                for f in self.folds.iter_mut().filter(|f| s <= f.start && f.end <= e) {
                    f.open = Some(true);
                }
            }
        }
        !at.is_empty()
    }

    /// `zc`: close the innermost fold at `l` that shows open, `count` times;
    /// `zC` (`recurse`) closes every fold at `l`. False when `l` has no fold.
    pub fn close(&mut self, l: usize, count: usize, recurse: bool, o: FoldOpts) -> bool {
        let at = self.at(l);
        if recurse {
            for &i in &at {
                self.folds[i].open = Some(false);
            }
        }
        for _ in 0..if recurse { 0 } else { count.max(1) } {
            // The fold above the outermost closed one, else the innermost.
            let shut = at.iter().position(|&i| Self::closed(&self.folds[i], o));
            match shut.map_or(at.last(), |p| p.checked_sub(1).map(|p| &at[p])) {
                Some(&i) => self.folds[i].open = Some(false),
                None => break,
            }
        }
        !at.is_empty()
    }

    /// The closed fold showing at `l`: (first line, last line).
    pub fn closed_at(&self, l: usize, o: FoldOpts) -> Option<(usize, usize)> {
        self.at(l)
            .into_iter()
            .map(|i| &self.folds[i])
            .find(|f| Self::closed(f, o))
            .map(|f| (f.start, f.end))
    }

    /// `zv`: open folds until line `l` shows.
    pub fn view(&mut self, l: usize, o: FoldOpts) {
        for i in self.at(l) {
            if Self::closed(&self.folds[i], o) {
                self.folds[i].open = Some(true);
            }
        }
    }

    /// `zX`: every fold follows 'foldlevel' again.
    pub fn reset(&mut self) {
        for f in &mut self.folds {
            f.open = None;
        }
    }

    /// The deepest nesting (`zR`'s 'foldlevel').
    pub fn deepest(&self) -> usize {
        self.folds.iter().map(|f| f.depth).max().unwrap_or(0)
    }

    /// The fold level of line `l`.
    pub fn level(&self, l: usize) -> usize {
        self.at(l).len()
    }

    /// `zj`: the start of the next fold below `l`.
    pub fn next_start(&self, l: usize) -> Option<usize> {
        self.folds.iter().map(|f| f.start).find(|&s| s > l)
    }

    /// `zk`: the end of the nearest fold above `l`.
    pub fn prev_end(&self, l: usize) -> Option<usize> {
        self.folds.iter().map(|f| f.end).filter(|&e| e < l).max()
    }

    /// `[z` and `]z`: the start and end of the innermost fold at `l`, or of
    /// the fold around it when `l` is already there (`end` picks which).
    pub fn edge(&self, l: usize, end: bool) -> Option<usize> {
        let edge = |i: &usize| if end { self.folds[*i].end } else { self.folds[*i].start };
        self.at(l).iter().rev().map(edge).find(|&e| e != l)
    }

    /// Lines `at..at + removed` were replaced by `inserted` lines: folds
    /// move with their lines until the levels are computed again.
    pub fn line_change(&mut self, at: usize, removed: usize, inserted: usize) {
        let end = at + removed;
        self.folds.retain_mut(|f| {
            if f.start >= end {
                f.start = f.start + inserted - removed;
            } else if f.start >= at && f.start - at >= inserted {
                return false;
            }
            if f.end >= end {
                f.end = f.end + inserted - removed;
            } else if f.end >= at {
                f.end = (at + inserted).saturating_sub(1);
            }
            f.start <= f.end
        });
    }
}

/// 'foldmethod' indent: a line's level is its indent in 'shiftwidth's, at
/// most `nestmax`; a blank line takes the lower level of its neighbours.
pub fn indent_levels(lines: impl Iterator<Item = String>, sw: usize, ts: usize, nestmax: usize) -> Vec<Level> {
    let mut out: Vec<Option<usize>> = lines
        .map(|line| {
            let mut col = 0;
            for c in line.chars() {
                match c {
                    ' ' => col += 1,
                    '\t' => col += ts - col % ts,
                    _ => return Some((col / sw.max(1)).min(nestmax)),
                }
            }
            None
        })
        .collect();
    let mut below = 0;
    for i in (0..out.len()).rev() {
        match out[i] {
            Some(n) => below = n,
            None => out[i] = Some(below.min(out[..i].iter().rev().find_map(|n| *n).unwrap_or(0))),
        }
    }
    out.into_iter()
        .map(|n| Level {
            n: n.unwrap_or(0),
            start: false,
        })
        .collect()
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

    fn lv(spec: &str) -> Vec<Level> {
        spec.split_whitespace()
            .map(|t| Level {
                n: t.trim_start_matches('>').parse().unwrap(),
                start: t.starts_with('>'),
            })
            .collect()
    }
    const O: FoldOpts = FoldOpts {
        enable: true,
        level: 1,
        minlines: 1,
    };

    #[test]
    fn levels_nest_and_foldlevel_closes_the_deep_ones() {
        let mut f = CodeFolds::default();
        // 0: a{  1-3: b{}  4-6: c{} (starts where b's level continues)  7: }  8: top
        f.set_levels(&lv(">1 >2 2 2 >2 2 2 1 0"));
        assert_eq!(f.closed_ranges(O), [(1, 3), (4, 6)]);
        assert_eq!(f.closed_ranges(FoldOpts { level: 0, ..O }), [(0, 7)]);
        assert!(f.closed_ranges(FoldOpts { level: 2, ..O }).is_empty());
        assert!(f.closed_ranges(FoldOpts { enable: false, ..O }).is_empty());
        // Three lines are not more than 'foldminlines' 3: they show open.
        assert!(f.closed_ranges(FoldOpts { minlines: 3, ..O }).is_empty());
        assert_eq!((f.level(2), f.level(8), f.deepest()), (2, 0, 2));
    }

    #[test]
    fn open_close_toggle_like_vim() {
        let mut f = CodeFolds::default();
        f.set_levels(&lv(">1 >2 2 2 >2 2 2 1 0"));
        assert!(f.open(2, 1, false, O));
        assert_eq!(f.closed_ranges(O), [(4, 6)]);
        // zc on an open inner fold closes it; again closes the one around it.
        f.close(2, 1, false, O);
        assert_eq!(f.closed_ranges(O), [(1, 3), (4, 6)]);
        f.close(2, 1, false, O);
        assert_eq!(f.closed_ranges(O), [(0, 7)]);
        // zo opens only the outermost closed fold; zO everything in it.
        f.open(2, 1, false, O);
        assert_eq!(f.closed_ranges(O), [(1, 3), (4, 6)]);
        f.close(0, 1, false, O);
        f.open(0, 1, true, O);
        assert!(f.closed_ranges(O).is_empty());
        f.close(5, 1, true, O);
        assert_eq!(f.closed_ranges(O), [(0, 7)]);
        f.view(5, O);
        assert!(f.closed_ranges(O).is_empty());
        f.reset();
        assert_eq!(f.closed_ranges(O), [(1, 3), (4, 6)]);
        assert!(!f.open(8, 1, false, O), "no fold on line 8");
    }

    #[test]
    fn hand_set_folds_survive_recomputing_and_edits() {
        let mut f = CodeFolds::default();
        f.set_levels(&lv(">1 >2 2 2 1"));
        f.open(1, 1, false, O);
        f.line_change(0, 0, 2);
        assert_eq!(f.closed_at(3, FoldOpts { level: 0, ..O }), Some((2, 6)));
        f.set_levels(&lv("0 0 >1 >2 2 2 1"));
        assert!(f.closed_ranges(O).is_empty(), "the fold opened by hand stays open");
        f.reset();
        assert_eq!(f.closed_ranges(O), [(3, 5)]);
    }

    #[test]
    fn fold_motions() {
        let mut f = CodeFolds::default();
        f.set_levels(&lv("0 >1 >2 2 1 0 >1 1"));
        assert_eq!((f.next_start(0), f.next_start(2), f.next_start(6)), (Some(1), Some(6), None));
        assert_eq!((f.prev_end(6), f.prev_end(4), f.prev_end(1)), (Some(4), Some(3), None));
        assert_eq!((f.edge(3, false), f.edge(2, false), f.edge(3, true)), (Some(2), Some(1), Some(4)));
    }

    #[test]
    fn indent_levels_follow_shiftwidth_and_blank_lines() {
        let l = indent_levels(
            ["a", "    b", "", "        c", "\td", "", "e"].into_iter().map(String::from),
            4,
            8,
            1,
        );
        assert_eq!(l.iter().map(|l| l.n).collect::<Vec<_>>(), [0, 1, 1, 1, 1, 0, 0]);
    }
}
