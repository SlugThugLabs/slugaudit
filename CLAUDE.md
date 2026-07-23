# SlugAudit agent guidance

Read `instructions.md` before changing the tool surface or synchronization
behavior. SlugAudit is an AI evidence index, not an autonomous auditor and not
a human-facing code browser.

## Non-negotiable invariants

- `.planning/slugaudit/` is the project activation trigger.
- Project activation UX is only `/slugaudit on|off` in host adapters.
- Every AI query must hash the complete supported source set and prove DB
  freshness first.
- New and modified files replace their derived facts; deleted files purge all
  obsolete evidence.
- Revision publication is transactional. Never expose partial evidence.
- Sync failure fails the query. Never restore a stale fallback.
- The standalone stdio MCP is canonical; clients must not create parallel
  schemas or indexing engines.
- Automated risk patterns are leads. The AI supplies judgment and may persist
  reviewed conclusions with `audit_finding`.

## Architecture

- `app/manifest.py`: deterministic polyglot discovery and disk hashing
- `app/state.py`: versioned local manifest and atomic state replacement
- `app/sync.py`: mandatory pre-query freshness gate and cross-process lock
- `services/import_service.py`: Tree-sitter reconciliation transaction
- `repositories/`: project-scoped persistence and current revision publication
- `app/server.py`: standalone MCP routing and freshness response metadata
- `app/activation.py`: reusable host adapter functions for on/off
- `schema.sql`: PostgreSQL schema and idempotent migrations

## Verification

```bash
python3 -m pytest -q
mypy --strict app languages repositories services domain infrastructure briefing mcp_server.py
ruff check app languages repositories services domain infrastructure briefing mcp_server.py
git diff --check
```

Use Python 3.11+ and PostgreSQL 15+. Preserve the public MCP and validate live
database behavior for schema, transaction, or synchronization changes.
