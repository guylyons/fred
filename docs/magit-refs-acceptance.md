# Magit refs acceptance (magit-refs.el)

| Scenario | Upstream | Fred | Evidence |
| --- | --- | --- | --- |
| Menu | -c -M -m -N -n -s; y c o r/v | Space m y; verbosity on v | menu_entries('y') |
| Sections | branch description, local branches, remotes, tags | Same order and content; cherry bodies per ref not shown | magit::tests::refs_list_branches_remotes_tags_with_counts_and_filters |
| Focus column | @ for HEAD, * for the focus, counts when enabled | Same | same test |
| Tracking | ahead N>, <N behind, upstream (gone) | Same | same test |
| Filters | for-each-ref / tag --list args | Same | same test |
| Commit counts | nil / t / all | v cycles nothing, branches, all | session test magit_refs_view_opens_from_dispatch_and_cycles_counts |
