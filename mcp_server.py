#!/usr/bin/env python3
"""
audit-db MCP Server

Provides code audit intelligence tools via the Model Context Protocol.
Import projects, generate briefings, and query audit state — all through
a persistent PostgreSQL-backed server.

Usage:
    # Install dependencies
    pip install -r requirements.txt

    # Run the MCP server (stdio mode for Claude Desktop / MCP clients)
    python3 mcp_server.py

    # Or via npx/uvx if installed as a package:
    npx -y audit-db-mcp
    uvx audit-db-mcp

Configuration:
    Set these environment variables, or pass a connection string on first use:
      PGHOST, PGPORT (default: 5432), PGDATABASE, PGUSER, PGPASSWORD

    Or use a connection string:
      postgresql://user:pass@host:5432/dbname
"""

import os
import sys
import logging
from typing import Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from db import get_connection, ConnectionPool, schema_exists, get_project_names, get_changed_files
from core import import_project
from brief import assemble_briefing

try:
    from mcp.server import Server
    from mcp.server.stdio import stdio_server
    from mcp.types import Tool, TextContent
except ImportError:
    print("Error: mcp is required. Install with: pip install mcp")
    print("  Or install everything: pip install -r requirements.txt")
    sys.exit(1)

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("audit-db-mcp")

server = Server("audit-db")

# Global connection pool — lazily initialized on first tool call
_pool: Optional[ConnectionPool] = None
_pool_error: Optional[str] = None  # Stores connection error message if pool init fails
_schema_initialized = False


def _get_pool() -> Optional[ConnectionPool]:
    """Get or create the global connection pool.

    Returns None if database connection is not configured.
    The first connection error is cached to avoid repeated failures.
    """
    global _pool, _pool_error
    if _pool is not None:
        return _pool
    if _pool_error is not None:
        return None
    try:
        _pool = ConnectionPool(minconn=1, maxconn=5)
        return _pool
    except Exception as e:
        _pool_error = str(e)
        logger.warning(f"Could not initialize connection pool: {e}")
        logger.warning("Database tools will be unavailable until PG connection is configured.")
        return None


def _get_connection():
    """Get a connection from the pool, initializing schema if needed.

    Raises ValueError if the database connection is not configured.
    """
    global _schema_initialized
    pool = _get_pool()
    if pool is None:
        raise ValueError(
            "Database connection not configured. "
            "Set PGHOST, PGDATABASE, PGUSER, PGPASSWORD environment variables, "
            "or use the --connection flag."
        )
    conn = pool.getconn()
    if not _schema_initialized:
        _schema_initialized = schema_exists(conn)
    if not _schema_initialized:
        logger.info("Schema not found — initializing automatically")
        _init_schema(conn)
        _schema_initialized = True
    return conn


def _release_connection(conn):
    """Return a connection to the pool."""
    pool = _get_pool()
    pool.putconn(conn)


