; Folds, from nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/python/folds.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
[
  (function_definition)
  (class_definition)
  (while_statement)
  (for_statement)
  (if_statement)
  (with_statement)
  (try_statement)
  (match_statement)
  (import_from_statement)
  (parameters)
  (argument_list)
  (parenthesized_expression)
  (generator_expression)
  (list_comprehension)
  (set_comprehension)
  (dictionary_comprehension)
  (tuple)
  (list)
  (set)
  (dictionary)
  (string)
] @fold

[
  (import_statement)
  (import_from_statement)
]+ @fold
