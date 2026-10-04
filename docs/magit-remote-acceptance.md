# Remote menu acceptance scenarios

Source: pinned magit-remote.el. Entry: Space m M (upstream dispatch key; merge
moved to Space m m as in magit-dispatch).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| a | Url default from origin, pushDefault ask-if-unset, -f fetch via terminal, invalid/duplicate names | remote_suffixes_add_rename_remove_and_prune_refspecs |
| r / k | Push variables follow a rename or are cleaned on removal | same test |
| P | Stale refspec detection via ls-remote, confirmation, tracking refs removed | same test |
| p | remote prune via the terminal | menu routing |
| Direct variables, C configure, d u update default branch, z unshallow | | Open |

Independent review (October 4): the suggested url is now a visible default in
its own prompt; background git calls set GIT_TERMINAL_PROMPT=0 and background
network reads (ls-remote) add ssh BatchMode on top of core.sshCommand, so they
fail instead of prompting on the TUI's terminal; stale-refspec pruning keeps
going past failures and reports them; ~ alone expands. Open (shared with
upstream): unqualified/negative refspecs and overlapping destinations in
prune-refspecs.
