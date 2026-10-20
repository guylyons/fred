# Split windows: design

Date: 2026-10-19. Roadmap tracks 2 (buffer / window separation) and 17
(windows), in `docs/nvim-core-roadmap.md`.

## Intent

fred shows one buffer at a time. You want to see several things at once
on one screen: two files side by side, the Claude chat next to the code it
is changing, Magit beside the file you are editing. Windows work the way
Neovim's do, because the roadmap's target is Neovim parity and your
fingers already know Vim.

## Goals

- Neovim windows: split, move between, close, resize, each with its own
  cursor and scroll, any number of windows on the same buffer, edits in
  one showing live in the others.
- Special buffers (Claude `Space c`, Magit `Space m`, dired, occur, Org
  special buffers) open in a window below the one you are in, the way
  Emacs Magit pops up beside your file, and give the window back when you
  quit them.
- Zap (`Space s`) labels words in every window and jumps across them.
- Claude, driving fred through `fred --mcp`, sees and works in the same
  windows you do.

## Non-goals (tracked as open ledger rows, not dropped)

- Tab pages (`:tab*`, `gt`, `CTRL-W T`).
- Rarer `CTRL-W` commands: `r R x` (rotate, exchange), `P` and `z`
  (preview), `]` `^` `}` `f F gf` variants, `CTRL-W H J K L` (move to
  edge), `:windo`, `:sball`, `:ball`.
- Per-window options beyond cursor and scroll (`:setlocal` stays per
  buffer until track 3 adds window-local options).
- Window-local folds, jumplist and changelist (tracks 12 and 21).

## Anti-goals: what would make this a failure even if it works

