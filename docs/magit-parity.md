# Magit source parity ledger

Baseline: [magit/magit at e9ed99c5e3cd](https://github.com/magit/magit/tree/e9ed99c5e3cdd3fab31204f2809ce9d01d672c89).

Goal: full user-visible command, option, section, and workflow parity as a Rust
component inside Fred. Global entry points use `Space m` as requested. The
upstream implementation is the behavioral reference, not just the manual.

[Command and option inventory](magit-parity.csv) records source symbols and line
numbers. `partial` means Fred has some behavior, not parity. `missing` means no
implementation is credited. No feature is complete merely because a similarly
named Git command exists.

The inventory combines explicit source declarations and a clean batch Emacs
expansion of all libraries in the pinned source. `origin` distinguishes these;
runtime-generated rows have no invented source line number. The first text scan
misclassified two internal helpers containing nested `(interactive)` forms; those
were removed after checking Emacs `commandp` and reading their source.

[Runtime surface snapshot](magit-runtime-surfaces.json) and
[reproducible extractor](../tools/magit-source-inventory.el) capture generated
commands/options, transient layouts, flattened inherited keymaps and mode
declarations. Emacs version is recorded in the snapshot. This is still a lower
bound for context-created menus, platform/version branches, Lisp customization
and external integrations; inventory is not verified behavioral parity.
Keymap records are raw traversal candidates, including shadowed inherited
bindings. They are not an effective binding lookup. Resolve inheritance and the
user's Evil-collection overrides before accepting the Fred UI mapping gate.
A percentage from these counts would not measure engineering effort.

The user's requested percentage is maintained separately in the
[completion roadmap](magit-roadmap.md): currently 3 / 100 accepted points.
It measures verified roadmap deliverables and intentionally credits no
full-family points for partial command wrappers.

| Upstream file | Commands (explicit + runtime) | Options |
| --- | ---: | ---: |
| git-commit.el | 23 | 13 |
| git-rebase.el | 26 | 3 |
| magit-apply.el | 16 | 5 |
| magit-autorevert.el | 1 | 5 |
| magit-base.el | 2 | 9 |
| magit-bisect.el | 12 | 1 |
| magit-blame.el | 20 | 9 |
| magit-branch.el | 26 | 7 |
| magit-bundle.el | 12 | 0 |
| magit-clone.el | 19 | 7 |
| magit-commit.el | 30 | 7 |
| magit-diff.el | 57 | 33 |
| magit-dired.el | 6 | 0 |
| magit-ediff.el | 11 | 4 |
| magit-extras.el | 33 | 4 |
| magit-fetch.el | 16 | 0 |
| magit-files.el | 23 | 1 |
| magit-git.el | 2 | 10 |
| magit-gitignore.el | 9 | 0 |
| magit-log.el | 76 | 20 |
| magit-margin.el | 4 | 0 |
| magit-merge.el | 16 | 0 |
| magit-mode.el | 19 | 20 |
| magit-notes.el | 14 | 0 |
| magit-patch.el | 25 | 1 |
| magit-process.el | 3 | 16 |
| magit-pull.el | 9 | 1 |
| magit-push.el | 20 | 1 |
| magit-reflog.el | 3 | 2 |
| magit-refs.el | 12 | 12 |
| magit-remote.el | 18 | 3 |
| magit-repos.el | 6 | 5 |
| magit-reset.el | 8 | 0 |
| magit-section.el | 26 | 8 |
| magit-sequence.el | 59 | 0 |
| magit-sparse-checkout.el | 7 | 0 |
| magit-stash.el | 25 | 2 |
| magit-status.el | 12 | 10 |
| magit-submodule.el | 17 | 7 |
| magit-subtree.el | 17 | 0 |
| magit-tag.el | 10 | 0 |
| magit-wip.el | 8 | 6 |
| magit-worktree.el | 6 | 2 |
| magit.el | 9 | 3 |

Current inventory: 773 commands and 237 options. Of the commands, 199 are
partial and 574 missing; 5 options are partial (defaults only) and 232 missing. The runtime surface
artifact also captures 51 transient prefixes, 565 menu entries, 53 keymaps with
1,621 bindings (including inheritance), and 25 companion/derived/minor modes.

Current foundation: generated read-only views, saved-file/index operations,
background reads, guarded mutation refresh, source-buffer preservation,
recoverable commit drafts and terminal handoff. Most upstream modules and
options still require porting; this remains a large implementation project.

Source review started with `magit-stash.el`: index-preserving apply/pop,
retaining a stash after failed index restoration, split worktree/index/untracked
inspection, stash creation plumbing, snapshots, branch transforms and export.
The source-specific differences are recorded as gaps, not silently omitted.
