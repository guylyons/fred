# File-log acceptance scenarios

Source: pinned magit-log.el:799-821 (file/region/follow), :498 (follow infix),
:525-554 (prefix), magit-git.el:1320-1338 (visited-file resolution).
This is a tested slice of the full matrix; open rows are still required.

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| magit-log prefix | l opens a menu without executing Git | user_leader_bindings_open_branch_commit_and_revert_menus |
| magit-log-buffer-file | File history excludes unrelated commits; gr retains filter; patch/return preserves unsaved source | magit_file_log_preserves_source_and_refreshes_its_filter |
| magit-log-current, follow infix | Current history inherits visited-file filter and follow; gr retains follow | magit_log_menu_inherits_file_filter_and_follow_option |
| magit-log-head | HEAD includes unrelated commits when file filter not explicitly selected | magit_log_menu_inherits_file_filter_and_follow_option; inherited explicit file filter open |
| Literal file selection | Colon/magic-looking filename does not select other files; renamed history follows only when enabled | file_history_filters_literal_paths_and_follows_renames |
| Invalid context | Unnamed/generated status buffers error without starting work | magit_file_log_rejects_buffers_without_a_source_file |
| Region tracing | Active range uses -L and suppresses file pathspec | Open |
| Revision/blob context | Use viewed revision rather than current HEAD | Open |
| Full log prefix | All remaining arguments, suffixes, persistence and graph/filter/range workflows | Open |
| Full file-log options | Dedicated-buffer customization and inherited defaults | Open |
