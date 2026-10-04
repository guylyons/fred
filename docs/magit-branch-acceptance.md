# Branch menu acceptance scenarios

Source: pinned magit-branch.el:196-260 (prefix), :265-480 (checkout/create),
:480-640 (names, spin-off/out, reset), :643-905 (delete, rename, shelve).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| n / c | Create with/without checkout; taken/invalid names and option-like starts rejected | branch_suffixes_follow_magit_branch |
| l | Local checkout, remote-tracking creates branch and sets pushRemote, new name asks for start | same test |
| s / S | Spin-off moves unpushed commits and resets source; spin-out stays and hard-resets | same test |
| m / x | Rename keeps push target; reset other branch via update-ref, current asks "yes" when dirty | same test |
| k | Merged delete, unmerged confirmation, current branch detach/target/abort, remote branch push --delete or local ref removal | same test |
| b | Picker of local and remote branches; typed revision detaches | Session/e2e menu tests; Enter on typed query |
| Direct configure (d/u/r/p, R/P/B), C configure, orphan, remote ref, worktree, shelve, -m/-r | | Open |
| Region selections, prefix arguments, upstream adjustment alist, PR remotes | | Open |

Independent review (October 4): fixed deleting the current branch force-deleting
unmerged commits (merged check now precedes detach/checkout, using
magit-branch-merged-p semantics), reset defaulting to the current branch's
upstream (now the chosen branch's, in a second step), option injection via a
stripped heads/ prefix (Switch and rename), the picker preferring a fuzzy row
over a typed revision, spin-out/reset --hard replacing untracked files the base
tracks (refused before any change), spin-off forcing an upstream, the delete
checkout target (indirect upstream, else main branch), remote names with "/",
remote checkout of a dirty tree, and unsetting pushRemote before a failed delete.
Open: push --delete failure fallback to local ref removal.

Configure (October 4): b C edits branch.<b>.description (Fred as the editor),
the upstream, branch.<b>.rebase and pushRemote, pull.rebase,
remote.pushDefault and the autoSetup variables; choices cycle then unset as
transient's git-variable:choices do. b o creates an orphan branch, b h/H
shelve and unshelve (refs/shelved/DATE-NAME with the reflog). Evidence:
magit::tests::configure_variables_orphan_shelve_and_unshallow.
