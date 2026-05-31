# slugaudit-mcp

A PostgreSQL-backed code audit system for AI-assisted code reviews. Extracts code signatures and import dependencies from source files using tree-sitter, builds a dependency graph, tracks changes between imports, and generates structured briefings for AI auditors.

Available as both a **CLI tool** (`slugaudit-mcp`) and an **MCP server** (`slugaudit-mcp`).

## Quick Start

```bash
# 1. Install dependencies (system packages, Python deps, tree-sitter parsers)
bash setup.sh

# 2. Set up PostgreSQL connection
export PGHOST=localhost
export PGPORT=5432
export PGDATABASE=my_audit_db
export PGUSER=my_user
export PGPASSWORD=my_password

# 3. Import a project (schema auto-creates on first use!)
slugaudit-mcp import /path/to/your/project --project-name "My Project"

# 4. Check project status
slugaudit-mcp status --project "My Project"

# 5. Generate an AI audit briefing
slugaudit-mcp briefing --project "My Project" --output briefing.md
```

### MCP Server Mode (Recommended for AI Assistants)

```bash
# Run the MCP server
python3 mcp_server.py
```

Then in your AI client (Claude Desktop, VS Code, Cursor, Codebuff, etc.), configure the MCP server:

```json
{
  "mcpServers": {
    "slugaudit-mcp": {
      "command": "python3",
      "args": ["/path/to/slugaudit-mcp/mcp_server.py"],
      "env": {
        "PGHOST": "localhost",
        "PGPORT": "5432",
        "PGDATABASE": "my_audit_db",
        "PGUSER": "my_user",
        "PGPASSWORD": "my_password"
      }
    }
  }
}
```

Once configured, the AI can use these tools directly:
- `audit_import` — Import a codebase
- `audit_brief` — Generate an audit briefing
- `audit_status` — Show project status
- `audit_changed` — List changed files
- `audit_list` — List all projects
- `audit_init_db` — Manually initialize schema (usually auto-done)

## Architecture

The system follows a layered architecture with clear separation of concerns:

```
[CLI / MCP] → [Services] → [Repositories] → [Infrastructure]
                ↓
        [Languages] → [Source Files]
```

| Package | Role |
|---------|------|
| `infrastructure/` | Connection management, input validation, file I/O abstraction |
| `domain/` | Entity models: `Project`, `File`, `Signature`, `ImportRecord`, `DependencyEdge` |
| `repositories/` | Data access: `ProjectRepository`, `FileRepository`, `ImportRepository`, `FindingRepository`, `ArchitectureRepository` |
| `services/` | Application services: `SchemaService`, `ImportService`, `BriefingService` |
| `briefing/` | Briefing assembly: data providers + Markdown formatters |
| `languages/` | 8 language extractors with shared `BaseExtractor` |

## Commands

| Command | Description |
|---------|-------------|
| `init-db` | Initialize the database schema (idempotent) |
| `import` | Scan a project, extract signatures/imports, build dependency graph |
| `status` | Show project overview (files, signatures, imports, edges, changes) |
| `changed` | List files changed since last import |
| `briefing` | Generate structured Markdown briefing for AI audit |
| `list` | List all projects in the database |

## Supported Languages

| Language | Detection | Key Features |
|----------|-----------|--------------|
| **Rust** | `Cargo.toml`, `.rs` | `use`, `pub`, `struct`, `fn`, `impl`, `trait`, `enum` |
| **Python** | `pyproject.toml`, `.py` | `import`, `from X import Y`, class/fn signatures |
| **TypeScript** | `package.json`, `.ts` | `import`, `export`, interfaces, types, functions |
| **Go** | `go.mod`, `.go` | Functions, methods, structs, interfaces, imports |
| **Java** | `pom.xml`, `.java` | Classes, interfaces, enums, records, methods |
| **C** | `.c`, `.h` | Functions, structs, unions, enums, `#include` |
| **C++** | `.cpp`, `.hpp` | Functions, classes, templates, namespaces |
| **Ruby** | `Gemfile`, `.rb` | Methods, classes, modules, `require` |

