//! Obsolete names (org-compat.el and every `define-obsolete-*-alias` in
//! the pinned upstream tree): old commands run their replacement, old
//! option names in `[org]` set the new option. Generated from upstream by
//! the script in docs/org-port-notes.md (org-compat section).

/// (obsolete function, replacement).
pub const FUNCTIONS: &[(&str, &str)] = &[
    (
        "ob-clojure-eval-with-babashka",
        "#'ob-clojure-eval-with-cmd",
    ),
    ("org--hide-drawers", "org-fold--hide-drawers"),
    ("org--hide-wrapper-toggle", "org-fold--hide-wrapper-toggle"),
    (
        "org--inline-image-overlays",
        "org-link-preview--get-overlays",
    ),
    ("org--math-always-on", "org--math-p"),
    ("org-activate-angle-links", "ignore"),
    ("org-activate-bracket-links", "org-activate-links"),
    ("org-activate-mark", "activate-mark"),
    ("org-activate-plain-links", "ignore"),
    ("org-add-angle-brackets", "org-link-add-angle-brackets"),
    ("org-add-hook", "add-hook"),
    ("org-add-link-props", "org-link-add-props"),
    ("org-advertized-archive-subtree", "#'org-archive-subtree"),
    ("org-agenda-todayp", "org-agenda-today-p"),
    ("org-at-property-block-p", "org-at-property-drawer-p"),
    ("org-at-regexp-p", "org-in-regexp"),
    ("org-babel-edit-distance", "org-string-distance"),
    ("org-babel-examplize-region", "org-babel-examplify-region"),
    ("org-babel-number-p", "org-babel--string-to-number"),
    ("org-babel-strip-quotes", "org-strip-quotes"),
    ("org-babel-trim", "org-trim"),
    ("org-bound-and-true-p", "bound-and-true-p"),
    ("org-buffer-narrowed-p", "buffer-narrowed-p"),
    ("org-called-interactively-p", "called-interactively-p"),
    ("org-capture-member", "org-capture-get"),
    ("org-char-to-string", "char-to-string"),
    (
        "org-check-before-invisible-edit",
        "org-fold-check-before-invisible-edit",
    ),
    ("org-completing-read-no-i", "completing-read"),
    ("org-copy", "org-refile-copy"),
    ("org-count", "cl-count"),
    ("org-days-to-time", "org-timestamp-to-now"),
    ("org-decode-time", "decode-time"),
    ("org-decompose-region", "decompose-region"),
    ("org-define-error", "#'define-error"),
    ("org-defvaralias", "defvaralias"),
    ("org-delete-directory", "delete-directory"),
    ("org-detach-overlay", "delete-overlay"),
    (
        "org-display-inline-remove-overlay",
        "org-link-preview--remove-overlay",
    ),
    ("org-duration-string-to-minutes", "org-duration-to-minutes"),
    ("org-element-remove-indentation", "org-remove-indentation"),
    ("org-email-link-description", "org-link-email-description"),
    ("org-every", "cl-every"),
    ("org-export-get-genealogy", "org-element-lineage"),
    ("org-export-get-parent", "org-element-parent"),
    (
        "org-export-get-parent-element",
        "org-element-parent-element",
    ),
    ("org-file-complete-link", "org-link-complete-file"),
    ("org-file-equal-p", "file-equal-p"),
    ("org-file-remote-p", "file-remote-p"),
    ("org-file-url-p", "org-url-p"),
    ("org-find-if", "cl-find-if"),
    (
        "org-flag-above-first-heading",
        "org-fold-flag-above-first-heading",
    ),
    ("org-flag-heading", "org-fold-heading"),
    ("org-flag-region", "org-fold-region"),
    ("org-flag-subtree", "org-fold-subtree"),
    ("org-float-time", "float-time"),
    ("org-floor*", "cl-floor"),
    (
        "org-fold-core-remove-optimisation",
        "#'org-fold-core-remove-optimization",
    ),
    (
        "org-fold-core-update-optimisation",
        "#'org-fold-core-update-optimization",
    ),
    ("org-force-cycle-archived", "org-cycle-force-archived"),
    ("org-format-seconds", "format-seconds"),
    ("org-format-time-string", "format-time-string"),
    ("org-get-indentation", "current-indentation"),
    ("org-get-last-sibling", "org-get-previous-sibling"),
    ("org-get-tags-at", "org-get-tags"),
    ("org-habit-get-priority", "org-habit-get-urgency"),
    ("org-hh:mm-string-to-minutes", "org-duration-to-minutes"),
    (
        "org-hide-archived-subtrees",
        "org-fold-hide-archived-subtrees",
    ),
    ("org-hide-block-all", "org-fold-hide-block-all"),
    ("org-hide-block-toggle", "org-fold-hide-block-toggle"),
    ("org-hide-drawer-all", "org-fold-hide-drawer-all"),
    ("org-hide-drawer-toggle", "org-fold-hide-drawer-toggle"),
    ("org-hide-entry", "org-fold-hide-entry"),
    ("org-icompleting-read", "completing-read"),
    ("org-image-file-name-regexp", "image-file-name-regexp"),
    ("org-indent-line-to", "indent-line-to"),
    ("org-indent-to-column", "indent-to-column"),
    ("org-insert-columns-dblock", "org-columns-insert-dblock"),
    ("org-iread-file-name", "read-file-name"),
    ("org-link-escape-browser", "url-encode-url"),
    ("org-list-parse-list", "org-list-to-lisp"),
    ("org-looking-at-p", "looking-at-p"),
    ("org-looking-back", "looking-back"),
    ("org-make-link-regexps", "org-link-make-regexps"),
    ("org-make-link-string", "org-link-make-string"),
    (
        "org-make-org-heading-search-string",
        "org-link-heading-search-string",
    ),
    (
        "org-match-string-no-properties",
        "match-string-no-properties",
    ),
    (
        "org-minutes-to-clocksum-string",
        "org-duration-from-minutes",
    ),
    ("org-no-warnings", "with-no-warnings"),
    ("org-number-sequence", "number-sequence"),
    ("org-on-heading-p", "org-at-heading-p"),
    ("org-open-link-from-string", "org-link-open-from-string"),
    (
        "org-optimize-window-after-visibility-change",
        "org-cycle-optimize-window-after-visibility-change",
    ),
    ("org-pop-to-buffer-same-window", "pop-to-buffer-same-window"),
    ("org-preview-latex-fragment", "org-toggle-latex-fragment"),
    ("org-print-speed-command", "org--print-speed-command"),
    ("org-propertize", "propertize"),
    (
        "org-property-global-value",
        "org-property-global-or-keyword-value",
    ),
    (
        "org-publish-cache-ctime-of-src",
        "org-publish-cache-mtime-of-src",
    ),
    ("org-redisplay-inline-images", "org-link-preview-refresh"),
    ("org-reduce", "cl-reduce"),
    ("org-remove-double-quotes", "org-strip-quotes"),
    (
        "org-remove-from-invisibility-spec",
        "remove-from-invisibility-spec",
    ),
    ("org-remove-if", "cl-remove-if"),
    ("org-remove-if-not", "cl-remove-if-not"),
    ("org-remove-inline-images", "org-link-preview-clear"),
    (
        "org-remove-latex-fragment-image-overlays",
        "org-clear-latex-preview",
    ),
    (
        "org-select-frame-set-input-focus",
        "select-frame-set-input-focus",
    ),
    (
        "org-set-startup-visibility",
        "org-cycle-set-startup-visibility",
    ),
    ("org-set-tags-to", "org-set-tags"),
    (
        "org-set-visibility-according-to-property",
        "org-cycle-set-visibility-according-to-property",
    ),
    ("org-show-all", "org-fold-show-all"),
    ("org-show-branches-buffer", "org-fold-show-branches-buffer"),
    ("org-show-children", "org-fold-show-children"),
    ("org-show-context", "org-fold-show-context"),
    ("org-show-entry", "org-fold-show-entry"),
    ("org-show-hidden-entry", "org-fold-show-hidden-entry"),
    ("org-show-set-visibility", "org-fold-show-set-visibility"),
    ("org-show-siblings", "org-fold-show-siblings"),
    ("org-show-subtree", "org-fold-show-subtree"),
    ("org-some", "cl-some"),
    ("org-store-link-props", "org-link-store-props"),
    ("org-string-collate-lessp", "string-collate-lessp"),
    ("org-string-match-p", "string-match-p"),
    ("org-table-p", "org-at-table-p"),
    ("org-time-add", "time-add"),
    ("org-time-less-p", "time-less-p"),
    ("org-time-since", "time-since"),
    ("org-time-subtract", "time-subtract"),
    ("org-timestamp-format", "org-format-timestamp"),
    ("org-toggle-latex-fragment", "org-latex-preview"),
    ("org-truely-invisible-p", "org-truly-invisible-p"),
    ("org-without-partial-completion", "progn"),
];

