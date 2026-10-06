;;; org-lint-golden.el --- Golden reports for Fred's org-lint tests  -*- lexical-binding: t; -*-
;; Usage: emacs --batch -Q -L LISP -l org -l org-lint -l json -l tools/org-lint-golden.el \
;;          --eval '(org-lint-golden "test-org-lint.el" "out.json" "DIR")'
;; Every (org-test-with-temp-text TEXT ... (org-lint '(CHECKERS)) ...) in
;; the upstream test file becomes a case; the expected reports are what
;; org-lint returns for TEXT (point markers removed) in an Org buffer whose
;; default-directory is DIR.
(require 'cl-lib)
(defun org-lint-golden--walk (form f)
  (when (consp form)
    (funcall f form)
    (while (consp form)
      (org-lint-golden--walk (car form) f)
      (setq form (cdr form)))))
(defun org-lint-golden--checkers (body)
  (let (found)
    (org-lint-golden--walk
     body (lambda (x) (when (and (eq (car-safe x) 'org-lint)
                                 (eq (car-safe (cadr x)) 'quote))
                        (setq found (cadr (cadr x))))))
    found))
(defun org-lint-golden (test-file out dir)
  (let (cases)
    (with-temp-buffer
      (insert-file-contents test-file)
      (goto-char (point-min))
      (condition-case nil
          (while t
            (let ((form (read (current-buffer))))
              (org-lint-golden--walk
               form
               (lambda (x)
                 (when (and (eq (car-safe x) 'org-test-with-temp-text)
                            (stringp (cadr x)))
                   (let ((checkers (org-lint-golden--checkers (cddr x))))
                     ;; Remote INCLUDE targets make Emacs prompt in batch mode.
                     (when (and checkers (not (string-match-p "https://" (cadr x))))
                       (push (cons (cadr x) checkers) cases))))))))
        (end-of-file nil)))
    (with-temp-file out
      (insert "[\n")
      (let ((first t))
        (dolist (c (nreverse cases))
          (let* ((text (replace-regexp-in-string "<point>" "" (car c)))
                 (reports
                  (with-temp-buffer
                    (setq default-directory (file-name-as-directory dir))
                    (insert text)
                    (org-mode)
                    (cl-letf (((symbol-function 'yes-or-no-p) #'ignore)
                              ((symbol-function 'y-or-n-p) #'ignore))
                      (condition-case nil
                          (mapcar (lambda (r) (list (string-to-number (aref (cadr r) 0)) (aref (cadr r) 2)))
                                  (org-lint (cdr c)))
                        (t :error))))))
            (unless (eq reports :error)
            (unless first (insert ",\n"))
            (setq first nil)
            (insert (json-encode `((text . ,text)
                                   (checkers . ,(vconcat (mapcar #'symbol-name (cdr c))))
                                   (expected . ,(vconcat (mapcar #'vconcat reports)))))))))
        (insert "\n]\n")))))
