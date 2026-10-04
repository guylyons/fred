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
