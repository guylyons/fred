# Native macOS Fred, Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `Fred.app`. It is a native Swift window that runs Fred's core
in a helper process and draws one or more editor windows with Core Text.
You can type, use Mac keys, select with the mouse and save. Every
keypress's latency is measured.

**Architecture:**
- **Core:** the `fred` binary gets a `--gui` mode with no terminal. It
  reads JSON-lines messages on stdin and writes JSON-lines frames on
  stdout.
- **Frames:** a frame is built by drawing the session into an in-memory
  ratatui buffer with the existing `ui::windows::draw_session`, which
  keeps every Fred feature at parity. The buffer is then sliced into
  per-window rows of styled runs plus structured status data.
- **App:** a pure Swift app with no FFI. It launches the helper, sends
  input, and draws each row with Core Text in a custom layer-backed
  `NSView`. Status bars and menus are native.

**Tech Stack:**
- Rust 2024, with serde/serde_json (already dependencies) and ratatui
  `TestBackend`.
- Swift 6 with SwiftPM, AppKit and Core Text, macOS 26+.
- The machine has Command Line Tools only, with no Xcode, so `swift test`
  is unavailable. Swift checks are an executable self-test target.

**Spec:** `docs/superpowers/specs/2026-10-21-native-macos-app-design.md`
(this plan is Phase 1 of the spec's "Phases").

## Global Constraints

- macOS 26+. Package platform: `.macOS("26.0")`.
- The app lives in `macos/` in this repo. Nothing in `macos/` may link
  the Rust code; the only interface is the helper process protocol below.
- The build turns the ghostty feature on: `cargo build --release
  --features ghostty`, with `PATH="/opt/homebrew/opt/zig@0.15/bin:$PATH"`.
- Latency targets:
  - Keypress to pixels under 16 ms at p99, aiming for 8 ms.
  - Measured from `keyDown` to the end of the draw that shows the echoed
    `seq`.
- Text must be crisp: glyph origins snap to device pixels, the layer
  `contentsScale` equals the window's `backingScaleFactor`, and the
  editor background is opaque.
- No character-cell chrome in Swift: no box-drawing borders. Status bars
  and dividers are native views or lines.
- AGENTS.md:
  - Preserve the user's uncommitted `notes.md`.
  - Never commit `claude-features.txt` or `sort-task.md`.
  - Commit only the files a task names.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Deviation from the spec (decided while planning)

- **No Cmd modifier on `Key`.** Mac keys arrive as their own `mac`
  message and are handled by `gui::mac::mac_key`, which looks at the
  mode. Fred's keymaps never bind Cmd, so the `Key` type and every
  keymap stay unchanged.
- **Option-drag column select moves to Phase 10**, together with multiple
  cursors.

## The protocol (shared by every task)

One JSON object per line, UTF-8.

**App → core (stdin):**

```json
{"t":"resize","cols":120,"rows":40}
{"t":"keys","seq":7,"keys":"<C-w>j"}
{"t":"paste","seq":8,"text":"pasted text"}
{"t":"mouse","seq":9,"kind":"down","col":12,"row":3}
{"t":"mac","seq":10,"key":"s","shift":false}
{"t":"quit"}
```

- **`keys`:**
  - Uses Fred's vim notation (`key::parse_keys`): `<Esc>`, `<Enter>`,
    `<BS>`, `<Tab>`, `<S-Tab>`, `<Up>` and the other arrows, `<Home>`,
    `<End>`, `<PageUp>`, `<PageDown>`, `<Del>` and `<lt>`.
  - Modifiers: `<C-x>`, `<M-x>` (Option is Alt), `<S-Left>`.
- **`mouse` kind:** one of `down`, `up`, `drag`, `right`, `scroll_up`,
  `scroll_down`. `col` and `row` are absolute cells.
- **`mac` key:** one of `s z c x a w f`. `shift` is only meaningful for
  `z`, where Cmd-Shift-Z is redo. Cmd-V is sent as `paste` with the
  pasteboard's text.
- **`quit`:** the window's close button. The core runs `:qa`. If that
  fails (unsaved changes), the core stays alive and the next frame shows
  the error.

**Core → app (stdout):**

```json
{"t":"frame","seq":10,"cols":120,"rows":40,"title":"fred — ~/github/fred",
 "windows":[{"x":0,"y":0,"w":120,"h":38,"divider":false,"active":true,
   "rows":[[{"x":0,"s":"  1 ","fg":8},{"x":4,"s":"fn","fg":33515186,"f":1}]],
   "status":{"mode":"NORMAL","name":"src/main.rs","modified":false,"pos":"12:5","branch":"main"}}],
 "bottom":[[{"x":0,"s":":w"}]],
 "cursor":{"x":4,"y":0,"shape":"block"}}
{"t":"exit","code":0}
```

- **`seq`:** the last input `seq` the core has handled. It is 0 before
  any input.
- **Window rects** are the layout rects, in cells:
  - They include the window's status row, which is the last row.
  - When `divider` is true, they also include the divider column, which
    is the last column.
  - `rows` has `h - 1` entries, one per text row. Columns are relative to
    the window.
  - A row is a list of runs. Adjacent cells with equal style merge into
    one run. Trailing default-styled blanks are dropped.
- **Run fields:**
  - `x`: the start column.
  - `s`: the text, where a wide character counts for 2 columns.
  - `fg` and `bg`: absent means the theme default. `0..=255` is a
    palette index (0–15 are the named ANSI colors, in ratatui order).
    `0x1000000 | rgb` is a 24-bit color, and the example's `33515186` is
    `0x1000000 | 0xff66b2`.
  - `f`: bit flags, absent when 0: 1 bold, 2 italic, 4 underline,
    8 reversed, 16 dim, 32 crossed out.
- **`bottom`:** the rows below the windows (the command row and the
  picker panel), as rows of runs.
- **`cursor`:**
  - Absolute cells. `null` when hidden.
  - `shape` is `bar` in Insert mode (not in a terminal buffer),
    `underline` in Replace mode, and `block` otherwise.
- **`exit`:** the last line before the core exits.

## File Structure

**Rust:**
- `src/app.rs`: extract `pub fn tick_session(...)` from `event_loop`.
- `src/gui/mod.rs`: `pub fn run(args, cfg) -> Result<i32>`, the stdin
  loop and background jobs.
- `src/gui/proto.rs`: `In` (deserialized input) and `Frame`, `Win`,
  `Run`, `Status`, `Cursor` (serialized output).
- `src/gui/frame.rs`: `pub fn build(...) -> Frame`.
- `src/gui/mac.rs`: `pub fn mac_key(s: &mut Session, key: char, shift: bool)`.
- `src/args.rs`, `src/main.rs`, `src/lib.rs`: add `--gui`.
- `tests/gui.rs`: end-to-end tests over the pipe.

**Swift (`macos/`):**
- `Package.swift`: executable `Fred`, library `FredKit`, executable
  `FredSelfTest`.
- `Sources/FredKit/Protocol.swift`: Codable mirrors of the protocol.
- `Sources/FredKit/KeyNotation.swift`: `NSEvent` facts to vim notation.
- `Sources/FredKit/Palette.swift`: color code to `NSColor`.
- `Sources/FredKit/Latency.swift`: the p50/p99 tracker.
- `Sources/Fred/main.swift`: app delegate, menu bar, open panel.
- `Sources/Fred/Core.swift`: helper process and JSON-lines I/O.
- `Sources/Fred/EditorView.swift`: Core Text drawing, input, mouse.
- `Sources/Fred/StatusBar.swift`: native per-window status strip.
- `Sources/FredSelfTest/main.swift`: assert-based checks.
- `build.sh`: builds the Rust release binary and the Swift app, then
  bundles `Fred.app` with the binary at `Contents/MacOS/fred-core`.

## Review Focus

1. **Unsaved changes when the window closes.** The `quit` message must
   not lose work. The core refuses with an E37-style error, the process
   stays alive, and the window stays open. The test is in Task 4.
2. **The helper crashes or exits unexpectedly.** The app must not hang
   or spin. It shows an alert with the core's stderr tail and closes the
   window; a panic still writes the swap file. The test is in Task 4
   (core exit on stdin EOF) and in Task 6 (EOF handling).
3. **Wide characters and tabs in runs.** A CJK character or a tab must
   not shift the columns after it. The test is in Task 2.
4. **Option-typed characters.** Option-j must reach Magit as `<M-j>` and
   not as `∆`. The test is in Task 7's self-test.
5. **A burst of fast typing.** Twenty `keys` messages sent at once must
   all apply in order, and the final frame's `seq` must equal the last
   one. The test is in Task 4.

---

## Track A: Rust core (Tasks 1–4, in order)

### Task 1: Share the background tick between the terminal loop and the GUI loop

**Files:**
- Modify: `src/app.rs` (the tail of `event_loop`, from the `checked.elapsed() >= AUTOREAD` block onward)

**Interfaces:**
- Produces: `pub fn tick_session(s: &mut Session, remote: Option<&crate::remote::Remote>, checked: &mut Instant) -> bool`.
  - Returns true when anything changed.
  - Covers autoread, `tick_magit`, `tick_claude`, `tick_terms`, the
    remote tool calls (only when `s.typing` is `None`) and `tick_typing`.
  - Also covers the `reloaded` handling, `pick::tick`, `pending_effect`,
    the git-marks refresh and `maybe_swap`, plus the message-changed
    check.
- `event_loop` keeps the terminal-only parts: `held` keys, `pending_git`,
  `pending_shell`, `pending_ai` and `pending_edit` (they need `ui`), and
  the window-height logic.

- [ ] **Step 1:** Move the listed code into `tick_session` without changing
  behavior. Its order inside the function must match today's order.
- [ ] **Step 2:** Run `cargo test` (with the ghostty PATH and
  `--features ghostty`).
  - Expected: everything passes except the known failures
    `magit::tests::bisect_finds_the_bad_commit_and_runs_scripts` and the
    occasionally flaky `process_kill_interrupts_background_git`.
- [ ] **Step 3:** Run `cargo clippy --features ghostty`. Expected: no new
  warnings (`draw_picker` "too many arguments" is pre-existing).
- [ ] **Step 4:** Commit `src/app.rs`: "refactor: share the background tick with a GUI loop".

### Task 2: Frame builder

**Files:**
- Create: `src/gui/mod.rs` (`pub mod proto; pub mod frame; pub mod mac;`
  for now; `run` comes in Task 4), `src/gui/proto.rs`, `src/gui/frame.rs`
- Modify: `src/lib.rs` (`pub mod gui;`)

**Interfaces:**
- Produces, in `proto.rs` (all `#[derive(Serialize, Deserialize, Debug, PartialEq)]`):
  - `pub struct Run { pub x: u16, pub s: String, #[serde(skip_serializing_if = "Option::is_none")] pub fg: Option<u32>, … bg, #[serde(default, skip_serializing_if = "is_zero")] pub f: u8 }`
  - `pub struct Status { pub mode: String, pub name: String, pub modified: bool, pub pos: String, pub branch: Option<String> }`
  - `pub struct Win { pub x: u16, pub y: u16, pub w: u16, pub h: u16, pub divider: bool, pub active: bool, pub rows: Vec<Vec<Run>>, pub status: Status }`
  - `pub struct Cursor { pub x: u16, pub y: u16, pub shape: String }`
  - `pub struct Frame { pub t: String /* "frame" */, pub seq: u64, pub cols: u16, pub rows: u16, pub title: String, pub windows: Vec<Win>, pub bottom: Vec<Vec<Run>>, pub cursor: Option<Cursor> }`
  - `pub fn color(c: ratatui::style::Color) -> Option<u32>`: `Reset` →
    `None`, named colors in ratatui order → 0–15, `Indexed(i)` → `i`,
    `Rgb` → `0x1000000 | rgb`.
- Produces, in `frame.rs`:
  `pub fn build(s: &mut Session, cfg: &Config, term: &mut ratatui::Terminal<TestBackend>, seq: u64) -> Frame`.
  - Resizing `term` is the caller's job.
  - `build` draws with `ui::windows::draw_session(f, s, cfg, app::HIGHLIGHT_BUDGET)`.
    Make `HIGHLIGHT_BUDGET` `pub(crate)`.
  - Then, as the terminal `Ui::draw` does after drawing, it sets
    `s.ed.viewport` and `s.ed.win_height` from the active window. Factor
    that into `pub(crate) fn drawn(s: &mut Session)` in `app.rs`, and have
    `Ui::draw` call it too.
  - Then it slices: windows from `s.tree.layout(s.area)` (active =
    `p.id == s.active`), and `bottom` = buffer rows from `s.area.bottom()`
    to the end.
  - The cursor comes from `term.backend().cursor_position()` and
    `cursor_visible()`.
  - `Status` comes straight from the `Editor` of each window (inside
    `s.each_window`):
    - `mode`: the same label the status line shows. Reuse or extract the
      label function `draw_status` uses (`TERMINAL` for Insert in a
      terminal buffer).
    - `name`: the path relative to the cwd, `term_title` for terminals,
      or `[No Name]`.
    - `pos`: `"{line+1}:{col+1}"`.
    - `branch`: the git branch, if `draw_status` has it.

- [ ] **Step 1: Write the failing tests** in `src/gui/frame.rs`, using a
  helper like `ui/tests.rs::session_screen`. Open a temp file and run
  `:set nonumber<Enter>`.

```rust
#[test]
fn a_window_row_is_styled_runs() {
    // "hello" in an 20x6 screen: one window, 4 text rows + status + command row.
    let (mut s, mut term, _d) = setup(20, 6, "hello\n");
    let f = build(&mut s, &Config::default(), &mut term, 3);
    assert_eq!(f.seq, 3);
    assert_eq!((f.cols, f.rows), (20, 6));
    assert_eq!(f.windows.len(), 1);
    let w = &f.windows[0];
    assert!(w.active);
    assert_eq!(w.rows.len(), w.h as usize - 1);
    let text: String = w.rows[0].iter().map(|r| r.s.as_str()).collect();
    assert_eq!(text, "hello");
    assert_eq!(w.status.mode, "NORMAL");
    assert_eq!(w.status.pos, "1:1");
    assert_eq!(f.cursor.as_ref().map(|c| (c.x, c.y, c.shape.as_str())), Some((0, 0, "block")));
}

#[test]
fn wide_chars_and_tabs_keep_columns() {
    let (mut s, mut term, _d) = setup(30, 6, "中x\ty\n");
    let f = build(&mut s, &Config::default(), &mut term, 0);
    let row = &f.windows[0].rows[0];
    // Rebuild the line by column: '中' covers columns 0-1, 'x' is at 2, the tab
    // pads to column 8 (tabstop 8), 'y' is at 8.
    let at = |col: u16| row.iter().find(|r| r.x <= col && col < r.x + unicode_width::UnicodeWidthStr::width(r.s.as_str()) as u16).map(|r| r.s.clone());
    assert!(at(0).unwrap().starts_with('中'));
    assert!(at(8).unwrap().contains('y'));
}

#[test]
fn insert_mode_is_a_bar_and_command_line_is_in_bottom() {
    let (mut s, mut term, _d) = setup(20, 6, "hello\n");
    s.handle_keys("i");
    let f = build(&mut s, &Config::default(), &mut term, 0);
    assert_eq!(f.cursor.unwrap().shape, "bar");
    assert_eq!(f.windows[0].status.mode, "INSERT");
    s.handle_keys("<Esc>:w");
    let f = build(&mut s, &Config::default(), &mut term, 0);
    let bottom: String = f.bottom.concat().iter().map(|r| r.s.as_str()).collect();
    assert!(bottom.starts_with(":w"), "{bottom:?}");
}

#[test]
fn split_windows_have_their_own_rects_and_status() {
    let (mut s, mut term, _d) = setup(40, 12, "hello\n");
    s.handle_keys("<C-w>v");
    let f = build(&mut s, &Config::default(), &mut term, 0);
    assert_eq!(f.windows.len(), 2);
    assert_eq!(f.windows.iter().filter(|w| w.active).count(), 1);
    assert!(f.windows.iter().any(|w| w.divider));
}

#[test]
fn colors_encode_as_palette_or_rgb() {
    use ratatui::style::Color;
    assert_eq!(proto::color(Color::Reset), None);
    assert_eq!(proto::color(Color::DarkGray), Some(8));
    assert_eq!(proto::color(Color::Indexed(244)), Some(244));
    assert_eq!(proto::color(Color::Rgb(0xff, 0x66, 0xb2)), Some(0x1ff66b2));
}

#[test]
fn frame_json_omits_defaults() {
    let run = proto::Run { x: 0, s: "a".into(), fg: None, bg: None, f: 0 };
    assert_eq!(serde_json::to_string(&run).unwrap(), r#"{"x":0,"s":"a"}"#);
}
```

- [ ] **Step 2:** Run `cargo test --features ghostty gui::frame`.
  Expected: FAIL to compile, because `build` is not defined.
- [ ] **Step 3:** Implement `proto.rs` and `frame.rs` as specified.
  - The highlighter must use truecolor (`Highlighter::new(theme, true)`)
    so syntax colors arrive as RGB. That is the caller's job in Task 4,
    but the tests construct theirs the same way.
  - Merge adjacent cells with equal `(fg, bg, modifier)` into one run.
  - Skip a cell that is covered by the wide character before it. Its
    symbol is `""` in ratatui 0.30, or `" "` after a wide character;
    follow the old branch's `covered` counting in
    `git show origin/guylyons/macos-gui:src/gui.rs`, `render()`.
- [ ] **Step 4:** Run `cargo test --features ghostty gui::frame`.
  Expected: PASS.
- [ ] **Step 5:** Commit `src/gui/`, `src/lib.rs` and `src/app.rs`:
  "feat(gui): frames of styled runs for a native front end".

### Task 3: Mac keys, by mode

**Files:**
- Create: `src/gui/mac.rs`

**Interfaces:**
- Produces: `pub fn mac_key(s: &mut Session, key: char, shift: bool)`.
  - Run ex commands with `Editor::run_ex` plus `Session::perform`, or with
    `Session::handle_keys`. Either is fine, as long as the mode after
    each key matches the table.

| key | Normal | Visual / VisualLine | Insert |
|---|---|---|---|
| `s` | `:w` | `:w`, selection kept | `:w`, still Insert, cursor unchanged |
| `z` | `u` | Esc, then `u` | undo, still Insert |
| `z` + shift | `<C-r>` | Esc, then `<C-r>` | redo, still Insert |
| `c` | current line to clipboard (`"+yy`), cursor unchanged | `"+y` (leaves Visual) | nothing |
| `x` | `"+dd` | `"+d` | nothing |
| `a` | `ggVG` | `ggVG` | Esc, then `ggVG` |
| `w` | `:q` | Esc, then `:q` | Esc, then `:q` |
| `f` | `/` | Esc, then `/` | Esc, then `/` |

Any other mode (command line, picker, Magit's own prompts): Esc first,
then the Normal column. Keys outside `s z c x a w f` do nothing.

- [ ] **Step 1: Write the failing tests** in `src/gui/mac.rs`. The
  clipboard is test-local under `cfg(test)`; see `clipboard.rs`.

```rust
#[test]
fn cmd_s_saves_without_leaving_insert() {
    let (mut s, d) = open("one\n");
    s.handle_keys("Ahi");
    mac_key(&mut s, 's', false);
    assert_eq!(s.ed.mode, Mode::Insert);
    assert_eq!(std::fs::read_to_string(d.path().join("f")).unwrap(), "onehi\n");
}

#[test]
fn cmd_z_and_cmd_shift_z_undo_and_redo_in_normal() {
    let (mut s, _d) = open("one\n");
    s.handle_keys("xx");
    mac_key(&mut s, 'z', false);
    assert_eq!(s.ed.buf.line(0), "ne"); // one x undone
    mac_key(&mut s, 'z', true);
    assert_eq!(s.ed.buf.line(0), "e");
}

#[test]
fn cmd_c_in_visual_copies_the_selection_and_leaves_visual() {
    let (mut s, _d) = open("hello world\n");
    s.handle_keys("vee");
    mac_key(&mut s, 'c', false);
    assert_eq!(s.ed.mode, Mode::Normal);
    assert_eq!(crate::clipboard::get().as_deref(), Some("hello world"));
}

#[test]
fn cmd_a_selects_everything() {
    let (mut s, _d) = open("a\nb\nc\n");
    mac_key(&mut s, 'a', false);
    assert!(matches!(s.ed.mode, Mode::VisualLine { .. }));
    assert_eq!(s.ed.visual_lines(), Some((0, 2)));
}

#[test]
fn cmd_f_opens_search() {
    let (mut s, _d) = open("a\n");
    s.handle_keys("i");
    mac_key(&mut s, 'f', false);
    assert!(matches!(s.ed.mode, Mode::Command(ref cl) if cl.kind == '/'));
}
```

  Match the helper names (`open`, `clipboard::get`) to what exists;
  `clipboard.rs` has the test-local getter. If `buf.line` keeps the
  newline, compare with it trimmed.

- [ ] **Step 2:** Run `cargo test --features ghostty gui::mac`.
  Expected: FAIL to compile.
- [ ] **Step 3:** Implement `mac_key` per the table.
- [ ] **Step 4:** Run `cargo test --features ghostty gui::mac`.
  Expected: PASS.
- [ ] **Step 5:** Commit `src/gui/mac.rs`: "feat(gui): Mac Cmd keys that respect the mode".

### Task 4: `fred --gui`: the helper process loop

**Files:**
- Modify: `src/gui/mod.rs`, `src/args.rs` (a `gui: bool` field and the
  `--gui` flag, plus one usage line: `--gui  speak the native app's
  protocol on stdin/stdout`), `src/main.rs` (dispatch to `gui::run` when
  `args.gui`)
- Create: `tests/gui.rs`

**Interfaces:**
- Consumes: `app::tick_session` (Task 1), `frame::build` and `proto`
  (Task 2), `mac::mac_key` (Task 3).
- Produces: `pub fn run(args: Args, cfg: Config, cfg_err: Option<String>) -> anyhow::Result<i32>`,
  which `main.rs` calls in place of `app::run`.
- Produces: `proto::In`, `#[serde(tag = "t", rename_all = "lowercase")]`,
  with the variants `Resize{cols,rows}`, `Keys{seq,keys}`,
  `Paste{seq,text}`, `Mouse{seq,kind,col,row}`, `Mac{seq,key,shift}` and
  `Quit`.

**Behavior:**
1. **Open.** Open the session like `app::run`: Magit options,
   `Session::open`, `+LINE`, and the highlighter with
   `Highlighter::new(&cfg.theme, true)` falling back to `"ansi"`.
   - Also `set_file`, the config error message, `nearby`,
     `remote::Remote::listen()` and `s.lock()`.
   - A leftover swap file opens read-only with the message
     `swap file found: opened read-only` (ponytail: the swap prompt goes
     native in Phase 6).
   - Skip the tty check, raw mode and the crossterm setup.
2. **Read input.** A reader thread reads stdin lines into an
   `mpsc::Receiver<In>`.
   - EOF (the app died) behaves like SIGHUP: `write_swap`, exit 1.
   - Unparseable lines are ignored, with one `eprintln!`.
3. **Loop.** `recv_timeout(tick)`, where tick is `TERM_TICK` while a
   terminal is active and recently used, `TICK` while busy, and
   `IDLE_TICK` otherwise, as in `event_loop`.
   - Drain every queued message before drawing, which batches bursts.
   - `Keys` → `s.handle_keys(&keys)`. In terminal mode
     (`s.in_terminal()`), map each parsed `Key` to a crossterm `KeyEvent`
     and call `s.term_key`.
   - `Paste` → `s.ed.paste` (or `s.term_paste` in a terminal).
   - `Mouse` → build a crossterm `MouseEvent` and call
     `s.mouse(full_area, m)`.
   - `Mac` → `mac_key`.
   - `Resize` → resize the `TestBackend` terminal and set `s.ed.zap = None`.
   - `Quit` → `s.handle_keys("<Esc>:qa<Enter>")`.
   - After each message, do the `take_dirty_from` → `hl.invalidate` step
     that `step()` does.
4. **Background work.** Run `app::tick_session`, plus the GUI's own jobs
   for `pending_git`, `pending_shell`, `pending_ai` and `pending_edit`.
   Port `Job`, `git_job` and the job polling from
   `git show origin/guylyons/macos-gui:src/gui.rs`; `pending_edit` takes
   the read-only path above.
5. **Draw.** When dirty, call `frame::build` and write one JSON line to
   stdout, then flush. Keep drawing while `s.hl_incomplete`.
6. **Quit.** On `s.quit`: `remember_place`, `cleanup`, write
   `{"t":"exit","code":0}`, return 0.
   - Nothing else may print to stdout. `s.written` is not printed in GUI
     mode.
   - A panic is caught like `app::run` does: write the swap, write
     `{"t":"exit","code":101}`, and put the note on stderr.

- [ ] **Step 1: Write the failing tests** in `tests/gui.rs`. Spawn
  `env!("CARGO_BIN_EXE_fred")` with `--gui FILE` in a temp dir, with
  `HOME` and `XDG_CONFIG_HOME` pointed at the temp dir. Use piped stdin
  and stdout, and a helper that sends lines and reads frames until a
  predicate holds or 5 s pass.

```rust
#[test]
fn types_saves_and_quits_over_the_pipe() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"Atwo<Esc>"}"#);
    let f = g.frame_until(|f| f["seq"] == 1);
    assert_eq!(row_text(&f, 0, 0), "onetwo");
    g.send(r#"{"t":"mac","seq":2,"key":"s","shift":false}"#);
    g.frame_until(|f| f["seq"] == 2);
    assert_eq!(std::fs::read_to_string(g.path("f")).unwrap(), "onetwo\n");
    g.send(r#"{"t":"quit"}"#);
    assert_eq!(g.exit_line()["code"], 0);
    assert!(g.wait().success());
}

#[test]
fn quit_with_unsaved_changes_keeps_running() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"x"}"#);
    g.send(r#"{"t":"quit"}"#);
    let f = g.frame_until(|f| bottom_text(f).contains("E37"));
    assert!(bottom_text(&f).contains("E37"));
    assert!(g.still_running());
}

#[test]
fn a_burst_of_keys_applies_in_order() {
    let mut g = Gui::spawn("f", "\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"i"}"#);
    for n in 2..=21 {
        g.send(&format!(r#"{{"t":"keys","seq":{n},"keys":"{}"}}"#, (b'a' + (n - 2) as u8) as char));
    }
    let f = g.frame_until(|f| f["seq"] == 21);
    assert_eq!(row_text(&f, 0, 0), "abcdefghijklmnopqrst");
}

#[test]
fn closed_stdin_saves_the_swap_and_exits() {
    let mut g = Gui::spawn("f", "one\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":"x"}"#);
    g.frame_until(|f| f["seq"] == 1);
    g.close_stdin();
    assert_eq!(g.wait().code(), Some(1));
}

#[test]
fn mouse_drag_selects() {
    let mut g = Gui::spawn("f", "hello world\n");
    g.send(r#"{"t":"resize","cols":40,"rows":10}"#);
    g.send(r#"{"t":"keys","seq":1,"keys":":set nonumber<Enter>"}"#);
    g.send(r#"{"t":"mouse","seq":2,"kind":"down","col":0,"row":0}"#);
    g.send(r#"{"t":"mouse","seq":3,"kind":"drag","col":4,"row":0}"#);
    g.send(r#"{"t":"mouse","seq":4,"kind":"up","col":4,"row":0}"#);
    let f = g.frame_until(|f| f["seq"] == 4);
    assert_eq!(f["windows"][0]["status"]["mode"], "VISUAL");
}
```

  `row_text(f, win, row)` joins the `s` of a row's runs. `bottom_text`
  joins every `bottom` run. Frames are parsed with `serde_json::Value`.

- [ ] **Step 2:** Run `cargo test --features ghostty --test gui`.
  Expected: FAIL, because `--gui` is unknown.
- [ ] **Step 3:** Implement `gui::run`, the `--gui` arg and the main
  dispatch.
- [ ] **Step 4:** Run `cargo test --features ghostty --test gui`, then
  the full `cargo test --features ghostty`. Expected: PASS, apart from
  the known failures listed in Task 1.
- [ ] **Step 5:** Commit `src/gui/mod.rs`, `src/gui/proto.rs`,
  `src/args.rs`, `src/main.rs` and `tests/gui.rs`:
  "feat(gui): fred --gui, the native app's helper process".

---

## Track B: Swift app (Tasks 5–8; Task 5 first, then 6 and 7 in parallel, then 8)

Track B depends only on the protocol section above. Until Task 4 lands,
test against a fake core: `macos/fake-core.sh`, created in Task 5. It
prints one canned frame line on every input line.

### Task 5: Package, protocol types, palette and the self-test runner

**Files:**
- Create: `macos/Package.swift`, `macos/Sources/FredKit/Protocol.swift`,
  `macos/Sources/FredKit/Palette.swift`,
  `macos/Sources/FredSelfTest/main.swift`, `macos/Sources/Fred/main.swift`
  (an empty window for now), `macos/build.sh`, `macos/fake-core.sh`,
  `macos/.gitignore` (`.build/`, `Fred.app/`)

**Interfaces:**
- Produces (FredKit, `public`):
  - `struct Run: Codable, Equatable { x: Int; s: String; fg: Int?; bg: Int?; f: Int? }`
  - `struct Status: Codable, Equatable { mode, name: String; modified: Bool; pos: String; branch: String? }`
  - `struct Win: Codable, Equatable { x, y, w, h: Int; divider, active: Bool; rows: [[Run]]; status: Status }`
  - `struct Cursor: Codable, Equatable { x, y: Int; shape: String }`
  - `struct Frame: Codable { seq: Int; cols, rows: Int; title: String; windows: [Win]; bottom: [[Run]]; cursor: Cursor? }`
  - `enum Incoming { case frame(Frame); case exit(code: Int) }` with
    `static func decode(_ line: Data) throws -> Incoming`, which switches
    on `"t"`.
  - `enum Outgoing { case resize(cols:Int, rows:Int); case keys(seq:Int, keys:String); case paste(seq:Int, text:String); case mouse(seq:Int, kind:String, col:Int, row:Int); case mac(seq:Int, key:String, shift:Bool); case quit }`
    with `func line() -> Data`: one JSON object plus `\n`, with the field
    names exactly as in the protocol.
  - `struct Palette { var fg: NSColor; var bg: NSColor; func color(_ code: Int?, default: NSColor) -> NSColor }`
    - `nil` → the default.
    - 0–15 → a fixed ANSI table. Use a dark, modern set (e.g. the One
      Dark values); this is the builder's call.
    - 16–231 → the 6×6×6 cube; 232–255 → the grey ramp.
    - `>= 0x1000000` → RGB.
- `Package.swift`:
  - swift-tools-version 6.0, `.macOS("26.0")`.
  - Targets: `FredKit` (library), `Fred` (executable, depends on
    FredKit) and `FredSelfTest` (executable, depends on FredKit).
- `build.sh`:
  - `cargo build --release --features ghostty`, with the zig@0.15 PATH
    prefix.
  - `swift build -c release --package-path macos`.
  - Bundle `macos/Fred.app/Contents/{MacOS/Fred, MacOS/fred-core, Info.plist}`.
    Info.plist as on the old branch, plus `LSMinimumSystemVersion 26.0`
    and the bundle id `dev.fred.editor`.

- [ ] **Step 1: Write the failing self-test** in
  `Sources/FredSelfTest/main.swift`. It is a list of `check(name, cond)`
  calls, prints `ok N` or `FAIL name`, and exits 1 on any failure.

```swift
let line = #"{"t":"frame","seq":3,"cols":20,"rows":6,"title":"t","windows":[{"x":0,"y":0,"w":20,"h":5,"divider":false,"active":true,"rows":[[{"x":0,"s":"hello","fg":33515186,"f":1}]],"status":{"mode":"NORMAL","name":"f","modified":false,"pos":"1:1","branch":null}}],"bottom":[],"cursor":{"x":0,"y":0,"shape":"block"}}"#
if case .frame(let f) = try Incoming.decode(Data(line.utf8)) {
    check("seq", f.seq == 3)
    check("run text", f.windows[0].rows[0][0].s == "hello")
    check("bold", f.windows[0].rows[0][0].f == 1)
} else { check("decodes a frame", false) }
check("exit", { if case .exit(let c) = try! Incoming.decode(Data(#"{"t":"exit","code":0}"#.utf8)) { return c == 0 }; return false }())
check("keys line", String(decoding: Outgoing.keys(seq: 7, keys: "<C-w>j").line(), as: UTF8.self) == #"{"t":"keys","seq":7,"keys":"<C-w>j"}"# + "\n")
let p = Palette(fg: .white, bg: .black)
check("rgb", p.color(0x1ff66b2, default: .white).usingColorSpace(.sRGB)!.redComponent == 1.0)
check("default", p.color(nil, default: .black) == .black)
```

  JSON key order matters for the `keys line` check, so encode by hand
  or with `.sortedKeys` off and a fixed field order. A hand-written
  encoder is fine.

- [ ] **Step 2:** Run `swift run --package-path macos FredSelfTest`.
  Expected: FAIL to compile.
- [ ] **Step 3:** Implement the package, `Protocol.swift`,
  `Palette.swift`, an empty-window `main.swift`, `build.sh` and
  `fake-core.sh`.
- [ ] **Step 4:** Run `swift run --package-path macos FredSelfTest`.
  Expected: every check prints `ok` and it exits 0. Then run
  `./macos/build.sh`. Expected: it ends with `built …/Fred.app`.
- [ ] **Step 5:** Commit `macos/`: "feat(macos): Swift package, protocol and palette".

### Task 6: Core process and the Core Text editor view

**Files:**
- Create: `macos/Sources/Fred/Core.swift`, `macos/Sources/Fred/EditorView.swift`
- Modify: `macos/Sources/Fred/main.swift`

**Interfaces:**
- Produces:
  - `final class Core`:
    - `init(executable: URL, args: [String], cwd: URL)`.
    - `var onFrame: (Frame) -> Void` and `var onExit: (Int, String) -> Void`.
      The `String` is the last 20 lines of stderr. Both are called on
      the main queue.
    - `func send(_ m: Outgoing)`.
    - A background thread reads stdout lines and decodes them with
      `Incoming.decode`.
    - EOF without an `exit` line calls `onExit(-1, stderrTail)`.
  - `final class EditorView: NSView`:
    - `var font: NSFont` (default `NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)`).
    - `var palette: Palette`.
    - `func apply(_ f: Frame)`.
    - `var cell: CGSize`, computed: width = advance of `"M"`,
      height = `ceil((ascent + descent + leading) * 1.25 * scale) / scale`.
    - `var onResize: (Int, Int) -> Void`. It is called on every
      `setFrameSize` and backing-scale change that changes cols or rows.
- Executable path: `Bundle.main.executableURL!.deletingLastPathComponent().appendingPathComponent("fred-core")`.
  The environment variable `FRED_CORE` overrides it for development (for
  example `target/release/fred` or `macos/fake-core.sh`).
- Args: `["--gui"] + [path]`.
- Path, in priority order:
  1. The first non-flag command-line argument.
  2. Otherwise an `NSOpenPanel` that can choose files and directories.
     The cwd is the chosen folder, or the file's parent.

**Drawing rules (these fix crispness and latency):**
- The view has `wantsLayer = true`, `layerContentsRedrawPolicy = .onSetNeedsDisplay`,
  `isOpaque = true`, and `layer.contentsScale = window.backingScaleFactor`.
- `apply` compares each window's rows with the previous frame's. It
  calls `setNeedsDisplay(rowRect)` only for changed rows, plus the old
  and new cursor rows. A layout change (window rects or cols/rows)
  redraws everything.
- Each row is drawn in `draw(_:)`:
  - Fill the background spans first, then draw each run's text with a
    cached `CTLine`. The cache is keyed by run text plus fg and flags,
    and is cleared when the font changes.
  - A run's origin x is `run.x * cell.width`. Snap the x origin and the
    baseline y to device pixels (`round(v * scale) / scale`).
  - Bold uses `font` with the `.bold` trait. Italic uses the italic
    trait, falling back to an oblique transform. Underline and crossed
    out are drawn as 1-pixel lines.
  - Reversed swaps fg and bg. Dim draws fg at 0.6 alpha.
- Window chrome:
  - A window's last row is left for `StatusBar` (Task 8).
  - A divider column is drawn as a 1-pixel `NSColor.separatorColor` line
    at the column's center.
- The cursor is drawn last:
  - `block`: a filled cell in the theme fg at 0.85 alpha, with the
    character redrawn on top in the bg color.
  - `bar`: 2 points wide.
  - `underline`: 2 points high.
  - It doesn't blink in Phase 1.
- `bottom` rows are drawn below the windows with the same run drawing.

- [ ] **Step 1:** Add the self-test checks for the pure parts. Move
  `cellSize(font:scale:)` and `snap(_:scale:)` into FredKit as `public`
  functions so they can be checked:

```swift
check("snap 2x", snap(10.3, scale: 2) == 10.5)
check("snap 1x", snap(10.3, scale: 1) == 10.0)
let c = cellSize(font: .monospacedSystemFont(ofSize: 13, weight: .regular), scale: 2)
check("cell height on pixel grid", (c.height * 2).rounded() == c.height * 2)
check("cell width positive", c.width > 7 && c.width < 9)
```

- [ ] **Step 2:** Run `swift run --package-path macos FredSelfTest`.
  Expected: FAIL to compile.
- [ ] **Step 3:** Implement `Core`, `EditorView`, `snap`, `cellSize`, and
  the `main.swift` wiring. One window whose content view is the
  `EditorView`; the title comes from `frame.title`.
  - `onExit(0, _)` closes the window and quits the app.
  - Any other `onExit` shows an `NSAlert` with the stderr tail, then
    quits.
- [ ] **Step 4:** Run the self-test (expected: all ok). Then run
  `FRED_CORE=$PWD/macos/fake-core.sh swift run --package-path macos Fred /tmp`.
  Expected: a window that shows the canned frame's text.
- [ ] **Step 5:** Commit `macos/Sources/`: "feat(macos): helper process and Core Text editor view".

### Task 7: Keyboard, mouse, paste and the menu bar

**Files:**
- Create: `macos/Sources/FredKit/KeyNotation.swift`
- Modify: `macos/Sources/Fred/EditorView.swift`, `macos/Sources/Fred/main.swift`

**Interfaces:**
- Consumes: `EditorView`, `Core.send` (Task 6). Task 7 can start in
  parallel with Task 6 on `KeyNotation`, which is pure.
- Produces: `public func notation(chars: String, charsIgnoringModifiers: String, keyCode: UInt16, control: Bool, option: Bool, shift: Bool, command: Bool) -> String?`.
  - It returns `nil` for Cmd chords; those go to the menu bar.
  - Special keys by `keyCode`: 53 → `<Esc>`, 36 and 76 → `<Enter>`,
    51 → `<BS>`, 48 → `<Tab>` (with shift, `<S-Tab>`), 117 → `<Del>`,
    123–126 → `<Left>`, `<Right>`, `<Down>`, `<Up>` (with shift, `<S-…>`),
    115 → `<Home>`, 119 → `<End>`, 116 → `<PageUp>`, 121 → `<PageDown>`.
  - Control: `<C-x>`, using `charsIgnoringModifiers`, lowercased.
  - Option is Alt: `<M-x>`, using `charsIgnoringModifiers`. With
    control too, it is `<C-M-x>`.
  - A plain character uses `chars`. `<` becomes `<lt>`.
- `EditorView` behavior:
  - `keyDown` → `notation(...)` → `core.send(.keys(seq: next, keys:))`,
    and it records `seq` with the `keyDown` timestamp for Task 8.
  - It uses `interpretKeyEvents` only for marked text (IME). Phase 1
    sends committed text through `insertText` as plain keys; dead keys
    and IME composition get the full treatment in a later phase.
  - Mouse:
    - `mouseDown` sends `down`, or `right` with the control key.
    - `mouseDragged` sends `drag` and `mouseUp` sends `up`.
    - `rightMouseDown` sends `right`.
    - `scrollWheel` sends one `scroll_up` or `scroll_down` per 3 points
      of accumulated `scrollingDeltaY`. Precise trackpad deltas
      accumulate.
    - Points are converted to cells with `cell`. The view is flipped, so
      the origin is the top-left.
- Menu bar, built in code. Each item sends the matching `mac` message:
  - **Fred:** About, Settings… (disabled in Phase 1), Hide, Quit (Cmd-Q
    sends `quit`).
  - **File:** Open… (Cmd-O, disabled in Phase 1), Save (Cmd-S, `s`),
    Close Window (Cmd-W, `w`).
  - **Edit:** Undo (Cmd-Z, `z`), Redo (Cmd-Shift-Z, `z` shift), Cut
    (Cmd-X, `x`), Copy (Cmd-C, `c`), Paste (Cmd-V, `paste` with
    `NSPasteboard.general.string(forType: .string)`), Select All (Cmd-A,
    `a`), Find (Cmd-F, `f`).
  - **View:** Bigger (Cmd-=), Smaller (Cmd--) and Actual Size (Cmd-0).
    These change `EditorView.font` size by 1 point, in the range 9–36,
    and trigger a resize.
  - **Window:** the standard menu.
- The window's close button sends `quit` (`windowShouldClose` returns
  false, and the window closes when `exit` arrives).

- [ ] **Step 1:** Add the self-test checks:

```swift
func n(_ c: String, _ ign: String? = nil, code: UInt16 = 0, ctrl: Bool = false, opt: Bool = false, shift: Bool = false, cmd: Bool = false) -> String? {
    notation(chars: c, charsIgnoringModifiers: ign ?? c, keyCode: code, control: ctrl, option: opt, shift: shift, command: cmd)
}
check("plain", n("a") == "a")
check("lt", n("<") == "<lt>")
check("ctrl", n("\u{17}", "w", ctrl: true) == "<C-w>")
check("option is alt", n("∆", "j", opt: true) == "<M-j>")
check("esc", n("\u{1b}", code: 53) == "<Esc>")
check("shift tab", n("\u{19}", code: 48, shift: true) == "<S-Tab>")
check("shift left", n("", code: 123, shift: true) == "<S-Left>")
check("cmd goes to menus", n("s", cmd: true) == nil)
check("space", n(" ") == " ")
```

- [ ] **Step 2:** Run the self-test. Expected: FAIL to compile.
- [ ] **Step 3:** Implement `notation`, the input handling and the menu
  bar.
- [ ] **Step 4:** Run the self-test (expected: all ok). Then build with
  `./macos/build.sh` and run `open macos/Fred.app --args $PWD/README.md`.
  Check by hand:
  - typing works;
  - `jj` leaves Insert;
  - Cmd-S saves;
  - click and drag selects;
  - the wheel scrolls;
  - Cmd-= zooms and the text stays sharp.
- [ ] **Step 5:** Commit `macos/Sources/`: "feat(macos): keys, mouse, paste and the menu bar".

### Task 8: Status bars, latency measurement and the README

**Files:**
- Create: `macos/Sources/Fred/StatusBar.swift`, `macos/Sources/FredKit/Latency.swift`
- Modify: `macos/Sources/Fred/EditorView.swift`, `README.md` (a short
  "Native macOS app (preview)" section: what it is, `./macos/build.sh`,
  the zig requirement, `open macos/Fred.app --args PATH`, `FRED_CORE`,
  `FRED_LATENCY=1`)

**Interfaces:**
- Produces: `public struct Latency { mutating func start(seq: Int, at: CFTimeInterval); mutating func shown(seq: Int, at: CFTimeInterval); var p50: Double; var p99: Double; var count: Int }`.
  - Values are in milliseconds.
  - `shown(seq:)` closes every started seq up to and including `seq`,
    using the time the draw finished.
  - It keeps the last 1000 samples.
- `StatusBar: NSView` is placed over each window's last cell row:
  - The background is `NSVisualEffectView` (`.titlebar` material) for the
    active window, and the same view at 0.5 alpha for inactive ones.
  - Left side: the mode badge, a rounded rect with SF 11 semibold,
    colored by mode: NORMAL blue, INSERT green, VISUAL orange, COMMAND
    and others purple, TERMINAL teal. Then the name, with `●` when
    modified.
  - Right side, in secondary label color: branch and `pos`.
- Latency is measured in `EditorView.draw(_:)`: the end of a draw calls
  `latency.shown(seq: frame.seq, at: CACurrentMediaTime())`.
  - With `FRED_LATENCY=1`, print `latency p50 X ms p99 Y ms (n=N)` to
    stderr every 100 samples.
  - Also show a small overlay at the top right with the same text.

- [ ] **Step 1:** Add the self-test checks:

```swift
var l = Latency()
l.start(seq: 1, at: 0.000); l.start(seq: 2, at: 0.001)
l.shown(seq: 2, at: 0.006)
check("closes both", l.count == 2)
check("p50", abs(l.p50 - 5.0) < 0.01 || abs(l.p50 - 6.0) < 0.01)
check("p99 is the max of two", abs(l.p99 - 6.0) < 0.01)
```

- [ ] **Step 2:** Run the self-test. Expected: FAIL to compile.
- [ ] **Step 3:** Implement `Latency`, `StatusBar` and the overlay, and
  write the README section.
- [ ] **Step 4:** Run the self-test (expected: all ok). Then
  `./macos/build.sh && FRED_LATENCY=1 open macos/Fred.app --args $PWD/src/session.rs`,
  hold `j` for 5 s, then type a paragraph in Insert mode.
  - Record the printed p50 and p99 in the commit message.
  - Pass: p99 < 16 ms.
  - On a miss: commit anyway, write down where the time goes (JSON
    encode, pipe, decode or draw; time each with `CACurrentMediaTime`),
    and report it. That is the spec's trigger for the binary protocol or
    Metal, which this plan doesn't do.
- [ ] **Step 5:** Commit `macos/Sources/` and `README.md`:
  "feat(macos): native status bars and latency measurement (p50 X / p99 Y ms)".

---

## Parallelism (up to 5 agents)

- **Agent 1:** Track A, Tasks 1→2→3→4, in a worktree on branch
  `gui-core`. Tasks 2 and 3 only share `src/gui/mod.rs`'s `pub mod`
  lines. A second agent may take Task 3 after Task 2's `proto.rs`
  exists.
- **Agent 2:** Track B, Task 5, then Task 6, in a worktree on branch
  `gui-app`.
- **Agent 3:** Task 7's `KeyNotation`, in parallel with Task 6, then the
  rest of Task 7 after Task 6.
- **Agent 4:** reviewer for each finished task.
- **Merge:** both branches merge into `main` before Task 8's latency run,
  which needs the real core.