def _init_schema(conn):
    """Initialize the database schema (idempotent)."""
    schema_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "schema.sql")
    if not os.path.exists(schema_path):
        raise FileNotFoundError(f"schema.sql not found at {schema_path}")

    with open(schema_path, "r") as f:
        schema_sql = f.read()

    statements = [s.strip() for s in schema_sql.split(";") if s.strip()]
    cur = conn.cursor()
    for stmt in statements:
        try:
            cur.execute(stmt)
        except Exception as e:
            err_str = str(e).lower()
            if "already exists" not in err_str and "duplicate" not in err_str:
                logger.warning(f"Schema init warning: {e}")
    conn.commit()
    cur.close()
    logger.info("Database schema initialized successfully")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="audit_import",
            description=(
                "Import a codebase into the audit database. "
                "Extracts signatures, imports, and builds a dependency graph. "
                "Subsequent imports are incremental — only changed files are re-processed."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "project_path": {
                        "type": "string",
                        "description": "Absolute path to the project directory",
                    },
                    "project_name": {
                        "type": "string",
                        "description": "Project name (default: directory name)",
                    },
                    "language": {
                        "type": "string",
                        "enum": ["auto", "rust", "python", "typescript"],
                        "default": "auto",
                        "description": "Programming language (default: auto-detect)",
                    },
                },
                "required": ["project_path"],
            },
        ),
        Tool(
            name="audit_brief",
            description=(
                "Generate an audit briefing for an AI to analyze. "
                "Returns structured Markdown with ghost context (unchanged file signatures), "
                "target files (changed + blast radius), and historical findings."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "project_name": {
                        "type": "string",
                        "description": "Project name (default: most recently imported)",
                    },
                    "max_ghost_lines": {
                        "type": "integer",
                        "default": 500,
                        "description": "Maximum ghost context lines to include",
                    },
                },
            },
        ),
        Tool(
            name="audit_status",
            description=(
                "Show the status of a project in the audit database. "
                "Includes file counts, signatures, imports, dependency edges, "
                "changed files, and open findings."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "project_name": {
                        "type": "string",
                        "description": "Project name (default: most recently imported)",
                    },
                },
            },
        ),
        Tool(
            name="audit_changed",
            description=(
                "List files that have changed since the last audit import. "
                "These are the files that will be targets in the next briefing."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "project_name": {
                        "type": "string",
                        "description": "Project name (default: most recent)",
                    },
                },
            },
        ),
        Tool(
            name="audit_list",
            description="List all projects in the audit database.",
            inputSchema={
                "type": "object",
                "properties": {},
            },
        ),
        Tool(
            name="audit_init_db",
            description=(
                "Manually initialize the database schema. "
                "Usually not needed — schema is auto-created on first import. "
                "Use this if you want to pre-create the schema or verify it exists."
            ),
            inputSchema={
                "type": "object",
                "properties": {},
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    try:
        if name == "audit_import":
            return await _handle_import(arguments)
        elif name == "audit_brief":
            return await _handle_brief(arguments)
        elif name == "audit_status":
            return await _handle_status(arguments)
        elif name == "audit_changed":
            return await _handle_changed(arguments)
        elif name == "audit_list":
            return await _handle_list(arguments)
        elif name == "audit_init_db":
            return await _handle_init_db(arguments)
        else:
            return [TextContent(type="text", text=f"Unknown tool: {name}")]

    except Exception as e:
        logger.error(f"Tool error ({name}): {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {e}")]


async def _handle_import(args: dict) -> list[TextContent]:
    """Handle the audit_import tool."""
    project_root = os.path.abspath(args["project_path"])

    if not os.path.isdir(project_root):
        return [TextContent(type="text", text=f"Error: not a directory: {project_root}")]

    conn = None
    try:
        conn = _get_connection()
        result = import_project(
            project_path=project_root,
            project_name=args.get("project_name"),
            language=args.get("language", "auto"),
            connection_string=None,
        )
        return [TextContent(type="text", text=str(result))]
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"Import error: {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {e}")]
    finally:
        if conn:
            _release_connection(conn)


async def _handle_brief(args: dict) -> list[TextContent]:
    """Handle the audit_brief tool."""
    try:
        briefing = assemble_briefing(
            project_name=args.get("project_name"),
            connection_str=None,
            max_ghost_lines=args.get("max_ghost_lines", 500),
        )
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"Brief error: {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {e}")]

    if not briefing:
        return [TextContent(type="text", text="Error: could not generate briefing. Is the project imported?")]

    return [TextContent(type="text", text=briefing)]


