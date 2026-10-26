//! The undo tree, after Neovim's `undo.c`: every change is a node below
//! the text state it was made in, so undoing and then changing again
//! starts a branch instead of dropping the undone changes. `u` and CTRL-R
//! move along the current branch; `g-`, `g+`, `:earlier`, `:later` and
//! `:undo N` move through the states in the order they were made, across
//! branches.

use crate::buffer::{Buffer, Edit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Pos = (usize, usize);

#[derive(Debug)]
struct Group {
    /// Inverse edits, in the order they were recorded.
    edits: Vec<Edit>,
    before: Pos,
    /// `:undojoin`: the node this group adds to.
    join: Option<u64>,
}

/// A change, numbered by when it was made (Vim's `uh_seq`); node 0 is the
/// text before the first change.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Node {
    parent: u64,
    /// The last used first: CTRL-R redoes it.
    children: Vec<u64>,
    /// The inverse edits (in recorded order) while the change is in the
    /// text; the edits that make it again while it is undone.
    edits: Vec<Edit>,
    before: Pos,
    /// Seconds since the Unix epoch.
    time: u64,
    /// The write (counted from 1) that saved this state; 0: none.
    save_nr: u64,
}

/// How `:earlier` and `:later` count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Changes,
    Seconds,
    Writes,
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn default_levels() -> i64 {
    1000
}

