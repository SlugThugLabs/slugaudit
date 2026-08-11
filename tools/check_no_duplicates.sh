#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
project_root="$(CDPATH= cd -- "$script_dir/.." && pwd)"

if [[ "${1:-}" == "--help" ]]; then
    printf '%s\n' "Checks for duplicated commit subjects and duplicated test names."
    exit 0
fi

if [[ "${1:-}" == "--root" ]]; then
    project_root="${2:?--root requires a project directory}"
fi

exec python3 - "$project_root" <<'PY'
from __future__ import annotations

import pathlib
import re
import subprocess
import sys


TEST_ATTR = re.compile(r"^\s*#\[(?:[a-z_:]+::)?test\]\s*(.*?)\s*$")
TEST_FN = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+([a-z0-9_]+)\b")
SAME_LINE_FN = re.compile(r"(?:pub\s+)?(?:async\s+)?fn\s+([a-z0-9_]+)\b")


def duplicate_commit_subjects(root: pathlib.Path) -> list[tuple[str, int]]:
    """Return (subject, count) pairs for commit subjects used more than once."""
    try:
        proc = subprocess.run(
            ["git", "log", "--format=%s"],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
    except FileNotFoundError:
        print("no-duplicates: git not found; skipping commit-subject check")
        return []
    if proc.returncode != 0:
        print("no-duplicates: not a git repository; skipping commit-subject check")
        return []
    counts: dict[str, int] = {}
    for line in proc.stdout.splitlines():
        subject = line.strip()
        if subject:
            counts[subject] = counts.get(subject, 0) + 1
    return [(subject, count) for subject, count in counts.items() if count > 1]


def test_names_by_file(paths: list[pathlib.Path]) -> dict[str, list[pathlib.Path]]:
    """Map each #[test]-attributed function name to the files defining it."""
    names: dict[str, list[pathlib.Path]] = {}
    for path in paths:
        lines = path.read_text(encoding="utf-8").splitlines()
        for index, line in enumerate(lines):
            attr = TEST_ATTR.match(line)
            if not attr:
                continue
            # Same-line form: `#[test] fn foo() { ... }`. The attribute
            # regex deliberately matches only `#[test]`-style attributes
            # (optionally path-qualified like `#[tokio::test]`), never
            # `#[cfg(test)]`, so a cfg-gated helper fn is not mistaken
            # for a test.
            inline = SAME_LINE_FN.search(attr.group(1))
            if inline:
                names.setdefault(inline.group(1), []).append(path)
                continue
            # Skip blank lines, comments, and further attributes that may
            # sit between the attribute and the `fn` item.
            next_index = index + 1
            while next_index < len(lines):
                stripped = lines[next_index].strip()
                if not stripped or stripped.startswith("//") or stripped.startswith("#["):
                    next_index += 1
                    continue
                break
            match = TEST_FN.match(lines[next_index]) if next_index < len(lines) else None
            if match:
                names.setdefault(match.group(1), []).append(path)
    return names


def main(root: pathlib.Path) -> int:
    failures: list[str] = []

    for subject, count in duplicate_commit_subjects(root):
        failures.append(f"commit subject {subject!r} used {count} times")
        print(f"no-duplicates: commit subject {subject!r} used {count} times")

    source_root = root / "src"
    # Scan every Rust file under src/: test functions live in dedicated
    # *_tests.rs modules *and* in inline #[cfg(test)] mod blocks (e.g.
    # server_runner.rs, connect.rs, parse/language.rs). #[cfg(test)]
    # module attributes are ignored naturally: the line after them is
    # `mod`, which the fn pattern does not match.
    test_files = sorted(source_root.rglob("*.rs")) if source_root.is_dir() else []
    integration_root = root / "tests"
    if integration_root.is_dir():
        test_files.extend(sorted(integration_root.glob("*.rs")))
    names = test_names_by_file(test_files)
    for name, files in sorted(names.items()):
        if len(files) > 1:
            joined = ", ".join(str(path.relative_to(root)) for path in files)
            failures.append(f"test {name!r} defined in {len(files)} files: {joined}")
            print(f"no-duplicates: test {name!r} defined in {len(files)} files: {joined}")

    if failures:
        print("no-duplicates: FAIL")
        return 1
    print("no-duplicates: PASS")
    return 0


raise SystemExit(main(pathlib.Path(sys.argv[1])))
PY
