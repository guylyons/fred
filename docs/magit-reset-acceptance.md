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

Independent review (October 4): the untracked-overwrite guard now covers
directory/file conflicts in both directions (shared with branch reset and
spin-out), worktree reset uses the guard too, the temporary index lives inside
the git dir (magit-with-temp-index) rather than a predictable /tmp path,
commit-draft seeding never writes under any open draft buffer (canonical
paths), and reset prompts default to the current branch. Open: saving HEAD's
message before a non-hard reset to HEAD~ (git-commit-save-message).
Regression: reset_guards_directory_file_conflicts_and_defaults_to_current_branch.
