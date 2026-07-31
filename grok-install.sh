#!/bin/bash
# Install slugaudit-mcp as an MCP server in Grok.
#
# Usage: ./grok-install.sh [--scope user|project]
#
# Defaults to --scope user (~/.grok/config.toml).
# Use --scope project to write ./.grok/config.toml in this directory.
#
# No PostgreSQL setup needed: with no config.toml, each project SlugAudit is
# enabled for gets its own zero-config SQLite index automatically. Only
# create config.toml (see config.toml.example) if you want a shared
# PostgreSQL index served across multiple developers or machines instead.
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

if [[ ! -x "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp" ]]; then
    "${SCRIPT_DIR}/setup.sh"
fi

echo "Adding slugaudit MCP server to Grok (scope=${SCOPE})..."

# Remove existing registration if any (ignore missing)
grok mcp remove slugaudit --scope "$SCOPE" 2>/dev/null || true

if [[ -f "${SCRIPT_DIR}/config.toml" ]]; then
    # config.toml present — PostgreSQL backend. Read by the server itself.
    grok mcp add slugaudit \
        --scope "$SCOPE" \
        -e "SLUGAUDIT_CONFIG=${SCRIPT_DIR}/config.toml" \
        -- "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp"
else
    # No config.toml — zero-config SQLite backend, one file per project.
    grok mcp add slugaudit \
        --scope "$SCOPE" \
        -- "${SCRIPT_DIR}/.venv/bin/slugaudit-mcp"
fi

echo "Done. Verify with: grok mcp list"
echo "Diagnose with:     grok mcp doctor slugaudit"
echo "In a Grok session: /mcps  (press r to refresh if already running)"
