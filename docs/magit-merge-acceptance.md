# Merge menu acceptance scenarios

Source: pinned magit-merge.el (prefix, plain/editmsg/nocommit/absorb/dissolve/
squash/preview/abort).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| m / n | argv with args, octopus commas, --no-ff added and --ff-only dropped for n, dirty refusal, option-like revs rejected | merge_suffixes_follow_magit_merge |
| e | Merge without commit, open Fred draft from MERGE_MSG | same test |
| p | merge-tree preview as a diff view; worktree untouched | same test |
| a / d | Absorb deletes the merged branch; absorbing main needs yes; dissolve merges current into target and removes it | same test |
| In-progress group | m commits (draft from MERGE_MSG), a aborts after y/n | same test (abort); session routing |
| -X options, gpg-sign, signoff, absorb force-push, checkout-stage | | Open |

Independent review (October 4): fixed MERGE_MSG never reaching the commit draft
(a blank, unmodified draft is now seeded; the user's own draft text wins),
dissolve/absorb accepting tags, commits or the current branch (now another
local branch only, preventing a detached-HEAD merge with the source deleted),
dirty worktrees refused instead of confirmed (magit-merge-assert asks; dirty
submodules ignored), push-remote config removed before a delete that can fail,
e opening a draft when already up to date, and multi-revision preview.
Regression: magit_merge_edit_message_seeds_a_blank_draft_and_commits_two_parents.
