#!/usr/bin/env python3
"""
audit_db Audit Database CLI

Populate and query the audit database for any project.

Usage:
    audit_db init-db [--connection CONN]
    audit_db import /path/to/project [options]
    audit_db status [options]
    audit_db changed [options]
    audit_db briefing [options]
    audit_db list
"""

import argparse
import os
import sys
from datetime import datetime, timezone

try:
    import psycopg2
except ImportError:
    print("Error: psycopg2 is required. Install with: pip install psycopg2-binary")
    sys.exit(1)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from db import (
    get_connection,
    get_or_create_project,
    upsert_file,
    delete_removed_files,
    insert_imports,
    build_dependency_edges,
    get_changed_files,
    get_project_names,
    schema_exists,
    update_audit_timestamps,
)
from core import get_extractor, import_project
from languages import detect_language, list_languages
from brief import assemble_briefing


def cmd_init_db(args):
    """Initialize the audit database schema."""
    conn = None
    try:
        conn = get_connection(args.connection)
        schema_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "schema.sql")
        if not os.path.exists(schema_path):
            print(f"Error: schema.sql not found at {schema_path}")
            sys.exit(1)

        with open(schema_path, "r") as f:
            schema_sql = f.read()

        statements = [s.strip() for s in schema_sql.split(";") if s.strip()]
        cur = conn.cursor()
        for stmt in statements:
            try:
                cur.execute(stmt)
            except Exception as e:
                err_str = str(e).lower()
                if "already exists" not in err_str and "duplicate" not in err_str:
                    print(f"  Warning: {e}")
        conn.commit()
        cur.close()

        print("Database schema initialized successfully.")
        print(f"  Applied schema from: {schema_path}")
    finally:
        if conn:
            conn.close()


def cmd_import(args):
    """Import a project into the audit database."""
    project_root = os.path.abspath(args.path)

    if not os.path.isdir(project_root):
        print(f"Error: not a directory: {project_root}")
        sys.exit(1)

    # Auto-initialize schema if missing — zero-config first use
    conn = get_connection(args.connection)
    if not schema_exists(conn):
        print("Database schema not found. Initializing automatically...")
        cmd_init_db(args)
    conn.close()

    def on_progress(processed, total, error=None):
        if error:
            print(f"  Warning: error processing file {processed}/{total}: {error}")
        elif processed % 50 == 0 or processed == total:
            print(f"  Processed {processed}/{total} files...")

    try:
        result = import_project(
            project_path=project_root,
            project_name=args.project_name,
            language=args.language,
            connection_string=args.connection,
            on_progress=on_progress,
        )
        print(f"\nProject: {result.project_name} ({result.language})")
        print(f"Project ID: {result.project_id}")
        print(f"\n{'='*50}")
        print("IMPORT COMPLETE")
        print(f"  Files processed: {result.files_processed}")
        print(f"  Signatures extracted: {result.signatures_extracted}")
        print(f"  Imports extracted: {result.imports_extracted}")
        print(f"  Dependency edges: {result.dependency_edges}")
        print(f"  Elapsed: {result.elapsed_seconds:.1f}s")
        print(f"{'='*50}")
    except ValueError as e:
        print(f"Error: {e}")
        sys.exit(1)


def cmd_status(args):
    """Show project status from the audit database."""
    conn = None
    try:
        conn = get_connection(args.connection)
        cur = conn.cursor()
        if args.project:
            cur.execute(
                "SELECT id, name, primary_language, repo_path, created_at "
                "FROM projects WHERE name = %s",
                (args.project,),
            )
        else:
            cur.execute(
                "SELECT id, name, primary_language, repo_path, created_at "
                "FROM projects ORDER BY created_at DESC"
            )

        rows = cur.fetchall()
        if not rows:
            print("No projects found.")
            return

        for row in rows:
            pid, name, lang, repo_path, created = row
            print(f"\n=== {name} ===")
            print(f"  ID: {pid}")
            print(f"  Language: {lang}")
            print(f"  Path: {repo_path}")
            print(f"  Created: {created}")

            cur.execute(
                "SELECT COUNT(*), COALESCE(SUM(size), 0), "
                "COUNT(*) FILTER (WHERE signature_cache IS NOT NULL "
                "AND jsonb_array_length(signature_cache) > 0) "
                "FROM files WHERE project_id = %s",
                (pid,),
            )
            fc, total_size, with_sigs = cur.fetchone()
            total_sigs = 0
            if with_sigs:
                cur.execute(
                    "SELECT SUM(jsonb_array_length(signature_cache)) "
                    "FROM files WHERE project_id = %s "
                    "AND signature_cache IS NOT NULL",
                    (pid,),
                )
                total_sigs = cur.fetchone()[0] or 0

            print(f"  Files: {fc}")
            print(f"  Total size: {total_size/1024:.0f}KB")
            print(f"  With signatures: {with_sigs}")
            print(f"  Total signatures: {total_sigs}")

            changed = get_changed_files(conn, pid)
            if changed:
                print(f"  Changed since last audit: {len(changed)}")
                for fid, fpath in changed[:10]:
                    print(f"    - {fpath}")
                if len(changed) > 10:
                    print(f"    ... and {len(changed)-10} more")

            cur.execute(
                "SELECT COUNT(*), status FROM findings "
                "WHERE project_id = %s GROUP BY status",
                (pid,),
            )
            findings = cur.fetchall()
            if findings:
                print("  Findings:")
                for cnt, status in findings:
                    print(f"    {status}: {cnt}")

            cur.execute(
                "SELECT COUNT(*) FROM file_imports WHERE project_id = %s",
                (pid,),
            )
            imp_count = cur.fetchone()[0]
            print(f"  Imports tracked: {imp_count}")

            cur.execute(
                "SELECT COUNT(*) FROM dependency_edges WHERE project_id = %s",
                (pid,),
            )
            edge_count = cur.fetchone()[0]
            print(f"  Dependency edges: {edge_count}")

    finally:
        if conn:
            conn.close()


