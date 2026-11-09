# Fred as one editor: native macOS and Emacs + Neovim coherence

Target: a Fred.app that someone who has never used Vim or Emacs can download,
open a file in from Finder, edit with the usual Mac keys, and save, without
reading the README. The same app should also feel like one editor rather
than a Vim core with Emacs features attached, for the Vim and Emacs users
it was built for.

This roadmap covers the work that spans several areas. The parity roadmaps
are still where the per-feature depth lives:
[nvim-core](nvim-core-roadmap.md), [magit](magit-roadmap.md) and
[org](org-roadmap.md). Where a track here is the same work as a parity
track, both roadmaps credit it, and the shared track is noted.

The audit behind this document was done on 2026-11-08 at `bc0ab89`. It read
the code only; nothing was run. File:line references point at that commit.

## Percentage and accounting

**Verified roadmap completion: 0 / 100 points = 0%. Remaining: 100%.**

A track's points are awarded together only when all five gates pass:

1. Every item in the track's checklist below is done, or the checklist says
   why it differs.
2. It works in both input profiles (see A2): Standard and Vim. If a profile
   doesn't apply, the checklist says so.
3. The core behavior has Rust tests. The GUI protocol cases go in
   `src/gui/` tests, and the Swift-side steps get a manual acceptance list in
   `docs/mac-acceptance.md`.
4. A user-facing change is documented in the README, and in the Help menu
   once B7 exists.
5. Review, with material findings fixed.

A partial track scores zero. Update this table, the execution queue and the
log in every implementation commit.

### Pillar A: native macOS (50 points)

| Order | Track | Points | Accepted | Today |
| --- | --- | ---: | ---: | --- |
| A1 | Key input pipeline | 8 | 0 | Cmd chords without a menu item are dropped (`macos/Sources/FredKit/KeyNotation.swift:4`); Option/Ctrl/Shift are lost on arrows, Home/End and PgUp/PgDn (`:5-12`); Option-letter becomes `<M-x>`; no `NSTextInputClient` (`macos/Sources/Fred/EditorView.swift:198`) |
| A2 | Standard (non-modal) input profile and first run | 8 | 0 | always starts in Vim Normal; the welcome pane is only a folder picker; the README's first five minutes teach Vim |
| A3 | Clipboard, selection and mouse | 5 | 0 | Cmd-A enters Visual, where Cmd-V silently does nothing (`src/editor.rs:671`); Cmd-C does nothing in a terminal or in Insert; mouse modifiers dropped (`src/gui/mod.rs:561-575`); no text drag |
| A4 | Document lifecycle | 7 | 0 | Cmd-S on an untitled buffer errors with `:w name`; quit, close and Cmd-W never show Save / Don't Save / Cancel (`src/gui/mod.rs:653`, `src/session/tabs.rs:41`); no `CFBundleDocumentTypes` or `application(_:open:)`, so Finder, the Dock and `open -a` can't open files; swap recovery opens read-only |
| A5 | Find, Go and the command palette | 5 | 0 | Cmd-F types `/`, Replace types `:%s/`; no Cmd-G / Cmd-Shift-G / Cmd-E, Cmd-P or Cmd-Shift-P; no Go menu |
| A6 | Menu bar completeness and Help menu | 3 | 0 | Settings… disabled; no Help menu, full screen, Hide Others / Show All / Services, recent files or Find Next |
| A7 | Settings, fonts and appearance | 5 | 0 | settings are split between `~/.config/fred/config.toml` and hidden UserDefaults; font fixed (`EditorView.swift:8`); dark forced; zoom not saved |
| A8 | Windows and state restoration | 3 | 0 | New Window starts another process; no session restore; window re-centred on each launch |
| A9 | Accessibility and spelling | 2 | 0 | the editor view has no accessibility role; no `NSSpellChecker` |
| A10 | Packaging and distribution | 4 | 0 | only `macos/build.sh`; Zig path hard-coded; no version keys, signing, notarization, DMG, cask or release CI; macOS 26 only |
| | Pillar total | 50 | 0 | |

### Pillar B: Emacs + Neovim coherence (50 points)

