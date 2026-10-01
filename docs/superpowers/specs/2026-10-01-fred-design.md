# fred — design

Date: 2026-10-01. Status: approved in brainstorming.

## Purpose

`fred` is a daily-driver terminal text editor for quick edits: get in, move
around, change something, save, get out. It keeps ed's footprint — it lives in
your shell prompt instead of taking over the screen — but you move and edit
with vim keys. It has autocompletion and syntax highlighting built in, and it
is robust: it never corrupts or silently loses a file.

Success: opening a file, making a vim-style edit, and saving feels as fast and
natural as vim, the shell scrollback is left intact, and no sequence of crashes,
signals, or concurrent modification loses data.

## Scope

In v1:
- Inline viewport under the shell prompt (not fullscreen).
- Core vim editing (Normal, Insert, Visual-line).
- ed-style `:` command line with full ed addresses (essential commands).
- Automatic completion popup: buffer words, nearby files, paths.
- syntect syntax highlighting.
- Safe writes, change detection, swap-file crash recovery, real-world file
  handling (large files, Unicode/wide chars, CRLF, missing final newline).

Out of v1 (designed not to preclude): text objects, named registers, macros,
charwise visual, LSP, tree-sitter, multiple buffers/windows, persistent undo,
system clipboard, mouse, `:set`, non-interactive script mode (`fred -s`).

## Command line

```
fred [--height N] [+LINE] [FILE]
fred --help | --version
```
- One file. A missing file opens an empty buffer named FILE (`"FILE" [new]`).
  No FILE opens an unnamed buffer; `:w name` names it.
- `+LINE` starts the cursor on LINE (`+` alone = last line).
- stdin and stdout must both be terminals, else `fred: not a terminal`, exit 1.
- Exit codes: 0 normal, 1 error at startup.

## Config

`$XDG_CONFIG_HOME/fred/config.toml` (default `~/.config/fred/config.toml`).
All keys optional; unknown keys are an error reported on the status line
(the editor still starts with defaults).

| key | default | meaning |
|---|---|---|
| `height` | 12 | text rows in the window |
| `wrap` | false | soft-wrap long lines |
| `numbers` | true | line-number gutter |
| `relative_numbers` | false | relative numbers in gutter |
| `theme` | `"ansi"` | syntect theme name (two-face set) |
| `tabstop` | 8 | display width of a tab |
| `autocomplete` | true | auto popup in Insert mode |

`--height` overrides `height`.

## Architecture

One crate, `fred`: `src/lib.rs` (everything testable) + `src/main.rs`.

```
main.rs        args, config, swap-recovery prompt, run loop
ui/            ratatui inline viewport: draws Editor, reads key events
editor.rs      Editor state; routes keys to vim / ex
vim/           key state machine → editor actions
ex/            ed address parser + command interpreter
buffer.rs      ropey rope + file format info; all changes via Buffer::apply
undo.rs        linear undo/redo of edit groups
highlight.rs   syntect wrapper with per-line state cache
complete/      word index, nearby-file indexer, path + command completion
fileio.rs      load, safe write, change detection
swap.rs        swap file write + recovery
config.rs      config file
```

Data flow per key: ui reads key → `Editor::handle_key` → vim/ex produce
`Edit`s → `Buffer::apply` records undo, marks dirty, reports first changed line
to highlighter and completion index, flags swap as stale → ui redraws visible
lines.

Invariant: vim and ex never mutate the rope directly; every text change is an
`Edit` applied through `Buffer::apply`.

### Text model

- Rope stores text with `\n` line endings only. The original line-ending style
  (LF or CRLF, by majority) and whether the file had a final newline are
  stored on the buffer and restored on write. Mixed line endings: majority
  wins, status warns `mixed line endings; writing as CRLF|LF`.
- A buffer always has at least one line. Line numbers in the API are 0-based;
  user-facing numbers are 1-based.
- Cursor = (line, byte offset into the line), always on a grapheme boundary.
  Movement steps by grapheme cluster (`unicode-segmentation`); display
  columns use `unicode-width`; tabs expand to `tabstop`; other control chars
  render as `^X`.
