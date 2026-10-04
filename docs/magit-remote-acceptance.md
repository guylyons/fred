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