| Order | Track | Points | Accepted | Today |
| --- | --- | ---: | ---: | --- |
| B1 | One command registry, plus M-x / palette | 8 | 0 | four registries: `ex::cmd::COMMANDS` (`src/ex/cmd.rs:530`), `magit::commands::by_name`, Org `MODULES` (`src/org/mod.rs:1010`), Claude `Action` (`src/claude.rs:85`); `:help` / `SPC h` only see ex commands |
| B2 | Leader tree as data, which-key everywhere | 5 | 0 | the handler (`src/vim/mod.rs:1570-1640`) and the which-key list (`:554-576`) are kept in sync by hand; `SPC c`, `SPC o` and `SPC m` each check `pending` themselves; `SPC m` has no which-key |
| B3 | One transient / menu engine | 6 | 0 | three engines: which-key (`src/ui/windows.rs:131`), `Kind::MagitMenu` (`src/pick/mod.rs:1186`), `Kind::OrgMenu` (`:1133`) |
| B4 | Editor-wide prefix argument | 2 | 0 | `SPC u` only sets `ed.org_arg`; Magit says "Fred has no prefix argument" (`src/magit/commands.rs:64`) |
| B5 | Special buffers: one major-mode trait and shared conventions | 7 | 0 | about 40 `Option<Box<…>>` feature fields on `Editor` (`src/editor.rs:159-248`); separate read-only lists in dired and occur; `q`, `gr`, `g?` and buffer names differ between Magit, dired, occur, agenda and chat |
| B6 | Keymaps as data and user rebinding | 5 | 0 | Vim `parse()`, Magit `match` arms (`src/magit/mod.rs:1005-1830`) and Org tables (`src/org/keymap.rs`); mappings don't persist |
| B7 | Help: describe key, mode, command and option | 4 | 0 | `:help` lists ex commands only |
| B8 | One option namespace and an init file | 5 | 0 | four stores: `src/options/`, the older top-level config keys (`src/config.rs:14-31`), `[org]` thread-local, `[magit]` RwLock; `:set` can't reach `org.*` or `magit.*`; no startup rc file |
| B9 | Ex entry and naming for every feature | 2 | 0 | `:Magit NAME` only; no `:Org`, `:Dired`, `:Claude`; `:org-set-option` mentioned (`src/org/options.rs:17`) but missing; mixed Emacs and Vim names and error styles |
| B10 | Which keys belong to Vim in special buffers | 3 | 0 | Magit takes counts `1`-`4` for fold levels (`src/magit/mod.rs:1290`) and swallows `[`/`]`; `C-c` is a prefix in Magit and Org but cancel elsewhere |
| B11 | Buffer / window separation (shared with nvim track 2) | 3 | 0 | `Editor` is both buffer and window; special buffers can't use `:split` or `quit-window` the Magit way |
| | Pillar total | 50 | 0 | |
| | **Total** | **100** | **0** | |

## Key decisions

These shape most tracks. Revisit them here, not in the code.

1. **Two input profiles, chosen at first run.** `keymodel`-style option
   `profile = standard | vim` (GUI default `standard` for a new user, `vim`
   when a config already exists).
   - **Standard:** typing inserts text, and there is no Normal mode to fall
     into: Esc closes popups and clears the selection. This follows Vim's
     old `'insertmode'` (removed from Neovim), built on the Select mode and
     `keymodel` that already exist (`src/vim/select.rs`). Cmd and Option
     keys do the usual Mac things. Every feature is reached through the
     menu bar, Cmd-Shift-P and Cmd-K (a leader that works in Insert).
     Shift-selection and mouse selections are ordinary selections.
   - **Vim:** what exists today. Cmd keys still do the Mac thing and stay
     out of the way of Vim keys.
   - A Settings switch and `:set profile=` change it live.
2. **Cmd keys are Mac keys in every mode and both profiles.** Cmd-C/X/V use
   the system clipboard and act on the current selection, whatever mode
   produced it (Visual, Select, picker or terminal). Vim's registers stay as
   they are; `clipboard` decides only whether `y`/`p` also use the system
   clipboard.
