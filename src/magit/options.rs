//! Magit's customization options (defcustoms), set in Fred's config file:
//!
//! ```toml
//! [magit]
//! magit-log-section-commit-count = 20
//! magit-no-confirm = ["discard", "reverse"]
//! ```
//!
//! Values are TOML: booleans for t/nil, numbers, strings, symbols as
//! strings and lists as arrays. Options read here take effect; the rest are
//! recognized but have no effect yet; they remain missing in the ledger.
use std::sync::RwLock;
use toml::Value;

/// Every upstream option name (docs/magit-parity.csv).
pub const NAMES: [&str; 237] = [
    "auto-revert-buffer-list-filter",
    "git-commit-cd-to-toplevel",
    "git-commit-finish-query-functions",
    "git-commit-major-mode",
    "git-commit-mode-hook",
    "git-commit-post-finish-hook",
    "git-commit-post-finish-hook-timeout",
    "git-commit-setup-hook",
    "git-commit-style-convention-checks",
    "git-commit-summary-max-length",
    "git-commit-trailers",
    "git-commit-use-local-message-ring",
    "git-rebase-auto-advance",
    "git-rebase-confirm-cancel",
    "git-rebase-show-instructions",
    "global-git-commit-mode",
    "global-git-commit-mode-hook",
    "magit-auto-revert-immediately",
    "magit-auto-revert-mode",
    "magit-auto-revert-mode-hook",
    "magit-auto-revert-tracked-only",
    "magit-bisect-show-graph",
    "magit-blame-disable-modes",
    "magit-blame-echo-style",
    "magit-blame-goto-chunk-hook",
    "magit-blame-mode-hook",
    "magit-blame-mode-lighter",
    "magit-blame-read-only",
    "magit-blame-read-only-mode-hook",
    "magit-blame-styles",
    "magit-blame-time-format",
    "magit-blob-mode-hook",
    "magit-branch-adjust-remote-upstream-alist",
    "magit-branch-direct-configure",
    "magit-branch-name-suggestions",
    "magit-branch-prefer-remote-upstream",
    "magit-branch-read-upstream-first",
    "magit-branch-rename-push-target",
    "magit-buffer-name-format",
    "magit-bury-buffer-function",
    "magit-cherry-margin",
    "magit-cherry-sections-hook",
    "magit-clone-always-transient",
    "magit-clone-default-directory",
    "magit-clone-name-alist",
    "magit-clone-set-remote-head",
    "magit-clone-set-remote.pushDefault",
    "magit-clone-url-format",
    "magit-commit-ask-to-stage",
    "magit-commit-diff-inhibit-same-window",
    "magit-commit-extend-override-date",
    "magit-commit-reword-override-date",
    "magit-commit-show-diff",
    "magit-commit-squash-confirm",
    "magit-completing-read-function",
    "magit-copy-revision-abbreviated",
    "magit-create-buffer-hook",
    "magit-credential-cache-daemon-socket",
    "magit-cygwin-mount-points",
    "magit-define-global-key-bindings",
    "magit-delete-by-moving-to-trash",
    "magit-diff-adjust-tab-width",
    "magit-diff-buffer-file-locked",
    "magit-diff-expansion-threshold",
    "magit-diff-extra-stat-arguments",
    "magit-diff-fontify-hunk",
    "magit-diff-hide-trailing-cr-characters",
    "magit-diff-highlight-hunk-body",
    "magit-diff-highlight-hunk-region-functions",
    "magit-diff-highlight-indentation",
    "magit-diff-highlight-trailing",
    "magit-diff-mode-hook",
    "magit-diff-paint-whitespace",
    "magit-diff-paint-whitespace-lines",
    "magit-diff-refine-hunk",
    "magit-diff-refine-ignore-whitespace",
    "magit-diff-sections-hook",
    "magit-diff-specify-hunk-foreground",
    "magit-diff-unmarked-lines-keep-foreground",
    "magit-diff-use-indicator-faces",
    "magit-diff-visit-prefer-worktree",
    "magit-diff-visit-previous-blob",
    "magit-direct-use-buffer-arguments",
    "magit-display-buffer-function",
    "magit-dwim-selection",
    "magit-ediff-dwim-resolve-function",
    "magit-ediff-dwim-show-on-hunks",
    "magit-ediff-quit-hook",
    "magit-ediff-show-stash-with-index",
    "magit-ellipsis",
    "magit-format-file-function",
    "magit-generate-buffer-name-function",
    "magit-git-executable",
    "magit-git-global-arguments",
    "magit-git-output-coding-system",
    "magit-gitk-executable",
    "magit-list-refs-namespaces",
    "magit-list-refs-sortby",
    "magit-log-auto-more",
    "magit-log-buffer-file-locked",
    "magit-log-color-graph-limit",
    "magit-log-header-line-function",
    "magit-log-margin",
    "magit-log-margin-show-committer-date",
    "magit-log-merged-commit-count",
    "magit-log-mode-hook",
    "magit-log-remove-graph-args",
    "magit-log-revision-headers-format",
    "magit-log-section-commit-count",
    "magit-log-select-margin",
    "magit-log-select-show-usage",
    "magit-log-show-refname-after-summary",
    "magit-log-show-signatures-limit",
    "magit-log-trace-definition-function",
    "magit-log-trailer-labels",
    "magit-log-wash-summary-hook",
    "magit-mode-hook",
    "magit-module-sections-hook",
    "magit-module-sections-nested",
    "magit-need-cygwin-noglob",
    "magit-no-confirm",
    "magit-no-message",
    "magit-openpgp-default-signing-key",
    "magit-patch-save-arguments",
    "magit-pop-revision-stack-format",
    "magit-post-clone-hook",
    "magit-post-commit-hook",
    "magit-post-create-buffer-hook",
    "magit-post-display-buffer-hook",
    "magit-post-refresh-hook",
    "magit-post-stage-hook",
    "magit-post-unstage-hook",
    "magit-pre-display-buffer-hook",
    "magit-pre-refresh-hook",
    "magit-prefer-push-default",
    "magit-prefer-remote-upstream",
    "magit-prefix-use-buffer-arguments",
    "magit-process-apply-ansi-colors",
    "magit-process-connection-type",
    "magit-process-display-mode-line-error",
    "magit-process-ensure-unix-line-ending",
    "magit-process-error-tooltip-max-lines",
    "magit-process-find-password-functions",
    "magit-process-log-max",
    "magit-process-password-prompt-regexps",
    "magit-process-popup-time",
    "magit-process-prompt-functions",
    "magit-process-timestamp-format",
    "magit-process-username-prompt-regexps",
    "magit-process-yes-or-no-prompt-regexp",
    "magit-published-branches",
    "magit-pull-or-fetch",
    "magit-push-options",
    "magit-read-worktree-directory-function",
    "magit-read-worktree-offsite-directory",
    "magit-reflog-limit",
    "magit-reflog-margin",
    "magit-refresh-buffer-hook",
    "magit-refresh-status-buffer",
    "magit-refresh-verbose",
    "magit-refs-filter-alist",
    "magit-refs-focus-column-width",
    "magit-refs-margin",
    "magit-refs-margin-for-tags",
    "magit-refs-mode-hook",
    "magit-refs-pad-commit-counts",
    "magit-refs-primary-column-width",
    "magit-refs-sections-hook",
    "magit-refs-show-branch-descriptions",
    "magit-refs-show-commit-count",
    "magit-refs-show-remote-prefix",
    "magit-region-highlight-hook",
    "magit-remote-add-set-remote.pushDefault",
    "magit-remote-direct-configure",
    "magit-remote-git-executable",
    "magit-repolist-column-flag-alist",
    "magit-repolist-columns",
    "magit-repolist-mode-hook",
    "magit-repolist-sort-key",
    "magit-repository-directories",
    "magit-reshelve-since-committer-only",
    "magit-reverse-atomically",
    "magit-revision-fill-summary-line",
    "magit-revision-filter-files-on-follow",
    "magit-revision-headers-format",
    "magit-revision-insert-related-refs",
    "magit-revision-insert-related-refs-display-alist",
    "magit-revision-mode-hook",
    "magit-revision-sections-hook",
    "magit-revision-show-gravatars",
    "magit-revision-use-dedicated-buffers",
    "magit-revision-use-hash-sections",
    "magit-revision-wash-message-hook",
    "magit-run-hooks-from-githooks",
    "magit-save-repository-buffers",
    "magit-section-cache-visibility",
    "magit-section-disable-line-numbers",
    "magit-section-highlight-current",
    "magit-section-highlight-selection",
    "magit-section-initial-visibility-alist",
    "magit-section-keep-region-overlay",
    "magit-section-show-child-count",
    "magit-section-visibility-indicators",
    "magit-setup-buffer-hook",
    "magit-shell-command-verbose-prompt",
    "magit-show-process-buffer-hint",
    "magit-slow-confirm",
    "magit-stash-sections-hook",
    "magit-stashes-margin",
    "magit-status-file-list-limit",
    "magit-status-goto-file-position",
    "magit-status-headers-hook",
    "magit-status-initial-section",
    "magit-status-margin",
    "magit-status-mode-hook",
    "magit-status-sections-hook",
    "magit-status-show-hashes-in-headers",
    "magit-status-show-untracked-files",
    "magit-status-use-buffer-arguments",
    "magit-submodule-list-columns",
    "magit-submodule-list-mode-hook",
    "magit-submodule-list-predicate",
    "magit-submodule-list-sort-key",
    "magit-submodule-remove-trash-gitdirs",
    "magit-uniquify-buffer-names",
    "magit-unstage-committed",
    "magit-update-other-window-delay",
    "magit-user-githook-file",
    "magit-verbose-messages",
    "magit-view-git-manual-method",
    "magit-visit-ref-behavior",
    "magit-wip-debug",
    "magit-wip-merge-branch",
    "magit-wip-mode",
    "magit-wip-mode-hook",
    "magit-wip-mode-lighter",
    "magit-wip-namespace",
];

