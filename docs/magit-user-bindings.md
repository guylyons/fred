# User's Magit binding contract

Confirmed by the user on October 4, 2026, and read from
`~/.emacs.d/lisp/gl-keys.el:101`. These are normal-state override bindings.
The prefix is `Space m`. This exact layout governs the Fred port.

| Key | Emacs command | Required Fred behavior | Current gap |
| --- | --- | --- | --- |
| B | magit-blame | Blame menu/modes | Reserved; blame missing |
| L | magit-log-buffer-file | Current file's history | Missing |
| P | magit-pull | Pull menu and contextual variants | Direct ff-only pull is partial |
| b | magit-branch | Branch menu | Menu routing corrected; full branch options/context remain partial |
| c | magit-commit | Commit menu | Menu routing corrected; full commit options/context remain partial |
| d | magit-diff | Diff menu | Missing global diff entry |
| f | magit-fetch | Fetch menu and variants | Direct fetch is partial |
| i | magit-init | Init workflow | Missing |
| l | magit-log | Log menu and variants | Direct recent log is partial |
| p | magit-push | Push menu and variants | Direct configured push is partial |
| r | magit-revert | Revert menu | Routing corrected; full variants/options remain partial |
| s | magit-status | Status view | Existing partial status view |

Do not reassign these keys to different Git commands. Additional capabilities
(stash, rebase, tags, merge, etc.) belong in source-style menus and additional
nonconflicting entry keys. A command prefix binding means opening its menu;
running one default Git operation immediately is not menu parity.

## In-buffer behavior

`gl-evil.el` enables Evil-collection, including Magit; it does not exclude Magit.
The installed `evil-collection-magit.el` provides the local binding baseline.
No overrides of its Magit-specific customization variables were found in the
user's top-level modules. Treat defaults as inferred from those files, not as a
measurement of an already-running Emacs session.

Relevant default bindings: j/k move lines; Ctrl-j/Ctrl-k move sections;
gj/gk move siblings; gr refreshes; gR refreshes all; x performs contextual
delete/discard (k remains movement); p opens push; q buries the view; gn/gu/gs
jump to untracked/unstaged/staged; gz jumps to stashes. Visual selection and
section-aware operations are part of the target. Stash-specific commands should
respect this inherited navigation rather than stealing normal-mode k for drop.
The source stash delete binding is adapted through contextual x; confirmation
and stable selected object identity remain required.

`gl-tools.el` displays transient menus at the bottom of the main area above its
status bar. Fred should preserve a full-width menu and visible status/footer.
`gl-completion.el` disables Corfu in Magit and commit buffers: avoid unsolicited
code completion in command-oriented views. `gl-evil.el` excludes status, log and
revision buffers from jj escape handling.

## Next implementation order

The b/c/r routing is corrected with regression tests; B is reserved for blame.
Implement B/L/d/i and convert network/log entries to source menus. Do not
credit menu parity until its arguments and contextual suffix workflows work.
Keep this personal binding map separate from the full upstream surface inventory;
it steers the interface without reducing the required functionality.
