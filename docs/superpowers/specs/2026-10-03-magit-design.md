# Magit component for Fred

## Intent and agreed scope

Implement a Rust Git component inside Fred, following the status-centered
workflow described in `notes.md`. The first slice covers collapsible status
sections, diffs, file and hunk staging/unstaging, commits, push/pull/fetch,
history, and branch selection. It must work in both inline and fullscreen
Fred and preserve existing open buffers and their unsaved changes.

The user requires a leader `m` namespace, with `s` for status and `p` for
push. Fred's leader is Space. The additional bindings below were included
in the approved scope proposal.

This is the first useful slice, not complete Magit parity. Rebase, merge
resolution, stash management, line selection within hunks, destructive
reset/discard operations, and configurable transient menus are later work.

## Approach

Use the installed Git CLI through Rust's `std::process::Command`, and reuse
Fred's buffer, navigation, prompt, and rendering facilities. Keep repository
state and operations in a `magit` module, distinct from `git.rs`, which
currently supplies editor gutter marks.

Alternatives are a libgit2 dependency or an external Git TUI. The CLI uses
the user's existing Git configuration and credentials without another Git
implementation dependency. An external TUI would not provide the requested
Fred component or its buffer workflow. CLI integration is the chosen approach.

## Entry points and keys

| Normal-mode sequence | Action |
| --- | --- |
| `Space m s` | Open or refresh repository status |
| `Space m p` | Push the current branch to its configured upstream |
| `Space m P` | Pull using fast-forward only |
| `Space m f` | Fetch from the configured remote |
| `Space m c` | Open a commit-message buffer |
| `Space m l` | Open recent history |
| `Space m b` | Pick a local branch to switch to |

After `Space m`, show the available suffixes in the message/prompt area.
Escape or Ctrl-G cancels the prefix. An invalid suffix reports the choices
without executing anything. Existing `Space` bindings retain their behavior.

Status-specific bindings are `Tab` to collapse/expand the selected section
or file, `s` to stage, `u` to unstage, `gr` to refresh, `Enter` to visit the
selected file or diff location, and `q` to return to the previous buffer.
Regular navigation and search work in the generated read-only status buffer.
The global `Space m` namespace remains available there.

## Status and diffs

Discover the repository from the current file's parent, the current Dired
directory, or Fred's working directory for an unnamed buffer. Resolve the
actual worktree root with Git, including linked worktrees. If no repository
exists, report that fact and leave the current buffer intact.

Show the branch (or detached HEAD), upstream and ahead/behind information
when available, and separate conflicts, untracked, unstaged, and staged
sections. A clean repository explicitly shows a clean status. Display
renames with their old and new paths and distinguish conflicts from ordinary
changes. Unborn repositories support status, staging, and a first commit.

Use NUL-delimited machine-readable status output. Retain paths separately
from their display labels so spaces, tabs, newlines, leading dashes, and
non-UTF-8 names cannot change command arguments or selection identity.
Escape control characters when displaying labels.

Expanding a tracked file loads its unified diff against the index for
unstaged changes or against HEAD for staged changes. Diff rows retain
file, side, and hunk identity. Color additions, deletions, and headers;
avoid syntax highlighting generated status text as a source file. Binary
changes and untracked files support whole-file staging without pretending
to have textual hunks. Git path quoting must not be used to reconstruct
filesystem paths from display text.

Refresh retains collapsed state and selection by section/file/hunk where
possible. If the selected item disappeared, select the nearest surviving
row. Opening a file from a hunk uses the corresponding source line, with
deleted lines clamped to a valid location.

## Staging and committing

On a file row, `s` stages that file's working-tree changes; `u` unstages
that file while preserving its working-tree contents. On a textual hunk,
apply only that hunk to the index, reversing it for unstaging. Section
headers do not implicitly stage or unstage every file in this slice.
Unsupported conflict, binary, or rename hunk operations explain that the
whole-file operation must be used instead. Whole-file unstaging also works
before the repository's first commit.

Generate index patches from Git's diff output with sufficient file headers
and correct hunk ranges, including additions, deletions, and missing final
newlines. Verify a selected patch against the current index before applying
it. If the repository changed since the displayed snapshot, reject stale
selection, refresh, and ask the user to select again; never silently apply
a different hunk. Refresh status and gutter baselines after index changes.

