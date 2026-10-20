//! Split windows: the layout tree, like Neovim's frames.

use ratatui::layout::Rect;

/// `Row` = side by side, `Col` = stacked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Row,
    Col,
}

/// A leaf is a window id; a split divides its area among its kids. Each
/// kid's `u16` is its size along `dir`, status line or divider included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Leaf(usize),
    Split { dir: Dir, kids: Vec<(Node, u16)> },
}

/// A window on screen. `rect` includes its status row (the last row);
/// `divider` means the last column is the `│` to a right-hand neighbour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: usize,
    pub rect: Rect,
    pub divider: bool,
}

/// One window: the buffer it shows (an index into `Session::bufs`), its
/// cursor (meaningful only while inactive: the active window's is
/// `Session::ed.cur`), its scroll, and its alternate buffer.
#[derive(Clone, Debug)]
pub struct Window {
    pub id: usize,
    pub buf: usize,
    pub cur: crate::editor::Cursor,
    pub view: crate::ui::View,
    pub prev: Option<usize>,
}

const E36: &str = "E36: Not enough room";
const E444: &str = "E444: Cannot close last window";

/// The smallest a window may be along `dir`: text row + status, or a column.
fn min(dir: Dir) -> u16 {
    match dir {
        Dir::Col => 2,
        Dir::Row => 1,
    }
}

/// Where each kid of a split goes in `area`: None for one left too small.
/// Sizes set for another screen size share the space in proportion.
fn rects(dir: Dir, kids: &[(Node, u16)], area: Rect) -> Vec<Option<Rect>> {
    let (start, end) = match dir {
        Dir::Col => (area.y, area.bottom()),
        Dir::Row => (area.x, area.right()),
    };
    let total: u32 = kids.iter().map(|(_, n)| u32::from(*n)).sum();
    let extent = u32::from(end - start);
    let mut pos = start;
    let mut out = Vec::with_capacity(kids.len());
    for (i, (_, size)) in kids.iter().enumerate() {
        let room = end - pos;
        let size = match total {
            0 => 0,
            t => (u32::from(*size) * extent / t) as u16,
        };
        // The last kid absorbs any rounding; the others clip to the room left.
        let n = if i + 1 == kids.len() { room } else { size.min(room) };
        if n < min(dir) {
            out.push(None);
            continue;
        }
        out.push(Some(match dir {
            Dir::Col => Rect { y: pos, height: n, ..area },
            Dir::Row => Rect { x: pos, width: n, ..area },
        }));
        pos += n;
    }
    out
}

impl Node {
    pub fn layout(&self, area: Rect) -> Vec<Placed> {
        let mut out = Vec::new();
        self.place(area, false, &mut out);
        out
    }

    fn place(&self, area: Rect, divider: bool, out: &mut Vec<Placed>) {
        let Node::Split { dir, kids } = self else {
            if let Node::Leaf(id) = self
                && area.width > 0
                && area.height > 0
            {
                out.push(Placed { id: *id, rect: area, divider });
            }
            return;
        };
        for (i, (kid, rect)) in kids.iter().zip(rects(*dir, kids, area)).enumerate() {
            let Some(rect) = rect else { continue };
            let div = divider || (*dir == Dir::Row && i + 1 < kids.len());
            kid.0.place(rect, div, out);
        }
    }

    /// Store the sizes the kids have on a screen of `area`, so a split or
    /// close after a resize works in today's cells.
    fn fit(&mut self, area: Rect) {
        let Node::Split { dir, kids } = self else { return };
        let placed = rects(*dir, kids, area);
        for ((kid, size), rect) in kids.iter_mut().zip(placed) {
            let r = rect.unwrap_or_default();
            // One squeezed out keeps its minimum, to come back when there's room.
            *size = match dir {
                Dir::Col => r.height,
                Dir::Row => r.width,
            }
            .max(min(*dir));
            kid.fit(r);
        }
    }