fn default_clock() -> fn() -> u64 {
    system_now
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Undo {
    nodes: BTreeMap<u64, Node>,
    /// The node whose state the text is in.
    cur: u64,
    seq_last: u64,
    /// For `:earlier {N}s`.
    time_cur: u64,
    save_nr_last: u64,
    save_nr_cur: u64,
    /// The text state node 0 stands for changed this many times (history
    /// dropped by 'undolevels'), so `state_id` stays unique.
    root_gen: u64,
    #[serde(skip)]
    open: Option<Group>,
    #[serde(skip)]
    depth: usize,
    /// 'undolevels': at most this many changes plus one are kept; below 0
    /// none.
    #[serde(skip, default = "default_levels")]
    pub levels: i64,
    /// `:undojoin`: the next change joins the last one.
    #[serde(skip)]
    join: bool,
    #[serde(skip, default = "default_clock")]
    clock: fn() -> u64,
}

impl Default for Undo {
    fn default() -> Undo {
        let mut nodes = BTreeMap::new();
        nodes.insert(0, Node::default());
        Undo {
            nodes,
            cur: 0,
            seq_last: 0,
            time_cur: 0,
            save_nr_last: 0,
            save_nr_cur: 0,
            root_gen: 0,
            open: None,
            depth: 0,
            levels: default_levels(),
            join: false,
            clock: system_now,
        }
    }
}

const MAGIC: &str = "fred-undo 1";

impl Undo {
    fn node(&mut self, n: u64) -> &mut Node {
        self.nodes.get_mut(&n).expect("undo node")
    }

    /// Start a group. Nested calls join the outer group.
    pub fn begin(&mut self, cursor: Pos) {
        if self.depth == 0 {
            let joinable = self.join
                && self.cur != 0
                && self.levels >= 0
                && self.nodes[&self.cur].children.is_empty();
            self.open = Some(Group {
                edits: vec![],
                before: cursor,
                join: joinable.then_some(self.cur),
            });
        }
        self.depth += 1;
    }

    /// Record the inverse of an edit applied with the cursor at `cursor`:
    /// the first edit of a group sets where undo puts the cursor (Vim's
    /// `uh_cursor`, taken when the first change is saved).
    pub fn record_at(&mut self, inverse: Edit, cursor: Pos) {
        if let Some(g) = &mut self.open
            && g.edits.is_empty()
            && g.join.is_none()
        {
            g.before = cursor;
        }
        self.record(inverse);
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

    /// End a group. Undo and redo both put the cursor where the change
    /// started (as Vim), so the cursor at the end is not kept.
    pub fn end(&mut self, _cursor: Pos) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth > 0 {
            return;
        }
        if let Some(g) = self.open.take()
            && !g.edits.is_empty()
        {
            self.commit(g);
        }
    }

    fn commit(&mut self, g: Group) {
        if let Some(j) = g.join {
            let n = self.node(j);
            n.edits.extend(g.edits);
            return;
        }
        self.trim();
        if self.levels < 0 {
            // No undo: the text before this change is gone.
            self.root_gen += 1;
            return;
        }
        self.seq_last += 1;
        let seq = self.seq_last;
        let time = (self.clock)();
        self.time_cur = time + 1;
        self.nodes.insert(
            seq,
            Node {
                parent: self.cur,
                children: vec![],
                edits: g.edits,
                before: g.before,
                time,
                save_nr: 0,
            },
        );
        let cur = self.cur;
        self.node(cur).children.insert(0, seq);
        self.cur = seq;
    }

    /// Drop the oldest changes until a new one fits in 'undolevels', as
    /// `u_savecommon()`.
    fn trim(&mut self) {
        while (self.nodes.len() - 1) as i64 > self.levels {
            let root = &self.nodes[&0].children;
            let Some(&first) = root.first() else { break };
            if self.cur == 0 {
                // Everything was undone: the whole tree goes.
                self.nodes.retain(|&k, _| k == 0);
                self.node(0).children.clear();
            } else if root.len() == 1 {
                // The oldest change can no longer be undone.
                let n = self.nodes.remove(&first).expect("undo node");
                for &c in &n.children {
                    self.node(c).parent = 0;
                }
                self.node(0).children = n.children;
                self.root_gen += 1;
                if self.cur == first {
                    self.cur = 0;
                }
            } else {
                // The least recently used branch.
                let last = *root.last().expect("a branch");
                self.remove_subtree(last);
            }
        }
    }

    fn remove_subtree(&mut self, top: u64) {
        let parent = self.nodes[&top].parent;
        self.node(parent).children.retain(|&c| c != top);
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.remove(&n) {
                stack.extend(node.children);
            }
        }
    }

    /// Edits recorded so far in the open group.
    pub fn open_len(&self) -> usize {
        self.open.as_ref().map_or(0, |g| g.edits.len())
    }

    /// Undo the open group's edits after the first `mark`, forgetting them.
    pub fn rollback_to(&mut self, buf: &mut Buffer, mark: usize) {
        if let Some(g) = &mut self.open {
            for e in g.edits.drain(mark.min(g.edits.len())..).rev() {
                buf.apply(e);
            }
        }
    }

    pub fn in_group(&self) -> bool {
        self.depth > 0
    }

    /// Apply a node's edits newest-first and keep their inverses.
    fn flip(&mut self, buf: &mut Buffer, n: u64) {
        let edits = std::mem::take(&mut self.node(n).edits);
        self.node(n).edits = edits.iter().rev().map(|e| buf.apply(e.clone())).collect();
        // The last used child comes first.
        let parent = self.nodes[&n].parent;
        let kids = &mut self.node(parent).children;
        kids.retain(|&c| c != n);
        kids.insert(0, n);
        let node = &self.nodes[&n];
        self.time_cur = node.time;
        let save = node.save_nr;
        if save != 0 {
            self.save_nr_cur = if self.cur == n { save - 1 } else { save };
        }
    }

    /// Undo the current change.
    fn up(&mut self, buf: &mut Buffer) -> Pos {
        let n = self.cur;
        self.flip(buf, n);
        self.cur = self.nodes[&n].parent;
        self.nodes[&n].before
    }

    /// Redo child `n` of the current state.
    fn down(&mut self, buf: &mut Buffer, n: u64) -> Pos {
        self.flip(buf, n);
        self.cur = n;
        self.nodes[&n].before
    }

    /// Undo the last change; returns the cursor to restore.
    pub fn undo(&mut self, buf: &mut Buffer) -> Option<Pos> {
        // 'undolevels' 0: after an undo, `u` undoes itself.
        let undone = self.nodes[&self.cur].children.first().copied();
        if let Some(n) = undone.filter(|_| self.levels == 0) {
            return Some(self.down(buf, n));
        }
        (self.cur != 0).then(|| self.up(buf))
    }

    pub fn redo(&mut self, buf: &mut Buffer) -> Option<Pos> {
        if self.levels <= 0 {
            return None;
        }
        let n = *self.nodes[&self.cur].children.first()?;
        Some(self.down(buf, n))
    }

    /// Node `n` and its ancestors, `n` first.
    fn path(&self, mut n: u64) -> Vec<u64> {
        let mut p = vec![n];
        while n != 0 {
            n = self.nodes[&n].parent;
            p.push(n);
        }
        p
    }

    /// Undo and redo until the text is in node `target`'s state.
    fn goto(&mut self, buf: &mut Buffer, target: u64) -> Option<Pos> {
        let path = self.path(target);
        let mut pos = None;
        while !path.contains(&self.cur) {
            pos = Some(self.up(buf));
        }
        let at = path.iter().position(|&n| n == self.cur).unwrap_or(0);
        for &n in path[..at].iter().rev() {
            pos = Some(self.down(buf, n));
        }
        pos
    }

    /// `:undo N`: the state after change `n` (0: before the first).
    pub fn undo_to(&mut self, buf: &mut Buffer, n: u64) -> Result<Option<Pos>, String> {
        if !self.nodes.contains_key(&n) {
            return Err(format!("E830: Undo number {n} not found"));
        }
        Ok(self.goto(buf, n))
    }

    /// `g-`, `g+`, `:earlier` and `:later` (`step` < 0 goes back), as
    /// `undo_time()`.
    pub fn time_travel(
        &mut self,
        buf: &mut Buffer,
        step: i64,
        unit: Unit,
    ) -> Result<Option<Pos>, String> {
        let back = step < 0;
        let mut unit = unit;
        let mut target = match unit {
            Unit::Changes => self.cur as i64 + step,
            Unit::Seconds => self.time_cur as i64 + step,
            Unit::Writes if back => {
                // Changes since the last write count as one write.
                let unsaved = self.cur == 0 || self.nodes[&self.cur].save_nr == 0;
                let t = self.save_nr_cur as i64 + step + i64::from(unsaved);
                if t <= 0 {
                    unit = Unit::Changes;
                }
                t
            }
            Unit::Writes => {
                let t = self.save_nr_cur as i64 + step;
                if t > self.save_nr_last as i64 {
                    unit = Unit::Changes;
                    self.seq_last as i64 + 1
                } else {
                    t
                }
            }
        };
        if back {
            target = target.max(0);
        } else {
            let limit = match unit {
                Unit::Changes => self.seq_last as i64 + 1,
                Unit::Seconds => (self.clock)() as i64 + 1,
                Unit::Writes => self.save_nr_last as i64 + 1,
            };
            target = target.min(limit);
        }
        if target == 0 && unit != Unit::Seconds {
            return Ok(self.goto(buf, 0));
        }
        let value = |n: &Node, seq: u64| match unit {
            Unit::Changes => seq as i64,
            Unit::Seconds => n.time as i64,
            Unit::Writes => n.save_nr as i64,
        };
        let real = |n: &Node| unit != Unit::Writes || n.save_nr != 0;
        if unit != Unit::Seconds
            && let Some((&seq, _)) = self
                .nodes
                .iter()
                .find(|&(&s, n)| s != 0 && real(n) && value(n, s) == target)
        {
            return Ok(self.goto(buf, seq));
        }
        // Else the nearest change in the right direction.
        let best = self
            .nodes
            .iter()
            .filter(|&(&s, n)| s != 0 && real(n) && if back { s <= self.cur } else { s > self.cur })
            .min_by_key(|&(&s, n)| {
                let tie = if back { s as i64 } else { -(s as i64) };
                ((value(n, s) - target).abs(), tie)
            })
            .map(|(&s, n)| (s, n.parent));
        match best {
            None if back => Err("Already at oldest change".into()),
            None => Err("Already at newest change".into()),
            // Going back stops above the change found.
            Some((_, parent)) if back => Ok(self.goto(buf, parent)),
            Some((seq, _)) => Ok(self.goto(buf, seq)),
        }
    }

    /// `:undo!`: undo `count` changes and forget them.
    pub fn undo_forget(&mut self, buf: &mut Buffer, count: usize) -> Option<Pos> {
        let mut pos = None;
        let mut last = None;
        for _ in 0..count {
            if self.cur == 0 {
                break;
            }
            last = Some(self.cur);
            pos = Some(self.up(buf));
        }
        let last = last?;
        self.remove_subtree(last);
        if self.seq_last == last {
            self.seq_last -= 1;
        }
        pos
    }

    /// `:undo! N`: undo back to change `n` on this branch and forget the
    /// changes undone.
    pub fn undo_forget_to(&mut self, buf: &mut Buffer, n: u64) -> Result<Option<Pos>, String> {
        let path = self.path(self.cur);
        match path.iter().position(|&p| p == n) {
            Some(steps) if n < self.cur => Ok(self.undo_forget(buf, steps)),
            _ => Err("E5767: Cannot use :undo! to redo or move to a different undo branch".into()),
        }
    }

    /// `:undojoin`: the next change joins the last one.
    pub fn join_next(&mut self) -> Result<(), String> {
        if !self.nodes[&self.cur].children.is_empty() {
            return Err("E790: undojoin is not allowed after undo".into());
        }
        self.join = true;
        Ok(())
    }

    /// A typed key ends `:undojoin` (Vim syncs undo when it waits for keys).
    pub fn sync(&mut self) {
        self.join = false;
    }

    /// The text was written: mark the state for `:earlier {N}f`.
    pub fn written(&mut self) {
        self.save_nr_last += 1;
        self.save_nr_cur = self.save_nr_last;
        let (cur, nr) = (self.cur, self.save_nr_last);
        if cur != 0 {
            self.node(cur).save_nr = nr;
        }
    }

    /// Identifies the current position in history; equal ids mean equal text.
    pub fn state_id(&self) -> u64 {
        match (self.cur, self.root_gen) {
            (0, 0) => 0,
            (0, g) => u64::MAX - g,
            (n, _) => n,
        }
    }

    /// `:undolist`: the leaves, as Vim lists them.
    pub fn list(&self) -> Vec<String> {
        let now = (self.clock)();
        let mut rows = vec![];
        for (&seq, n) in &self.nodes {
            if seq == 0 || !n.children.is_empty() {
                continue;
            }
            let changes = self.path(seq).len() - 1;
            let mut row = format!("{seq:6} {changes:7}  {}", fmt_time(now, n.time));
            if n.save_nr > 0 {
                while row.len() < 33 {
                    row.push(' ');
                }
                row += &format!("  {:3}", n.save_nr);
            }
            rows.push(row);
        }
        rows
    }

    /// 'undoreload': `old` (the text before a reload) as one change to
    /// undo, the reloaded text `new_len` chars long.
    pub fn record_reload(&mut self, old: String, new_len: usize, cursor: Pos) {
        self.begin(cursor);
        self.record(Edit {
            start: 0,
            end: new_len,
            text: old,
        });
        self.end(cursor);
    }

    /// The undo file's contents for `text`.
    pub fn to_file(&self, text: &str) -> String {
        serde_json::to_string(&(MAGIC, hash(text), self)).unwrap_or_default()
    }

    /// An undo file read back, if it was written for `text`.
    pub fn from_file(data: &str, text: &str) -> Result<Undo, String> {
        let (magic, h, mut u): (String, u64, Undo) =
            serde_json::from_str(data).map_err(|_| "not an undo file".to_string())?;
        if magic != MAGIC {
            return Err("not an undo file".into());
        }
        if h != hash(text) {
            return Err("File contents changed, cannot use undo info".into());
        }
        u.node(0); // node 0 must be there
        Ok(u)
    }
}

