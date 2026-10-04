# Reset menu acceptance scenarios

Source: pinned magit-reset.el. Entry: Space m X (upstream X; not in the user's
leader contract, added as a nonconflicting key).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| m / s / h / k | HEAD/index/worktree effects, keep refuses to lose local edits, option-like/unknown revisions rejected | reset_suffixes_move_head_index_and_worktree_as_named |
| i / w | Index-only and worktree-only via temporary index | same test |
| h safety | Refuses to replace untracked files the target tracks (Fred addition) | same test |
| b / f | Branch reset and file checkout reuse their menus | branch/file tests |
| reset-quickly, HEAD~ message saving (git-commit-save-message) | | Open |
