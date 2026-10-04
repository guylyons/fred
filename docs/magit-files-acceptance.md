# File, blob and file-dispatch acceptance scenarios

Source: pinned magit-files.el:53-260 (find-file, blob buffers), :452-505
(file dispatch), :507-640 (blob mode, navigation); magit-blame.el:416-560,
:798-860 (removal/reverse, visit other file).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| Blob navigation | worktree -> index (when staged) -> HEAD -> ancestors following renames; successor back to worktree | blob_history_walks_index_commits_and_renames |
| Blob buffers | Read-only, p/n with commit message, "beginning of time", n returns to file, q kills | magit_blob_buffers_navigate_history_and_blame_revisions |
| find-file | Prompts with defaults; revision answer opens blob | same session test |
| Blame on blobs | addition with revision, removal/reverse, recursion into previous blob, "no further history" | same session test; blame_removal_and_reverse_on_revisions |
| Removal outside blobs | Rejected as upstream | same session test |
| Position restore through diffs, other-window/frame, completion | | Open |
| File actions | stage (ignored confirm), unstage, untrack, rename with buffer path update, checkout, delete with unsaved-buffer and recursive-yes guards, traversal rejected | magit_file_dispatch_stages_renames_deletes_and_checks_out |
| Region multi-file actions, prefix --force, trace, edit line | | Open |

Independent review (October 4): fixed repository discovery from blob buffers
(commands used the synthetic path), C-quoted non-ASCII/quote names in log and
blame porcelain, blob buffers acting as files (recent list, :e, :w!, editing —
now generated/read-only like Magit views), stage-like index names (:0:),
conflicted index falling back to the worktree, reverse blame of the index,
recursion landing on the chunk's orig-line, and menu blame recursion.
Open: one git log per revision lacking blame headers (removal mode cost);
file log from a blob uses HEAD rather than the blob revision.

File-action review (October 4): fixed deletes/renames following symlinked
directories out of the repository and case-insensitive aliases bypassing git's
modified-file checks (blob::exact requires exact on-disk names without symlinked
intermediates, like upstream's require-match readers); directory renames now
retarget contained buffers; untracked deletion needs a typed "yes" (Fred has no
trash) and ignored files are refused; trailing-slash destinations must exist.
Regressions: exact_paths_reject_symlinks_case_aliases_and_existing_targets and
magit_file_dispatch_stages_renames_deletes_and_checks_out.

Security re-review of 5a59b0f (October 4): fixed rename into a directory
silently replacing an existing file there (exact() now refuses any existing
destination), and refused paths with a .git component (deleting .git with a
typed yes wiped the repository). The unsaved-buffer guard now compares file
identity, catching case-aliased and symlinked buffers. Open: rename retarget
of buffers opened through such aliases.
