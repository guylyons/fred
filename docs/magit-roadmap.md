# Magit full-parity roadmap

Target: the complete user-visible behavior of upstream Magit at
e9ed99c5e3cdd3fab31204f2809ce9d01d672c89, adapted to Fred with `Space m`.
Nothing below is excluded from the port.

## Percentage and accounting

**Verified roadmap completion: 3 / 100 points = 3%. Remaining: 97%.**
This deliberately measures accepted deliverables, not elapsed effort, lines of
code, or how many familiar Git verbs run. Existing partial functionality is
listed below but earns no full-family credit. This is a conservative completion
measure, not a delivery-time estimate. Weights are planning allocations; change
them only with an explicit explanation and preserve the prior score.

The baseline's five one-point deliverables are: pinned source checkout (done),
explicit command/customization inventory (done), generated commands and companion
mode inventory (done; runtime snapshot and extractor), complete menu/key/section mapping (open), and an
acceptance scenario matrix linked to every ledger entry (open).

For every other row, its points are awarded together only when all five gates
pass: full upstream source/option mapping; navigable menus and contextual keys;
all behaviors implemented; real-Git/UI/error/recovery scenarios verified; and
independent review with material findings fixed. A partial row stays at zero.
This prevents arbitrary fractional credit for untested branches. Discovery of
additional behavior reopens its row and lowers the score until it is covered.

| Order | Track | Points | Accepted | Existing partial work / remaining acceptance |
| --- | --- | ---: | ---: | --- |
| 1 | Source and acceptance baseline | 5 | 3 | Pin, expanded command/option and companion-mode inventory done; full Fred UI map and scenario matrix open |
| 2 | Transient menus and section framework | 7 | 0 | Basic panels/toggles; remaining arguments, persistence, help, navigation, selection and context |
| 3 | Status and refresh | 7 | 0 | Basic status/conflicts; all upstream sections, hooks, contextual operations, auto-refresh |
| 4 | Diff, staging, discard, reset and blobs | 10 | 0 | File/hunk stage/unstage; region operations, diff variants/options, reverse/discard/reset, revision/blob navigation |
| 5 | Commit and message editing | 8 | 0 | Create/amend/reword/fixup; remaining modes/options, editor integrations, trailers, crash metadata |
| 6 | Stash and snapshots | 6 | 0 | List/show/apply/pop/drop; source creation plumbing, snapshots, transforms, fallback negotiation, refs/autostash |
| 7 | History rewriting and conflicts | 9 | 0 | Basic merge/rebase/cherry-pick/revert; interactive sequencer editing, all variants/options, conflict workflows |
| 8 | Log, reflog, refs and revision inspection | 6 | 0 | Simple log/tag/branch lists; full arguments, filtering, graph, ranges, navigation and refs views |
| 9 | Branch and tracking workflows | 6 | 0 | Basic local branch verbs; start points, tracking, spin-off/out, rename/delete options and contextual defaults |
| 10 | Remotes, fetch, pull, push and refspecs | 7 | 0 | Default network verbs; complete menus, remote editing, publication variants, refspecs and options |
| 11 | Tags and notes | 4 | 0 | Lightweight tags; annotations/signing, release defaults, deletion/pruning and notes workflows |
| 12 | Blame and bisect | 4 | 0 | All modes, navigation, options, bisect sessions and automation |
| 13 | Worktrees, submodules and subtrees | 4 | 0 | All workflows, linked-worktree contexts and nested repository operations |
| 14 | Repository/file dispatch and maintenance | 3 | 0 | Clone/init, repository lists, ignore, sparse checkout, file actions and maintenance |
| 15 | Patches, mail and bundles | 3 | 0 | Format/apply/export, mail-related editor equivalents, bundles and interruption handling |
| 16 | Configuration, process UI and integrations | 5 | 0 | 237 options plus hooks; process/history diagnostics, companion modes, WIP, autorevert and Fred equivalents |
| | Total | 100 | 3 | |

