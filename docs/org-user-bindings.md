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

## Agenda keys (`Space o a`)

evil-collection-org-agenda is looked up first, then org-agenda-mode-map.
Evil motions not rebound by evil-collection stay Vim's (h l w W b e E 0 ^ $
G gg / ? N * # % ( ) { } ; , M y Y V z K, C-f C-b C-d C-u C-e C-y C-o,
counts); keys evil-collection-inhibit-insert-state disables do nothing
(a c C i o O R P < > =). `\` (evil-execute-in-emacs-state) sends the next
key to org-agenda-mode-map: `\ /` filter, `\ =` regexp filter, `\ <`
category filter, `\ v` view dispatcher, `\ E` entry text, `\ K` habits,
`\ C-_` undo.

| Key | Command |
| --- | --- |
| j / k | org-agenda-next-line / previous-line |
| gj gk, C-j C-k | org-agenda-next-item / previous-item |
| ]] / [[ | org-agenda-forward-block / backward-block |
| TAB / RET | org-agenda-goto / org-agenda-switch-to |
| S-RET, go | org-agenda-open-link |
| SPC | org-agenda-show (shadowed by the `Space` leader) |
| x, ZQ / q, ZZ | org-agenda-exit / org-agenda-quit |
| S | org-save-all-org-buffers |
| L / H / . / J | later / earlier / today / goto-date |
| vd vw vm vy | day / week / month / year view |
| t s D T p r A n d | todo, schedule, deadline, tags, priority, refile, archive, note, kill |
| f / F | filter-by-tag / filter-remove-all |
| I O X gJ | clock in / out / cancel / goto |
| m u B | bulk mark / unmark / action |
| U / Q | bulk unmark all / org-agenda-Quit (org-agenda-mode-map) |
| gr / gR | redo / redo-all |
| C-c chords, S- and M- arrows | org-agenda-mode-map |
