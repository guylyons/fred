# Magit log acceptance (magit-log.el)

Reference: magit-log transient and suffixes at the pinned upstream revision.

| Scenario | Upstream | Fred | Evidence |
| --- | --- | --- | --- |
| Log menu groups | Commit limiting, History simplification, Commit ordering, Formatting; Log/Reflog/Other suffix columns | Same groups and keys; `/` prefix keys; shortlog and wiplog absent | menu_entries('l') |
| Default arguments | -n256 --graph --decorate | Seeded on first open per buffer | session test magit_log_menu_reads_option_values_and_limits_in_the_buffer |
| Free-form values | transient-option reads; set option is unset by its key | `-n`, `-A`, `=s`, `=u`, `-F`, `-G`, `-S`, `-L` read from the minibuffer and return to the menu | same session test |
| Buffer arguments | magit-prefix-use-buffer-arguments | Menu opened in a log buffer shows the buffer's arguments | same session test |
| current / HEAD / other | branch or HEAD; HEAD; revs split on space/comma | Same; option-like revisions rejected | magit::tests::log_variants_arguments_and_merged |
| related | current (or rebased/previous) branch, push target, upstream, upstream's upstream | Same, via @{push} and @{upstream} | code review |
| local/all branches, all refs, reflog | --branches (+HEAD when detached), --remotes, --all, --reflog | Same | log test |
| matching branches / tags | HEAD --branches=P / --tags=P | Same | log test |
| merged | git-when-merged: M^1..M, or first-parent neighborhood | Native: oldest first-parent descendant on the ancestry path | log test |
| Commit limit keys | = toggle, + double, - half | = and +; - stays revert (evil-collection) | session test |
| Graph and refs | washed graph with decorations | git --graph prefixes kept; refs shown in parentheses with -d | log test |
| Shortlog | magit-shortlog since/range with --numbered --summary | Menu S from `l s`; output in a read-only view | log test |
| Cherry | magit-cherry head/upstream, +/- commits newest first | Space m Y | log test |
| Move to parent | C-c C-n | Same; error suggests + when the parent is beyond the limit | session test |

Independent review (October 4): --reverse drops --graph as upstream does; a
log line is a commit only when its id field is a full object id (patch text
containing \x1e no longer panics); typed option values return to the open
menu instead of reseeding it from the log buffer; + without a limit sets 256
and a zero limit is removed.
