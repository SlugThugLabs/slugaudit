"""MCP server setup, tool routing, and runner.

Binds together: input validation (mcp/tools.py), auto-sync (mcp/sync.py),
connection pool (mcp/pool.py), and handlers (mcp/handlers.py).
"""

import asyncio
import hmac
import json
import logging
import os
from pathlib import Path
from typing import Any

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent

from app.activation import disable_project, enable_project
from app.config import load_config
from app.handlers import HANDLERS
from app.instructions import MCP_INSTRUCTIONS
from app.pool import get_connection_for_project as get_db, init_pool
from app.sync import synchronized_project
from app.tools import TOOLS

logger = logging.getLogger("slugaudit-mcp.server")

SERVER = Server("slugaudit-mcp", instructions=MCP_INSTRUCTIONS)

# Reserved host-adapter protocol. Like the arguments a host injects for its
# own built-in Read/Write/Bash tools, these two names are never advertised to
# the model and are meant to be set only by the trusted host layer that
# constructs each tool call, not by the model itself. See README's
# "Host-adapter protocol" section for the full rationale and the one thing
# SlugAudit genuinely cannot verify from inside this codebase: whether a
# given host actually enforces that separation.
PROJECT_ROOT_ARGUMENT = "_project_root"
PROJECT_CONTROL_TOOL = "_slugaudit_project_control"

# Shared secret gating the _project_root override. No unauthenticated
# fallback: without SLUGAUDIT_HOST_TOKEN configured, _project_root is never
# honored at all and this process always uses its own cwd. See README.
HOST_TOKEN_ENV_VAR = "SLUGAUDIT_HOST_TOKEN"  # noqa: S105 - a name, not a secret
HOST_TOKEN_ARGUMENT = "_host_token"  # noqa: S105 - a name, not a secret


def _host_token_configured() -> str | None:
    configured = os.environ.get(HOST_TOKEN_ENV_VAR)
    return configured if configured else None


def _host_token_matches(supplied: Any, configured: str) -> bool:
    """Constant-time comparison so token checks can't be timed out."""
    if not isinstance(supplied, str) or not supplied:
        return False
    return hmac.compare_digest(supplied, configured)


def _extract_project_root(arguments: dict[str, Any]) -> str:
    """Remove and normalize the host-injected root, preserving cwd fallback.

    The override is only ever honored when SLUGAUDIT_HOST_TOKEN is
    configured and the caller supplies a matching "_host_token" argument.
    No unauthenticated fallback: if no token is configured at all, this
    process always uses its own cwd, exactly as if no _project_root had
    been sent — there is no backward-compatible "honor it anyway" path.
    """
    injected = arguments.pop(PROJECT_ROOT_ARGUMENT, None)
    supplied_token = arguments.pop(HOST_TOKEN_ARGUMENT, None)
    if injected is None:
        return os.getcwd()
    if not isinstance(injected, str) or not injected.strip():
        raise ValueError(f"{PROJECT_ROOT_ARGUMENT} must be a non-empty path string")

    configured_token = _host_token_configured()
    if configured_token is None:
        logger.warning(
            "Rejected %s: %s is not configured, so this override can never "
            "be authenticated. Falling back to this process's own working "
            "directory.",
            PROJECT_ROOT_ARGUMENT,
            HOST_TOKEN_ENV_VAR,
        )
        return os.getcwd()
    if not _host_token_matches(supplied_token, configured_token):
        logger.warning(
            "Rejected %s: missing or invalid %s. Falling back to this "
            "process's own working directory.",
            PROJECT_ROOT_ARGUMENT,
            HOST_TOKEN_ARGUMENT,
        )
        return os.getcwd()
    return str(Path(injected).expanduser().resolve())


def _control_content(action: str, project_root: str, changed: bool) -> TextContent:
    """Return a small machine-readable result for native client adapters."""
    return TextContent(
        type="text",
        text=json.dumps(
            {
                "slugaudit_control": {
                    "action": action,
                    "project_root": str(Path(project_root).resolve()),
                    "changed": changed,
                }
            },
            separators=(",", ":"),
        ),
    )


async def _project_control(arguments: dict[str, Any]) -> list[TextContent]:
    """Execute the hidden host lifecycle route without exposing it to AI tools."""
    project_root = _extract_project_root(arguments)
    action = arguments.pop("action", None)
    if arguments:
        unexpected = ", ".join(sorted(arguments))
        raise ValueError(f"Unexpected project control arguments: {unexpected}")
    if action == "on":
        was_enabled = (Path(project_root) / ".planning" / "slugaudit").is_dir()
        activation = enable_project(project_root)
        return [_control_content(action, project_root, not was_enabled and activation.is_dir())]
    if action == "off":
        if not (Path(project_root) / ".planning" / "slugaudit").is_dir():
            # Nothing to purge, and the SQLite backend's connection requires
            # this directory to exist — check first so an already-disabled
            # project behaves identically (no-op) under either backend,
            # matching "on" not opening a connection when nothing changes.
            return [_control_content(action, project_root, False)]
        async with get_db(project_root) as conn:
            changed = await asyncio.to_thread(disable_project, project_root, conn)
        return [_control_content(action, project_root, changed)]
    raise ValueError("Project control action must be exactly 'on' or 'off'")


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
    routed_arguments = dict(arguments)
    if name == PROJECT_CONTROL_TOOL:
        return await _project_control(routed_arguments)

    try:
        project_root = _extract_project_root(routed_arguments)

        # Single connection shared between sync and handler
        async with get_db(project_root) as conn:
            async with synchronized_project(project_root, conn) as state:
                handler = HANDLERS.get(name)
                if handler is None:
                    return [TextContent(type="text", text=f"Unknown tool: {name}")]
                content = await handler(conn, state, routed_arguments)
                return [*content, _freshness_content(state)]

    except Exception:
        logger.exception("Tool error (%s)", name)
        raise


async def run_server() -> None:
    """Run the MCP server using stdio transport."""
    cfg = load_config()
    if cfg.is_configured:
        init_pool()
        logger.info(f"DB: {cfg.user}@{cfg.host}:{cfg.port}/{cfg.database}")

    if _host_token_configured() is None:
        logger.info(
            "%s is not set: this process will always use its own working "
            "directory as the active project. Any %s argument on a tool "
            "call is rejected outright rather than honored — there is no "
            "unauthenticated fallback. Set %s in this process's environment "
            "and have your host adapter send the same value as %s if you "
            "need this server to serve more than one project directory.",
            HOST_TOKEN_ENV_VAR,
            PROJECT_ROOT_ARGUMENT,
            HOST_TOKEN_ENV_VAR,
            HOST_TOKEN_ARGUMENT,
        )

    async with stdio_server() as (read_stream, write_stream):
        await SERVER.run(
            read_stream,
            write_stream,
            SERVER.create_initialization_options(),
        )


__all__ = [
    "HOST_TOKEN_ARGUMENT",
    "HOST_TOKEN_ENV_VAR",
    "PROJECT_CONTROL_TOOL",
    "PROJECT_ROOT_ARGUMENT",
    "run_server",
    "SERVER",
]
