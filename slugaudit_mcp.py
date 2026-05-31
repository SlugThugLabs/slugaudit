#!/usr/bin/env python3
"""
slugaudit-mcp Audit Database CLI

Populate and query the audit database for any project.

Usage:
    slugaudit-mcp init-db [--connection CONN]
    slugaudit-mcp import /path/to/project [options]
    slugaudit-mcp status [options]
    slugaudit-mcp changed [options]
    slugaudit-mcp briefing [options]
    slugaudit-mcp list
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

from infrastructure import (
    get_connection,
    validate_project_name,
    validate_project_path,
)
from services import SchemaService, ImportService
from services.import_service import import_project
from briefing import assemble_briefing
from repositories import ProjectRepository, FileRepository
from languages import get_extractor, detect_language, list_languages


_schema_service = SchemaService()


def cmd_init_db(args):
    """Initialize the audit database schema."""
    conn = None
    try:
        conn = get_connection(args.connection)
        _schema_service.initialize(conn)
        print("Database schema initialized successfully.")
    finally:
        if conn:
            conn.close()


def cmd_import(args):
    """Import a project into the audit database."""
    project_root = validate_project_path(args.path)

    # Validate project name if provided
    if args.project_name:
        validate_project_name(args.project_name)

    if not os.path.isdir(project_root):
        print(f"Error: not a directory: {project_root}")
        sys.exit(1)

    # Auto-initialize schema if missing — zero-config first use
    conn = get_connection(args.connection)
    repo = ProjectRepository(conn)
    if not repo.schema_exists():
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
        project_repo = ProjectRepository(conn)
        file_repo = FileRepository(conn)

        if args.project:
            row = project_repo.get_by_name(args.project)
        else:
            row = project_repo.get_latest()

        if not row:
            print("No projects found.")
            return

        # Could be multiple if no project specified
        rows = [row] if args.project else project_repo.get_all()

        for row in rows:
            pid, name, lang, repo_path, *rest = row
            created = rest[0] if rest else None
            print(f"\n=== {name} ===")
            print(f"  ID: {pid}")
            print(f"  Language: {lang}")
            print(f"  Path: {repo_path}")
            if created:
                print(f"  Created: {created}")

            stats = project_repo.get_status(pid)
            print(f"  Files: {stats['file_count']}")
            print(f"  Total size: {stats['total_size']/1024:.0f}KB")
            print(f"  With signatures: {stats['files_with_sigs']}")
            print(f"  Total signatures: {stats['signatures_count']}")

            changed = file_repo.get_changed(pid)
            if changed:
                print(f"  Changed since last audit: {len(changed)}")
                for fid, fpath in changed[:10]:
                    print(f"    - {fpath}")
                if len(changed) > 10:
                    print(f"    ... and {len(changed)-10} more")

            findings = project_repo.get_findings_summary(pid)
            if findings:
                print("  Findings:")
                for cnt, status in findings:
                    print(f"    {status}: {cnt}")

            print(f"  Imports tracked: {stats['imports_count']}")
            print(f"  Dependency edges: {stats['edge_count']}")

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
        project_repo = ProjectRepository(conn)
        file_repo = FileRepository(conn)

        if args.project:
            row = project_repo.get_by_name(args.project)
            if not row:
                print(f"Project not found: {args.project}")
                return
            project_id = row[0]
        else:
            row = project_repo.get_latest()
            if not row:
                print("No projects found.")
                return
            project_id, name = row[0], row[1]
            print(f"Project: {name}")

        changed = file_repo.get_changed(project_id)
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
        project_repo = ProjectRepository(conn)
        names = project_repo.get_names()
        if names:
            print("Projects in audit database:")
            for n in names:
                print(f"  - {n}")
        else:
            print("No projects found.")
    finally:
        if conn:
            conn.close()


# Add schema_exists helper to ProjectRepository for backward compat
def _repo_schema_exists(self):
    """Check if the schema has been initialized."""
    cur = self._cursor()
    try:
        cur.execute(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables "
            "WHERE table_name = 'projects')"
        )
        result = cur.fetchone()[0]
        cur.close()
        return result
    except Exception:
        cur.close()
        return False

ProjectRepository.schema_exists = _repo_schema_exists


def main():
    parser = argparse.ArgumentParser(
        description="Audit Database - populate and query for any project",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  slugaudit-mcp init-db
  slugaudit-mcp import . --project-name "My Project"
  slugaudit-mcp import /path/to/project --language rust
  slugaudit-mcp status --project SLUG-ID
  slugaudit-mcp changed --project SLUG-ID
  slugaudit-mcp list
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
        choices=["auto", "rust", "python", "typescript", "go", "java", "c", "cpp", "ruby"],
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
