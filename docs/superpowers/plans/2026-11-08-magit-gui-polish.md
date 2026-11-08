# Magit GUI Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In Fred's macOS app, draw Magit buffers and the transient key menu "polished" (section bands, chips, badges, hunk boxes, key chips) on the existing cell grid, with the TUI unchanged.

**Architecture:** The core classifies each visible Magit row (and the transient panel rows) into a *role* plus in-row *marks* (column spans), and sends them in the frame beside the existing runs. The Swift app draws decorations from those tags; untagged rows draw exactly as today. Classification reuses the signals `magit_styles` already uses (row text, `RowAction`, hunk state).

**Tech Stack:** Rust core (`src/magit`, `src/ui/render.rs`, `src/gui`), Swift/AppKit (`macos/Sources/Fred`).

**Spec:** `docs/superpowers/specs/2026-11-07-magit-gui-polish-design.md`

## Global Constraints

- Monospace editor font only inside Magit buffers; every decoration sits on the cell grid; no column moves.
- No change to Magit text, order, keys, behavior, or TUI rendering.
- Scope: Magit-row buffers (status, log, diff, revision, stash, refs, reflog, cherry …). Not blame, rebase todo, commit message.
- Non-Magit frames serialize exactly as before (new fields `skip_serializing_if` empty).
- Transient menu stays a bottom panel, laid out by the core as today.
- Chevron lives in the GUI's one-column left pad (`gui_pad`, `src/ui/render.rs`); clicking it folds (sends Tab); clicking elsewhere behaves as today.
- Ref chips: header lines (Head/Merge/Rebase/Push/Upstream) only in this plan. Commit rows carry refs as plain words (`ref_labels`, `src/magit/repo.rs:476`) with no offsets; chips there need the row builders to record spans and are deferred (spec deviation, noted to the user).

## Execution results (November 9)

Implemented and built; installed at `/Applications/Fred.app`. On the user's
commit request, implementation changes were isolated from overlapping, uncommitted
chat/native-menu work using selective staging. The five planned commit slices are
consolidated into one GUI implementation commit, including the follow-up leader
menu animation. Unrelated changes remain in the working tree.

- Wire format: `role: {kind: "heading", open: true}` (other roles carry only
  `kind`); marks are `[start, end, kind]`. Empty hints are omitted. The frame
  converts buffer character ranges to visible cell ranges, accounting for tabs,
  Unicode, gutter and horizontal scrolling; `paint_gutter` locates the pad.
- Diff/revision file headings have no RowAction. Their existing `diff_files`
  map supplies file hints and click-to-fold eligibility without changing actions.
- Menu selection uses a faint band rather than reversed text, keeping the
  selected entry legible over the darker panel. Off options use dim foreground.
- Panel animation is 120 ms and respects macOS Reduce Motion. Chevrons are
  stroked on the grid rather than SF images. Groups use the editor's bold
  monospace face (no proportional font).
- Pinned `magit-section.el:1164` confirms mouse folding uses section toggle;
  Fred's adaptation intentionally limits the new single-click target to the pad.
- Tests: `cargo test --features ghostty --lib`: **767 passed, 10 ignored**.
  Swift build and `swift run FredSelfTest`: **135 checks passed**.
  Isolated staged-snapshot verification (without the pre-existing chat work):
  **763 passed, 10 ignored**, Swift build passed, **135 Swift checks passed**.
  This includes the leader menu's separate frame layer and 120 ms animation;
  the buffer underneath stays stationary and Reduce Motion is respected.
  `./macos/build.sh`: release core and app built successfully.
- Visual inspection: status (collapsed and expanded), diff, log and commit
  transient captured; pad clicks expand status diffs and collapse revision/diff
  files. Fixed the selected-menu contrast found during this check.
- Broader `cargo test --features ghostty` stops at seven `tests/gui.rs`
  padding/coordinate assertion failures (the pre-existing GUI pad is enabled
  in HEAD). Strict Clippy stops at `src/session.rs:1550`, a nested `if` in
  the pre-existing chat changes; with `collapsible_if` allowed, library Clippy
  passes. These unrelated files/tests were not repaired in this batch.
- Remaining: independent review. Commit-row ref chips,
  blame, rebase todo and commit-message decoration remain out of scope as above.
  Overall Magit acceptance remains **5/100**, not increased by visual polish.

## Review Focus

1. A collapsed section heading ends in `…` — still classified `Heading`, chevron shows ▸.
2. `+++`/`---` file header lines in diff/revision buffers are not `Added`/`Removed`.
3. A context line (leading space) outside any hunk (e.g. an indented log body) is not `Context` and gets no hunk box.
4. A window scrolled so its top row is mid-hunk still boxes/fills the visible hunk rows (hunk state seeded by scanning upward).
5. A heading without a count (`Recent commits`) gets no count mark.

