#!/usr/bin/env bash
# Install SlugAudit MCP from this checkout into an isolated local environment.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV_DIR="${SCRIPT_DIR}/.venv"

if [[ ! -f "${SCRIPT_DIR}/pyproject.toml" ]]; then
    echo "Run setup.sh from a SlugAudit MCP checkout." >&2
    exit 1
fi

if command -v uv >/dev/null 2>&1; then
    uv venv "${VENV_DIR}"
    uv pip install --python "${VENV_DIR}/bin/python" "${SCRIPT_DIR}"
else
    python3 -m venv "${VENV_DIR}"
    "${VENV_DIR}/bin/python" -m pip install --upgrade pip
    "${VENV_DIR}/bin/python" -m pip install "${SCRIPT_DIR}"
fi

echo "Installed SlugAudit MCP: ${VENV_DIR}/bin/slugaudit-mcp"
echo "Configure PostgreSQL with config.toml or PG* environment variables."
echo "Then register that executable as a stdio MCP server in any AI client."
