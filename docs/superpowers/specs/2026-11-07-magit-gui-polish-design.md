# Magit GUI polish — design

Date: 2026-11-07 · Branch: `gui-vscode-ui`

## Intent

In Fred's macOS app the rest of the UI now looks like VS Code, but Magit
still looks like a terminal: plain colored text in a grid. The user wants
"just polished Magit": the same Magit, drawn like it belongs in the app.

It must stay recognizably Magit. Same sections, same order, same text,
same keys, same workflow. Only the drawing changes.

## Goals

1. **Every Magit buffer built from Magit rows** gets the treatment:
   status, log, diff, commit (revision), stash, refs, reflog, cherry, and
   the rest that use the same rows. Blame, the rebase to-do list and the
   commit-message editor are built differently and are **out of scope for
   now** (user: "make it", taking the regular-screens-first suggestion).
2. **Status-style polish (mockup "A", chosen by the user):** everything
   stays in the editor's monospace font and on the cell grid, dressed up:
   - **Section headings** ("Untracked files (1)", "Unstaged changes (2)",
     "Recent commits", …) become full-width bands: a faint lighter fill, a
     hairline top border, a ▸/▾ fold chevron, and the count drawn as a
     small rounded pill instead of "(2)".
   - **Ref names** are drawn as rounded chips (local branch blue-tinted,
     remote neutral, tags their own tint): on the Head / Merge / Push /
     Upstream lines *and* wherever a commit row shows refs (log, "Recent
     commits", refs buffer). *(My call: Magit itself colors refs on commit
     rows, so chips belong there too.)*
   - **File status words** ("modified", "new file", "deleted", "renamed",
     "untracked" rows): drawn as a small rounded colored badge
     (modified yellow, new/added green, deleted red, renamed blue).
   - **Diff hunks:** each hunk (its `@@` header through its last line) sits
     in a rounded box with a hairline border. Hunk rows start at column 0,
     so the box is a purely visual pixel inset: its left edge sits in the
     GUI's existing one-column left pad (below), not in new cells. No
     columns move. *(My call.)*
     The `@@` header row has a faint blue tint; added and removed lines get
     soft green / red full-row fills (not just colored text, and replacing
     today's saturated indexed-22/52 backgrounds in the GUI).
   - Hover on a heading or file row shows the subtle hover fill the rest
     of the app uses.
3. **Transient menus (mockup "1", chosen by the user):** stay a panel at
   the bottom of the window, where they are today and where Magit puts
   them, but drawn polished:
   - the panel has its own slightly darker background and a hairline top
     border, and slides up (short animation) when it opens;
   - each key is a rounded key chip;
   - group headings ("Arguments", "Create", "Edit HEAD") in the heading
     color, semibold;
   - an option that is on (e.g. `(--all)`) is highlighted; off options dim;
   - the layout (Arguments full width first, action groups in columns,
     wrapping) stays exactly as the core lays it out today.
4. **Click a chevron to fold**, same as Tab. Chevrons appear on every
   foldable row: section headings and file rows (which expand their inline
   diff in Magit). Clicking elsewhere on the row behaves as today (moves
   the cursor, drag selects, double-click selects a word). *(My call: this
   keeps selection working and matches Magit's own click-the-indicator
   behavior.)*

## Non-goals

- No change to Magit's content, ordering, wording, keys, or behavior.
- No change to the terminal (TUI) rendering. The terminal look stays.
- No proportional (UI) font inside Magit buffers. That was mockup "B",
  rejected.
- No floating transient card (mockup "2", rejected).
- No new Git features.

## Anti-goals (would be a failure even if it "works")

- Anything that makes it stop feeling like Magit (re-ordered sections,
  replaced text, VS Code Source Control layout).
- Cursor/selection or j/k movement drifting from what's drawn. Every
  decoration must sit on the existing cell grid; the core's rows and
  columns stay authoritative.
- The GUI and TUI disagreeing about what is on a row.

## Current state (from recon)

- Magit rows are `Row { text, action: Option<RowAction> }`
  (`src/magit/mod.rs:380`); `RowAction` is
  `Section | File | Hunk | Stash | Commit | Module | Repo`. There is no
  face enum.
- Colors are guessed from text at draw time in `magit_styles()`
  (`src/ui/render.rs:540`): `+` green, `-` red, `@@` cyan, `Head:` and
  section rows yellow, hunk added/removed bg Indexed(22)/(52), whitespace
  errors red bg, refined words reversed, margin styling ~`render.rs:873`.
- The transient panel is drawn at `render.rs:1337-1344` from
  `transient_lines()` (`render.rs:1777`) and reaches the app as plain
  `Frame.bottom` rows.
- The GUI protocol sends rows as `Run { x, s, fg, bg, f }` only
  (`src/gui/proto.rs:11`, built by `runs()` in `src/gui/frame.rs`). No
  semantic tags reach Swift.
- Swift draws rows in `EditorView.draw` → `drawBackgrounds` / `drawText`
  (`macos/Sources/Fred/EditorView.swift`).
- Tab folds by section (`src/session/magit.rs:3428-3485`). A mouse click
  on a leading `v ` / `> ` indicator already sends Tab
  (`render.rs:376-386`); a click elsewhere on a heading does not.

## Decided approach

The **core decides what each thing is; the app only draws it.** This was
the user-confirmed principle ("the core still decides what's on screen and
the app only draws it nicer").

- The core attaches **semantic tags** to what it already renders, in GUI
  mode only, and sends them in the frame alongside the existing runs:
  - a **row role** per text row (e.g. `heading`, `file`, `hunk-header`,
    `added`, `removed`, `context`, `ref-line`), plus for the hunk box which
    rows begin/end a hunk;
  - **span tags** for in-row pieces drawn specially: section count, ref
    name chip (local/remote), file status badge (with its kind), transient
    key, transient group heading, transient option on/off.
- The Swift app draws decorations from those tags on the cell grid:
  bands, chips, badges, boxes, fills, key chips. Untagged rows draw exactly
  as today, so non-Magit buffers are unaffected.
- The tags are derived from the same logic that colors rows today
  (`magit_styles`, `transient_lines`, `RowAction`), so the TUI and GUI
  can't disagree.
- Fold-on-click: clicking anywhere on a heading row (or its chevron)
  folds it, via the existing Tab path.

## Calls I made (not discussed with the user — flag if wrong)

- **Status words stay as words.** The mockup showed a one-letter "M"
  badge; to keep Magit's text and the cell grid intact, the badge is the
  existing word ("modified") drawn as a pill, not a replacement letter.
- **Count pill:** "(2)" is drawn as a pill showing "2" over the same
  cells; the parentheses are not drawn.
- **Chevron:** drawn in the one-column left pad the GUI already gives
  gutterless buffers, Magit included (`gui_pad` in `src/ui/render.rs`,
  commit 7342bae). Fred's headings start at column 0 with no `v `/`> `
  indicator, and that pad column is free. The trailing `…` on a collapsed
  heading stays as text (no text is hidden).
- Colors come from the existing palette/theme (`Palette` in Swift) with
  low-alpha tints, matching the app's other chrome; light themes get the
  equivalent.

## Left to the builder

- How tags are captured during rendering and carried into the frame (e.g.
  a side table filled by the Magit render path in GUI mode, keyed by
  window and row, read by `frame.rs`), and their exact wire format, as long
  as non-Magit frames are unchanged and old/new app/core pairs degrade to
  today's look.
- Exact sizes, radii, alphas, and the panel slide animation timing.
- Which Magit buffer kinds need extra tag cases beyond status/diff/log
  (refs, blame, etc.), discovered while implementing.
- Tests: core-side tests that the right rows/spans get the right tags for
  status, a diff, a log and a transient; Swift build. Visual check in the
  running app for status, a diff, log and a transient.
