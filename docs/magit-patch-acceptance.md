# Magit patch and am acceptance (magit-patch.el, magit-am)

| Scenario | Upstream | Fred | Evidence |
| --- | --- | --- | --- |
| Patch menu | c w a s r | Space m W, same keys | menu_entries('W') |
| Create | range, or X^..X for one commit; format-patch args; visit cover letter | Same; upstream's `C-m x` argument keys are `=x` | magit::tests::patch_create_am_apply_save_and_request_pull |
| Apply | git apply [--index/--cached/-3] -- FILE | Same | same test |
| Save | diff buffer's range and args written to FILE | Same; refuses to overwrite | same test |
| Request pull | compose-mail with request-pull output | Read-only view of the output | same test |
| am | --3way default; maildir, patches; continue/skip/abort while applying | Same keys resolved at run time | same test |

Independent review (October 4): the output directory is expanded before
format-patch and the cover letter visited is the file Git reports; defaults
follow the action (commit or current branch for create, file for apply/am;
request-pull remote from the branch, end defaults to the current branch);
output views run once instead of re-reaching the network on refresh; commit
and stash buffers can be saved, without --stat; the am menu's idle `a` opens
the apply menu so its arguments are visible; am continue ignores submodules
when checking unstaged changes.
