"""MCP tool definitions and input validation.

Separated from handlers so tool definitions are easy to find and
validation logic is testable in isolation.
"""

from typing import Any

from mcp.types import Tool

# ── Constants ──────────────────────────────────────────────────────────

MAX_SEARCH_RESULTS = 200
MAX_SQL_ROWS = 100
MAX_READ_PATHS = 10
MAX_READ_CHARS = 100_000
MAX_PATH_LENGTH = 500

# Raw SQL is intentionally a small query language, not an unrestricted
# PostgreSQL console. Every allowed table has a direct project_id column so
# the handler can enforce tenant isolation without parsing arbitrary SQL.
RAW_SQL_TABLES = frozenset({
    "architecture_state",
    "audit_configs",
    "audit_runs",
    "dependency_edges",
    "file_imports",
    "files",
    "findings",
    "risk_patterns",
})

_RAW_SQL_FUNCTIONS = frozenset({
    "avg", "cast", "coalesce", "count", "length", "lower", "max", "min",
    "nullif", "round", "sum", "upper",
})

_RAW_SQL_FORBIDDEN_KEYWORDS = frozenset({
    "alter", "analyze", "call", "cluster", "comment", "copy", "create",
    "deallocate", "delete", "discard", "do", "drop", "execute", "grant",
    "insert", "listen", "load", "lock", "merge", "notify", "prepare",
    "refresh", "reindex", "reset", "revoke", "set", "show", "truncate",
    "update", "vacuum",
})


# ── Input Validation ──────────────────────────────────────────────────

def validate_paths(paths: Any) -> list[str]:
    """Validate file paths to prevent traversal attacks.

    Rejects:
      - Non-list or non-tuple inputs
      - Non-string or empty values
      - Paths longer than MAX_PATH_LENGTH
      - Paths with directory traversal (\"..\")
      - Absolute paths or paths starting with ~
    """
    if not isinstance(paths, (list, tuple)):
        return []
    validated = []
    for p in (paths or []):
        if not isinstance(p, str) or not p.strip():
            continue
        if len(p) > MAX_PATH_LENGTH:
            continue
        if ".." in p.split("/"):
            continue
        if p.startswith("/") or p.startswith("~"):
            continue
        validated.append(p.strip())
    return validated[:MAX_READ_PATHS]


def validate_pattern(pattern: Any) -> str | None:
    """Validate search pattern. Returns None if invalid."""
    if not isinstance(pattern, str) or not pattern.strip():
        return None
    if len(pattern) > 200:
        return None
    return pattern.strip()


def _mask_sql_string_literals(query: str) -> str | None:
    """Replace single-quoted string contents while preserving positions."""
    chars = list(query)
    index = 0
    in_string = False
    while index < len(chars):
        if chars[index] != "'":
            index += 1
            continue
        if in_string and index + 1 < len(chars) and chars[index + 1] == "'":
            chars[index] = " "
            chars[index + 1] = " "
            index += 2
            continue
        in_string = not in_string
        chars[index] = " "
        index += 1
        while in_string and index < len(chars) and chars[index] != "'":
            chars[index] = " "
            index += 1
    if in_string:
        return None
    return "".join(chars)


def raw_sql_source(query: str) -> tuple[str, str] | None:
    """Return the validated source table and effective alias for raw SQL."""
    import re

    masked = _mask_sql_string_literals(query.strip().rstrip(";"))
    if masked is None:
        return None
    match = re.search(
        r"\bFROM\s+(?P<table>[a-z_][a-z0-9_]*)"
        r"(?:\s+(?:AS\s+)?(?P<alias>[a-z_][a-z0-9_]*))?"
        r"(?=\s*(?:WHERE\b|GROUP\s+BY\b|ORDER\s+BY\b|HAVING\b|"
        r"LIMIT\b|OFFSET\b|$))",
        masked,
        re.IGNORECASE,
    )
    if match is None:
        return None
    table = match.group("table").lower()
    if table not in RAW_SQL_TABLES:
        return None
    return table, (match.group("alias") or table)


