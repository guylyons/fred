# Magit subtree acceptance (magit-subtree.el)

| Scenario | Upstream | Fred | Evidence |
| --- | --- | --- | --- |
| Menus | magit-subtree i/e; import -P -m -s a c m f; export -P -a -b -o -i -j p s | Space m O, then i or e, same keys | menu_entries('u','I','E') |
| Prefix | from -P, else read; absolute paths must be inside the toplevel | Same; `..`, `.git` and option-like prefixes refused | magit::tests::subtree_commands_take_prefix_from_arguments_or_answers |
| Arguments | all but --prefix= passed after the prefix | Same | same test |
| Execution | async git subtree | Terminal Git (network for add/pull/push) | same test (argv) |
