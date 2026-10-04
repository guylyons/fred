# Magit source parity ledger

Baseline: [magit/magit at e9ed99c5e3cd](https://github.com/magit/magit/tree/e9ed99c5e3cdd3fab31204f2809ce9d01d672c89).

Goal: full user-visible command, option, section, and workflow parity as a Rust
component inside Fred. Global entry points use `Space m` as requested. The
upstream implementation is the behavioral reference, not just the manual.

[Command and option inventory](magit-parity.csv) records source symbols and line
numbers. `partial` means Fred has some behavior, not parity. `missing` means no
implementation is credited. No feature is complete merely because a similarly
named Git command exists.

The inventory includes explicit interactive defun/cl-defun, transient prefix/
suffix declarations and defcustom options. It is a lower bound: generated
commands, section/keymap interactions, git-commit/git-rebase companion features,
Lisp customization hooks and external integrations require separate review.
A percentage from these counts would not measure engineering effort.

The user's requested percentage is maintained separately in the
[completion roadmap](magit-roadmap.md): currently 2 / 100 accepted points.
It measures verified roadmap deliverables and intentionally credits no
full-family points for partial command wrappers.

| Upstream file | Explicit commands | Customization options |
| --- | ---: | ---: |
| git-commit.el | 18 | 10 |
| git-rebase.el | 25 | 3 |
| magit-apply.el | 16 | 5 |
| magit-autorevert.el | 0 | 3 |
| magit-base.el | 2 | 9 |
| magit-bisect.el | 8 | 1 |
| magit-blame.el | 14 | 7 |
| magit-branch.el | 17 | 7 |
| magit-bundle.el | 6 | 0 |
| magit-clone.el | 8 | 7 |
| magit-commit.el | 17 | 7 |
| magit-diff.el | 35 | 33 |
| magit-dired.el | 6 | 0 |
| magit-ediff.el | 11 | 4 |
| magit-extras.el | 26 | 4 |
| magit-fetch.el | 10 | 0 |
| magit-files.el | 20 | 0 |
| magit-git.el | 2 | 10 |
| magit-gitignore.el | 9 | 0 |
| magit-log.el | 32 | 20 |
| magit-margin.el | 4 | 0 |
| magit-merge.el | 10 | 0 |
| magit-mode.el | 18 | 20 |
| magit-notes.el | 7 | 0 |
| magit-patch.el | 5 | 1 |
| magit-process.el | 4 | 16 |
| magit-pull.el | 5 | 1 |
| magit-push.el | 12 | 1 |
| magit-reflog.el | 3 | 2 |
| magit-refs.el | 6 | 12 |
| magit-remote.el | 11 | 3 |
| magit-repos.el | 6 | 5 |
| magit-reset.el | 8 | 0 |
| magit-section.el | 26 | 8 |
| magit-sequence.el | 33 | 0 |
| magit-sparse-checkout.el | 6 | 0 |
| magit-stash.el | 18 | 2 |
| magit-status.el | 6 | 10 |
| magit-submodule.el | 10 | 7 |
| magit-subtree.el | 9 | 0 |
| magit-tag.el | 5 | 0 |
| magit-wip.el | 7 | 4 |
| magit-worktree.el | 6 | 2 |
| magit.el | 7 | 3 |

Inventory: 514 command declarations and 227 options.

Current foundation: generated read-only views, saved-file/index operations,
background reads, guarded mutation refresh, source-buffer preservation,
recoverable commit drafts and terminal handoff. Most upstream modules and
options still require porting; this remains a large implementation project.

Source review started with `magit-stash.el`: index-preserving apply/pop,
retaining a stash after failed index restoration, split worktree/index/untracked
inspection, stash creation plumbing, snapshots, branch transforms and export.
The source-specific differences are recorded as gaps, not silently omitted.
