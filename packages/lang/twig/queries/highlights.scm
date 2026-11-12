; From nvim-treesitter (https://github.com/nvim-treesitter/nvim-treesitter,
; runtime/queries/twig/highlights.scm at 81295eb0c5fc05ab3796a28c9f160c3ac4098106). Apache-2.0.
(comment) @comment @spell

(filter_identifier) @function.call

(function_identifier) @function.call

(test) @function.builtin

(variable) @variable

(string) @string

(interpolated_string) @string

(operator) @operator

(number) @number

(boolean) @boolean

(null) @constant.builtin

(keyword) @keyword

(attribute) @attribute

(tag) @tag

(conditional) @keyword.conditional

(repeat) @keyword.repeat

(method) @function.method

(parameter) @variable.parameter

[
  "{{"
  "}}"
  "{{-"
  "-}}"
  "{{~"
  "~}}"
  "{%"
  "%}"
  "{%-"
  "-%}"
  "{%~"
  "~%}"
] @tag.delimiter

[
  ","
  "."
] @punctuation.delimiter

[
  "?"
  ":"
  "="
  "|"
] @operator

(interpolated_string
  [
    "#{"
    "}"
  ] @punctuation.special)

[
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

(hash
  [
    "{"
    "}"
  ] @punctuation.bracket)