/// (obsolete variable, replacement).
pub const VARIABLES: &[(&str, &str)] = &[
    ("hfy-optimisations", "hfy-optimizations"),
    (
        "org-agenda-ignore-drawer-properties",
        "org-agenda-ignore-properties",
    ),
    (
        "org-agenda-overriding-columns-format",
        "org-overriding-columns-format",
    ),
    ("org-angle-link-re", "org-link-angle-re"),
    ("org-any-link-re", "org-link-any-re"),
    ("org-attach-directory", "org-attach-id-dir"),
    (
        "org-babel-capitalize-example-region-markers",
        "org-babel-uppercase-example-markers",
    ),
    ("org-babel-ditaa-java-cmd", "org-ditaa-java-exec"),
    ("org-bracket-link-analytic-regexp", "org-link-bracket-re"),
    ("org-bracket-link-regexp", "org-link-bracket-re"),
    (
        "org-catch-invisible-edits",
        "org-fold-catch-invisible-edits",
    ),
    (
        "org-checkbox-hierarchical-statistics",
        "org-checkbox-children-only-statistics",
    ),
    (
        "org-confirm-elisp-link-function",
        "org-link-elisp-confirm-function",
    ),
    (
        "org-confirm-elisp-link-not-regexp",
        "org-link-elisp-skip-confirm-regexp",
    ),
    (
        "org-confirm-shell-link-function",
        "org-link-shell-confirm-function",
    ),
    (
        "org-confirm-shell-link-not-regexp",
        "org-link-shell-skip-confirm-regexp",
    ),
    ("org-context-in-file-links", "org-link-context-for-files"),
    ("org-descriptive-links", "org-link-descriptive"),
    (
        "org-display-internal-link-with-indirect-buffer",
        "org-link-use-indirect-buffer-for-internals",
    ),
    ("org-doi-server-url", "org-link-doi-server-url"),
    (
        "org-edit-src-content-indentation",
        "org-src-content-indentation",
    ),
    ("org-effort-durations", "org-duration-units"),
    (
        "org-email-link-description-format",
        "org-link-email-description-format",
    ),
    ("org-export-babel-evaluate", "org-export-use-babel"),
    (
        "org-export-before-parsing-hook",
        "org-export-before-parsing-functions",
    ),
    (
        "org-export-before-processing-hook",
        "org-export-before-processing-functions",
    ),
    (
        "org-export-htmlized-org-css-url",
        "org-org-htmlized-css-url",
    ),
    (
        "org-export-ignored-local-variables",
        "org-element-ignored-local-variables",
    ),
    ("org-from-is-user-regexp", "org-link-from-user-regexp"),
    (
        "org-hierarchical-todo-statistics",
        "org-todo-children-only-statistics",
    ),
    ("org-html-style", "org-html-head"),
    ("org-inline-image-overlays", "org-link-preview-overlays"),
    (
        "org-keep-stored-link-after-insertion",
        "org-link-keep-stored-after-insertion",
    ),
    (
        "org-latex-create-formula-image-program",
        "org-preview-latex-default-process",
    ),
    ("org-latex-listings", "org-latex-src-block-backend"),
    (
        "org-latex-preview-ltxpng-directory",
        "org-preview-latex-image-directory",
    ),
    ("org-latex-with-hyperref", "org-latex-hyperref-template"),
    (
        "org-make-link-description-function",
        "org-link-make-description-function",
    ),
    ("org-plain-link-re", "org-link-plain-re"),
    ("org-plantuml-executable-args", "org-plantuml-args"),
    ("org-reveal-start-hook", "org-fold-reveal-start-hook"),
    (
        "org-scroll-position-to-restore",
        "org-cycle-scroll-position-to-restore",
    ),
    ("org-show-context-detail", "org-fold-show-context-detail"),
    (
        "org-sort-agenda-noeffort-is-high",
        "org-agenda-sort-noeffort-is-high",
    ),
    (
        "org-sort-agenda-notime-is-late",
        "org-agenda-sort-notime-is-late",
    ),
    (
        "org-texinfo-def-table-markup",
        "org-texinfo-table-default-markup",
    ),
    (
        "org-usenet-links-prefer-google",
        "org-gnus-prefer-web-links",
    ),
];

