# Observability and operational handling

## Tracing

`src/main.rs` initializes one `tracing-subscriber` writing exclusively to
stderr (`.with_writer(std::io::stderr)`), with ANSI color disabled
unconditionally — the MCP host that spawns this process pipes stderr for
its own logging, not a terminal, and color escape codes would corrupt any
log aggregation or parsing on the receiving end. Level defaults to `info`
and is overridable via `RUST_LOG` (standard `tracing-subscriber` env-filter
syntax, e.g. `RUST_LOG=debug` or `RUST_LOG=slugaudit_mcp_rust=debug`).

`stdout` carries only MCP JSON-RPC traffic; it is never a `tracing`
sink. This separation is asserted in `tests/stdio_protocol.rs`, which spawns
the real compiled binary and checks both directions: every stdout line
parses as JSON-RPC (so any stray tracing text there would fail the test
outright), and stderr is asserted to actually contain the fields described
below rather than merely "some output arrived."

Every tool call (`report`/`query`/`structure`/`finding`) runs inside one
`tool_call` span (`src/server.rs::run_blocking`), tagged with the tool name,
covering permit acquisition, the blocking work itself, and the outcome:

- a `tool call started` event when the call begins
- a `tool call completed` (info) or `tool call failed` (warn) event with
  `duration_ms`, at the end
- `publish completed` (info, from `sync::publish`) or `publish failed`
  (warn) with `revision_id`, `added`/`modified`/`deleted`/`unchanged`
  counts, and `retries` — every tool call syncs first, so this fires on
  every call, not just explicit writes
- per-tool completion events with tool-specific counts: `report built`
  (file/parser-failure/open-finding counts), `query executed` (row count,
  truncated), `finding recorded` (file, line-derived id) — deliberately
  never the finding's title/description/severity/category text

The span is entered inside the `spawn_blocking` closure via an explicit
`Span::enter()` guard, since Tokio's blocking thread pool runs outside the
async task machinery that normally carries span context across `.await`
points automatically — without that guard, events emitted from
`tools::*`/`sync::*` during the blocking work would not attach to the
per-call span at all.

## What never reaches the log

By construction, nothing in `src/server.rs`, `src/tools/*.rs`, or
`src/sync/*.rs` logs: SQL text passed to `query`, row/result content
returned by `query`, a finding's `title`/`description`/`severity`/
`category`, source file content, or any credential (this server holds none
— it only ever opens a local SQLite file by path). Only structural metadata
is logged: tool name, revision id, counts, durations, file paths (as
identifiers, not content), and error messages from typed error variants
(which are themselves written to never embed request content — see e.g.
`PublishError`'s `Display` impls).

## Operational handling

- **Busy timeouts**: every connection sets `PRAGMA busy_timeout = 5000`
  (`src/store/connection.rs::BUSY_TIMEOUT`) so a connection contending for a
  lock (e.g. two processes publishing concurrently) waits up to 5 seconds
  before SQLite returns `SQLITE_BUSY`, which surfaces as a typed
  `rusqlite::Error` propagated through the relevant `*Error` enum — never a
  silent hang, never a panic. Ordinary CAS retries (`sync::publish`'s retry
  loop) are a separate, faster-acting mechanism for the common case of a
  losing publisher; the busy timeout is the backstop for actual lock
  contention at the SQLite level.
- **Corrupted databases**: SQLite validates file format lazily, on first
  real access rather than at open — a garbage/corrupted file at the
  database path opens "successfully" at the OS level but fails on the
  first pragma with a typed `StoreError::Configure` error (verified by
  `store::connection::tests::a_corrupted_database_file_fails_closed_with_a_typed_error`).
  There is currently no automatic repair or quarantine-and-reinitialize
  path — a corrupted database blocks that project entirely until an
  operator removes or restores the file (see `PACKAGING.md`'s removal
  section for how the database file relates to the `.planning/slugaudit/`
  activation marker).
- **Parser failures**: never silently reported as a complete parse. A
  language pack that fails to load records `ParserAvailability::LoadFailed`;
  a parse that runs and then fails records `ParseOutcome::Failed`; both
  carry a `reason` string persisted to `files.parse_error_reason` and
  surfaced through `report`'s `parser_failure_count` — the AI calling
  `report` sees this as ordinary evidence, no special-case handling needed
  by the caller.
- **Resource-limit rejection**: every cap defined in `src/model/limits.rs`
  (file size, total import bytes, query response bytes, query value bytes,
  query wall-clock/step budgets, structure query size and match count,
  evidence item count/per-item/cumulative bytes) rejects with a typed error
  identifying which limit was hit and its configured value — never a
  truncated-but-unlabeled response, except where truncation is the
  documented behavior itself (e.g. `query`'s row cap sets `truncated: true`
  in the response rather than erroring, since partial results are more
  useful there than a hard failure). `structure` currently bounds query
  text size and match count but has no execution-time/CPU limit on the
  Tree-sitter query itself — a pathological pattern against a large file
  has no timeout today; this is a known gap, not a documented guarantee.
