# Commit menu acceptance scenarios (fixup family)

Source: pinned magit-commit.el:273-500. Entry: Space m c / C.

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| f / s / A / n / W | --fixup=, --squash=, amend:, reword:; edit variants run Fred as Git's editor; option-like targets rejected | commit_fixup_family_follows_magit_commit |
| commit-assert | Nothing staged asks to commit all; nothing at all errors; MERGE_MSG states; unresolved conflicts refused | same test |
| F / S | Commit then autosquash; refuse during merge/sequence/rebase; merges in range asked first | same test |
| Published | Remote branches containing the target (excluding */HEAD and branches exactly at it) ask first | review notes |
| Rebase-in-progress continue offer, not-ancestor [c]/[s]/[a] choice, log-select, magit-published-branches option, --author/--date/gpg | | Open |

Independent review (October 4): instant variants no longer flatten merges or
drop staged work during a merge (they refuse inside other operations and ask
about merges); assert follows MERGE_MSG; published check ignores */HEAD and
branches exactly at the target; prompts show a default only when one exists.

Commit menu remainder (October 4): -v is seeded as upstream's :value; -A, -D
and -S read values; -C reuses a message and commits at once, -c starts the
draft from a commit's message; d reshelves HEAD's dates (GIT_COMMITTER_DATE,
and --date for your own commit); R rewords a past commit; X writes a fixup
commit per modified module; x runs git-autofixup when installed. Evidence:
magit::tests::commit_reshelve_and_absorb_modules,
session::tests::magit_commit_reuse_and_reedit_message.
