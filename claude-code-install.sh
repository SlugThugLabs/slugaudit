#!/bin/bash
# Install slugaudit-mcp as an MCP server in Claude Code.
#
# Usage: ./claude-code-install.sh
#
# Requires: config.toml with [database] section containing host, port, database, user, password.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Adding slugaudit MCP server to Claude Code..."

if [[ ! -x "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp" ]]; then
    "${SCRIPT_DIR}/setup.sh"
fi

# Remove existing registration if any
claude mcp remove slugaudit -s local 2>/dev/null || true

# Add the server — config.toml is read by the MCP server itself (no env var overrides needed)
claude mcp add slugaudit \
    -e SLUGAUDIT_CONFIG="${SCRIPT_DIR}/config.toml" \
    -- "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp"

echo "Done. Verify with: claude mcp list"
