# Magit port: binding scope and continuation notes

## User's requirement

Port Magit from Emacs to Fred as a Rust component. Reproduce its menus,
functionality, options and workflow behavior: all of it. This is a clone/port,
not a smaller Magit-inspired Git interface. The user supplied
https://github.com/magit/magit specifically as the source reference.
Global entry points must use Fred's leader followed by `m`; `Space m s` opens
status and `Space m p` pushes. Preserve that requirement while bringing over
Magit's menu groups, arguments, toggles, context and command behavior.

The user explicitly authorized continued implementation and commits and said
not to ask for permission or repeatedly ask whether to continue. Complete
reversible implementation/testing work autonomously. This does not authorize
pushing to remotes or changing the user's repository history as a test.

## Source baseline and parity standard

Baseline source commit: e9ed99c5e3cdd3fab31204f2809ce9d01d672c89.
Read the upstream implementation for every feature being ported. Manuals are
supplementary. Temporary checkout: /private/tmp/fred-magit-upstream; if it is
missing, recreate it from the repository and check out the pinned revision.
Do not silently replace Magit behavior with whatever the similarly named Git
CLI command happens to do. Document behavior differences and keep them open.

`docs/magit-parity.csv` records 514 explicit command declarations and 227
customization options from the pinned Lisp sources. It is a lower bound,
not a complete count of behaviors or a percentage estimate. Generated commands,
section interactions, contextual operations, option menus, hooks and companion
integrations need coverage too. Most source modules remain unported. Full parity
is substantial work, not nearly complete because the common Git verbs work.

For each source feature, track: upstream symbol/menu/option, Fred UI mapping,
behavior implementation, real-Git or UI test evidence, and unresolved differences.
Only credit complete after behavior and menu/options are covered. Partial
implementations remain partial; a wrapper or a passing test for one path is
not full parity. Do not mark an entire feature family done prematurely.

## Existing committed implementation

Branch: feat-magit.
247cbfb: foundational status, file/hunk staging, commit drafts, log, branch picker,
network operations, background jobs, read-only views and terminal restoration.
bf97152: basic workflow menus, stash/tag list inspection, refs, amend without
message editing/fixup, merge/rebase/cherry-pick/revert with continuation guards.
These are useful partial implementations, not the finished port.

`notes.md` belongs to the user and is intent-to-add; do not accidentally include
it in implementation commits. Its original Magit repository link is the scope.

## Current work and findings from upstream

Continue unfinished workflow tasks in
`docs/superpowers/plans/2026-10-03-magit-workflows.md`, then execute the broader
source parity ledger. Current changes add typed stash identities (OID and reflog
selector), index-preserving apply/pop, failed-pop retention, real stash patch
inspection and stale-selection rejection. UI drop confirmation and session/PTY tests now cover this path. Menu panels
replace single-line summaries, with source keys and initial argument toggles.
Editable amend/reword use separate HEAD-pinned drafts, preserve ordinary drafts
and options across recovery, and guard the HEAD again before Git execution.
Further menu groups/options remain open in the parity ledger.

`magit-stash.el` shows that apply/pop first preserve the saved index and have
fallback behavior, conflict handling, separate worktree/index/untracked sections,
custom stash creation plumbing, snapshots, branch transforms and patch export.
Fred's prior `git stash apply` and `git stash push` wrappers are not equivalent.
Menus must eventually include upstream groups and argument switches, not just
single-line lists of whichever subset Fred currently has.

Other substantial remaining families: region staging/discard/reset, complete
status/refs/log/diff/blob and revision navigation, interactive history rewriting,
conflict comparison/resolution, remaining commit options, blame/reflog/bisect, complete
remote/refspec/network options, tags/notes, worktrees/submodules/subtrees,
patch/mail, clone/init, sparse checkout, ignore rules, repository maintenance,
process/history UI, file dispatch and configuration/customization equivalents.
Nothing in this list is excluded by default; adapt Emacs integration to Fred.

## Implementation and validation constraints

Use existing Rust/std and Fred facilities; no shell interpolation. Keep raw
paths and stable objects separate from rendered labels. Escape repository
control characters. Never overwrite unsaved source buffers. Git operates on
saved files and the index. Preserve drafts on failures and recovery.
Background prompt results must not replace newer key or paste input.
Use worktree-local Git paths for active operation metadata. Serialize mutations,
refresh on failures and preserve diagnostics. Retain Git hook/auth terminal
handoff and interruption behavior.

Test first with temporary real Git repositories; destructive tests never target
the user's actual repo. Add session and PTY coverage for UI behavior. Before
completion/commit: cargo test, cargo fmt --check, cargo clippy --all-targets --
-D warnings -A clippy::too_many_arguments (existing draw_picker warning),
cargo build --release. The prior full suite had 356 passing/18 ignored tests;
new work requires fresh verification. One independent final review is required
by the execution-plan skill. Do not delegate routine source exploration.

## Independent review and open safety/behavior work

The commit-menu option divergence found in review was reproduced and fixed:
menus synchronize from draft arguments, and toggles update those same execution
arguments. A real failing hook and signoff trailer verify both directions.

Stash reflog selection is checked immediately before deletion, but the comparison
and Git deletion are separate processes. A concurrent external Git mutation in
that final window can still renumber entries. Stronger atomic protection remains
open; the current checks are not an atomic guarantee. Upstream also uses separate
calls, but that is not a reason to conceal the limit.

Draft text is persistent and modes/options survive in-session recovery. Commit
option metadata is not yet serialized for process-crash/restart recovery. Reopening
amend/reword through its command reconstructs mode from the HEAD-specific path;
persisting/restoring options and older draft context still needs implementation.
This is unfinished parity work, not an exclusion from the goal.

Reword with --all is rejected by Git (--only conflicts with --all); the pinned
upstream passes this combination too. Preserve its failure and draft retention
rather than silently including the staged tree in a message-only operation.

During full-suite verification, an existing definition-picker race was reproduced:
a worker could finish between row rendering and automatic-jump selection. The
jump now requires a matching complete results snapshot. A deterministic test
catches the former choice from a stale partial list.

The fullscreen PTY test also waited for the first content chunk before asserting
the status line. It now waits for the status text too, so the assertion checks a
complete initial frame instead of a partially received terminal write.

Latest verification of this batch: 376 tests passed, 18 ignored; cargo fmt
--check, git diff --check, Clippy (with the documented existing argument-count
allowance), and cargo build --release passed. This verifies the implemented
batch, not full Magit parity. Continue using the source inventory for remaining
menus, arguments, behavior, integrations, and customization options.