## Execution queue

Submodule, subtree and patch/am batches done. Bundles and clone done.
Refs, sparse checkout and gitignore done. Next: the remaining diff and
log commands (reference movement, log-select), then commit menu remainder,
subtrees, patches/bundles, clone, refs view, sparse checkout and gitignore,
recording each accepted gate's evidence here. The baseline's UI map and
scenario matrix remain open.

At every implementation commit, update the command ledger and this queue. Report
the same score even when useful partial functionality lands but no acceptance
row closes. Never describe a batch passing tests as overall full parity.

Evidence: [source ledger](magit-parity.csv), [scope and continuation notes](magit-port-notes.md).

October 4 checkpoint: worktree-only stash and three snapshot commands now have
source plumbing, menus and regression coverage. Full suite: 385 passed / 18
ignored; format/diff checks, Clippy and release build passed. Independent review
found two Important defects; both were reproduced and fixed. The stash row
remains partial, so the accepted score remains 2%. Next: generated-command and
companion inventory, full menu/section map, and acceptance scenario matrix.

October 4 source/binding checkpoint: a clean Emacs process loaded the pinned
checkout and captured 773 commands, 237 options, 51 transient prefixes, 565 menu
entries, 53 keymaps and 25 modes. Provenance and representative generated/menu
entries were checked; the generated-command/companion-mode inventory point is
accepted. Score: 3%. Context-created dynamic layouts and version/platform
branches remain a declared inventory limit and require source review in the UI
mapping gate. This credit covers discovery, not implemented functionality.

The user's exact leader contract is [recorded here](magit-user-bindings.md).
b/c/r now route to branch/commit/revert menus, R is a nonconflicting rebase
entry, and B is reserved for unported blame. Stash-view k again moves up; x/d
request a confirmed drop. Next: complete the Fred UI mapping and acceptance
scenario matrix, prioritizing B/L/d/i and menu semantics for network/log entries.

October 4 continuation: resumed after the integration stop. File history and a
partial log prefix are now in progress: Space m L, log l/h, follow-renames,
filtered refresh and commit inspection/return. This is a cross-track UI mapping
prerequisite, not acceptance of the log family or either open baseline gate.
Score remains 3%. Next finish source-linked UI mapping and acceptance scenarios;
continue missing B/d/i and full log/network menu semantics. The file-log gaps
include region tracing, revision/blob context and inherited arguments/settings.

October 4 network checkpoint: committed the file-log slice (625f834) after
fixing two failing tests. Space m p/P/f are now source-style push/pull/fetch
menus with upstream argument switches and configured/elsewhere/other/refspec/
tag suffixes ([scenarios](magit-network-acceptance.md)). Row 10 remains partial
(remote editing, refspec configuration, completion, prefix-arg variants, hidden
levels, dynamic descriptions), so the accepted score stays 3%. Ledger: 80
partial / 693 missing commands. Next: d diff menu, i init, B blame, then the
UI map and acceptance-matrix baseline gates.

October 4 diff checkpoint (worktree /Users/guy/github/fred-magit): Space m d
is a source-style diff menu with ten arguments, d/r/p/u/s/w/c/t suffixes,
seeded defaults and buffer-argument seeding ([scenarios](magit-diff-acceptance.md)).
Pull -r and diff choices now share one cycling Choice option; prompts are a
generic Ask chain. Row 4 stays partial, so the score remains 3%. Ledger: 94
partial / 679 missing commands. Next: i init, B blame, then baseline gates.

October 4 init checkpoint: Space m i follows magit-status.el:252-277 (directory
prompt, nested/reinitialize confirmation, status afterwards), tested by
magit_init_creates_repository_and_confirms_nesting. Score remains 3%; ledger
95 partial / 678 missing. Next: B blame, reviewed together with init.

