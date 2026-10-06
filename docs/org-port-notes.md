# Org-mode port: scope and continuation notes

## User's requirement

"Just like magit, port org-mode (/Users/guy/github/org-mode) to fred. Document
all functions in a csv and complete a full working port of org-mode; we will
be able to handle org-mode in its entirety. Work until completion."

This is a port of upstream Org as a Rust component of Fred, not an
Org-inspired outliner. Every command, option, key, view and workflow in the
source is in scope. Emacs-only plumbing is adapted to a Fred equivalent, and
the adaptation is recorded; nothing is silently dropped.

## Source baseline

Upstream: https://git.savannah.gnu.org/git/emacs/org-mode.git (local clone
`/Users/guy/github/org-mode`, origin github.com/bzg/org-mode).
Pinned commit: 3b73b8a0cecf6435c265ce0f680dc3c135294c39 (2026-10-03).
Temporary checkout with generated autoloads: `/private/tmp/fred-org-upstream`;
recreate with `git clone` + `git checkout <pin>` + `make autoloads` if missing.
The user's checkout is never modified.

## Inventory

`tools/org-source-inventory.el` loads every library in the pinned checkout in
`emacs --batch -Q` and writes:

- [docs/org-parity.csv](org-parity.csv): every function, macro and user option
  defined by an upstream file (5715 rows: 906 commands, 3610 functions,
  93 macros, 1106 options), with definition line and first doc line.
- [docs/org-keys.csv](org-keys.csv): 597 effective bindings of org-mode-map,
  org-agenda-mode-map, org-capture-mode-map, org-src-mode-map, org-columns-map.

Ledger `state` values:
- `missing`: no Fred behavior.
- `partial`: some behavior; named gaps in `fred_behavior`.
- `done`: behavior matches upstream for the cases the source handles.
- `internal`: a non-interactive helper whose role is carried by the named Rust
  function (recorded in `fred_behavior`); credited only with its callers.
- `adapted`: Emacs-specific plumbing (faces, overlays, compat shims, mouse
  menus, buffer-local machinery) whose user-visible effect Fred provides
  another way; `fred_behavior` says how.

## Architecture in Fred

- `src/org/`: the component. Parser (`element.rs`) works on buffer lines;
  commands mutate the buffer through Fred's undoable edits.
- Org buffers: files ending `.org` (or `-*- mode: org -*-`) get `ed.org`
  state: fold set, cached in-buffer settings (`#+TODO`, `#+TAGS`,
  `#+STARTUP`, ...).
- Folding is a Fred core feature added for Org: hidden line ranges skipped by
  rendering and vertical motion, with `...` after the visible header.
- Generated views (agenda, column summary, clock report, etc.) are read-only
  buffers like Magit views.
- Options: the Emacs variable names are read from `[org]` in
  `~/.config/fred/config.toml`, e.g. `org-todo-keywords = [...]`. Defaults are
  upstream defaults.
- `:org <command-name>` runs any ported interactive command by its Emacs name
  (M-x equivalent). Keys follow org-mode-map with Emacs chords (C-c C-t,
  M-left, S-right, ...), plus the user's `Space o` leader layout.

## Continuation

Follow [the roadmap](org-roadmap.md) and the [binding contract](org-user-bindings.md).
Update the ledger rows and roadmap queue in each implementation commit.
Preserve the user's uncommitted `notes.md`; never `git add -A`.

## Module contracts (for every implementation batch)

- Each module (`src/org/<name>.rs`) exposes
  `pub fn command(ed, name, arg) -> Option<Result<(), String>>` answering
  every upstream interactive command of its files by exact upstream name
  (aliases too), registered in `MODULES` in `src/org/mod.rs`. Context
  dispatchers (org-metaleft, org-shiftup, org-ctrl-c-ctrl-c, TAB) call
  commands by name through `org::call`, so modules do not depend on each
  other at compile time.
- `org::run` wraps each command in one undo group and records
  `last_command` (Emacs `last-command` for repeated keys).
- Edit only through `org::{splice, set_line, insert_lines, delete_lines}`;
  read with `org::{line, lines}`. Folds follow edits automatically.
- Prompts: `org::read` (minibuffer), `org::complete` (completing-read),
  `org::menu` (org-mks style key menus), `org::yes_or_no`; each takes a
  continuation. Session-wide work (other buffers/files, views):
  `org::effect(ed, |session| ...)`.
- Options: `org::options` / `org::sexp::option` by Emacs variable name with
  the upstream default; buffer-local `#+STARTUP` values via
  `settings(ed).opt(...)`.
- Context predicates (org-at-*-p): `org::ctx`. Visibility: `org::fold`.
- Keep pure logic in functions over `&str`/`&[String]` with unit tests;
  editor-level tests use `org::tests::org(text, keys)` and `shown(&ed)`.
