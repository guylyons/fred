<p align="center"><img src="docs/dr-fred-head.png" alt="Dr. Fred" width="160"></p>
<p align="center">
  <img src="https://img.shields.io/badge/Rust-2024_edition-b7410e?logo=rust&logoColor=white" alt="Rust 2024 edition">
  <a href="https://ratatui.rs"><img src="https://img.shields.io/badge/Built_With_Ratatui-000?logo=ratatui&logoColor=fff" alt="Built with Ratatui"></a>
  <img src="https://img.shields.io/badge/platform-macOS_%7C_Linux-555" alt="macOS | Linux">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
  <a href="https://github.com/guylyons/fred/commits/main"><img src="https://img.shields.io/github/last-commit/guylyons/fred" alt="Last commit"></a>
</p>

# fred

`fred` started from wanting to make my own verson of `ed`.

Use `fred -i README.md` to quickly inline edit in place. Think of it as the `ed` that
Dr. Fred himself would have wanted.

A text editor for quick edits. You move and edit with vim keys, and `:` takes
ed-style commands with full ed addresses. Completion, syntax highlighting, a
fuzzy file finder and project grep are built in.

fred runs fullscreen, like vim. Like `ed`, it can instead live in your shell
prompt (`-i`, or `fullscreen = false` in the config): a small window right
under the prompt:

```
$ fred main.rs
  3 fn main() {
  4     let greeting = "hello";
  5     let x = gre
  6     println greeting     {greeting_len}");
  7 }           greeting_len
 INSERT  main.rs [+]                                   rust  5:16
```

When you quit, either way, the screen is back as it was and your scrollback is
untouched. If you saved, fred leaves one line behind:

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
fred [-i | -f] [--height N] [+LINE] [FILE | DIR]
```

- `FILE` doesn't have to exist; it's created on the first `:w`.
- `+LINE` starts on that line; `+` alone starts on the last line.
- `DIR` (say `fred .`) lists that directory (dired) instead of a file.
- `-i`/`--inline` opens the window under your prompt; `-f`/`--fullscreen`
  takes over the terminal even if the config says otherwise.
- A file you've opened before opens where you left the cursor (unless you
  give `+LINE`), as do files picked with `Space p`, `Space r` or `Space j`.
- `--height N` (inline) shows N lines of text (default 12); `--height max`
  uses the whole terminal, leaving your prompt visible. The window starts as
  small as the file and grows as the file gets longer.

## Keys

**Moving:** `h j k l`, `w b e W B E`, `0 ^ $`, `gg G` / `{n}G`, `{ }`,
`f F t T ; ,`, `/` `?` `n N`, `'a` (go to mark), `Ctrl-D Ctrl-U` (half a window),
`Ctrl-F Ctrl-B`, arrow keys, Home and End. Counts work: `3w`, `5j`.

**Editing:** `d c y` combined with any motion (including searches: `d/foo<Enter>`), plus `dd cc yy D C Y`,
`x X s S r{c} J`, `p P`, `o O i a I A`, `u` (undo) and `Ctrl-R` (redo), `.`
(repeat the last change), `m{a-z}` (set a mark), `V` (visual-line mode, then
`d c y J :`).
Yanks and deletes go to the system clipboard too, and `p`/`P` put what
another app copied since (`clipboard = false` keeps them to fred).

**In Insert mode:** completions pop up as you type. `Tab`/`Ctrl-N` selects the
next suggestion and `Shift-Tab`/`Ctrl-P` the previous one. `Enter` accepts a
selected suggestion, or starts a new line when nothing is selected. `Esc`
closes the popup and leaves Insert mode; so does typing `jj`. `Ctrl-W` deletes the word before the
cursor and `Ctrl-U` deletes to the start of the line. Pasted text goes in
exactly as pasted.

`Ctrl-G` cancels, like `Esc`: a picker, the `:` line, a half-typed command.

`ZZ` saves (if there are changes) and quits; `ZQ` quits without saving.
`Space Enter` saves (`:w`) and `Space ;` closes (`:q`).
`Ctrl-Z` suspends fred; `fg` brings it back.

## Finding files and grepping

Space is the leader key (`l` still moves right).

