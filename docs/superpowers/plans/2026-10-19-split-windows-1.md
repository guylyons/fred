# Split windows, part 1 (stages 1–2): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Neovim split windows in fred: `:split`, `:vsplit`, `:new`,
`:vnew`, `:sbuffer`, `CTRL-W` moves, `:close`, `:only`, `:hide`, `:q`
closing windows, `:qall`, each window with its own cursor and scroll.

**Architecture:** A pure layout tree in `src/window.rs`. `Session` owns the
windows; the active window's buffer stays the live `Session::ed` (the
existing live/parked swap), other windows draw from parked editors.
Rendering splits into one window's text and status line, plus the shared
command row and full-screen overlays.

**Tech Stack:** Rust, ratatui, existing modules. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-19-split-windows-design.md`
(stages 1 and 2; stages 3–7 get their own plans).

## Global Constraints

- One window looks exactly like today: every existing test passes
  unchanged, including `src/ui/tests.rs` drawing through `ui::draw`.
- Closing a window never discards a buffer; a modified buffer stays as a
  hidden buffer. `:q` on the last window keeps today's checks.
- Mode, registers, command line, message and pickers stay global.
- Neovim error texts: `E36: Not enough room`, `E444: Cannot close last
  window`.
- Never `git add -A`; never stage `notes.md` / `notes`.
- Each commit updates `docs/nvim-core-roadmap.md` (execution queue and
  log; points stay 0 for tracks 2 and 17) and the touched rows of
  `docs/nvim-parity.csv` (`partial`, fred_behavior `present; not yet
  verified against nvim`).
- Cargo is not on PATH: prefix commands with
  `PATH="$(ls -d ~/.rustup/toolchains/*/bin | head -1):$PATH"`.

## Review Focus

1. `:bd` of a buffer shown in other windows: as Neovim, those windows
   close; if it was in the last window, that window shows another buffer.
   (Task 2 test `bdelete_closes_its_windows`.)
2. Editing in one window while another shows the same buffer further
   down: the other window's cursor is clamped to the text, never panics.
   (Task 3 test `other_window_cursor_clamped_after_delete`.)
3. Terminal shrunk below the layout's size: windows shrink to at least
   1 text row or are skipped; drawing never panics. (Task 1 test
   `layout_smaller_than_sizes`.)
4. Insert or Visual mode in the active window while an inactive window
   shows the same buffer: the inactive one draws with no selection and no
   popup. (Task 3 test `inactive_same_buffer_no_visual`.)
5. Splitting a window too short to hold two: `E36: Not enough room`,
   nothing changes. (Task 4 test `split_too_small`.)

---

### Task 1: The layout tree (`src/window.rs`)

**Files:**
- Create: `src/window.rs` (declare `pub mod window;` in `src/lib.rs`)
- Test: `#[cfg(test)] mod tests` at the bottom of `src/window.rs`

