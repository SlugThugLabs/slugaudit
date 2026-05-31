# Audit Database (audit-db)

A tool that builds and queries a **PostgreSQL audit database** for any Rust, Python, TypeScript, Go, Java, C, C++, or Ruby codebase. It extracts signatures and imports from source files, builds a dependency graph, tracks changed files, and assembles structured briefings for AI-assisted code audits.

Available as both a **CLI tool** and an **MCP server** for seamless AI integration.

## Quick Start

### CLI Mode

```bash
# 1. Install dependencies
pip install psycopg2-binary tree-sitter tree-sitter-rust tree-sitter-python tree-sitter-typescript

# 2. Set up PostgreSQL connection
export PGHOST=localhost
export PGPORT=5432
export PGDATABASE=my_audit_db
export PGUSER=my_user
export PGPASSWORD=my_password

# 3. Import a project (schema auto-creates on first use!)
audit_db import /path/to/your/project --project-name "My Project"

# 4. Check project status
audit_db status --project "My Project"

# 5. Generate an AI audit briefing
audit_db briefing --project "My Project" --output briefing.md
```

### MCP Server Mode (Recommended for AI Assistants)

```bash
# Install and run the MCP server
pip install mcp
python3 mcp_server.py

# Or via package manager (after publishing):
# npx -y audit-db-mcp
# uvx audit-db-mcp
```

Then in your AI client (Claude Desktop, VS Code, Cursor, etc.), configure the MCP server:

```json
{
  "mcpServers": {
    "audit-db": {
      "command": "python3",
      "args": ["/path/to/audit-db/mcp_server.py"],
      "env": {
        "PGHOST": "localhost",
        "PGDATABASE": "audit_db",
        "PGUSER": "audit_user",
        "PGPASSWORD": "your_password"
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

## Zero-Config Setup

**No manual `init-db` needed!** The schema is automatically created on first import. Just set your PostgreSQL connection (env vars or `--connection`) and start importing.

> If you prefer explicit setup, `audit_db init-db` is idempotent and safe to re-run.

## Requirements

- **Python 3.10+**
- **PostgreSQL 15+** (uses UUID, JSONB, `gen_random_uuid()`)
- **Tree-sitter parsers** for each language you want to scan:
  - `tree-sitter-rust` for Rust
  - `tree-sitter-python` for Python
  - `tree-sitter-typescript` for TypeScript

## Installation

```bash
# Clone or copy the audit-db directory
git clone <url> ~/audit-db  # or copy it manually

# Install Python dependencies
pip install -r ~/audit-db/requirements.txt

# Optional: symlink for easy access
ln -s ~/audit-db/audit_db.py /usr/local/bin/audit_db
chmod +x /usr/local/bin/audit_db
```

## Database Setup

You need a PostgreSQL database. Create one:

```bash
createdb my_audit_db
```

**That's it!** The schema is auto-created on first import. If you want to pre-create it manually:

```bash
# Set env vars first, then:
audit_db init-db

# Or with a connection string:
audit_db init-db --connection "postgresql://user:pass@host:5432/dbname"
```

## Configuration

Connection can be configured in two ways:

### 1. Environment Variables (recommended)

| Variable | Description | Required |
|----------|-------------|----------|
| `PGHOST` | Database server hostname | **Yes** |
| `PGPORT` | Database server port | No (default: 5432) |
| `PGDATABASE` | Database name | **Yes** |
| `PGUSER` | Database username | **Yes** |
| `PGPASSWORD` | Database password | **Yes** |

```bash
export PGHOST=localhost
export PGPORT=5432
export PGDATABASE=audit_db
export PGUSER=audit_user
export PGPASSWORD=your_secure_password
```

### 2. Connection String (`--connection` / `-c`)

Pass this to any command. Overrides env vars:

```bash
audit_db import . --connection "postgresql://user:pass@host:5432/dbname"
```

## Commands

### `init-db`

Initialize the database schema. Creates all tables, indexes, and foreign keys.

```bash
audit_db init-db
```

### `import`

Scan a project directory and store signatures, imports, and dependency edges.

```bash
# Auto-detect language
audit_db import /path/to/project --project-name "My App"

# Specify language explicitly
audit_db import /path/to/project --language rust

# Use current directory
audit_db import . --project-name "$(basename $PWD)"
```

**Note:** The database schema is auto-created on first import — no `init-db` needed.

**What it does:**
1. Scans for source files (excludes `.`, `target`, `node_modules`, `__pycache__`, etc.)
2. Extracts public API signatures from each file using tree-sitter
3. Extracts import statements
4. Stores hashes for change detection
5. Builds dependency edges between files
6. Marks removed files

### `status`

Show an overview of a project in the database.

```bash
# Show latest project
audit_db status

# Show specific project
audit_db status --project "My Project"
```

Displays:
- File count and total size
- Signatures extracted
- Imports tracked
- Dependency edges
- Changed files since last audit
- Open findings

### `changed`

List files whose hash has changed since the last import.

```bash
audit_db changed --project "My Project"
```

### `briefing`

Generate a structured Markdown briefing for AI-assisted code audit. This is the main output of the tool — it packages everything an AI needs to audit the codebase.

```bash
# Print to stdout
audit_db briefing --project "My Project"

