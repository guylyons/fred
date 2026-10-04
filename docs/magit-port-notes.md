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

The first `docs/magit-parity.csv` scan recorded 514 apparent commands and 227
options. The October 4 runtime expansion corrected two internal-helper false
positives and expanded the inventory to 773 commands and 237 options. It is a lower bound,
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

## October 4 continuation

The user explicitly requires a percentage-based roadmap and execution through it.
`docs/magit-roadmap.md` allocates 100 acceptance points across the full scope.
Initial accepted score is 2%: pinned source and explicit inventory. Partial
families have zero accepted points until their source/menu/behavior/tests/review
gates all pass. This is conservative verified-deliverable progress, not an effort
estimate or a claim that the existing useful code does not exist.

Current source-based batch adds `Space m z w` worktree-only stash and `Z/I/W`
snapshots. Magit's pre-stash-index base, index parent, optional untracked parent,
private worktree tree and reflog storage are reproduced with Git plumbing.
Snapshots never clean/reset files; worktree-only stash publishes its saved
object before restoring tracked files from the index and cleaning requested
untracked/ignored files. Real-Git tests cover separate staged/unstaged trees,
untracked parents, literal paths, binary/deleted files and linked worktrees.
Mac filesystems reject non-UTF8 filenames; that physical-path case runs on Linux,
while both platforms test leading dash, wildcard and newline path bytes.

Remaining stash gaps include converting both/index/keep-index from Git wrappers,
merge-state confirmation,
transforms/export, autostash/custom refs, apply 3way/reject negotiation and richer
context/options. The new commands remain partial in the source ledger.

Review correction: use the staged index as the diff base when populating a
snapshot's worktree tree. The pinned upstream uses HEAD for the both-sides path
and can omit an unstaged reversal of a staged edit; Fred intentionally corrects
that edge so the saved worktree really matches disk. A real-Git regression
failed with staged content in the root tree and passed after this correction.

Completed background stash/snapshot mutation outcomes must survive switching
buffers. Their errors/completion are handled separately from obsolete read or
draft requests. A delayed rejecting reference-transaction hook reproduces the
former silent failure and verifies the diagnostic without replacing the newly
selected source buffer. Completed terminal and source-plumbing mutations mark
matching generated views dirty; a parked stash list refreshes in the background
when shown again, with a regression test. Other background mutation result kinds
still need the same comprehensive lifecycle audit in the framework track.

This continuation's final verification: 385 tests passed, 18 ignored; formatting,
diff whitespace checks, Clippy with the existing allowance and release build
passed. The source ledger now has 46 partial and 468 missing explicit commands;
227 customization options remain missing. No whole family is yet accepted as
full parity. Continue from the roadmap's next baseline deliverables.

## User binding and stop/integration checkpoint

Read the user's `~/.emacs.d/lisp/gl-keys.el`, gl-tools.el, gl-evil.el and
gl-completion.el, plus the installed Evil-collection Magit binding source.
`docs/magit-user-bindings.md` records the authoritative exact leader contract.
b/c/r now open branch/commit/revert menus; R is an extra rebase entry; B is
reserved for still-missing blame. Creating/submitting a commit is Space m c c;
branch checkout is Space m b b. Stash-view k moves up; x/d ask to drop.
Network/log menus and B/L/d/i remain incomplete.

The read-only tools/magit-source-inventory.el helper runs a clean --batch -Q
Emacs without initializing packages or reading user configuration. It loads the
pinned Magit checkout and dependency paths, capturing commands/options, menus,
raw inherited keymap candidates and companion modes. Independent review
reproduced the artifact exactly and confirmed user-init-file was nil. Keymap
shadow resolution and Evil-collection overrides belong in the open UI mapping
gate. The generated inventory gate raises the roadmap's accepted score to 3%,
with 97% remaining. Partial command counts remain 46 / 773.

The user requested a stopping checkpoint: build, install, commit, push and merge
the other worktree. Only other worktree: /Users/guy/github/fred-explain on
feat/ai-explain. It contains the explanation overlay feature; commit it there,
merge into feat-magit, test the combined tree and install/push feat-magit. Keep
both worktrees and the user's notes.md. Do not start another parity increment
after this integration checkpoint.

Review of the explanation feature found invisible replies for full-viewport or
multiscreen selections. Correct with an overlay fallback and regression before
integration. Deferred minor: Visual K in read-only Magit views remains rejected
by the Magit allowlist; the equivalent ranged :explain command works. Existing
AI cancellation behavior is unchanged; wider full parity stays on the roadmap.
