#!/usr/bin/env python3
"""
slugaudit-mcp MCP Server

Single entry point for AI-powered codebase auditing.

Usage:
    python3 mcp_server.py

Config:
    /opt/slugaudit-mcp/config.toml  or environment variables
    PGHOST, PGPORT, PGDATABASE, PGUSER, PGPASSWORD

Supported platforms: Linux and macOS only. Cross-process synchronization
(app/activation.py, app/sync.py) uses POSIX fcntl file locks, which do not
exist on Windows.
"""

import os
import asyncio
import sys


def _require_posix() -> None:
    """Fail with a clear message instead of a bare ImportError deep in fcntl.

    Cross-process synchronization (app/activation.py, app/sync.py) uses
    POSIX fcntl file locks, which do not exist on Windows. This must run
    before anything imports those modules.
    """
    if os.name != "posix":
        sys.exit(
            "slugaudit-mcp requires Linux or macOS: it uses POSIX fcntl file "
            "locks (app/activation.py, app/sync.py) for cross-process "
            f"synchronization, which are not available on {sys.platform}."
        )


_require_posix()

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app.server import run_server  # noqa: E402 - after the required platform/path setup above


async def run() -> None:
    await run_server()


def main() -> None:
    """Console-script entry point."""
    asyncio.run(run())


if __name__ == "__main__":
    main()
