"""Client-visible operating contract for AI users of SlugAudit."""

MCP_INSTRUCTIONS = """\
START HERE: SlugAudit is an evidence index for AI auditors, not an auditor.
The database removes repetitive filesystem discovery and source reads. You still
have to analyze behavior, prove failures, triage evidence, and write conclusions.

A project is enabled only when its root contains .planning/slugaudit/. Every tool
call verifies the complete non-ignored file set before answering: new and changed
files are re-indexed, deleted files and derived facts are purged, and only an
atomic, current revision may be queried. Sync is automatic; there are no manual
sync, rebuild, changed-file, or database-maintenance tools. Native host
integrations bind calls to their active project automatically.

Every file is indexed for audit_search and audit_read_file, regardless of
language — configs, docs, scripts, infra-as-code, anything that isn't binary.
You should never need to fall back to your own file-reading tools inside an
active project; if a file exists in the repo, it is in this index. Files in
one of the 8 languages with a Tree-sitter grammar additionally get parsed
signatures — functions, methods, classes, and every variable/field
declaration, not just top-level ones — resolved imports, and risk-pattern
leads; other files are fully indexed content without that extra structure.
To check how often a name is referenced, or whether two names look
suspiciously similar, use audit_search over the full indexed content rather
than assuming a name's role from its declaration alone.

Use audit_overview to orient, audit_search to find evidence, audit_read_file for
source, audit_dependents for blast radius, audit_file_tree for structure, and
audit_finding to persist conclusions that you have actually reviewed.
audit_raw_sql is an advanced constrained SELECT surface: it is always scoped to
the active project and cannot override that scope. Treat automated risk patterns
as leads, never as findings or scores. Do not focus only on recently changed files;
the full indexed codebase remains in audit scope.
"""

__all__ = ["MCP_INSTRUCTIONS"]