- Closing a window loses unsaved text. A buffer leaves a window, never
  memory: closing the last window on a modified buffer keeps it as a
  hidden buffer (Neovim's default `hidden`), and `:q` on the last window
  still refuses as today.
- Claude's typing lands in a different window than the one you watch.
- The view jumps: switching windows or buffers must keep each window's
  scroll, which today resets on every buffer switch.
- One window becomes slower to draw than fred is today. Highlighting work
  per frame stays inside the existing budget, shared across windows.
- Today's single-window behavior changes. Every existing test passes
  unchanged, and with one window the screen is identical, pixel for pixel.

## Behavior

### Commands (Neovim's keys and names)

- Split: `:split [file]` / `:sp`, `:vsplit` / `:vs`, `:new`, `:vnew`,
  `:sbuffer N`, `CTRL-W s`, `CTRL-W S`, `CTRL-W v`, `CTRL-W n`,
  `CTRL-W ^`. A count sets the new window's size (`:5split`, `5 CTRL-W s`).
- Move: `CTRL-W h j k l` (and arrow keys, `CTRL-W CTRL-H` forms),
  `CTRL-W w` / `W` (with count: window N), `CTRL-W t` / `b` (top, bottom),
  `CTRL-W p` (previous).
- Close: `CTRL-W c` / `:close`, `CTRL-W q` / `:quit` (closes the window
  when there is more than one; on the last window it is today's `:q`),
  `CTRL-W o` / `:only`, `:hide`, and `:qall` / `:wqall` / `:xall` where
  fred lacks them. `:q` on a modified buffer still shown in another window closes
  the window without asking, as Neovim does.
- Resize: `CTRL-W + - < > = _ |` with counts, `:resize [+-]N`,
  `:vertical resize [+-]N`, `:vertical {cmd}` modifier for `:split` and
  `:new`, `:wincmd {c}`.
- Options (in the options table, settable with `:set` and config.toml):
  `splitbelow`, `splitright`, `equalalways`, `winminheight`,
  `winminwidth`. Defaults as Neovim (all off except `equalalways`, minimum
  1 line and 1 column).
- Error messages and numbers match Neovim (`E444: Cannot close last
  window`, `E36: Not enough room`).

### Screen

- Every window has its own status line; the active window's is bright,
  the others dim (Neovim's `StatusLine` / `StatusLineNC`). With a single
  window it looks exactly like today.
- Side-by-side windows are separated by a one-column `│` divider.
- One command line at the bottom of the screen, shared by all windows.
- Overlays that belong to a window (completion popup, explain box, zap
  labels, right-click menu) draw inside that window. Overlays that belong
  to fred (the picker panel, Magit transient menus, the Claude approval
  dialog) cover the whole screen: the picker and menus stay at the bottom,
  the approval dialog in the screen's middle.
- Inline mode (`--height`) works the same inside its smaller area; it does
  not switch to fullscreen. With more than one window it grows straight
  to its full `--height` (my call), so windows don't squeeze or jump.

### Mouse

- Clicking in a window makes it active and places the cursor. Wheel
  scrolling scrolls the window under the pointer, active or not.
- Dragging a status line or a `│` divider resizes the windows on both
  sides of it.
- Selection by drag stays inside the window where it started.

### Special buffers open below

When something opens a special buffer (Claude chat, Magit status and its
other buffers, dired, occur, Org special buffers):

1. If a window already shows that buffer, go to it.
2. Else, if there is a window below or beside the current one that was
   itself opened this way, reuse it.
3. Else split the current window: the new window goes below, taking a
   third of the height (at least `winminheight`, never less than 3 rows).
   If the current window is too short to split, use the current window,
   as today.

Quitting the special buffer (Magit `q`, Org `C-c C-c`/`C-c C-k`, and the
others' existing "return" actions) gives the window back the way Emacs's
`quit-window` does: a window that was made for it closes; a window that was
reused shows its previous buffer again. Where those actions return by
buffer number today, they now mean "show this buffer in this window".
Ordinary buffer commands (`:e`, `:b`, pickers) stay in the current window.

Opening a file from a special buffer (my call, following Emacs): Enter in
occur, Magit (a file, hunk or commit's file) and Org agenda shows the file
in the window the special buffer was opened from, and that window becomes
active; the special window stays open. Dired opens files in its own
window, as Emacs's dired does.

A special buffer opened from inside a special window (Magit status, then
log, then a commit) replaces it in place, as Magit does. `q` steps back
one buffer at a time; quitting the first one closes the window if it was
made for it. Editing a file in a special window with `:e` or `:b` makes it
an ordinary window, so rule 2 never hides your file.

### Zap across windows

`Space s` labels the visible words in every window at once. The active
window gets the shortest labels. Typing a label makes its window active
and puts the cursor on the word. Escape cancels with nothing changed.

### Claude driving fred

`fred_keys` types into the active window, exactly as your keys would,
with one exception you chose: when the active window shows the Claude
chat, Claude's keys go to the window the chat was opened from (the code
window above it). That window becomes active while Claude types, so you
watch the edits happen, and when Claude's turn ends fred puts you back in
the chat at its prompt, as today.
`fred_state` adds the windows: for each, its number, the buffer it shows,
cursor line and column, the lines on screen, and which one is active.
`fred_read` with a path must not reload or discard a buffer that has
unsaved changes (it does today: found while writing this design).

## How it gets made (fred's existing stack; these are my calls)

- Rust, ratatui, the existing modules. No new dependencies.
- **Window state leaves `Editor`.** A `Window` holds what is per window:
  the buffer it shows, cursor, `View` (top, top_row, left, detached), the
  zap viewport, `recenter`, its height and width, the last buffer shown
  (for `#` and quit-window) and whether it was opened for a special
  buffer. `Editor` keeps what is per buffer (text, undo, marks, folds,
  options, Magit/Org/Claude/dired state). Mode, registers, the command
  line, the message and pickers stay global, as in Neovim.
- **The live/parked swap stays for now.** The active window's buffer is
  still the one live `Session::ed`, so the hundreds of `self.ed` call
  sites keep working. Switching windows saves the window's cursor and
  view, then swaps buffers through the existing `show`. Other windows
  draw from the parked editor of their buffer (or from `ed` when they
  show the same buffer as the active one) with their own cursor and view.
  Removing the swap entirely is later work in track 2, not this design.
- **Layout is a tree** like Neovim's frames: a leaf is a window; a node
  splits its area in a row or a column among its children, with sizes.
  `src/window.rs` owns windows, the tree, layout from a `Rect`, focus
  movement by geometry, resizing and `equalalways`.
- **Drawing per window.** `ui::draw` takes the window's `Rect` and draws
  text, gutter and status line there; a new top-level function lays out
  the tree, draws each window, the dividers, the shared command row and
  the full-screen overlays. Syntax highlighting is cached per buffer, not
  per screen, and the frame budget is shared.
- **Mouse** hit-tests the window first, then hands the click to today's
  code with that window's area.
- Ex commands go in the `COMMANDS` table, keys in Normal mode's
  `CTRL-W` handling, options in `src/options/`.

## Stages (each one committed, tests passing)

1. `Window` struct and per-window cursor and scroll, one window. No
   visible change except that each buffer's scroll survives switching.
2. The layout tree, drawing per window, `:split`, `:vsplit`, `:new`,
   `:vnew`, `:close`, `:only`, `:q` closing windows, `CTRL-W` moves.
3. Resizing: keys, `:resize`, options, `equalalways`, mouse dragging.
4. Special buffers open below; quit-window.
5. Zap across windows.
6. `fred_state` windows; the `fred_read` reload fix.
7. Oracle cases, ledger rows to `done`, and the non-goals recorded as
   open rows. No roadmap points (my call): tab pages are out of scope and
   the live/parked swap stays, so neither track passes its gates yet and
   both stay at 0; the execution queue and log record the progress.

## Testing

- Session tests (`T::open`) for each command, count and error.
- UI tests (`TestBackend`) for the screen: two and three windows, mixed
  splits, dividers, active and inactive status lines, overlays, and one
  window drawn exactly as before.
- The Neovim oracle grows a window mode: the Lua side reports
  `winlayout()`, each window's buffer, cursor, height, width and topline;
  the Rust side runs a `Session`. Cases cover every command above with
  counts, edges (too small to split, last window) and errors.

## Left to the builder

The exact `Window` field list, how dividers meet at corners, the label
order in multi-window zap beyond "active window first", and how the
highlight budget is split between windows.
