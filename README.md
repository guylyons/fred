<p align="center"><img src="docs/dr-fred-head.png" alt="Dr. Fred" width="160"></p>

<h1 align="center">fred</h1>

<p align="center">
  <b>A fast, small terminal editor: Vim keys, ed commands, and Git, Org mode and fuzzy finding built in.</b>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2024_edition-b7410e?logo=rust&logoColor=white" alt="Rust 2024 edition">
  <a href="https://ratatui.rs"><img src="https://img.shields.io/badge/Built_With_Ratatui-000?logo=ratatui&logoColor=fff" alt="Built with Ratatui"></a>
  <img src="https://img.shields.io/badge/platform-macOS_%7C_Linux-555" alt="macOS | Linux">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
  <a href="https://github.com/guylyons/fred/commits/main"><img src="https://img.shields.io/github/last-commit/guylyons/fred" alt="Last commit"></a>
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#performance">Performance</a> ·
  <a href="#keys">Keys</a> ·
  <a href="#finding-files-and-text">Finding things</a> ·
  <a href="#git">Git</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#roadmap">Roadmap</a>
</p>

fred began as my own take on `ed`, the original Unix editor, and grew into
an everyday editor. You move and edit with Vim keys, and `:` takes ed's
commands with all of its line addresses. The things other editors need
plugins for are already inside: a fuzzy file finder, project grep,
completion, a Git client modeled on Emacs's Magit, and Org mode. It is one
8.6 MB binary with nothing else to install or configure.

## Highlights

- **Vim keys, ed commands.** Normal, Insert, Visual and Visual-line modes;
  `:s`, `:g`, `:m`, `:t` with ranges and marks; `:set` for Neovim's options.
