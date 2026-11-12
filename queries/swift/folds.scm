; Folds, from nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/swift/folds.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
; format-ignore
[
  (protocol_body)               ; protocol Foo { ... }
  (class_body)                  ; class Foo { ... }
  (enum_class_body)             ; enum Foo { ... }
  (function_body)               ; func Foo (...) {...}
  (computed_property)           ; { ... }

  (computed_getter)             ; get { ... }
  (computed_setter)             ; set { ... }

  (do_statement)
  (if_statement)
  (for_statement)
  (switch_statement)
  (while_statement)
  (guard_statement)
  (switch_entry)

  (type_parameters)             ; x<Foo>
  (tuple_type)                  ; (...)
  (array_type)                  ; [String]
  (dictionary_type)             ; [Foo: Bar]

  (call_expression)             ; callFunc(...)
  (tuple_expression)            ; ( foo + bar )
  (array_literal)               ; [ foo, bar ]
  (dictionary_literal)          ; [ foo: bar, x: y ]
  (lambda_literal)
  (willset_didset_block)
  (willset_clause)
  (didset_clause)

  (import_declaration)+
] @fold