/// 'undofile': the tree kept for `file` in 'undodir' (`dirs`), if it was
/// written for `text`.
pub fn read_for(dirs: &str, file: &Path, text: &str) -> Option<Undo> {
    let name = file_name(dirs, file, true)?;
    Undo::from_file(&std::fs::read_to_string(name).ok()?, text).ok()
}

impl Undo {
    /// 'undofile': keep the tree for `file`, whose text is now `text`.
    pub fn write_for(&self, dirs: &str, file: &Path, text: &str) -> Result<(), String> {
        let name = file_name(dirs, file, false)
            .ok_or("Cannot write undo file in any directory in 'undodir'")?;
        std::fs::write(&name, self.to_file(text)).map_err(|_| {
            format!(
                "E828: Cannot open undo file for writing: {}",
                name.display()
            )
        })
    }
}

/// An undo file starts this way.
pub fn is_undo_file(data: &str) -> bool {
    data.starts_with(&format!("[\"{MAGIC}\""))
}

/// FNV-1a, stable across builds (Vim uses SHA-256 of the text).
fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// How long ago `t` was, as `undo_fmt_time()`.
fn fmt_time(now: u64, t: u64) -> String {
    let ago = now.saturating_sub(t);
    if ago >= 100 {
        let f = if ago < 12 * 60 * 60 {
            "%H:%M:%S"
        } else {
            "%Y/%m/%d %H:%M:%S"
        };
        crate::magit::margin::strftime(f, t as i64)
    } else if ago == 1 {
        "1 second ago".into()
    } else {
        format!("{ago} seconds ago")
    }
}