- `Space s` zaps to a visible word. Type its prefix to highlight matches;
  a unique exact word jumps straight to its start. Otherwise type the letter
  label shown at the word. Labels avoid letters that can continue your search;
  `Enter` switches to label selection if all letters are needed for searching.
  Larger sets use multiple letters, showing the remaining keys as you select.
  `Backspace` shortens the search and `Esc`/`Ctrl-G` cancels. Like `/`, matching
  ignores case unless you type a capital. Only whole words visible in the text
  area participate, including wrapped rows and horizontally scrolled text.
- `Space p` finds a file. Type a few letters of its path in any order that
  follows the path: `sesrs` finds `src/session.rs`. With nothing typed you
  see the files you opened most recently first.
- `Space g` greps the project as you type, with the same regex rules as `/`.
  `Enter` opens the file on the match, and `n` goes on to the next one.
- Both skip what `.gitignore` ignores. `Ctrl-o` in either one also searches
  ignored files (Drupal core, `vendor/`), marked `[all]`, until you press it
  again.
- `Space d` (or `gd`) goes to the definition of the word under the cursor,
  in any language, without a language server: it greps the project for
  lines that define it (`fn`, `def`, `class`, `func`, `const`, `let`, JS
  arrow functions, C-style declarations, `#define`, shell functions,
  Makefile targets, CSS selectors, …). Keyword definitions rank above
  variables, this file above others, and files of the same type above the
  rest. When one stands out it jumps straight there; otherwise pick from the
  list (edit the name to search for another).
- `Space j` browses files like Emacs's find-file with vertico and consult:
  the prompt is a path, and what you type after the last `/` fuzzy-matches
  every file below that directory (plus its subdirectories, to go into),
  honoring `.gitignore`. With nothing typed you see the directory itself.
  `Tab` on a directory goes in and `Enter` lists it (dired), `Backspace`
  after a `/` goes up a directory, `~/` jumps home, and `Enter` on a name
  that doesn't exist starts a new file. It starts in the current file's
  directory. Dotfiles show once you type a `.`.
- `Space b` lists the open buffers (like `:ls`); type to filter. `Ctrl-^`
  (`Ctrl-6`) goes straight back to the one you were in before.
- `Space B` searches the lines of every open buffer, like `Space k`: this
  buffer's matches nearest the prompt, then the others'. `Enter` goes to
  that buffer and line.
- `Space r` lists recently opened files (any project), newest first;
  type to filter.
- `Space k` searches the lines of the file you're editing, like swiper:
  every space-separated word must appear in the line, in any order (each is
  a regex). Matches are listed in file order, starting at the cursor; `Enter`
  jumps there and `n` finds the next line with the first word.

Pickers open as a panel at the bottom, only as tall as their results (up
to half the screen); the file stays where it was above. The status line sits above the
prompt, with candidates listed downward beneath it, best match first;
`Up`/`Ctrl-P` and `Down`/`Ctrl-N` move the selection, `Enter` opens it and
`Esc` goes back. The project is the enclosing git repo (or the current
directory), `.gitignore` is respected, and grep skips binary files and files
over 1 MB. Opening another file keeps this one open as a buffer, unsaved
changes and all.

## Dired

A directory opens as a listing, as in Emacs's dired: `fred DIR`, `:e DIR`,
`Enter` on a directory in `Space j`, or `Space -` for the current file's
directory (cursor on the file). It's a read-only buffer, so every vim motion,
count and search moves around it; these keys act on the entry under the
cursor, on a `V` range, or on the `*` marked entries:

| Key | |
|---|---|
| `Enter` | open a file (as a buffer) or go into a directory |
| `-`, `^` | up to the parent directory |
| `m`, `u`, `U`, `t` | mark, unmark, unmark all, toggle marks |
| `%` | mark names matching a regexp |
| `d`, `x` | flag for deletion; delete the flagged (asks first) |
| `D` | delete (asks first; directories recursively) |
| `R`, `C` | rename/move, copy (`cp -R`); several marked go into a directory. Never over an existing file |
| `+` | create a directory (`a/b/c` makes them all) |
| `M`, `T` | chmod (octal), touch |
| `!` | run a shell command on the files, in the directory (`*` stands for them) |
| `s`, `(`, `gh` | sort by name/time, hide details, hide dotfiles |
| `gr` | re-read the directory |
| `i` | edit the names in place (wdired); `:w` renames them all, `gr` discards |

## `:` commands

