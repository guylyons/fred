# Magit customization configuration

Pinned source: e9ed99c5e3cdd3fab31204f2809ce9d01d672c89. Fred reads
upstream customization names from `[magit]` in its TOML configuration.
Booleans represent t/nil; symbols are strings and lists are arrays.

```toml
[magit]
magit-log-section-commit-count = 20
magit-slow-confirm = true
magit-no-confirm = false
magit-log-margin = [true, "age", "magit-log-margin-width", true, 18]
```

This batch wires 21 options, all partial: confirmation and slow confirmation;
recent/merged/reflog counts; hunk fontification/refinement; log, cherry,
reflog, stashes, refs and status margins; committer-date display and ellipsis;
squash confirmation; external commit-message detection; autorevert and
tracked-only filtering; WIP and refresh diagnostics defaults.

Sources: magit-base.el (confirmation and ellipsis), magit-margin.el and the
per-buffer margin declarations, magit-log.el, magit-reflog.el,
magit-diff.el, magit-commit.el, git-commit.el, magit-autorevert.el,
magit-wip.el and magit-mode.el. Fontification is nil by default upstream.

Evidence: `magit_table_preserves_upstream_names_and_values`,
`confirmation_selection_handles_boolean_list_and_default`,
`confirmation_actions_keep_delete_and_remote_configuration_separate`,
`author_truncation_respects_width_even_with_long_ellipsis`, and existing
real-Git/session/UI tests. Boolean selection and oversized ellipsis checks
were observed failing before their fixes. Tests use pure inputs rather than
changing process-wide configuration while other tests are running.

Independent review fixed explicit nil confirmation handling, boolean slow
confirmation, stash short answers, revert abort action selection, destructive
file deletion action selection, remote configuration isolation, inherited
margin visibility and ellipsis overflow. Staged-section discard always keeps
its confirmation because it can include both deletion and discard actions.

Remaining: known but unwired names have no effect and remain missing in the
ledger. Arbitrary Lisp functions/hooks need Fred equivalents. Confirmation
coverage is incomplete; safe-with-wip grouping and runtime WIP state integration
remain open. Margins do not implement every width/style/selected-log default;
refinement t currently refines all hunks. Count nil semantics, live reload,
configuration type validation, full upstream option bounds and every
real-Git/UI scenario for customized values remain open. These are explicit
parity gaps, not accepted exclusions. Roadmap score stays 3%.
