"""Client-visible operating contract for AI users of SlugAudit."""

MCP_INSTRUCTIONS = """\
START HERE: SlugAudit is an evidence index for AI auditors, not an auditor.
The database removes repetitive filesystem discovery and source reads. You still
have to analyze behavior, prove failures, triage evidence, and write conclusions.

A project is enabled only when its root contains .planning/slugaudit/. Every tool
call verifies the complete supported source set before answering: new and changed
files are parsed, deleted files and derived facts are purged, and only an atomic,
current revision may be queried. Sync and parsing are automatic; there are no
manual sync, rebuild, changed-file, parsing, or database-maintenance tools.

Use audit_overview to orient, audit_search to find evidence, audit_read_file for
source, audit_dependents for blast radius, audit_file_tree for structure, and
audit_finding to persist conclusions that you have actually reviewed.
audit_raw_sql is an advanced constrained SELECT surface: it is always scoped to
the active project and cannot override that scope. Treat automated risk patterns
as leads, never as findings or scores. Do not focus only on recently changed files;
the full indexed codebase remains in audit scope.
"""

__all__ = ["MCP_INSTRUCTIONS"]
