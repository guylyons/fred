# Org-mode port: scope and continuation notes

## User's requirement

"Just like magit, port org-mode (/Users/guy/github/org-mode) to fred. Document
all functions in a csv and complete a full working port of org-mode; we will
be able to handle org-mode in its entirety. Work until completion."

This is a port of upstream Org as a Rust component of Fred, not an
Org-inspired outliner. Every command, option, key, view and workflow in the
source is in scope. Emacs-only plumbing is adapted to a Fred equivalent, and
the adaptation is recorded; nothing is silently dropped.

## Source baseline

Upstream: https://git.savannah.gnu.org/git/emacs/org-mode.git (local clone
`/Users/guy/github/org-mode`, origin github.com/bzg/org-mode).
Pinned commit: 3b73b8a0cecf6435c265ce0f680dc3c135294c39 (2026-10-03).
Temporary checkout with generated autoloads: `/private/tmp/fred-org-upstream`;
recreate with `git clone` + `git checkout <pin>` + `make autoloads` if missing.
The user's checkout is never modified.

## Inventory

`tools/org-source-inventory.el` loads every library in the pinned checkout in
`emacs --batch -Q` and writes:

- [docs/org-parity.csv](org-parity.csv): every function, macro and user option
  defined by an upstream file (5715 rows: 906 commands, 3610 functions,
  93 macros, 1106 options), with definition line and first doc line.
- [docs/org-keys.csv](org-keys.csv): 597 effective bindings of org-mode-map,
  org-agenda-mode-map, org-capture-mode-map, org-src-mode-map, org-columns-map.

Ledger `state` values:
- `missing`: no Fred behavior.
- `partial`: some behavior; named gaps in `fred_behavior`.
- `done`: behavior matches upstream for the cases the source handles.
- `internal`: a non-interactive helper whose role is carried by the named Rust
  function (recorded in `fred_behavior`); credited only with its callers.
- `adapted`: Emacs-specific plumbing (faces, overlays, compat shims, mouse
  menus, buffer-local machinery) whose user-visible effect Fred provides
  another way; `fred_behavior` says how.

## Architecture in Fred

- `src/org/`: the component. Parser (`element.rs`) works on buffer lines;
  commands mutate the buffer through Fred's undoable edits.
- Org buffers: files ending `.org` (or `-*- mode: org -*-`) get `ed.org`
  state: fold set, cached in-buffer settings (`#+TODO`, `#+TAGS`,
  `#+STARTUP`, ...).
- Folding is a Fred core feature added for Org: hidden line ranges skipped by
  rendering and vertical motion, with `...` after the visible header.
- Generated views (agenda, column summary, clock report, etc.) are read-only
  buffers like Magit views.
- Options: the Emacs variable names are read from `[org]` in
  `~/.config/fred/config.toml`, e.g. `org-todo-keywords = [...]`. Defaults are
  upstream defaults.
- `:org <command-name>` runs any ported interactive command by its Emacs name
  (M-x equivalent). Keys follow org-mode-map with Emacs chords (C-c C-t,
  M-left, S-right, ...), plus the user's `Space o` leader layout.

## Continuation

Follow [the roadmap](org-roadmap.md) and the [binding contract](org-user-bindings.md).
Update the ledger rows and roadmap queue in each implementation commit.
Preserve the user's uncommitted `notes.md`; never `git add -A`.
