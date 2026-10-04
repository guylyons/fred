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