**Interfaces:**
- Produces:
  - `pub enum Dir { Row, Col }`: `Row` = side by side, `Col` = stacked.
  - `pub enum Node { Leaf(usize), Split { dir: Dir, kids: Vec<(Node, u16)> } }`
    (leaf = window id; the `u16` is the child's size along `dir`, in
    cells, including its status line or divider).
  - `pub struct Placed { pub id: usize, pub rect: Rect, pub divider: bool }`:
    `rect` includes the window's status row (its last row); `divider`
    means the rect's last column is the `│` to a right-hand neighbour
    (Vim gives the separator to the left window).
  - `impl Node`:
    - `pub fn layout(&self, area: Rect) -> Vec<Placed>`: leaves in window
      order (depth-first, top-left first). If the stored sizes do not add
      up to `area`, the last child absorbs the difference; a child that
      ends up below 2 rows (Col) or 1 column (Row) gets 0 and is omitted.
    - `pub fn order(&self) -> Vec<usize>`: window ids in window order.
    - `pub fn split(&mut self, area: Rect, target: usize, new: usize, dir: Dir, after: bool, size: Option<u16>) -> Result<(), &'static str>`:
      the target's space divides between target and `new`; `new` gets
      `size` or half (rounded down for `new`); `after` puts `new` below /
      right. If `target`'s parent already splits in `dir`, `new` joins it
      as a sibling; else the leaf becomes a `Split`. Errors with
      `"E36: Not enough room"` when either part would have under 2 rows
      (Col) or under 2 columns incl. divider (Row).
    - `pub fn close(&mut self, area: Rect, id: usize) -> Result<(), &'static str>`:
      removes the leaf, giving its size to the previous sibling (the next
      one if it was first); a `Split` left with one child is replaced by
      it. `"E444: Cannot close last window"` when `id` is the only leaf.
    - `pub fn only(&mut self, id: usize)`: the tree becomes `Leaf(id)`.
    - `pub fn neighbor(&self, area: Rect, id: usize, dir: char, count: usize, at: (u16, u16)) -> Option<usize>`:
      `dir` in `h j k l`; steps `count` windows that way (stopping at the
      edge), choosing among several adjacent windows the one whose span
      contains `at` (the cursor's screen cell), as Neovim does.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn split_halves_and_orders() {
    let a = Rect::new(0, 0, 80, 24);
    let mut t = Node::Leaf(1);
    t.split(a, 1, 2, Dir::Col, false, None).unwrap();   // :split: new above
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
fn split_too_small() {
    let a = Rect::new(0, 0, 80, 3);
    let mut t = Node::Leaf(1);
    assert_eq!(t.split(a, 1, 2, Dir::Col, false, None), Err("E36: Not enough room"));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --quiet window::tests`
Expected: compile errors (`Node` not defined).

- [ ] **Step 3: Implement `src/window.rs` with the interfaces above.**

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test --quiet window::tests`
Expected: `6 passed`.

- [ ] **Step 5: Commit** (`git add src/window.rs src/lib.rs`,
  `feat: window layout tree for splits`).

### Task 2: Session owns windows; per-buffer scroll and highlighting

**Files:**
- Modify: `src/session.rs` (struct `Session`, `Parked`, `show`,
  `switch_to`, `replace`, `buffer` delete fix-ups near line 1020)
- Modify: `src/highlight.rs` (add `blank`)
- Modify: `src/app.rs` (`Ui::view` goes; `Ui::draw` and the `reloaded`
  block near line 883 use the session's)
- Test: `src/session.rs` `mod tests` (the `T` helper)

**Interfaces:**
- Consumes: `window::{Node, Dir, Placed}` (Task 1).
- Produces:
  - `pub struct Window { pub id: usize, pub buf: usize, pub cur: Cursor, pub view: View, pub prev: Option<usize> }`
    in `src/window.rs`. `buf` indexes `Session::bufs`; `cur` is
    meaningful only while the window is inactive (the active window's
    cursor is `Session::ed.cur`); `prev` is the window's alternate buffer.
  - `Session` fields: `pub windows: Vec<Window>`, `pub tree: Node`,
    `pub active: usize` (window id), `prev_win: Option<usize>`
    (for `CTRL-W p`), `next_win_id: usize`, `pub area: Rect` (the windows'
    area, set by the app every draw; default `Rect::new(0, 0, 80, 23)`),
    `pub hl: Highlighter` (the live buffer's).
  - `Parked` gains `hl: Highlighter` and `view: View` (the buffer's last
    scroll), swapped by `show` like `stamp`.
  - `Session::win(&self) -> &Window`, `win_mut(&mut self) -> &mut Window`:
    the active window.
  - `Highlighter::blank(&self) -> Highlighter`: same theme (shared `Arc`)
    and truecolor, no file, empty cache. `Session::open` takes the
    app's `Highlighter` (new parameter `hl: Highlighter`) so tests pass
    `Highlighter::new("ansi", false).unwrap()`.
  - `show(i)` additionally: saves the active window's `view` into the
    outgoing buffer, restores the incoming buffer's saved `view` into
    the window, and sets `win.prev`/`win.buf`. `set_file` on the
    highlighter only happens when a buffer is created or replaced, not
    on `show` (its cache survives switching).
  - Deleting buffer `i` (`:bd`): windows showing `i` close (tree `close`)
    unless it is the last window, which shows the `#` buffer as today;
    every `Window::buf`/`prev` above `i` shifts down by one, as
    `return_to` already does.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn scroll_survives_buffer_switch() {
    let mut t = T::open(Some("a"), Some(&"x\n".repeat(200)));
    t.keys("150G");
    t.s.win_mut().view.top = 140;
    t.keys(":e b<Enter>:b 1<Enter>");
    assert_eq!(t.s.win().view.top, 140);
    assert_eq!(t.s.ed.cur.line, 149);
}
#[test]
fn one_window_at_start() {
    let t = T::open(Some("a"), Some("1\n"));
    assert_eq!(t.s.tree.order(), vec![t.s.active]);
    assert_eq!(t.s.win().buf, 0);
}
```
  (`bdelete_closes_its_windows` is added in Task 4, which has `:split`.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --quiet -- session::tests::scroll_survives session::tests::one_window`
Expected: compile errors (`win_mut` not defined).

- [ ] **Step 3: Implement the interfaces above.** `app.rs` reads and
  writes `s.win_mut().view` where it used `ui.view`, passes `&mut s.hl`
  where it passed `hl`, and on `s.reloaded` resets the view only when
  the buffer is new (a flag `Session::fresh: bool` set by
  `switch_to`/`replace`, cleared by the app). `zap.rs:297` and the
  `ui/tests.rs` `View::default()` uses stay as they are.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --quiet 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: every `test result: ok`, no `FAILED`.

- [ ] **Step 5: Commit** (`feat: windows own cursor and scroll; buffers keep theirs`).

### Task 3: Draw every window

**Files:**
- Modify: `src/ui/render.rs` (`draw` at line 704 splits in two)
- Create: `src/ui/windows.rs` (declare in `src/ui/mod.rs`)
- Modify: `src/session.rs` (add `each_window`), `src/app.rs` (`Ui::draw`)
- Test: `src/ui/tests.rs`

**Interfaces:**
- Consumes: `Node::layout`, `Placed` (Task 1); `Session::{windows, tree, active, area, hl}` (Task 2).
- Produces:
  - `render::draw_window(buf: &mut Screen, rect: Rect, ed: &Editor, view: &mut View, hl: &mut Highlighter, cfg: &Config, budget: Duration, active: bool, divider: bool) -> Option<(u16, u16)>`:
    today's text, gutter, zap labels, splash and explain box inside
    `rect` minus its last row (and last column when `divider`), the
    status line on `rect`'s last row, the `│` column when `divider`.
    Returns the cursor cell when `active`. When `!active`: no Visual
    selection, no popup, no zap labels, no explain box, the status line
    in the dim style (`bg Indexed(236)`, `fg Indexed(244)`) and its mode
    label omitted.
  - `render::draw_chrome(f: &mut Frame, area: Rect, ed: &Editor, view: &mut View, hl: &mut Highlighter, cfg: &Config, budget: Duration, cursor: Option<(u16, u16)>)`:
    the command row, picker panel and Magit transient (bottom of `area`),
    the approval dialog (middle of `area`), ex completion, the
    right-click menu and the completion popup, then sets the terminal
    cursor.
  - `render::draw` keeps its signature and output: it calls
    `draw_window` on `area` minus the command row and panel, then
    `draw_chrome`. Every existing `ui/tests.rs` test still passes.
  - `render::chrome_rows(ed: &Editor, area: Rect) -> u16`: command row
    plus panel height, so callers can compute the windows' area.
  - `Session::each_window(&mut self, f: impl FnMut(&Editor, &mut View, &mut Highlighter, Placed, bool))`:
    lays out `tree` in `area`, then calls `f` per window with its
    buffer's editor and highlighter; for an inactive window on the live
    buffer it swaps `win.cur` into `ed.cur` around the call and clamps
    it to the text first.
  - `ui::windows::draw_session(f: &mut Frame, s: &mut Session, cfg: &Config, budget: Duration)`:
    sets `s.area`, draws all windows, then the chrome for the live
    editor. `app.rs` calls it instead of `ui::draw`, and keeps setting
    `s.ed.viewport` and `s.ed.win_height` from the active window's rect.

- [ ] **Step 1: Write the failing tests** (in `src/ui/tests.rs`, with a
  helper `session_screen(w, h, files: &[(&str, &str)]) -> (Session, Screen, TempDir)`)

```rust
#[test]
fn two_windows_two_status_lines() {
    let (mut s, mut sc, _d) = session_screen(40, 10, &[("a", "alpha\n"), ("b", "beta\n")]);
    s.handle_keys(":split b<Enter>");
    sc.draw_session(&mut s);
    assert_eq!(sc.row(0), "beta");
    assert!(sc.row(3).contains("b"));      // status of the top window
    assert_eq!(sc.row(4), "alpha");
    assert!(sc.row(8).contains("a"));      // status of the bottom window
    assert_eq!(sc.cursor(), (0, 0));       // in the new, top window
}
#[test]
fn vsplit_draws_divider() {
    let (mut s, mut sc, _d) = session_screen(41, 6, &[("a", "alpha\n")]);
    s.handle_keys(":vsplit<Enter>");
    sc.draw_session(&mut s);
    assert!(sc.row(0).starts_with("alpha"));
    assert_eq!(sc.row(0).chars().nth(19), Some('│')); // new window: 20 cols incl. divider
}
#[test]
fn inactive_same_buffer_no_visual() {
    let (mut s, mut sc, _d) = session_screen(40, 10, &[("a", "alpha\n")]);
    s.handle_keys(":split<Enter>vl");
    sc.draw_session(&mut s);
    let b = sc.term.backend().buffer();
    assert_ne!(b[(0, 0)].bg, b[(0, 4)].bg);   // top (active) shows Visual
    assert_eq!(b[(0, 4)].bg, b[(3, 4)].bg);   // bottom does not
}
#[test]
fn other_window_cursor_clamped_after_delete() {
    let (mut s, mut sc, _d) = session_screen(40, 10, &[("a", &"x\n".repeat(50))]);
    s.handle_keys("50G:split<Enter>ggdG");
    sc.draw_session(&mut s);   // must not panic
    s.handle_keys("<C-w>j");
    assert_eq!(s.ed.cur.line, 0);
}
```
  (`handle_keys` is a test helper on `Session` that parses and feeds keys;
  `session_screen` opens the first file and runs `:set nonumber<Enter>`
  first so columns start at 0;
  `Screen::draw_session` wraps `ui::windows::draw_session`.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --quiet -- ui::tests::two_windows ui::tests::vsplit_draws ui::tests::inactive_same ui::tests::other_window`
Expected: compile errors. (They need `:split` and `CTRL-W`, which
Task 4 adds: write them now, mark `#[ignore = "needs Task 4"]` after
seeing them fail to compile, and remove the `ignore` in Task 4.)

- [ ] **Step 3: Implement the interfaces above.**

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --quiet 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all ok; the four new tests show as ignored.

- [ ] **Step 5: Commit** (`feat: draw each window with its own status line`).

### Task 4: Splitting and moving: `:split` family, `CTRL-W`, `:wincmd`

**Files:**
- Create: `src/session/windows.rs` (declared next to `session/occur.rs`)
- Modify: `src/ex/cmd.rs` (`ExEffect`, `COMMANDS`), `src/vim/mod.rs`
  (`parse` near line 207, `simple` near line 567), `src/session.rs`
  (`perform`; `show` split in two)
- Test: `src/session.rs` `mod tests`; un-ignore Task 3's four tests

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `ExEffect::Window { cmd: char, count: Option<usize> }` (a `CTRL-W`
    command), `ExEffect::Split { vertical: bool, what: SplitWhat, size: Option<usize> }`,
    `pub enum SplitWhat { Same, File(String), Empty, Buffer(String) }`
    in `src/ex/cmd.rs`.
  - Commands (name, shortest): `split` 2, `vsplit` 2, `new` 3, `vnew` 3,
    `sbuffer` 2, `wincmd` 4. `:[N]split [file]` / `:[N]vsplit [file]`
    give `size: N`; `:wincmd {c}` gives `Window { cmd: c, count: N }`.
  - Normal mode: `CTRL-W` waits for one more key (`Parse::Incomplete`);
    a ctrl key maps to its letter (`CTRL-W CTRL-J` = `CTRL-W j`),
    arrows to `h j k l`; the result is `Cmd::Simple { key: CTRL-W, arg: Some(c) }`
    and `simple` sets `ExEffect::Window { cmd: c, count }`.
  - `src/session/windows.rs`, all `impl Session`:
    - `fn make_live(&mut self, i: usize)`: today's `show` swap, nothing
      about windows. `show(i)` = `make_live(i)` plus the Task 2 view and
      `win.buf`/`win.prev` bookkeeping.
    - `pub fn split(&mut self, vertical: bool, what: SplitWhat, size: Option<usize>)`:
      `Node::split` of the active window (`after` = false: above /
      left, Neovim's defaults until `splitbelow`/`splitright` in plan 2),
      `size` = N text rows + 1 status row (columns + divider when
      vertical); the new window copies the active one's buffer, cursor
      and view, becomes active, then `what` loads `File` (as `:e`),
      `Empty` (a new unnamed buffer) or `Buffer` (as `:b`). On error the
      message is set and nothing changes.
    - `pub fn focus(&mut self, id: usize)`: saves `ed.cur` into the old
      window, sets `prev_win`, `make_live`s the new window's buffer if
      different, then puts its `cur` (clamped) in `ed.cur`.
    - `pub fn wincmd(&mut self, c: char, count: Option<usize>)`:
      `s S v n ^` split (`^` = split showing `#`), `h j k l` via
      `Node::neighbor` with the cursor's screen cell, `w W` next/previous
      (with count: window N), `t b` first/last, `p` previous, `c q o`
      as Task 5. Any other char: Neovim's message for an unknown command
      is none; do nothing.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn split_opens_above_and_moves_there() {
    let mut t = T::open(Some("a"), Some("1\n2\n3\n"));
    t.keys("3G:split<Enter>");
    assert_eq!(t.s.tree.order(), vec![t.s.active, t.s.windows[0].id]);
    assert_eq!(t.s.ed.cur.line, 2);
}
#[test]
fn ctrl_w_moves_keep_each_cursor() {
    let mut t = T::open(Some("a"), Some("1\n2\n3\n4\n5\n"));
    t.keys("3G<C-w>s5G<C-w>j");
    assert_eq!(t.s.ed.cur.line, 2);
    t.keys("<C-w>k");
    assert_eq!(t.s.ed.cur.line, 4);
    t.keys("<C-w><C-j>");
    assert_eq!(t.s.ed.cur.line, 2);
    t.keys("<C-w>p");
    assert_eq!(t.s.ed.cur.line, 4);
}
#[test]
fn vsplit_file_and_window_numbers() {
    let mut t = T::open(Some("a"), Some("a\n"));
    fs::write(t.dir.path().join("b"), "b\n").unwrap();
    t.keys(":vsplit b<Enter>");
    assert_eq!(t.s.ed.buf.text(), "b\n");
    t.keys(":wincmd l<Enter>");
    assert_eq!(t.s.ed.buf.text(), "a\n");
    t.keys("1<C-w>w");
    assert_eq!(t.s.ed.buf.text(), "b\n");
}
#[test]
fn split_count_sets_height() {
    let mut t = T::open(Some("a"), Some("a\n"));
    t.keys(":5split<Enter>");
    let p = t.s.tree.layout(t.s.area);
    assert_eq!(p[0].rect.height, 6); // 5 text rows + status line
}
#[test]
fn split_too_small() {
    let mut t = T::open(Some("a"), Some("a\n"));
    t.s.area = ratatui::layout::Rect::new(0, 0, 80, 3);
    t.keys(":split<Enter>");
    assert_eq!(t.msg(), "E36: Not enough room");
    assert_eq!(t.s.windows.len(), 1);
}
#[test]
fn new_and_sbuffer() {
    let mut t = T::open(Some("a"), Some("a\n"));
    t.keys(":new<Enter>");
    assert_eq!(t.s.ed.buf.text(), "");
    t.keys(":sbuffer 1<Enter>");
    assert_eq!(t.s.ed.buf.text(), "a\n");
    assert_eq!(t.s.windows.len(), 3);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --quiet -- session::tests::split session::tests::ctrl_w session::tests::vsplit_file session::tests::new_and`
Expected: FAIL (`split` is not an editor command).

- [ ] **Step 3: Implement the interfaces above**, then remove the four
  `#[ignore]`s from Task 3.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --quiet 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all ok, nothing ignored from Task 3.

- [ ] **Step 5: Commit** (`feat: :split, :vsplit, :new, CTRL-W moves`),
  with the ledger rows `split vsplit new vnew sbuffer wincmd` and the
  `CTRL-W s S v n ^ h j k l w W t b p` keys set to `partial`.

### Task 5: Closing: `:close`, `:only`, `:hide`, `:q` in a split, `:qall` family, `:bd`

**Files:**
- Modify: `src/session/windows.rs`, `src/ex/cmd.rs`, `src/session.rs`
  (`perform`'s `Quit`, `WriteIfModifiedQuit` and `Write { then_quit }`
  arms; `buffer()` delete path)
- Test: `src/session.rs` `mod tests`

**Interfaces:**
- Consumes: Task 4's `focus`, `wincmd`.
- Produces:
  - `pub fn close_window(&mut self, id: usize) -> Result<(), &'static str>`:
    `Node::close`, drop the `Window`, focus the window that took its
    space (Neovim's `win_altframe`: the one above/left, else
    below/right). The buffer is never deleted, modified or not.
  - `pub fn only(&mut self)`: closes every other window.
  - `ExEffect::Close { force: bool }`, `ExEffect::Only`,
    `ExEffect::QuitAll { force: bool, write: bool }`; commands `close` 3,
    `only` 2, `hide` 3 (= `close`), `qall` 2, `quitall` 5 (= `qall`),
    `wqall` 3, `xall` 2 (= `wqall`). `CTRL-W c` = `:close`,
    `CTRL-W q` = `:quit`, `CTRL-W o` = `:only`.
  - `:q`, `:wq`, `:x` with more than one window: do their write (if any),
    then close the active window instead of quitting. With one window,
    today's behavior exactly.
  - `:qall` = today's `quit_if_all_saved`; `:qall!` quits; `:wqall`
    writes every modified buffer that has a file, then `:qall`
    (stopping, with the write's error, at the first that fails).
  - `:bd` (Task 2's rule): windows showing the deleted buffer close;
    if that would close the last window, it shows `#` as today.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn quit_in_split_closes_window_and_keeps_buffer() {
    let mut t = T::open(Some("a"), Some("a\n"));
    fs::write(t.dir.path().join("b"), "b\n").unwrap();
    t.keys(":split b<Enter>ix<Esc>:q<Enter>");
    assert!(!t.s.quit);
    assert_eq!(t.s.windows.len(), 1);
    assert_eq!(t.s.ed.buf.text(), "a\n");
    assert!((0..t.s.bufs.len()).any(|i| t.s.ed_at(i).buf.text() == "xb\n"));
}
#[test]
fn close_last_window_errors() {
    let mut t = T::open(Some("a"), Some("a\n"));
    t.keys("<C-w>c");
    assert_eq!(t.msg(), "E444: Cannot close last window");
    t.keys(":close<Enter>");
    assert_eq!(t.msg(), "E444: Cannot close last window");
}
#[test]
fn only_and_hide() {
    let mut t = T::open(Some("a"), Some("a\n"));
    t.keys(":split<Enter>:vsplit<Enter>:only<Enter>");
    assert_eq!(t.s.windows.len(), 1);
    t.keys(":split<Enter>:hide<Enter>");
    assert_eq!(t.s.windows.len(), 1);
}
#[test]
fn qall_checks_every_buffer() {
    let mut t = T::open(Some("a"), Some("a\n"));
    t.keys(":split b<Enter>ix<Esc><C-w>j:qa<Enter>");
    assert!(!t.s.quit);
    t.keys(":wqa<Enter>");
    assert!(t.s.quit);
    assert_eq!(t.file("b"), "x\n");
}
#[test]
fn bdelete_closes_its_windows() {
    let mut t = T::open(Some("a"), Some("a\n"));
    fs::write(t.dir.path().join("b"), "b\n").unwrap();
    t.keys(":split b<Enter>:bd<Enter>");
    assert_eq!(t.s.windows.len(), 1);
    assert_eq!(t.s.ed.buf.text(), "a\n");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --quiet -- session::tests::quit_in_split session::tests::close_last session::tests::only_and session::tests::qall_checks session::tests::bdelete_closes`
Expected: FAIL.

- [ ] **Step 3: Implement the interfaces above.**

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --quiet 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all ok, including the existing `quit_rules`.

- [ ] **Step 5: Commit** (`feat: close windows: :close, :only, :q in splits, :qall`),
  with ledger rows `close only hide quit qall quitall wqall xall` and
  `CTRL-W c q o` set to `partial`.

### Task 6: Roadmap, ledger and a run in the real terminal

**Files:**
- Modify: `docs/nvim-core-roadmap.md` (execution queue item 2 and the
  log), `docs/nvim-parity.csv` (check every row touched above)

- [ ] **Step 1:** Add a log entry dated the day of the work: "split
  windows part 1: layout tree (`src/window.rs`), windows in `Session`,
  per-window drawing, `:split` family, `CTRL-W` moves, closing. Tracks 2
  and 17 stay at 0: tabs, resizing and the buffer/window struct split
  remain." Update execution queue item 2 to point at plan part 2
  (resizing, mouse, special buffers, zap, remote, oracle).
- [ ] **Step 2:** Run `cargo build --release` and start
  `target/release/fred src/window.rs` in a terminal: `:vsplit`, `:split
  src/lib.rs`, `CTRL-W h/j/k/l`, type in one window and watch the other,
  `:q` twice. Expected: as the spec's Screen section; no panic on a
  terminal resize.
- [ ] **Step 3: Commit** (`docs: roadmap and ledger for split windows part 1`).

