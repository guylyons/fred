//! Marks with columns, the previous context mark (`''`), the jumplist
//! (CTRL-O, CTRL-I) and the changelist (`g;`, `g,`), after Neovim's
//! `mark.c`. Line marks live in `Editor::marks` (the ex ranges read them);
//! their columns in `Editor::mark_cols`.

use crate::editor::Editor;

pub type Pos = (usize, usize);

/// Jumps and changes kept, as Neovim's JUMPLISTSIZE and JUMPLISTSIZE.
const LIST_SIZE: usize = 100;

#[derive(Debug, Default)]
pub struct Lists {
    /// The previous context mark (`''`) and the one before it.
    pcmark: Option<Pos>,
    prev_pcmark: Option<Pos>,
    jumps: Vec<Pos>,
    jump_idx: usize,
    changes: Vec<Pos>,
    change_idx: usize,
    /// Where the last change was (`'.`).
    last_change: Option<Pos>,
}

pub(crate) fn set(ed: &mut Editor, c: char, p: Pos) {
    match c {
        '\'' | '`' => setpcmark_at(ed, p),
        c if c.is_ascii_alphabetic() || "[]<>".contains(c) => {
            ed.marks.insert(c, p.0);
            ed.mark_cols.insert(c, p.1);
        }
        _ => {}
    }
}

/// Where mark `c` is.
pub(crate) fn get(ed: &Editor, c: char) -> Option<Pos> {
    let p = match c {
        '\'' | '`' => ed.vim.lists.pcmark.or(Some((0, 0)))?,
        '.' => ed.vim.lists.last_change?,
        '^' => ed.vim.insert_stop?,
        // Where `(`, `)`, `{` and `}` would go.
        '(' | ')' => super::textobj::findsent(&ed.buf, ed.cur.pos(), c == ')', 1)?,
        '{' | '}' => paragraph(ed, c == '}'),
        c => (
            *ed.marks.get(&c)?,
            ed.mark_cols.get(&c).copied().unwrap_or(0),
        ),
    };
    let l = p.0.min(ed.line_count() - 1);
    Some((l, p.1.min(ed.buf.line_len(l))))
}

/// Remember the cursor as the previous context and add it to the jumplist.
pub(crate) fn setpcmark(ed: &mut Editor) {
    let p = ed.cur.pos();
    setpcmark_at(ed, p);
}

fn setpcmark_at(ed: &mut Editor, p: Pos) {
    let m = &mut ed.vim.lists;
    m.prev_pcmark = m.pcmark;
    m.pcmark = Some(p);
    if m.jumps.len() >= LIST_SIZE {
        m.jumps.remove(0);
    }
    m.jumps.push(p);
    m.jump_idx = m.jumps.len();
}

/// After a jump: if the cursor didn't move, the previous context stays.
pub(crate) fn checkpcmark(ed: &mut Editor) {
    let cur = ed.cur.pos();
    let m = &mut ed.vim.lists;
    if m.prev_pcmark.is_some() && m.pcmark == Some(cur) {
        m.pcmark = m.prev_pcmark;
    }
    m.prev_pcmark = None;
}

/// Neovim's `cleanup_jumplist()`: drop entries on a line a later entry is
/// on, and a last one on the cursor's line.
fn cleanup(ed: &mut Editor) {
    let cur_line = ed.cur.line;
    let m = &mut ed.vim.lists;
    let mut to = 0;
    let n = m.jumps.len();
    for from in 0..n {
        if m.jump_idx == from {
            m.jump_idx = to;
        }
        let dup = m.jumps[from + 1..].iter().any(|j| j.0 == m.jumps[from].0);
        if !dup {
            m.jumps[to] = m.jumps[from];
            to += 1;
        }
    }
    if m.jump_idx == n {
        m.jump_idx = to;
    }
    m.jumps.truncate(to);
    if !m.jumps.is_empty()
        && m.jump_idx == m.jumps.len()
        && m.jumps[m.jumps.len() - 1].0 == cur_line
    {
        m.jumps.pop();
        m.jump_idx -= 1;
    }
}

