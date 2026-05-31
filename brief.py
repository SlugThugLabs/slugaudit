#!/usr/bin/env python3
"""
brief.py — Briefing Assembler

Queries the PostgreSQL audit database and assembles a structured Markdown
prompt (the "Briefing") designed to be fed directly to an AI for audit.

Usage:
    python3 brief.py --project SLUG-ID [--output briefing.md]
"""

import argparse
import os
import sys
from datetime import datetime, timezone
from typing import Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from db import get_connection, get_changed_files


def fmt_sig(sig: dict) -> str:
    """Format a single signatures entry into a compact line."""
    vis = sig.get("visibility", "")
    vis_str = f"{vis} " if vis else ""
    name = sig.get("name", "")
    sig_text = sig.get("signature", "")
    if sig_text:
        return f"- {vis_str}{sig_text}"
    return f"- {vis_str}{sig.get('type', '?')} {name}"


def assemble_briefing(
    project_name: str = None,
    output_path: Optional[str] = None,
    connection_str: Optional[str] = None,
    max_ghost_lines: int = 500,
):
    """Assemble the audit briefing from the database."""
    conn = None
    try:
        conn = get_connection(connection_str)
        cur = conn.cursor()

        # --- Get project ---
        if project_name:
            cur.execute(
                "SELECT id, name, primary_language, repo_path FROM projects WHERE name = %s",
                (project_name,),
            )
        else:
            cur.execute(
                "SELECT id, name, primary_language, repo_path FROM projects "
                "ORDER BY created_at DESC LIMIT 1"
            )

        row = cur.fetchone()
        if not row:
            print("No project found.")
            return None

        project_id, name, language, repo_path = row

        # --- Get architecture state ---
        architecture_text = ""
        cur.execute(
            "SELECT summary, layer_map FROM architecture_state "
            "WHERE project_id = %s ORDER BY created_at DESC LIMIT 1",
            (project_id,),
        )
        arch_row = cur.fetchone()
        if arch_row:
            summary, layer_map = arch_row
            architecture_text = summary or ""
            if layer_map:
                architecture_text += f"\n\nCrate Structure:\n```\n{layer_map}\n```"

        # --- File stats ---
        cur.execute(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE project_id = %s",
            (project_id,),
        )
        total_files, total_bytes = cur.fetchone()

        cur.execute(
            "SELECT COUNT(*) FROM files WHERE project_id = %s "
            "AND signature_cache IS NOT NULL AND jsonb_array_length(signature_cache) > 0",
            (project_id,),
        )
        files_with_sigs = cur.fetchone()[0]

        cur.execute(
            "SELECT COALESCE(SUM(jsonb_array_length(signature_cache)), 0) "
            "FROM files WHERE project_id = %s AND signature_cache IS NOT NULL",
            (project_id,),
        )
        total_sigs = cur.fetchone()[0]

        # --- Detect changed files ---
        changed_paths_raw = get_changed_files(conn, project_id)
        changed_paths = [r[1] for r in changed_paths_raw]  # (id, path) tuples

        # --- Blast radius: files that depend on changed files ---
        blast_radius_paths = set()
        if changed_paths:
            cur.execute(
                """
                SELECT DISTINCT f2.path
                FROM dependency_edges de
                JOIN files f1 ON f1.id = de.source_file_id
                JOIN files f2 ON f2.id = de.target_file_id
                WHERE de.project_id = %s
                  AND f1.path = ANY(%s)
                """,
                (project_id, changed_paths),
            )
            for (rp,) in cur.fetchall():
                blast_radius_paths.add(rp)

        target_paths = list(set(changed_paths) | blast_radius_paths)
        target_paths.sort()

        # --- Ghost context: signatures of unchanged files ---
        unchanged_with_sigs = []
        if target_paths:
            cur.execute(
                """
                SELECT path, signature_cache
                FROM files
                WHERE project_id = %s
                  AND signature_cache IS NOT NULL
                  AND jsonb_array_length(signature_cache) > 0
                  AND path != ALL(%s)
                ORDER BY path
                """,
                (project_id, target_paths),
            )
        else:
            cur.execute(
                """
                SELECT path, signature_cache
                FROM files
                WHERE project_id = %s
                  AND signature_cache IS NOT NULL
                  AND jsonb_array_length(signature_cache) > 0
                ORDER BY path
                """,
                (project_id,),
            )
        unchanged_with_sigs = cur.fetchall()

        # --- Historical findings ---
        findings_list = []
        cur.execute(
            """
            SELECT f.path, fi.line_start, fi.line_end, fi.severity,
                   fi.category, fi.message
            FROM findings fi
            JOIN files f ON f.id = fi.file_id
            WHERE fi.project_id = %s AND fi.status = 'open'
            ORDER BY fi.created_at DESC
            LIMIT 50
            """,
            (project_id,),
        )
        findings_list = cur.fetchall()

        # --- Count dependency edges ---
        cur.execute(
            "SELECT COUNT(*) FROM dependency_edges WHERE project_id = %s",
            (project_id,),
        )
        edge_count = cur.fetchone()[0]

        cur.execute(
            "SELECT COUNT(*) FROM file_imports WHERE project_id = %s",
            (project_id,),
        )
        import_count = cur.fetchone()[0]

        # ============================================================
        # ASSEMBLE THE BRIEFING
        # ============================================================
        lines = []
        timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")

        # Header
        scope = "full" if not changed_paths else "incremental"
        lines.append(f"# Audit Briefing — {name} (Phase 1, {scope.capitalize()})")
        lines.append(f"")
        lines.append(f"Generated: {timestamp}")
        lines.append(f"Project: {name} ({language})")
        lines.append(f"")

        # Overview
        lines.append(f"## Project Overview")
        lines.append(f"")
        lines.append(f"| Metric | Value |")
        lines.append(f"|--------|-------|")
        lines.append(f"| Language | {language} |")
        lines.append(f"| Total files | {total_files} |")
        lines.append(f"| Total size | {total_bytes/1024:.0f} KB |")
        lines.append(f"| Files with signatures | {files_with_sigs} |")
        lines.append(f"| Total signatures | {total_sigs} |")
        lines.append(f"| Imports tracked | {import_count} |")
        lines.append(f"| Dependency edges | {edge_count} |")
        lines.append(f"| Changed files | {len(changed_paths)} |")
        lines.append(f"| Blast radius files | {len(blast_radius_paths)} |")
        lines.append(f"| Open findings | {len(findings_list)} |")
        lines.append(f"")

        # Architecture
        lines.append(f"## Architecture")
        lines.append(f"")
        if architecture_text:
            lines.append(architecture_text)
        else:
            lines.append(f"Hexagonal Architecture (Ports and Adapters). {language} project.")
        lines.append(f"")

        # Ghost Context
        lines.append(f"## GHOST CONTEXT — Unchanged Files ({len(unchanged_with_sigs)} files, {total_sigs} signatures)")
        lines.append(f"")
        lines.append(f"The following files are unchanged. Their public API signatures are provided ")
        lines.append(f"for reference. **Do NOT read these files** — use the signatures below.")
        lines.append(f"")

        ghost_line_count = 0
        file_index = 0
        for fpath, sig_cache in unchanged_with_sigs:
            if ghost_line_count >= max_ghost_lines:
                remaining = len(unchanged_with_sigs) - file_index
                lines.append(f"")
                lines.append(f"*[... {remaining} more files omitted for brevity ...]*")
                break
            file_index += 1
            crate = fpath.split("/")[0] if "/" in fpath else "root"
            lines.append(f"")
            lines.append(f"### {fpath}")
            for sig in sig_cache:
                s = fmt_sig(sig)
                lines.append(f"{s}")
                ghost_line_count += 1
                if ghost_line_count >= max_ghost_lines:
                    break

        lines.append(f"")

        # Target Files (Changed + Blast Radius)
        lines.append(f"## TARGET FILES — Audit These")
        lines.append(f"")
        lines.append(f"{len(target_paths)} file(s) to audit:")
        lines.append(f"")
        if changed_paths:
            lines.append(f"**Changed files:**")
            for fp in changed_paths:
                lines.append(f"- `{fp}` (CHANGED)")
            lines.append(f"")
        if blast_radius_paths:
            lines.append(f"**Blast radius (dependents):**")
            for fp in sorted(blast_radius_paths):
                lines.append(f"- `{fp}` (BLAST RADIUS)")
            lines.append(f"")

        # File size limit: 1 MB
        MAX_FILE_SIZE = 1 * 1024 * 1024  # 1 MB

        # Canonicalize repo_path once for path traversal checks
        canonical_repo = os.path.realpath(repo_path)

        # Full source code of target files
        for fp in target_paths:
            abs_path = os.path.join(repo_path, fp)
            # Path traversal protection: canonicalize and verify within repo
            canonical_abs = os.path.realpath(abs_path)
            if not os.path.commonpath([canonical_abs, canonical_repo]) == canonical_repo:
                lines.append(f"### {fp}")
                lines.append(f"*Skipped: path traversal detected*")
                lines.append(f"")
                continue
            lines.append(f"### {fp}")
            label = "CHANGED" if fp in changed_paths else "BLAST RADIUS"
            lines.append(f"*Status: {label}*")
            lines.append(f"")
            lines.append(f"```{language}")
            try:
                # Check file size before reading
                file_size = os.path.getsize(abs_path)
                if file_size > MAX_FILE_SIZE:
                    lines.append(f"// File skipped: size {file_size} bytes exceeds {MAX_FILE_SIZE} byte limit")
                else:
                    with open(abs_path, 'r') as f:
                        code = f.read()
                    lines.append(code.rstrip())
            except Exception as e:
                lines.append(f"// Error reading file: {e}")
            lines.append(f"```")
            lines.append(f"")

        # Historical Findings
        lines.append(f"## Historical Findings")
        lines.append(f"")
        if findings_list:
            for fpath, ls, le, sev, cat, msg in findings_list:
                short_title = msg[:100] if msg else "No message"
                if len(msg) > 100:
                    short_title += "..."
                lines.append(f"- **[{sev.upper()}]** `{fpath}:{ls}-{le}` — {short_title}")
                lines.append(f"")
        else:
            lines.append(f"No open findings on record.")
            lines.append(f"")

        # Output Contract
        lines.append(f"## Output Contract")
        lines.append(f"")
        lines.append(f"Return findings in this exact JSON format:")
        lines.append(f"")
        lines.append(f"```json")
        lines.append(f"{{")
        lines.append(f"  \"findings\": [")
        lines.append(f"    {{")
        lines.append(f"      \"file\": \"path/to/file.rs\",")
        lines.append(f"      \"line_start\": 42,")
        lines.append(f"      \"line_end\": 55,")
        lines.append(f"      \"severity\": \"high\",")
        lines.append(f"      \"category\": \"correctness\",")
        lines.append(f"      \"title\": \"Short descriptive title\",")
        lines.append(f"      \"description\": \"Full explanation of the issue...\"")
        lines.append(f"    }}")
        lines.append(f"  ]")
        lines.append(f"}}")
        lines.append(f"```")
        lines.append(f"")

        briefing = "\n".join(lines)

        # Output
        if output_path:
            with open(output_path, 'w') as f:
                f.write(briefing)
            print(f"Briefing written to {output_path}")
            print(f"  {total_files} files, {total_sigs} signatures, {len(target_paths)} targets")
            print(f"  Ghost context: {len(unchanged_with_sigs)} files")
            print(f"  Briefing size: {len(briefing)} chars / ~{len(briefing.split())} words")

        return briefing

    finally:
        if conn:
            conn.close()


def main():
    parser = argparse.ArgumentParser(
        description="Briefing Assembler — generate audit briefing from the database",
    )
    parser.add_argument(
        "--project", "-p",
        help="Project name (default: latest)",
    )
    parser.add_argument(
        "--output", "-o",
        help="Output file path (default: print to stdout)",
    )
    parser.add_argument(
        "--connection", "-c",
        default=None,
        help="PostgreSQL connection string "
             "(default: uses PGHOST/PGPORT/PGDATABASE/PGUSER/PGPASSWORD env vars)",
    )
    parser.add_argument(
        "--max-ghost-lines", type=int, default=500,
        help="Maximum ghost context lines (default: 500)",
    )

    args = parser.parse_args()
    briefing = assemble_briefing(
        project_name=args.project,
        output_path=args.output,
        connection_str=args.connection,
        max_ghost_lines=args.max_ghost_lines,
    )
    if not args.output and briefing:
        print(briefing)


if __name__ == "__main__":
    main()
