# Neovim core parity roadmap

Target: the editing core of Neovim v0.12.5
(5885a30e1e1225349079e7a1c4a3848aa8e43e42): every mode's keys, the ex
commands, the options and the autocommand events, built into Fred's Rust
code. Vimscript, the other script languages and the GUI are out of scope;
a few non-blocking subsystems are deferred (see below).

## Percentage and accounting

**Verified roadmap completion: 6 / 100 points = 6%. Remaining: 94%.**

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
| 5 | Registers (named, numbered, `-` `0` `_` `+` `*` `.` `:` `%` `/`) | 4 | 0 | all of them (`src/vim/register.rs`); no `:registers`, Insert CTRL-R |
| 6 | Motions (all of normal-index, `g`, `[ ]`) | 5 | 0 | `ge gE H M L \| _ g_ % N% ( ) [[ ]] [] ][ [( [{ ]) ]} * # g* g# go` added; no `` ` `` marks, `[m`, `[*`, `gm`, `''` |
| 7 | Text objects | 4 | 0 | all, `it`/`at` included (`src/vim/textobj.rs`) |
| 8 | Operators (`> < = g~ gu gU gq gw g? ! zf g@`) | 5 | 0 | `d c y > < g~ gu gU g? gq gw !`, `~`, `gJ`; no `=` (C indenting), `zf`, `g@` |
| 9 | Visual, Visual-line, Visual-block | 5 | 0 | all three (`src/vim/block.rs`); text objects, counts, `p`, `r`, CTRL-A; Select mode; no block `p` (replace), Select-block (`g CTRL-H`) |
| 10 | Insert and Replace mode keys | 4 | 0 | most keys (CTRL-R, CTRL-O, CTRL-V, CTRL-T/CTRL-D, CTRL-A, CTRL-E/CTRL-Y, CTRL-G); counts; Replace mode; no CTRL-X submodes, CTRL-K digraphs, abbreviations |
| 11 | Command-line editing keys | 3 | 0 | CTRL-R (and CTRL-R CTRL-W…), CTRL-V, CTRL-B/E/H/J/M, word moves, history by prefix; no CTRL-D/CTRL-A/CTRL-L completion lists, 'incsearch' keys |
| 12 | Marks, jumplist, changelist | 3 | 0 | marks with columns, `` ` `` and `'`, special marks, CTRL-O/CTRL-I, `g;`/`g,` (`src/vim/marks.rs`); A-Z buffer-local, no 0-9, `:marks`, `:jumps`, `:changes`, `:delmarks` |
| 13 | Macros, `:normal`, `.`, CTRL-A/CTRL-X | 3 | 0 | `.`, `q`, `@`, `@@`, `@:`, `Q`, CTRL-A/CTRL-X (`g CTRL-A`); no `:normal` |
| 14 | Search and Vim regex, hlsearch, incsearch | 6 | 0 | Vim magic patterns (no `\zs`, `\@`, `\_`, back-references), `/ ? n N` |
| 15 | `:substitute` and `:global` in full | 3 | 0 | flags `& e g i I n r`, count, `~`, case escapes, `:&` `:&&` `:~` `&` `g&`; no `c` (confirm), `\=` |
| 16 | Editing, file, buffer and argument-list ex commands | 5 | 0 | about 60 commands; editor-run ones (`:normal`, `:put`, `:yank`…) in `src/vim/excmd.rs`; `\|` between commands |
| 17 | Windows and tab pages (`CTRL-W`, `:split`, `:tab*`) | 6 | 0 | none |
| 18 | Mappings, abbreviations, user commands | 5 | 0 | mappings and abbreviations (`src/vim/mappings.rs`), `:source`; no 'timeout', `:command` |
| 19 | Autocommands | 3 | 0 | none |
| 20 | Undo tree, `:earlier`/`:later`, undofile | 3 | 3 | undo tree (`src/undo.rs`): `g-` `g+`, `:undo N`/`:undo!`, `:earlier`/`:later` (count, time, writes), `:undojoin`, `:undolist`, `:wundo`/`:rundo`, 'undolevels' 'undofile' 'undodir' 'undoreload'; accepted 2026-10-26 (no `:{N}undo`) |
| 21 | Folding (`z` commands, foldmethods) | 3 | 0 | Org-only hidden ranges |
| 22 | Quickfix, location lists, `:grep`, `:make`, tags | 5 | 0 | pickers stand in |
| 23 | Option behaviors (statusline, listchars, scrolloff, textwidth…) | 5 | 0 | a few |
| | Total | 100 | 6 | |

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
   9 options.
2. Buffer / window separation (track 2) and windows (track 17): split
   windows part 2 (spec stages 3–7 of
   `docs/superpowers/specs/2026-10-19-split-windows-design.md`): resizing
   (`CTRL-W + - < > = _ |`, `:resize`, options, `equalalways`), mouse in
   windows, special buffers opening below with quit-window, zap across
   windows, `fred_state` windows and the `fred_read` reload fix, oracle
   cases. Part 1 is done.
3. Registers (track 5), then key tables (track 4).
4. Tracks 6–13 (the editing grammar), each with oracle cases. Done so
   far: the motions, text objects and operators in the table above. Next:
   `=`, `zf`, `zh`/`zl`/`zs`/`ze`, scrolling with wrapped lines and
   folds. Track 20 (undo tree) awaits review.
5. Track 9 continued: Select block mode (`g CTRL-H`, needs Visual-block),
   Select CTRL-O and CTRL-R, `.` repeating a Select-mode replacement,
   'selection' (exclusive), `<S-Right>`/`<S-Left>` as word motions when
   'keymodel' lacks startsel. Done so far: Select mode, 'keymodel',
   'selectmode'.
6. Fuzz findings outside the default seed range (`FRED_FUZZ_SEED=30000
   FRED_FUZZ_ITERS=100000`): Visual-block panics (`<PageDown><C-v>s<BS>`
   on "\n\n"; `old<Esc><C-v>c<C-w>` + U+2028), `@2` after
   `Ox'…Vy3pS<Esc>dd` changing text on E20, and undo reaching an unseen
   state after `V<C-n>S<Esc>3ggVDVs<Esc>dG@3`.
7. Track 13: `.` after a Visual-mode operator (Vim repeats it on the same
   amount of text; Fred does nothing). Search offsets under an operator
   (`d/x/e`).
8. Track 14 continued: `\zs`/`\ze`, `\@` look-around, `\_` multi-line
   and back-references (the `regex` crate has none of them), `~`, hlsearch,
   incsearch. Done so far: Vim pattern syntax for `/ ? :s :g` and range
   addresses (`src/search.rs` `compile_vim`).

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
- 2026-10-16: right-click menu (`src/menu.rs`): Neovim's PopUp menu from
  `runtime/lua/vim/_core/defaults.lua` with 'mousemodel' popup_setpos, items
  by mode (Open in web browser, Go to definition, Cut, Copy, Paste, Delete,
  Select All), `"+` via the system clipboard and `"_` via
  `ops::delete_black_hole`; keyboard and mouse selection. Unit tests now
  use an in-process clipboard instead of the user's.
- 2026-10-20: Vim patterns. `search::compile_vim` turns Vim's magic syntax
  (`\v \m \M \V`, `\( \| \+ \= \{n,m} \{-}`, `\< \>`, `\%( \%d \%x \%u`,
  `\c \C`, classes `\s \d \w \a \x`..., `[...]` with `[:alpha:]`) into Rust
  regex for `/ ? :s :g` and addresses; a bare `( | + {` now matches
  itself, as in Vim. `/` keeps Fred's smartcase. Oracle: 108 cases.
- 2026-10-20: split windows part 1: layout tree (`src/window.rs`), windows
  in `Session` (each with its own cursor and scroll; buffers keep their
  scroll and highlighting across switches), per-window drawing with
  active/inactive status lines and `│` dividers (`src/ui/windows.rs`),
  `:split` family with counts, `CTRL-W` moves, closing (`:close`, `:only`,
  `:hide`, `:q` in a split, `:qall` family, `:bd` closing its windows).
  Tracks 2 and 17 stay at 0: tabs, resizing and the buffer/window struct
  split remain.
- 2026-10-20: `:help` (partial): every `COMMANDS` entry now carries a
  one-line description, and its usage is generated from the table. `:help
  [query]` searches them in a picker; the `:` line shows the typed
  command's usage in a box, and Tab's names say what each does. A test
  checks every name and shortest abbreviation runs its own command. No
  help files or tags.
- 2026-10-20: `:terminal` (partial, `--features ghostty`): a terminal
  buffer (`Editor::term`, `src/term.rs`, `src/session/term.rs`) running the
  shell in libghostty-vt in a PTY, drawn in its window; `Space t` splits
  below. Insert mode in it is terminal mode; `Ctrl-\ Ctrl-n` and `jj`
  leave. The buffer goes when the shell exits. Normal mode shows the live
  screen, not scrollback text; no `{cmd}` argument, no mouse reporting. No
  points: terminal is out of scope.
- 2026-10-21: Select mode (`src/vim/select.rs`), as Neovim's
  `VIsual_select` flag on a Visual selection: `gh`, `gH`, CTRL-G both
  ways, typing replaces (a linewise selection is changed charwise, so its
  line break goes too), `<BS>`/`<Del>`/CTRL-H delete and leave Normal mode;
  'keymodel' startsel/stopsel (also from Insert, returning to Insert when
  the selection ends, as Neovim's `restart_edit`), 'selectmode' mouse, key
  and cmd; triple-click selects a line. `fred --gui` sets
  `selectmode=mouse,key` and `keymodel=startsel,stopsel` unless `[set]`
  says otherwise. Oracle: 157 cases. Track 9 stays at 0: Visual-block and
  most Visual keys remain.
- 2026-10-25: editing grammar, part 1. Neovim's `inc()`/`dec()` walker,
  word classes, `findsent()`, `findmatchlimit()` (quotes and `'x'`
  skipped) and the text objects `iw aw iW aW is as ip ap i( a( ib ab i{
  a{ iB aB i[ a[ i< a< i" a" i' a' i\` a\`` ported in `src/vim/textobj.rs`,
  in operators and Visual mode (where they extend the selection). Motions
  `ge gE H M L | _ g_ % N% ( ) ]] ][ [[ [] [( [{ ]) ]} * # g* g# go`,
  CTRL-H, CTRL-J; `g0 g^ g$ gj gk` alias the line motions. Operators `> <
  g~ gu gU g?` with their doubled forms, `~`, `gJ`; Visual `< > u U ~ g?
  g~ gu gU gJ`. Options 'shiftwidth' and 'expandtab' (Fred sets them for
  a space-indented file). `G`, `gg`, `H`, `L` and `>>` keep the column
  ('nostartofline'), and the cursor rests on a tab's last column, as in
  Neovim. `N%` under an operator is characterwise and exclusive, as with
  Neovim's default matchit. Ledger: 168 keys done. Oracle: 367 cases. No
  track is complete yet, so points stay at 3.
- 2026-10-25: registers and macros (`src/vim/register.rs`). `"x` before
  any command (counts on both sides multiply), named `a-z` with `A-Z`
  appending, `"0` for yanks, `"1`-`"9` shifting for deletes of a line or
  more (or by `% ( ) / ? n N { }`), `"-`, `"_`, `"+`/`"*`, read-only `".
  ": "/ "%`; `.` after `"1p` steps to `"2p`. Visual `p` and `P`, `gp`,
  `gP`, `2dd` on the last line fails. Macros: `q{reg}` records typed keys
  as register text (the status line shows `recording @x`), `q` stops,
  `@{reg}`, `@@`, `@:`, `Q`; a failing command ends a run, as in Vim.
  Oracle: 427 cases. Ledger: 182 keys done.
- 2026-10-25: CTRL-A/CTRL-X as Neovim's `do_addsub()` (decimal with
  wraparound, hex keeping its case, binary, leading zeros kept, option
  'nrformats' with alpha/octal/unsigned/blank), in Visual mode on each
  line's first number and `g CTRL-A` counting up. Counts in Visual mode
  (`3>`). Counts on `i a I A o O` type the text again; Replace mode `R`
  (with counts, `<BS>` putting text back, REPLACE in the status line);
  `gi`, `gI`, `gv`. Oracle: 495 cases. Ledger: 193 keys done.
- 2026-10-25: Insert-mode keys: CTRL-R {reg} (and CTRL-R CTRL-R/O/P),
  CTRL-A, CTRL-@, CTRL-T, CTRL-D, `0 CTRL-D`, CTRL-E, CTRL-Y, CTRL-V as
  Neovim's `get_literal()` (decimal, `x`, `o`, `u`, `U`), CTRL-J/M/I,
  CTRL-O (one Normal command, coming back after the last character as
  Neovim does), CTRL-G j/k/u, `<S-Left>`/`<C-Left>` and right,
  `<C-Home>`, `<C-End>`, page keys. Oracle: 543 cases. Ledger: 218 keys
  done.
- 2026-10-25: marks (`src/vim/marks.rs`, after Neovim's `mark.c`): every
  mark keeps its column (`` `a ``), `''`/` `` ` the previous context mark
  (set by the jump motions and searches, checked as `checkpcmark()`),
  `'[ ']` around the last change or yank, `'< '>`, `'.`, `'^`, `'( ') '{
  '}`; the jumplist (CTRL-O, CTRL-I/Tab, with `cleanup_jumplist()`) and
  the changelist (`g;`, `g,`, one entry per line within 79 columns).
  Oracle: 589 cases. Ledger: 241 keys done.
- 2026-10-25: `:substitute` in full but confirm: flags `& e g i I n r`
  (and `p # l` accepted), a count, `~` for the previous replacement,
  `\u \U \l \L \e \E` case changes, `\r`, `\t`; `N substitutions on M
  lines` and the `n` flag's count; E486. `:&`, `:&&`, `:~`, Normal `&`
  and `g&`. Command names of one symbol (`:&`, `:~`) in the table.
  Oracle: 629 cases.
- 2026-10-25: ex commands that need the editor carry the `EDITOR` flag in
  `COMMANDS`: the interpreter reads their range, then `src/vim/excmd.rs`
  runs them. `:normal` (also `:g/pat/normal`), `:yank`, `:put` (`:0put`),
  `:delete` into registers with a count, `:>`, `:<`, `:k`, `:mark`,
  `:undo`, `:redo`, `:startinsert`, `:startreplace`, `:stopinsert`,
  `:retab`, `:left`, `:center`, `:right`, `:registers`/`:display`,
  `:marks`, `:delmarks`, `:jumps`, `:changes`, `:clearjumps` (lists in
  a picker), `:=`, `:print`, `:number`, `:#`, `:list`, `:update`;
  `:silent` and the other modifiers are accepted; `|` separates commands
  (not in `:g`, `:normal`, `:!` or a `:s` pattern). `:g` with an editor
  command runs it line by line in the editor, following the lines.
  Oracle: 704 cases.
- 2026-10-25: `it`/`at` (tags scanned as Neovim's searchpair patterns
  match them; an end in column 0 isn't adjusted, as upstream), `gq`/`gw`
  with 'textwidth' (79 when 0) and auto-wrap while typing past it, the
  `!` filter operator (`!{motion}`, `!!`, Visual `!`) opening `:.,.+N!`,
  and `:sort` with `! b f i n o r u x /pat/`. Filters leave the cursor on
  the first line. Oracle: 765 cases.
- 2026-10-25: scrolling (`src/vim/scroll.rs`, after Neovim's
  `pagescroll()`, `scroll_redraw()`, `scroll_cursor_top/bot/halfway()`):
  CTRL-E, CTRL-Y, CTRL-D/CTRL-U (a count sets 'scroll'), CTRL-F/CTRL-B
  and the page keys, `zt zb zz z<CR> z. z- z+ z^`. The editor asks the
  window for a top line (`Editor::view_req`); after a scroll command the
  view may show past the end, as Vim's. 'scrolloff' is an option (Fred's
  default 2; Neovim's 0) and `H`/`L` use it. The oracle now emulates a
  22-row window (`Editor::follow_view`). Oracle: 838 cases.
- 2026-10-25: operator-pending `v`/`V` (`dvj`, `dVw`), `gn`/`gN` (with
  `cgn` and `.`), `U`, CTRL-G, `ga`, `g8`, `gm`, `gM`, `g'`/`` g` ``,
  `['` `]'` `` [` `` `` ]` ``, `[p` `]p` `[P` `]P`, `{count}<Del>`, the
  CTRL and Shift arrows and `<C-Home>`/`<C-End>`; Visual `O`, `R`,
  `r{c}`, CTRL-C and `gv` swapping with the previous selection.
  Command-line keys: CTRL-R {reg} (and CTRL-R CTRL-W/A/L/F), CTRL-V with
  codes (shared with Insert mode), CTRL-B/E/H/J/M/I/[, Shift and CTRL
  arrows, Up/Down recalling lines that start with what was typed,
  PageUp/PageDown, CTRL-N/CTRL-P. Oracle: 915 cases.
- 2026-10-25: Visual-block mode (`src/vim/block.rs`, after Neovim's
  `block_prep()`): CTRL-V (from Normal, `v` and `V`), `o`, `O`, `$`, `d x
  D y c s C I A r ~ u U g? > < J`, blockwise registers (`"ay`, `p`, `P`
  with padding), `I`/`A`/`c` typed again on every line when Insert mode
  ends (short lines skipped for `I`, padded for `A`), V-BLOCK in the
  status line and the block's columns highlighted. In Visual mode the
  cursor may stand on a line's end, as in Vim (`v$`, `l` past the last
  character), so the line break is selected. Fixed CTRL-G/L/\ and CTRL-V
  not reaching their handlers. Oracle: 960 cases.
- 2026-10-25: key mappings (`src/vim/mappings.rs`): `:map`, `:noremap`,
  `:unmap`, `:mapclear` for `n x s o i c` (and `:map!`), `<buffer>`,
  `<silent>`, `<nowait>`, `<unique>`, `<Nop>`, `<leader>` (Space, Fred's
  leader); typed keys are matched (buffer maps first), remapped as Vim
  does (not where the rhs starts with its own lhs; E223 for a loop);
  `:normal` maps, `:normal!` doesn't; listings in a picker. Key notation
  is case-insensitive and knows `<Space> <Bar> <Bslash> <NL> <Return>`.
  No 'timeout': a key that starts a longer mapping waits for the next.
  Oracle: 993 cases.
- 2026-10-25: `:source` (ex commands, `"` comments, `\` continuations;
  `!` for keys) and abbreviations (`:ab :iab :cab :noreab :una :abc` and
  their Insert/Cmdline forms, `<buffer>`): full-id and end-id ones expand
  before a non-keyword character, Esc, Enter or CTRL-], when the word
  starts there or where Insert mode began, as in Vim. Oracle: 1011 cases.
- 2026-10-26: undo tree (`src/undo.rs`, after Neovim's `undo.c`): a
  change made after undoing starts a branch; `u`/CTRL-R follow the branch
  last used, `g-`/`g+` and `:earlier`/`:later` move through the states in
  the order they were made (`{N}`, `{N}s m h d`, `{N}f` by writes, as
  `undo_time()`), `:undo N` (E830), `:undo!` and `:undo! N` (E5767),
  `:undojoin` (for the rest of the command line, E790), `:undolist` (a
  picker). Undo and redo put the cursor where the change started (Vim's
  `uh_cursor`, taken at the first edit). 'undolevels' (0: `u` undoes
  itself, -1: none, N trims as `u_savecommon()`; a buffer option until
  global-local exists), 'undoreload' (`:e!` is a change to undo),
  'undofile'/'undodir' and `:wundo`/`:rundo` in Fred's own JSON format
  with a text hash, the default 'undodir' `$XDG_STATE_HOME/fred/undo`.
  `<Undo>` excluded: no key sends it. The oracle now clears undo after
  setting the text. Oracle: 1078 cases.
- 2026-10-26: track 20 review fixes: the saved state is named by a node
  id that is never reused (`:undo!` gives its seq to the next change, as
  Vim, which had cleared 'modified' over unsaved text); undo files are
  0600 in a 0700 'undodir', entries expand `~` and `$VAR` and drop `//`,
  a corrupt or tampered file is refused (links, `cur`, edit ranges,
  counters near overflow) instead of panicking, an existing
  non-undo file is never overwritten;
  `:earlier`/`:later` counts past an int are 0 as `getdigits_int()`;
  tree moves wait while a change is open. `zg zw zG zW zuw zug zuW zuG`
  deferred with spell checking. Oracle: 1085 cases.
- 2026-10-26: track 20 accepted (3 points; 6/100). Gate-5 review found
  and the fixes closed: the save state reusing a forgotten seq after
  `:undo!`, private undo files (0600/0700), `~`/`$VAR` in 'undodir',
  corrupt or crafted undo files refused without panicking, saturating
  `:earlier`/`:later` counts. Next: make `tests/fuzz.rs` pass again
  (seed 808 hangs on `1000000i<Tab>`; two failure classes to triage).
- 2026-10-28: `tests/fuzz.rs` passes again (seeds 0..30000), checked
  against Neovim. Fixes: Visual `<Del>` deletes the selection; `:g` stops
  at the first error, a `:s` that finds nothing inside it is quiet (E486
  at the end only if nothing matched) and the cursor goes to the first
  non-blank after substituting; line 0 is line 1 for ranges
  (`correct_range()`, `:0d`; `:0r` refused until reading above line 1
  exists); a `|` command line is one undo step; a cursor key in Insert
  mode starts a new insert (`start_arrow()`: no count repeat, `.` repeats
  what came before it or restarts as `i`), typing sets the column
  `<Up>`/`<Down>` keep, `<BS>` onto the line before moves the insert
  start; an unused auto-indent from `o O cc S <CR>` goes on `<Esc>`,
  CTRL-O, `<CR>` or leaving the line (did_ai); a CTRL-O command that
  starts its own insert (`o`, `S`) ends in Normal mode; a multi-line
  charwise `d` from the indent to trailing blanks is linewise
  (`op_delete()`); `col_of_byte` tolerates a byte inside a character.
  Fuzz harness: `:g`, `:normal` and `|` lines may change text and fail,
  Visual modes may stand on the line end, undo states made inside a key
  (Insert CTRL-O) count as seen, insert counts and `{count}.` over 1000
  are steered away, `{count}<Del>` is understood. Known difference: Neovim
  records one more (empty) undo step in `o<C-o>X<Esc>`. Oracle: 1168
  cases.