- `Edit` = replace a char range with a string. `Buffer::apply` returns the
  inverse edit for undo.

### Undo

Linear stack of groups; a group is a list of edits plus cursor before/after.
New edit after undo truncates redo. Group boundaries: each Normal-mode command
(an entire insert session counts as part of the command that started it), each
ex command (a whole `g//` is one group).

## Inline window

- Window = `height` text rows + status row + command row. Clamped to
  `terminal_rows - 2`, and shrinks to fit when the file has fewer lines
  (minimum 1 text row).
- Opens below the prompt; the terminal scrolls if needed (ratatui
  `Viewport::Inline`).
- `scrolloff` = 2 lines (reduced if the window is too small).
- No wrap by default: horizontal scroll follows the cursor; lines cut off at
  the right edge end in `›`. With `wrap = true`, long lines soft-wrap.
- Gutter: right-aligned 1-based line numbers (relative optional).
- Status row: ` MODE  name [+]  filetype  line:col` (`[RO]` when read-only).
- Command row: `:` input, `/` `?` input, messages. Errors are `? message`.
- Exit: window erased, cursor restored to where the window started, so the
  shell prompt follows the `fred` command. If the file was written during the
  session, one line is printed: `"name" 212L, 6.1K written`.
- Resize: clear from window origin to end of screen, rebuild viewport, redraw.
- `Ctrl-Z`: restore terminal, `SIGTSTP`; on resume re-enter raw mode, rebuild
  viewport, redraw, and run change detection.

## vim engine

Grammar `[count] operator [count] motion | [count] command`.

- Modes: Normal, Insert, Visual-line (`V`).
- Motions: `h j k l`, `w b e W B E`, `0 ^ $`, `gg G {n}G`, `{ }`,
  `f F t T ; ,`, `/ ? n N`, arrows, Home, End, `'{a-z}`.
- Operators: `d c y` + motion; `dd cc yy`; `D C Y`. Linewise/charwise per
  motion type as in vim (`j k G gg { } '` linewise; others charwise; `e f t`
  inclusive).
- Commands: `x X s S r{c} J p P o O i a I A u Ctrl-R . m{a-z}`,
  `Ctrl-D Ctrl-U` (half window), `:`.
- Visual-line: motions extend selection; `d c y` operate on it; `:` pre-fills
  `'<,'>`; `Esc` cancels.
- Insert: text, Backspace (joins lines at col 0), Enter (copies indentation),
  Tab (inserts `\t`, or spaces if the file is detected as space-indented),
  arrows, `Esc` → Normal (cursor moves left one, vim style).
- `.` repeats the last change by replaying its recorded keys (including
  typed insert text).
- One unnamed register (text + linewise flag), shared by d/c/y/x/p.
- Unknown keys are ignored; pending state resets.

## ex engine

Grammar `[addr[,|; addr]] cmd [args]`.

- Addresses: `N`, `.`, `$`, `+n`/`-n` (offsets, also after another address),
  `/re/`, `?re?` (wrapping search), `'a`, `,` (= `1,$`), `;`.
  `'<` `'>` are the last visual-line selection.
- Commands:
  - `w [file]`, `w!`, `wq`, `x` (write if modified, then quit),
    `q` (fails if modified: `? unsaved changes (q! to discard)`), `q!`
  - `e[!] file` (fails if modified without `!`)
  - `s/re/rep/[g]` (default `.`), `&` and `\1`–`\9` in rep, `\n` in rep
    splits lines; empty `re` reuses last pattern
  - `d` (default `.`), `j` (default `.,.+1`), `m addr`, `t addr`
  - `g/re/cmd`, `v/re/cmd` (default `1,$`; cmd is any of the above
    line-editing commands; default cmd `p` is not supported — empty cmd is an
    error)
  - `N` alone moves the cursor to line N.
- Current line after each command follows ed (e.g. after `d`, the line after
  the deleted range, or the new last line).
- Regex: Rust `regex` crate syntax. Search in the window (`/`, `?`) and ex share
  the last pattern.
- Tab on the `:` line completes command names and paths after `w`, `e`.
- Command history: Up/Down on `:` and `/` lines (session only).

