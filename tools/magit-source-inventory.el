;;; magit-source-inventory.el --- Read-only upstream surface inventory -*- lexical-binding: t; -*-

;; Run with a clean Emacs process, never the user's interactive Emacs:
;; emacs --batch -Q -l tools/magit-source-inventory.el -- SOURCE OUTPUT ELPA
;; SOURCE is a pinned checkout; ELPA supplies dependency load paths only.
;; This does not initialize packages or enable user configuration.
(require 'json)
(require 'cl-lib)

(when (equal (car command-line-args-left) "--")
  (pop command-line-args-left))
(let* ((source (file-truename (pop command-line-args-left)))
       (output (pop command-line-args-left))
       (elpa (pop command-line-args-left))
       (lisp (expand-file-name "lisp" source))
       (files (directory-files lisp t "\\.el\\'"))
       (print-circle t)
       (print-escape-nonascii t)
       (print-escape-newlines t)
       (coding-system-for-write 'utf-8-unix)
       commands options menus menu-entries keymaps modes)
  (unless (and output elpa)
    (error "Usage: SOURCE OUTPUT ELPA"))
  ;; Never use installed Magit definitions as the baseline. Package initialization
  ;; can load them before our load-path entry, so only add dependency directories.
  (dolist (dir (directory-files elpa t "^[^.]"))
    (when (and (file-directory-p dir)
               (not (string-match-p "/\\(magit\\|git-commit\\|git-rebase\\)" dir)))
      (push dir load-path)))
  (push lisp load-path)
  (require 'magit)
  (dolist (file files)
    (require (intern (file-name-base file))))
  (cl-labels
      ((owned (symbol type)
         (let ((file (symbol-file symbol type)))
           (and file
                (file-in-directory-p (file-truename file) lisp)
                (file-name-nondirectory file))))
       (text (value) (prin1-to-string value))
       (row (symbol file)
         `((symbol . ,(symbol-name symbol)) (source_file . ,file)))
       (entries (value prefix file groups)
         (cond
          ((and (vectorp value) (> (length value) 2)
                (symbolp (aref value 0)))
           (entries (aref value 2) prefix file
                    (append groups (list (text (plist-get (aref value 1) :description))))))
          ((and (consp value) (symbolp (car value))
                (stringp (plist-get (cdr value) :key)))
           (push `((prefix . ,(symbol-name prefix))
                   (source_file . ,file)
                   (key . ,(plist-get (cdr value) :key))
                   (groups . ,(vconcat groups))
                   (command . ,(text (plist-get (cdr value) :command)))
                   (properties . ,(text (cdr value)))) menu-entries))
          ((vectorp value)
           (mapc (lambda (child) (entries child prefix file groups)) value))
          ((consp value)
           (dolist (child value) (entries child prefix file groups)))))
       (bindings (map prefix ancestors)
         (let (entries)
           (unless (memq map ancestors)
             (map-keymap
              (lambda (event binding)
                (let ((keys (vconcat prefix (vector event))))
                  (if (keymapp binding)
                      (setq entries (append (bindings binding keys (cons map ancestors)) entries))
                    (push `((key . ,(substring (text (key-description keys)) 1 -1))
                            (binding . ,(text binding))) entries))))
              map))
           entries)))
    ;; Runtime expansion includes macro-generated jumpers, modes, aliases and
    ;; transient arguments that explicit defun declarations cannot enumerate.
    (mapatoms
     (lambda (symbol)
       (let ((function-file (owned symbol 'defun))
             (variable-file (owned symbol 'defvar)))
         (when (and function-file (commandp symbol))
           (push (append (row symbol function-file)
                         `((interactive . ,(text (interactive-form symbol))))) commands))
         (when (and variable-file (get symbol 'custom-type))
           (push (append (row symbol variable-file)
                         `((type . ,(text (get symbol 'custom-type)))
                           (default . ,(text (get symbol 'standard-value))))) options))
         (when (and function-file (get symbol 'transient--layout))
           (entries (get symbol 'transient--layout) symbol function-file nil)
           (push (append (row symbol function-file)
                         `((layout . ,(text (get symbol 'transient--layout))))) menus))
         (when (and variable-file (boundp symbol)
                    (keymapp (symbol-value symbol)))
           (push (append (row symbol variable-file)
                         `((parent . ,(text (keymap-parent (symbol-value symbol))))
                           (bindings . ,(vconcat (bindings (symbol-value symbol) [] nil))))) keymaps)))))
    ;; Record source mode declarations separately: a mode command alone does not
    ;; account for its setup, hooks, integrations or inherited map behavior.
    (dolist (file files)
      (with-temp-buffer
        (insert-file-contents file)
        (set-syntax-table emacs-lisp-mode-syntax-table)
        (goto-char (point-min))
        (condition-case nil
            (while t
              (forward-comment (point-max))
              (let* ((position (point))
                     (form (read (current-buffer))))
                (when (memq (car-safe form)
                            '(define-derived-mode define-minor-mode define-globalized-minor-mode))
                  (push `((symbol . ,(symbol-name (cadr form)))
                          (source_file . ,(file-name-nondirectory file))
                          (declaration . ,(symbol-name (car form)))
                          (parent . ,(text (and (not (eq (car form) 'define-minor-mode)) (nth 2 form))))
                          (source_line . ,(line-number-at-pos position))) modes))))
          (end-of-file nil))))
    (dolist (symbol '(magit-status magit-stash-both git-commit-mode git-rebase-mode))
      (unless (owned symbol 'defun)
        (error "Baseline provenance mismatch for %s: %s" symbol (symbol-file symbol))))
    (let ((sort-rows (lambda (rows)
                       (vconcat (sort rows (lambda (a b)
                                            (string< (alist-get 'symbol a) (alist-get 'symbol b))))))))
      (with-temp-file output
        (insert
         (json-serialize
          `((baseline . ,(string-trim
                          (with-temp-buffer
                            (unless (zerop (call-process "git" nil t nil "-C" source "rev-parse" "HEAD"))
                              (error "Cannot identify upstream revision"))
                            (buffer-string))))
            (emacs_version . ,emacs-version)
            (commands . ,(funcall sort-rows commands))
            (options . ,(funcall sort-rows options))
            (menus . ,(funcall sort-rows menus))
            (menu_entries . ,(vconcat (sort menu-entries
                                         (lambda (a b)
                                           (string< (concat (alist-get 'prefix a) (alist-get 'key a))
                                                    (concat (alist-get 'prefix b) (alist-get 'key b)))))))
            (keymaps . ,(funcall sort-rows keymaps))
            (modes . ,(funcall sort-rows modes)))))
        (insert "\n")))
    (princ (format "Commands: %d; options: %d; menus: %d; keymaps: %d; modes: %d\n"
                   (length commands) (length options) (length menus) (length keymaps) (length modes)))))
