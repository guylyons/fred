# Blame acceptance scenarios

Source: pinned magit-blame.el:39 (styles), :305 (read-only keymap), :416-560
(process and chunk parsing), :687-743 (formatting), :766-946 (suffixes, prefix).
This is a tested slice of the full matrix; open rows are still required.

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| Prefix B | Opens without Git; -w seeded as upstream :value | magit_blame_navigates_chunks_shows_commits_and_quits |
| blame-addition | Chunks, commit info, read-only, args passed | same test; blame_attributes_chunks_and_commit_info |
| Chunk keys | n/p/N/P movement and "No more chunks"; RET shows commit; q quits and restores | same session test |
| Styles | headings margin, highlight, lines rule + message; cycling | blame_margin_headings_and_lines_shift_text_and_show_message, blame_time_uses_commit_zone |
| blame-echo | Not read-only, keys not stolen, edits drop blame | same session test |
| Unsaved/invalid buffers | Modified buffer refused; non-file buffers error | same session test |
| Removal / reverse / recursive / visit-other-file | Need revision (blob) buffers | Open |
| Times | Upstream uses Emacs local zone | Open: Fred uses the commit's zone |
| Overlays tracking edits, M-w copy hash, SPC/DEL scroll, -M/-C values, incremental quickstart | | Open |

Independent review (October 4): fixed blame stealing word-jump (Space s) and
explain keys, the git sign bar drawing into text when the gutter clamps,
Enter on "Not Yet Committed" chunks, `b` shadowing vim word-back (recursive
blame stays open), and echo now shows only the message with a single style.
Init now confirms inside .git directories and bare repositories. Open: the
48-column headings margin consumes half of narrow terminals; wide characters
in author/summary misalign the margin.
