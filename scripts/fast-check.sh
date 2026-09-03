#!/usr/bin/env bash
set -uo pipefail

# Fast Rust pattern checker - lightweight 5ms sanity check before cargo/clippy
# Uses regex patterns to catch common issues without full compilation

EXIT_CODE=0
WARNINGS=0
ERRORS=0
CRITICAL=0

# Color output
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
NC='\033[0m'

check_one_file() {
    local RUST_FILE="$1"

    if [ ! -f "$RUST_FILE" ]; then
        echo "Error: File not found: $RUST_FILE"
        return 1
    fi

    # Detect if this is a test file (standard Rust or SlugAudit convention)
    local IS_TEST_FILE=0
    if [[ "$RUST_FILE" =~ /tests?/ ]] || [[ "$RUST_FILE" =~ _tests?\.rs$ ]] || [[ "$RUST_FILE" =~ /tests\.rs$ ]] || [[ "$RUST_FILE" =~ test_support\.rs$ ]] || [[ "$RUST_FILE" =~ proptest\.rs$ ]]; then
        IS_TEST_FILE=1
    fi

    # Critical patterns (fails the check across all files)
    declare -A CRITICAL_PATTERNS=(
        ["unsafe"]='\bunsafe\s+(fn|impl|trait|\{|struct|enum)'
        ["todo"]='(TODO|FIXME|HACK|XXX|TEMP|WIP|PLACEHOLDER)'
        ["unimplemented"]='\bunimplemented!\s*\('
        ["allow_dead_code"]='#\[allow\(dead_code\)\]'
        ["allow_unused"]='#\[allow\(unused'
        ["dbg_macro"]='\bdbg!\s*\('
    )

    # Production-only critical patterns (tests are allowed to unwrap/expect/panic)
    declare -A PRODUCTION_ONLY_PATTERNS=(
        ["unwrap"]='\.unwrap\s*\('
        ["expect"]='\.expect\s*\('
        ["panic"]='\bpanic!\s*\('
    )

    # Error patterns (bad habits / anti-patterns)
    declare -A ERROR_PATTERNS=(
        ["clone_on_copy"]='\.clone\(\)'
        ["as_str_to_string"]='\.as_str\(\)\.to_string\(\)'
    )

    # Warning patterns
    declare -A WARNING_PATTERNS=(
        ["missing_doc_comment"]='^\s*pub\s+(fn|struct|enum|trait)\s+\w+'
    )

    check_pattern() {
        local pattern="$1"
        local severity="$2"
        local description="$3"
        local file="$4"
        local prod_only="${5:-0}"

        local line_numbers=""
        if [ "$prod_only" -eq 1 ] && grep -qn '#\[cfg(test)\]' "$file" 2>/dev/null; then
            local test_start
            test_start=$(grep -n '#\[cfg(test)\]' "$file" | head -n1 | cut -d: -f1)
            line_numbers=$(grep -Pn "$pattern" "$file" 2>/dev/null | awk -F: -v limit="$test_start" '$1 < limit {print $1}' | tr '\n' ',' | sed 's/,$//')
        else
            line_numbers=$(grep -Pn "$pattern" "$file" 2>/dev/null | cut -d: -f1 | tr '\n' ',' | sed 's/,$//')
        fi

        if [ -n "$line_numbers" ]; then
            case "$severity" in
                critical)
                    echo -e "${RED}🚨 CRITICAL${NC}: $description"
                    echo "   File: $file"
                    echo "   Lines: $line_numbers"
                    ((CRITICAL++))
                    EXIT_CODE=1
                    ;;
                error)
                    echo -e "${RED}❌ ERROR${NC}: $description"
                    echo "   File: $file"
                    echo "   Lines: $line_numbers"
                    ((ERRORS++))
                    EXIT_CODE=1
                    ;;
                warning)
                    echo -e "${YELLOW}⚠️  WARNING${NC}: $description"
                    echo "   File: $file"
                    echo "   Lines: $line_numbers"
                    ((WARNINGS++))
                    ;;
            esac
        fi
    }

    echo "🔍 Fast-checking Rust file: $RUST_FILE"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    # Check file length (200 line auto-pass limit, 300 with approved exception header)
    local LINE_COUNT
    LINE_COUNT=$(wc -l < "$RUST_FILE")
    if [ "$LINE_COUNT" -gt 300 ]; then
        echo -e "${RED}🚨 CRITICAL${NC}: File exceeds hard 300-line ceiling"
        echo "   File: $RUST_FILE"
        echo "   Lines: $LINE_COUNT (limit: 300)"
        echo "   Split this file into smaller modules"
        ((CRITICAL++))
        EXIT_CODE=1
    elif [ "$LINE_COUNT" -gt 200 ] && [ "$IS_TEST_FILE" -eq 0 ]; then
        if ! grep -q "slugaudit-line-exception:" "$RUST_FILE" 2>/dev/null; then
            echo -e "${RED}🚨 CRITICAL${NC}: File exceeds 200 lines without approved slugaudit-line-exception header"
            echo "   File: $RUST_FILE"
            echo "   Lines: $LINE_COUNT (limit: 200)"
            ((CRITICAL++))
            EXIT_CODE=1
        fi
    fi

    # Check critical patterns
    for desc in "${!CRITICAL_PATTERNS[@]}"; do
        check_pattern "${CRITICAL_PATTERNS[$desc]}" "critical" "No $desc allowed" "$RUST_FILE"
    done

    # Check production-only patterns
    if [ "$IS_TEST_FILE" -eq 0 ]; then
        for desc in "${!PRODUCTION_ONLY_PATTERNS[@]}"; do
            check_pattern "${PRODUCTION_ONLY_PATTERNS[$desc]}" "critical" "No $desc allowed in production code" "$RUST_FILE" 1
        done
    fi

    # Check println! outside of main.rs, CLI tools (cli.rs, menu.rs, install.rs, update.rs), bin tools, and tests
    local IS_CLI_OR_BIN=0
    if [[ "$RUST_FILE" =~ main\.rs$ ]] || [[ "$RUST_FILE" =~ src/(cli|menu|install|update)\.rs$ ]] || [[ "$RUST_FILE" =~ src/bin/ ]] || [ "$IS_TEST_FILE" -eq 1 ]; then
        IS_CLI_OR_BIN=1
    fi

    if [ "$IS_CLI_OR_BIN" -eq 0 ]; then
        if grep -Pq '\bprintln!\s*\(' "$RUST_FILE" 2>/dev/null; then
            local line_numbers
            line_numbers=$(grep -Pn '\bprintln!\s*\(' "$RUST_FILE" 2>/dev/null | cut -d: -f1 | tr '\n' ',' | sed 's/,$//')
            echo -e "${RED}❌ ERROR${NC}: println! to stdout will corrupt MCP JSON-RPC protocol"
            echo "   File: $RUST_FILE"
            echo "   Lines: $line_numbers"
            echo "   Use tracing (tracing::info!/debug!) instead"
            ((ERRORS++))
            EXIT_CODE=1
        fi
    fi

    # Check for single-letter variable names (production only)
    if [ "$IS_TEST_FILE" -eq 0 ]; then
        if grep -Pn '^\s*let\s+(mut\s+)?[a-hln-wA-Z]\s*[=:]' "$RUST_FILE" 2>/dev/null | grep -v '\s*[ijk]\s*[=:]' | grep -v '\s*[xyz]\s*[=:]' > /dev/null; then
            local line_numbers
            line_numbers=$(grep -Pn '^\s*let\s+(mut\s+)?[a-hln-wA-Z]\s*[=:]' "$RUST_FILE" 2>/dev/null | grep -v '\s*[ijk]\s*[=:]' | grep -v '\s*[xyz]\s*[=:]' | cut -d: -f1 | tr '\n' ',' | sed 's/,$//')
            echo -e "${YELLOW}⚠️  WARNING${NC}: Single-letter variable names detected"
            echo "   File: $RUST_FILE"
            echo "   Lines: $line_numbers"
            echo "   Use descriptive names (except i,j,k for loops or x,y,z for math)"
            ((WARNINGS++))
        fi
    fi

    # Check function length (50 line limit) - matches pub, pub(crate), async, and standard fn
    awk '
    /^[[:space:]]*(pub(\([^\)]+\))?[[:space:]]+)?(async[[:space:]]+)?(const[[:space:]]+)?fn[[:space:]]+[a-zA-Z0-9_]+/ {
        fn_start = NR
        brace_count = 0
        in_function = 1
    }
    in_function && /{/ { brace_count++ }
    in_function && /}/ {
        brace_count--
        if (brace_count == 0) {
            fn_length = NR - fn_start + 1
            if (fn_length > 50) {
                printf "Function too long: line %d, length %d lines\n", fn_start, fn_length
            }
            in_function = 0
        }
    }
    ' "$RUST_FILE" | while read -r line; do
        if [[ -n "$line" ]]; then
            echo -e "${RED}🚨 CRITICAL${NC}: $line"
            echo "   File: $RUST_FILE"
            echo "   Limit: 50 lines per function"
            echo "   Split into smaller functions"
            ((CRITICAL++))
            EXIT_CODE=1
        fi
    done

    # Check error patterns
    for desc in "${!ERROR_PATTERNS[@]}"; do
        check_pattern "${ERROR_PATTERNS[$desc]}" "error" "$desc should be avoided" "$RUST_FILE"
    done
}

# Collect target files:
# 1. If arguments provided, check those files
# 2. If no arguments, check modified/staged git files
FILES=()
if [ $# -gt 0 ]; then
    FILES=("$@")
else
    while IFS= read -r f; do
        [ -n "$f" ] && FILES+=("$f")
    done < <(git diff --name-only --diff-filter=d HEAD '*.rs' 2>/dev/null || true)
    
    if [ ${#FILES[@]} -eq 0 ]; then
        # Check all tracked files in src/ if git status was clean
        echo "No modified .rs files found in git status. Pass a filename to check: $0 <path.rs>"
        exit 0
    fi
fi

for f in "${FILES[@]}"; do
    check_one_file "$f"
done

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 Summary:"
echo "   🚨 Critical: $CRITICAL"
echo "   ❌ Errors: $ERRORS"
echo "   ⚠️  Warnings: $WARNINGS"

if [ $EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}✅ Fast-check passed!${NC}"
else
    echo -e "${RED}❌ Fast-check FAILED - review issues above${NC}"
fi

exit $EXIT_CODE
