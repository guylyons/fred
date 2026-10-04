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

Independent review (October 4): updating a tracked bundle only rewrites a
`.bundle` file (expanded like the create prompt, never through a symlink, an
existing file must already be a bundle) and the tag name is checked, since
tags can be fetched from others; clone now runs in the terminal like fetch
(credentials, progress, Ctrl-C) and its follow-up (remote HEAD, sparse init,
status) runs afterwards; bare and mirror clones report success instead of
opening status; a sparse clone of an empty remote skips the checkout.
