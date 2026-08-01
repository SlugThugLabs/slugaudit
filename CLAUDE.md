# SlugAudit agent guidance

Read `app/instructions.py` (the client-visible `MCP_INSTRUCTIONS` contract) and
`README.md` before changing the tool surface or synchronization behavior.
SlugAudit is an AI evidence index, not an autonomous auditor and not a
human-facing code browser.

## What this is, precisely

SlugAudit is infrastructure an LLM calls — the same category as a host's
built-in `Read`/`Grep`/`Write` tools, not a CLI or application a person
operates. The *only* thing a human ever invokes directly is the two-word
activation toggle (`/slugaudit on|off`). Every one of the 8 MCP tools
(`audit_overview`, `audit_search`, `audit_read_file`, `audit_dependents`,
`audit_brief`, `audit_finding`, `audit_raw_sql`, `audit_file_tree`) exists
purely for the calling model to use unattended. Its value proposition is
specifically about what it removes from that model's job: mandatory,
automatic freshness (no manual sync/rebuild step to remember) and a
pre-parsed, searchable index (no re-reading flat files file-by-file to
rebuild a mental model of the codebase from scratch every call). If a change would
add a step a human has to remember to run, or would make the model re-derive
something the database already computed once, it's moving against this
tool's actual reason to exist — reconsider it rather than adding it.

## Non-negotiable invariants

- `.planning/slugaudit/` is the project activation trigger.
- Project activation UX is only `/slugaudit on|off` in host adapters.
- Every AI query must hash the complete non-ignored, non-binary file set —
  not just the 8 Tree-sitter languages — and prove DB freshness first. Every
  file gets indexed for `audit_search`/`audit_read_file`; the 8 languages
  additionally get signature/import/risk-pattern extraction. Never scope
  discovery back down to a language allowlist — that reintroduces exactly
  the "fall back to reading flat files" cost this tool exists to remove.
- New and modified files replace their derived facts; deleted files purge all
  obsolete evidence.
- Revision publication is transactional. Never expose partial evidence.
- Sync failure fails the query. Never restore a stale fallback — there is no
  `auto_sync` opt-out; that would contradict this guarantee.
- The standalone stdio MCP is canonical; clients must not create parallel
  schemas or indexing engines.
- Automated risk patterns are leads. The AI supplies judgment and may persist
  reviewed conclusions with `audit_finding`.
