# Magit full-parity roadmap

Target: the complete user-visible behavior of upstream Magit at
e9ed99c5e3cdd3fab31204f2809ce9d01d672c89, adapted to Fred with `Space m`.
Nothing below is excluded from the port.

## Percentage and accounting

**Verified roadmap completion: 2 / 100 points = 2%. Remaining: 98%.**
This deliberately measures accepted deliverables, not elapsed effort, lines of
code, or how many familiar Git verbs run. Existing partial functionality is
listed below but earns no full-family credit. This is a conservative completion
measure, not a delivery-time estimate. Weights are planning allocations; change
them only with an explicit explanation and preserve the prior score.

The baseline's five one-point deliverables are: pinned source checkout (done),
explicit command/customization inventory (done), generated commands and companion
integration inventory (open), complete menu/key/section mapping (open), and an
acceptance scenario matrix linked to every ledger entry (open).

For every other row, its points are awarded together only when all five gates
pass: full upstream source/option mapping; navigable menus and contextual keys;
all behaviors implemented; real-Git/UI/error/recovery scenarios verified; and
independent review with material findings fixed. A partial row stays at zero.
This prevents arbitrary fractional credit for untested branches. Discovery of
additional behavior reopens its row and lowers the score until it is covered.

| Order | Track | Points | Accepted | Existing partial work / remaining acceptance |
| --- | --- | ---: | ---: | --- |
| 1 | Source and acceptance baseline | 5 | 2 | Pin and explicit inventory done; generated commands, complete UI map and scenario matrix open |
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
| 16 | Configuration, process UI and integrations | 5 | 0 | 227 options plus hooks; process/history diagnostics, companion modes, WIP, autorevert and Fred equivalents |
| | Total | 100 | 2 | |

## Execution queue

Continue the in-flight stash source work first (worktree-only save and the three
snapshot variants), then finish the baseline's three open deliverables. This
short exception to order completes the source work already underway; it does
not award stash-family points early. Then work through rows in order, preserving
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