## Key Concepts

- **Signature cache**: Public API surface of each file stored as JSONB. Lets the AI understand a file's interface without reading its full source.
- **Dependency edges**: File-to-file dependencies resolved from import statements. Enables blast radius computation.
- **Change detection**: SHA-256 hash compared against `last_audited_hash`. Only changed files become audit targets.
- **Blast radius**: Files that depend on changed files, computed from dependency edges.
- **Ghost context**: Signatures of unchanged files provided for AI reference — reduces token usage by avoiding full source reads.

## Configuration

### Environment Variables

| Variable | Required | Default |
|----------|----------|---------|
| `PGHOST` | Yes | — |
| `PGPORT` | No | 5432 |
| `PGDATABASE` | Yes | — |
| `PGUSER` | Yes | — |
| `PGPASSWORD` | Yes | — |
| `PGSSLMODE` | No | — |

### Connection String

Override env vars with `--connection`:

```bash
slugaudit-mcp import . --connection "postgresql://user:pass@host:5432/dbname?sslmode=require"
```

## Database Schema

All projects share one database. Each project's data is isolated via `project_id` foreign keys with `ON DELETE CASCADE`:

| Table | Purpose |
|-------|---------|
| `projects` | One row per project — the root entity |
| `files` | File metadata, hashes, signature cache (JSONB) |
| `file_imports` | Import statements extracted from files |
| `dependency_edges` | Resolved file-to-file dependency graph |
| `findings` | Audit findings with severity, category, location |
| `architecture_state` | Architecture summaries and layer maps (reserved) |
| `audit_configs` | Audit configuration per project (reserved) |
| `audit_runs` | Individual audit execution tracking (reserved) |
| `file_staleness` | Files that may be stale due to changes (reserved) |
| `static_tool_results` | Raw output from static analysis tools (reserved) |
| `ingestor_rejections` | Failed ingestion attempts (reserved) |

## Workflow

### First-time audit

```bash
# 1. Import the baseline
slugaudit-mcp import /path/to/project --project-name "My Project"

# 2. Make edits to the codebase...

# 3. Re-import to sync
slugaudit-mcp import /path/to/project

# 4. Generate briefing for changed files
slugaudit-mcp briefing --project "My Project" --output briefing.md

# 5. Feed briefing.md to an AI for audit
```

### Continuous audit with MCP

1. Call `audit_import` to scan the project
2. Call `audit_status` to verify the import
3. After code changes, call `audit_import` again to sync
4. Call `audit_brief` to get the generated briefing
5. The AI returns findings directly in conversation

## Adding a New Language

1. Create `languages/yourlang.py` extending `BaseExtractor`
2. Implement: `name()`, `source_extensions()`, `find_source_files()`, `extract_signatures()`, `extract_imports()`, `resolve_import()`, `hash_file()`
3. Register in `LANG_MAP` in `languages/__init__.py`
4. Install tree-sitter grammar: `pip install tree-sitter-yourlang`

## Testing

110 tests using `unittest` with mocking — no database required:

```bash
python3 -m pytest tests/ -v       # preferred
python3 -m unittest tests          # alternative
```

## Code Quality

```bash
# Linting
ruff check .

# Type checking
mypy .

# Pre-commit hooks
pre-commit install
pre-commit run --all-files
```

Configured tools: **ruff** (E/W/F/UP/S/B/C4 rules, line-length=100), **mypy** (strict mode), **pre-commit** (trailing-whitespace, end-of-file-fixer, check-yaml, ruff, mypy).

## Requirements

- **Python 3.10+**
- **PostgreSQL 15+** (uses UUID, JSONB, `gen_random_uuid()`)
- **tree-sitter** parsers for each supported language

## License

MIT
