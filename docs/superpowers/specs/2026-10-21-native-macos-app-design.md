# Native macOS Fred — design

## Intent

Fred is a Rust terminal editor: Vim editing, Space leader menus, Magit,
the Claude chat, a Ghostty terminal, splits, `:` commands. The user wants
a native Mac app of it: "Fred + Sublime". It should be a real Mac app,
not Fred's terminal screen in a window.

An earlier attempt (`origin/guylyons/macos-gui`, commit ca21ca7) had a
Swift window paint Fred's terminal cell grid (`src/gui.rs` renders
through ratatui's `TestBackend` and hands Swift a `FredCell` array). The
user called that "a rectangle around Fred". The point of this project is
the opposite: the Mac draws the text and builds the UI, and Fred's core
supplies the editing brain and the data.

## Goals

- **Native Mac app, built in Swift.** It uses AppKit/SwiftUI windowing,
  macOS text rendering, system fonts and the Liquid Glass look (glass
  sidebar, glass modals and panels).
- **Sublime-style layout** (the user picked mockup A,
  `.superpowers/brainstorm/*/content/layout.html`):
  - A glass folder sidebar on the left: a tree with disclosure arrows,
    plus a right-click menu with new file, new folder, rename, delete,
    Reveal in Finder and Copy Path.
  - Tabs above the editor.
  - A minimap on the right edge of each editor. Click or drag it to
    scroll.
  - A status bar with the mode badge (NORMAL, INSERT, ...), file, git
    branch and line:col.
  - The title bar shows the project folder.
- **Every Fred feature still works:** Vim modes, `:` commands, Space and
  `Ctrl-w` keys, splits (`Space w c N`), Magit, the Claude chat, the
  Ghostty terminal, which-key, pickers, and anything else Fred does
  today.
- **More menus than the terminal has:**
  - A full menu bar (Fred, File, Edit, Selection, Find, View, Go, Git,
    Window, Help).
  - Right-click menus on text, tabs, the sidebar and Git items.
  - Each menu item runs an existing Fred command and shows its Fred key
    as the shortcut hint, so menus and keys never drift apart.
- **Two glass modals, the same look:**
  - **`Space p`** finds a file. Today this is `pick::Kind::Files`.
  - **`Cmd-P`** is the command palette: every ex command, Space menu
    entry and menu item, with its key binding shown. It works like
    Sublime's Cmd-Shift-P.
- **A good Settings window** (`Cmd-,`). It is a native macOS Settings
  scene with tabs, and it reads and writes the same
  `~/.config/fred/config.toml` the terminal Fred uses (`src/config.rs`,
  including the `[set]`, `[magit]` and `[org]` tables). It also decides
  where Claude, the terminal and Magit open; the default is a Fred pane
  in the editor area, as today.

