; Folds, from nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/c/folds.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
[
  (for_statement)
  (if_statement)
  (while_statement)
  (do_statement)
  (switch_statement)
  (case_statement)
  (function_definition)
  (struct_specifier)
  (enum_specifier)
  (comment)
  (preproc_if)
  (preproc_elif)
  (preproc_else)
  (preproc_ifdef)
  (preproc_function_def)
  (initializer_list)
  (gnu_asm_expression)
  (preproc_include)+
] @fold

(compound_statement
  (compound_statement) @fold)
