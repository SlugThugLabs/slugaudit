;; Tree-sitter AST query for Rust function definitions, parameters, and bodies.
;; Pass this to the `structure` tool to match exact AST spans across files.
(function_item
  name: (identifier) @fn_name
  parameters: (parameters) @params
  return_type: (_)? @return_type
  body: (block) @body)
