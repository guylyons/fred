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

Customization configuration is wired for 21 upstream options (all partial).
Next: repository-list commands and their directory/column options, then remaining
customization behavior, complete menu/section mapping and the source-linked
acceptance scenario matrix. Keep incomplete command behavior and safety/recovery
gaps open; command-name coverage alone does not close a family.

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

October 4 section checkpoint: Magit buffers gain evil-collection's section
movement (C-j C-k, gj gk ] [ M-j M-k, gh) and folds (za zo zc zO zC z1-z4
zr); z is a prefix in Magit buffers since Fred's Vim has no z commands.
Ledger 526 partial / 247 missing commands. Score remains 3%.

October 4 commit-message checkpoint: drafts gain git-commit's trailer keys
(C-c C-s C-a C-m C-r C-t C-o C-p M-i, C-c C-i any trailer) via git
interpret-trailers, and message history (M-k/gk, M-j/gj). Fix: Ctrl/Alt
section movement keys never matched (Key::char is None with modifiers); C-j
in diff buffers visits the worktree file. Ledger 539 partial / 234 missing
commands. Score remains 3%.

October 4 discard-safety checkpoint: discard follows upstream's per-status
table and never deletes or overwrites work it did not show (review fixes);
todo buffer text-mode toggle; hunks are single sections for movement; levels
1-4 / M-1..M-4 replace the z fold keys, which evil-collection leaves off by
default (the user's config keeps defaults); ys/yb/yr and gR.

October 4 arguments checkpoint: fetch -u/C, pull -A/f/F/C, push -o/C, merge
-X -b -w -A -S +s, tag -u, bisect =o =n, and the stash push menu (z P).
Ledger 571 partial / 202 missing commands. Score remains 3%.

October 4 M-x checkpoint: `:Magit NAME` runs upstream commands by name (the
Fred equivalent of M-x for commands upstream leaves unbound): menus, common
suffixes, fetch-all-prune/no-prune, push-implicitly, refresh-all and section
levels. Ledger 575 partial / 198 missing commands. Score remains 3%.

October 4 run checkpoint: magit-run (!) and magit-git-command (| and Q) run
git subcommands in the terminal; o resets quickly (mixed); x on a conflicted
file checks out a side (magit-checkout-stage); remote set/unset-head by name;
M-w copies a blame chunk's hash. Ledger 585 partial / 188 missing commands.
Score remains 3%.

October 4 file-dispatch checkpoint: F t traces the definition at point (git
log -L), F M merged, F G status, F e edits the commit that added the line.
Ledger 591 partial / 182 missing commands. Score remains 3%.

October 4 status/process checkpoint: Space m j status jump (fu fp pu pp),
:Magit magit-parent-status, and a process buffer (` / Space m $) listing the
repository's terminal Git commands with their errors. Ledger 598 partial /
175 missing commands. Score remains 3%.

October 4 wip checkpoint: magit-wip commit (index and worktree refs through a
temporary index), wip logs (l i, l w, current) and purge; magit-wip-mode's
automatic saving is not ported. Ledger 604 partial / 169 missing commands.
Score remains 3%.

October 4 magit-mode-map checkpoint: Magit buffers take upstream's single
keys as evil-collection's defaults leave or move them (b c d f l m r t z ...,
p push, O reset, ' submodule, " subtree, _ revert, X untrack, h/? dispatch,
L log refresh; - and a on a commit revert-no-commit and cherry-apply), in
addition to the Space m contract. Ledger 608 partial / 165 missing commands.
Score remains 3%.

October 4 apply/smerge checkpoint: a and - apply and reverse hunks from diff,
commit and stash buffers; C-c ^ u/b/l/a/RET keep a side of the conflict at
point in a file buffer. Ledger 614 partial / 159 missing commands. Score
remains 3%.

October 4 ediff checkpoint: magit-ediff is adapted to Git's own tools (Fred
has no Ediff): E opens the menu and e is dwim; comparisons run git difftool
and conflicts git mergetool in the terminal with the user's configured tools;
magit-ediff-stage has no equivalent. Also gd, diff file-filter toggle and
diff-unmerged. Ledger 654 partial / 119 missing commands. Score remains 3%.

October 4 small-commands checkpoint: dired stage/unstage/log, push notes
ref (p n), delete shelved branch, commit-buffer jumps, blame visit-file,
half commit limit and version by name; review fixes for hunk apply
(raw output, confirmation, renames) and smerge's C-c. Ledger 669 partial /
104 missing commands. Score remains 3%.

October 4 branch/network checkpoint: branch -m/-r reach every checkout
(lifting the dirty check for --merge), B updates the default branch from
the branch and remote menus, and :Magit gains branch-or-checkout,
checkout-remote-ref (terminal fetch, then FETCH_HEAD via a chained Git
command), pull-into-upstream and push-to-remote. Ledger 676 partial / 97
missing commands. Score remains 3%.

October 4 apply/commit-message checkpoint: u reverses committed changes in
the index (magit-unstage-committed), C adds a changelog stub to the commit
draft, the absorb and autofixup transients carry their arguments (c x opens
autofixup's, as C-u x does upstream), F s/u read files when no file is
visited, drafts save messages with C-c M-s and insert GNU/plain changelogs
built from Git's hunk-header function context, and ChangeLog files get
dated entries. Ledger 693 partial / 80 missing commands. Score remains 3%.

October 4 margin/log-select checkpoint: log, reflog, stash, cherry, refs
and status buffers get magit-margin's right-aligned author and age (L, l,
d, x; per-buffer defaults), diffs gain hunk fontification (on) and word
refinement (t), commit fixup/squash/absorb and rebase i/m/w/k pick their
commit in magit-log-select-mode when none is at point, and logs gain
=g, C-c C-r references, move-to-revision, history (C-c C-b/C-f) and M-Tab
diff cycling. Ledger 714 partial / 59 missing commands. Score remains 3%.

October 4 run/tools checkpoint: Space m ! gains shell commands and the
gitk/git gui/mergetool --gui launchers; the mergetool menu gains its six
variables; f m opens magit-fetch-modules' transient; abort-dwim, wip-mode
(after save and after Git commands), per-file wip commits, auto-revert of
unmodified buffers, git-commit-mode for message files Git opens in Fred,
debug/record/profiling toggles feeding the process buffer, & shell
commands, dired am, update-index, and dash-for-space in branch names.
Ledger 757 partial / 16 missing commands. Score remains 3%.

October 4 history-tools checkpoint: magit-pop-revision-stack (C-c C-w;
ys/yb push), magit-reshelve-since (log-select, plumbing rewrite),
C-c C-t / C-c C-e from hunks, process-kill for background Git, and the
menu-bar menus mapped to transients. Ledger 765 partial / 8 missing
commands (repolist, which needs magit-repository-directories, and two
Emacs-only commands). Score remains 3%.

October 4 customization checkpoint: `[magit]` TOML settings wire confirmation,
counts, margins, hunk styling, squash selection, autorevert/WIP and diagnostic
defaults ([scenarios and limits](magit-options-acceptance.md)). Independent review
corrected action misclassification and margin overflow. Ledger: 765 partial / 8
missing commands; 25 partial / 212 missing options. No acceptance gate closes;
score remains 3%. Next: repository list, remaining customization behavior and
baseline mapping/scenario gates.

October 5 repository-list checkpoint: magit-list-repositories lists the
repositories under magit-repository-directories with upstream's default
columns (name, version, B<U, B>U, path), configurable columns, flags and
sort key; RET status, m/u marks, f fetch (marked or, confirmed, all) and
5 find-file. Ledger 771 partial / 2 missing commands (Emacs-only
git-commit-elisp-text-mode and magit-emacs-Q-command). Score remains 3%.

October 5 options checkpoint: 35 more options wired (commit style checks,
executables, wip namespace/merge, trash, rename push target, save-repository
buffers, hooks as shell commands, rebase todo options and more; see
magit-options-acceptance.md). Ledger 771 partial / 2 missing commands;
64 partial / 173 missing options. Score remains 3%.

October 5 options checkpoint 2: refs display options, status initial
position/file limits/refresh, commit ask-to-stage (drafts now refuse or ask
when nothing is staged, as upstream), extend/reword committer dates (via a
new GitInvocation env), and magit-published-branches (upstream default
origin/master). Ledger: 80 partial / 71 missing / 86 n/a options.
Score remains 3%.

October 5 revision/clone checkpoint: revision buffers follow
magit-revision-mode (ref labels and hash, headers format, Parent/Merged/
Contained/Follows/Precedes, unindented message, notes, diffstat, diff);
clone options (name-to-url, url formats, pushDefault, remote HEAD,
default directory; magit-clone runs the regular clone at once unless
magit-clone-always-transient); whitespace painting, visit options,
process hint and timestamps. Options: 98 partial / 51 missing / 88 n/a.
Score remains 3%.

October 5 status-sequence checkpoint: status buffers now show upstream's
in-progress sections — "Merging X:" with incoming commits, "Rebasing X onto
Y" (remaining todo, stop/join/work/gone, done, onto), "Applying patches",
"Cherry Picking"/"Reverting", and the bisect output, Bisect Rest (with
magit-bisect-show-graph) and Bisect Log sections. More options: remote-add
pushDefault, module gitdir trashing, patch save arguments, pull-or-fetch.
Options: 125 partial / 22 missing / 90 n/a. Score remains 3%.

October 5 stopping checkpoint (user request): every upstream command and
option now has a Fred mapping or an explicit n/a reason. Ledger: commands
771 partial / 0 missing / 2 n/a; options 144 partial / 0 missing / 93 n/a.
Last batch: ref-at-point (RET behaviors in refs buffers, start-point
defaults with magit-prefer-remote-upstream), hash words in revision
messages, indentation highlighting. Partial means implemented with the
limits recorded per row, not accepted. Score remains 3%: no roadmap row has
passed its five acceptance gates (source/option mapping, menus, behavior,
real-Git/UI/error scenarios, independent review). Next: the baseline UI map
and scenario matrix, then per-track acceptance reviews starting with
status/diff/commit.