- **Fast and light.** Opening a file and quitting takes a fraction of the CPU
  Neovim needs (as little as a quarter), and about half the memory. Sitting idle, fred uses
  almost no CPU. [See the numbers.](#performance)
- **Inline or fullscreen.** `fred -i` opens a small window under your shell
  prompt and leaves your scrollback as it was.
- **Finds things.** A file finder that forgives typos, grep across the project
  as you type, and go to definition in any language without a language server.
- **Git built in.** Changed lines are marked in the gutter as you type, and
  `Space m` opens Magit's status, staging, commits, branches, stashes, rebases,
  log and blame.
- **Org mode.** Folding, TODO states, agenda, capture and clocking in `.org`
  files.
- **Careful with your files.** Atomic writes, a warning when the file changed
  on disk, crash recovery, and line endings and encodings kept as they were.
- **Claude when you want it.** `Space c c` chats with Claude Code in a buffer, and `Space c` drives it like Magit drives git;
  `:ai` rewrites the selected lines; `K` on a selection explains it.

```
$ fred main.rs
  3 fn main() {
  4     let greeting = "hello";
  5     let x = gre
  6     println greeting     {greeting_len}");
  7 }           greeting_len
 INSERT  main.rs [+]                                   rust  5:16
```

When you quit, the screen is back as it was. If you saved, fred leaves one
line behind:

```
$ fred main.rs
"main.rs" 8L, 154B written
$
```

## Install

You need [Rust](https://rustup.rs) 1.88 or newer, on macOS or Linux.

```sh
cargo install --git https://github.com/guylyons/fred
```

Or from a clone: `cargo install --path .`

With the Ghostty terminal (`:terminal`, `Space t`), which needs
[Zig](https://ziglang.org) 0.15 to build Ghostty's terminal library:

```sh
brew install zig@0.15
PATH="$(brew --prefix zig@0.15)/bin:$PATH" cargo install --path . --features ghostty
```

Optional, for the features that use them:

| For | Install |
|---|---|
| Git gutter and `Space m` | `git` |
| System clipboard | nothing on macOS; `wl-clipboard` (Wayland) or `xclip` (X11) on Linux |
| `:ai` and `:explain` | the [Claude Code](https://claude.com/claude-code) CLI (`claude`) |
| File icons in pickers | a [Nerd Font](https://www.nerdfonts.com), then `icons = true` |

## Quick start

```sh
fred notes.md           # edit a file (a new one is created on the first :w)
fred -i notes.md        # edit in a small window under your prompt
fred +42 src/main.rs    # start on line 42 (+ alone: the last line)
fred .                  # browse a directory
```

Inside, press `i` to type and `Esc` to stop. `:w` saves and `:q` quits (`ZZ`
does both). `Space p` finds a file, `Space g` searches the project and
`Space m s` shows Git status. Files you open again start where you left the
cursor.

## Performance

fred aims to start instantly and cost nothing while it waits for you.
Opening a file and quitting straight away:

| | fred | Neovim |
|---|---:|---:|
| 460-line Markdown file | **22 ms** | 92 ms |
| the same, inside a git repository | **32 ms** | 92 ms |
| 5,900-line Rust file | **20 ms** | 64 ms |
| the same, inside a git repository | **38 ms** | 65 ms |
| open the Rust file, wait 6 seconds, quit | **31 ms** | 68 ms |
| peak memory | **10–11 MB** | 17–20 MB |
| install size | **8.6 MB**, one file | 33 MB |

Times are CPU time (user plus system), including every process the editor
starts: Neovim's server process, and the `git` fred runs inside a repository
to mark changed lines. Medians of 15 runs on an Apple M5 with macOS 27,
against Neovim 0.12.6 started with `--clean` (no config or plugins). To
measure on your machine:

```sh
tools/bench.py fred FILE
tools/bench.py nvim --clean FILE
tools/bench.py --wait 6 fred FILE
```

How fred stays light: words for completion from nearby files are read the
first time you type, not at startup; the branch name is read from `.git`
directly, and outside a repository fred starts no `git` at all; and after two
seconds without input it checks for background work four times a second
instead of twenty. Inside a repository, most of the extra time is the one
`git show` that fetches the staged file for the gutter.

## Native macOS app (preview)

`macos/` holds a native Mac front end: a Swift app that draws Fred's windows
with Core Text and per-window status bars, and runs `fred --gui` as its helper
process. It needs macOS 26 and the Swift toolchain, plus Zig 0.15 for the
Ghostty build.

```sh
brew install zig@0.15
./macos/build.sh                      # builds the core and macos/Fred.app
open macos/Fred.app --args "$PWD/file"  # an absolute path (the app starts in /): a file or directory; omit it for an open panel
```

`open` does not pass your shell environment. To set the variables below, run
the binary directly: `FRED_LATENCY=1 macos/Fred.app/Contents/MacOS/Fred PATH`.

- `FRED_CORE=/path/to/fred` runs a different core binary than the bundled one.
- `FRED_LATENCY=1` shows keypress-to-pixels p50/p99 in an overlay at the top
  right, and prints it to stderr every 100 samples.

## Usage

```
fred [-i | -f] [--height N] [+LINE] [FILE | DIR]
```

| | |
|---|---|
| `FILE` | the file to edit; it doesn't have to exist yet |
| `DIR` | list a directory instead (see [Dired](#directory-browser-dired)) |
| `+LINE` | start on that line; `+` alone starts on the last line |
| `-i`, `--inline` | open in a window under your prompt |
| `-f`, `--fullscreen` | take over the terminal, whatever the config says |
| `--height N` | inline: show N lines of text (default 12), or `max` for the whole terminal. The window starts as small as the file and grows with it |

## Keys

Counts work as in Vim: `3w`, `5j`, `2dd`.

### Moving

| Keys | Moves |
|---|---|
| `h j k l`, arrows | by character and line |
| `w b e`, `W B E` | by word, or by WORD (anything but spaces) |
| `0 ^ $`, Home, End | to the line's start, first character, end |
| `gg`, `G`, `{n}G` | to the first, last or nth line |
| `{ }` | by paragraph |
| `f F t T`, `; ,` | to a character on the line, and again |
| `/` `?`, `n N` | search forward or back, next or previous match |
| `m{a-z}`, `'{a-z}` | set a mark, go to it |
| `Ctrl-D Ctrl-U`, `Ctrl-F Ctrl-B` | half a window, a whole window |
| `zz` | put the cursor's line in the middle of the window |
| `gd` | go to the definition of the word under the cursor |

### Editing

| Keys | Does |
|---|---|
| `d c y` + motion | delete, change or copy (yank) over any motion, even a search: `d/foo<Enter>` |
| `dd cc yy`, `D C Y` | the same for whole lines, or to the end of the line |
| `x X s S r{c} J` | delete a character, substitute, replace, join lines |
| `p P` | put after or before the cursor |
| `i a I A o O` | start inserting |
| `u`, `Ctrl-R` | undo, redo |
| `.` | repeat the last change |
| `v`, `V` | Visual mode (select by character) and Visual-line mode (by line) |

In Visual mode, move to grow the selection, then `d x y c s` act on it,
`D X Y C S J :` act on its whole lines, `o` jumps to the other end and `K`
asks Claude to explain it.

`gh` and `gH` start Select mode, Neovim's selection that works like other
editors': typing replaces it, `Backspace` deletes it, and `Ctrl-G` switches
between Visual and Select. With `:set selectmode=mouse,key
keymodel=startsel,stopsel` (the Mac app's default) dragging, double-click
(word), triple-click (line) and `Shift`+arrows select this way, and a plain
arrow ends the selection.

Yanks and deletes also go to the system clipboard, and `p` puts what another
app copied since (`clipboard = false` keeps them inside fred).

### Insert mode

Completions pop up as you type: `Tab` or `Ctrl-N` picks the next, `Shift-Tab`
or `Ctrl-P` the previous, and `Enter` accepts one (or starts a new line when
none is picked). `Ctrl-W` deletes the word before the cursor and `Ctrl-U`
deletes back to the indent; both stop first where you started typing, as in
Neovim. `Esc` leaves Insert mode, and so does typing `jj`. Pasted text goes
in exactly as pasted.

### Mouse

Click to move the cursor, drag to select, double-click to select a word,
Shift-click to extend a selection, and scroll with the wheel. **Right-click**
opens Neovim's menu: Cut, Copy, Paste and Delete for a selection, Paste and
Select All anywhere, Go to definition on a word and Open in web browser on a
link. Cut, Copy and Paste use the system clipboard; Delete leaves your
clipboard alone.

### Everywhere

| Keys | Does |
|---|---|
| `Esc`, `Ctrl-G` | cancel: a picker, the `:` line, a half-typed command |
| `ZZ`, `ZQ` | save (if changed) and quit; quit without saving |
| `Space Enter`, `Space ;` | save (`:w`), close (`:q`) |
| `Ctrl-^` | back to the buffer you were in before |
| `Space`, then a short pause | show every `Space` key and what it does (as Emacs's which-key) |
| `Space w` then `1`–`9` | jump to a split window: each one shows its number until you pick (or click the window) |
| `Space w c` then `1`–`9` | start over with that many windows in as even a grid as fits: `4` is 2×2, `9` is 3×3, `3` is two above one |
| `Ctrl-Z` | suspend fred (`fg` brings it back) |

## Finding files and text

`Space` is the leader key (`l` still moves right).

| Keys | Opens |
|---|---|
| `Space p` | find a file in the project |
| `Space g` | grep the project as you type |
| `Space k` | search the lines of this file |
| `Space B` | search the lines of every open buffer |
| `Space s` | jump to any word on screen, in any window |
| `Space i` | list this file's functions, types, constants and variables (headings in Markdown and Org), as Emacs's imenu; search and jump |
| `Space d` | go to definition (same as `gd`) |
| `Space t` | your shell in a Ghostty terminal, in a new pane below this one (`:terminal` opens one in this pane); `jj` or `Ctrl-\ Ctrl-n` leaves terminal mode |
| `Space c` then a key | Claude Code: `c` chat (`:claude`), `/` slash command, `m` model, `p` permission mode, `r` resume, `n` new, `x` context, `i` interrupt |
| `Space j` | browse files from a path, like Emacs's find-file |
| `Space b` | switch between open buffers |
| `Space r` | recently opened files, from any project |
| `Space -` | list this file's directory (Dired) |
| `Space O` | show this directory in Finder or your file manager |

Pickers open as a panel at the bottom of the screen, only as tall as their
results. Type to filter; `Up`/`Ctrl-P` and `Down`/`Ctrl-N` move, `Enter`
opens and `Esc` goes back. Click a row to select it and double-click to open
it. The project is the git repository you're in (or the current directory),
and anything `.gitignore` ignores is skipped; `Ctrl-O` in `Space p` or
`Space g` includes ignored files (`vendor/`, Drupal core) until you press it
again. Opening
another file keeps this one open as a buffer, unsaved changes and all.

**Finding files** (`Space p`). Type a few letters in the order they appear in
the path: `sesrs` finds `src/session.rs`. Ranking follows
[fff](https://github.com/dmtrKovalenko/fff):

- Files you open often and recently come first, even before you type.
- The file a query led to before leads again, strongly from the third time.
- Changed files, files in the last ten commits and files near this one rank
  higher; the file you're editing ranks lower.
- A match in the file's name beats one in its directories.
- Typos are forgiven when little else matches: `sesison` finds `session.rs`
  and `cnotrolelr` finds `controller.rs`. Up to a quarter of the letters may
  be swapped, wrong, extra or missing.

Words in the query narrow the files: `*.rs`, `src/`, `**/*_test.go`,
`type:rust` and `status:modified` (or `untracked`, `unmodified`), with `!` in
front for "not". `name:12` or `name:12:3` opens the file at that line and
column.

**Grepping** (`Space g`). It starts in plain text (`[plain]`: `a.b(c)` means
exactly that); `Shift-Tab` switches to a regular expression (`[regex]`), then
to fuzzy (`[fuzzy]`), then back. All three ignore case unless you type a
capital. Fuzzy finds near misses: `UserController` finds
`UserAuthController`, and `shcema` finds `schema`. The same file filters work
before or after the pattern (`fn main *.rs !tests/`). `Enter` opens the match
and `n` goes on to the next. Files stay in memory while you type and are
searched on every core: on a 12,000-file project a keystroke takes under
30 ms (fuzzy, about 200).

**Go to definition** (`gd`, `Space d`) needs no language server. It greps the
project for lines that define the word (`fn`, `def`, `class`, `func`,
`const`, `let`, JavaScript arrow functions, C declarations, `#define`, shell
functions, Makefile targets, CSS selectors and more) and jumps straight there
when one stands out, or lets you pick.

**Browsing** (`Space j`). The prompt is a path; what you type after the last
`/` fuzzy-matches the files below it. `Tab` or `Right` goes into a directory,
`Left` (or `Backspace` after a `/`) goes up, `~/` jumps home, `Enter` on a
directory lists it in Dired, and `Enter` on a name that doesn't exist starts a
new file.

**Jumping** (`Space s`). Type the start of a word you can see; a unique match
jumps there, otherwise type the label shown on the word you want.

## Directory browser (Dired)

A directory opens as a listing, as in Emacs's Dired: `fred DIR`, `:e DIR`,
`Space -`, or `Enter` on a directory in `Space j`. Every Vim motion and search
works in it. These keys act on the entry under the cursor, on a `V`
selection, or on the entries marked with `*`:

| Key | Does |
|---|---|
| `Enter` | open a file, or go into a directory |
| `-`, `^` | up to the parent directory |
| `m`, `u`, `U`, `t` | mark, unmark, unmark all, toggle marks |
| `%` | mark names matching a regular expression |
| `d`, `x` | flag for deletion; delete the flagged (asks first) |
| `D` | delete now (asks first; directories with everything in them) |
| `R`, `C` | rename or move, copy; several marked go into a directory. Never overwrites |
| `+` | create a directory (`a/b/c` creates all three) |
| `M`, `T` | change permissions (octal), touch |
| `!` | run a shell command on the files (`*` stands for them) |
| `s`, `(`, `gh` | sort by name or time, hide details, hide dotfiles |
| `gr` | read the directory again |
| `i` | edit the names in place; `:w` renames them all, `gr` cancels |

## Matching lines (Occur)

`:occur REGEXP` (or `M-s o`, as in Emacs: Alt-s, then o) lists the buffer's matching lines in an
`*Occur*` buffer, as Emacs's `occur`: a summary line, then each line with its
number. Select lines first (`V`) to search only those. Case is
ignored unless the regexp has a capital letter. In the listing, `Enter` goes
to the match (`n` then finds the next one) and `gr` searches again. `i` makes
the listed lines editable; `:w` puts your changes back into the original
buffer as one undoable change (not saved to disk), and `gr` throws them away.

## `:` commands

Lines are addressed as in ed: `N`, `.`, `$`, `+n`, `-n`, `/re/`, `?re?`, `'a`,
`%` for the whole file, joined with `,` or `;`. In Visual mode `:` starts with
`'<,'>` filled in. Commands can be shortened as in Vim (`:w`, `:s`, `:bn`),
`Tab` completes command names, file names and option names, and `Up`/`Down`
go through the history. Each command is one undo step, even a `:g` that
changes 500 lines.

`:help` lists every command with how to write it and what it does; search it,
and `Enter` puts the one you pick on the `:` line. `:help split` starts the
search for you. While you type a command, a box above the `:` line shows its
usage, and `Tab`'s list of names says what each one does.

`:terminal` runs your shell in Ghostty's own terminal emulator,
[libghostty-vt](https://ghostty.org), in the pane you're on; `Space t` opens
one in a new pane below it. Each is a buffer like any other: split it, switch
to it with `:b`, have several. Colors keep your terminal's palette, and keys
are encoded the way Ghostty sends them, so editors, `less`, `htop` and the
kitty keyboard protocol work. Typing in it is terminal mode (`TERMINAL` in the
status line); `jj` or `Ctrl-\ Ctrl-n` leaves it for Normal mode, where
`CTRL-W`, `Space w` and `:` work as anywhere, and `i`, a click, or coming
back to the pane types in it again. The wheel scrolls back through 10,000
lines. When the shell exits, its buffer and pane go. Programs don't get mouse
clicks yet.

| Command | Does |
|---|---|
| `:w [file]`, `:w!` | write (`!` writes even if the file changed on disk) |
| `:[range]w file` | write just those lines to another file |
| `:wq`, `:x` | write and quit (`:x` only writes if there are changes) |
| `:q`, `:q!` | quit (refuses while a buffer has unsaved changes); quit anyway |
| `:e [file]`, `:e!` | edit another file, keeping this one as a buffer; reload; discard changes |
| `:b N`, `:b name`, `:b#` | go to buffer N, the one whose name contains `name`, or the one before |
| `:bn`, `:bp`, `:bd[!] [N]`, `:ls` | next, previous, close a buffer; pick one |
| `:[range]s/re/rep/[g]` | substitute; `&` and `\1`–`\9` in `rep`, `\n` or `\r` splits the line |
| `:[range]g/re/cmd`, `:g!`, `:v` | run `cmd` on matching (or not matching) lines |
| `:[range]d`, `:[range]j[!]` | delete lines; join them (`!` keeps spaces as they are) |
| `:[range]m addr`, `:[range]t addr` | move, copy lines (`0` means before the first line) |
| `:N` | go to line N |
| `:r file`, `:r !cmd` | insert a file, or a command's output, below the line |
| `:!cmd`, `:[range]!cmd` | run a shell command; filter lines through it (`:%!sort`) |
| `:pwd`, `:cd [dir]` | show or change the directory (`:cd` alone goes home) |
| `:set`, `:setlocal`, `:setglobal` | change options, as in Neovim ([below](#options-and-set)) |
| `:[range]ai ask` | have Claude rewrite the lines: `:'<,'>ai make this async` |
| `:[range]explain [question]` | have Claude explain the lines in a box over them |
| `:Magit NAME` | run a Magit command by name, e.g. `:Magit magit-fetch-all-prune` |

Patterns use Rust's [`regex`](https://docs.rs/regex) syntax, which is close
to extended regular expressions. `/` and `?` ignore case unless the pattern
has a capital letter; `:` commands match case exactly, as ed does, so a
substitution never touches text you didn't spell. Add `(?i)` to ignore case.
In shell commands `%` is the current file's name (`\%` for a literal `%`).

## Completion

Suggestions come from three places:

1. Words in this file, nearest the cursor first.
2. Words in nearby files: the same directory, then files with the same
   extension in the repository (skipping ignored, binary and large files).
   They're read in the background the first time you type.
3. File paths, once the text before the cursor contains `/` or starts with `~`.

## Git

In a git repository, lines that differ from the staged version are marked in
the gutter as you type: green for added, yellow for changed and red where
lines were removed. The status line shows the branch.

`Space m` opens a command panel modeled on
[Magit](https://magit.vc). Press the letter shown to run a command or open a
submenu; arrows and `Enter` work too, and `Esc` cancels. Submenus show their
switches and whether each is on.

| Keys | Opens |
|---|---|
| `Space m s` | status: untracked, unstaged, staged and conflicted files, stashes, unpushed and unpulled commits |
| `Space m c`, `Space m C` | commit, amend, extend, reword, fixup |
| `Space m d` | diff: unstaged, staged, a range, a commit, a stash |
| `Space m l`, `Space m L` | log (current, other, all, reflog…); this file's history |
| `Space m b` | branches: checkout, create, spin off, rename, reset, delete |
| `Space m p`, `P`, `f` | push, pull, fetch |
| `Space m m`, `R` | merge, rebase |
| `Space m x`, `r` | cherry-pick, revert |
| `Space m z` | stash: save, apply, pop, drop, list |
| `Space m t`, `M`, `X` | tags, remotes, reset |
| `Space m B`, `F` | blame; file actions (stage, untrack, rename, blobs) |
| `Space m i` | create a repository |

In status, `Tab` folds a section or shows a file's diff, `s` and `u` stage and
unstage a file or hunk, `Enter` visits it, `gr` refreshes and `q` goes back.
Git works on saved files and the index; it never saves your buffers for you.
Commit messages are drafts you can leave and come back to (`:w` keeps one,
`Space m c c` commits). Commands that may need a password or run hooks
(push, pull, fetch, commit, checkout) hand the terminal to Git and then come
back. Push never forces, and pull refuses histories that have diverged.

## Org mode

`.org` files open in Org mode: `Tab` folds headings, and Org's keys work for
TODO states, tags, properties, lists, tables, timestamps and links.
`Space o` opens Org commands from anywhere:

| Keys | Does |
|---|---|
| `Space o a`, `Space o c` | agenda, capture |
| `Space o t`, `Space o h` | new TODO heading, new heading |
| `Space o i`, `Space o s` | insert a link, store a link to here |
| `Space o I`, `Space o O` | clock in, clock out |
| `Space o f` | list your Org directory |

Org's options go in the `[org]` table of the config, by their Emacs names
(`org-todo-keywords = …`). Coverage is still partial; see the
[Org roadmap](docs/org-roadmap.md).

## Claude

`Space c c` (or `:claude`) opens `*claude*` in a pane below the one you're
on (or goes to its pane, if it's open): Claude Code, laid out like its
command line; `jj` or `Esc` leaves the prompt. Type after the `>` and press `Enter` to send (`Ctrl-J` for a
new line). The reply streams in above the prompt, `⏺` for text and tool
calls and `⎿` for their output, with a spinner on the status line. It's a
plain buffer, so you can leave Insert mode, move around, yank and switch
buffers while Claude works. `Ctrl-C` interrupts, as `Esc` does in the CLI:
the turn stops and the conversation goes on. `/clear` starts a new
conversation. Each prompt tells Claude which files are open in fred and on
what line, the one you used last first, so "this file" means the one you were
just in. One `claude_command` runs for the chat and takes each prompt in
turn. A command Claude runs that takes over 30 seconds moves to the
background, as `Ctrl-B` does in the CLI, so the chat never sits on it; when
it finishes, Claude picks the answer up and replies on its own
(`CLAUDE_CODE_AUTO_BACKGROUND_TIMEOUT_MS` in your environment changes the
30 seconds).
`claude_permissions` sets what Claude may do without asking: by default
`bypassPermissions`, no restrictions at all (it can run any command and edit
any file). `acceptEdits`, `plan`, `auto` and `manual` are stricter, and `""` leaves
it to your Claude Code settings. Where those would ask, fred asks:
`⏺ Allow Bash(cargo test)? y/n` in the chat, and in a box over the text
showing all of the command. Answer with `y` or `n` in Normal mode, from any
buffer (fred passes `claude` its own MCP server and
`--permission-prompt-tool`). Anything denied is counted on the status line.

Around the chat is a porcelain, as Magit is to git: `Space c` then a key,
from any buffer (pause after `Space c` and the keys show). It drives Claude
Code through its own control protocol (the one the Agent SDK uses), so
everything comes from your `claude`, nothing is copied into fred:

| `Space c` then | Does |
|---|---|
| `c` | the chat |
| `/` | pick a slash command: your skills, plugins and Claude Code's own, each with what it does; one that takes an argument goes on the prompt for you to finish |
| `m` | pick the model, from the ones your account has |
| `p` | pick the permission mode (`manual`, `acceptEdits`, `plan`, `auto`, `dontAsk`, `bypassPermissions`) |
| `r` | resume one of this directory's conversations, newest first, by title; it reads back as the chat |
| `n` | a new conversation (`/clear`) |
| `x` | how full the context is (`/context`) |
| `i` | interrupt (`Ctrl-C`) |

The chat's status line shows the model and permission mode in use, and
slash commands typed at the prompt (`/context`, `/compact`) show their output.

Claude can also drive fred in front of you. Add fred's MCP server once with
`claude mcp add fred -- fred --mcp`, and Claude Code, whether in another
terminal or in the chat, gets three tools: `fred_state` (the mode, the
cursor, and every open buffer with its line and whether it's unsaved),
`fred_read` (a buffer's text, unsaved changes included; given a path, it
opens the file in fred first) and `fred_keys` (typing into fred as you would,
`:e src/main.rs<CR>42Gciwfoo<Esc>`, a key a frame so you watch each one,
your own keys waiting until it's done and dropped if it moved you to another
buffer or mode; `Ctrl-C` stops it). Shell commands it types (`:!`, `:r !`,
filters, Org shell links) are refused: they go through its Bash tool and your
permission settings instead. Everything stays
undoable with `u`. The tools reach the fred most recently started, or the
one in `$FRED_SOCKET`. Each fred listens on a socket only your user can open.

In the chat, Claude works as it normally would, with its own Read, Edit and
Write tools, while you keep driving fred; a file it changes reloads unless
its buffer has unsaved changes. It can see your buffers (`fred_state`,
`fred_read`) but `fred_keys` is off there, so it never types into fred.

Select lines and run `:'<,'>ai make this async` (or any request): Claude
rewrites just those lines, seeing the whole file for context, and a spinner
shows while it works. `K` on a Visual selection, or `:[range]explain
[question]`, shows an explanation in a box over the lines; it stays until you
press `Esc` or click its ✕. Both run the `claude` command line; see
`ai_command`, `explain_command` and `ai_rules` in the config.

## Your files are safe

- **Atomic writes.** fred writes a temporary file beside yours, syncs it to
  disk and renames it over the original, keeping its permissions. Symlinks are
  followed; hard-linked files and files owned by someone else are written in
  place so the links and owner are kept.
- **Changes on disk are noticed.** If the file changed since you opened it,
  `:w` refuses and suggests `:w!`.
- **Crash recovery.** Unsaved changes go to a swap file
  (`~/.local/state/fred/swap/`) when you pause for a second, after 200 edits,
  and if the terminal closes, fred is killed or it crashes. Open the file
  again and fred offers to **r**ecover or **d**elete them. If another fred
  already has the file open, you get a read-only view instead.
- **Formats are kept.** CRLF line endings, a missing final newline and a UTF-8
  byte-order mark stay as they were. Files that aren't valid UTF-8 open
  read-only, so fred never mangles them.

## Configuration

Settings live in `~/.config/fred/config.toml` (or under `$XDG_CONFIG_HOME`).
Every key is optional; these are the defaults:

```toml
fullscreen = true        # false: inline; "auto": inline unless the file is
                         # longer than the inline window
height = 12              # inline: lines of text in the window, or "max"
wrap = false             # wrap long lines (otherwise scroll sideways)
numbers = true           # line numbers
relative_numbers = false
hl_line = false          # highlight the cursor's line
theme = "ansi"           # any bat theme: "Nord", "Dracula", "gruvbox-dark", …
tabstop = 8
autocomplete = true      # suggest completions while typing (Ctrl-N works anyway)
icons = false            # file icons in pickers (needs a Nerd Font)
clipboard = true         # share yanks and puts with the system clipboard
claude_command = "claude -p --output-format stream-json --verbose --include-partial-messages"
claude_permissions = "bypassPermissions"   # or acceptEdits, plan, default, "" (your settings)
ai_command = "claude -p --tools '' --safe-mode"   # what :ai runs, prompt on stdin
explain_command = "claude -p --tools '' --safe-mode --model sonnet"
ai_rules = ""            # added to every :ai, e.g. "Smallest change that works."
```

The default `ansi` theme uses your terminal's own colors, so it matches your
terminal. Highlighting uses [syntect](https://github.com/trishume/syntect)
with [bat](https://github.com/sharkdp/bat)'s language definitions, and turns
off for files over 10 MB or lines over 20,000 characters.

### Options and `:set`

`wrap`, `numbers`, `relative_numbers`, `hl_line`, `tabstop`, `autocomplete`
and `clipboard` above are Neovim's `wrap`, `number`, `relativenumber`,
`cursorline`, `tabstop`, `autocomplete` and `clipboard`. A `[set]` table sets
any option by its Neovim name or abbreviation:

```toml
[set]
ts = 4
cursorline = true
```

While editing, `:set` works as in Neovim: `:set ts=4`, `:set nowrap`,
`:set invnu`, `:set rnu!`, `:set ts?`, `:set ts&`, `:set cb+=unnamed`, and
`:set` alone lists what you've changed. `:setlocal` changes only this buffer,
`:setglobal` only what new buffers start with, and `Tab` completes option
names.

## Roadmap

fred is working toward full parity with three projects, each tracked by
percentage against a pinned upstream version, with every command, option
and key listed in a ledger:

| Goal | Plan | Ledger |
|---|---|---|
| Neovim's editing core (no Vimscript) | [roadmap](docs/nvim-core-roadmap.md) | [nvim-parity.csv](docs/nvim-parity.csv) |
| Emacs's Magit | [roadmap](docs/magit-roadmap.md) | [magit-parity.md](docs/magit-parity.md) |
| Emacs's Org mode | [roadmap](docs/org-roadmap.md) | [org-parity.csv](docs/org-parity.csv) |

Behavior is checked against the real thing: `tests/nvim_oracle.rs` types the
same keys into fred and a headless Neovim and compares the results.

## Development

```sh
cargo test                       # unit, fuzz, pseudo-terminal and Neovim-oracle tests
cargo test --test e2e show_screen -- --ignored --nocapture        # print real screens
FRED_FUZZ_ITERS=400000 FRED_FUZZ_THREADS=8 cargo test --release --test fuzz
tools/bench.py fred FILE         # startup CPU and memory
```

`tests/fuzz.rs` feeds random buffers and keys to the editor and checks the
cursor, undo and file-format invariants after every key. `tests/explore.rs`
drives the real binary in a pseudo-terminal: suspending, resizing, two freds
on one file, long lines, wrapping. The original design is in
[docs/superpowers/specs/2026-10-01-fred-design.md](docs/superpowers/specs/2026-10-01-fred-design.md).

## License

MIT. See [LICENSE](LICENSE).