static OPTIONS: RwLock<Option<toml::Table>> = RwLock::new(None);

/// Install the [magit] table; unknown names are reported, not used.
pub fn set(table: toml::Table) -> Vec<String> {
    let unknown: Vec<String> = table
        .keys()
        .filter(|k| !NAMES.contains(&k.as_str()))
        .map(|k| format!("config: unknown Magit option {k}"))
        .collect();
    if let Ok(mut o) = OPTIONS.write() {
        *o = Some(table);
    }
    unknown
}

/// The configured value of an option.
pub fn value(name: &str) -> Option<Value> {
    debug_assert!(NAMES.contains(&name), "not an upstream option: {name}");
    OPTIONS.read().ok()?.as_ref()?.get(name).cloned()
}

/// A boolean option (t / nil); a non-boolean value counts as t, like Lisp.
pub fn flag(name: &str, default: bool) -> bool {
    match value(name) {
        Some(Value::Boolean(b)) => b,
        Some(_) => true,
        None => default,
    }
}

/// A number option.
pub fn int(name: &str, default: i64) -> i64 {
    match value(name) {
        Some(Value::Integer(n)) => n,
        Some(Value::Float(f)) => f as i64,
        _ => default,
    }
}

/// A number option that may be nil (false).
pub fn int_or_nil(name: &str, default: Option<i64>) -> Option<i64> {
    match value(name) {
        Some(Value::Integer(n)) => Some(n),
        Some(Value::Float(f)) => Some(f as i64),
        Some(Value::Boolean(false)) => None,
        _ => default,
    }
}

