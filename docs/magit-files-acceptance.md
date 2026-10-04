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
| File actions (stage, unstage, untrack, rename, delete, checkout), trace, edit line | | Open |

Independent review (October 4): fixed repository discovery from blob buffers
(commands used the synthetic path), C-quoted non-ASCII/quote names in log and
blame porcelain, blob buffers acting as files (recent list, :e, :w!, editing —
now generated/read-only like Magit views), stage-like index names (:0:),
conflicted index falling back to the worktree, reverse blame of the index,
recursion landing on the chunk's orig-line, and menu blame recursion.
Open: one git log per revision lacking blame headers (removal mode cost);
file log from a blob uses HEAD rather than the blob revision.
