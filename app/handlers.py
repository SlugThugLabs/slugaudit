"""MCP tool handlers.

Each handler receives (conn, state, args) and returns list[TextContent].
Handlers are pure logic \u2014 no server setup, no sync, no validation.
That happens in mcp/server.py and mcp/tools.py.
"""

import asyncio
import hashlib
import json
import logging
import re
import regex
from pathlib import Path
from typing import Any

from repositories import FileRepository, FindingRepository, RiskPatternRepository
from app.tools import (
    validate_paths,
    validate_pattern,
    validate_sql_query,
    raw_sql_source,
    RAW_SQL_TABLES,
    MAX_READ_CHARS,
    MAX_SQL_ROWS,
    MAX_SEARCH_RESULTS,
)
from mcp.types import TextContent

logger = logging.getLogger("slugaudit-mcp.handlers")

# ── Path Tree Builder ─────────────────────────────────────────────────

def _build_path_tree(paths: list[str], max_depth: int = 3, max_entries: int = 20) -> list[str]:
    """Build a compact indented directory tree from flat file paths.
    Shows up to max_depth levels of nesting and max_entries items
    at each level to keep the output token-efficient.
    """
    tree: dict[str, dict[str, Any]] = {}
    for path in paths:
        parts = path.split("/")
        node = tree
        for part in parts:
            if part not in node:
                node[part] = {}
            node = node[part]
    lines: list[str] = []
    _render_tree(tree, lines, max_depth, max_entries)
    return lines


def _render_tree(
    node: dict[str, Any],
    lines: list[str],
    max_depth: int,
    max_entries: int,
    depth: int = 0,
) -> None:
    """Recursively render tree nodes as indented text."""
    items = sorted(node.items())
    if len(items) > max_entries:
        items = items[:max_entries]
        items.append(("...", {}))
    for name, subtree in items:
        indent = "  " * depth
        if not subtree:
            lines.append(f"{indent}{name}")
        elif depth >= max_depth:
            lines.append(f"{indent}{name}/ ({len(subtree)} items)")
        else:
            lines.append(f"{indent}{name}/")
            _render_tree(subtree, lines, max_depth, max_entries, depth + 1)


# ── Handler: audit_overview ───────────────────────────────────────────