- Read the pinned upstream source (`/private/tmp/fred-org-upstream/lisp`)
  for every ported command; reproduce messages, prompts and edge cases.
- Update `docs/org-parity.csv` rows (`state`, `fred_behavior`) for the
  symbols a batch ports.

## Time (`src/org/time/`)

Timestamps, the date prompt, scheduling, repeaters, diary sexps and
org-duration. Submodules: `civil` (Gregorian arithmetic, libc local time,
`format-time-string`, injectable now: `civil::set_now`), `stamp` (the
timestamp object and regexps, org-timestamp-change on strings), `read`
(org-read-date-analyze and Emacs `parse-time-string`), `diary`, `duration`.

API for other modules:
- `time::read_date(ed, ReadOpts, then)`: org-read-date; `then(ed, Analysis)`
  gets `tm`, `time_given`, `end_time`; `read::date_string` is the string form.
- `time::set_planning(ed, h, Some((Planning, ts_text)), &remove)` and
  `time::add_planning_info(ed, h, Some((Planning, tm, with_time)), &remove)`:
  org-add-planning-info. `planning_get`, `remove_timestamp_with_keyword`.
- `time::auto_repeat(ed, h) -> Result<Option<String>, String>`: the
  timestamp half of org-auto-repeat-maybe (the TODO module resets the
  state, writes LAST_REPEAT and logs first).
- `time::change_at_point` / `change_at` / `stamp::change`: org-timestamp-change.
- `time::insert_timestamp`, `stamp::stamp_text`, `stamp::time_stamp_format`.
- `time::schedule(ed, deadline, arg, Some(time))`: org-schedule/org-deadline
  with a TIME argument.
- Logging: when org-log-reschedule/org-log-redeadline apply, the time module
  stores a `time::LogRequest` and calls `org-add-log-setup` by name; the
  logging module takes it with `time::take_log_request()`.
- `time::occur(ed, re, keep)`: org-occur with a callback (sparse trees);
  `time::set_ts_type` is org-ts-type for org-sparse-tree's `c`.
- `time::clock_update_time_maybe(ed, l)` for the clock module.
- `time::duration::{to_minutes, from_minutes, is_duration, Format}`.

Adaptations:
- No calendar window. The prompt shows the live interpretation
  (`Date+time [2026-10-04] => <2026-10-09 Fri>: `, `(=>F)` when pushed to
  the future). The keys of org-read-date-minibuffer-local-map (S-arrows,
  M-S-arrows, `<` `>` C-v M-v, `.` on an empty answer, `!`) move a virtual
  calendar date that joins the answer as upstream's org-ans2.
- org-goto-calendar (C-c >) sets that date (from the timestamp on the line,
  or today) and reports it with the day's holidays; the org-calendar-*
  commands move it; org-date-from-calendar (C-c <) inserts it or changes the
  timestamp at point to it.
- org-display-custom-times is per buffer (`Org::custom_times`, then
  `#+STARTUP: customtime`, then the option): `time::display_custom_times`;
  the renderer replaces `stamp::custom_overlays(line)` ranges.
- Holidays for org-calendar-holiday/org-class: US general holidays, Good
  Friday, Easter, Christmas (other calendars not computed).
- `++` repeaters do not ask "Continue?" after 10 shifts.
- Commands never see an active Visual region, so org-schedule/org-deadline
  do not loop over headlines in a region.

## Attachments (`src/org/attach.rs`)

org-attach.el and org-attach-git.el. API: `attach::dir(ed, no_fs_check)`
(org-attach-dir), `attach::with_dir(ed, then)` (org-attach-dir-get-create;
may prompt), `attach::expand`, `attach::file_list`, `attach::tag`,
`attach::expand_links` (for export). links.rs follows `attachment:` links
through `org-attach-follow` (call_with the path); refile.rs's
archive_subtree runs `org-attach-archive-delete-maybe`;
`org-attach-dired-to-subtree` (`:org-attach-dired-to-subtree` in dired)
attaches to the most recent Org buffer (`Session::org_recent_org_buffer`).
Hooks: org-attach-after-change-hook / org-attach-open-hook are lists of
names; `org-attach-git-commit` and `org-attach-git-annex-get-maybe` run the
git CLI. Gaps: no agenda entry point, custom org-attach-commands ignored,
file prompts take a typed path.

## org-crypt (src/org/crypt.rs)

- `gpg --armor` replaces EasyPG. A recipient key comes from CRYPTKEY
  (inherited) or `org-crypt-key`; otherwise symmetric encryption with a
  passphrase read in a masked prompt (`CmdLine::secret`) and passed via
  `--pinentry-mode loopback --passphrase-fd 0`.
- Ciphertext of an unchanged decrypted entry is reused by plaintext checksum
  (thread-local table) instead of text properties.
- `org-crypt-disable-auto-save` governs Fred's swap file
  (`Session::org_disable_swap`); `encrypt` and
  `org-crypt-use-before-save-magic` encrypt keyed entries in `write_own`.

