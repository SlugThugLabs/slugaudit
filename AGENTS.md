# Guide for AI Coding Agents

Welcome! When inspecting, auditing, or developing within this repository, follow these guidelines:

## 1. Architectural Source of Truth
- **[`.planning/ARCHITECTURE.md`](.planning/ARCHITECTURE.md)** is the single, authoritative source of truth for the entire codebase.
- Consult it for the complete module map, data flow pipelines, process lifecycle, and the **11 core architectural invariants** (including `#![forbid(unsafe_code)]`, CAS atomic publishing, the no-panic-on-error policy, and the disposable SQLite database design).

## 2. Repository Layout & The `.planning/` Directory
- `.planning/`: High-density architectural and planning artifacts.
  - `.planning/ARCHITECTURE.md`: System design and specifications.
  - `.planning/archive/`: Historical decision logs (ADRs) and dependency records.
  - `.planning/slugaudit/project.db`: Ephemeral SQLite index and evidence cache (disposable derived data).
- `src/`: Production Rust implementation.
  - `server.rs`: Declarative MCP tool router.
  - `server_runner.rs`: Semaphore-gated (8 permits) blocking-pool execution.
  - `resources.rs`: Static MCP resources — the query cookbook and the index schema.
  - `sync/`: Indexing, discovery, and watcher reconciliation pipeline.
  - `store/`: Hardened SQLite storage (read-only query connections, WAL mode).
  - `watch/`: File watching and change reconciliation.
  - `tools/`: MCP tool handlers (`report`, `query`, `structure`, `leads`, `finding`, `finding_read`, `health`, `project_control`).
- `docs/`: MCP client setup guides for AI agents.
- `tests/`: Integration and stdio protocol test suites.

## 3. Threat Model (read this before auditing)
- SlugAudit's threat model is a **lazy/unsupervised AI agent**, NOT a hostile attacker. The AI agent already has filesystem access to the same directory as the DB. The DB is disposable derived data (delete and rebuild from source). Findings are session-scoped and auto-purged on process restart. The human is in the loop.
- Security is about **convenience guards** (preventing lazy agents from shooting themselves in the foot), not defending against a malicious attacker. The stdio transport is the auth boundary — no network listener, no multi-client multiplexing. SlugAudit does not implement auth because the process boundary handles it.
- `query` tool's read-only connection + subquery wrapper + authorizer hook are the real guards; a SQL keyword blocklist is defense-in-depth, not a blocker.
- `finding` tool stores AI-authored text verbatim — this is a feature, not a vulnerability, because the human reviewer reads it directly and session-scoped purge prevents cross-contamination.
- `structure` tool executes arbitrary Tree-sitter queries — the most interesting surface because the AI agent can ask SlugAudit to do work the human hasn't explicitly asked for.
- DB is meant to be throwaway so past agents don't contaminate it with lazy comments leading other AI to think something is meant to be left undone that isn't.

## 4. Working with SlugAudit MCP Tools
When performing code audits or symbol lookups on this codebase, prefer the native SlugAudit tools:
- **`report`**: High-level project shape, file counts, and language breakdown.
- **`query`**: Read-only SQL queries against the index. Inspect code and file contents directly via `files.content` without disk reads. Always join `evidence` with `files` (`ON evidence.file_id = files.id`).
- **`structure`**: Tree-sitter AST queries returning exact source `text`, line numbers, and spans (e.g. Rust: `(function_item name: (identifier) @name body: (block) @body)`).
- **`leads`**: Per-mechanic counts plus paged file/line rows for mechanical observations — call-site shapes (unwrap/expect/panic/todo/assert), diagnostics, TODO/FIXME/HACK/XXX comment markers, credential-shaped paths. Unranked: a count says how many observations of a shape exist, never whether any matters.
- **`finding` / `finding_read`**: Record and inspect validated findings bound to content hashes. Static `slugaudit://` resources carry the index schema and the SQL/Tree-sitter query recipes.

## 5. Production Code Must Not Panic
`clippy::unwrap_used` and `clippy::expect_used` are denied outside `cfg(test)` (`src/lib.rs` and every gate binary). Fallible work returns `Result`; a genuinely fatal condition (failed process startup) reports the error and picks an exit code instead of panicking. Tests are exempt — there a panic *is* the failure signal.
