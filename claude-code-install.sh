#!/bin/bash
# Install slugaudit-mcp as an MCP server in Claude Code.
#
# Usage: ./claude-code-install.sh
#
# No PostgreSQL setup needed: with no config.toml, each project SlugAudit is
# enabled for gets its own zero-config SQLite index automatically. Only
# create config.toml (see config.toml.example) if you want a shared
# PostgreSQL index served across multiple developers or machines instead.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Adding slugaudit MCP server to Claude Code..."

if [[ ! -x "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp" ]]; then
    "${SCRIPT_DIR}/setup.sh"
fi

# Remove existing registration if any
claude mcp remove slugaudit -s local 2>/dev/null || true

if [[ -f "${SCRIPT_DIR}/config.toml" ]]; then
    # config.toml present — PostgreSQL backend. Read by the server itself.
    claude mcp add slugaudit \
        -e SLUGAUDIT_CONFIG="${SCRIPT_DIR}/config.toml" \
        -- "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp"
else
    # No config.toml — zero-config SQLite backend, one file per project.
    claude mcp add slugaudit -- "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp"
fi

echo "Done. Verify with: claude mcp list"
