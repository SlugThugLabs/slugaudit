# SlugAudit MCP — AI Operating Contract

## Start here

SlugAudit is an evidence index for AI auditors. It does not perform audit
judgment. Tree-sitter and PostgreSQL remove repetitive discovery and file reads;
the AI still analyzes behavior, proves failures, triages evidence, and writes the
findings.

A project is enabled only when its root contains `.planning/slugaudit/`. The only
human-facing project command is `/slugaudit on|off`, supplied by an integrating
client such as ClauRust. This standalone MCP exposes no manual sync, rebuild,
changed-file, parsing, or database-maintenance tools.

Before every AI query, SlugAudit must:

1. Discover the complete supported source set.
2. Hash it and compare it with the stored manifest.
3. Import new files.
4. Completely replace facts derived from changed files.
5. Purge deleted files and their symbols, imports, dependency edges, risks, and
   obsolete finding evidence.
6. Publish the new revision atomically.
7. Return evidence only from that verified revision.

Sync failures are query failures. Never return partial or knowingly stale
evidence.

## Tool workflow

- Start with `audit_overview` for project orientation.
- Use `audit_search` broadly to locate evidence across the complete codebase.
- Use `audit_read_file` to retrieve source from the verified index.
- Use `audit_dependents` to inspect incoming or outgoing dependency edges.
- Use `audit_file_tree` to understand repository structure.
- Use `audit_raw_sql` only for advanced, constrained single-table `SELECT`
  queries. The server always forces the active project scope.
- Use `audit_brief` only as a lead summary. It is not an audit conclusion.
- Use `audit_finding` to persist conclusions only after reviewing the evidence.

Automated risk patterns are leads, not findings or scores. Do not limit an audit
to recently changed files: old code may contain previously missed defects, and
the full indexed source set remains in scope.
