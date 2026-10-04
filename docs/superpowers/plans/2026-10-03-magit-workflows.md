# Magit Workflow Expansion Implementation Plan

> **For agentic workers:** use superpowers:executing-plans inline, with one independent review at the end.

**Goal:** Extend the existing component with daily history/stash/ref workflows.
**Architecture:** Typed operations construct structured Git invocations. Existing picker and command-line mechanisms present menus and prompts. Existing Session jobs and terminal handoff execute and refresh.
**Tech Stack:** Rust/std and existing Fred dependencies; no new packages.
**Spec:** docs/superpowers/specs/2026-10-03-magit-workflows-design.md

## Global Constraints

Preserve Space m s/p/P/f/c/l/b. Operate on saved files/index. Preserve source
buffers, anchored prompts and recoverable drafts. Validate revisions and refs
before mutations; no shell interpolation. Real Git tests precede implementation.

## Review Focus

Stash selector renumbering must not drop the wrong entry. Cancelled prompts
must never leak into another command. Worktree-local in-progress paths must
come from Git. Failed operations must refresh conflict status. Amendment mode
must survive recovery without overwriting a nonempty ordinary draft.

## Tasks

- [ ] Add failing real-Git tests for typed stash/ref/history operations in src/magit/tests.rs; add operation types and implement in src/magit/workflows.rs. Repo::operation(&Operation) -> Result<GitInvocation,String>, Repo::stashes() -> Result<Vec<Stash>,String>, Repo::tags() -> Result<Vec<String>,String>.
- [ ] Add failing state and conflict tests; extend Snapshot with active OperationState from git-resolved paths and validate continue/skip/abort against matching state. Verify linked worktrees and stale stash identity.
- [ ] Add failing key/prompt tests. Extend Action with Menu, Prompt, Submit and Workflows. Add visible suffix menus through Kind::MagitMenu in src/pick/mod.rs and anchored Prompt in Editor; reuse existing cmdline editing and cancellation.
- [ ] Add failing session stash/tag view tests; extend View/RowAction, read-only local bindings, refresh and inspect. Execute mutations through existing GitInvocation handoff and refresh failed-operation state without losing its diagnostic.
- [ ] Add failing amend-draft tests; preserve amend mode across Editor recovery, prepare HEAD message without clobbering existing drafts, and submit git commit --amend through existing draft protection.
- [ ] Add pseudo-terminal menu/stash regression tests; update README and parity ledger. Run full test suite, fmt, Clippy and release build, review and fix reproduced findings, commit explicit files without notes.md.

## Execution record

The first workflow increment implements typed stash save/apply, branch create/
switch/rename/merged-delete, lightweight tags, amend without message editing,
fixup, merge/squash, rebase, cherry-pick and revert, including continuation,
skip and abort where supported. Stash/tag lists support inspection and refresh.
Menus use the existing message area rather than introducing another picker type.
Prompts are repository anchored and reject delayed results after newer input.
Failed terminal operations refresh the current generated view and retain errors.
Git-owned metadata paths support linked worktrees, and GIT_EDITOR is explicitly
set for Fred's noninteractive message continuation.

Remaining tasks from this design: stash pop/drop with stale-identity protection,
annotated/deleted tags and remote-tracking branch creation, editable amend drafts,
and richer transient options. Active operation state is displayed in status and
sequencer todo files cover multicommit cherry-pick/revert between conflicts. These are not marked complete or represented as
full Magit parity. README records remaining broader feature families.

Independent review reproduced delayed-prompt cancellation and inherited-editor
defects; both fixed with regression tests. Real Git covers stash round trips,
refs, linked-worktree merge conflict/abort, cherry-pick, revert, fixup and rebase.
Session tests cover prompt cancellation and failed operation refresh. A PTY test
covers menu/prompt cancellation, terminal failure and restoration.

## Source-parity continuation (binding user clarification)

The user requires all of upstream Magit: menus, arguments, command behavior and
edge cases, adapted from Emacs to Fred under `Space m`. The partial increments
are not the agreed finish line. `docs/magit-port-notes.md` supersedes any earlier
subset interpretation and records the pinned source baseline and continuation
requirements. `docs/magit-parity.csv` is the explicit-command/option lower-bound
inventory; `docs/magit-parity.md` explains its limitations.

This continuation implements navigable menu panels and typed argument switches,
source stash use/drop keys with stable reflog identities, index-preserving
application and plain fallback retaining the stash, dedicated notes/staged/
unstaged/untracked stash diffs, confirmed drop and pop through the list, and
separate HEAD-pinned editable amend/reword drafts. Common commit flags remain
attached through draft creation and swap recovery. Draft outcomes reject newer
key/paste input, and terminal execution validates the target HEAD again.

An integration test exposed that the global `--literal-pathspecs` flag interferes
with Git stash's internal cleanup. Ruling: protect file operations using raw
`:(literal)` pathspec arguments instead of a global flag, so Git's own subprocess
pathspecs retain their intended interpretation. Existing unusual-path/hunk tests
continue to verify safety. Tests specifically cover saving and removing ignored
files through `-a`, keeping staged versus unstaged stash sides, retaining conflicts,
stale drop confirmation, cancelled drop, recoverable amend mode/options, HEAD
movement and two-mode PTY workflows.

Further source differences remain open: stash worktree-only, snapshots, transforms,
3way/reject fallback negotiation, customizable refs/autostash and section-level
interaction; remaining commit options, publication warnings, richer commit
editing and rewrite variants; annotated/deleted tags, tracking-branch creation,
and the wider source inventory. These remain implementation work, not exclusions.
