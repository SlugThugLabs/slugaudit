#!/usr/bin/env bash
# SlugAudit Universal Installer
# Usage: curl -q -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash

set -euo pipefail

INSTALL_DIR="${HOME}/.slugthug/slugaudit"
BINARY_NAME="slugaudit"
TARGET_PATH="${INSTALL_DIR}/${BINARY_NAME}"
PREVIOUS_PATH="${TARGET_PATH}.prev"
GITHUB_REPO="SlugThugLabs/slugaudit"

usage() {
    cat << 'EOF'
Usage: install.sh [OPTIONS]

Install the SlugAudit binary into ~/.slugthug/slugaudit/slugaudit.

Options:
  --from-source, --local   Build and install from ./Cargo.toml in the current directory
  -h, --help               Show this help message and exit
EOF
}

FROM_SOURCE=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --from-source|--local)
            FROM_SOURCE=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

# Temp dir and staging files; removed on every exit path.
TMP_DIR=""
STAGED_BIN=""
STAGED_PREV=""
cleanup() {
    if [ -n "${TMP_DIR}" ]; then
        rm -rf "${TMP_DIR}"
    fi
    if [ -n "${STAGED_BIN}" ]; then
        rm -f "${STAGED_BIN}"
    fi
    if [ -n "${STAGED_PREV}" ]; then
        rm -f "${STAGED_PREV}"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

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
    STAGED_PREV="$(mktemp "${INSTALL_DIR}/.slugaudit.prev.XXXXXX")"
    if ! cp -p "${TARGET_PATH}" "${STAGED_PREV}" || ! mv -f "${STAGED_PREV}" "${PREVIOUS_PATH}"; then
        echo "Error: could not retain the current binary as ${PREVIOUS_PATH}; refusing to replace it." >&2
        exit 1
    fi
    STAGED_PREV=""
}

# Stages the new binary in ${INSTALL_DIR} and atomically moves it into place.
install_binary() {
    local src="$1"
    STAGED_BIN="$(mktemp "${INSTALL_DIR}/.slugaudit.new.XXXXXX")"
    install -m 0755 "${src}" "${STAGED_BIN}"
    mv -f "${STAGED_BIN}" "${TARGET_PATH}"
    STAGED_BIN=""
}