def cmd_briefing(args):
    """Generate an audit briefing for the AI."""
    assemble_briefing(
        project_name=args.project,
        output_path=args.output,
        connection_str=args.connection,
        max_ghost_lines=args.max_ghost_lines,
    )


def cmd_changed(args):
    """List files changed since last audit."""
    conn = None
    try:
        conn = get_connection(args.connection)
        cur = conn.cursor()
        if args.project:
            cur.execute("SELECT id FROM projects WHERE name = %s", (args.project,))
            row = cur.fetchone()
            if not row:
                print(f"Project not found: {args.project}")
                return
            project_id = row[0]
        else:
            cur.execute("SELECT id, name FROM projects ORDER BY created_at DESC LIMIT 1")
            row = cur.fetchone()
            if not row:
                print("No projects found.")
                return
            project_id, name = row
            print(f"Project: {name}")

        changed = get_changed_files(conn, project_id)
        if changed:
            print(f"\n{len(changed)} changed file(s):")
            for fid, fpath in changed:
                print(f"  {fpath}")
        else:
            print("\nNo files changed since last audit.")

    finally:
        if conn:
            conn.close()


def cmd_list(args):
    """List all projects in the audit database."""
    conn = None
    try:
        conn = get_connection(args.connection)
        names = get_project_names(conn)
        if names:
            print("Projects in audit database:")
            for n in names:
                print(f"  - {n}")
        else:
            print("No projects found.")
    finally:
        if conn:
            conn.close()


def main():
    parser = argparse.ArgumentParser(
        description="Audit Database - populate and query for any project",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  audit_db init-db
  audit_db import . --project-name "My Project"
  audit_db import /path/to/project --language rust
  audit_db status --project SLUG-ID
  audit_db changed --project SLUG-ID
  audit_db list
        """,
    )
    parser.add_argument(
        "--connection", "-c",
        default=None,
        help="PostgreSQL connection string "
             "(e.g. postgresql://user:pass@host:5432/dbname; "
             "default: PGHOST/PGPORT/PGDATABASE/PGUSER/PGPASSWORD env vars)",
    )

    subparsers = parser.add_subparsers(dest="command", help="Available commands")

    subparsers.add_parser("init-db", help="Initialize the database schema")

    import_parser = subparsers.add_parser(
        "import", help="Import a project into the audit database"
    )
    import_parser.add_argument("path", help="Path to the project directory")
    import_parser.add_argument(
        "--project-name", "-n",
        help="Project name (default: directory name)",
    )
    import_parser.add_argument(
        "--language", "-l",
        choices=["auto", "rust", "python", "typescript"],
        default="auto",
        help="Language (default: auto-detect)",
    )

    status_parser = subparsers.add_parser("status", help="Show project status")
    status_parser.add_argument(
        "--project", "-p",
        help="Project name (default: latest)",
    )

    changed_parser = subparsers.add_parser("changed", help="List changed files since last audit")
    changed_parser.add_argument(
        "--project", "-p",
        help="Project name (default: latest)",
    )

    briefing_parser = subparsers.add_parser(
        "briefing", help="Generate an audit briefing for the AI"
    )
    briefing_parser.add_argument(
        "--project", "-p",
        help="Project name (default: latest)",
    )
    briefing_parser.add_argument(
        "--output", "-o",
        help="Output briefing file path (default: stdout)",
    )
    briefing_parser.add_argument(
        "--max-ghost-lines", type=int, default=500,
        help="Maximum ghost context lines (default: 500)",
    )

    subparsers.add_parser("list", help="List all projects")

    args = parser.parse_args()

    if args.command == "init-db":
        cmd_init_db(args)
    elif args.command == "import":
        cmd_import(args)
    elif args.command == "status":
        cmd_status(args)
    elif args.command == "changed":
        cmd_changed(args)
    elif args.command == "briefing":
        cmd_briefing(args)
    elif args.command == "list":
        cmd_list(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