3. **Every feature is a registry command first** (B1). Keys, leader
   entries, menu-bar items, palette rows and ex commands all point at
   registry names. That one change gives the palette, the Help menu,
   describe-key and rebinding.
4. **Emacs names are the IDs, plain words are the labels.**
   `magit-log-buffer-file` is the ID and "Log this file" is the label shown
   in menus. The ID is shown dimmed in the palette and in help.
5. **The core owns behavior, Swift draws it.** Native sheets (save
   prompts, save panel, swap recovery) are requested by the core over the
   protocol and answered back, so the TUI keeps its own prompts.

## Track checklists

### A1 Key input pipeline (8)
- `KeyNotation.swift`: send every Cmd chord to the core as `<D-…>`. Menu
  items still take priority, and the core passes unbound chords on.
- Keep Shift, Option and Ctrl on arrows, Home/End, PgUp/PgDn, Delete and
  forward Delete, and on F1-F12.
- Mac text-navigation defaults, in both profiles while in Insert or Select:
  Option-←/→ by word, Cmd-←/→ line start/end, Cmd-↑/↓ file start/end,
  Option-Delete and Cmd-Delete, plus the Shift variants that extend the
  selection.
- `NSTextInputClient` on the editor view: marked text drawn inline, dead
  keys, Option-composed characters (é ü ß €), CJK IME, Ctrl-Cmd-Space
  emoji, dictation and press-and-hold accents. A `macOptionIsMeta` setting
  keeps today's `<M-x>` for Emacs users.

### A2 Standard profile and first run (8)
- The `profile` option and the Standard behavior from decision 1.
- Cmd-K as a which-key leader in Standard: Cmd-K then `m` gives the same
  Magit layout as `Space m` (the user's binding goal still applies to Vim's
  `Space m`).
- Mode indicator: hidden in Standard, and replaced by a status-bar profile
  switch.
- First run: a choice of "Standard Mac editing" or "Vim keys" and an
  optional "Emacs-style Alt/Meta". A short welcome tab lists the ten keys
  that matter in the chosen profile.
- Sidebar, welcome pane and pickers work with arrows, Return and Esc in
  both profiles, not only j/k (`Sidebar.swift:511-543`).
- README: a "First five minutes" for each profile.

### A3 Clipboard, selection and mouse (5)
- Cmd-A makes a Select-mode selection (`src/gui/mac.rs:63`), so typing
  replaces it and Cmd-V pastes over it.
