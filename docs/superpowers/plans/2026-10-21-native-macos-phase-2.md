# Native macOS Fred, Phase 2: Tabs and Minimap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Sublime-style tabs, one strip per split window, and a
Sublime-style minimap on the right edge of each window, to Fred.app.

**Architecture:**
- **Tabs:** tabs are new core state. Each `Window` keeps the list of
  buffers it has shown and an optional preview buffer. The core owns every
  rule (open, preview, close, move, `:bn`/`:bp`). The app only draws the
  strip and sends tab operations.
- **GUI layout:** each window reserves one cell row at its top for the tab
  strip, and N cell columns at its right for the minimap. The cell grid stays
  uniform, so mouse mapping and cursor positions stay simple.
- **Minimap data:** a separate, throttled `minimap` message carries
  per-line color runs. It is sent only when a buffer changes, never on the
  key path.

**Tech Stack:** Rust (src/gui, src/session, src/window), Swift 6 AppKit
(macos/), JSON-lines protocol.

**Spec:** `docs/superpowers/specs/2026-10-21-native-macos-app-design.md`
(Phase 2: "Tabs, splits, the status bar and the minimap"; splits and the
status bar shipped in Phase 1). Phase 1 plan with the protocol:
`docs/superpowers/plans/2026-10-21-native-macos-phase-1.md`.

## Global Constraints

- Everything in the Phase 1 plan's Global Constraints still holds:
  - macOS 26+;
  - Swift in `macos/`, with no Rust linked in;
  - latency p99 < 16 ms (8 ms aim);
  - crisp text;
  - no character-cell chrome.
- Spec, "What a tab is" (verbatim intent):
  - Each window keeps a list of the buffers it has shown, like Sublime
    groups.
  - A single click in the sidebar opens a preview tab, shown in italics,
    which the next preview replaces.
  - Editing it or double-clicking makes it permanent.
  - Closing a tab removes it from that window only; `:bd` still deletes
    the buffer everywhere.
  - `:bn`, `:bp` and Ctrl-^ move through the window's tabs.
  - `:ls` and `Space b` still list every buffer.
