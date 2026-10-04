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