async def handle_overview(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Return a high-level project overview with directory tree."""
    file_repo = FileRepository(conn)
    stats = await asyncio.to_thread(file_repo.get_file_stats, state.project_id)

    all_paths = await asyncio.to_thread(file_repo.get_all_paths_ordered, state.project_id)

    # Extension breakdown
    extensions: dict[str, int] = {}
    for path in all_paths:
        ext = Path(path).suffix or "(no ext)"
        extensions[ext] = extensions.get(ext, 0) + 1
    ext_summary = ", ".join(
        f"{ext} ({count})" for ext, count in
        sorted(extensions.items(), key=lambda x: -x[1])[:10]
    )

    # Directory tree
    tree_lines = _build_path_tree(all_paths, max_depth=3, max_entries=50)
    tree_text = "\n".join(tree_lines)

    text = (
        f"## Project: {state.project_name}\n\n"
        f"**Files:** {stats['total_files']}  "
        f"**Size:** {stats['total_bytes'] / 1024:.0f} KB  "
        f"**Signatures:** {stats['total_sigs']}  "
        f"**Language:** {state.language}\n\n"
        f"**Extensions:** {ext_summary}\n\n"
        f"**Directory tree:**\n{tree_text}\n\n"
        f"**Last synced:** {state.last_synced_at}"
    )
    return [TextContent(type="text", text=text)]


# ── Handler: audit_search ─────────────────────────────────────────────

async def handle_search(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Search source code across all files."""
    pattern = validate_pattern(args.get("pattern"))
    if pattern is None:
        return [TextContent(type="text", text="Invalid or missing search pattern.")]

    is_regex = bool(args.get("is_regex", False))
    try:
        max_results = min(
            max(int(args.get("max_results", 50)), 1), MAX_SEARCH_RESULTS
        )
    except (TypeError, ValueError):
        return [TextContent(type="text", text="max_results must be an integer.")]

    # ReDoS protection: limit regex length and reject nested quantifiers
    if is_regex and len(pattern) > 100:
        return [TextContent(type="text", text="Regex pattern too long (max 100 chars).")]
    if is_regex:
        # Simple check for catastrophic backtracking patterns like (a+)+b, (a|b)*c
        dangerous = re.search(r'\([^)]+[+*]\)[+*]', pattern)
        if dangerous:
            return [TextContent(
                type="text",
                text="Pattern is too complex — use substring search or simplify the regex."
            )]
        try:
            compiled_pattern = regex.compile(pattern, regex.IGNORECASE)
        except regex.error as error:
            return [TextContent(type="text", text=f"Invalid regex: {error}")]
    else:
        compiled_pattern = None

    file_repo = FileRepository(conn)
    rows = await asyncio.to_thread(
        file_repo.search_by_pattern,
        state.project_id, pattern, is_regex, max_results,
    )

    if not rows:
        return [TextContent(type="text", text=f"No matches for '{pattern}'.")]

    results = []
    regex_error_count = 0
    for path, content in rows:
        if not content:
            continue
        lines = content.split("\n")
        if is_regex:
            matching = []
            for i, line in enumerate(lines, 1):
                try:
                    if compiled_pattern is not None and compiled_pattern.search(
                        line, timeout=0.05
                    ):
                        matching.append(
                            f"  {i}: {line.strip()[:200]}"
                        )
                except (regex.error, TimeoutError):
                    regex_error_count += 1
        else:
            matching = [
                f"  {i}: {line.strip()[:200]}"
                for i, line in enumerate(lines, 1)
                if pattern.lower() in line.lower()
            ]
        if matching:
            results.append(
                f"**{path}** ({len(matching)} matches):\n"
                + "\n".join(matching[:5])
                + ("\n  ..." if len(matching) > 5 else "")
            )

    if not results:
        msg = f"Pattern found in {len(rows)} files but no matching lines (content is in DB)."
        if regex_error_count:
            msg += f" {regex_error_count} line(s) had regex errors."
        return [TextContent(type="text", text=msg)]

    text = "\n\n".join(results[:20])
    if len(rows) > 20:
        text += f"\n\n... and {len(rows) - 20} more files"
    if regex_error_count:
        text += f"\n({regex_error_count} line(s) had regex errors)"
    return [TextContent(type="text", text=text)]


# ── Handler: audit_read_file ──────────────────────────────────────────

async def handle_read_file(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Read bounded source ranges from the verified database revision."""
    raw_paths = args.get("paths", [])
    if not isinstance(raw_paths, list):
        return [TextContent(type="text", text="'paths' must be an array.")]

    paths = validate_paths(raw_paths)
    if not paths:
        return [TextContent(type="text", text="No valid paths provided.")]

    try:
        start_line = max(int(args.get("start_line", 1)), 1)
        raw_end_line = args.get("end_line")
        end_line = int(raw_end_line) if raw_end_line is not None else None
        max_chars = min(
            max(int(args.get("max_chars", MAX_READ_CHARS)), 1000),
            MAX_READ_CHARS,
        )
    except (TypeError, ValueError):
        return [TextContent(type="text", text="Invalid line or character bounds.")]
    if end_line is not None and end_line < start_line:
        return [TextContent(type="text", text="end_line must be at or after start_line.")]

    file_repo = FileRepository(conn)
    rows = await asyncio.to_thread(
        file_repo.get_file_contents, state.project_id, paths,
    )

    if not rows:
        return [TextContent(type="text", text=f"Files not found: {paths}")]

    parts = []
    remaining = max_chars
    truncated = False
    found = {r[0] for r in rows}
    for path, content in rows:
        lines = (content or "").splitlines()
        selected = lines[start_line - 1:end_line]
        rendered = "\n".join(
            f"{number}: {line}"
            for number, line in enumerate(selected, start=start_line)
        )
        if len(rendered) > remaining:
            rendered = rendered[:remaining]
            truncated = True
        parts.append(
            f"// \u2500\u2500 {path}:{start_line}-{end_line or len(lines)} \u2500\u2500 //\n"
            f"{rendered or '(empty range)'}"
        )
        remaining -= len(rendered)
        if remaining <= 0:
            truncated = True
            break

    missing = [p for p in paths if p not in found]
    if missing:
        parts.append(f"Not found: {', '.join(missing)}")
    if truncated:
        parts.append(f"Output truncated at {max_chars} characters.")

    return [TextContent(type="text", text="\n\n".join(parts))]


# ── Handler: audit_dependents ─────────────────────────────────────────

async def handle_dependents(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Analyse import dependency graph."""
    file_path = args.get("file_path", "")
    if not isinstance(file_path, str) or not file_path.strip():
        return [TextContent(type="text", text="'file_path' is required.")]

    direction = args.get("direction", "incoming")
    if direction not in ("incoming", "outgoing"):
        return [TextContent(
            type="text",
            text="direction must be 'incoming' or 'outgoing'."
        )]

    file_repo = FileRepository(conn)
    paths = await asyncio.to_thread(
        file_repo.get_dependents, state.project_id, file_path, direction,
    )

    label = (
        f"Files that depend on `{file_path}`"
        if direction == "incoming"
        else f"Files imported by `{file_path}`"
    )

    if not paths:
        return [TextContent(
            type="text",
            text=f"No dependencies for `{file_path}`."
        )]

    result = f"## {label}\n\n{len(paths)} file(s):\n"
    for path in paths:
        result += f"  - {path}\n"
    return [TextContent(type="text", text=result)]


# ── Handler: audit_brief ──────────────────────────────────────────────

async def handle_brief(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Return compact global leads without biasing the AI toward changed files."""
    max_leads = min(max(int(args.get("max_leads", 50)), 1), 200)
    file_repo = FileRepository(conn)
    risk_repo = RiskPatternRepository(conn)
    finding_repo = FindingRepository(conn)
    # A psycopg2 connection may move to a worker thread, but it cannot be used
    # concurrently by multiple workers.
    stats = await asyncio.to_thread(file_repo.get_file_stats, state.project_id)
    risk_files = await asyncio.to_thread(
        risk_repo.get_project_patterns, state.project_id
    )
    findings = await asyncio.to_thread(
        finding_repo.get_open_findings, state.project_id, max_leads
    )
    risk_files = sorted(
        risk_files,
        key=lambda item: sum(count for _, count in item[1]),
        reverse=True,
    )[:max_leads]
    payload = {
        "project": state.project_name,
        "scope": "complete_index",
        "stats": stats,
        "risk_leads": [
            {
                "path": path,
                "patterns": [
                    {"type": pattern_type, "count": count}
                    for pattern_type, count in patterns
                ],
            }
            for path, patterns in risk_files
        ],
        "open_findings": [
            {
                "path": path,
                "line_start": line_start,
                "line_end": line_end,
                "severity": severity,
                "category": category,
                "message": message,
            }
            for path, line_start, line_end, severity, category, message in findings
        ],
        "truncated": len(risk_files) >= max_leads or len(findings) >= max_leads,
        "guidance": "Risk patterns are search leads, not audit conclusions.",
    }
    return [TextContent(type="text", text=json.dumps(payload, separators=(",", ":")))]


async def handle_finding(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Persist one AI-reviewed conclusion against the current file evidence."""
    paths = validate_paths([args.get("path")])
    if len(paths) != 1:
        return [TextContent(type="text", text="A valid relative 'path' is required.")]
    path = paths[0]
    severity = args.get("severity")
    if severity not in {"info", "low", "medium", "high", "critical"}:
        return [TextContent(type="text", text="Invalid finding severity.")]
    category = args.get("category")
    title = args.get("title")
    description = args.get("description")
    if (
        not isinstance(category, str)
        or not category.strip()
        or not isinstance(title, str)
        or not title.strip()
        or not isinstance(description, str)
        or not description.strip()
    ):
        return [TextContent(
            type="text",
            text="Finding category, title, and description are required.",
        )]
    line_start = args.get("line_start")
    line_end = args.get("line_end", line_start)
    if not isinstance(line_start, int) or line_start < 1:
        return [TextContent(type="text", text="line_start must be a positive integer.")]
    if not isinstance(line_end, int) or line_end < line_start:
        return [TextContent(type="text", text="line_end must be at or after line_start.")]

    file_repo = FileRepository(conn)
    identity = await asyncio.to_thread(
        file_repo.get_file_identity, state.project_id, path
    )
    if identity is None:
        return [TextContent(type="text", text=f"Indexed file not found: {path}")]
    file_id, file_hash = identity
    identity_text = "\0".join(
        (path, str(line_start), str(line_end), category.strip(), title.strip(), file_hash)
    )
    identity_hash = hashlib.sha256(identity_text.encode("utf-8")).hexdigest()
    message = f"{title.strip()}: {description.strip()}"
    finding_id, created = await asyncio.to_thread(
        FindingRepository(conn).record,
        project_id=state.project_id,
        file_id=file_id,
        identity_hash=identity_hash,
        severity=severity,
        category=category.strip(),
        message=message,
        line_start=line_start,
        line_end=line_end,
    )
    payload = {
        "finding_id": str(finding_id),
        "created": created,
        "path": path,
        "line_start": line_start,
        "line_end": line_end,
        "severity": severity,
        "category": category.strip(),
        "evidence_revision": state.revision_id,
    }
    return [TextContent(type="text", text=json.dumps(payload, separators=(",", ":")))]


# ── Project-ID scoping for raw SQL ───────────────────────────────────

# Tables in the audit schema that have a project_id foreign key.
# Queries referencing these are auto-scoped to the current project.
_SCOPED_TABLES = RAW_SQL_TABLES


def _scope_sql_query(query: str, project_id: str) -> str:
    """Force a qualified project predicate into validated single-table SQL."""
    q = query.strip().rstrip(";")
    if validate_sql_query(q) is None:
        raise ValueError("Raw SQL is outside the constrained SELECT grammar")
    source = raw_sql_source(q)
    if source is None:
        raise ValueError("Raw SQL must select from one project-owned table")
    _, alias = source
    project_filter = f"{alias}.project_id = %s"
    _ = project_id  # Bound by the caller; never interpolate project IDs.

    # Walk through the query tracking paren depth and string literals
    # to find the correct insertion point for the project_id filter.
    depth = 0
    in_string = False
    quote_char = None
    i = 0

    where_pos = -1          # Position of WHERE keyword
    clause_pos = len(q)     # Position of next clause after FROM

    while i < len(q):
        c = q[i]

        # Skip string literals
        if in_string:
            if c == "\\" and i + 1 < len(q):
                i += 2
                continue
            if c == quote_char:
                in_string = False
            i += 1
            continue

        if c in ("'", '"', "`"):
            in_string = True
            quote_char = c
            i += 1
            continue

        # Track paren depth to avoid subqueries
        if c == "(":
            depth += 1
            i += 1
            continue
        if c == ")":
            depth -= 1
            i += 1
            continue

        # Only look for keywords at the outermost depth
        if depth == 0:
            remaining_upper = q[i:].upper()

            # Check for WHERE (word boundary check)
            if where_pos < 0 and remaining_upper.startswith("WHERE"):
                next_char = remaining_upper[5:6] if len(remaining_upper) > 5 else ""
                if not (next_char and (next_char.isalnum() or next_char == "_")):
                    where_pos = i
                    i += 5
                    continue

            if where_pos < 0:
                # Check for clauses that should come after WHERE
                for kw in ("GROUP BY", "ORDER BY", "HAVING", "LIMIT", "OFFSET", "UNION"):
                    if remaining_upper.startswith(kw):
                        next_char = (
                            remaining_upper[len(kw):len(kw) + 1]
                            if len(remaining_upper) > len(kw)
                            else ""
                        )
                        if not (next_char and (next_char.isalnum() or next_char == "_")):
                            clause_pos = i
                            i += len(kw)
                            break
                else:
                    i += 1
                    continue
                continue

        i += 1

    # Build the scoped query with a %%s placeholder for project_id
    if where_pos >= 0:
        # Insert AND project_id = %%s after WHERE
        where_end = where_pos + 5  # "WHERE"
        while where_end < len(q) and q[where_end] in " \t\n\r":
            where_end += 1
        scoped = q[:where_end] + project_filter + " AND " + q[where_end:]
    else:
        # Insert WHERE project_id = %%s before next clause or at end
        if clause_pos < len(q):
            scoped = q[:clause_pos] + f" WHERE {project_filter} " + q[clause_pos:]
        else:
            scoped = q + f" WHERE {project_filter}"

    return scoped


# ── Handler: audit_raw_sql ────────────────────────────────────────────

async def handle_raw_sql(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Run constrained SQL in a database-enforced read-only transaction."""
    query = validate_sql_query(args.get("query", ""))
    if query is None:
        return [TextContent(
            type="text",
            text=("Only a single-table SELECT over a project-owned audit table is allowed. "
                  "CTEs, joins, subqueries, comments, and mutating SQL are rejected.")
        )]

    scoped = _scope_sql_query(query, state.project_id)

    cur = conn.cursor()
    try:
        # Sync completes before handlers run. End any implicit read transaction,
        # then make PostgreSQL itself reject writes and long-running queries.
        conn.commit()
        cur.execute("BEGIN READ ONLY")
        cur.execute("SET LOCAL statement_timeout = '3000ms'")
        cur.execute(scoped, (state.project_id,))
        col_names = [desc[0] for desc in cur.description] if cur.description else []
        rows = cur.fetchmany(MAX_SQL_ROWS)
    except Exception as e:
        conn.rollback()
        return [TextContent(type="text", text=f"Query error: {e}")]
    finally:
        cur.close()
    conn.rollback()

    if not rows:
        return [TextContent(type="text", text="No results.")]

    parts = [" | ".join(col_names)]
    parts.append("-|-" * len(col_names))
    for row in rows[:50]:
        parts.append(
            " | ".join(str(v)[:100] if v is not None else "" for v in row)
        )
    result = "\n".join(parts)
    scope_note = f"\n\n_Scoped to project `{state.project_name}`._"
    if len(rows) > 50:
        result += f"\n\n... and {len(rows) - 50} more rows"
    result += scope_note
    return [TextContent(type="text", text=result)]


# ── Handler: audit_file_tree ───────────────────────────────────────────

async def handle_file_tree(conn: Any, state: Any, args: dict[str, Any]) -> list[TextContent]:
    """Return the project directory tree with configurable depth."""
    try:
        max_depth = min(max(int(args.get("max_depth", 3)), 1), 10)
    except (TypeError, ValueError):
        return [TextContent(type="text", text="max_depth must be an integer.")]
    max_entries = 50  # Show up to 50 items per level

    file_repo = FileRepository(conn)
    all_paths = await asyncio.to_thread(file_repo.get_all_paths_ordered, state.project_id)
    if not all_paths:
        return [TextContent(type="text", text="No files in project.")]

    tree_text = "\n".join(
        _build_path_tree(all_paths, max_depth=max_depth, max_entries=max_entries)
    )

    text = (
        f"## Directory Tree: {state.project_name}\n"
        f"(depth={max_depth}, {len(all_paths)} files)\n\n"
        f"{tree_text}"
    )
    return [TextContent(type="text", text=text)]


# ── Handler Dispatch ──────────────────────────────────────────────────

HANDLERS = {
    "audit_overview": handle_overview,
    "audit_search": handle_search,
    "audit_read_file": handle_read_file,
    "audit_dependents": handle_dependents,
    "audit_brief": handle_brief,
    "audit_finding": handle_finding,
    "audit_raw_sql": handle_raw_sql,
    "audit_file_tree": handle_file_tree,
}


__all__ = ["HANDLERS", "handle_overview", "handle_search", "handle_read_file",
           "handle_dependents", "handle_brief", "handle_finding", "handle_raw_sql",
           "handle_file_tree"]