- `_project_root` is default-safe by construction, not by convention: with no
  `SLUGAUDIT_HOST_TOKEN` configured it is unconditionally rejected and the
  process always uses its own `cwd` — there is no unauthenticated fallback
  path (see README's "Host-adapter protocol" section). Do not reintroduce one
  for "backward compatibility"; this project has no downstream deployments to
  stay compatible with. Do not widen what `_project_root`/
  `_slugaudit_project_control` accept without also widening
  `SLUGAUDIT_HOST_TOKEN` enforcement in `app/server.py`.
- Linux/macOS only. Do not introduce another POSIX-only dependency without
  updating the startup platform check in `mcp_server.py`, and do not silently
  drop the check either.
- PostgreSQL is optional, not required (see README's "Backends" section).
  SQLite (one file per project, `.planning/slugaudit/audit.db`) is the
  zero-config default whenever `Config.is_configured` is False. Never make
  PostgreSQL mandatory again without a very good reason — the zero-setup
  path is a deliberate distribution/adoption decision, not a stopgap.
- The SQLite backend is deliberately not full parity with PostgreSQL: it's
  scoped to the 7 tables and 7 tools (everything except `audit_raw_sql`)
  that the current design actually uses. Do not "complete" it into a second
  full-parity backend without discussing the maintenance-burden tradeoff
  first — see `repositories/sqlite/`'s module docstring.

## Language extractor lessons (from real bugs, not theory)

Three bug classes were found and fixed across the 8 extractors in one pass;
all three are easy to reintroduce by accident when adding or touching a
language, so they're recorded here rather than only in commit history:

- **Tree walkers must iterate `node.named_children`, never `node.children`.**
  Some grammars give a definition node and its own literal keyword token the
  *same* `.type` string — confirmed for tree-sitter-ruby, where the `class`/
  `module` definition node and the bare `class`/`module` keyword token
  inside it are both typed `"class"`/`"module"`, one named, one anonymous.
  Walking `node.children` dispatches on both, recording a bogus second
  "unnamed" signature for the keyword token. Anonymous nodes are literal
  syntax (keywords, punctuation) and must never be a dispatch target.
- **`_classify_import` must attempt real resolution, never guess "internal
  vs external" from a naming heuristic alone.** Python, Go, Java, and Ruby
  all shipped this exact bug simultaneously: classification defaulted to
  "external" for anything that didn't match a narrow syntactic pattern
  (a dot-prefix, a known JDK package prefix, a specific method name), which
  silently misclassified real local imports as external — meaning
  `resolve_import`'s otherwise-correct logic never even ran, since
  `build_dependency_edges` only resolves imports already tagged
  `"internal"`. The fix pattern: classify by calling `resolve_import` (or an
  equivalent local-module-map check, like Rust's `crate_map`) and trust
  whatever it returns, so classification and resolution can never disagree
  by construction. Go additionally needed `go.mod`'s declared module path to
  have any basis at all for "is this the same module" — there is no
  syntactic signal for that the way Python/JS relative imports or C/C++
  quoted includes provide.
- **A `decorated_definition`-style wrapper node must never be reached into
  and extracted from directly if the walker will also visit the wrapped
  node naturally.** Python's decorated-function handling did both — manually
  extracting the wrapped `function_definition` from within the
  `decorated_definition` branch, then the walker visited that same child
  again as an ordinary named child — recording every decorated function
  twice. Decorated classes never had this bug because `class_definition`
  was only ever handled by the plain dispatch branch.

`tests/test_import_resolution.py` and `tests/test_circular_imports.py` are
the regression suites for these; `tests/test_extractors.py` has the specific
duplicate-extraction regressions inline on the tests they broke.

## Architecture

- `domain/`: plain data classes shared across layers (`Project`, `File`,
  `Signature`, `ImportRecord`, `DependencyEdge`, `ImportResult`) — no
  behavior, no persistence, no backend awareness
- `app/manifest.py`: deterministic polyglot discovery and disk hashing
- `app/state.py`: versioned local manifest and atomic state replacement
- `app/sync.py`: mandatory pre-query freshness gate, cross-process lock, and
  the call into `services/sqlite_migration.py` once PostgreSQL is available
- `app/server.py`: standalone MCP routing, freshness response metadata, and
  the host-token gate on `_project_root`/`_slugaudit_project_control`
- `app/activation.py`: reusable host adapter functions for on/off
- `app/pool.py`: `get_connection_for_project()` picks PostgreSQL or SQLite
  per call based on `Config.is_configured` — this is the one place that
  decision gets made; everything downstream just gets "a connection"
- `app/tools.py`: tool schemas (all `additionalProperties: false`) and input
  validation, including the constrained `audit_raw_sql` grammar
- `app/handlers.py`: pure tool logic, one function per tool;
  `handle_raw_sql` is the one handler that branches on backend
  (`isinstance(conn, sqlite3.Connection)`) since that tool has no SQLite
  equivalent
- `languages/`: one Tree-sitter extractor per language (`LANG_MAP`); shared
  tree-walking lives in `languages/base.py` and is iterative, not recursive —
  do not reintroduce unbounded recursion over untrusted ASTs here
- `services/import_service.py`: Tree-sitter reconciliation transaction —
  backend-agnostic; it only ever calls the `make_*_repository()` factories
- `services/schema_service.py`: idempotent PostgreSQL schema migration;
  statement splitting is dollar-quote/string aware — do not revert to a bare
  `sql.split(";")`
- `services/sqlite_schema_service.py`: idempotent SQLite schema
  initialization — simpler by construction (`executescript`, no
  legacy-migration savepoint dance; see its module docstring)
- `services/sqlite_migration.py`: one-time findings migration when
  PostgreSQL becomes available for a project that had been running on
  SQLite — see its module docstring for why only findings, not everything
- `repositories/`: project-scoped PostgreSQL persistence and current
  revision publication; every query is parameterized except the hardcoded
  (never user-derived) table-name list in `ProjectRepository.purge_project`.
  `make_project_repository()`/`make_file_repository()`/etc. (in
  `repositories/__init__.py`) are how callers get the right backend without
  branching themselves — always go through these, never import a concrete
  `ProjectRepository`/`FileRepository`/etc. class directly outside a test
- `repositories/sqlite/`: SQLite siblings of the repositories above, same
  method names/signatures, scoped to only what the current tool surface
  calls — see its module docstring for the maintenance-burden rationale
- `infrastructure/sqlite_db.py`: per-project SQLite connection setup
  (`PRAGMA foreign_keys`, the registered `REGEXP` function backing
  `audit_search`'s regex mode under SQLite)
- `schema.sql`: PostgreSQL schema and idempotent migrations
- `sqlite_schema.sql`: SQLite schema — the 7-table scoped subset, not a
  translation of the full `schema.sql`

## Verification

```bash
python3 -m pytest -q
mypy --strict app languages repositories services domain infrastructure mcp_server.py
ruff check app languages repositories services domain infrastructure mcp_server.py
git diff --check
```

Use Python 3.11+. PostgreSQL 15+ is only needed if you're testing the
PostgreSQL backend specifically — the SQLite backend needs nothing beyond
the stdlib and is exercised directly (not mocked) by the normal test run.
Preserve the public MCP contract and validate live PostgreSQL behavior for
schema, transaction, or synchronization changes using
`tests/test_integration_db.py` against a disposable schema — see README for
how it's gated.
