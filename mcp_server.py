#!/usr/bin/env python3
"""
slugaudit-mcp MCP Server

Provides code audit intelligence tools via the Model Context Protocol.
Import projects, generate briefings, and query audit state — all through
a persistent PostgreSQL-backed server.

Usage:
    # Install dependencies
    pip install -r requirements.txt

    # Run the MCP server (stdio mode for Claude Desktop / MCP clients)
    python3 mcp_server.py

    # Or via npx/uvx if installed as a package:
    npx -y slugaudit-mcp
    uvx slugaudit-mcp

Configuration:
    Set these environment variables, or pass a connection string on first use:
      PGHOST, PGPORT (default: 5432), PGDATABASE, PGUSER, PGPASSWORD

    Or use a connection string:
      postgresql://user:pass@host:5432/dbname
"""

import os
import sys
import asyncio
import logging
import threading
from typing import Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from infrastructure import (
    get_connection,
    ConnectionPool,
    validate_project_path,
)
from services import SchemaService, ImportService
from services.import_service import import_project
from briefing import assemble_briefing
from repositories import ProjectRepository

try:
    from mcp.server import Server
    from mcp.server.stdio import stdio_server
    from mcp.types import Tool, TextContent
except ImportError:
    print("Error: mcp is required. Install with: pip install -r requirements.txt")
    sys.exit(1)

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("slugaudit-mcp")

server = Server("slugaudit-mcp")

# Global connection pool — lazily initialized on first tool call
_pool: Optional[ConnectionPool] = None
_pool_error: Optional[str] = None  # Stores connection error message if pool init fails
_schema_initialized = False
_pool_lock = threading.Lock()  # Protects _pool, _pool_error, and _schema_initialized

_schema_service = SchemaService()


def _get_pool() -> Optional[ConnectionPool]:
    """Get or create the global connection pool.

    Returns None if database connection is not configured.
    The first connection error is cached to avoid repeated failures.
    Thread-safe via _pool_lock.
    """
    global _pool, _pool_error
    with _pool_lock:
        if _pool is not None:
            return _pool
        if _pool_error is not None:
            return None
        try:
            _pool = ConnectionPool(minconn=1, maxconn=5)
            return _pool
        except Exception as e:
            # Security: don't log full exception which may contain credentials
            _pool_error = "Could not initialize connection pool (check PG connection settings)"
            logger.warning("Could not initialize connection pool")
            logger.warning("Database tools will be unavailable until PG connection is configured.")
            return None


def _check_connection_health(conn) -> bool:
    """Check if a database connection is still alive and usable.

    Returns True if the connection is healthy, False if it is closed or broken.
    """
    try:
        # psycopg2 connections have a closed property: 0=open, 1=closed, 2="bad"
        if getattr(conn, "closed", True):
            return False
        # Attempt a lightweight ping by executing a simple query
        cur = conn.cursor()
        cur.execute("SELECT 1")
        cur.fetchone()
        cur.close()
        return True
    except Exception:
        return False


async def _get_connection():
    """Get a connection from the pool, initializing schema if needed.

    Raises ValueError if the database connection is not configured.
    Runs synchronous DB operations in a thread to avoid blocking the event loop.
    """
    global _schema_initialized
    pool = _get_pool()
    if pool is None:
        raise ValueError(
            "Database connection not configured. "
            "Set PGHOST, PGDATABASE, PGUSER, PGPASSWORD environment variables, "
            "or use the --connection flag."
        )

    # Run getconn in a thread since it's synchronous
    conn = await asyncio.to_thread(pool.getconn)

    # Check connection health — if broken, discard and raise
    if not await asyncio.to_thread(_check_connection_health, conn):
        try:
            await asyncio.to_thread(conn.close)
        except Exception:
            pass
        # Try to get another connection from the pool
        try:
            conn = await asyncio.to_thread(pool.getconn)
        except Exception:
            pass
        if not await asyncio.to_thread(_check_connection_health, conn):
            raise RuntimeError("Could not obtain a healthy database connection")

    # Check and initialize schema if needed (threaded)
    with _pool_lock:
        if not _schema_initialized:
            exists = await asyncio.to_thread(
                ProjectRepository.schema_exists,
                ProjectRepository(conn),
            )
            if not exists:
                logger.info("Schema not found — initializing automatically")
                await asyncio.to_thread(_schema_service.initialize, conn, logger)
            _schema_initialized = True

    return conn


async def _release_connection(conn):
    """Return a connection to the pool.

    Checks connection health first — broken connections are closed and
    discarded rather than returned to the pool. Runs in a thread to avoid
    blocking the event loop.
    """
    pool = _get_pool()
    if pool is None:
        return

    # Only return healthy connections to the pool
    if await asyncio.to_thread(_check_connection_health, conn):
        await asyncio.to_thread(pool.putconn, conn)
    else:
        logger.warning("Discarding unhealthy connection from pool")
        try:
            await asyncio.to_thread(conn.close)
        except Exception:
            pass


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
                        "enum": [
                            "auto", "rust", "python", "typescript",
                            "go", "java", "c", "cpp", "ruby",
                        ],
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
        # Security: return generic error message to client
        return [TextContent(type="text", text="An internal error occurred")]


