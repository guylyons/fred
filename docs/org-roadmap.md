# Org full-parity roadmap

Target: the complete user-visible behavior of upstream Org at
3b73b8a0cecf6435c265ce0f680dc3c135294c39, adapted to Fred.

## Percentage and accounting

**Verified roadmap completion: 2 / 100 points = 2%. Remaining: 98%.**

Points measure accepted deliverables. A row's points are awarded only when all
gates pass: (1) every upstream command/option of the row mapped in the ledger;
(2) reachable by keys, `:org NAME` and menus; (3) behavior implemented;
(4) tests covering normal, error and edge cases; (5) review with material
findings fixed. Partial rows stay at zero.

| Order | Track | Upstream | Points | Accepted |
| --- | --- | --- | ---: | ---: |
| 1 | Source baseline: pin, function/option CSV, keymap CSV, UI map | tools, docs | 4 | 2 |
| 2 | Syntax and element parser, in-buffer settings | org-element, org-macs, org.el | 7 | 0 |
| 3 | Visibility: folding, cycling, startup, narrowing, sparse trees, indent | org-fold, org-cycle, org-indent | 6 | 0 |
| 4 | Structure editing: headings, subtrees, move, promote, sort, cut/paste | org.el | 7 | 0 |
| 5 | TODO, priorities, tags, properties, logging, statistics | org.el | 7 | 0 |
| 6 | Plain lists and checkboxes | org-list | 4 | 0 |
| 7 | Timestamps, scheduling, repeaters, date prompt, durations | org.el, org-duration | 5 | 0 |
| 8 | Tables and spreadsheet | org-table, org-plot | 9 | 0 |
| 9 | Links, IDs, footnotes, radio targets | ol*, org-id, org-footnote | 5 | 0 |
| 10 | Agenda: views, commands, filters, bulk, export | org-agenda, org-habit | 10 | 0 |
| 11 | Capture, refile, archive, datetree | org-capture, org-refile, org-archive, org-datetree | 6 | 0 |
| 12 | Clocking, timers, effort, column view | org-clock, org-timer, org-colview | 6 | 0 |
| 13 | Babel: execution, results, tangle, library, languages | ob-* | 8 | 0 |
| 14 | Export framework, backends, publishing, citations | ox*, oc* | 9 | 0 |
| 15 | Src editing, attachments, crypt, inline tasks, misc modules | org-src, org-attach, org-crypt, ... | 4 | 0 |
| 16 | Display: faces, emphasis, entities, pretty entities, options | org-faces, org-entities | 3 | 0 |
| | Total | | 100 | 2 |

## Execution queue

1. Core: Key shift modifier, folding in Fred core, org buffer detection,
   `[org]` options, parser, faces.
2. Visibility and structure editing, TODO/tags/properties, lists, timestamps.
3. Tables, links, footnotes.
4. Agenda, capture, refile, archive, clock.
5. Babel, export, remaining modules.

Evidence: [source ledger](org-parity.csv), [keys](org-keys.csv),
[notes](org-port-notes.md), [bindings](org-user-bindings.md).

## Checkpoints

2026-10-04: pinned checkout and inventory: 5715 definitions and 597 bindings.
Baseline points for pin and inventory accepted (2). UI map open.
