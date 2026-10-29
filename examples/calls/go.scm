; Go call sites for SlugAudit's Call evidence (one capture: @callee).
; A plain call `f(x)` captures `f`; a selector call `pkg.F(x)` / `v.M(x)`
; captures `F` / `M`.
(call_expression function: (identifier) @callee)
(call_expression function: (selector_expression field: (field_identifier) @callee))
