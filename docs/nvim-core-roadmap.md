# Neovim core parity roadmap

Target: the editing core of Neovim v0.12.5
(5885a30e1e1225349079e7a1c4a3848aa8e43e42): every mode's keys, the ex
commands, the options and the autocommand events, built into Fred's Rust
code. Vimscript, the other script languages and the GUI are out of scope;
a few non-blocking subsystems are deferred (see below).

## Percentage and accounting

**Verified roadmap completion: 3 / 100 points = 3%. Remaining: 97%.**

Points measure accepted deliverables, not lines written. A track's points are
awarded together only when all five gates pass:

1. every row of the track in `docs/nvim-parity.csv` is `done` or carries an
   explicit reason it differs;
2. the behavior is reachable the Neovim way (same keys, commands, options);
3. it is implemented in the module that owns it (see Architecture);
4. `tests/nvim_oracle.rs` cases cover normal, count, edge and error
   behavior and match real Neovim;
5. review with material findings fixed.

A partial track stays at zero. Finding more upstream behavior reopens a
track.

| Order | Track | Points | Accepted | Today in Fred |
| --- | --- | ---: | ---: | --- |
| 1 | Source baseline: pin, parity CSV, oracle, architecture | 4 | 3 | pin, CSV and oracle done; review open |
| 2 | Buffer / window separation | 6 | 0 | `Editor` is both |
| 3 | Options table, `:set`/`:setlocal`/`:setglobal` | 6 | 0 | 14 global settings in `config.toml` |
| 4 | Key tables for every mode | 4 | 0 | hand-written `parse()` |
| 5 | Registers (named, numbered, `-` `0` `_` `+` `*` `.` `:` `%` `/`) | 4 | 0 | one register |
| 6 | Motions (all of normal-index, `g`, `[ ]`) | 5 | 0 | 21 motions |
| 7 | Text objects | 4 | 0 | none |
| 8 | Operators (`> < = g~ gu gU gq gw g? ! zf g@`) | 5 | 0 | `d c y` |
| 9 | Visual, Visual-line, Visual-block | 5 | 0 | charwise and linewise, few keys |
| 10 | Insert and Replace mode keys | 4 | 0 | basics |
| 11 | Command-line editing keys | 3 | 0 | basics, history, Tab |
| 12 | Marks, jumplist, changelist | 3 | 0 | `a-z` line marks |
| 13 | Macros, `:normal`, `.`, CTRL-A/CTRL-X | 3 | 0 | `.` only |
| 14 | Search and Vim regex, hlsearch, incsearch | 6 | 0 | Rust regex, `/ ? n N` |
| 15 | `:substitute` and `:global` in full | 3 | 0 | `g` flag only |
| 16 | Editing, file, buffer and argument-list ex commands | 5 | 0 | 26 commands |
| 17 | Windows and tab pages (`CTRL-W`, `:split`, `:tab*`) | 6 | 0 | none |
| 18 | Mappings, abbreviations, user commands | 5 | 0 | none |
| 19 | Autocommands | 3 | 0 | none |
| 20 | Undo tree, `:earlier`/`:later`, undofile | 3 | 0 | linear undo |
| 21 | Folding (`z` commands, foldmethods) | 3 | 0 | Org-only hidden ranges |
| 22 | Quickfix, location lists, `:grep`, `:make`, tags | 5 | 0 | pickers stand in |
| 23 | Option behaviors (statusline, listchars, scrolloff, textwidth…) | 5 | 0 | a few |
| | Total | 100 | 3 | |

## Scope

The ledger, `docs/nvim-parity.csv`, has one row per ex command, option, key
and autocommand event, with `state` = `missing`, `partial`, `done`,
`deferred` or `excluded` and the reason in `fred_behavior`. `partial` means
Fred has it but it is not yet verified against Neovim.

**Excluded:** the Vimscript language (`:let`, `:if`, `:function`,
`:echo`, `:source`…); Lua, Python, Ruby, Perl, Tcl and plugin management;
GUI commands and options; Vimscript/Lua/remote-UI events.

**Deferred (not blocking, revisit later):** diff mode, spell checking, the
terminal, sessions/views/shada, the help system, right-to-left text and
input methods, and options that take an expression or function
(`foldexpr`, `indentexpr`, `omnifunc`…). Features that compute values with
Vimscript (`<expr>` mappings, `\=` in `:s`, the `=` register) follow the
expression-hook decision: Rust built-ins first, an embedded Lua (mlua)
later if wanted.

## Architecture

Each area is one declarative table that is the only place its definitions
live; lookup, abbreviations, validation, completion and the ledger all
read from it. The ex `COMMANDS` table in `src/ex/cmd.rs` is the model.

- `src/options/`: one table mirroring `src/nvim/options.lua` (name,
  abbreviation, type, default, scope, change hook); `:set` parsing;
  `config.toml` sets the same options.
- `src/vim/keys/`: a key table per mode (normal, visual, operator-pending,
  insert, cmdline) with what each key takes (count, char, register,
  motion) and whether it changes text.
- `src/ex/cmds/`: ex handlers split by family; the table stays in one
  place.
- Buffer (text, undo, marks, buffer options) separate from Window (cursor,
  scroll, jumplist, window options) and a layout tree for splits.
- One module each for registers, marks/jumps, text objects, macros, Vim
  regex, mappings, autocommands, quickfix, undo tree and folds.

## Tools

- Pinned source: `git clone --depth 1 --branch v0.12.5 --filter=blob:none
  --sparse https://github.com/neovim/neovim /private/tmp/fred-nvim-upstream`
  then `git sparse-checkout set --no-cone /src/nvim/ex_cmds.lua
  /src/nvim/options.lua /src/nvim/auevents.lua /runtime/doc/index.txt`.
- Ledger: `nvim --clean -l tools/nvim-source-inventory.lua
  /private/tmp/fred-nvim-upstream docs/nvim-parity.csv` (keeps recorded
  states).
- Oracle: `nvim --clean --headless -l tools/nvim-oracle.lua TEXT KEYS`
  prints Neovim's text, cursor and mode; `cargo test --test nvim_oracle`
  compares Fred against it for every case in `tests/nvim_oracle.rs`.

## Execution queue

1. Track 3 continued: multi-line display for `:set` / `:set all`,
   global-local options, then each new option arrives with its behavior
   (track 23). Done so far: the table (`src/options/`), `:set`,
   `:setlocal`, `:setglobal`, `[set]` in config.toml, Tab completion,
   7 options.
2. Buffer / window separation (track 2).
3. Registers (track 5), then key tables (track 4).
4. Tracks 6–13 (the editing grammar), each with oracle cases.

## Log

- 2026-10-06: baseline. Ledger: 1769 rows (166 partial, 1360 missing, 73
  deferred, 170 excluded). Oracle: 93 cases, all matching after fixing
  `:j` (now joins with a space; `:j!` added) and Insert CTRL-W/CTRL-U (stop
  at the insert start; CTRL-U keeps the indent).
- 2026-10-06: options table and `:set` family. `src/options/` holds the
  table (one `options!` entry per option, typed `Opt` enum), global values
  shared between buffers and local values per `Editor`; `:set`,
  `:setlocal`, `:setglobal` with every argument form and Neovim's error
  numbers; `[set]` in config.toml; render reads options instead of
  `Config`. 7 options: number, relativenumber, wrap, cursorline, tabstop,
  clipboard, autocomplete. Oracle: 99 cases.