# Write to file
audit_db briefing --project "My Project" --output briefing.md

# Limit ghost context
audit_db briefing --project "My Project" --max-ghost-lines 200
```

**The briefing includes:**
- **Project overview** — file count, signatures, imports, dependency edges
- **Architecture** — from the `architecture_state` table
- **Ghost context** — signatures of *unchanged* files (so the AI doesn't need to read them)
- **Target files** — full source of changed files + blast radius dependents
- **Historical findings** — previous open findings from the DB
- **Output contract** — JSON schema for the AI's response

### `list`

List all projects in the database.

```bash
audit_db list
```

## Output Contract

The `briefing` command includes a structured output contract for the AI to return findings in this JSON format:

```json
{
  "findings": [
    {
      "file": "path/to/file.rs",
      "line_start": 42,
      "line_end": 55,
      "severity": "high",
      "category": "correctness",
      "title": "Short descriptive title",
      "description": "Full explanation of the issue..."
    }
  ]
}
```

## Supported Languages

| Language | Detection | Notes |
|----------|-----------|-------|
| **Rust** | `Cargo.toml`, `rust-toolchain.toml`, `.rs` files | Full support: `use`, `pub`, `struct`, `fn`, `impl`, `trait`, `enum` |
| **Python** | `pyproject.toml`, `setup.py`, `requirements.txt`, `.py` files | `import X`, `from X import Y`, class/fn signatures |
| **TypeScript** | `package.json`, `tsconfig.json`, `.ts`, `.tsx`, `.js` files | `import`, `export`, interfaces, types, functions |
| **Go** | `go.mod`, `go.sum`, `.go` files | Functions, methods, structs, interfaces, `import` specs |
| **Java** | `pom.xml`, `build.gradle`, `.java` files | Classes, interfaces, enums, records, methods, constructors |
| **C** | `.c`, `.h` files | Functions, structs, unions, enums, typedefs, `#include` |
| **C++** | `.cpp`, `.hpp`, `.cc`, `.cxx`, `.hxx`, `.hh`, `.ixx`, `.tpp` files | Functions, classes, structs, templates, namespaces, `#include` |
| **Ruby** | `Gemfile`, `Rakefile`, `.rb` files | Methods, classes, modules, `require`, `include`, `extend` |

Language is auto-detected. Override with `--language`.

## How It Works

The data flows through three stages:

```
[Source Files] 
     ↓ tree-sitter parsing
[Extractors] —→ signatures + imports
     ↓
[PostgreSQL DB] —→ dependency edges, change tracking
     ↓
[Briefing Assembler] —→ structured Markdown for AI audit
```

### Key Concepts

- **Signature cache**: Public API surface of each file (pub functions, structs, traits, exports). Stored as JSONB in `files.signature_cache`.
- **Dependency edges**: Which files depend on which, resolved from import statements to actual file paths. Stored in `dependency_edges`.
- **Change detection**: File hash is compared with `last_audited_hash`. Changed files become audit targets.
- **Blast radius**: Files that depend on changed files (computed from dependency edges).
- **Ghost context**: Signatures of unchanged files, provided for AI reference without reading the full source.

## Architecture

The database schema has 11 tables:

| Table | Purpose |
|-------|---------|
| `projects` | Top-level project metadata |
| `audit_configs` | Audit configuration per project |
| `audit_runs` | Individual audit execution tracking |
| `files` | File metadata, hashes, signature cache |
| `file_imports` | Import statements extracted from files |
| `dependency_edges` | Resolved file-to-file dependency graph |
| `file_staleness` | Files that may be stale due to changes |
| `findings` | Audit findings with severity, category, location |
| `architecture_state` | Architecture summaries and layer maps |
| `static_tool_results` | Raw output from static analysis tools |
| `ingestor_rejections` | Failed ingestion attempts |

## Adding a New Language

1. Create `languages/yourlang.py` extending `base.py`
2. Implement: `name()`, `source_extensions()`, `find_source_files()`, `extract_signatures()`, `extract_imports()`, `resolve_import()`, `hash_file()`
3. Add to `LANG_MAP` in `audit_db.py`
4. Install the tree-sitter grammar: `pip install tree-sitter-yourlang`

## Workflow

### First-time audit
```bash
# 1. Init DB
audit_db init-db

# 2. Import the baseline
audit_db import /path/to/project --project-name "My Project"

# 3. Make edits to the codebase...

# 4. Re-import to sync
audit_db import /path/to/project

# 5. Generate briefing for changed files
audit_db briefing --project "My Project" --output briefing.md

# 6. Feed briefing.md to an AI for audit
```

### Continuous audit
```bash
# Pre-commit hook: check what's changed
if audit_db changed --project "My Project" | grep -q "changed file"; then
    echo "Files have changed since last audit"
    audit_db briefing --project "My Project" --output briefing.md
fi

# Re-sync after each commit
audit_db import /path/to/project
```

## Tips

- The first import is always a **full** baseline. Subsequent imports only update changed files.
- `briefing --max-ghost-lines 500` controls how many ghost context lines to include. Lower = shorter briefings.
- Use `--connection` to switch between databases (e.g., local dev vs shared audit server).
- The schema is idempotent — `init-db` is safe to re-run.
