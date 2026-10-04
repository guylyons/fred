# Cherry-pick and revert acceptance scenarios

Source: pinned magit-sequence.el (cherry-pick and revert prefixes and suffixes).
Entries: Space m x / A (cherry-pick), Space m v / r / V (revert).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| A / a | Pick with -x, apply without commit (drops --ff), option-like answers rejected | cherry_pick_and_revert_suffixes_follow_magit_sequence |
| V / v | --edit single revert opens Fred draft from MERGE_MSG; mainline asked for merges | same test |
| d / n / s / h | Donate, spin out (non-tip, later commits kept), spin-off reachability, harvest | same test |
| In progress | A/V continue, s skip, a abort resolved at run time | session routing |
| Region selections, --mainline as an infix, gpg-sign/signoff, non-contiguous cherries | | Open |

Independent review (October 4): harvest only accepts a branch that contains
the cherries; donating from a detached HEAD stays on the rebased result;
reverting a range goes newest first; -e uses Git's --edit with Fred as the
editor (keeps the original author, works for series); continue/skip/abort act
on whichever sequence runs and abort asks first; moves refuse dirty trees and
merge commits before changing anything; better defaults; ssh BatchMode also
extends GIT_SSH_COMMAND.
