# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**slugaudit-mcp** is a PostgreSQL-backed code audit system for AI-assisted code reviews. It extracts code signatures and import dependencies from source files using tree-sitter, builds a dependency graph, tracks changes between imports, and generates structured briefings for AI auditors.

Available as both a **CLI tool** (`slugaudit-mcp`) and an **MCP server** (`slugaudit-mcp`).

## Key Commands

```bash
# Run tests (110 tests, ~0.06s)
python3 -m pytest tests/ -v
python3 -m unittest tests          # alternative

# Install dependencies
pip install -r requirements.txt

# CLI usage (after setting PG* env vars)
slugaudit-mcp init-db                           # Initialize DB schema
slugaudit-mcp import /path/to/project           # Import/scan a codebase
slugaudit-mcp status --project "Name"           # Show project overview
slugaudit-mcp changed --project "Name"          # List changed files
slugaudit-mcp briefing --project "Name"         # Generate audit briefing
slugaudit-mcp list                              # List all projects

# MCP server
python3 mcp_server.py                      # Start MCP server (stdio)
```

## Architecture

The system follows a layered architecture:

```
[CLI / MCP] → [Services] → [Repositories] → [Infrastructure]
                ↓
        [Languages] → [Source Files]
```

### Package Structure

| Package | Role |
|---------|------|
| `infrastructure/` | Connection management (`db.py`), input validation (`validators.py`), file I/O abstraction (`filesystem.py`) |
| `domain/` | Entity models: `Project`, `File`, `Signature`, `ImportRecord`, `DependencyEdge`, `ImportResult` |
| `repositories/` | Data access layer: `ProjectRepository`, `FileRepository`, `ImportRepository`, `FindingRepository`, `ArchitectureRepository` |
| `services/` | Application services: `SchemaService`, `ImportService`, `BriefingService` |
| `briefing/` | Briefing assembly: `providers.py` (data sources), `formatter.py` (Markdown generation), `assembler.py` (orchestration) |
| `languages/` | Language extractors for 8 languages with shared `BaseExtractor` |

### Core Modules (Backward Compatibility)

| Module | Role |
|--------|------|
| `slugaudit_mcp.py` | CLI entry point (6 commands) — delegates to services/repositories |
| `core.py` | Re-exports `import_project()`, `get_extractor()`, `ImportResult` from new packages |
| `db.py` | Re-exports connection functions; delegates CRUD to repositories |
| `brief.py` | Re-exports `assemble_briefing()` — still contains original implementation |
| `mcp_server.py` | MCP protocol server (6 tools) with async/thread-safe DB access |

### Key Concepts

- **Ghost context**: Provides signatures of unchanged files to AI without full source reads, reducing token usage
- **Blast radius**: Computes which files depend on changed files via dependency edges
- **Change detection**: SHA-256 hash comparison against `last_audited_hash`
- **Zero-config setup**: Schema auto-creates on first import
- **Connection pooling**: Thread-safe `ConnectionPool` with lazy initialization and lock protection
- **Batched imports**: DB commits every 100 files instead of per-file for performance

### Database Tables

`projects`, `audit_configs`, `audit_runs`, `files` (with JSONB `signature_cache`), `file_imports`, `dependency_edges` (with unique constraint), `file_staleness`, `findings`, `architecture_state`, `static_tool_results`, `ingestor_rejections`

## Configuration

Set PostgreSQL connection via environment variables:

```bash
export PGHOST=localhost
export PGDATABASE=audit_db
export PGUSER=audit_user
export PGPASSWORD=your_password
```

Or pass `--connection "postgresql://user:pass@host:5432/dbname"` to any CLI command.

## Adding a New Language

1. Create `languages/yourlang.py` extending `BaseExtractor`
2. Implement: `name()`, `source_extensions()`, `find_source_files()`, `extract_signatures()`, `extract_imports()`, `resolve_import()`, `hash_file()`
3. Register in `LANG_MAP` in `languages/__init__.py` (all 8 languages must be in LANG_MAP)
4. Install tree-sitter grammar: `pip install tree-sitter-yourlang`

## Testing

110 tests using `unittest` with mocking — no database required for most tests:

```bash
python3 -m pytest tests/ -v       # preferred
python3 -m unittest tests          # alternative
```

Test coverage includes: connection parsing (with sslmode), schema checks, `ImportResult`, `ConnectionPool` (thread safety), all DB upsert/delete/import functions, `get_changed_files()`, `get_project_names()`, `update_audit_timestamps()`, briefing formatting (`fmt_sig`), and extractor validation.

## Code Quality Tooling

```bash
# Linting
ruff check .

# Type checking
mypy .

# Pre-commit hooks (install first)
pre-commit install
pre-commit run --all-files
```

Configured tools: **ruff** (E/W/F/UP/S/B/C4 rules, line-length=100), **mypy** (strict mode), **pre-commit** (trailing-whitespace, end-of-file-fixer, check-yaml, ruff, mypy).