- Cmd-V in Visual replaces the selection (Vim's `p` semantics); never
  silently nothing.
- Cmd-C copies the terminal's selection; Cmd-C in Insert with no selection
  copies the line, as it does in Normal.
- Pass the mouse modifiers through: Shift-click extends, Option-drag makes a
  block selection, Cmd-click goes to the definition, link or file.
- Drag selected text to move it; Option-drag to copy it.
- Paste of multi-line text in the `:` line and pickers: one line, with a
  message instead of silent truncation.

### A4 Document lifecycle (7)
- Cmd-S and Save As on an untitled buffer open an `NSSavePanel`.
- Quit, the close button, Close Window and Cmd-W on a modified buffer ask
  Save / Don't Save / Cancel. Use a sheet, one per buffer, or a "Review
  changes" list for several.
- `CFBundleDocumentTypes` (text, source and Org), `application(_:open:)`,
  Dock-icon drop and `open -a Fred file`. Opening a file in a running app
  opens a tab instead of a new process.
- Open Recent with files as well as folders, through
  `NSDocumentController.noteNewRecentDocumentURL`.
- Native dialogs for swap recovery (Recover / Open read-only / Delete swap)
  and for "changed on disk while modified" (Reload / Keep mine / Compare).

### A5 Find, Go and palette (5)
- A find bar (Cmd-F) with match count, case, word and regex toggles, and a
  replace row (Cmd-Opt-F) with Replace / Replace All. It drives Vim search,
  so `n`/`N` and the search register stay in step.
- Cmd-G / Cmd-Shift-G / Cmd-E.
- Cmd-P opens the file picker. Cmd-Shift-P opens the command palette over
  the B1 registry, showing key bindings in the current profile.
- Go menu: Go to Line (Ctrl-G), Back / Forward (the jumplist, Ctrl-- /
  Ctrl-Shift--), Go to Symbol (Cmd-Shift-O, the breadcrumb `@` picker),
  Go to Definition.

### A6 Menu bar and Help (3)
- App menu: working Settings…, Hide Others, Show All, Services.
- Edit: Find Next/Previous, Use Selection for Find, Spelling (after A9),
  Emoji & Symbols (after A1).
- View: Enter Full Screen, Command Palette, word wrap, line numbers.
- A Magit menu and an Org menu in the menu bar, generated from B1/B2
  registry entries. They are only enabled where they apply.
- Help menu: the system search field, Fred Help (the README or the
  `:help` buffer), Keyboard Shortcuts, Report an Issue.

### A7 Settings and appearance (5)
- A Settings window (Cmd-,) with General, Editor, Appearance, Keys and Git
  panes. It reads and writes `config.toml`, so there is one source of
  truth. Move the `Fred*` UserDefaults keys into the config file, except
  window frames.
- Font family and size, with zoom saved.
- Light and dark palettes that follow the system appearance (drop the
  forced `.darkAqua`), and a theme picker that sets the window and syntax
  colors together.
- Config errors appear in an alert with the line number, not only in the
  status line.

### A8 Windows and state (3)
- Several windows in one process (Cmd-Shift-N), each with its own folder,
  listed in the Window menu.
- Restore windows, tabs and splits on launch (opt-out in Settings).
- Full screen, and remembered window frames.

### A9 Accessibility and spelling (2)
- `NSAccessibility` text role for the editor view: value, selected range,
  line for index, so VoiceOver can read and follow the cursor.
- Spell checking through `NSSpellChecker` for prose buffers (Markdown,
  Org, commit messages, comments), with `spell` / `spelllang` as the
  options (this also starts the nvim spell row).

### A10 Packaging (4)
- `CFBundleShortVersionString` / `CFBundleVersion` from `Cargo.toml`, a
  copyright line and a category.
- Make the Zig toolchain optional, or locate it instead of using a
  hard-coded Homebrew path.
- Developer ID signing, hardened runtime, notarization and a DMG, built
  by a release workflow on tags.
- A Homebrew cask, and an update check (Sparkle or a GitHub-release check).
- Decide the minimum macOS version deliberately; 26 rules out most users
  today.

### B1 Command registry (8)
- One `Command { id, label, scope, run, args }` table. Ex commands, Magit,
  Org, Claude, dired, occur, draw, terminal and the GUI menu ops register
  into it.
- `:` completion, `SPC :` / `M-x` / Cmd-Shift-P pickers and the menu bar all
  read it.
- Commands report the keys bound to them in the current profile and
  buffer, which the palette and help use.

### B2 Leader tree as data (5)
- One nested table with entries `key → command id | submenu`. It drives
  both dispatch and which-key, replacing the two hand-synced lists and
  the per-feature `pending == [' ']` checks.
- Which-key after every prefix: `SPC`, `SPC m`, `SPC o`, `SPC c`, `C-c`,
  `C-x`, `g`, `z`, `[`, `]`, `C-w`, and Cmd-K in Standard.
- Keep the user's exact `Space m` layout (`docs/magit-user-bindings.md`)
  and `Space o` layout (`docs/org-user-bindings.md`).

### B3 One transient engine (6)
- Merge `Kind::MagitMenu` and `Kind::OrgMenu` into one transient component
  with infix arguments, levels, saved values and history.
- Use the same overlay for which-key, so all menus look and behave alike.
  This is also magit roadmap track 2's engine.

### B4 Prefix argument (2)
- `ed.prefix_arg`, set by `SPC u` (and `C-u` in Standard) and by Vim
  counts. Org, Magit, dired and occur read it; remove the workarounds.

### B5 Special buffers (7)
- A `BufferKind` / major-mode trait with `name()`, `keymap()`,
  `readonly()`, `refresh()`, `wants_alt()` and `help()`. It replaces the
  `Option<Box<…>>` fields and feeds `buf_name`, `generated()` and the key
  dispatch order (`src/editor.rs:409-424`).
- Conventions for every generated buffer: `q` quits the window, `gr`
  refreshes, `g?` shows the mode's keys, `i` starts editing where editing
  makes sense, and `:w` commits the edit, which dired and occur already do.
  One shared list of blocked edit keys.
- Names: `*magit: repo*`, `*dired: dir*`, `*occur: pattern*`,
  `*Org Agenda*`, `*claude*`.

### B6 Keymaps as data (5)
- Key tables for the Vim modes (shared with nvim track 4) and for Magit,
  following `src/org/keymap.rs`.
- `[keys]` in `config.toml`, per profile and per mode, bound to registry
  ids. `:map` can use `<Cmd>…<CR>`.

### B7 Help (4)
- `SPC h k` / `C-h k` / Help ▸ Describe Key: which layer owns the key and
  which command it runs.
- `SPC h m` (describe mode), `SPC h f` (command), `SPC h v` (option).
- A Keyboard Shortcuts sheet for each profile, generated from the tables.

### B8 Options and init file (5)
- `:set` / `:setlocal` reach `org.*` and `magit.*`. Retire the older
  top-level config keys in favor of `[set]`, with a migration warning.
- Source `~/.config/fred/init.vim` (ex commands and mappings) at startup.

### B9 Ex entries and naming (2)
- `:Org NAME`, `:Dired`, `:Claude NAME` beside `:Magit NAME`, completed
  from the registry.
- One error style: Vim's `E…` messages for core errors, and plain
  sentences for feature errors. Remove `:org-set-option` or implement it.

### B10 Vim keys in special buffers (3)
- Counts work in Magit: fold levels move to `z1`-`z4` / `M-1`-`M-4`
  (evil-collection's choice, to be checked). Stop swallowing `[`/`]`.
- `C-c` becomes a declared buffer-local prefix with which-key, and cancel
  everywhere else.
- Record each Magit or dired key that shadows a Vim key, and why, in
  `docs/magit-evil-keys.tsv`.

### B11 Buffer / window separation (3)
- nvim track 2. It lets Magit, dired and help open in splits and close with
  `quit-window`.

## Execution queue

1. **A1 key pipeline.** Every later Mac track depends on keys arriving
   whole.
2. **Quick wins in A3 and A4:** Cmd-A as Select, Cmd-V in Visual, terminal
   Cmd-C, save panel for untitled buffers, save prompts on close and quit,
   and Finder / `open -a` support.
3. **B1 registry**, then **A5 palette and find bar** and **A6 menus** on top
   of it.
4. **B2 leader as data**, then **A2 Standard profile and first run.** The
   profile needs Cmd-K, which needs the data leader.
5. **A7 settings and appearance**, **A10 packaging.** At this point a first
   public preview build is possible.
6. **B5 special buffers**, **B10**, **B4**, **B3 transient engine** (with
   magit track 2).
7. **B6 keymaps**, **B7 help**, **B8 options and init**, **B9**.
8. **A8 windows**, **A9 accessibility and spelling**, **B11** (with nvim
   track 2).

Housekeeping found by the audit: rows of `docs/nvim-core-roadmap.md`'s
table are stale. Track 17 says "none", but splits and `:wincmd` exist, and
track 3 says "14 global settings", but `src/options/` exists. Fix these
the next time that roadmap is touched.

## Log

- 2026-11-08: roadmap created from a code audit at `bc0ab89`. 0 / 100.
- 2026-11-09: `Space ;` in the terminal acts as `:q` again; the GUI keeps close-tab-then-quit. No points change.
- 2026-11-09: the GUI's trackpad scroll stops at line 1; the rubber band stays only past the last line. No points change.
- 2026-11-09: the sidebar tree has no scroll bar; it scrolls by trackpad and wheel. No points change.
- 2026-11-09: Material Dark colours comment delimiters (`//`, `#`, `/* */`) as the comment, not as punctuation. No points change.
