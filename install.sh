#!/usr/bin/env bash
# SlugAudit Universal Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash

set -euo pipefail

INSTALL_DIR="${HOME}/.slugthug/slugaudit"
BINARY_NAME="slugaudit-mcp"
TARGET_PATH="${INSTALL_DIR}/${BINARY_NAME}"
GITHUB_REPO="SlugThugLabs/slugaudit"

# Print banner
echo -e "\033[36m"
cat << 'BANNER'
  ____  _             _             _ _ _   
 / ___|| |_   _  __ _/ \  _   _  __| (_) |_ 
 \___ \| | | | |/ _` / _ \| | | |/ _` | | __|
  ___) | | |_| | (_| / ___ \ |_| | (_| | | |_ 
 |____/|_|\__,_|\__, /_/   \_\__,_|\__,_|_|\__|
                |___/                          
BANNER
echo -e "\033[0m"
echo "🐌 SlugAudit Installer: Codebase Intelligence over MCP"
echo "--------------------------------------------------------"

# Detect OS and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

mkdir -p "${INSTALL_DIR}"

if command -v cargo >/dev/null 2>&1 && [ -f "Cargo.toml" ] && grep -q "slugaudit-mcp-rust" Cargo.toml 2>/dev/null; then
    echo "==> Building directly from local repository with cargo..."
    cargo build --release --locked --bin slugaudit-mcp
    install -m 0755 target/release/slugaudit-mcp "${TARGET_PATH}"
elif [ "${OS}" = "Linux" ] && [ "${ARCH}" = "x86_64" ]; then
    echo "==> Downloading latest release for Linux x86_64..."
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/slugaudit-mcp-x86_64-unknown-linux-gnu"
    TMP_FILE="$(mktemp)"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "${DOWNLOAD_URL}" -o "${TMP_FILE}"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "${TMP_FILE}" "${DOWNLOAD_URL}"
    else
        echo "Error: curl or wget is required to download SlugAudit." >&2
        exit 1
    fi
    install -m 0755 "${TMP_FILE}" "${TARGET_PATH}"
    rm -f "${TMP_FILE}"
elif command -v cargo >/dev/null 2>&1; then
    echo "==> Installing via cargo..."
    cargo install --git "https://github.com/${GITHUB_REPO}.git" --bin slugaudit-mcp
else
    echo "Error: Pre-built binary currently available for Linux x86_64."
    echo "To build on ${OS} (${ARCH}), install Rust (https://rustup.rs) and run: cargo install --git https://github.com/${GITHUB_REPO}.git"
    exit 1
fi

echo ""
echo "✅ SlugAudit successfully installed to: ${TARGET_PATH}"
echo ""
echo "Next steps:"
echo "  1. Add to PATH (if not already present):"
echo "     export PATH=\"\${HOME}/.slugthug/slugaudit:\${PATH}\""
echo ""
echo "  2. Connect to your AI coding agent (Claude Code, Cursor, Codex, Bob, Grok):"
echo "     ${TARGET_PATH} connect"
echo ""
echo "  3. Or run the interactive setup menu:"
echo "     ${TARGET_PATH} menu"
echo ""
