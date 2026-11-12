; Folds, from nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/cpp/folds.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
; inherits: c

[
  (for_range_loop)
  (class_specifier)
  (field_declaration
    type: (enum_specifier)
    default_value: (initializer_list))
  (template_declaration)
  (namespace_definition)
  (try_statement)
  (catch_clause)
  (lambda_expression)
] @fold