/// A string (or symbol) option; false is nil.
pub fn string(name: &str, default: Option<&str>) -> Option<String> {
    match value(name) {
        Some(Value::String(s)) => Some(s),
        Some(Value::Boolean(false)) => None,
        Some(Value::Boolean(true)) => Some("t".into()),
        _ => default.map(str::to_owned),
    }
}

/// A list-of-strings option.
pub fn strings(name: &str, default: &[&str]) -> Vec<String> {
    match value(name) {
        Some(Value::Array(a)) => a
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => vec![s],
        Some(Value::Boolean(false)) => vec![],
        _ => default.iter().map(|s| s.to_string()).collect(),
    }
}

/// magit-confirm: whether ACTION asks first (magit-no-confirm lists the
/// actions that do not; t means none do).
pub fn confirm(action: &str) -> bool {
    !selected(action, value("magit-no-confirm"), &["set-and-push"])
}

/// Whether this action requires the full word "yes".
pub fn slow_confirm(action: &str) -> bool {
    selected(action, value("magit-slow-confirm"), &["drop-stashes"])
}

fn selected(action: &str, value: Option<Value>, default: &[&str]) -> bool {
    match value {
        Some(Value::Array(a)) => a.iter().any(|v| v.as_str() == Some(action)),
        Some(Value::String(s)) => s == action,
        Some(Value::Boolean(b)) => b,
        _ => default.contains(&action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_selection_handles_boolean_list_and_default() {
        assert!(selected("discard", Some(Value::Boolean(true)), &[]));
        assert!(!selected(
            "set-and-push",
            Some(Value::Boolean(false)),
            &["set-and-push"]
        ));
        assert!(selected("drop-stashes", None, &["drop-stashes"]));
        assert!(selected(
            "discard",
            Some(Value::Array(vec![Value::String("discard".into())])),
            &[]
        ));
        assert!(!selected(
            "reverse",
            Some(Value::Array(vec![Value::String("discard".into())])),
            &[]
        ));
    }
}