def validate_sql_query(query: Any) -> str | None:
    """Validate a project-scoped, single-table read-only SELECT.

    CTEs, joins, subqueries, set operations, comments, quoted identifiers,
    and arbitrary PostgreSQL function calls are rejected.
    """
    import re

    if not isinstance(query, str) or not query.strip():
        return None
    if len(query) > 5000:
        return None
    stripped = query.strip()
    if "--" in stripped or "/*" in stripped or "*/" in stripped:
        return None
    if '"' in stripped or "$$" in stripped:
        return None
    if ";" in stripped.rstrip(";"):
        return None

    masked = _mask_sql_string_literals(stripped.rstrip(";"))
    if masked is None or not re.match(r"^\s*SELECT\b", masked, re.IGNORECASE):
        return None
    words = [
        word.lower()
        for word in re.findall(r"\b[a-z_][a-z0-9_]*\b", masked, re.IGNORECASE)
    ]
    if words.count("select") != 1:
        return None
    if any(word in _RAW_SQL_FORBIDDEN_KEYWORDS for word in words):
        return None
    if any(word in words for word in ("with", "join", "union", "intersect", "except")):
        return None
    if raw_sql_source(stripped) is None:
        return None

    for function in re.findall(r"\b([a-z_][a-z0-9_]*)\s*\(", masked, re.IGNORECASE):
        if function.lower() not in _RAW_SQL_FUNCTIONS and function.lower() not in {"in", "not"}:
            return None
    return stripped

# ── Tool Definitions ──────────────────────────────────────────────────

TOOLS = [
    Tool(
        name="audit_overview",
        description=(
            "Get a high-level overview of the project: languages used, "
            "total files, total signatures, and file extension breakdown. "
            "Start here before making other queries."
        ),
        inputSchema={"type": "object", "properties": {}},
    ),
    Tool(
        name="audit_search",
        description=(
            "Search source code across all files in the audit database. "
            "Returns matching file paths, line numbers, and line context. "
            "Supports case-insensitive substring search and regex."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Search pattern (case-insensitive substring match, or regex)",
                },
                "is_regex": {
                    "type": "boolean",
                    "default": False,
                },
                "max_results": {
                    "type": "integer",
                    "default": 50,
                    "maximum": 200,
                    "minimum": 1,
                },
            },
            "required": ["pattern"],
        },
    ),
    Tool(
        name="audit_read_file",
        description=(
            "Read indexed source for one or more project-relative paths. "
            "Optional line bounds and a total character cap keep retrieval token-efficient."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                },
                "start_line": {"type": "integer", "minimum": 1, "default": 1},
                "end_line": {"type": "integer", "minimum": 1},
                "max_chars": {
                    "type": "integer",
                    "minimum": 1000,
                    "maximum": 100000,
                    "default": 100000,
                },
            },
            "required": ["paths"],
        },
    ),
    Tool(
        name="audit_dependents",
        description=(
            "Find what depends on a file, or what a file depends on. "
            "Uses the pre-computed dependency graph. "
            "incoming = blast radius, outgoing = imports."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "file_path": {"type": "string"},
                "direction": {
                    "type": "string",
                    "enum": ["incoming", "outgoing"],
                    "default": "incoming",
                },
            },
            "required": ["file_path"],
        },
    ),
    Tool(
        name="audit_brief",
        description=(
            "Return compact project-wide risk leads and open AI findings. "
            "This never narrows the audit to recently changed files and never "
            "treats automated patterns as conclusions."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "max_leads": {
                    "type": "integer",
                    "default": 50,
                    "maximum": 200,
                    "minimum": 1,
                },
            },
        },
    ),
    Tool(
        name="audit_finding",
        description=(
            "Persist an AI-reviewed audit conclusion against the current file hash. "
            "The finding is automatically purged if that source evidence changes."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "line_start": {"type": "integer", "minimum": 1},
                "line_end": {"type": "integer", "minimum": 1},
                "severity": {
                    "type": "string",
                    "enum": ["info", "low", "medium", "high", "critical"],
                },
                "category": {"type": "string"},
                "title": {"type": "string"},
                "description": {"type": "string"},
            },
            "required": [
                "path",
                "line_start",
                "severity",
                "category",
                "title",
                "description",
            ],
        },
    ),
    Tool(
        name="audit_raw_sql",
        description=(
            "Run a constrained READ-ONLY SELECT against one project-owned audit table. "
            "The server always enforces the current project. CTEs, joins, subqueries, "
            "set operations, and mutating SQL are rejected."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "query": {"type": "string"},
            },
            "required": ["query"],
        },
    ),
    Tool(
        name="audit_file_tree",
        description=(
            "Show the project directory tree structure. "
            "Returns an indented tree of all files. "
            "Useful for understanding project layout at a glance. "
            "Configurable depth to control how deep the tree goes."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "max_depth": {
                    "type": "integer",
                    "default": 3,
                    "minimum": 1,
                    "maximum": 10,
                    "description": "Maximum depth of the directory tree (1=top level only)",
                },
            },
        },
    ),
]


__all__ = [
    "TOOLS",
    "validate_paths",
    "validate_pattern",
    "validate_sql_query",
    "MAX_SEARCH_RESULTS",
    "MAX_SQL_ROWS",
    "MAX_READ_PATHS",
    "RAW_SQL_TABLES",
    "raw_sql_source",
]
