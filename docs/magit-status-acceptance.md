# Status buffer acceptance scenarios

Source: pinned magit-status.el:45-90 (hooks), :629-755 (headers);
magit-log.el:1980-2110 (log sections); magit-stash.el:512 (stashes);
evil-collection-magit.el:356-364 (user jumpers).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| Head/Merge/Rebase/Push/Tag headers | Upstream summary, rebase keyword, push target, missing/invalid variants, tag distance | status_headers_and_log_sections_follow_upstream_and_push_remote |
| Unborn and detached HEAD | Head header variants, recent commits | status_of_unborn_and_detached_heads |
| Log sections | Unmerged into upstream vs recent commits; unpulled; push sections hidden when equal to upstream | status test above |
| Stashes, sections, jumpers | gz gn gu gs gpu gfu (missing section message), Tab collapse, RET on commit/stash | magit_status_sections_jump_and_visit_commits_and_stashes |
| Sequence/merge-log/bisect sections, error and diff-filter headers, upstream file-section names for unmerged | | Open |
| Section options (show hashes, commit count, hooks) | Defaults only | Open |

Independent review (October 4): log sections are capped at 256 commits with
"(256+)" (magit-status default -n256); headers/sections are computed only for
status buffers; config values are labelled so newlines cannot split rows; the
rebase keyword follows upstream's true/false/pull.rebase boolean rule;
unnamed/valid/invalid upstream wording matches magit--unnamed/valid-upstream-p;
sections start collapsed per magit-section-initial-visibility-alist and HIDE;
dwim on log sections diffs their endpoints; the cursor keeps the nearest
duplicate commit row; jumper messages name their sections. Open: duplicated
git calls per refresh (no refresh cache yet).

Sections (October 4): C-j/C-k move to the next/previous section start (C-j on
a file or hunk visits the worktree file, as evil-collection's section maps
do); gj gk ] [ M-j M-k move between siblings without leaving the parent; gh
goes up; za zo zc zO zC fold the heading, file or hunk's file at point; z1
closes every heading, z2 opens headings with files collapsed, z3/z4/zr
expand every file. Evidence: session::tests::magit_section_movement_and_folding.

Independent review of discard/reverse and keys (October 4): discard now
follows magit-discard-files' status table per file: staged changes with
unstaged work keep that work (index reversed, worktree only where it
applies); a new file with unstaged edits becomes untracked instead of being
deleted; a staged rename is undone (git mv back); intent-to-add files are
refused; section discards act only on the files the buffer listed and skip
any whose status changed; conflicts are never touched; reverse uses the
hardened diff (fixed prefixes). Hunk rows count as one section each for
movement; Space stays the leader in the rebase todo buffer, which also gets
evil-collection's text-mode toggle (C-t or \); zC only applies to diff
sections. Evidence: magit::tests::discard_keeps_unrelated_work_by_status,
session::tests::magit_rebase_todo_buffer_upstream_keys.

Independent review (October 4, keys): Alt keys reach Magit, blame, todo and
draft buffers whole in the real terminal (they were always split into Esc +
key, so M-j/M-k/M-1..M-4/M-w never worked outside tests); elsewhere an
unhandled Alt key is still Esc + key. Magit keys stand aside while word-jump
or explain overlays are open (labels could stage or discard). The git
command run from a subdirectory keeps the toplevel repository (git -C), so
Magit buffers refresh afterwards. Level keys 1-4 act on the heading above
any row and move point to it. Not done: reset-quickly does not save the
undone commit's message (Fred's message history reads the log).
