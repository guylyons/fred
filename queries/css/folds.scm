; Folds, from nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/css/folds.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
[
  ; top-level block statements from https://github.com/tree-sitter/tree-sitter-css/blob/master/grammar.js
  ; note: (block) is not used due to unideal behavior when node before block node spans multiple lines
  (rule_set)
  (at_rule)
  (supports_statement)
  (media_statement)
  (keyframe_block)
  (import_statement)+
] @fold
