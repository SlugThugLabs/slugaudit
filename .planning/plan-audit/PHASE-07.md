# Plan Audit — Phase 7

Status: audited — FAIL

## Verdict

The MCP phase is too late and too underspecified to be the product boundary.
The SDK is selected only after most internals are designed, while tool schemas,
protocol framing, cancellation, concurrency, progress, and error mapping are
not locked early enough.

## Findings

1. MCP SDK selection is deferred until Phase 7, so earlier response/model
   decisions may not map cleanly to the actual SDK.
2. No exact tool list, JSON schemas, required fields, limits, or response
   examples are checked into the Rust plan.
3. No initialize/version capability contract is defined.
4. Stdout purity is tested, but framing, partial writes, malformed frames, and
   EOF behavior are not specified.
5. Concurrent tool calls and per-project serialization are not defined.
6. Cancellation and shutdown during sync/parser download are absent.
7. Progress is mentioned but no MCP capability or fallback behavior is chosen.
8. Internal errors versus user-visible protocol errors are not mapped.
9. The plan does not define whether a tool call automatically syncs before every
   query and how that affects latency.
10. The AI-facing descriptions are deferred to documentation, risking a tool
    surface that encourages interpretation beyond evidence.

## Required corrections

Move an MCP contract probe to Phase 0. Define exact schemas, initialize
handshake, framing, error mapping, cancellation, concurrency, progress, and
freshness-before-query behavior before implementing tool modules.

## Testing / logging

Add subprocess handshake, malformed frame, EOF, cancellation, concurrent call,
stdout contamination, progress, and protocol-error tests. Trace request ID,
tool name, project/revision, duration, result count, and error class to stderr.