All operations act on disk and the index. Fred does not automatically save
modified file buffers. Report unsaved buffers for the same repository when
opening status or starting a commit so the user understands what is included.

`Space m c` opens a normal editable commit-message buffer tied to the
repository. `:w` retains its draft; `Space m c` in that buffer submits the
message, with the submission binding shown in its help. Reject an empty
message. Feed the message to Git on stdin and commit the index as it stands,
using the user's normal hooks and Git settings. Keep the draft on failure
and return to refreshed status on success. Do not stage files implicitly.
Require an explicit message rather than launching an external Git editor.

## Network operations, history, and branches

Push uses ordinary `git push`, without force or automatic upstream creation.
Pull uses `git pull --ff-only` to avoid starting an interactive merge/rebase.
Fetch uses ordinary `git fetch`. Show Git's diagnostics when upstream or
remote configuration is missing instead of inventing configuration.

Use Fred's existing terminal handoff facilities for operations that may
need authentication or interactive hooks. Restore the editor reliably on
success, failure, or interruption, then refresh repository state. These
commands are user-triggered; simply opening status never contacts remotes.

History initially shows the latest 100 commits reachable from HEAD, with
abbreviated hashes, subjects, author, and date. Enter opens the selected
commit's patch in a read-only buffer; an unborn repository shows empty
history. Local branch selection uses the existing picker and `git switch`;
Git decides whether working-tree changes permit switching. Detached HEAD
is displayed accurately. Branch creation and remote branch checkout are
outside this slice.

## Fred integration and operation lifecycle

Store status/diff data and row actions together in the component rather
than interpreting rendered text as commands. Session manages opening,
parking, and returning from component buffers; Editor routes component
keys before ordinary editing keys in read-only component views. Commit
drafts use ordinary editing, with component submission handled explicitly.
Rendering adds component colors and prefix help to the existing UI.

Generated buffers have stable identities per repository and view, are
read-only, create no swap files, and are never written over source files.
Commit drafts remain recoverable editable buffers with a repository-specific
identity and must not be mistaken for generated read-only buffers. Store
drafts under Fred's state directory, keyed by the canonical worktree root;
`:w` saves there and existing swap recovery protects unsaved draft edits.
Never use the repository's source files or Git's own message files as the
draft path. Reopening the commit view resumes its existing draft. Clear the
saved draft only after a successful commit.

Repository reads run off the UI thread and return results on the application's
normal tick. Permit one active repository operation per component and show
its progress. Tag reads with repository identity and request generation so
late results cannot overwrite another repository or a newer snapshot.
Mutations are serialized with refresh; failed operations show stderr and
exit status without a success message. Cap displayed command output and
load file diffs on demand rather than collecting every patch on startup.

Pass arguments directly to `Command`, with `--` before paths where Git
supports it. Never interpolate filenames, branch names, or messages into
shell commands. Keep the existing terminal handoff API compatible while
adding a structured Git invocation if necessary.

## Verification and acceptance

Use temporary real Git repositories for meaningful integration tests:

- Status separates untracked, staged, unstaged, renamed, and conflicted
  paths; clean, unborn, detached HEAD, and linked worktree cases work.
- File and hunk stage/unstage change only the intended index content,
  preserving the working tree; cover multiple hunks, additions/deletions,
  missing final newlines, binary changes, and unusual filenames.
- Stale hunks fail safely and status selection survives ordinary refreshes.
- Commit creation uses exactly the staged content and entered message;
  failed hooks preserve the draft and editor state.
- A local bare remote verifies fetch, ordinary push, fast-forward pull,
  and refusal of a divergent pull without needing network access.
- Branch selection and commit inspection work, including Git failures.
- Key-routing tests cover every `Space m` suffix, prefix cancellation,
  existing leader bindings, and component-local stage/unstage keys.
- Terminal/render tests cover inline and fullscreen status, collapsing,
  file visits, and return to an unsaved original buffer.

Run the existing test suite, formatting, and lint checks appropriate to the
repository. Update README with the new bindings, status workflow, commit
submission, and the fact that Git operations use saved on-disk content.

The slice is complete when these workflows work inside Fred and the tests
demonstrate index correctness, preservation of unsaved buffers, and usable
terminal restoration after Git commands.
