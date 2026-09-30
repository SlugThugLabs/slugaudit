#!/usr/bin/env bash
# SlugAudit Universal Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash

set -euo pipefail

INSTALL_DIR="${HOME}/.slugthug/slugaudit"
BINARY_NAME="slugaudit"
TARGET_PATH="${INSTALL_DIR}/${BINARY_NAME}"
PREVIOUS_PATH="${TARGET_PATH}.prev"
GITHUB_REPO="SlugThugLabs/slugaudit"

# Retains the binary being replaced as "${TARGET_PATH}.prev", so a version that
# turns out to be worse than the one it replaced can be restored with a copy.
# Fails closed: installing silently without a retained copy would leave the
# operator believing a revert is possible when it is not.
retain_previous() {
    [ -f "${TARGET_PATH}" ] || return 0
    # `cp` would happily copy *into* a directory of that name and report
    # success, leaving the operator believing a revert is available.
    if [ -e "${PREVIOUS_PATH}" ] && [ ! -f "${PREVIOUS_PATH}" ]; then
        echo "Error: ${PREVIOUS_PATH} exists and is not a regular file; refusing to replace ${TARGET_PATH}." >&2
        exit 1
    fi
    if ! cp -p "${TARGET_PATH}" "${PREVIOUS_PATH}"; then
        echo "Error: could not retain the current binary as ${PREVIOUS_PATH}; refusing to replace it." >&2
        exit 1
    fi
}

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
    cargo build --release --locked --bin slugaudit
    retain_previous
    install -m 0755 target/release/slugaudit "${TARGET_PATH}"
elif [ "${OS}" = "Linux" ] && [ "${ARCH}" = "x86_64" ]; then
    echo "==> Downloading latest release for Linux x86_64..."
    ASSET_NAME="slugaudit-x86_64-unknown-linux-gnu"
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/${ASSET_NAME}"
    CHECKSUM_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/SHA256SUMS"
    TMP_DIR="$(mktemp -d)"
    TMP_FILE="${TMP_DIR}/${ASSET_NAME}"
    TMP_SUMS="${TMP_DIR}/SHA256SUMS"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "${DOWNLOAD_URL}" -o "${TMP_FILE}"
        curl -fsSL "${CHECKSUM_URL}" -o "${TMP_SUMS}"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "${TMP_FILE}" "${DOWNLOAD_URL}"
        wget -qO "${TMP_SUMS}" "${CHECKSUM_URL}"
    else
        rm -rf "${TMP_DIR}"
        echo "Error: curl or wget is required to download SlugAudit." >&2
        exit 1
    fi

    echo "==> Verifying SHA256 checksum..."
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "${TMP_DIR}" && sha256sum --check --ignore-missing SHA256SUMS >/dev/null 2>&1) || {
            rm -rf "${TMP_DIR}"
            echo "Error: SHA256 checksum verification failed." >&2
            exit 1
        }
    elif command -v shasum >/dev/null 2>&1; then
        (cd "${TMP_DIR}" && shasum -a 256 --check --ignore-missing SHA256SUMS >/dev/null 2>&1) || {
            rm -rf "${TMP_DIR}"
            echo "Error: SHA256 checksum verification failed." >&2
            exit 1
        }
    else
        rm -rf "${TMP_DIR}"
        echo "Error: sha256sum or shasum is required to verify the downloaded binary." >&2
        exit 1
    fi

    retain_previous
    install -m 0755 "${TMP_FILE}" "${TARGET_PATH}"
    rm -rf "${TMP_DIR}"
elif command -v cargo >/dev/null 2>&1; then
    echo "==> Installing via cargo..."
    cargo install --git "https://github.com/${GITHUB_REPO}.git" --bin slugaudit
else
    echo "Error: Pre-built binary currently available for Linux x86_64."
    echo "To build on ${OS} (${ARCH}), install Rust (https://rustup.rs) and run: cargo install --git https://github.com/${GITHUB_REPO}.git"
    exit 1
fi

echo ""
echo "✅ SlugAudit successfully installed to: ${TARGET_PATH}"
echo ""
if [ -f "${PREVIOUS_PATH}" ]; then
    echo "The version this replaced is kept at: ${PREVIOUS_PATH}"
    echo "To revert:  cp \"${PREVIOUS_PATH}\" \"${TARGET_PATH}\""
    echo ""
fi
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
