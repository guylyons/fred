;;; org-element-golden.el --- Golden parse trees for Fred's org-element port  -*- lexical-binding: t -*-

;; Usage:
;;   emacs --batch -Q -l tools/org-element-golden.el \
;;     /private/tmp/fred-org-upstream src/org/element/testdata/golden.txt [TEST-FILES...]
;;
;; With TEST-FILES (names under testing/lisp), only their texts are
;; used; FRED_ORG_GOLDEN=FILE cargo test --lib golden_file -- --ignored
;; checks such a file.
;;
;; Parses every literal Org text of the pinned upstream
;; testing/lisp/test-org-element.el (plus the extra cases below) with
;; upstream `org-element-parse-buffer', and writes one record per text:
;;
;;   #text "TEXT"
;;   #tree (TYPE :prop value ... CHILDREN)
;;   #interp "org-element-interpret-data of the tree"
;;
;; Positions are 0-based byte offsets.  src/org/element/tests.rs checks
;; Fred's parser and interpreter against this file.

(setq coding-system-for-write 'utf-8)
(set-language-environment "UTF-8")
(let ((root (car command-line-args-left)))
  (add-to-list 'load-path (expand-file-name "lisp" root))
  (setq golden-root root))
(require 'org)
(require 'org-element)
(require 'org-inlinetask)
(require 'org-footnote)
(setq org-element-use-cache nil)
(setq max-lisp-eval-depth 20000)

(defconst golden-extra
  '("* TODO [#A] COMMENT Title :tag:ARCHIVE:\nSCHEDULED: <2012-03-29 Thu +1w -2d> DEADLINE: <2012-04-01 Sun 10:00-11:30>\n:PROPERTIES:\n:CUSTOM_ID: x\n:A+: 1\n:A+: 2\n:END:\nBody *bold* /it/ _u_ +s+ =v= ~c~.\n\n** Child\n"
    "#+TITLE: Doc\n#+CAPTION[short]: long *cap*\n#+NAME: tbl\n#+ATTR_HTML: :width 10\n#+ATTR_HTML: :x y\n| a | b |\n|---+---|\n| 1 | 22 |\n#+TBLFM: $2=$1\n"
    "- [X] one :: two\n- [ ] three\n  1. nested\n  2. [@5] counter\n\n\n- after\n"
    "Text with [[https://orgmode.org][site]] and https://example.com/path, <mailto:a@b.c> [[file:a.org::*h]] [[#cid]] [[(ref)]] [[fuzzy target]].\n"
    "Footnote[fn:1] and [fn::inline *def*] and [fn:lbl:named].\n\n[fn:1] Definition\n"
    "\\alpha \\alpha{} \\_ x \\there4 $a^2$ \\(x\\) \\[y\\] $$z$$ \\frac{a}{b} H_2O x^{10} a_{b_c} 3^(x)\n"
    "src_python[:exports code]{print(1)} call_f[:h](x=1)[:e] @@html:<b>@@ {{{macro(a, b\\, c)}}} [3/4] [50%]\n"
    "<<target>> <<<Radio>>> and radio text.\n"
    "[cite/style:pre @key1 s1; @key2; post]\n"
    "<%%(diary-float t 4 2)> [2012-03-29 Thu 10:00]--[2012-03-30 Fri] <2012-03-29 Thu 10:00-12:00 .+1d/3d>\n"
    "#+BEGIN_SRC emacs-lisp -n 5 -r -l \"(ref:%s)\" :tangle yes\n  ,* (+ 1 2)\n#+END_SRC\n\n#+begin_example -i\nx\n#+end_example\n"
    "#+BEGIN: clocktable :scope file\n| a |\n#+END:\n\n#+begin_quote\nQ\n#+end_quote\n#+begin_center\nC\n#+end_center\n#+begin_verse\n  V\n#+end_verse\n#+begin_comment\nc\n#+end_comment\n#+begin_export html\n<b>\n#+end_export\n#+begin_foo bar\nF\n#+end_foo\n"
    ":DRAWER:\ncontent\n:END:\n: fixed\n: width\n-----\n# comment\n%%(diary)\n\\begin{equation}\nx\n\\end{equation}\n#+CALL: f[:a](b=1)[:c]\nCLOCK: [2012-03-29 Thu 10:00]--[2012-03-29 Thu 11:30] =>  1:30\n"
    "+-----+\n| a   |\n+-----+\n"
    "* Überschrift → *fett* äöü :tæg:\nText mit ümlauts and \\eacute; ~código~ [[https://ex.org/ä][lïnk]] x_{ñ}\n| é | 12 |\n| ∑ | 3 |\n"
    "1. a\n1. b\n1. c\n1. d\n1. e\n1. f\n1. g\n1. h\n1. i\n1. j\n   body\n1. k\n"
    "- a\n  + b\n    - c\n      text\n  + d\n- [ ] e\n  - [X] f\n  - [ ] g\n"
    "- outer\n\n  #+begin_src sh\n  ls\n  #+end_src\n\n  | x | y |\n\n- second\n  :DRAWER:\n  - not item?\n  :END:\n"
    "* A\n** B\n*** C\nbody\n**** D\n* E :x:\n  - item\n*************** inline :t:\n*************** END\n"
    "* H\nCLOSED: [2012-03-29 Thu 10:00] SCHEDULED: <2012-03-28 Wed>\n:PROPERTIES:\n:ID: abc\n:EMPTY:\n:END:\n:LOGBOOK:\nCLOCK: [2012-03-29 Thu 10:00]--[2012-03-29 Thu 11:00] =>  1:00\n:END:\n"
    "#+TODO: TODO NEXT | DONE CANCELED\n* NEXT [#B] task\n* CANCELED other\n* Footnotes\n[fn:1] note\n"
    "Paragraph one\nstill one\n\n\nParagraph two \\\\\nwith break\n#+begin_quote\n  - quoted item\n#+end_quote\n"
    "| 1 | <r> | text |\n| 22 | x | 3.5 |\n|----+-----+------|\n| a | b |\n"
    "[fn:a] multi\nline def\n\n[fn:b] other\n\n\n* H\n"
    "#+RESULTS[abc123]:\n: result\n\n#+HEADER: :var x=1\n#+HEADERS: :exports both\n#+NAME: blk\n#+begin_src python\nprint(x)\n#+end_src\n"
    "Plain links: http://a.b/c(d) and file:/tmp/x.org and doi:10.1/x, mailto:me@x.org.\n<2020-01-01 Wed 25:70> [2021-02-30 Tue]\n"
    "*************** TODO inline task\nbody\n*************** END\n"
    "* H\n:PROPERTIES:\n:A: 1\n:END:\n\n  - item\n\n\n* H2 [1/2] :t:\n"))

(defun golden--texts ()
  (let ((files (or (cddr command-line-args-left) '("test-org-element.el")))
        texts)
    (dolist (f files)
      (if (string-suffix-p ".org" f)
          ;; A whole Org file (path relative to the upstream root).
          (push (with-temp-buffer
                  (insert-file-contents (expand-file-name f golden-root))
                  (buffer-string))
                texts)
      (with-temp-buffer
        (insert-file-contents (expand-file-name f (expand-file-name "testing/lisp" golden-root)))
        (goto-char (point-min))
        (while (re-search-forward
                "(org-test-\\(?:with-temp-text\\(?:-in-file\\)?\\|parse-and-interpret\\)[ \t\n]+\"" nil t)
          (backward-char)
          (push (replace-regexp-in-string "<point>" "" (read (current-buffer))) texts)))))
    (delete-dups (append (nreverse texts) (and (not (cddr command-line-args-left)) golden-extra)))))

(defun golden--str (s)
  (concat "\""
          (mapconcat (lambda (c)
                       (pcase c
                         (?\\ "\\\\") (?\" "\\\"") (?\n "\\n") (?\t "\\t") (?\r "\\r")
                         (_ (string c))))
                     s "")
          "\""))

(defconst golden--skip
  '(:parent :buffer :mode :granularity :cached :org-element--cache-sync-key
            :robust-begin :robust-end :structure :secondary :deferred
            :standard-properties :path :true-level))

(defconst golden--positions
  '(:begin :end :contents-begin :contents-end :post-affiliated))

(defconst golden--booleans '(:preserve-indent :retain-labels :use-labels))

(defun golden--node-p (v)
  (and (consp v) (symbolp (car v)) (listp (cdr v))
       (memq (car v) (append '(org-data) org-element-all-elements org-element-all-objects))))

(defun golden--val (v)
  (cond ((null v) "nil")
        ((eq v t) "t")
        ((numberp v) (number-to-string v))
        ((stringp v) (golden--str v))
        ((symbolp v) (symbol-name v))
        ((golden--node-p v) (golden--node v))
        ((consp v)
         (let (parts)
           (while (consp v) (push (golden--val (pop v)) parts))
           (concat "(" (mapconcat #'identity (nreverse parts) " ")
                   (if v (concat " . " (golden--val v)) "")
                   ")")))
        (t (format "%S" v))))

(defun golden--node (n)
  (if (stringp n) (golden--str n)
    (let (props)
      (org-element-properties-mapc
       (lambda (k v)
         (unless (or (memq k golden--skip) (null v))
           (push (cons k v) props)))
       n t)
      (setq props (sort props (lambda (a b) (string< (symbol-name (car a)) (symbol-name (car b))))))
      (concat "(" (symbol-name (org-element-type n))
              (mapconcat (lambda (kv)
                           (let ((k (car kv)) (v (cdr kv)))
                             (concat " " (symbol-name k) " "
                                     (cond ((memq k golden--positions) (number-to-string (1- (position-bytes v))))
                                           ((memq k golden--booleans) "t")
                                           (t (golden--val v))))))
                         props "")
              (mapconcat (lambda (c) (concat " " (golden--node c))) (org-element-contents n) "")
              ")"))))

(unless (bound-and-true-p golden-no-main)
(with-temp-file (cadr command-line-args-left)
  (dolist (text (seq-remove (lambda (s) (string-match-p "SETUPFILE: *\"?https?:" s)) (golden--texts)))
    (let (tree interp)
      (with-temp-buffer
        (insert text)
        (org-mode)
        (condition-case err
            (progn
              (setq tree (golden--node (org-element-parse-buffer)))
              (setq interp (condition-case nil
                               (golden--str (org-element-interpret-data (org-element-parse-buffer)))
                             (error "error"))))
          (error (setq tree (format "error %S" err)))))
      (insert "#text " (golden--str text) "\n"
              "#tree " tree "\n"
              "#interp " (or interp "error") "\n\n")))))
