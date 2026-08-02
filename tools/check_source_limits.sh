#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
project_root="$(CDPATH= cd -- "$script_dir/.." && pwd)"

if [[ "${1:-}" == "--help" ]]; then
    printf '%s\n' "Checks production Rust code-line limits under src/."
    exit 0
fi

if [[ "${1:-}" == "--root" ]]; then
    project_root="${2:?--root requires a project directory}"
fi

exec python3 - "$project_root" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys


EXCEPTION = re.compile(
    r"slugaudit-line-exception:\s*approved-by=agent;\s*reason=(\S.+)"
)


def code_lines(source: str) -> int:
    """Count lines containing Rust tokens outside comments and whitespace."""
    count = 0
    has_code = False
    state = "normal"
    block_depth = 0
    raw_hashes = 0
    index = 0
    line_start = 0

    def finish_line(end: int) -> None:
        nonlocal count, has_code, line_start
        if has_code:
            count += 1
        has_code = False
        line_start = end + 1

    while index < len(source):
        char = source[index]
        following = source[index + 1 : index + 2]
        if char == "\n":
            if state == "line_comment":
                state = "normal"
            finish_line(index)
            index += 1
            continue
        if state == "line_comment":
            index += 1
            continue
        if state == "block_comment":
            if source[index : index + 2] == "/*":
                block_depth += 1
                index += 2
            elif source[index : index + 2] == "*/":
                block_depth -= 1
                index += 2
                if block_depth == 0:
                    state = "normal"
            else:
                index += 1
            continue
        if state == "string":
            has_code = True
            if char == "\\":
                index += 2
            elif char == '"':
                state = "normal"
                index += 1
            else:
                index += 1
            continue
        if state == "char":
            has_code = True
            if char == "\\":
                index += 2
            elif char == "'":
                state = "normal"
                index += 1
            else:
                index += 1
            continue
        if state == "raw_string":
            has_code = True
            terminator = '"' + ("#" * raw_hashes)
            if source.startswith(terminator, index):
                state = "normal"
                index += len(terminator)
            else:
                index += 1
            continue
        if source[index : index + 2] == "//":
            state = "line_comment"
            index += 2
            continue
        if source[index : index + 2] == "/*":
            state = "block_comment"
            block_depth = 1
            index += 2
            continue
        if char == '"':
            state = "string"
            has_code = True
            index += 1
            continue
        if char == "'":
            char_literal = re.match(r"'(?:\\.|[^'\\\n])'", source[index:])
            if char_literal:
                state = "char"
                has_code = True
                index += 1
                continue
        if char == "r":
            match = re.match(r'r(#+)?"', source[index:])
            if match:
                raw_hashes = len(match.group(1) or "")
                state = "raw_string"
                has_code = True
                index += len(match.group(0))
                continue
        if not char.isspace():
            has_code = True
        index += 1
    if has_code:
        count += 1
    return count


def exception_reason(source: str) -> str | None:
    for line in source.splitlines():
        if match := EXCEPTION.search(line):
            return match.group(1).strip()
    return None


def main(root: pathlib.Path) -> int:
    source_root = root / "src"
    files = sorted(source_root.rglob("*.rs")) if source_root.is_dir() else []
    failures: list[str] = []
    if not files:
        print("source-limit: no production Rust files found under src/")
        return 0
    for path in files:
        source = path.read_text(encoding="utf-8")
        lines = code_lines(source)
        relative = path.relative_to(root)
        reason = exception_reason(source)
        if lines > 300:
            failures.append(f"{relative}: {lines} code lines (>300; hard failure)")
        elif lines >= 200 and not reason:
            failures.append(
                f"{relative}: {lines} code lines (200-300 requires an exception)"
            )
        elif lines >= 200:
            print(f"source-limit: {relative}: {lines} lines; exception: {reason}")
        else:
            print(f"source-limit: {relative}: {lines} code lines; pass")
    if failures:
        print("source-limit: FAIL")
        for failure in failures:
            print(f"  {failure}")
        return 1
    print("source-limit: PASS")
    return 0


raise SystemExit(main(pathlib.Path(sys.argv[1])))
PY
