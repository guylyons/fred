# Magit workflow expansion

Continue toward functional parity using the published Magit manual
(https://docs.magit.vc/magit/). This is a cohesive extension of the existing
component. The user has authorized continuing the port without further
permission prompts; execute inline and verify with real Git repositories.

## Remaining parity work

The initial component covers status, file/hunk stage/unstage, ordinary
commits, default remotes, recent history, and local branch switching.
Outstanding areas include transient command arguments, line/region staging,
discard/reset, richer log/diff/ref/blob views, blame/reflog/bisect,
interactive history editing, conflict comparison, worktrees/submodules,
remote configuration and refspecs, patches/mail workflows, cloning/init,
sparse checkouts, notes and repository maintenance. Functional workflow
parity is the target; Emacs Lisp extension APIs are not Rust UI features.

## This expansion

Add visible suffix menus in Fred's existing picker panel and typed prompts
through the existing command line. All global entry points remain under
Space m. Preserve Space m s status, p push, P pull, f fetch, c commit,
l log and b local branch picker.

- Space m z: stash menu: z save tracked changes, u save including untracked,
  i save staged changes, k save while keeping index, l list, a apply,
  p pop, d drop. Optional messages are prompted for stash creation.
- Stash view: Enter inspect patch, a apply, p pop, d drop (type yes),
  gr refresh, q return. Preserve original stash OIDs and reflog selectors;
  reject stale pop/drop selection rather than deleting another entry.
- Space m B: branch menu: b existing local picker, c create, s create and
  switch, r rename current branch, d delete using Git's merged-only -d,
  t switch/create a tracking branch from a remote branch.
- Space m t: tags: l list, c create lightweight, a create annotated
  with a typed message, d delete (type yes). Enter inspects selected tag.
- Space m C: commit actions: a amend using an editable draft prefilled
  with HEAD's message, e amend without editing the message, f fixup a
  selected/read commit. Ordinary Space m c behavior remains unchanged.
- Space m M: merge: m merge target, s squash target, c continue, a abort.
- Space m r: rebase: r onto target, u onto upstream, c continue,
  s skip, a abort. Interactive todo editing remains explicit parity work.
- Space m x: cherry-pick: p pick commit, c continue, s skip, a abort.
- Space m v: revert: v revert commit, c continue, s skip, a abort.

Use revision prompts for operations requiring a target, accepting a single
revision, never shell text. Resolve revisions to Git OIDs with --end-of-options
before building a mutation command, so inputs beginning with '-' cannot
become options. Branch names use Git's check-ref-format validation; remote
tracking requires a remote ref, not arbitrary HEAD. Tag arguments similarly
validate as refs. Structured argv remains mandatory.

Status displays merge/rebase/cherry-pick/revert in-progress state, using
Git-resolved metadata paths for linked worktrees. Continue/skip/abort check
that the matching operation is active. A failed operation still refreshes
status so conflicts and in-progress state are visible while preserving
Git's failure message. Conflict resolution uses ordinary file editing,
staging, and operation continue; do not pretend a three-way merge UI exists.

Mutations use the established interactive terminal handoff and Ctrl-C
restoration. If a continuation would need Git's editor, reuse Fred's commit
draft where supported or use --no-edit/GIT_EDITOR=true for the existing
message; never start an unconfigured external editor in raw mode.

No source buffers are automatically saved. Successful commands refresh
component state and gutter baselines. Menus and prompts cancel on Esc or
Ctrl-G without an operation or a stale question affecting a later prompt.
An open prompt/selection carries repository identity. Draft amend mode
survives parking and recovery; failed commits retain both message and mode.
Amend refuses to replace a nonempty ordinary draft silently.

## Verification

Real repository tests cover stash creation/apply/pop/drop, staged/keep-index
variants, stale stash selectors, branch create/rename/tracking/delete refusal,
lightweight and annotated tags, amend/fixup contents, merge conflicts plus
abort/continue, rebase conflicts plus abort/continue, cherry-pick/revert,
linked-worktree operation state, and option-injection rejection.
Session tests cover menu suffixes, cancellation, prompts, repository anchors,
read-only generated views, amend draft recovery, and failure refresh.
Pseudo-terminal tests exercise visible menus and stash/operation workflows.
Run cargo test, cargo fmt --check, release build, and Clippy with only the
known existing draw_picker too_many_arguments warning excluded. Independent
review follows implementation. Preserve the user's notes.md.
