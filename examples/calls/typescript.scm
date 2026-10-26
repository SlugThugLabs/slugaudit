; TypeScript call sites for SlugAudit's Call evidence (one capture: @callee).
; A plain call `f(x)` captures `f`; a member call `obj.m(x)` captures `m`.
(call_expression function: (identifier) @callee)
(call_expression function: (member_expression property: (_) @callee))
