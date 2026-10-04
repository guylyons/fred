# Rebase acceptance scenarios

Source: pinned magit-sequence.el rebase section and git-rebase.el; user keys
from evil-collection-magit.el:504-562. Entry: Space m R.

Design: Git cannot wait on Fred as a sequence editor, so interactive rebases
capture Git's own todo list (a sequence editor copies it and fails, which Git
handles by aborting cleanly and reapplying any autostash), Fred edits the copy,
and a replay installs it with `cp`. Replay refuses if HEAD moved. Rewording,
squashing, --edit-todo and --continue run Fred itself as GIT_EDITOR in the
terminal (Fred's with-editor).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| i | Capture without starting a rebase (autostash survives), replay edited list, HEAD-moved refusal, --root for root commits | rebase_captures_and_replays_todo_lists |
| m / w / k | One-line todo rewrite then replay; stop for edit; continue/abort | same test |
| Published / merges | Confirmation before rewriting published commits or ranges with merges | same test |
| e / u / p / s / f | Non-interactive rebases with arguments; -i turns them interactive | same test |
| Todo buffer keys | p r e s f d, x exec, M-j/M-k, ZZ run, ZQ cancel | magit_rebase_todo_buffer_keys_edit_and_run_the_list |
| Rebase sequence section in status, log-select for subset/autosquash, strategy/exec/gpg/signoff, show commit from todo | | Open |

Independent review (October 4): each capture writes a fresh todo file, so an
open buffer of an older list can never be replayed against a new base; the plan
attaches only to the todo buffer itself; bases are resolved to commits at
capture and replay also checks the branch; the index is restored after an
autostash capture; `fixup -C` lines change action cleanly; M-j/M-k move within
the todo region across blank and `# Branch` lines; abbreviated commands are
matched; subset honors -i. Open: pre-rebase hook runs at capture and replay;
counts/`dd` fall through to vim; `:wq` closes without running; continue's
amend-published confirmation.

Todo keys (October 4): evil-collection makes git-rebase-mode-map an
overriding map, so upstream's keys apply in normal state: c/w/m alias pick,
reword and edit; S squish (fixup -c), F/A alter (fixup -C); b break, z noop;
l label, t reset, y insert, M M merge read their argument; M t toggles a
merge's -C/-c; Enter and SPC show the line's commit. u remains undo
(evil-collection), so update-ref has no key. Evidence:
session::tests::magit_rebase_todo_buffer_upstream_keys.
