#!/bin/sh
# tools/org-oracle.sh TEXT ELISP: run ELISP in an org buffer containing TEXT (point at 1), print buffer and point.
cd /private/tmp/fred-org-upstream && emacs --batch -Q -L lisp -l org --eval "(with-temp-buffer (org-mode) (insert \"$1\") (goto-char (point-min)) $2 (prin1 (list (buffer-string) (point))) (terpri))" 2>/dev/null
