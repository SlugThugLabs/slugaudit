"""MCP server setup, tool routing, and runner.

Binds together: input validation (mcp/tools.py), auto-sync (mcp/sync.py),
connection pool (mcp/pool.py), and handlers (mcp/handlers.py).
"""

import logging
import os
import json

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent

from app.tools import TOOLS
from app.handlers import HANDLERS
from app.sync import synchronized_project
from app.pool import get_connection as get_db, init_pool
from typing import Any

from app.config import load_config
from app.instructions import MCP_INSTRUCTIONS

logger = logging.getLogger("slugaudit-mcp.server")

SERVER = Server("slugaudit-mcp", instructions=MCP_INSTRUCTIONS)


def _freshness_content(state: Any) -> TextContent:
    """Build the machine-readable evidence revision contract."""
    metadata = {
        "contract_version": state.contract_version,
        "schema_version": state.schema_version,
        "project_id": state.project_id,
        "revision_id": state.revision_id,
        "manifest_hash": state.manifest_hash,
        "synced_at": state.last_synced_at,
        "freshness": "verified",
    }
    missing = [key for key, value in metadata.items() if key != "freshness" and not value]
    if missing:
        raise RuntimeError(
            "Sync did not return required freshness metadata: " + ", ".join(missing)
        )
    return TextContent(
        type="text",
        text=json.dumps({"slugaudit_meta": metadata}, separators=(",", ":")),
    )


@SERVER.list_tools()  # type: ignore[no-untyped-call,untyped-decorator]
async def list_tools() -> list[Any]:
    return TOOLS


@SERVER.call_tool()  # type: ignore[untyped-decorator]
async def call_tool(name: str, arguments: dict[str, Any]) -> list[TextContent]:
    cwd = os.getcwd()

    try:
        # Single connection shared between sync and handler
        async with get_db() as conn:
            async with synchronized_project(cwd, conn) as state:
                handler = HANDLERS.get(name)
                if handler is None:
                    return [TextContent(type="text", text=f"Unknown tool: {name}")]
                content = await handler(conn, state, arguments)
                return [*content, _freshness_content(state)]

    except ValueError as e:
        return [TextContent(type="text", text=str(e))]
    except RuntimeError as e:
        return [TextContent(type="text", text=str(e))]
    except Exception as e:
        logger.error(f"Tool error ({name}): {e}", exc_info=True)
        return [TextContent(type="text", text=f"Error: {str(e)}")]


async def run_server() -> None:
    """Run the MCP server using stdio transport."""
    cfg = load_config()
    if cfg.is_configured:
        init_pool()
        logger.info(f"DB: {cfg.user}@{cfg.host}:{cfg.port}/{cfg.database}")

    async with stdio_server() as (read_stream, write_stream):
        await SERVER.run(
            read_stream,
            write_stream,
            SERVER.create_initialization_options(),
        )


__all__ = ["run_server", "SERVER"]