---

### Task 1: Row classifier in the core

**Files:**
- Create: `src/magit/paint.rs` (add `pub mod paint;` in `src/magit/mod.rs`)
- Test: `src/magit/paint.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub enum Role { Heading { open: bool }, File, HunkHeader, Added, Removed, Context }` (derive `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize`, `#[serde(rename_all = "snake_case")]`)
  - `pub enum Mark { Count, RefLocal, RefRemote, Status, Key, Group, OptOn, OptOff }` (same derives)
  - `#[derive(Default, …)] pub struct Paint { pub role: Option<Role>, pub marks: Vec<(u16, u16, Mark)> }` — marks are char-column ranges `start..end` within the row text.
  - `pub fn paint_rows(action_at: impl Fn(usize) -> Option<RowAction>, line: impl Fn(usize) -> String, from: usize, to: usize) -> Vec<Paint>` — one `Paint` per buffer line `from..to` (closures keep it testable without a repo).

- [x] **Step 1: Write failing tests** (closures over a `&[&str]` and a `Vec<Option<RowAction>>`):
  - `heading_with_count`: `"Unstaged changes (2)"` with `Section` action → `Heading { open: true }`, marks contain `(17, 20, Count)`.
  - `collapsed_heading`: `"Untracked files (1)…"` → `Heading { open: false }`, `(16, 19, Count)`.
  - `hunk_rows`: `["modified   src/a.rs", "@@ -1,2 +1,2 @@", " ctx", "-old", "+new", "Recent commits"]`, row 0 `File`, row 5 `Section` → roles `[File, HunkHeader, Context, Removed, Added, Heading{open:true}]`; row 0 marks contain `(0, 8, Status)`; row 5 has no `Count`.
  - `file_headers_not_signs`: `"+++ b/x"`, `"--- a/x"` → role `None`.
  - `seeded_mid_hunk`: same lines, `from = 3, to = 5` → `[Removed, Added]`.
  - `context_outside_hunk`: `["abc1234 subject", "    body line"]` → both role `None`.
  - `head_line_ref`: `"Head:     main subject"` → `(10, 14, RefLocal)`; `"Push:     origin/main subject"` → `(10, 21, RefRemote)`.

- [x] **Step 2: Run** `cargo test --features ghostty --lib magit::paint` — expect FAIL (module missing).

