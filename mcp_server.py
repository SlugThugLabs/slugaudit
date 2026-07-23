#!/usr/bin/env python3
"""
slugaudit-mcp MCP Server

Single entry point for AI-powered codebase auditing.

Usage:
    python3 mcp_server.py

Config:
    /opt/slugaudit-mcp/config.toml  or environment variables
    PGHOST, PGPORT, PGDATABASE, PGUSER, PGPASSWORD
"""

import os
import asyncio
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app.server import run_server


async def run() -> None:
    await run_server()


def main() -> None:
    """Console-script entry point."""
    asyncio.run(run())


if __name__ == "__main__":
    main()