async def _handle_import(args: dict) -> list[TextContent]:
    """Handle the audit_import tool."""
    # Security: validate project path
    try:
        project_root = validate_project_path(args["project_path"])
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}")]

    if not os.path.isdir(project_root):
        return [TextContent(type="text", text=f"Error: not a directory: {project_root}")]

    conn = None
    try:
        conn = await _get_connection()
        # Run the synchronous import_project in a thread
        result = await asyncio.to_thread(
            import_project,
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
        return [TextContent(type="text", text="An internal error occurred during import")]
    finally:
        if conn:
            await _release_connection(conn)


async def _handle_brief(args: dict) -> list[TextContent]:
    """Handle the audit_brief tool."""
    try:
        # Run the synchronous assemble_briefing in a thread
        briefing = await asyncio.to_thread(
            assemble_briefing,
            project_name=args.get("project_name"),
            connection_str=None,
            max_ghost_lines=args.get("max_ghost_lines", 500),
        )
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"Brief error: {e}", exc_info=True)
        return [TextContent(type="text", text="An internal error occurred during briefing generation")]

    if not briefing:
        return [TextContent(type="text", text="Error: could not generate briefing. Is the project imported?")]

    return [TextContent(type="text", text=briefing)]


async def _handle_status(args: dict) -> list[TextContent]:
    """Handle the audit_status tool."""
    conn = None
    cur = None
    try:
        conn = await _get_connection()
        project_repo = ProjectRepository(conn)

        project_name = args.get("project_name")
        if project_name:
            row = project_repo.get_by_name(project_name)
        else:
            row = project_repo.get_latest()

        if not row:
            return [TextContent(type="text", text="No projects found.")]

        project_id, name, language, repo_path = row

        stats = project_repo.get_status(project_id)
        # Use FileRepository for changed files
        from repositories import FileRepository
        file_repo = FileRepository(conn)
        changed = await asyncio.to_thread(file_repo.get_changed, project_id)

        result = (
            f"Project: {name}\n"
            f"  ID: {project_id}\n"
            f"  Language: {language}\n"
            f"  Path: {repo_path}\n"
            f"  Files: {stats['file_count']}\n"
            f"  Total size: {stats['total_size'] / 1024:.0f} KB\n"
            f"  Signatures: {stats['signatures_count']}\n"
            f"  Imports tracked: {stats['imports_count']}\n"
            f"  Dependency edges: {stats['edge_count']}\n"
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
        return [TextContent(type="text", text="An internal error occurred")]
    finally:
        if conn:
            await _release_connection(conn)


async def _handle_changed(args: dict) -> list[TextContent]:
    """Handle the audit_changed tool."""
    conn = None
    try:
        conn = await _get_connection()
        project_repo = ProjectRepository(conn)
        from repositories import FileRepository
        file_repo = FileRepository(conn)

        project_name = args.get("project_name")
        if project_name:
            row = project_repo.get_by_name(project_name)
        else:
            row = project_repo.get_latest()

        if not row:
            return [TextContent(type="text", text="No projects found.")]

        project_id = row[0]
        changed = await asyncio.to_thread(file_repo.get_changed, project_id)

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
        return [TextContent(type="text", text="An internal error occurred")]
    finally:
        if conn:
            await _release_connection(conn)


async def _handle_list(args: dict) -> list[TextContent]:
    """Handle the audit_list tool."""
    conn = None
    try:
        conn = await _get_connection()
        project_repo = ProjectRepository(conn)
        names = project_repo.get_names()

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
        return [TextContent(type="text", text="An internal error occurred")]
    finally:
        if conn:
            await _release_connection(conn)


async def _handle_init_db(args: dict) -> list[TextContent]:
    """Handle the audit_init_db tool."""
    conn = None
    try:
        conn = await _get_connection()
        project_repo = ProjectRepository(conn)
        exists = project_repo.schema_exists()
        if exists:
            return [TextContent(type="text", text="Schema already exists. No action needed.")]

        await asyncio.to_thread(_schema_service.initialize, conn, logger)
        return [TextContent(type="text", text="Database schema initialized successfully.")]
    except ValueError as e:
        return [TextContent(type="text", text=f"Error: {e}\n\nSet PostgreSQL env vars (PGHOST, PGDATABASE, PGUSER, PGPASSWORD) and restart the server.")]
    except Exception as e:
        logger.error(f"Init DB error: {e}", exc_info=True)
        return [TextContent(type="text", text="An internal error occurred")]
    finally:
        if conn:
            await _release_connection(conn)


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