- **Mac keys work everywhere** (user's call):
  - Cmd-S, Cmd-C, Cmd-V, Cmd-X, Cmd-Z, Cmd-Shift-Z, Cmd-A, Cmd-F and
    Cmd-W behave the Mac way in every mode. Cmd-C copies a Visual
    selection.
  - The menu bar carries real Cmd shortcuts. Where an item also has a
    Fred key, it shows that key too.
  - Fred's `Key` type (`src/key.rs`) gains a Cmd modifier.
- **Sublime-style editing comes along** (user: "it's like Sublime"):
  - Mouse drag-select, double- and triple-click select, and Option-drag
    column select.
  - Multiple cursors with Cmd-D and Cmd-click.
- **Multiple projects, windows belong to projects** (user's call):
  - Each project (a folder) gets its own Mac window or windows, and
    several projects can be open at once.
  - A project opens through any of these: File › Open Folder, dropping a
    folder on the Dock icon or a window, `fred .`-style launching from
    the terminal into the app, and Open Recent.
  - Launching from the Dock with nothing open shows a welcome window with
    recent projects.

## Anti-goals (would make it a failure even if it works)

- **Blurry or soft text.** Text must be crisp at every size, on Retina
  and on non-Retina displays.
- **Latency.** Typing, cursor movement, scrolling and opening `Space p`
  or `Cmd-P` must feel instant.
- **Looking like a terminal in a box.** Every piece of UI must be a
  native control: no character-cell chrome, no box-drawing borders, no
  ratatui popups.

## Non-goals (for now)

- Windows or Linux GUIs.
- Replacing the terminal Fred. It keeps working and shares the core and
  the config.

## Constraints

- macOS 26+ (Liquid Glass needs the macOS 26 SDK). The user runs a newer
  macOS.
- Swift for the app; Fred's Rust core underneath. One repo: the app lives
  in `macos/` next to the crate, as on the old branch.
- The Ghostty terminal pane uses `libghostty-vt` behind the `ghostty`
  cargo feature. Building it needs Zig 0.15
  (`/opt/homebrew/opt/zig@0.15/bin` on PATH). The app build turns the
  feature on.
- Repo rules in AGENTS.md still apply: preserve the user's uncommitted
  `notes.md`, keep the Neovim and Magit roadmaps current when a commit
  touches those ports, and leave untracked files the user owns alone.

## How it fits together (decided)

- **Fred's core becomes a library the app drives** through a C interface
  (as on the old branch: `fred_new`, `fred_key`, `fred_mouse`,
  `fred_paste`, `fred_tick`, ...). Keys go in as Fred's own `Key` type
  (`src/key.rs`). Mouse events go in as logical positions (window, line,
  column), not screen cells.
- **The core hands the app a model, not pixels or cells.** What the old
  `fred_render` returned as a cell grid becomes structured data the app
  draws natively:
  - The window layout: the split tree, sizes in rows and columns, and
    which window is active.
  - For each window: its buffer name and kind (file, Magit, Claude,
    terminal, ...), the visible lines' text, highlight spans with
    colors, cursor, selections, scroll position, line numbers, folds and
    signs.
  - The status data, the `:` line, messages, the which-key list, picker
    items, the right-click menu and popups (explain, completion,
    approvals).
  - The tab list and the command list (for `Cmd-P` and the menus).
- **The core still thinks in rows and columns.** The editor font is
  monospaced, so Swift converts rows and columns to points using the font
  metrics. Split sizing, `Space w c N` and scrolling keep their logic.
- **The editor surface is a custom layer-backed `NSView`**, not SwiftUI
  and not `NSTextView`. It draws each visible line with Core Text, caches
  the shaped line until that line's text or highlights change, and
  redraws only the lines that changed. That is how text stays sharp and
  typing stays fast. Metal is the upgrade path only if measured latency
  needs it.
- **Chrome is SwiftUI/AppKit:** sidebar, tabs, status bar, modals,
  which-key, menus and Settings, with glass materials.
- **Buffers that are already text use the same editor view:** Magit
  status, log and diff, the Claude chat, help, dired. The terminal pane
  draws `libghostty-vt`'s render-state cells with Core Text in the same
  kind of view.

## My calls (not stated by the user; open to change)

- **`Space p` ranking:**
  - Ranks by fuzzy path match, then recently opened or currently open
    files, then files near the current one.
  - Gitignored files are skipped.
  - The file list is kept warm in the background, so results appear on
    the first keystroke.
- **Tabs and splits:** each split gets its own tab strip, like Sublime's
  groups.
- **Panes:** Claude, the terminal and Magit open as Fred panes in the
  editor area by default. Settings can move each of them to a right
  panel or a bottom panel instead. The layout model must have room for
  those side regions from phase 1, rather than being redone later.
- **What a tab is:**
  - Each window keeps a list of the buffers it has shown, like Sublime
    groups. That list is new core state.
  - A single click in the sidebar opens a preview tab, shown in italics,
    which the next preview replaces. Editing it or double-clicking makes
    it permanent.
  - Closing a tab removes it from that window only. `:bd` still deletes
    the buffer everywhere.
  - `:bn`, `:bp` and Ctrl-^ move through the window's tabs. `:ls` and
    `Space b` still list every buffer.
- **Shared core, not app-only:**
  - Tabs, the command list behind `Cmd-P`, and the better `Space p`
    ranking all live in the core.
  - The terminal Fred gets the ranking straight away. It can show tabs
    later.
  - Commits that touch the Neovim or Magit ports still update their
    ledgers.
- **What "instant" means:**
  - Keypress to pixels under 16 ms at the 99th percentile, aiming for
    one frame at 120 Hz (8 ms).
  - `Space p` and `Cmd-P` show results within one frame of opening and
    of each keystroke, on a repo with 50k files.
  - Phase 1 measures both. Missing them is the trigger to move to Metal.

- **One Fred core per project, in its own process.**
  - Fred's core assumes one project per process: `:cd` changes the
    process cwd, `remote::SOCKET` is a global, and `shell.rs`,
    `clipboard.rs` and `org/` keep thread-local state.
  - So the Swift app starts a helper Fred process per project and talks
    to it over a pipe (keys and mouse in, the model out), rather than
    linking the core in and making all of that per-instance.
  - This also keeps a crash in one project from taking down the others.
  - Each project's Claude socket stays per process, so "the newest Fred"
    lookup in `src/remote.rs` must pick the right project's process. The
    builder decides how.
  - The pipe must fit inside the latency budget; phase 1 measures it.
  - The C interface described above becomes this message protocol. Its
    contents stay the same.
- **Multiple cursors are the last phase.** They're a large new core
  feature, so they come after everything else works.

## Phases

Each phase ends with something the user can run.

1. **The core in a helper process plus one native editor.** Includes Mac
   keys and mouse selection. One buffer drawn with
   Core Text. Measure keypress-to-screen latency from this phase on.
2. Tabs, splits, the status bar and the minimap.
3. The sidebar and its right-click menu.
4. The `Space p` modal and the `Cmd-P` palette.
5. The menu bar and the right-click menus on text and tabs.
6. Native `:` line, which-key, messages and popups.
7. Claude, terminal and Magit panes in the app.
8. The Settings window.
9. Several projects at once, Open Recent and the welcome window.
10. Multiple cursors (Cmd-D, Cmd-click).

## Success

The user drops the terminal Fred for the app as a daily driver:
- Text is never blurry.
- Typing never lags.
- Everything they do in Fred today still works.

## Left to the builder

- **Swift package layout and build:**
  - SwiftPM plus a bundling script (like the old `macos/build.sh`) or an
    Xcode project.
  - How the Rust staticlib is built and linked (cbindgen or a
    hand-written header).
- **The exact C interface and its data layout.** Old branch code worth
  reusing: the `gui.rs` session and job plumbing and the `app.rs` input
  refactor. Do not reuse its cell-grid rendering.
- **How much of `src/ui` gets split** into "compute the model" versus
  "draw with ratatui", so the terminal Fred and the app share one model.
- **Theme mapping** from Fred's themes to app colors.
- **Exact menu contents and Settings tab layout**, within the goals
  above.