/// CTRL-O (`count` < 0) and CTRL-I: the position `count` jumps away.
pub(crate) fn jump(ed: &mut Editor, count: isize) -> Option<Pos> {
    cleanup(ed);
    let len = ed.vim.lists.jumps.len() as isize;
    let idx = ed.vim.lists.jump_idx as isize;
    if len == 0 || idx + count < 0 || idx + count >= len {
        return None;
    }
    let mut idx = idx;
    if idx == len {
        // The first CTRL-O after a jump comes back here with CTRL-I.
        setpcmark(ed);
        idx = ed.vim.lists.jump_idx as isize - 1;
        if idx + count < 0 {
            ed.vim.lists.jump_idx = idx as usize;
            return None;
        }
    }
    let idx = (idx + count) as usize;
    ed.vim.lists.jump_idx = idx;
    ed.vim.lists.jumps.get(idx).copied()
}

/// `g;` (`count` < 0) and `g,`: the position in the changelist.
pub(crate) fn change(ed: &mut Editor, count: isize) -> Result<Pos, &'static str> {
    let m = &mut ed.vim.lists;
    let len = m.changes.len() as isize;
    if len == 0 {
        return Err("E664: Changelist is empty");
    }
    let n = m.change_idx as isize;
    let n = if n + count < 0 {
        if n == 0 {
            return Err("E662: At start of changelist");
        }
        0
    } else if n + count >= len {
        if n == len - 1 {
            return Err("E663: At end of changelist");
        }
        len - 1
    } else {
        n + count
    };
    m.change_idx = n as usize;
    Ok(m.changes[n as usize])
}

/// A change at `p`: the `'.` mark and the changelist, where typing along a
/// line keeps one entry (as Neovim, within 79 columns).
pub(crate) fn changed(ed: &mut Editor, p: Pos) {
    let m = &mut ed.vim.lists;
    m.last_change = Some(p);
    let add = match m.changes.last() {
        None => true,
        Some(&(l, c)) => l != p.0 || c + 79 < p.1 || p.1 + 79 < c,
    };
    if add {
        if m.changes.len() >= LIST_SIZE {
            m.changes.remove(0);
        }
        m.changes.push(p);
    } else if let Some(last) = m.changes.last_mut() {
        *last = p;
    }
    m.change_idx = m.changes.len();
}

/// Lines `at..at + removed` became `inserted` lines: move the positions.
pub(crate) fn line_change(lists: &mut Lists, at: usize, removed: usize, inserted: usize) {
    let fix = |p: &mut Pos| {
        if p.0 >= at + removed {
            p.0 = p.0 + inserted - removed;
        } else if p.0 >= at + inserted {
            p.0 = at;
        }
    };
    for p in lists.jumps.iter_mut().chain(lists.changes.iter_mut()) {
        fix(p);
    }
    for p in [&mut lists.pcmark, &mut lists.last_change]
        .into_iter()
        .flatten()
    {
        fix(p);
    }
}

/// The blank line before (or after) the cursor's paragraph, as `{` and `}`
/// go; the buffer's start or end if there is none.
fn paragraph(ed: &Editor, fwd: bool) -> Pos {
    let n = ed.line_count();
    let blank = |l: usize| ed.buf.line_len(l) == 0;
    let mut p = ed.cur.line;
    if fwd {
        while p < n && blank(p) {
            p += 1;
        }
        while p < n && !blank(p) {
            p += 1;
        }
        if p >= n {
            return (n - 1, ed.last_grapheme(n - 1));
        }
    } else {
        while p > 0 && blank(p) {
            p -= 1;
        }
        while p > 0 && !blank(p) {
            p -= 1;
        }
    }
    (p, 0)
}