async def _handle_status(args: dict) -> list[TextContent]:
    """Handle the audit_status tool."""
    conn = None
    try:
        conn = _get_connection()
        cur = conn.cursor()

        project_name = args.get("project_name")
        if project_name:
            cur.execute(
                "SELECT id, name, primary_language, repo_path FROM projects WHERE name = %s",
                (project_name,),
            )
        else:
            cur.execute(
                "SELECT id, name, primary_language, repo_path FROM projects ORDER BY created_at DESC LIMIT 1"
            )

        row = cur.fetchone()
        if not row:
            return [TextContent(type="text", text="No projects found.")]

        project_id, name, language, repo_path = row

        cur.execute(
            "SELECT COUNT(*), COALESCE(SUM(size), 0), "
            "COUNT(*) FILTER (WHERE signature_cache IS NOT NULL AND jsonb_array_length(signature_cache) > 0) "
            "FROM files WHERE project_id = %s",
            (project_id,),
        )
        file_count, total_size, with_sigs = cur.fetchone()

        total_sigs = 0
        if with_sigs:
            cur.execute(
                "SELECT SUM(jsonb_array_length(signature_cache)) "
                "FROM files WHERE project_id = %s AND signature_cache IS NOT NULL",
                (project_id,),
            )
            total_sigs = cur.fetchone()[0] or 0

        changed = get_changed_files(conn, project_id)

        cur.execute(
            "SELECT COUNT(*) FROM file_imports WHERE project_id = %s",
            (project_id,),
        )
        import_count = cur.fetchone()[0]

        cur.execute(
            "SELECT COUNT(*) FROM dependency_edges WHERE project_id = %s",
            (project_id,),
        )
        edge_count = cur.fetchone()[0]

        result = (
            f"Project: {name}\n"
            f"  ID: {project_id}\n"
            f"  Language: {language}\n"
            f"  Path: {repo_path}\n"
            f"  Files: {file_count}\n"
            f"  Total size: {total_size / 1024:.0f} KB\n"
            f"  Signatures: {total_sigs}\n"
            f"  Imports tracked: {import_count}\n"
            f"  Dependency edges: {edge_count}\n"
            f"  Changed files: {len(changed)}"
        )

        if changed:
            result += "\n  Changed files:"
            for _, fpath in changed[:10]:
                result += f"\n    - {fpath}"
            if len(changed) > 10:
                result += f"\n    ... and {len(changed) - 10} more"

        return [TextContent(type="text", text=result)]
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"Status error: {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {e}")]
    finally:
        if conn:
            _release_connection(conn)


async def _handle_changed(args: dict) -> list[TextContent]:
    """Handle the audit_changed tool."""
    conn = None
    try:
        conn = _get_connection()
        cur = conn.cursor()

        project_name = args.get("project_name")
        if project_name:
            cur.execute("SELECT id FROM projects WHERE name = %s", (project_name,))
        else:
            cur.execute("SELECT id FROM projects ORDER BY created_at DESC LIMIT 1")

        row = cur.fetchone()
        if not row:
            return [TextContent(type="text", text="No projects found.")]

        project_id = row[0]
        changed = get_changed_files(conn, project_id)

        if not changed:
            return [TextContent(type="text", text="No files changed since last audit.")]

        result = f"{len(changed)} changed file(s):\n"
        for _, fpath in changed:
            result += f"  {fpath}\n"

        return [TextContent(type="text", text=result)]
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"Changed error: {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {e}")]
    finally:
        if conn:
            _release_connection(conn)


async def _handle_list(args: dict) -> list[TextContent]:
    """Handle the audit_list tool."""
    conn = None
    try:
        conn = _get_connection()
        names = get_project_names(conn)

        if not names:
            return [TextContent(type="text", text="No projects found.")]

        result = "Projects in audit database:\n"
        for name in names:
            result += f"  - {name}\n"

        return [TextContent(type="text", text=result)]
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"List error: {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {e}")]
    finally:
        if conn:
            _release_connection(conn)


async def _handle_init_db(args: dict) -> list[TextContent]:
    """Handle the audit_init_db tool."""
    conn = None
    try:
        conn = _get_connection()
        if schema_exists(conn):
            return [TextContent(type="text", text="Schema already exists. No action needed.")]

        _init_schema(conn)
        return [TextContent(type="text", text="Database schema initialized successfully.")]
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"Init DB error: {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {e}")]
    finally:
        if conn:
            _release_connection(conn)


async def main():
    """Run the MCP server using stdio transport."""
    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream,
            write_stream,
            server.create_initialization_options(),
        )


if __name__ == "__main__":
    import asyncio
    asyncio.run(main())
