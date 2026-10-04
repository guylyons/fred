# Tag menu acceptance scenarios

Source: pinned magit-tag.el (prefix, create, delete, prune, release, list-releases).

| Surface | Scenario | Evidence / remaining work |
| --- | --- | --- |
| t | Lightweight and annotated tags; option-like names/revisions rejected; message required when annotating | tag_suffixes_create_release_delete_and_prune |
| r | Highest release, Release version commit naming, derived annotated message | same test; tag_versions_sort_like_version_to_list |
| k / p | Delete, prune local-only and remote-only tags with confirmations | same test |
| Fred adaptation | Messages are read by Fred and passed with -m (GIT_EDITOR is disabled) | Documented |
| --local-user, region delete, refs buffer after release | | Open |