October 4 blame checkpoint: Space m B ports blame-addition/echo with Fred
gutter adaptations of the three default styles ([scenarios](magit-blame-acceptance.md)).
Every user-binding key now opens its source menu or workflow. Row 12 stays
partial (blob buffers, bisect), so the score remains 3%. Next: revision/blob
buffers (unblocks removal/reverse/recursive blame and the open file-log gap),
then the full UI map and acceptance-matrix baseline gates.

October 4 blob checkpoint: revision/blob buffers, blob navigation, find-file,
file dispatch (Space m F) and removal/reverse/recursive blame
([scenarios](magit-files-acceptance.md)). Ledger 120 partial / 653 missing;
score remains 3%. Next: file actions in the file dispatch, then status
sections and the remaining families in ledger order.

October 4 status checkpoint: status headers, stash and log sections and the
user's evil-collection jumpers ([scenarios](magit-status-acceptance.md)); file
action safety re-review fixes. Ledger 134 partial / 639 missing commands, 5
options partial (defaults) / 232 missing. Score remains 3%.

October 4 branch checkpoint: Space m b uses upstream keys b/l/c/s/n/S/m/x/k
([scenarios](magit-branch-acceptance.md)); status review fixes applied.
Ledger 139 partial / 634 missing commands. Score remains 3%.

October 4 merge checkpoint: Space m M ports magit-merge
([scenarios](magit-merge-acceptance.md)); tag review fixes. Ledger 156
partial / 617 missing commands. Score remains 3%.

October 4 reset checkpoint: Space m X ports magit-reset
([scenarios](magit-reset-acceptance.md)); merge review fixes. Ledger 163
partial / 610 missing commands. Score remains 3%.

October 4 remote checkpoint: Space m M ports magit-remote; Space m m is merge
as in magit-dispatch ([scenarios](magit-remote-acceptance.md)); reset review
fixes. Ledger 170 partial / 603 missing commands. Score remains 3%.

October 4 rebase checkpoint: Space m R ports magit-rebase with interactive
todo editing ([scenarios](magit-rebase-acceptance.md)); Git can now use Fred as
its editor in the terminal (GitInvocation.editor); cherry-pick review fixes.
Ledger 199 partial / 574 missing commands. Score remains 3%. Next: transient
popup rendering (user request), then stash variants and the commit menu.

October 4 transient checkpoint (user request): Magit menus now render as a
transient popup above the status line — argument groups first with switches
highlighted when on and choice values inline, then action groups in columns
(ui::render::transient_lines; test magit_menus_render_as_transient_popups).
Row 2 remains open: dynamic descriptions, inapt/hidden suffixes and levels,
set/save of arguments, help, and free-form option values.

October 4 log checkpoint: Space m l ports magit-log's argument groups and
suffixes (o other, u related, L/b/a/R, B/T matching, m merged without
git-when-merged) with --graph/--decorate rendering, = and + commit limits in
log buffers ([scenarios](magit-log-acceptance.md)). Menus now read free-form
option values (transient-option): pressing a set option unsets it. `l l` from
a plain file buffer no longer filters to that file (upstream); Space m L does.
Bisect (Space m G) and worktree/stash review fixes landed in 2ee8bdd.
Ledger 277 partial / 496 missing commands. Score remains 3%.