# Print banner
printf '\033[36m\n'
cat << 'BANNER'
  ____  _             _             _ _ _   
 / ___|| |_   _  __ _/ \  _   _  __| (_) |_ 
 \___ \| | | | |/ _` / _ \| | | |/ _` | | __|
  ___) | | |_| | (_| / ___ \ |_| | (_| | | |_ 
 |____/|_|\__,_|\__, /_/   \_\__,_|\__,_|_|\__|
                |___/                          
BANNER
printf '\033[0m\n'
echo "SlugAudit Installer: Codebase Intelligence over MCP"
echo "--------------------------------------------------------"

# Detect OS and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

mkdir -p "${INSTALL_DIR}"

if [ "${FROM_SOURCE}" -eq 1 ]; then
    if ! command -v cargo >/dev/null 2>&1 || [ ! -f "Cargo.toml" ] || ! grep -Eq '^name = "slugaudit-mcp-rust"$' Cargo.toml 2>/dev/null; then
        echo "Error: --from-source requires cargo and a slugaudit-mcp-rust Cargo.toml in the current directory." >&2
        exit 1
    fi
    echo "==> Building directly from local repository with cargo..."
    TARGET_DIR="${CARGO_TARGET_DIR:-target}"
    BUILT_BIN="${TARGET_DIR}/release/slugaudit"
    TSLP_LINK_MODE=static TSLP_LANGUAGES=rust cargo build --release --locked --bin slugaudit
    if [ ! -f "${BUILT_BIN}" ]; then
        echo "Error: expected built binary at ${BUILT_BIN}, but it was not found." >&2
        exit 1
    fi
    retain_previous
    install_binary "${BUILT_BIN}"
elif [ "${OS}" = "Linux" ] && [ "${ARCH}" = "x86_64" ]; then
    echo "==> Resolving latest release tag for Linux x86_64..."
    ASSET_NAME="slugaudit-x86_64-unknown-linux-gnu"
    API_URL="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"

    if command -v curl >/dev/null 2>&1; then
        RELEASE_JSON="$(curl -q -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 30 -- "${API_URL}")"
    elif command -v wget >/dev/null 2>&1; then
        RELEASE_JSON="$(wget --https-only --timeout=30 -qO- "${API_URL}")"
    else
        echo "Error: curl or wget is required to download SlugAudit." >&2
        exit 1
    fi

    TAG="$(printf '%s\n' "${RELEASE_JSON}" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
    if [ -z "${TAG}" ] || ! printf '%s\n' "${TAG}" | grep -Eq '^v?[0-9]+\.[0-9]+(\.[0-9]+)*$'; then
        echo "Error: could not determine a valid numeric release tag from ${API_URL}." >&2
        exit 1
    fi

    echo "==> Downloading release ${TAG} for Linux x86_64..."
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/${ASSET_NAME}"
    CHECKSUM_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/SHA256SUMS"
    TMP_DIR="$(mktemp -d)"
    TMP_FILE="${TMP_DIR}/${ASSET_NAME}"
    TMP_SUMS="${TMP_DIR}/SHA256SUMS"

    if command -v curl >/dev/null 2>&1; then
        curl -q -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 300 -o "${TMP_FILE}" -- "${DOWNLOAD_URL}"
        curl -q -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 300 -o "${TMP_SUMS}" -- "${CHECKSUM_URL}"
    else
        wget --https-only --timeout=300 -qO "${TMP_FILE}" "${DOWNLOAD_URL}"
        wget --https-only --timeout=300 -qO "${TMP_SUMS}" "${CHECKSUM_URL}"
    fi

    echo "==> Verifying SHA256 checksum..."
    EXPECTED_SHA="$(awk -v target="${ASSET_NAME}" '
        {
            sub(/\r$/, "")
            if (NF == 2) {
                name = $2
                sub(/^\*/, "", name)
                sub(/^\.\//, "", name)
                if (name == target) {
                    print tolower($1)
                    exit
                }
            }
        }
    ' "${TMP_SUMS}")"

    if ! printf '%s\n' "${EXPECTED_SHA}" | grep -Eq '^[0-9a-f]{64}$'; then
        echo "Error: SHA256SUMS has no valid entry for ${ASSET_NAME}." >&2
        exit 1
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        ACTUAL_SHA="$(sha256sum -- "${TMP_FILE}" | awk '{print tolower($1)}')"
    elif command -v shasum >/dev/null 2>&1; then
        ACTUAL_SHA="$(shasum -a 256 -- "${TMP_FILE}" | awk '{print tolower($1)}')"
    else
        echo "Error: sha256sum or shasum is required to verify the downloaded binary." >&2
        exit 1
    fi

    if [ "${ACTUAL_SHA}" != "${EXPECTED_SHA}" ]; then
        echo "Error: SHA256 checksum verification failed for ${ASSET_NAME}." >&2
        exit 1
    fi

    retain_previous
    install_binary "${TMP_FILE}"
elif command -v cargo >/dev/null 2>&1; then
    echo "==> Installing via cargo..."
    TMP_DIR="$(mktemp -d)"
    TSLP_LINK_MODE=static TSLP_LANGUAGES=rust cargo install --locked --git "https://github.com/${GITHUB_REPO}.git" --bin slugaudit --root "${TMP_DIR}"
    retain_previous
    install_binary "${TMP_DIR}/bin/slugaudit"
else
    echo "Error: Pre-built binary currently available for Linux x86_64." >&2
    echo "To build on ${OS} (${ARCH}), install Rust (https://rustup.rs) and run: TSLP_LINK_MODE=static TSLP_LANGUAGES=rust cargo install --locked --git https://github.com/${GITHUB_REPO}.git --bin slugaudit" >&2
    exit 1
fi

echo ""
echo "SlugAudit successfully installed to: ${TARGET_PATH}"
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
