;;; The keys Magit buffers have under evil-collection, in each mode's initial
;;; evil state: run inside any Git repository,
;;; emacs -Q --batch -l tools/magit-evil-keys.el MAGIT-LISP-DIR OUT-FILE
(require 'package)
(package-initialize)
(let ((lisp (nth 0 command-line-args-left)))
  ;; The pinned upstream before the installed package.
  (setq load-path (cons lisp (seq-remove (lambda (d) (string-match-p "/magit-[0-9]" d)) load-path))))
;; As the user's gl-evil.el: every collection, no C-i jump.
(setq evil-want-keybinding nil evil-want-integration t evil-want-C-i-jump nil)
(require 'evil)
(require 'evil-collection)
(require 'magit)
(require 'git-rebase)
(require 'magit-blame)
(require 'magit-repos)
(require 'magit-submodule)
(evil-collection-init (seq-difference evil-collection-mode-list '(vterm eshell)))
(evil-mode 1)

(defvar keys-single
  (append (mapcar #'string (number-sequence 33 126))
          '("SPC" "TAB" "RET" "<backtab>" "DEL" "S-SPC" "C-<return>")
          (mapcar (lambda (c) (format "C-%c" c)) (number-sequence ?a ?z))
          (mapcar (lambda (c) (format "M-%c" c)) (number-sequence ?a ?z))))

(defun binding-name (b)
  (cond ((symbolp b) (symbol-name b))
        ((keymapp b) "PREFIX")
        (t "OTHER")))

(defun dump (out label lookup)
  (dolist (k keys-single)
    (let ((b (funcall lookup (kbd k))))
      (cond ((and b (keymapp b))
             (dolist (k2 keys-single)
               (let ((b2 (funcall lookup (kbd (concat k " " k2)))))
                 (when (and b2 (not (keymapp b2)) (not (numberp b2)))
                   (princ (format "%s\t%s %s\t%s\n" label k k2 (binding-name b2)) out)))))
            ((and b (not (numberp b)))
             (princ (format "%s\t%s\t%s\n" label k (binding-name b)) out))))))

(with-temp-file (nth 1 command-line-args-left)
  (let ((out (current-buffer)))
    (dolist (mode '(magit-status-mode magit-log-mode magit-revision-mode magit-diff-mode
                    magit-refs-mode magit-stashes-mode magit-stash-mode magit-reflog-mode
                    magit-cherry-mode magit-process-mode magit-log-select-mode
                    magit-repolist-mode magit-submodule-list-mode git-rebase-mode))
      (with-temp-buffer
        (delay-mode-hooks (funcall mode))
        (when (eq mode 'magit-log-select-mode) nil)
        (evil-local-mode 1)
        (evil-initialize-state)
        (message "%s %s" mode evil-state)
        (dump out (symbol-name mode) (lambda (k) (key-binding k)))))
    ;; Minor modes over a file buffer.
    (dolist (minor '(magit-blame-read-only-mode magit-blob-mode git-commit-mode))
      (with-temp-buffer
        (delay-mode-hooks (text-mode))
        (evil-local-mode 1)
        (funcall minor 1)
        (evil-initialize-state)
        (message "%s %s" minor evil-state)
        (dump out (symbol-name minor) (lambda (k) (key-binding k)))))
    ;; Point on a section: its keymap is a text property there, under the
    ;; mode's and evil's maps (key-binding applies the section's remaps).
    (dolist (spec '((magit-status-mode magit-hunk-section-map "status@hunk")
                    (magit-status-mode magit-file-section-map "status@file")
                    (magit-status-mode magit-staged-section-map "status@staged")
                    (magit-status-mode magit-unstaged-section-map "status@unstaged")
                    (magit-status-mode magit-untracked-section-map "status@untracked")
                    (magit-status-mode magit-stash-section-map "status@stash")
                    (magit-status-mode magit-stashes-section-map "status@stashes")
                    (magit-status-mode magit-module-section-map "status@module")
                    (magit-status-mode magit-worktree-section-map "status@worktree")
                    (magit-log-mode magit-commit-section-map "log@commit")
                    (magit-refs-mode magit-branch-section-map "refs@branch")
                    (magit-refs-mode magit-remote-section-map "refs@remote")
                    (magit-refs-mode magit-tag-section-map "refs@tag")
                    (magit-revision-mode magit-hunk-section-map "revision@hunk")
                    (magit-diff-mode magit-hunk-section-map "diff@hunk")))
      (with-temp-buffer
        (delay-mode-hooks (funcall (nth 0 spec)))
        (evil-local-mode 1)
        (evil-initialize-state)
        (let ((inhibit-read-only t))
          (insert (propertize "section\n" 'keymap (symbol-value (nth 1 spec)))))
        (goto-char (point-min))
        (dump out (nth 2 spec) (lambda (k) (key-binding k)))))))