October 4 shortlog/cherry checkpoint: Space m l s (shortlog menu), Space m Y
(cherry), C-c C-n in logs; log review fixes (--reverse drops --graph, patch
text is never parsed as a commit, typed option values survive in a log
buffer's menu, + without a limit sets 256 and 0 means none). Ledger 288
partial / 485 missing commands. Score remains 3%.

October 4 submodule checkpoint: Space m o ports magit-submodule (add,
register, populate, update, sync, unpopulate, remove with dirty-module
safety, list and visit) ([scenarios](magit-submodule-acceptance.md)). Ledger
305 partial / 468 missing commands. Score remains 3%.

October 4 subtree checkpoint: Space m O ports magit-subtree (import add/add
commit/merge/pull, export push/split) with its options; prefixes must lie
inside the repository ([scenarios](magit-subtree-acceptance.md)). Ledger 322
partial / 451 missing commands. Score remains 3%.

October 4 patch checkpoint: Space m W ports magit-patch (create with its
mail/patch/diff arguments, apply, save, request-pull) and magit-am (apply
patches/maildir; continue, skip, abort while applying)
([scenarios](magit-patch-acceptance.md)). Submodule review fixes (33938d6).
Ledger 360 partial / 413 missing commands. Score remains 3%.

October 4 bundle/clone checkpoint: Space m & ports magit-bundle (tracked
bundles keep upstream's tag format, so Emacs and Fred share them) and Space m
C ports magit-clone (dispatch C; the duplicate commit binding moved to c
only). ([scenarios](magit-bundle-clone-acceptance.md)). Ledger 391 partial /
382 missing commands. Score remains 3%.

October 4 refs checkpoint: Space m y ports magit-show-refs (branch
description, local branches with upstream tracking, remotes with their urls,
tags with messages, focus column and commit counts, for-each-ref filters)
([scenarios](magit-refs-acceptance.md)); patch review fixes (bbd1264). Ledger
403 partial / 370 missing commands. Score remains 3%.

October 4 ignore checkpoint: Space m I ports magit-gitignore (toplevel,
subdirectory, private, global rules; skip-worktree and assume-unchanged) and
Space m > ports magit-sparse-checkout ([scenarios](magit-ignore-acceptance.md)).
Ledger 419 partial / 354 missing commands. Score remains 3%.

October 4 commit-menu checkpoint: Space m c gains -v (seeded), -A, -D, -S,
-C reuse (commit at once) and -c reedit (draft from that message), d
reshelve, R reword past, x autofixup and X absorb modules. Ledger 430
partial / 343 missing commands. Score remains 3%.

October 4 diff-refresh checkpoint: Space m D ports magit-diff-refresh (shared
arguments, g, switch range type, flip revisions); the diff menus gain -D -U
-C -H -R =m =w; = + ~ change a diff buffer's context; bundle, clone, ignore
and refs review fixes (9f79b8a; clone now runs in the terminal). Ledger 446
partial / 327 missing commands. Score remains 3%.

October 4 diff-visit checkpoint: Enter on a line of a diff, commit or stash
buffer visits the blob at that revision (old side for removed lines) or the
worktree file at that line; C-j visits the worktree file. Diffs use fixed
a/ b/ prefixes so diff.noprefix cannot break this. Ledger 452 partial / 321
missing commands. Score remains 3%.

October 4 file-diff checkpoint: Space m F d ports magit-diff-buffer-file, C-c
C-d in a draft ports magit-diff-while-committing, and the diff menus gain the
-- file limit. Ledger 455 partial / 318 missing commands. Score remains 3%.

October 4 sequence-arguments checkpoint: rebase gains -f, -x, -S, +s;
cherry-pick and revert gain -m, -S, +s; a test now keeps every menu's keys
unique; ledger rows for switches already present are recorded. Ledger 474
partial / 299 missing commands. Score remains 3%.

October 4 configure checkpoint: Space m b gains o orphan, w/W worktrees, C
configure (branch and repository variables), h/H shelve; the remote menu
gains C configure (remote variables) and z unshallow. Variables cycle or
prompt and report their new value; the menu does not yet show live values.
Ledger 494 partial / 279 missing commands. Score remains 3%.

October 4 apply checkpoint: status buffers gain x discard, - reverse, S stage
all modified and U unstage all (evil-collection keys), each confirmed as
upstream's magit-confirm defaults. Ledger 498 partial / 275 missing
commands. Score remains 3%.

October 4 todo-buffer checkpoint: the rebase todo buffer gains upstream's
overriding keys (c w m S F A b z l t y M M, M t, Enter/SPC show commit); u
stays undo as evil-collection binds it; commit/diff review fixes (10eb821).
Ledger 512 partial / 261 missing commands. Score remains 3%.
