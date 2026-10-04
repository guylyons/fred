# Push/fetch/pull acceptance scenarios

Source: pinned magit-push.el:42-285, magit-fetch.el:34-159, magit-pull.el:42-202,
magit-remote.el:393-416, magit-git.el:1975-2003 and :2090-2101.
This is a tested slice of the full matrix; open rows are still required.

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| Prefixes p/P/f | Open menus without running Git; arguments reach Git in order | magit_push_menu_collects_arguments_and_prompts_before_git, leader_m_routes_and_cancels |
| push/fetch/pull pushRemote | Unset or unknown pushRemote prompts, validates, writes branch.<b>.pushRemote, then runs | push_remote_sets_unconfigured_push_remote_then_pushes |
| push-current-to-upstream | Unset upstream prompts, adds --set-upstream once; configured upstream pushes branch:merge | push_upstream_sets_upstream_or_uses_configured_one |
| push elsewhere/other/refspecs/matching/tag | Target qualification, single-remote shortcut, tag validation, option-like answers rejected | push_elsewhere_other_tag_and_matching_targets |
| fetch upstream/all/branch, pull upstream/branch | Current-remote resolution, upstream set via branch --set-upstream-to, detached HEAD rejected | fetch_and_pull_suffixes_use_current_remote_and_upstream |
| Pull -r / -f | Rebase choices cycle; --ff-only incompatible with rebasing choices | magit_push_menu_collects_arguments_and_prompts_before_git |
| Failed network command | Terminal handoff returns to editor | e2e magit_failed_fetch_restores_terminal |
| Prefix argument variants | Change pushRemote/upstream even when configured | Open |
| Completion and defaults | Remote/branch/refspec completion, defaults at point | Open |
| Dynamic descriptions and :if | "Push main to origin/main, creating it" etc.; hide unavailable suffixes | Open |
| Hidden-level suffixes | push -o, notes ref, fetch -u, pull -A and U, Configure C/r | Open |
| set-and-push confirmation | magit-confirm before writing new push target | Open; the prompt answer currently acts as confirmation |

Independent review (October 4): fixed an option injection through the branch part
of a remote/branch answer (regression in push_elsewhere_other_tag_and_matching_targets),
a buffer-switch race that dropped a push after its config was written, and
magit-primary-remote ordering. Open review gaps: fetch pushRemote on detached
HEAD via remote.pushDefault; URL upstream remotes and pruned @{upstream}
validity; re-reading branch.<b>.merge after --set-upstream-to; magit-get-tracked
refspec mapping for non-default fetch refspecs; early validation per prompt.
