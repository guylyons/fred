# fred

A text editor for quick edits that lives in your shell prompt. Like `ed`, it
doesn't take over the screen: it opens a small window right under the prompt.
Inside that window you move and edit with vim keys, and `:` takes ed-style
commands with full ed addresses. Completion and syntax highlighting are built in.

```
$ fred main.rs
  3 fn main() {
  4     let greeting = "hello";
  5     let x = gre
  6     println greeting     {greeting_len}");
  7 }           greeting_len
 INSERT  main.rs [+]                                   rust  5:16
```

When you quit, the window disappears and your scrollback is untouched. If you
saved, fred leaves one line behind:

```
$ fred main.rs
"main.rs" 8L, 154B written
$
```

## Install

```
cargo install --path .
```

## Usage

```
fred [--height N] [+LINE] [FILE]
```

- `FILE` doesn't have to exist; it's created on the first `:w`.
- `+LINE` starts on that line; `+` alone starts on the last line.
- `--height N` shows N lines of text (default 12); `--height max` uses the
  whole terminal, leaving your prompt visible. The window starts as small
  as the file and grows as the file gets longer.

## Keys

**Moving:** `h j k l`, `w b e W B E`, `0 ^ $`, `gg G` / `{n}G`, `{ }`,
`f F t T ; ,`, `/` `?` `n N`, `'a` (go to mark), `Ctrl-D Ctrl-U` (half a window),
`Ctrl-F Ctrl-B`, arrow keys, Home and End. Counts work: `3w`, `5j`.

**Editing:** `d c y` combined with any motion (including searches: `d/foo<Enter>`), plus `dd cc yy D C Y`,
`x X s S r{c} J`, `p P`, `o O i a I A`, `u` (undo) and `Ctrl-R` (redo), `.`
(repeat the last change), `m{a-z}` (set a mark), `V` (visual-line mode, then
`d c y J :`).

**In Insert mode:** completions pop up as you type. `Tab`/`Ctrl-N` selects the
next suggestion and `Shift-Tab`/`Ctrl-P` the previous one. `Enter` accepts a
selected suggestion, or starts a new line when nothing is selected. `Esc`
closes the popup and leaves Insert mode. `Ctrl-W` deletes the word before the
cursor and `Ctrl-U` deletes to the start of the line. Pasted text goes in
exactly as pasted.

`ZZ` saves (if there are changes) and quits; `ZQ` quits without saving.
`Ctrl-Z` suspends fred; `fg` brings it back.

## `:` commands

Addresses work as in ed: `N . $ +n -n /re/ ?re? 'a`, `,` or `%` (the whole
file) and `;`. In visual-line mode, `:` starts with `'<,'>` filled in. Marks
follow their lines as you edit.

| command | does |
|---|---|
| `w [file]`, `w!` | write (`w!` overrides change detection) |
| `wq`, `x` | write and quit (`x` writes only if there are changes) |
| `q`, `q!` | quit (`q!` discards changes) |
| `e[!] [file]` | edit another file; with no file, reload this one (`e!` discards changes) |
| `[range]s/re/rep/[g]` | substitute; `&` and `\1`–`\9` in `rep`, `\n` or `\r` splits the line |
| `[range]d` | delete lines |
| `[range]j` | join lines |
| `[range]m addr`, `[range]t addr` | move, copy (`0` means before the first line) |
| `[range]g/re/cmd`, `v/re/cmd` | run `cmd` on matching / non-matching lines |
| `N` | go to line N |
| `[range]w file` | write just those lines to another file |

Patterns use Rust's [`regex`](https://docs.rs/regex) syntax, which is like
extended regular expressions (ERE). `/` and `?` searches ignore case unless
the pattern has a capital letter. `:` commands match case exactly, as in ed,
so a substitution never changes text you didn't spell; add `(?i)` to ignore
case. `Tab` completes command names and file paths, and
`Up`/`Down` go through the command history.

Each `:` command is a single undo step, even a `g` that changes 500 lines.

## Completion

Suggestions come from three places:

1. Words in the file you're editing, closest to the cursor first.
2. Words in nearby files: the same directory, plus files with the same
   extension in the git repo. `.gitignore` is respected, and binary or large
   files are skipped. This runs in the background, so start-up never waits for it.
3. File paths, once the text before the cursor contains `/` or starts with `~`.

## Highlighting

Highlighting uses [syntect](https://github.com/trishume/syntect) with
[bat](https://github.com/sharkdp/bat)'s language definitions. The default
`ansi` theme uses your terminal's own 16 colors, so it matches your
terminal's color scheme. It turns off for files over 10 MB or lines longer
than 20,000 characters.

## Your files are safe

- **Writes are atomic.** fred writes a temporary file in the same directory,
  syncs it to disk, and renames it over the original, keeping the original's
  permissions. Symlinks are followed, and hard-linked files or files owned by
  someone else are written in place so the links and the owner are kept.
- **Changes by others are detected.** If the file changed on disk since you
  opened it, `:w` refuses and asks you to use `:w!`.
- **Crash recovery.** While a file is open, fred keeps a swap file for it in
  `~/.local/state/fred/swap/` (or `$XDG_STATE_HOME/fred/swap/`). Unsaved changes
  are written there after you pause for a second or after 200 edits, and also
  if the terminal closes, fred is killed, or it crashes. The next time you
  open the file (at startup or with `:e`), fred offers to **r**ecover or
  **d**elete them. If the file is already open in another fred, it offers a
  read-only view instead, and the two never touch each other's swap file.
- **Formats are preserved.** CRLF line endings, a missing final newline, and a
  UTF-8 byte-order mark are all kept as they were. Files that aren't valid
  UTF-8 open read-only and fred never writes them.

## Config

`~/.config/fred/config.toml` (or `$XDG_CONFIG_HOME/fred/config.toml`). Every
key is optional:

```toml
height = 12              # lines of text in the window, or "max"
wrap = false             # wrap long lines (otherwise the view scrolls sideways)
numbers = true           # line numbers
relative_numbers = false
theme = "ansi"           # any bat theme name, e.g. "Nord", "Dracula", "gruvbox-dark"
tabstop = 8
autocomplete = true      # pop up completions while typing (Ctrl-N works either way)
```

## Development

```
cargo test            # unit, randomized (fuzz) and pseudo-terminal tests
cargo test --test e2e show_screen -- --ignored --nocapture   # print real screens
FRED_FUZZ_ITERS=400000 FRED_FUZZ_THREADS=8 cargo test --release --test fuzz
cargo build --release && cargo test --test explore perf_ -- --ignored --nocapture --test-threads=1
```

`tests/fuzz.rs` feeds generated buffers and key sequences to the editor and
checks the cursor, undo/redo and file-format invariants after every key;
`tests/explore.rs` drives the real binary in a pseudo-terminal (suspend,
resizes, two freds on one file, long lines, wrap mode, performance).

The design spec is in `docs/superpowers/specs/2026-10-01-fred-design.md`.