    pub fn order(&self) -> Vec<usize> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split { kids, .. } => kids.iter().flat_map(|(k, _)| k.order()).collect(),
        }
    }

    /// The split whose kids hold `Leaf(id)`, and its index there.
    fn parent_of(&mut self, id: usize) -> Option<(&mut Node, usize)> {
        let Node::Split { kids, .. } = self else { return None };
        if let Some(i) = kids.iter().position(|(k, _)| *k == Node::Leaf(id)) {
            return Some((self, i));
        }
        // Find the kid first so the recursive borrow is taken only once.
        let k = kids.iter().position(|(k, _)| k.order().contains(&id))?;
        let Node::Split { kids, .. } = self else { unreachable!() };
        kids[k].0.parent_of(id)
    }

    fn leaf_mut(&mut self, id: usize) -> Option<&mut Node> {
        match self {
            Node::Leaf(i) if *i == id => Some(self),
            Node::Leaf(_) => None,
            Node::Split { kids, .. } => kids.iter_mut().find_map(|(k, _)| k.leaf_mut(id)),
        }
    }

    pub fn split(
        &mut self,
        area: Rect,
        target: usize,
        new: usize,
        dir: Dir,
        after: bool,
        size: Option<u16>,
    ) -> Result<(), &'static str> {
        self.fit(area);
        let rect = self.layout(area).into_iter().find(|p| p.id == target).ok_or(E36)?.rect;
        let extent = match dir {
            Dir::Col => rect.height,
            Dir::Row => rect.width,
        };
        let n = size.unwrap_or(extent / 2);
        if n < 2 || extent < n + 2 {
            return Err(E36);
        }
        let rest = extent - n;
        let pair = |a: (Node, u16), b: (Node, u16)| if after { vec![a, b] } else { vec![b, a] };
        if let Some((Node::Split { dir: d, kids }, i)) = self.parent_of(target)
            && *d == dir
        {
            kids.splice(i..=i, pair((Node::Leaf(target), rest), (Node::Leaf(new), n)));
            return Ok(());
        }
        let leaf = self.leaf_mut(target).ok_or(E36)?;
        *leaf = Node::Split { dir, kids: pair((Node::Leaf(target), rest), (Node::Leaf(new), n)) };
        Ok(())
    }

    pub fn close(&mut self, area: Rect, id: usize) -> Result<(), &'static str> {
        self.fit(area);
        let (parent, i) = self.parent_of(id).ok_or(E444)?;
        let Node::Split { kids, .. } = parent else { unreachable!() };
        let (_, size) = kids.remove(i);
        let to = if i > 0 { i - 1 } else { 0 };
        kids[to].1 += size;
        if kids.len() == 1 {
            *parent = kids.remove(0).0;
        }
        Ok(())
    }

    pub fn only(&mut self, id: usize) {
        *self = Node::Leaf(id);
    }

    /// Step `count` windows in direction `dir` (`h j k l`) from `id`,
    /// stopping at the edge; among several neighbours take the one whose
    /// span holds `at`, the cursor's screen cell.
    pub fn neighbor(&self, area: Rect, id: usize, dir: char, count: usize, at: (u16, u16)) -> Option<usize> {
        let wins = self.layout(area);
        let mut cur = wins.iter().find(|w| w.id == id)?.rect;
        let mut found = None;
        for _ in 0..count.max(1) {
            let next = wins.iter().filter(|w| {
                let r = w.rect;
                let across = |lo: u16, len: u16, mlo: u16, mlen: u16| lo < mlo + mlen && mlo < lo + len;
                match dir {
                    'h' => r.right() == cur.x && across(r.y, r.height, cur.y, cur.height),
                    'l' => r.x == cur.right() && across(r.y, r.height, cur.y, cur.height),
                    'k' => r.bottom() == cur.y && across(r.x, r.width, cur.x, cur.width),
                    'j' => r.y == cur.bottom() && across(r.x, r.width, cur.x, cur.width),
                    _ => false,
                }
            });
            // Prefer the neighbour under the cursor, else the nearest one.
            let dist = |w: &&Placed| {
                let r = w.rect;
                let (p, lo, hi) = match dir {
                    'h' | 'l' => (at.1, r.y, r.bottom()),
                    _ => (at.0, r.x, r.right()),
                };
                if p < lo { lo - p } else if p >= hi { p + 1 - hi } else { 0 }
            };
            let Some(w) = next.min_by_key(dist) else { break };
            cur = w.rect;
            found = Some(w.id);
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn split_halves_and_orders() {
        let a = Rect::new(0, 0, 80, 24);
        let mut t = Node::Leaf(1);
        t.split(a, 1, 2, Dir::Col, false, None).unwrap(); // :split: new above
        let p = t.layout(a);
        assert_eq!(t.order(), vec![2, 1]);
        assert_eq!((p[0].rect.y, p[0].rect.height), (0, 12));
        assert_eq!((p[1].rect.y, p[1].rect.height), (12, 12));
    }
    #[test]
    fn vsplit_divider_belongs_to_left() {
        let a = Rect::new(0, 0, 81, 24);
        let mut t = Node::Leaf(1);
        t.split(a, 1, 2, Dir::Row, false, None).unwrap();
        let p = t.layout(a);
        assert!(p[0].divider && !p[1].divider);
        assert_eq!(p[0].rect.width + p[1].rect.width, 81);
    }
    #[test]
    fn close_gives_space_back_and_collapses() {
        let a = Rect::new(0, 0, 80, 24);
        let mut t = Node::Leaf(1);
        t.split(a, 1, 2, Dir::Col, true, None).unwrap();
        t.close(a, 2).unwrap();
        assert!(matches!(t, Node::Leaf(1)));
        assert_eq!(t.close(a, 1), Err("E444: Cannot close last window"));
    }
    #[test]
    fn neighbor_uses_cursor_column() {
        // [1 | 2] over 3: from 3, `k` goes to whichever is above the cursor.
        let a = Rect::new(0, 0, 80, 24);
        let mut t = Node::Leaf(3);
        t.split(a, 3, 1, Dir::Col, false, None).unwrap();
        t.split(a, 1, 2, Dir::Row, true, None).unwrap();
        assert_eq!(t.neighbor(a, 3, 'k', 1, (5, 20)), Some(1));
        assert_eq!(t.neighbor(a, 3, 'k', 1, (70, 20)), Some(2));
        assert_eq!(t.neighbor(a, 1, 'h', 1, (5, 2)), None);
    }
    #[test]
    fn layout_smaller_than_sizes() {
        let mut t = Node::Leaf(1);
        t.split(Rect::new(0, 0, 80, 24), 1, 2, Dir::Col, false, None).unwrap();
        let p = t.layout(Rect::new(0, 0, 80, 3));
        assert!(p.iter().all(|w| w.rect.height >= 2 && w.rect.bottom() <= 3));
    }
    #[test]
    fn layout_scales_with_the_screen() {
        // A 100-column screen split in half, then the terminal narrows to 50.
        let mut t = Node::Leaf(1);
        t.split(Rect::new(0, 0, 100, 30), 1, 2, Dir::Row, false, None).unwrap();
        let p = t.layout(Rect::new(0, 0, 50, 30));
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].rect.width, p[1].rect.width), (25, 25));
    }
    #[test]
    fn split_after_resize_keeps_proportions() {
        let mut t = Node::Leaf(1);
        t.split(Rect::new(0, 0, 100, 30), 1, 2, Dir::Row, false, None).unwrap();
        let a = Rect::new(0, 0, 50, 30);
        t.split(a, 2, 3, Dir::Row, false, None).unwrap();
        let w: Vec<u16> = t.layout(a).iter().map(|p| p.rect.width).collect();
        assert_eq!(w, vec![12, 13, 25]);
    }
    #[test]
    fn split_too_small() {
        let a = Rect::new(0, 0, 80, 3);
        let mut t = Node::Leaf(1);
        assert_eq!(t.split(a, 1, 2, Dir::Col, false, None), Err("E36: Not enough room"));
    }
}
