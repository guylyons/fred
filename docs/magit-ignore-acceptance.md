# Magit gitignore and sparse checkout acceptance

| Scenario | Upstream | Fred | Evidence |
| --- | --- | --- | --- |
| Gitignore menu | dispatch i; t s p g; w W u U | Space m I (i is the user's init) | menu_entries('g') |
| Rules | appended on their own line, backslashes doubled, shared files staged | Same; symlinked targets refused | magit::tests::gitignore_rules_skip_worktree_and_sparse_checkout |
| Pattern default | /FILE if untracked, else *.EXT | Same (no completion list) | same test |
| Skip worktree / assume unchanged | update-index flags on a file | Same; untracked files refused | same test |
| Sparse menu | > ; -i; e only when disabled, d r when enabled, s a | Same, checked at run time | same test |
| Set/add | auto-enable cone mode | Same | same test |

Independent review (October 4): subdirectory rules need an existing directory
inside the worktree (absolute paths allowed; symlinks leaving the worktree
refused) and nothing is created for a mistyped directory; Git's warnings from
sparse-checkout are shown; enable/disable/reapply always run Git as upstream
does. The refs view resolves tag commits with one for-each-ref.