/// The replacement of an obsolete command name.
pub fn function(name: &str) -> Option<&'static str> {
    FUNCTIONS
        .binary_search_by_key(&name, |(o, _)| o)
        .ok()
        .map(|i| FUNCTIONS[i].1)
}

/// Obsolete names of option `name`.
pub fn old_names(name: &str) -> impl Iterator<Item = &'static str> + '_ {
    VARIABLES
        .iter()
        .filter(move |(_, n)| *n == name)
        .map(|(o, _)| *o)
}

/// org-compat commands that are not plain aliases.
pub fn command(
    ed: &mut crate::editor::Editor,
    name: &str,
    _arg: super::Prefix,
) -> Option<Result<(), String>> {
    use super::fold;
    let l = ed.cur.line;
    Some(match name {
        "org-show-block-all" => {
            fold::show_all(ed, &[fold::Spec::Block]);
            Ok(())
        }
        "org-hide-block-toggle-maybe" => {
            fold::toggle_wrapper(ed, l, false, None);
            Ok(())
        }
        "org-hide-block-toggle-all" => {
            let starts: Vec<usize> = (0..ed.line_count())
                .filter(|&i| {
                    ed.buf
                        .line(i)
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("#+begin")
                })
                .collect();
            for b in starts {
                fold::toggle_wrapper(ed, b, false, None);
            }
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn aliases() {
        assert!(super::FUNCTIONS.is_sorted_by_key(|(o, _)| *o));
        assert_eq!(
            super::function("org-show-subtree"),
            Some("org-fold-show-subtree")
        );
        super::super::options::put("org-attach-directory", toml::Value::String("/x".into()));
        assert_eq!(
            super::super::options::string("org-attach-id-dir", "data/"),
            "/x"
        );
        super::super::options::remove("org-attach-directory");
    }
}