Addresses work as in ed: `N . $ +n -n /re/ ?re? 'a`, `,` or `%` (the whole
file) and `;`. In visual-line mode, `:` starts with `'<,'>` filled in. Marks
follow their lines as you edit.

| command | does |
|---|---|
| `w [file]`, `w!` | write (`w!` overrides change detection) |
| `wq`, `x` | write and quit (`x` writes only if there are changes) |
| `q`, `q!` | quit; `q` refuses while any buffer has unsaved changes, `q!` discards them |
| `e[!] [file]` | edit another file (this one stays open as a buffer); with no file, reload this one. `e!` discards this file's changes |
| `b N`, `b name`, `b#` | go to buffer N, the one whose name contains `name`, or the one before (`b2` works too) |
| `bn`, `bp` | next, previous buffer |
| `bd[!] [N]` | close a buffer (`bd!` discards its changes) |
| `ls` | pick a buffer, the one you were in last first |
| `[range]s/re/rep/[g]` | substitute; `&` and `\1`–`\9` in `rep`, `\n` or `\r` splits the line |
| `[range]d` | delete lines |
| `[range]j` | join lines |
| `[range]m addr`, `[range]t addr` | move, copy (`0` means before the first line) |
| `[range]g/re/cmd`, `v/re/cmd` | run `cmd` on matching / non-matching lines |
| `N` | go to line N |
| `[range]w file` | write just those lines to another file |
| `!cmd` | run a shell command (`$SHELL`), then press Enter to come back |
| `[range]!cmd` | filter lines through a command: `%!sort`, `'<,'>!column -t` |
| `[range]ai ask` | have Claude rewrite the lines (it sees the whole file): `'<,'>ai make this async`. A spinner holds the editor until it answers. See `ai_command` and `ai_rules` below |
| `r file`, `r !cmd` | insert a file, or a command's output, below the line |
| `pwd`, `cd [dir]` | show or change the directory (`cd` alone goes home); `Space p` and `Space g` follow it |

Patterns use Rust's [`regex`](https://docs.rs/regex) syntax, which is like
extended regular expressions (ERE). `/` and `?` searches ignore case unless
the pattern has a capital letter. `:` commands match case exactly, as in ed,
so a substitution never changes text you didn't spell; add `(?i)` to ignore
case. `Tab` shows command-name and file-path candidates above the command
line and cycles forward; `Shift-Tab` cycles backward. `Enter` runs the completed
command. The first `Esc` dismisses the candidate menu; a second leaves the command
line. `Up`/`Down` go through the command history.

In shell commands `%` is the current file's name (`\%` for a literal `%`).

Each `:` command is a single undo step, even a `g` that changes 500 lines.

## Completion

Suggestions come from three places:

1. Words in the file you're editing, closest to the cursor first.
2. Words in nearby files: the same directory, plus files with the same
   extension in the git repo. `.gitignore` is respected, and binary or large
   files are skipped. This runs in the background, so start-up never waits for it.
3. File paths, once the text before the cursor contains `/` or starts with `~`.

## Git

In a git repository, lines that differ from the staging area (what `git
diff` shows) are marked in the gutter as you type: a green bar for added
lines, yellow for changed ones, and a red mark where lines were removed.
The line number takes the same color. Staging a file elsewhere (`git add`)
shows up the next time you open it.

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
fullscreen = true        # false: inline; "auto": inline unless the file is
                         # longer than the inline window
height = 12              # inline: lines of text in the window, or "max"
wrap = false             # wrap long lines (otherwise the view scrolls sideways)
numbers = true           # line numbers
relative_numbers = false
theme = "ansi"           # any bat theme name, e.g. "Nord", "Dracula", "gruvbox-dark"
tabstop = 8
autocomplete = true      # pop up completions while typing (Ctrl-N works either way)
icons = false            # file-type icons in the file pickers (needs a Nerd Font)
clipboard = true         # yank, delete and put use the system clipboard
ai_command = "claude -p --tools '' --safe-mode"  # what :ai runs (prompt on stdin);
                         # --safe-mode skips your Claude plugins, hooks and
                         # CLAUDE.md: faster, and far fewer tokens
ai_rules = ""            # extra instructions for every :ai, e.g.
                         # "Smallest change that works. No new abstractions."
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

## License

MIT. See [LICENSE](LICENSE).
