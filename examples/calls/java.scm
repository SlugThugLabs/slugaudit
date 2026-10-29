; Java call sites for SlugAudit's Call evidence (one capture: @callee).
; `m(x)`, `obj.m(x)` and `Type.m(x)` all capture the method name `m`.
(method_invocation name: (identifier) @callee)
