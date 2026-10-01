# fred — file and grep pickers

Date: 2026-10-01. Status: approved in brainstorming.

## Purpose

Find a file or grep the project without leaving fred, in the spirit of
[fff](https://github.com/dmtrKovalenko/fff): fuzzy file finding that favors
files you used recently, and live regex grep. `Space p` finds files and
`Space g` greps; picking a result opens it in the same inline window.

fff itself is not embedded. It pulls in libgit2, LMDB, a file watcher, rayon
and tracing, and its speed comes from an index kept in a long-lived process,
which fred (started per edit) does not have. fred builds the same behavior
on what it already uses (`ignore`, `regex`, threads) plus one small crate,
`nucleo-matcher` (Helix's fuzzy scorer).

Success: `Space p`, a few letters, `Enter` opens the right file; `Space g`,
a pattern, `Enter` lands on the match; startup time is unchanged.

## Scope

In: file picker, live grep picker, a recent-files list, the Space leader.

Out: preview pane, git status in results, typo-tolerant or fuzzy grep,
frecency beyond recency, multi-select, opening in splits (fred has one
buffer), user-configurable leader or bindings.

## Behavior

### Keys

- Space is the leader in Normal mode. It no longer moves right (`l` does).
- `Space p` opens the file picker; `Space g` opens the grep picker. Space
  followed by any other key does nothing. There is no timeout.
- In a picker: typed text edits the query; `Ctrl-N`/`Down` and
  `Ctrl-P`/`Up` move the selection; `Enter` opens the selection; `Esc` (or
  `Ctrl-C`) closes the picker and returns to the file exactly as it was.
  `Backspace`, `Ctrl-W`, `Ctrl-U`, `Left`/`Right` edit the query as on the
  `:` line.

### Window

- The picker uses the window's text rows for results and the command line
  for the prompt (`find> ` or `grep> `). It uses whatever height the window
  has; with `height = "max"` that is most of the screen.
- The best result is on the bottom row, next to the prompt, and the list
  grows upward (fzf order). The selection starts on the best result.
- The status line shows the picker kind and a count (`files 1234/5678`,
  `grep 42 matches`, `1000+ matches`, `(200000+ files, list truncated)`).

### File picker

- Candidates are the project's files: the enclosing git repo, else the
  current directory. `.gitignore` and `.ignore` are honored (the `ignore`
  crate with `require_git(false)`, as in `nearby.rs`). Paths are shown
  relative to the project root.
- Matching is fuzzy (nucleo). Smart case: a capital letter in the query makes
  it case-sensitive. Matched characters are highlighted.
- Empty query: recently opened files that are in this project, most recent
  first, then the remaining files in walk order. With a query, ties in score
  are broken by recency, then by shorter path.
- `Enter` opens the file at its first line.

### Grep picker

- The query is a regex with the `/` rules: Rust `regex` syntax, smart case
  (`search::compile`). Matching is per line.
- Results update 100 ms after the last keystroke. A row is
  `path:line: text`, text trimmed of leading whitespace and cut to the window
  width, the match highlighted.
- Skipped: binary files (NUL in the first 8 KB, as in `nearby.rs`), files
  over 1 MB, unreadable files. At most 1000 matches.
- An invalid regex shows `bad pattern: …` on the status line and keeps the
  previous results. An empty query shows nothing.
- `Enter` opens the file with the cursor on the match, and sets the query as
  the last search pattern (forward), so `n`/`N` continue from there.

### Opening a result

- It follows `:e`: with unsaved changes fred refuses with
  `unsaved changes (:w first)` and the picker stays open with its query.
  Swap-file recovery prompts appear as they do for `:e`.
- A file that can no longer be opened reports the error as `:e` does and
  closes the picker.
- Opening any file (from the command line, `:e`, or a picker) records it in
  the recent-files list.

## Design

### New module `src/pick/` (terminal-free)

| File | Role |
|---|---|
| `files.rs` | Project root (reuse `repo_root` from `complete/nearby.rs`, made `pub(crate)`). Background walk into a shared, growing `Vec<PathBuf>` (relative), sorted by file name; capped at 200,000 with a `truncated` flag; a `done` flag. Started on first `Space p`/`Space g`, reused for the rest of the session. |
| `fuzzy.rs` | `rank(query, files, recent, limit) -> Vec<(index, score, Vec<char positions>)>` over nucleo-matcher. |
| `grep.rs` | `Grep` handle: `start(regex, files)` bumps a generation counter and spawns a thread that scans files in order and pushes `Hit { path, line, col, len, text }` into shared results, checking the generation between files to stop early; cap 1000. |
| `recent.rs` | `~/.local/state/fred/recent` (same base as the swap dir): absolute paths, newest first, deduplicated, at most 200. `load()`, `record(path)`. Read and write errors are ignored. |
| `mod.rs` | `Picker { kind: Files \| Grep, query: CmdLine, items, sel, last_query_change }` and `pick_key(ed, key)`. |

### Changes to existing code

- `editor.rs`: `Mode::Pick(Picker)`; `handle_key` dispatches to
  `pick::pick_key`. The editor owns the shared file list and grep handle
  (like `nearby`).
- `vim/mod.rs`: Space becomes a pending prefix (`' '` removed from
  `Motion::Right`); `Space p` / `Space g` open a picker.
- `ex`: new `ExEffect::Open { path, line, col, pattern: Option<String> }`.
- `session.rs`: `perform` handles `Open` through `edit()` (unsaved check,
  swap recovery), then moves the cursor and sets `last_pat`. On refusal the
  error is shown and the editor stays in `Mode::Pick`. `open_file` and
  successful `edit` call `recent::record`.
- `ui/render.rs`: in `Mode::Pick`, draw the result rows in the text area
  (bottom-up), the prompt on the command line, the count on the status line.
- `app.rs`: each tick, if the picker's results changed (file walk grew, grep
  results arrived, the 100 ms debounce expired and a grep must start), mark
  the frame dirty. Same pattern as `hl.incomplete()`. Startup does nothing
  new.
- `README.md`: keys section (Space leader, pickers).

### Dependency

`nucleo-matcher = "0.3"`.

## Errors and limits

- Walk capped at 200,000 files (`list truncated` shown); so a stray
  `Space p` in `~` does not hang.
- Grep capped at 1000 matches; per-file limits above.
- No filesystem watcher: the file list is built once per session. A file
  created after the first `Space p` is not listed until fred restarts.
  (ponytail: rebuild on each picker open if this turns out to matter.)

## Testing

- Unit (`src/pick/`, temp dirs as in `nearby.rs`): fuzzy ranking
  (`sesrs` ranks `src/session.rs` first; recency breaks ties), ignore rules,
  grep hits and cap, a new generation stops a stale grep, recent list
  ordering, dedup and trim to 200, unreadable state file ignored.
- Editor: Space sequences (`Space p`, `Space g`, `Space x` does nothing),
  picker keys, `Esc` leaves buffer, cursor and mode unchanged, `Enter`
  yields `ExEffect::Open`.
- Session: `Open` moves to the line and sets the search pattern; unsaved
  changes refuse and keep the picker and query.
- e2e (pty): `Space p`, type, `Enter` opens the file; `Space g`, type,
  `Enter` opens it on the right line.
- Perf: the existing perf suite shows no startup change; time to first
  `Space p` list and first grep results on this repo and a large one.

## As built

Differences from the design above, found while building:

- While a picker is open the window grows to the configured height (it
  would otherwise be as short as the file: one result for a one-line file).
  Like any growth, it does not shrink back.
- A picked file is opened by its path relative to the current directory
  when it is inside it, so the status line shows `src/app.rs`, not an
  absolute path.
- Errors from opening a pick (unsaved changes) show on the picker's status
  line in place of the count, until the next key.
- Measured (release, 20,000-file project, `perf_pickers` in
  `tests/explore.rs`): picker shows in ~2 ms, all files listed in ~55 ms,
  ranking after typing ~25 ms, a full grep scan for a rare word ~0.9 s,
  1000 hits for a common one ~0.16 s. Startup is unchanged (~15 ms).

## Later additions

- `Space k` (`Kind::Lines`, `src/pick/lines.rs`): swiper-style search of
  the current buffer. Space-separated words, each a smart-case regex, must
  all match; results in file order (shown top to bottom), selection starting
  at the first match at or after the cursor. `Enter` moves the cursor (no
  file is reopened) and makes the first word the last search. Rescans the
  buffer per keystroke: ~30 ms at 100k lines.
- fred now runs fullscreen (alternate screen) by default: `fullscreen = true`
  in the config. `--inline`, `fullscreen = false`, or `--height N` give the
  inline window. `tests/explore.rs` runs with `fullscreen = false`.
