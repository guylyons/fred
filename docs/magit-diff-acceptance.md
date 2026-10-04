# Diff menu acceptance scenarios

Source: pinned magit-diff.el:1079-1117 (prefix and infixes), :1151-1298
(argument readers), :1307-1400 (dwim), :1463-1611 (suffixes), :2410 (defaults),
:2556/:3087 (always -p); magit-stash.el:601 (stash-show).
This is a tested slice of the full matrix; open rows are still required.

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| Prefix d | Opens without Git; --stat/--no-ext-diff defaults seeded once per buffer | magit_diff_menu_dwim_prompts_and_refreshes_with_buffer_arguments |
| Buffer arguments | A diff buffer's args seed the reopened menu; gr keeps target and args | same test |
| Choice infixes | -A/-X/-i cycle choices then off | same test (-A); magit_push_menu_collects_arguments_and_prompts_before_git (shared cycling) |
| u / s / w / r / c / p | Index, worktree, range/revision, commit and --no-index file diffs | diff_targets_render_index_worktree_range_commit_and_paths |
| dwim | Staged section → staged; log commit → c show commit; prompt fallback | session test above |
| Answer validation | Option-like ranges/commits rejected; unknown revisions and missing files error | diff_targets_render_index_worktree_range_commit_and_paths |
| -- files, -U, -D/-C/-H/-R/color-moved | Value readers and higher levels | Open |
| dwim region, unmerged, module, unpushed/unpulled | Additional contexts | Open; conflicts currently show the unstaged diff |
| Prefix-arg variants | Range ..., staged/worktree against a read revision | Open |
| Diff refresh prefix (D) and set/save defaults | magit-diff-refresh, persistence | Open |
| Diff buffer sections | Hunk navigation/staging inside diff buffers, revision headers | Open; views are read-only patch text |

Independent review (October 4): fixed diff paths failing when -X was set
(magit-diff-paths passes no arguments), one reusable diff buffer per repository,
untracked dwim falling through to the range prompt, stash-buffer dwim showing
stash^..stash, and ranges such as HEAD^! being rejected (now passed to Git as
magit-diff-range does). Open: stash-show does not yet apply diff arguments.

Diff refresh (October 4): Space m D shows magit-diff's arguments (shared
state with Space m d) and g applies them to the current diff buffer; r
switches A..B and A...B, f flips the revisions; = + ~ adjust -U in a diff
buffer as evil-collection binds them. Evidence:
session::tests::magit_diff_context_keys_and_refresh_menu.

Independent review (October 4): C-j reaches its handler; -C/-c apply to one
new commit only and never reach drafts, amend or fixups; diff-line visiting
handles hunk headers (first change), "\ No newline" markers, removed lines
starting with "--", renames (old name for removed lines), names with spaces,
unquoted non-ASCII names (core.quotePath=false), combined @@@ hunks and file
headers; sides follow magit-diff-visit--sides (staged: HEAD/index, unstaged:
index/worktree, A...B: merge base, stash sections ^2/stash, ^1/^2, ^3);
stash patches use fixed prefixes. Absorb modules only takes moved gitlinks
and reports per-module failures; autofixup is found on PATH or Git's exec
path; reshelve "now" keeps the local zone and treats same name or email as
yours; diff while committing shows HEAD^..HEAD for reword and the worktree
with --all; a buffer's -- file limit and --cached stay with that buffer; C-c
followed by another key in a draft keeps that key's meaning.

Independent review (October 4, apply): a and - in diff, commit and stash
buffers build the patch from Git's raw output (display text escapes tabs,
CRs and invalid bytes), refuse when the output is over 1 MB, refuse - on
unstaged diffs and a on unstaged/staged diffs (already in the worktree) and
diffs between files, and ask before reversing; a single hunk of a renamed
file applies to the new name without the rename; combined (--cc) hunks are
refused. C-c starts the smerge prefix only on a conflict, and other keys
after C-c ^ keep their meaning.
