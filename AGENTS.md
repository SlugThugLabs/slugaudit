# Guide for AI Coding Agents

Welcome! When inspecting, auditing, or developing within this repository, follow these guidelines:

## 1. Architectural Source of Truth
- **[`.planning/ARCHITECTURE.md`](.planning/ARCHITECTURE.md)** is the single, authoritative source of truth for the entire codebase.
- Consult it for the complete module map, data flow pipelines, process lifecycle, and the **10 core architectural invariants** (including `#![forbid(unsafe_code)]`, CAS atomic publishing, and the disposable SQLite database design).

## 2. Repository Layout & The `.planning/` Directory
- `.planning/`: High-density architectural and planning artifacts.
  - `.planning/ARCHITECTURE.md`: System design and specifications.
  - `.planning/archive/`: Historical decision logs (ADRs) and dependency records.
  - `.planning/slugaudit/project.db`: Ephemeral SQLite index and evidence cache (disposable derived data).
- `src/`: Production Rust implementation.
  - `server.rs`: Declarative MCP tool router.
  - `server_runner.rs`: Semaphore-gated (8 permits) blocking-pool execution.
  - `sync/`: Indexing, discovery, and watcher reconciliation pipeline.
  - `store/`: Hardened SQLite storage (read-only query connections, WAL mode).
  - `watch/`: File watching and change reconciliation.
  - `tools/`: MCP tool handlers (`report`, `query`, `structure`, `finding`, `finding_read`, `health`, `project_control`).
- `docs/`: MCP client setup guides for AI agents.
- `tests/`: Integration and stdio protocol test suites.

## 3. Working with SlugAudit MCP Tools
When performing code audits or symbol lookups on this codebase, prefer the native SlugAudit tools:
- **`report`**: High-level project shape, file counts, and language breakdown.
- **`query`**: Read-only SQL queries against the index. Inspect code and file contents directly via `files.content` without disk reads. Always join `evidence` with `files` (`ON evidence.file_id = files.id`).
- **`structure`**: Tree-sitter AST queries returning exact source `text`, line numbers, and spans (e.g. Rust: `(function_item name: (identifier) @name body: (block) @body)`).
- **`finding` / `finding_read`**: Record and inspect validated findings bound to content hashes.
