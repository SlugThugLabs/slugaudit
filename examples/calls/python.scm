; Python call sites for SlugAudit's Call evidence (one capture: @callee).
; A plain call `f(x)` captures `f`; an attribute call `obj.m(x)` captures `m`.
(call function: (identifier) @callee)
(call function: (attribute attribute: (identifier) @callee))
