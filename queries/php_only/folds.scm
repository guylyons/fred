; Folds, from nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/php_only/folds.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
[
  (if_statement)
  (switch_statement)
  (while_statement)
  (do_statement)
  (for_statement)
  (foreach_statement)
  (try_statement)
  (function_definition)
  (class_declaration)
  (interface_declaration)
  (trait_declaration)
  (enum_declaration)
  (function_static_declaration)
  (method_declaration)
  (namespace_use_declaration)+
  (array_creation_expression)
  (match_expression)
] @fold
