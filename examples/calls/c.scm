; C call sites for SlugAudit's Call evidence (one capture: @callee).
; A plain call `f(x)` captures `f`; a call through a struct member
; `s.f(x)` / `p->f(x)` captures `f`.
(call_expression function: (identifier) @callee)
(call_expression function: (field_expression field: (field_identifier) @callee))