- **The terminal Fred must not change.** Tab state may exist in the core,
  but the terminal shows no tab row and no minimap.
  - `:bn`/`:bp` keep Neovim's all-buffers behavior in the terminal. They
    follow tabs only when `Session::gui` is true.
  - Ctrl-^ (the window's alternate, `prev`) is unchanged everywhere, since
    the alternate is always one of the window's tabs.
- The protocol stays additive. Every new field is optional or defaulted on
  both sides.
- The user is working on this Mac. No agent may send synthetic keystrokes or
  clicks, or activate windows. Verify through tests, the pipe and
  `screencapture` only.
- AGENTS.md:
  - Preserve `notes.md`.
  - Never commit `claude-features.txt` or `sort-task.md`.
  - Commit messages end with
    `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Decisions made while planning

- **Tab strip height is one cell row**, reserved at the top of every window
  in GUI mode. Minimap width is 96 pt, reserved as
  `ceil(96 / cellWidth)` columns at the right of every window. The app
  sends the column count in `resize`, and 0 turns the minimap off.
- **Cmd-W closes the active tab**, as in Sublime, not the window. Closing a
  window's last tab:
  - closes the window if there are other windows;
  - otherwise shows an empty `[No Name]` buffer in it.
- **Opening a buffer adds it to the window's tabs:** right after the current
  tab, unless it is already there, in which case it just becomes current.
  This covers `:e`, the picker, the sidebar, `:b`, Magit, the Claude chat,
  the terminal and help.
- **A preview tab becomes permanent when** its buffer's text changes, it is
  re-opened without `preview`, or the user double-clicks it. A window has
  at most one preview tab; opening a new preview replaces the old one in
  place. The replaced buffer stays open (hidden) unless it was opened only
  as that preview and is unmodified, in which case the core deletes it like
  `:bd`.
- **Minimap scale:** 1 pt per column, 2 pt per line. Up to 96 columns are
  drawn. Files over 50 000 lines get no minimap, and the `minimap` message
  carries `"lines": null`.

## The protocol additions

**Core → app, per `Win` (all optional or defaulted):**

```json
{"id":3, "header":1, "margin":8, "top":120, "lines":2048,
 "tabs":[{"name":"main.rs","path":"/abs/src/main.rs","modified":false,"preview":false,"current":true}]}
```

- `id` is the window id, used by the operations below.
- `header` is the number of rows reserved at the window's top for the tab
  strip (0 or 1). `rows` now starts at `y + header` and has
  `h - header - 1` entries.
- `margin` is the number of columns reserved at the window's right for the
  minimap. Text rows never cover them.
- `top` is the first buffer line shown. `lines` is the buffer's line count.
- `tabs` is in display order, with exactly one `current`.

**Core → app, new message, at most every 250 ms per buffer, only after a
frame and never in the same loop iteration as handling a key:**

```json
{"t":"minimap","win":3,"version":17,"lines":[[[0,4,16737970],[5,3,null]], []]}
```

- `lines` holds one entry per buffer line, made of `[col, len, color]`
  runs. Whitespace is skipped, so runs never contain it. `color` uses the
  run color encoding, with `null` for the default.
- `lines` is `null` when the file is over 50 000 lines.
- It is sent for each window whose buffer `version` changed since that
  window's last minimap.

**App → core:**

```json
{"t":"resize","cols":120,"rows":40,"margin":8}
{"t":"open","seq":5,"path":"/abs/f.rs","preview":true}
{"t":"tab","seq":6,"win":3,"op":"select","index":2}
{"t":"tab","seq":7,"win":3,"op":"close","index":2}
{"t":"tab","seq":8,"win":3,"op":"move","index":2,"to":0}
{"t":"tab","seq":9,"win":3,"op":"keep","index":2}
{"t":"scroll","seq":10,"win":3,"top":400}
```

- `margin` and `preview` default to 0 and false.
- `keep` makes a preview tab permanent.
- `scroll` sets that window's first shown line, clamped. It does not move
  that window's cursor unless the cursor would leave the screen, which
  follows the same rule as wheel scrolling.
- The `mac` key `w` now closes the active tab.

## File Structure

- **Rust:**
  - `src/window.rs`: `Window` gains `tabs` and `preview`. `Placed` gains
    `header` and `margin`.
  - `src/session/tabs.rs` (new): the tab rules (`add_tab`, `close_tab`,
    `move_tab`, `select_tab`, `keep_tab`, `next_tab`, plus the hooks in
    `show`, `switch_to` and `drop_buffer`).
  - `src/session.rs`: `gui: bool`, `chrome: (u16, u16)`,
    `pub fn placed(&self) -> Vec<Placed>`, and the hooks.
  - `src/ui/render.rs` and `src/ui/windows.rs`: honour `header` and
    `margin`.
  - `src/gui/{proto.rs,frame.rs,mod.rs,mac.rs}`, and a new
    `src/gui/minimap.rs`.
- **Swift:**
  - `macos/Sources/Fred/TabStrip.swift` (new) and
    `macos/Sources/Fred/Minimap.swift` (new).
  - `macos/Sources/FredKit/Protocol.swift`, plus
    `macos/Sources/FredKit/MinimapMath.swift` (new, pure) and
    `macos/Sources/FredKit/TabMath.swift` (new, pure).
  - `EditorView.swift`, `main.swift` and `Sidebar.swift`.

## Review Focus

1. **Deleting a buffer that sits in several windows' tabs.** `:bd` and the
   buffer-index shift in `drop_buffer` must leave every window's `tabs` and
   `preview` pointing at the right buffers. The test is in Task 1.
2. **Closing the current tab while its buffer has unsaved changes.** The
   tab closes and the buffer stays open (hidden), as `:hide` does. Nothing
   is lost, and `:ls` still shows it with `+`. The test is in Task 1.
3. **Every layout path honours the header row and margin.** That covers
   mouse clicks, drag selection, zap labels, the cursor, the which-key
   popup, window numbers and divider dragging. Off-by-one-row clicks are the
   classic bug. The test is in Task 2: a mouse click on the first text row
   lands on buffer line 1.
4. **A burst of typing in a large file.** The minimap must not be built on
   the key path or more often than every 250 ms. The test is in Task 3: 50
   keys in one batch produce at most one minimap message.
5. **A preview replaced while modified.** Typing into a preview makes it
   permanent first, so a modified buffer is never silently replaced. The
   test is in Task 1.

---

## Track A: core (Tasks 1–3, in order)

### Task 1: Tab state and rules in the core

**Files:**
- Create: `src/session/tabs.rs`
- Modify: `src/window.rs` (`Window`), `src/session.rs` (`show`,
  `switch_to`, `drop_buffer`, `buffer()` Next/Prev, `mod tabs;`, and a
  `pub gui: bool` field defaulting to false)

**Interfaces (produces):**
- `Window { …, pub tabs: Vec<usize>, pub preview: Option<usize> }`.
  `tabs` holds buffer indices in display order. `preview`, when set, is an
  index that is also in `tabs`.
- On `Session`:
  - `pub fn select_tab(&mut self, win: usize, index: usize) -> Result<(), String>`
  - `pub fn close_tab(&mut self, win: usize, index: usize) -> Result<(), String>`
  - `pub fn move_tab(&mut self, win: usize, index: usize, to: usize) -> Result<(), String>`
  - `pub fn keep_tab(&mut self, win: usize, index: usize) -> Result<(), String>`
  - `pub fn open_preview(&mut self, path: &Path, preview: bool)`: what the
    `open` message calls. It reuses the existing `open_pick` path, then
    applies the preview rules.
- `win` is a window id, and `index` is a position in that window's `tabs`.
  An unknown window or an out-of-range index returns
  `Err("no such tab")`.
- Invariant, checked by a `debug_assert` helper `check_tabs()` that the
  tests call: the shown buffer `w.buf` is in `w.tabs`, every index is
  `< bufs.len()`, there are no duplicates, and `preview ⊆ tabs`.

- [ ] **Step 1: Write the failing tests** in `src/session/tabs.rs`. Use the
  existing session test helpers (see `session_screen` in
  `src/ui/tests.rs`, or the `T` helper in `src/session.rs` tests).

```rust
#[test] fn opening_files_adds_tabs_after_the_current_one()   // :e a, :e b, :b a, :e c → tabs [a, c, b]
#[test] fn closing_the_current_tab_shows_its_right_neighbour_then_left()
#[test] fn closing_a_modified_tab_keeps_the_buffer_hidden()  // :ls still lists it with "+"
#[test] fn closing_the_last_tab_closes_the_window_or_leaves_no_name()
#[test] fn bd_removes_the_buffer_from_every_windows_tabs_and_shifts_indices()
#[test] fn a_preview_is_replaced_by_the_next_preview_in_place()
#[test] fn editing_a_preview_keeps_it()                      // type in it, then preview another: both tabs stay
#[test] fn an_unmodified_preview_only_buffer_is_deleted_when_replaced()
#[test] fn bn_and_bp_follow_tabs_only_in_gui()               // gui=false: all buffers (unchanged)
#[test] fn move_tab_reorders()
```

  Each test ends with `s.check_tabs()`.

- [ ] **Step 2:** Run `PATH="/opt/homebrew/opt/zig@0.15/bin:$PATH" cargo test --features ghostty session::tabs`.
  Expected: FAIL to compile.
- [ ] **Step 3:** Implement the state, the rules and the hooks.
  - "Editing a preview keeps it" is detected where the session sees an
    edit, by comparing `ed.buf.version` with the version recorded when the
    preview opened. Store it as `preview: Option<(usize, u64)>` if that is
    simpler; it is internal.
  - The terminal path must behave exactly as before:
    `cargo test --features ghostty` passes the full suite, apart from the
    known `magit::tests::bisect_finds_the_bad_commit_and_runs_scripts`.
- [ ] **Step 4:** Run the tests above, then the full suite. Expected: PASS.
- [ ] **Step 5:** Commit `src/session/tabs.rs`, `src/session.rs` and
  `src/window.rs`: "feat(session): windows keep tabs, Sublime-style".

### Task 2: GUI layout chrome and the tab protocol

**Files:**
- Modify:
  - `src/window.rs` (`Placed` gains `pub header: u16, pub margin: u16`,
    both defaulting to 0);
  - `src/session.rs` (`pub chrome: (u16, u16)` = (header rows, margin
    cols), defaulting to (0, 0), and `pub fn placed(&self) -> Vec<Placed>`
    = `tree.layout(self.area)` with `header` and `margin` filled in);
  - every `tree.layout(` call site (9: `src/session.rs`, `src/app.rs`,
    `src/ui/windows.rs`, `src/session/windows.rs`) → `placed()`;
  - `src/ui/render.rs` (`draw_window`'s text area starts `header` rows
    lower and ends `margin` columns earlier, and `mouse`/`text_pos` use the
    same areas);
  - `src/gui/{proto.rs, frame.rs, mod.rs, mac.rs}`.
- Test: `src/gui/frame.rs` tests, `tests/gui.rs`.

**Interfaces:**
- Consumes: Task 1's tab functions and `Window.tabs`/`preview`, and
  `Session::gui`.
- Produces, in proto:
  - `Tab { name: String, path: Option<String>, modified: bool, preview: bool, current: bool }`
  - `Win` gains `id: usize`, `header: u16`, `margin: u16`, `top: usize`,
    `lines: usize` and `tabs: Vec<Tab>`. All are `#[serde(default)]`.
  - `In::Resize` gains `#[serde(default)] margin: u16`.
  - `In::Open` gains `#[serde(default)] preview: bool`.
  - New `In::Tab { seq, win, op: String, index: usize, #[serde(default)] to: usize }`
    and `In::Scroll { seq, win, top: usize }`.
- `gui::run` sets `s.gui = true` and `s.chrome = (1, margin)` on every
  resize.
- `mac_key('w')` closes the active window's current tab (Task 1's
  `close_tab`), in every mode, after Esc.

- [ ] **Step 1: Write the failing tests.**

```rust
// src/gui/frame.rs
#[test] fn gui_windows_reserve_a_tab_row_and_a_minimap_margin() // chrome (1,8), 40x12: rows.len()==h-2, no run reaches x >= w-8, Win.header==1, margin==8
#[test] fn the_terminal_layout_is_unchanged()                   // chrome (0,0): rows.len()==h-1 as before
#[test] fn tabs_and_top_and_lines_are_in_the_frame()
// tests/gui.rs
#[test] fn a_click_on_the_first_text_row_lands_on_line_one()    // resize margin 8, click row 1 (under the tab row) → cursor line 1
#[test] fn tab_ops_select_close_move_keep_over_the_pipe()
#[test] fn open_preview_then_open_preview_replaces_the_tab()
#[test] fn scroll_sets_the_windows_top()
#[test] fn cmd_w_closes_the_tab_not_the_window()
```

- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. Also check that zap labels, window numbers
  (`Space w`), which-key and divider/status dragging use `placed()`, and
  that their tests still pass.
- [ ] **Step 4:** Run the full suite. Expected: PASS, apart from the known
  bisect failure.
- [ ] **Step 5:** Commit: "feat(gui): a tab row and minimap margin per window; tab, scroll and preview messages".

### Task 3: Minimap data

**Files:**
- Create: `src/gui/minimap.rs`
- Modify: `src/gui/proto.rs` (`Minimap { t: "minimap", win: usize, version: u64, lines: Option<Vec<Vec<(u16, u16, Option<u32>)>>> }`),
  `src/gui/mod.rs` (send after frames, throttled).

**Interfaces:**
- Produces: `pub fn build(ed: &Editor, hl: &mut Highlighter, budget: Duration) -> Option<Vec<Vec<(u16, u16, Option<u32>)>>>`.
  - It returns `None` over 50 000 lines.
  - Columns are display columns, with tabs expanded by the buffer's
    tabstop. Up to 96 columns per line; the rest is cut.
  - Colors come from the highlighter's styles (fg only) via
    `proto::color`. Lines the highlighter hasn't reached within `budget`
    get `None` colors.
  - Magit, Org, dired and terminal buffers use their own styles if cheap.
    Otherwise all `None` (plain text shape).
- The send rule in `gui::run`: after writing a frame, for each window whose
  buffer `version` differs from that window's last minimap version, and at
  least 250 ms after that window's last minimap, build and send one
  `minimap` line. Never do this in a loop iteration that handled a `keys`,
  `paste` or `mac` message; wait for an idle iteration.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn runs_skip_whitespace_and_expand_tabs()      // "  ab\tc" tabstop 4 → [(2,2,_),(4,1,_)]
#[test] fn long_lines_are_cut_at_96_columns()
#[test] fn huge_files_have_no_minimap()                // 50_001 lines → None
#[test] fn rust_keywords_carry_their_color()           // material-dark: "fn" run color == 0x1000000|0xC792EA
// tests/gui.rs
#[test] fn a_minimap_follows_the_first_frame()
#[test] fn fifty_keys_in_one_batch_send_at_most_one_minimap()
```

- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests and the full suite. Also measure: the core
  round trip p99 for 300 `j` plus Insert typing, with the Phase 1 method (a
  script over the pipe), must not regress by more than 1 ms against
  Phase 1's 9.7 ms. Record both numbers.
- [ ] **Step 5:** Commit: "feat(gui): minimap data, throttled off the key path".

---

## Track B: app (Tasks 4–5; can start on the protocol above, before Track A lands)

### Task 4: Native tab strip

**Files:**
- Create: `macos/Sources/Fred/TabStrip.swift`,
  `macos/Sources/FredKit/TabMath.swift`
- Modify:
  - `macos/Sources/FredKit/Protocol.swift` (`Tab`; the new `Win` fields as
    optionals; `Outgoing.tab(seq:win:op:index:to:)`,
    `.scroll(seq:win:top:)`, `.open(seq:path:preview:)` and
    `.resize(cols:rows:margin:)`);
  - `EditorView.swift` (place one `TabStrip` per window over its header
    row, reused like the status bars, and draw rows from `y + header`);
  - `main.swift` (Cmd-Shift-] / Cmd-Shift-[ for next/previous tab and
    Cmd-1…9 to select a tab, both via `tab select`; Cmd-W stays `mac w`);
  - `Sidebar.swift` (a single click opens with `preview: true`; a
    double-click opens with `preview: false`).

**Look (Sublime, Material Dark):**
- The strip background is the window background darkened 6%.
- Each tab is a rounded-top rect, 6 pt corner radius, sized to its name
  plus padding: minimum 90 pt, maximum 220 pt.
- When the tabs don't fit, they shrink evenly down to 90 pt, then the
  strip scrolls horizontally, keeping the current tab visible.
- The current tab takes the editor background, #212121, so it joins the
  text area. Other tabs are transparent with secondary label text.
- The name uses SF 12. A preview tab's name is italic.
- A modified tab shows a `●` in place of the close button; on hover the
  `●` becomes `×`. A hovered tab shows `×` on its right.
- The active window's strip is at full opacity, inactive windows' strips
  at 0.7.
- Interaction:
  - click selects;
  - `×` and a middle-click close;
  - double-clicking a preview tab sends `keep`;
  - dragging a tab within its strip reorders it (`move`);
  - right-click opens a menu with Close, Close Others, Close Tabs to the
    Right, Copy Path and Reveal in Sidebar. Close Others and Close to the
    Right are a sequence of `close` ops sent right to left.

- [ ] **Step 1: Write the failing self-test checks** for
  `TabMath.layout(widths: [CGFloat], available: CGFloat, current: Int) -> (frames: [CGRect], offset: CGFloat)`:
  - tabs that fit keep their natural widths;
  - an overflow shrinks every tab to at least 90;
  - an overflow beyond that scrolls so the current tab is fully visible.
- [ ] **Step 2:** Run `swift run --package-path macos FredSelfTest`.
  Expected: FAIL to compile.
- [ ] **Step 3:** Implement. Decode the new fields as optionals, so a frame
  without them, from an older core, still decodes; add a self-test for
  that.
- [ ] **Step 4:** Run the self-test, run `./macos/build.sh`, then launch
  your own copy with `open -g` (or launch it in the background) on the repo
  and open a few files through the core's protocol. Screenshot it with
  `screencapture -x` and report the path. No synthetic input.
- [ ] **Step 5:** Commit: "feat(macos): Sublime-style tab strips".

### Task 5: Native minimap

**Files:**
- Create: `macos/Sources/Fred/Minimap.swift`,
  `macos/Sources/FredKit/MinimapMath.swift`
- Modify: `EditorView.swift` (one `Minimap` view per window over its
  margin columns; `resize` sends `margin = ceil(96 / cell.width)`, or 0
  when hidden), `main.swift` (View ▸ Show/Hide Minimap, saved in
  UserDefaults, on by default), `Core.swift` / `Protocol.swift` (decode
  `minimap` messages into a per-window cache keyed by window id and
  version).

**Look and behavior:**
- Each line is drawn 2 pt tall. Each run is a 1 pt-per-column rectangle,
  1.5 pt tall, in its color at 0.75 alpha. `null` uses fg at 0.45 alpha.
- Rendered once per minimap message into a cached `CGImage` or layer
  contents, not per frame.
- A viewport box over the lines in `top ..< top + visibleRows` is filled
  white at 0.07 alpha. It shows on hover or always; always is Sublime's
  default, so use always.
- **Scrolling the minimap itself:** if `lines * 2 <= height` it doesn't
  scroll. Otherwise its offset is
  `top / max(1, lines - visibleRows) * (lines * 2 - height)`.
- A click centres the view on the clicked line (sends `scroll`). Dragging
  the viewport box scrolls continuously, sending `scroll` at most once per
  display frame.
- With `lines: null` (a huge file), the margin stays empty.

- [ ] **Step 1: Write the failing self-test checks** for `MinimapMath`:
  - `offset(top:lines:visible:height:)` for a short and a long file;
  - `line(atY:offset:)`;
  - `topForCentering(line:visible:lines:)` clamped to `0…lines-visible`.
- [ ] **Step 2:** Run the self-test. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the self-test and the build, then screenshot your own
  copy (`open -g`) on a long file such as `src/session.rs`. Check that the
  minimap shows the code shape and colors, and the viewport box. Report
  the path. No synthetic input.
- [ ] **Step 5:** Commit: "feat(macos): Sublime-style minimap".

## Parallelism

- **Track A:** one agent, Tasks 1→2→3, worktree branch `p2-core` from
  `main`.
- **Track B:** one agent, Tasks 4→5, worktree branch `p2-app` from `main`.
  It codes against the protocol above. A fake core is optional; the
  pure-math self-tests carry the checks until merge.
- **Merge:** `p2-core` into `p2-app`, then a joint screenshot check and
  the final whole-branch review.
