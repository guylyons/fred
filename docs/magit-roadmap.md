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

The in-flight stash batch is committed. Next finish the baseline's two remaining
deliverables: full Fred UI mapping and the acceptance scenario matrix, using the
user's exact leader contract. Then work through rows in order, preserving
cross-track prerequisites and recording each accepted gate's evidence here.

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
