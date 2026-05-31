#!/bin/bash
# setup.sh — One-command setup for audit-db MCP server
#
# Usage:
#   curl -sSL https://raw.githubusercontent.com/SlugThugEnterprises/slugaudit-mcp/main/setup.sh | bash
#
# Or:
#   chmod +x setup.sh && ./setup.sh

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo_info() { echo -e "${GREEN}>>> $1${NC}"; }
echo_warn() { echo -e "${YELLOW}!!! $1${NC}"; }
echo_error() { echo -e "${RED}!!! $1${NC}"; }

# Check for required environment variables
check_env() {
    local missing=()
    for var in PGHOST PGDATABASE PGUSER PGPASSWORD; do
        if [ -z "${!var:-}" ]; then
            missing+=("$var")
        fi
    done

    if [ ${#missing[@]} -gt 0 ]; then
        echo_warn "Missing environment variables: ${missing[*]}"
        echo_warn "The MCP server will start, but database tools won't work until these are set."
        echo_warn "Set them before running the server:"
        echo_warn "  export PGHOST=your_db_host"
        echo_warn "  export PGDATABASE=your_db_name"
        echo_warn "  export PGUSER=your_db_user"
        echo_warn "  export PGPASSWORD=your_db_password"
        echo ""
        # Don't fail — server can start without DB
    fi
    return 0
}

# Install system dependencies
install_system_deps() {
    echo_info "Installing system dependencies..."
    if command -v apt &> /dev/null; then
        apt update
        apt install -y python3 python3-pip python3-venv
    elif command -v yum &> /dev/null; then
        yum install -y python3 python3-pip
    elif command -v apk &> /dev/null; then
        apk add --no-cache python3 py3-pip
    else
        echo_error "Unsupported package manager. Please install Python 3 and pip manually."
        exit 1
    fi
}

# Install Python dependencies
install_python_deps() {
    echo_info "Installing Python dependencies..."
    if [ "$INSTALL_MODE" = "venv" ]; then
        python3 -m venv .venv
        source .venv/bin/activate
    fi
    pip3 install --break-system-packages \
        psycopg2-binary \
        mcp \
        tree-sitter \
        tree-sitter-rust \
        tree-sitter-python \
        tree-sitter-typescript \
        tree-sitter-go \
        tree-sitter-java \
        tree-sitter-c \
        tree-sitter-cpp \
        tree-sitter-ruby
}

# Verify installation
verify_install() {
    echo_info "Verifying installation..."
    if python3 -c "import psycopg2; import mcp; import tree_sitter" 2>/dev/null; then
        echo_info "All dependencies installed successfully!"
        return 0
    else
        echo_error "Dependency verification failed."
        return 1
    fi
}

# Show usage
usage() {
    cat << EOF
audit-db MCP Server Setup Script

Usage: $0 [OPTIONS]

Options:
  --mode MODE       Installation mode: 'system' (default) or 'venv'
  --run             Start the MCP server after setup
  --help            Show this help message

Examples:
  # Basic setup
  ./setup.sh

  # Setup in virtual environment and start server
  ./setup.sh --mode venv --run

  # Pipe from curl
  curl -sSL https://raw.githubusercontent.com/SlugThugEnterprises/slugaudit-mcp/main/setup.sh | bash -s -- --run

Environment Variables (required):
  PGHOST            PostgreSQL server hostname
  PGPORT            PostgreSQL port (default: 5432)
  PGDATABASE        Database name
  PGUSER            Database username
  PGPASSWORD        Database password

EOF
}

# Main
main() {
    INSTALL_MODE="system"
    RUN_SERVER=false

    while [[ $# -gt 0 ]]; do
        case $1 in
            --mode)
                INSTALL_MODE="$2"
                shift 2
                ;;
            --run)
                RUN_SERVER=true
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                echo_error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done

    echo_info "audit-db MCP Server Setup"
    echo_info "========================="
    echo ""

    if ! check_env; then
        exit 1
    fi

    install_system_deps
    install_python_deps

    if verify_install; then
        echo ""
        echo_info "Setup complete!"
        echo ""
        echo "To start the MCP server:"
        echo "  export PGHOST=your_db_host"
        echo "  export PGDATABASE=your_db_name"
        echo "  export PGUSER=your_db_user"
        echo "  export PGPASSWORD=your_db_password"
        echo "  python3 mcp_server.py"
        echo ""

        if [ "$RUN_SERVER" = true ]; then
            echo_info "Starting MCP server..."
            python3 mcp_server.py
        fi
    else
        echo_error "Setup failed."
        exit 1
    fi
}

main "$@"
