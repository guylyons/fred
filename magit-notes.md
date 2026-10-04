# Magit in Fred

Fred includes a port of [Magit](https://github.com/magit/magit) (pinned at
upstream e9ed99c5), adapted to Fred's terminal editor and to evil-collection's
default Magit bindings. This page is the overview; detailed evidence lives in
`docs/`.

## Status

- Ledger: 669 of 773 upstream commands work at least partly, 104 are missing
  (`docs/magit-parity.csv`, summary in `docs/magit-parity.md`).
- Strict roadmap score: 3% (`docs/magit-roadmap.md`). A track only counts once
  every row in it is complete and tested; every track still has partial rows.
- Each feature family has an acceptance page (`docs/magit-*-acceptance.md`)
  recording upstream behavior, Fred's behavior, divergences and the tests that
  prove it, including the independent review fixes.

## Getting in

- `Space m` opens Magit's dispatch menu. The exact `Space m` layout from the
  user's Emacs config is preserved (`docs/magit-user-bindings.md`): `s` status,
  `l` log, `L` file log, `b` branch, `c` commit, `d` diff, `f` fetch, `p` push,
  `P` pull, `r` revert, `B` blame, `i` init.
- Inside Magit buffers, upstream's single keys work as evil-collection's
  defaults leave or move them: `b c d f l m r t z A C D F I L M T W Y Z`, plus
  `p` push, `O` reset, `o` reset quickly, `'` submodule, `"` subtree, `_`
  revert, `x` discard, `X` untrack, `-` reverse, `|` git command, `` ` ``
  process log, `h` / `?` dispatch. Vim keeps `j k v V n N g G :`.
- `:Magit NAME` runs an upstream command by name (Fred's M-x for Magit), e.g.
  `:Magit magit-fetch-all-prune`. This reaches commands upstream leaves
  unbound.
- Menus render as transient popups: switches show on/off, options show their
  value (pressing a set option clears it), and `-`, `=`, `+`, `,`, `/` act as
  prefix keys.

## What is covered

Status (sections, staging, discard/reverse, section movement and levels), diff
(all targets, refresh menu, context, visit, apply/reverse hunks), log (all
variants and arguments, graph, shortlog, cherry, reflog, refresh), commit
(drafts in Fred, fixup family, reshelve, absorb modules, trailers, message
history), branch (incl. configure, orphan, shelve), merge, rebase (interactive
todo editing), cherry-pick/revert, reset, stash, tag, notes, remote (incl.
configure, unshallow), fetch/pull/push, bisect, blame, blob and file
dispatch, worktrees, submodules, subtrees, patches and `git am`, bundles,
clone, refs view, gitignore, sparse checkout, wip refs, smerge conflict keys,
and the process log.

## Adaptations

- Network and editor-driven Git commands (push, pull, fetch, clone, rebase,
  am, mergetool, difftool) hand the terminal to Git, so credentials prompts,
  progress and Ctrl-C work; Fred is Git's editor while it runs.
- Fred has no Ediff: `E`/`e` run the user's `git difftool` and `git mergetool`.
- Fred has no windows or frames: other-window/other-frame variants behave like
  the plain command.
- Configuration variables in branch/remote configure menus cycle or prompt and
  report the new value; menus do not show live values yet.
- Commit message history steps through the repository log rather than Emacs's
  comment ring.
- Destructive actions always confirm, and discard follows upstream's
  per-status rules so it never removes work it did not show.

## Not done yet

Transient set/save and levels, live variable values in menus, log-select,
margins, the repository list, wip-mode autosave, smerge on status hunks, and
Emacs-only commands (gitk/git-gui launchers, mouse commands, profiling, Info,
ChangeLog helpers). The ledger tracks every remaining row.

## Working on it

Read `docs/magit-port-notes.md` and `docs/magit-parity.md` first, follow the
roadmap's execution queue, update the ledger in every implementation commit,
and keep the `Space m` layout. Tests: `cargo test` (real-Git tests in
`src/magit/tests.rs`, editor flows in `src/session.rs`), `cargo clippy
--all-targets -- -D warnings -A clippy::too_many_arguments`, `cargo fmt
--check`.