## Completion

- Sources, ranked in this order of preference:
  1. words in the current buffer (index updated per changed line), nearer the
     cursor first;
  2. words from nearby files: same directory, plus files with the same
     extension in the enclosing git repo; `.gitignore` respected
     (`ignore` crate); skip binary and >256 KB files; ≤200 files, ≤2 MB total;
     built on a background thread;
  3. paths: if the text before the cursor contains `/` or starts with `~`,
     directory entries matching the last component.
- Word = run of Unicode alphanumerics and `_`; indexed if ≥3 chars.
- Matching: prefix, smart case. Max 8 items. The word being typed is excluded.
- Auto popup after ≥2 word chars typed with ≥1 match. Nothing selected
  initially: Enter inserts a newline and closes the popup. `Tab`/`Ctrl-N`
  select next and insert it in place; `Shift-Tab`/`Ctrl-P` previous. Typing
  continues filtering. `Esc` closes the popup and leaves Insert mode.
  `Ctrl-N`/`Ctrl-P` open the popup manually when it is closed.
- Popup draws below the cursor row, or above if there is no room; inside the
  window; hidden if the window has fewer than 3 text rows.

## Highlighting

- syntect + two-face (bat's syntax and theme sets). Detection: extension,
  then first line (shebang/modeline), else plain text.
- Default theme `ansi` (uses the terminal's 16-color palette). Truecolor
  themes are converted to the nearest 256-color index when `COLORTERM` is not
  `truecolor`/`24bit`.
- Per-line cache of parse state; an edit invalidates from the first changed
  line. Highlighting runs only up to the last visible line with a ~20 ms budget
  per frame; unparsed lines render plain and are filled in on later ticks.
- Disabled (status message) for files >10 MB or any line >20,000 chars.

## Robustness

### Reading
- File read as bytes. Valid UTF-8 → normal. Invalid UTF-8 → opened read-only
  with lossy decoding, status `[RO] not valid UTF-8`.
- A leading UTF-8 BOM is kept and written back.
- Directories and unreadable files: startup error with a message, exit 1.
- Files without write permission open with `[RO]`; `w` fails with
  `? permission denied` (and `w!` does not change that).

### Safe write
1. Resolve symlinks; write to the real path.
2. Change check (below).
3. Write the full contents to `.<name>.fred~<pid>` in the same directory,
   `fsync`, copy the original's permission bits, `rename` over the target,
   `fsync` the directory.
4. If the temp file cannot be created (directory not writable) or the target
   has more than one hard link, fall back to an in-place write
   (truncate + write + `fsync`).
5. Record the new mtime/size/hash; mark buffer clean; update the swap file.

### Change detection
- At load and after each write, record (mtime, size, content hash).
- Before writing, stat the file; if mtime or size differ, re-hash; if the
  content differs: `? file changed on disk (w! to overwrite)`.
- Also checked on resume from `Ctrl-Z`; the status warns
  `file changed on disk` without blocking.

### Swap file
- Location: `$XDG_STATE_HOME/fred/swap/` (default `~/.local/state/fred/swap/`),
  name = absolute path with `%` escaping of `/` and `%`, plus `.swp`.
  Unnamed buffers use `unnamed-<pid>.swp`.
- Content: a header line of JSON (`magic`, `version`, `pid`, `host`, `path`,
  `saved_at`) then the raw buffer text.
- Written (atomically, temp + rename) when the buffer is modified and either
  1 s has passed since the last edit or 200 edits accumulated. Removed when the
  buffer becomes clean and on normal exit.
- On open, if a swap exists:
  - owner pid alive on this host → `swap: file is open in fred (pid N):
    [o]pen read-only, [q]uit`
  - otherwise → `swap found (saved Xm ago): [r]ecover, [d]elete, [q]uit`.
    Recover loads the swap text as the buffer contents, marked modified.
- Panic hook: restore the terminal, write the swap, print the panic message.
- `SIGHUP`/`SIGTERM`: write the swap, restore the terminal, exit 1.

## Error handling

- All user-facing errors in the editor go to the command row as `? message`;
  the editor never exits on an error other than at startup.
- Internal operations return `Result`; no `unwrap` on I/O paths.

## Testing

- Unit tests per module: buffer (edits, line endings, BOM, grapheme stepping),
  undo, ex (address parsing; command table tests: input text + command →
  expected text and current line), vim (key-string harness: `feed("3wdw")`
  then assert text + cursor + mode), completion (ranking, smart case),
  fileio (temp dirs: symlink, permissions, hard links, change detection),
  swap (round trip, stale vs live pid), highlight (detection, cache
  invalidation).
- UI: render the Editor into ratatui `TestBackend` and assert on rows.
- End-to-end: run the binary in a pseudo-terminal, send keys, assert the file
  on disk and that the process exits 0.

## Dependencies

`ropey`, `ratatui` (crossterm backend), `crossterm`, `syntect`, `two-face`,
`regex`, `unicode-segmentation`, `unicode-width`, `ignore`, `serde` + `toml` +
`serde_json`, `signal-hook`, `libc`, `anyhow`. Dev: `tempfile`, `portable-pty`.

## As built: changes made during review (2026-10-01)

A code review, a randomized test harness and real-terminal testing led to
these deliberate changes from the design above:

- **Swap file is a lock for the whole session.** It is created at open (a
  "clean" marker while nothing is unsaved) and removed on normal exit, so a
  second fred is always warned and offered read-only. fred only overwrites or
  removes a swap file it wrote. `:e` into a file with a swap asks the same
  recover / delete / read-only question as startup (`q` cancels the `:e`).
  A clean lock or an identical copy left by a dead fred is removed silently.
- **ex patterns match case exactly** (`:s`, `:g`, `:v`, `/re/` addresses), as
  in ed, so a substitution never changes text you didn't spell; `(?i)`
  ignores case. `/`, `?`, `n`, `N` keep smart case.
- **Additions:** `%` range; `:e` / `:e!` with no file reloads the current
  file; `ZZ` / `ZQ`; `\r` in a replacement splits the line (vim's idiom);
  operators take search motions (`d/pat`); `:w` with another path to the
  same file saves the buffer; bracketed paste inserts text verbatim.
- **Marks follow their lines** through edits; deleting a marked line deletes
  the mark. `:g/re/s/x/y/` substitutes where it can and errors only if
  nothing matched. A failed ex command rolls back without touching redo.
- **Tab** inserts a tab unless the file is space-indented; the space width
  (2, 4, …) is detected.
- **Window** starts as small as the file and grows as lines are added (up to
  `height`); it never shrinks during a session.
- **Input:** Alt+key is read as Esc then key (an Esc batched with the next
  key over SSH/tmux arrives as Alt+key). Known limitation: `Esc O <letter>`
  in one read is parsed by crossterm as a function-key sequence.
- **Text model:** only `\n` splits lines (ropey's `unicode_lines` is off);
  only a zero-byte file gains a final newline, and only while it has text.
- **Writes keep the file's group** (or fall back to writing in place).
- **Terminal:** Ctrl-Z stops the whole process group (fred as `$EDITOR`);
  any panic in the event loop writes the swap, restores the terminal and
  prints its message; a startup failure restores the terminal; windows
  under 3 rows tall don't crash.
- **Performance:** fred draws only when something changed (idle CPU ≈ 0,
  nothing sent to the terminal); lines are laid out only where visible, and
  printable-ASCII lines use column arithmetic, so a 1 MB one-line file stays
  responsive. Wrap mode scrolls by screen rows within long lines.

Deferred (known, minor): 2-row terminals show no text rows; the wrap-mode
window is sized by logical lines; terminals disagree with fred on some
ZWJ/skin-tone emoji widths; `.` doesn't repeat a bracketed paste; the
read-only message on permission-denied files suggests `w!`; a dangling
symlink is replaced by a regular file on write; `:w name` on an unnamed
buffer doesn't re-detect the filetype until restart; Esc keeps an untouched
auto-indent; a reused pid can hide [r]ecover; `//` lists `/` in completion;
the cursor can sit on the second half of a wide char in wrap mode near a
row end only if the terminal disagrees on its width.
