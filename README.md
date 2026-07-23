# SlugAudit MCP

SlugAudit is a PostgreSQL-backed evidence index built for AI code auditors. It
uses Tree-sitter to pre-parse supported source files and lets an AI search,
retrieve, and connect evidence without repeatedly opening hundreds of flat
files. SlugAudit does not decide whether code is correct; the AI still performs
the audit judgment.

## Project contract

A project is enabled by the presence of `.planning/slugaudit/`. An integrating
client may expose the only project-level human command:

```text
/slugaudit on
/slugaudit off
```

`on` creates the directory. `off` purges that project's database evidence and
then removes the directory. The reusable adapter functions are
`app.activation.enable_project` and `app.activation.disable_project`.

There are no human commands for importing, syncing, rebuilding, parsing,
changed files, or database maintenance. Before every AI tool query SlugAudit:

1. Discovers and hashes the complete supported, non-ignored source set.
2. Proves the local manifest and published database revision agree.
3. Parses and imports added files.
4. Replaces every derived fact for modified files.
5. Purges deleted files and obsolete derived evidence.
6. Rebuilds dependency edges and publishes one atomic current revision.
7. Answers only from that verified revision.

Any discovery, parsing, database, or state failure fails the query. There is no
stale fallback.

## MCP tools

| Tool | AI use |
|---|---|
| `audit_overview` | Project statistics, languages, and compact tree |
| `audit_search` | Case-insensitive literal or constrained regex search |
| `audit_read_file` | Retrieve indexed source by project-relative path |
| `audit_dependents` | Trace incoming blast radius or outgoing dependencies |
| `audit_file_tree` | Browse the complete indexed source tree |
| `audit_brief` | Compact project-wide risk leads and open findings |
| `audit_finding` | Persist an AI-reviewed conclusion against current evidence |
| `audit_raw_sql` | Constrained, project-scoped, database-enforced read-only query |

Every successful response includes `slugaudit_meta` with the contract version,
schema version, project ID, revision ID, manifest hash, sync timestamp, and
`freshness: verified`. Automated risk patterns are leads, not findings or
scores.

## Installation and configuration

SlugAudit requires Python 3.11+ and PostgreSQL 15+.

```bash
./setup.sh
cp config.toml.example config.toml
.venv/bin/slugaudit-mcp
```

Database settings can come from `config.toml`, a file selected by
`SLUGAUDIT_CONFIG`, or standard `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`, and
`PGPASSWORD` environment variables. Environment values take precedence.

Example stdio MCP registration:

```json
{
  "slugaudit": {
    "type": "stdio",
    "command": "slugaudit-mcp"
  }
}
```

`claude-code-install.sh` and `grok-install.sh` register that same standalone
stdio MCP executable for their respective clients. They do not fork the index,
schema, or synchronization behavior.

The MCP process must start in the project working directory, or beneath a
parent containing `.planning/slugaudit/`. The schema is migrated automatically
when the server first connects.

## Source coverage

Rust, Python, TypeScript/JavaScript, Go, Java, C, C++, and Ruby are indexed
together in polyglot repositories. Git projects use tracked plus untracked,
non-ignored files; non-Git projects use deterministic discovery. Generated,
vendor, dependency, hidden-state, binary, and unsupported files are excluded.

## Development gates

```bash
python3 -m pytest -q
mypy --strict app languages repositories services domain infrastructure briefing mcp_server.py
ruff check app languages repositories services domain infrastructure briefing mcp_server.py
python3 -m build
```

The standalone stdio MCP is the canonical engine. Native clients such as
ClauRust should act as adapters for working-directory and `/slugaudit on|off`
UX rather than maintaining a second schema or indexing implementation.