## org-protocol (src/org/protocol.rs)

- No server: `fred 'org-protocol://SUB?k=v&…'` runs the handler at startup
  (register fred as the URL handler); `:org-protocol URL` does the same.
- `:org-NAME REST` passes REST as the command's string argument
  (`take_call_arg`).
- Capture link properties (`%:link`, `%:description`, `%:annotation`,
  `%:initial`, query keys) live in `capture::set_link_props`; plain captures
  fill them from the stored link.
- org-protocol-create(-for-org) stay missing until ox-publish exists.

## org-ctags and org-mobile (src/org/ctags.rs, src/org/mobile.rs)

- ctags: Fred reads etags `TAGS` tables itself. links.rs calls the
  `org-open-link-functions` command before searching custom-id, fuzzy,
  coderef and radio links; Err means "not handled".
- mobile: push/pull with openssl encryption and an internal MD5 for
  checksums.dat; edits apply through `todo::todo_to_inhibit` (no
  interactive notes). The agenda export (org-mobile-write-agenda-for-mobile)
  waits for the agenda port.

## org-compat (src/org/compat.rs)

- `FUNCTIONS`/`VARIABLES` are every `define-obsolete-{function,variable}-alias`
  in the pinned tree, sorted. Regenerate with a regexp over
  `lisp/*.el`: `\(define-obsolete-(function|variable)-alias\s+'(\S+)\s+'?(\S+)`.
- `dispatch` falls back to the replacement of an obsolete command;
  `options::get` reads an option's obsolete names from `[org]`.
- Emacs-version shims are `internal`; flyspell, imenu, speedbar, bookmark,
  calendar and ECB hooks stay `missing` until Fred has an equivalent.

## org-feed (src/org/feed.rs)

- `feed::update_in` is the buffer work of org-feed-update (verified against
  Emacs, including the FEEDSTATUS drawer's pp layout and sha1 hashes);
  fetching uses curl/wget, `file://` works.
- `org::effect` now chains: a second effect queued before the session runs
  the first runs after it (it used to replace it).

## org-lint (src/org/lint.rs)

- `lint::CHECKERS` mirrors `org-lint--checkers` (order, trust, categories);
  `generate` sorts reports by position like upstream.
- Golden data: `tools/org-lint-golden.el` turns every org-lint case of
  upstream's test-org-lint.el into src/org/lint/golden.json (Emacs output);
  `lint::tests` must match all of it.
- Static tables (OPTIONS items, export option keywords, default
  properties) were printed by Emacs with the default backends loaded;
  regenerate when the export port registers more backends.
- Upstream quirks kept: obsolete-properties-drawer always says "Incorrect
  contents"; `{{{title}}}` etc. need one argument; LaTeX-$ is the
  ambiguous-$ checker only.

## Clocking, timers and column view (`src/org/clock/`, `timer.rs`, `colview.rs`)

org-clock.el, org-timer.el and org-colview.el, registered in `MODULES`.

API for other modules:
- `clock::running()`, `clock::history()`, `clock::out_time()`; the running
  clock is session state found again by its open `CLOCK: [start]` line,
  tasks by `clock::Marker` (file, line, heading text; `clock::locate`).
- `clock::clock_in(ed, Prefix, start)`, `clock::clock_out(ed, state, quiet, at)`,
  `clock::out_if_current` (org-todo calls `org-clock-out-if-current`).
- `clock::sum::{sum, subtree_minutes, special_range}` (org-clock-sum,
  org-clock-special-range; pure over lines). `clock::clocksum` answers the
  CLOCKSUM/CLOCKSUM_T special properties (always computed, not only after
  an org-clock-sum); `clock::minutes` serves org-sort's `k`.
- `clock::table::get_clocktable(&Params)` for the agenda clock report;
  `clock::table::{dblock_write, table_data, write_default}`.
- `org::mode_line()` (clock + timer) is drawn in the status line;
  `org::tick(s)` runs from the app loop (status refresh, effort overrun
  notification, idle resolution / auto clock-out, countdown end, loading
  the persisted clock).
- `clock::display(ed, l, line)` is the renderer hook for org-clock-display's
  per-heading times (`Org::clock_overlays`, dropped on the next change).
- `colview::{compile_format, uncompile_format, summarize, entry_values,
  compute_all, capture}` for the agenda column view. Column view is a
  read-only `*Org Columns: FILE*` buffer; COLUMNS_MAP keys edit the source
  entries through the session and rebuild the view.

Remaining gaps (`partial` in the ledger): Lisp-valued options (state
switch functions, heading function, clocktable/columnview formatters,
colview collect functions) are not evaluated; scopes and `:id` targets in
other files are read from disk; persistence uses Fred's own file in the
state dir, written on every clock change; no agenda column view yet (the
agenda must call the exposed functions); frame-title display and beeps
have no Fred equivalent.
