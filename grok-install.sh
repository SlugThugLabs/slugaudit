#!/bin/bash
# Install slugaudit-mcp as an MCP server in Grok.
#
# Usage: ./grok-install.sh [--scope user|project]
#
# Defaults to --scope user (~/.grok/config.toml).
# Use --scope project to write ./.grok/config.toml in this directory.
#
# Requires: config.toml with [database] section containing host, port, database, user, password.
# Requires: grok CLI on PATH.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCOPE="user"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --scope)
            SCOPE="${2:-}"
            if [[ "$SCOPE" != "user" && "$SCOPE" != "project" ]]; then
                echo "Error: --scope must be 'user' or 'project'" >&2
                exit 1
            fi
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [--scope user|project]"
            echo ""
            echo "Register slugaudit-mcp with Grok (stdio transport)."
            echo "  user    — ~/.grok/config.toml (default)"
            echo "  project — ./.grok/config.toml in this directory"
            exit 0
            ;;
        *)
            echo "Error: unknown argument: $1" >&2
            echo "Usage: $0 [--scope user|project]" >&2
            exit 1
            ;;
    esac
done

if ! command -v grok >/dev/null 2>&1; then
    echo "Error: 'grok' not found on PATH. Install Grok CLI first." >&2
    exit 1
fi

if [[ ! -f "${SCRIPT_DIR}/config.toml" ]]; then
    echo "Warning: ${SCRIPT_DIR}/config.toml not found."
    echo "  cp config.toml.example config.toml  # then edit credentials"
    echo "Continuing anyway (you can set PG* env vars instead)..."
fi

if [[ ! -x "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp" ]]; then
    "${SCRIPT_DIR}/setup.sh"
fi

echo "Adding slugaudit MCP server to Grok (scope=${SCOPE})..."

# Remove existing registration if any (ignore missing)
grok mcp remove slugaudit --scope "$SCOPE" 2>/dev/null || true

# Add the server — config.toml is read by the MCP server itself
grok mcp add slugaudit \
    --scope "$SCOPE" \
    -e "SLUGAUDIT_CONFIG=${SCRIPT_DIR}/config.toml" \
    -- "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp"

echo "Done. Verify with: grok mcp list"
echo "Diagnose with:     grok mcp doctor slugaudit"
echo "In a Grok session: /mcps  (press r to refresh if already running)"
