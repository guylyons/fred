# Magit submodule acceptance (magit-submodule.el)

| Scenario | Upstream | Fred | Evidence |
| --- | --- | --- | --- |
| Menu | -f -r -N -C -R -M -U; a r p u s d k; l f | Space m o, same keys and descriptions | menu_entries('o') |
| Add | url, path from url, name; submodule add --name N [--force] -- URL PATH | Same; paths outside the tree or naming .git refused | magit::tests::submodule_add_populate_list_and_remove |
| Suitable modules | register/populate: no worktree; update/sync: worktree | Same, enforced on the typed modules | same test |
| Arguments per suffix | magit-submodule-arguments filters | Same filters | same test (--rebase dropped from add) |
| Remove | dirty modules omitted, or with --force confirmed and stashed | Same; gitdir trashing (prefix) not offered | same test |
| List and visit | path, branch, describe or hash; RET visits | Same; Enter opens the module's status | same test |
| Fetch modules | magit-fetch-modules | o f | network tests |