- [x] **Step 3: Implement `paint_rows`.** Seed `in_hunk` by scanning upward from `from` (max 5000 lines) the way `magit_styles` does (`@@` → in hunk; a line not starting with `+`/`-`/space/`\` → stop). Per line: `@@` → `HunkHeader` (in_hunk = true); in hunk and first char `+`/`-`/space and not `+++`/`---` → `Added`/`Removed`/`Context`; any other line ends the hunk. `RowAction::Section` → `Heading { open: !text.ends_with('…') }`, `Count` over a trailing `(N)` (before any `…`). `RowAction::File` → `File`, `Status` over a leading status word (`modified|new file|deleted|renamed|untracked|unmerged|copied|typechange`). Lines starting `Head:`/`Merge:`/`Rebase:` → `RefLocal` over the first token from column 10; `Push:`/`Upstream:` → `RefRemote`.

- [x] **Step 4: Run** the same command — expect PASS.

- [x] **Step 5: Commit** `feat(magit): classify rows for the GUI's polish`.

### Task 2: Send row paint in the frame

**Files:**
- Modify: `src/gui/proto.rs` (`Win`), `src/gui/frame.rs` (`build`), `macos/Sources/FredKit/Protocol.swift` (`Win`)
- Test: `src/gui/frame.rs` tests

**Interfaces:**
- Consumes: `paint_rows`, `Paint`, `Role`, `Mark` (Task 1).
- Produces: `Win.paint: Vec<Paint>` (`skip_serializing_if = "Vec::is_empty"`), aligned with `Win.rows`. Swift: `public var paint: [RowPaint]?`, `struct RowPaint: Codable { var role: String?; var open: Bool?; var marks: [[Mark]]? }` — wire shape decided by serde of `Paint` (role as `"heading"`/`{"heading":{"open":true}}` etc.); Swift decodes to `role: String`, `open: Bool`, `marks: [(start, end, kind)]`.

- [x] **Step 1: Failing test** `magit_window_sends_paint`: open Magit status on a temp repo with one modified file (pattern from existing Magit GUI tests); build a frame; the Magit window's `paint.len() == rows.len()` and one entry is a `Heading`; a non-Magit window has empty `paint`.
- [x] **Step 2: Run** `cargo test --features ghostty --lib gui::frame` — FAIL.
- [x] **Step 3: Implement:** in the `each_window` pass, for Magit editors call `paint_rows(|l| view.action_at(l), |l| ed.buf.line(l).to_string(), view.top, view.top + rows)` (Magit buffers don't wrap: text row r ↔ line `top + r`).
- [x] **Step 4: Run** — PASS; `cd macos && swift build` — Build complete.
- [x] **Step 5: Commit** `feat(gui): Magit row roles and marks in the frame`.

### Task 3: Transient panel tags

**Files:**
- Modify: `src/ui/render.rs` (`transient_lines` ~1777, panel draw ~1337), `src/gui/proto.rs` (`Frame.bottom_paint`), `src/gui/frame.rs`, Swift `Frame`.

**Interfaces:**
- Produces: `pub fn transient_layout(ed: &Editor, selected: usize, width: usize) -> (Vec<Line<'static>>, Vec<Paint>)`; `transient_lines` becomes a wrapper returning `.0`. Marks: `Key` over each key, `Group` over each heading, `OptOn`/`OptOff` over each option value. `Frame.bottom_paint: Vec<Paint>` aligned with `Frame.bottom` (panel rows carry the transient paint; other bottom rows default); empty with no menu.

- [x] **Step 1: Failing test** `commit_menu_paint`: Magit open, press `c`; the paint has a `Group` mark on the row whose text contains "Arguments", and `Key` marks whose spans in their lines read `-a` and `c`.
- [x] **Step 2: Run** `cargo test --features ghostty --lib commit_menu_paint` — FAIL.
- [x] **Step 3: Implement** as above; the frame fills `bottom_paint` for the same rows the panel draws.
- [x] **Step 4: Run** — PASS.
- [x] **Step 5: Commit** `feat(gui): tag the transient panel's keys, groups and options`.

### Task 4: Swift drawing

**Files:**
- Create: `macos/Sources/Fred/MagitPaint.swift` (an `EditorView` extension)
- Modify: `macos/Sources/Fred/EditorView.swift` (`draw`: after backgrounds, before text, for windows with `paint` and for bottom rows with `bottom_paint`)

**Interfaces:**
- Consumes: `Win.paint`, `Frame.bottom_paint`.
- Produces: `func drawPaint(_ ctx: CGContext, _ w: Win)` and `func drawPanelPaint(_ ctx: CGContext, _ f: Frame)`.

Values (mockup A; tune by eye):
- Heading: band fill `fg @ 0.035`, hairline top `fg @ 0.06`, chevron (SF `chevron.right` / `chevron.down`, 9 pt, secondary) centred in the pad column (one cell left of the text).
- Count: pill radius 8, fill `fg @ 0.08`; the parentheses' cells painted over in band color.
- Ref chips: radius 9; local `systemBlue @ 0.18`, remote `fg @ 0.07`.
- Status badge: radius 4; modified `systemYellow @ 0.18`, new file `systemGreen @ 0.18`, deleted `systemRed @ 0.18`, renamed `systemBlue @ 0.18`.
- Hunk: one rounded rect (radius 6, hairline `fg @ 0.07`) around each contiguous `HunkHeader…` run, left edge in the pad column; header fill `systemBlue @ 0.08`; Added `systemGreen @ 0.13`, Removed `systemRed @ 0.13` across the text width — and skip the core's run backgrounds on Added/Removed rows (they replace indexed 22/52).
- Heading/File rows: hover fill `fg @ 0.06`.
- Panel: bg blended 0.25 toward black, hairline top `fg @ 0.1`; Key chip radius 4 `systemBlue @ 0.16`; Group text redrawn semibold; OptOn `systemYellow`, OptOff dim; 0.12 s slide-up when the panel first appears.

- [x] **Step 1:** Implement; `cd macos && swift build` — Build complete.
- [x] **Step 2:** `./build.sh`, install, `Space m s` in this repo with a modified file; screenshot status (expand a file with Tab), a diff, the log, and the commit menu (`c`); check Review Focus 1–5.
- [x] **Step 3: Commit** `feat(gui): draw Magit polished`.

### Task 5: Click a chevron to fold

**Files:**
- Modify: `src/ui/render.rs` mouse handling (~376-386).
- Test: `src/ui/tests.rs`.

- [x] **Step 1: Failing test** `click_in_pad_on_heading_folds`: `GUI_PAD` on, Magit status with a section; a left click at the pad column of the heading row collapses it (heading ends with `…`); the same on a file row expands its diff; a click on the heading's text does not fold.
- [x] **Step 2: Run** `cargo test --features ghostty --lib click_in_pad` — FAIL.
- [x] **Step 3: Implement:** in Magit buffers with the GUI pad, a left click in the pad column on a `Section` or `File` row sends Tab (the existing magit-mouse-toggle-section path).
- [x] **Step 4: Run** — PASS; full `cargo test --features ghostty --lib`.
- [x] **Step 5: Commit** `feat(magit): click the chevron to fold`.
