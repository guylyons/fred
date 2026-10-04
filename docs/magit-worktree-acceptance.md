# Worktree and reflog acceptance scenarios

Source: pinned magit-worktree.el, magit-reflog.el, magit-log.el:545-547.
Entries: Space m Z (worktree), Space m l r/O/H (reflog).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| Z b / c | Sibling default directory, named directory, new branch; option-like names rejected | worktree_suffixes_create_move_delete_and_visit |
| Z m / k | Main worktree protected; locked refused; dirty needs typed yes; prune after removal | same test |
| Z g | Status of another worktree; unknown paths rejected | same test |
| l r / O / H | Reflog views (256 entries), RET shows commit, invalid refs rejected | magit_reflog_views_list_entries_and_visit_commits |
| Worktrees status section, other directory-reading functions, trash deletion, reflog margins/labels | | Open |

Independent review (October 4): delete and move have no default (upstream
offers only the worktree at point) and visit skips the current worktree;
deleting or moving the worktree you are in runs from the primary and reopens
status there; the dirty check fails closed and --force is passed only after
the typed yes; relative directory answers resolve against the sibling parent;
operation messages (e.g. stash index warnings) reach the user; stash at point
is identified by object id; format-patch never follows symlinks (create_new).
Bisect: revisions precede "--" in bisect start (found by a real-Git test).
