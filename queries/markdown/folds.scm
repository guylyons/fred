; Folds, from nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/markdown/folds.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
([
  (fenced_code_block)
  (indented_code_block)
  (list_item
    (list))
  (section)
] @fold
  (#trim! @fold))

(section
  (list) @fold
  (#trim! @fold))
