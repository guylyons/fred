# Magit bundle and clone acceptance

| Scenario | Upstream | Fred | Evidence |
| --- | --- | --- | --- |
| Bundle menus | magit-bundle c v l; create args and c t u | Space m &, same keys | menu_entries('J','j') |
| Tracked bundles | tag message `;; git-bundle tracking` + alist | Same format written and read; update bundles TAG..BRANCH | magit::tests::bundle_create_tracked_update_verify_and_heads |
| Verify / list heads | process buffer | Message / output view | same test |
| Clone menu | dispatch C; fetch, setup, sharing args; C s d e > b m | Same | menu_entries('k') |
| Clone target | non-empty directory gets the repo name inside | Same | magit::tests::clone_regular_sparse_and_into_non_empty_directory |
| Sparse clone | --no-checkout, sparse-checkout init --cone, checkout | Same | same test |
| Credentials | async process with prompts | Background clone without prompts (agent keys only) | divergence |
