;;; magit-render.el --- What Magit buffers show in a terminal -*- lexical-binding: t -*-
;;; emacs -Q --batch -l tools/magit-render.el MAGIT-LISP-DIR FIXTURES OUT
;;; renders the buffers of each repository tools/magit-fixtures.sh made in
;;; FIXTURES into OUT/<fixture>.<buffer>.txt, as a terminal frame shows them
;;; (invisible text left out, the "…" of collapsed sections put in). Margins
;;; are off: Fred draws them beside the text.
(require 'package)
(package-initialize)
(let ((lisp (nth 0 command-line-args-left)))
  (setq load-path (cons lisp (seq-remove (lambda (d) (string-match-p "/magit-[0-9]" d)) load-path))))
(require 'magit)
(setq magit-display-buffer-function (lambda (b) (set-window-buffer (selected-window) b) (set-buffer b) (selected-window))
      magit-log-margin '(nil age magit-log-margin-width t 18)
      magit-refs-margin '(nil age magit-log-margin-width t 18)
      magit-reflog-margin '(nil age magit-log-margin-width nil 18)
      magit-stashes-margin '(nil age magit-log-margin-width nil 18)
      magit-cherry-margin '(nil age magit-log-margin-width t 18)
      magit-status-margin '(nil age magit-log-margin-width nil 18))

(defun visible-text ()
  "The buffer as a terminal shows it."
  (let ((out "") (pos (point-min)))
    (while (< pos (point-max))
      (let ((next (min (next-single-char-property-change pos 'invisible)
                       (next-single-char-property-change pos 'display)
                       (next-overlay-change pos))))
        (dolist (o (overlays-at pos))
          (let ((s (overlay-get o 'before-string)))
            ;; Fringe bitmaps show nothing in a terminal.
            (when (and (= (overlay-start o) pos) s (not (get-text-property 0 'display s)))
              (setq out (concat out s)))))
        (unless (invisible-p pos)
          (let ((d (get-char-property pos 'display))
                (col (length (car (last (split-string out "\n"))))))
            (setq out (concat out
                              (cond ((stringp d) d)
                                    ;; (space :align-to N) and (space :width N).
                                    ((eq (car-safe d) 'space)
                                     (let ((to (plist-get (cdr d) :align-to))
                                           (w (plist-get (cdr d) :width)))
                                       (make-string (max 0 (cond ((numberp to) (- to col))
                                                                 ((numberp w) w)
                                                                 (t 1)))
                                                    ?\s)))
                                    (t (buffer-substring-no-properties pos next)))))))
        (dolist (o (overlays-in pos next))
          (let ((s (overlay-get o 'after-string)))
            (when (and (= (overlay-end o) next) s (not (get-text-property 0 'display s)))
              (setq out (concat out s)))))
        (setq pos next)))
    (substring-no-properties out)))

(defun listing ()
  "The tabulated list a list command just filled (it may not be current)."
  (seq-find (lambda (b)
              (with-current-buffer b
                (and (derived-mode-p 'tabulated-list-mode) (> (buffer-size) 0))))
            (buffer-list)))

(defun render (fixtures out fixture name setup)
  (let ((dir (file-name-as-directory (expand-file-name fixture fixtures))))
    (when (file-directory-p dir)
      ;; Start from a buffer in the repository, as a user would.
      (set-buffer (get-buffer-create " *render*"))
      (setq default-directory dir)
      (let ((buffer (condition-case err (progn (funcall setup) (current-buffer))
                      (error (message "%s %s: %S" fixture name err) nil))))
        (when buffer
          (with-current-buffer buffer
            (with-temp-file (expand-file-name (format "%s.%s.txt" fixture name) out)
              (insert (with-current-buffer buffer (visible-text)))))
          ;; Every Magit buffer goes, so the next repository gets its own.
          (dolist (b (buffer-list))
            (when (with-current-buffer b (derived-mode-p 'magit-mode 'magit-repolist-mode))
              (kill-buffer b))))))))

(let ((fixtures (nth 1 command-line-args-left))
      (out (nth 2 command-line-args-left)))
  (make-directory out t)
  (dolist (fixture '("changes" "clean" "conflict" "rebase" "detached" "empty"))
    (render fixtures out fixture "status"
            (lambda () (set-buffer (magit-status-setup-buffer default-directory))))
    (render fixtures out fixture "log"
            (lambda () (set-buffer (magit-log-setup-buffer
                                    (list (or (magit-get-current-branch) "HEAD"))
                                    '("-n256" "--graph" "--decorate") nil)))))
  (dolist (fixture '("changes" "clean"))
    (render fixtures out fixture "refs"
            (lambda () (set-buffer (magit-refs-setup-buffer "HEAD" nil)))))
  (render fixtures out "changes" "stashes"
          (lambda () (set-buffer (magit-stashes-setup-buffer))))
  (render fixtures out "changes" "revision"
          (lambda () (set-buffer (magit-revision-setup-buffer "HEAD" (list "--stat" "--no-ext-diff") nil))))
  (render fixtures out "changes" "unstaged"
          (lambda () (set-buffer (magit-diff-setup-buffer nil nil '("--no-ext-diff" "--stat") nil 'unstaged))))
  (render fixtures out "changes" "staged"
          (lambda () (set-buffer (magit-diff-setup-buffer nil "--cached" '("--no-ext-diff" "--stat") nil 'staged))))
  (render fixtures out "changes" "reflog"
          (lambda () (set-buffer (magit-reflog-setup-buffer "HEAD"))))
  (render fixtures out "changes" "cherry"
          (lambda () (set-buffer (magit-cherry-setup-buffer "main" "origin/main"))))
  (render fixtures out "changes" "filelog"
          (lambda () (set-buffer (magit-log-setup-buffer (list "main") '("-n256" "--follow") '("a")))))
  (render fixtures out "modules" "status"
          (lambda () (set-buffer (magit-status-setup-buffer default-directory))))
  (render fixtures out "modules" "submodules"
          #'magit-list-submodules)
  (let ((magit-repository-directories (list (cons fixtures 1))))
    (render fixtures out "changes" "repolist"
            (lambda () (magit-list-repositories) (set-buffer (listing)))))
  (render fixtures out "bisect" "status"
          (lambda () (set-buffer (magit-status-setup-buffer default-directory))))
  ;; magit-blame-addition's default style (headings) on a file buffer;
  ;; blame runs Git asynchronously.
  (render fixtures out "blame" "blame"
          (lambda ()
            (find-file "f")
            (magit-blame-addition nil)
            (while (and magit-blame-process (process-live-p magit-blame-process))
              (accept-process-output magit-blame-process 0.1))
            (accept-process-output nil 0.2)))
  (kill-emacs 0)))