/// Fred's 'undodir' default: `$XDG_STATE_HOME/fred/undo`, beside the swap
/// files.
pub fn default_dir() -> PathBuf {
    crate::swap::swap_dir().with_file_name("undo")
}

/// The undo file for `file` in 'undodir' (`dirs`, comma-separated; `.` is
/// the file's own directory), as `u_get_undo_file_name()`: when reading
/// the first that exists, when writing the first directory that exists
/// (the last one is created).
pub fn file_name(dirs: &str, file: &Path, reading: bool) -> Option<PathBuf> {
    let file = crate::swap::canonical(file);
    let default = default_dir();
    let dirs: Vec<PathBuf> = if dirs.is_empty() {
        vec![default]
    } else {
        dirs.split(',').map(PathBuf::from).collect()
    };
    for (i, d) in dirs.iter().enumerate() {
        let name = if d.as_os_str() == "." {
            let tail = file.file_name()?.to_string_lossy();
            file.with_file_name(format!(".{tail}.un~"))
        } else {
            if !d.is_dir() && i + 1 == dirs.len() && !reading {
                std::fs::create_dir_all(d).ok()?;
            }
            if !d.is_dir() {
                continue;
            }
            d.join(file.to_string_lossy().replace('/', "%"))
        };
        if !reading || name.exists() {
            return Some(name);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Edit};

    fn ed(start: usize, end: usize, t: &str) -> Edit {
        Edit {
            start,
            end,
            text: t.into(),
        }
    }

    /// Replace the text's first char with `c`, as one change.
    fn change(u: &mut Undo, b: &mut Buffer, c: &str) {
        u.begin((0, 0));
        u.record(b.apply(ed(0, 1, c)));
        u.end((0, 0));
    }

    #[test]
    fn undo_redo_group() {
        let mut b = Buffer::from_text("abc");
        let mut u = Undo::default();
        u.begin((0, 0));
        u.record(b.apply(ed(0, 1, "X")));
        u.record(b.apply(ed(1, 2, "Y")));
        u.end((0, 1));
        assert_eq!(b.text(), "XYc");
        assert_eq!(u.undo(&mut b), Some((0, 0)));
        assert_eq!(b.text(), "abc");
        // Redo too puts the cursor where the change started, as Vim.
        assert_eq!(u.redo(&mut b), Some((0, 0)));
        assert_eq!(b.text(), "XYc");
        assert_eq!(u.redo(&mut b), None);
    }

    #[test]
    fn empty_group_dropped() {
        let mut b = Buffer::from_text("a");
        let mut u = Undo::default();
        u.begin((0, 0));
        u.end((0, 0));
        assert_eq!(u.undo(&mut b), None);
    }

    #[test]
    fn nested_begin_is_one_group() {
        let mut b = Buffer::from_text("abc");
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
    fn new_edit_branches() {
        let mut b = Buffer::from_text("abc");
        let mut u = Undo::default();
        change(&mut u, &mut b, "X");
        u.undo(&mut b);
        change(&mut u, &mut b, "Z");
        assert_eq!(u.redo(&mut b), None);
        assert_eq!(b.text(), "Zbc");
        // g- walks back through the abandoned branch.
        u.time_travel(&mut b, -1, Unit::Changes).unwrap();
        assert_eq!(b.text(), "Xbc");
        u.time_travel(&mut b, -1, Unit::Changes).unwrap();
        assert_eq!(b.text(), "abc");
        // CTRL-R takes the branch last used.
        u.redo(&mut b);
        assert_eq!(b.text(), "Xbc");
        u.undo_to(&mut b, 2).unwrap();
        assert_eq!(b.text(), "Zbc");
        assert!(u.undo_to(&mut b, 5).is_err());
        assert_eq!(u.list().len(), 2);
    }

    #[test]
    fn state_id_returns_after_undo_redo() {
        let mut b = Buffer::from_text("abc");
        let mut u = Undo::default();
        let saved = u.state_id();
        change(&mut u, &mut b, "X");
        let changed = u.state_id();
        assert_ne!(saved, changed);
        u.undo(&mut b);
        assert_eq!(u.state_id(), saved);
        u.redo(&mut b);
        assert_eq!(u.state_id(), changed);
    }

    #[test]
    fn earlier_by_time_and_writes() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NOW: AtomicU64 = AtomicU64::new(0);
        let mut b = Buffer::from_text("abc");
        let mut u = Undo {
            clock: || NOW.load(Ordering::Relaxed),
            ..Undo::default()
        };
        for (t, c) in [(1000, "1"), (1100, "2"), (1200, "3")] {
            NOW.store(t, Ordering::Relaxed);
            change(&mut u, &mut b, c);
        }
        NOW.store(1300, Ordering::Relaxed);
        // 50 seconds back from the last change: before it.
        u.time_travel(&mut b, -50, Unit::Seconds).unwrap();
        assert_eq!(b.text(), "2bc");
        u.time_travel(&mut b, 1000, Unit::Seconds).unwrap();
        assert_eq!(b.text(), "3bc");
        // Equally near 1000 and 1100: the older change, as Vim.
        u.time_travel(&mut b, -150, Unit::Seconds).unwrap();
        assert_eq!(b.text(), "abc");
        assert!(u.time_travel(&mut b, -1, Unit::Seconds).is_err());

        // Writes: after change 1 and change 2; then change 3 unsaved.
        let mut b = Buffer::from_text("abc");
        let mut u = Undo::default();
        change(&mut u, &mut b, "1");
        u.written();
        change(&mut u, &mut b, "2");
        u.written();
        change(&mut u, &mut b, "3");
        u.time_travel(&mut b, -1, Unit::Writes).unwrap();
        assert_eq!(b.text(), "2bc");
        u.time_travel(&mut b, -1, Unit::Writes).unwrap();
        assert_eq!(b.text(), "1bc");
        u.time_travel(&mut b, -1, Unit::Writes).unwrap();
        assert_eq!(b.text(), "abc");
        u.time_travel(&mut b, 1, Unit::Writes).unwrap();
        assert_eq!(b.text(), "1bc");
        u.time_travel(&mut b, 5, Unit::Writes).unwrap();
        assert_eq!(b.text(), "3bc");
    }

    #[test]
    fn undolevels_trims_and_join_joins() {
        let mut b = Buffer::from_text("abc");
        let mut u = Undo {
            levels: 1,
            ..Undo::default()
        };
        for c in ["1", "2", "3"] {
            change(&mut u, &mut b, c);
        }
        assert_eq!(u.nodes.len() - 1, 2);
        u.undo(&mut b);
        u.undo(&mut b);
        assert_eq!(b.text(), "1bc");
        assert_eq!(u.undo(&mut b), None);
        // The oldest state is not the text before the first change.
        assert_ne!(u.state_id(), 0);

        let mut b = Buffer::from_text("abc");
        let mut u = Undo::default();
        change(&mut u, &mut b, "1");
        u.join_next().unwrap();
        u.begin((0, 0));
        u.record(b.apply(ed(1, 2, "2")));
        u.end((0, 0));
        u.sync();
        u.undo(&mut b);
        assert_eq!(b.text(), "abc");
        assert!(u.join_next().is_err());

        // -1: no undo at all.
        u.levels = -1;
        change(&mut u, &mut b, "Q");
        assert_eq!(u.undo(&mut b), None);
        assert_eq!(b.text(), "Qbc");
    }

    #[test]
    fn forget_and_file_round_trip() {
        let mut b = Buffer::from_text("abc");
        let mut u = Undo::default();
        change(&mut u, &mut b, "1");
        change(&mut u, &mut b, "2");
        assert!(u.undo_forget_to(&mut b, 2).is_err());
        u.undo_forget_to(&mut b, 1).unwrap();
        assert_eq!(b.text(), "1bc");
        assert_eq!(u.redo(&mut b), None);
        let data = u.to_file(&b.text());
        assert!(is_undo_file(&data));
        assert!(Undo::from_file(&data, "other").is_err());
        let mut v = Undo::from_file(&data, &b.text()).unwrap();
        v.undo(&mut b);
        assert_eq!(b.text(), "abc");
    }
}
