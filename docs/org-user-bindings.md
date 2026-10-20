# User's Org binding contract

Read from `~/.emacs.d/lisp/gl-keys.el:76` (normal state, override map) and
`gl-org.el`. Prefix `Space o`. This layout governs the Fred port.

| Key | Emacs command | Fred behavior |
| --- | --- | --- |
| a | org-agenda | Agenda dispatcher |
| c | org-capture | Capture template menu |
| f | gl/org-folder | Dired on `org-directory` |
| h | org-insert-heading | Insert heading |
| i | org-insert-link | Insert link prompt |
| s | org-store-link | Store link at point |
| t | org-insert-todo-heading | Insert TODO heading |
| I | org-clock-in | Clock in |
| O | org-clock-out | Clock out |

`gl-evil.el:82` binds normal-state `<tab>` to `org-cycle` in Org buffers.
`Space x` (org-roam) is a separate package and is outside upstream Org.

## User configuration (gl-org.el) to mirror in `[org]`

- org-directory: `~/org` (or `ORG_DIR` from `.env`; `~/work/org` on the work profile)
- org-agenda-files: org-directory, `org-roam`, `org-roam/daily`
- org-default-notes-file: `notes.org` in org-directory
- org-log-done t, org-startup-folded content, org-src-tab-acts-natively t
- org-agenda-include-diary t
- org-todo-keywords: `TODO DOING | DONE CANCELED`, `SENT APPROVED | PAID`
- org-todo-keyword-faces: TODO #ff39a3 bold, DOING #E35DBF bold, DONE #50fa7b
  bold, CANCELED white on #4d4d4d bold, SENT #8be9fd, APPROVED #f1fa8c, PAID #008080
- org-capture-templates: `w` "Work todo" entry (file+headline notes "Tasks")
  `* TICKET %?\nEntered on %U\n** Description\n** Notes\n** Resolution`
- Babel languages: R, dot, emacs-lisp, latex, python, ruby, shell, sqlite
