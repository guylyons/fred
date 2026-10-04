;;; org-source-inventory.el --- Read-only Org source inventory -*- lexical-binding: t; -*-

;; Run with a clean Emacs process, never the user's interactive Emacs:
;; emacs --batch -Q -l tools/org-source-inventory.el -- SOURCE OUTPUT.csv KEYS.csv
;; SOURCE is a pinned org-mode checkout.  Every function, macro and option
;; defined by a file in SOURCE/lisp is listed with its definition line;
;; `interactive' says whether `commandp' holds after loading every library.
(require 'cl-lib)

(when (equal (car command-line-args-left) "--")
  (pop command-line-args-left))
(let* ((source (file-truename (pop command-line-args-left)))
       (output (pop command-line-args-left))
       (keys-output (pop command-line-args-left))
       (lisp (expand-file-name "lisp" source))
       (files (directory-files lisp t "\\.el\\'"))
       (lines (make-hash-table :test #'equal))
       rows key-rows)
  (push lisp load-path)
  (dolist (file files)
    (let ((feature (intern (file-name-base file))))
      (condition-case err (require feature)
        (error (message "skip %s: %S" feature err)))))
  ;; Definition lines by a text scan; runtime generated symbols get none.
  (dolist (file files)
    (with-temp-buffer
      (insert-file-contents file)
      (goto-char (point-min))
      (while (re-search-forward
              "^(\\(?:cl-\\)?def\\(?:un\\|macro\\|subst\\|custom\\|var\\|var-local\\|const\\|alias\\|ine-derived-mode\\|ine-minor-mode\\|generic\\|method\\|ine-obsolete-function-alias\\)\\*?[ \t\n]+'?\\([^ \t\n()]+\\)" nil t)
        (let ((key (cons (file-name-nondirectory file) (match-string 1))))
          (unless (gethash key lines)
            (puthash key (line-number-at-pos (match-beginning 0)) lines))))))
  (cl-labels
      ((owned (symbol type)
         (let ((file (symbol-file symbol type)))
           (and file
                (file-in-directory-p (file-truename file) lisp)
                (concat (file-name-base file) ".el"))))
       (csv (s) (if (string-match-p "[\",\n]" s)
                    (concat "\"" (replace-regexp-in-string "\"" "\"\"" s) "\"")
                  s))
       (doc1 (s) (car (split-string (or s "") "\n"))))
    (mapatoms
     (lambda (symbol)
       (let ((ffile (owned symbol 'defun))
             (vfile (owned symbol 'defvar)))
         (when (and ffile (fboundp symbol))
           (push (list ffile (gethash (cons ffile (symbol-name symbol)) lines)
                       (cond ((commandp symbol) "command")
                             ((macrop symbol) "macro")
                             (t "function"))
                       (symbol-name symbol)
                       (doc1 (ignore-errors (documentation symbol t))))
                 rows))
         (when (and vfile (custom-variable-p symbol))
           (push (list vfile (gethash (cons vfile (symbol-name symbol)) lines)
                       "option" (symbol-name symbol)
                       (doc1 (ignore-errors
                               (documentation-property symbol 'variable-documentation t))))
                 rows)))))
    (setq rows (sort rows (lambda (a b)
                            (if (string= (car a) (car b))
                                (< (or (nth 1 a) most-positive-fixnum)
                                   (or (nth 1 b) most-positive-fixnum))
                              (string< (car a) (car b))))))
    (with-temp-file output
      (insert "source_file,source_line,kind,symbol,state,fred_behavior,doc\n")
      (dolist (r rows)
        (insert (mapconcat #'identity
                           (list (car r) (if (nth 1 r) (number-to-string (nth 1 r)) "")
                                 (nth 2 r) (csv (nth 3 r)) "missing" "" (csv (nth 4 r)))
                           ",")
                "\n")))
    ;; Effective bindings of the main keymaps, as Emacs resolves them.
    (dolist (map '(org-mode-map org-agenda-mode-map org-capture-mode-map
                   org-src-mode-map org-columns-map org-agenda-keymap))
      (when (and (boundp map) (keymapp (symbol-value map)))
        (cl-labels ((walk (keys b)
                      (cond ((and (symbolp b) (keymapp b) (not (fboundp b))) nil)
                            ((keymapp b)
                             (map-keymap (lambda (e b2) (walk (vconcat keys (vector e)) b2)) b))
                            ((and b (symbolp b))
                             (push (list (symbol-name map) (key-description keys)
                                         (symbol-name b))
                                   key-rows)))))
          (walk [] (symbol-value map)))))
    (with-temp-file keys-output
      (insert "keymap,key,command,state,fred_key\n")
      (dolist (r (nreverse key-rows))
        (insert (mapconcat #'csv r ",") ",missing,\n")))
    (message "%d rows, %d bindings" (length rows) (length key-rows))))
