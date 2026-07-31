"""Tests for services/sqlite_migration.py.

pg_conn is a real sqlite3.Connection standing in for what would be a real
psycopg2 connection in production — the migration function only ever calls
repositories/__init__.py's make_*_repository() factories against it, which
dispatch purely on isinstance(conn, sqlite3.Connection), so a second SQLite
connection genuinely exercises the same code path the migration logic itself
is responsible for (path resolution, identity_hash reuse, upsert dedup).
What it does *not* exercise is anything psycopg2-specific, which the
migration code never touches directly anyway — see the module docstring.
"""

import sqlite3
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from infrastructure.sqlite_db import _regexp, sqlite_db_path
from repositories import make_file_repository, make_finding_repository, make_project_repository
from repositories import repository_transaction
from services.sqlite_migration import migrate_sqlite_findings
from services.sqlite_schema_service import SqliteSchemaService


def _new_connection() -> sqlite3.Connection:
    conn = sqlite3.connect(":memory:")
    conn.execute("PRAGMA foreign_keys = ON")
    conn.create_function("REGEXP", 2, _regexp)
    SqliteSchemaService().initialize(conn)
    return conn


class TestMigrateSqliteFindings(unittest.TestCase):
    def test_no_op_when_no_legacy_database_exists(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".planning" / "slugaudit").mkdir(parents=True)
            pg_conn = _new_connection()
            migrated = migrate_sqlite_findings(root, "project-1", pg_conn)
            self.assertEqual(migrated, 0)

    def test_migrates_finding_for_a_file_that_still_exists(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".planning" / "slugaudit").mkdir(parents=True)

            # Populate the "old" local SQLite database directly at the real
            # per-project path, exactly as the SQLite backend would have.
            old_conn = sqlite3.connect(str(sqlite_db_path(root)))
            old_conn.execute("PRAGMA foreign_keys = ON")
            SqliteSchemaService().initialize(old_conn)
            old_project_repo = make_project_repository(old_conn, auto_commit=False)
            old_file_repo = make_file_repository(old_conn, auto_commit=False)
            old_finding_repo = make_finding_repository(old_conn)  # auto_commit=True
            with repository_transaction(old_conn):
                old_project_id = old_project_repo.get_or_create(
                    name="demo", language="python", repo_path=str(root)
                )
                old_file_id, _ = old_file_repo.upsert(
                    project_id=old_project_id, relpath="a.py", file_hash="old-hash",
                    file_size=1, mtime="2026-01-01T00:00:00Z", signatures=[],
                    content="x = 1\n", force=True,
                )
            old_finding_repo.record(
                project_id=old_project_id, file_id=old_file_id, identity_hash="ih1",
                severity="high", category="security", message="title: desc",
                line_start=1, line_end=1,
            )
            old_conn.close()

            # "PostgreSQL" (a second, independent SQLite connection) already
            # has a fresh sync's files row for the same path/project.
            pg_conn = _new_connection()
            pg_project_repo = make_project_repository(pg_conn, auto_commit=False)
            pg_file_repo = make_file_repository(pg_conn, auto_commit=False)
            with repository_transaction(pg_conn):
                pg_project_id = pg_project_repo.get_or_create(
                    name="demo", language="python", repo_path=str(root)
                )
                pg_file_repo.upsert(
                    project_id=pg_project_id, relpath="a.py", file_hash="new-hash",
                    file_size=1, mtime="2026-01-01T00:00:00Z", signatures=[],
                    content="x = 1\n", force=True,
                )

            migrated = migrate_sqlite_findings(root, pg_project_id, pg_conn)
            self.assertEqual(migrated, 1)

            pg_finding_repo = make_finding_repository(pg_conn)
            findings = pg_finding_repo.get_open_findings(pg_project_id)
            self.assertEqual(len(findings), 1)
            self.assertEqual(findings[0][0], "a.py")
            self.assertEqual(findings[0][3], "high")
            self.assertEqual(findings[0][5], "title: desc")

            # The old file is renamed, not deleted, and not left in place
            # under its original name (so this doesn't re-run every call).
            self.assertFalse(sqlite_db_path(root).exists())
            self.assertTrue(sqlite_db_path(root).with_name("audit.db.migrated").exists())

    def test_finding_for_a_deleted_file_is_not_migrated(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".planning" / "slugaudit").mkdir(parents=True)

            old_conn = sqlite3.connect(str(sqlite_db_path(root)))
            old_conn.execute("PRAGMA foreign_keys = ON")
            SqliteSchemaService().initialize(old_conn)
            old_project_repo = make_project_repository(old_conn, auto_commit=False)
            old_file_repo = make_file_repository(old_conn, auto_commit=False)
            old_finding_repo = make_finding_repository(old_conn)  # auto_commit=True
            with repository_transaction(old_conn):
                old_project_id = old_project_repo.get_or_create(
                    name="demo", language="python", repo_path=str(root)
                )
                old_file_id, _ = old_file_repo.upsert(
                    project_id=old_project_id, relpath="deleted.py", file_hash="h",
                    file_size=1, mtime="2026-01-01T00:00:00Z", signatures=[],
                    content="x = 1\n", force=True,
                )
            old_finding_repo.record(
                project_id=old_project_id, file_id=old_file_id, identity_hash="ih1",
                severity="high", category="security", message="stale finding",
                line_start=1, line_end=1,
            )
            old_conn.close()

            # "PostgreSQL" has no files at all — deleted.py no longer exists.
            pg_conn = _new_connection()
            pg_project_repo = make_project_repository(pg_conn, auto_commit=False)
            with repository_transaction(pg_conn):
                pg_project_id = pg_project_repo.get_or_create(
                    name="demo", language="python", repo_path=str(root)
                )

            migrated = migrate_sqlite_findings(root, pg_project_id, pg_conn)
            self.assertEqual(migrated, 0)

    def test_rerunning_after_migration_is_a_safe_no_op(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".planning" / "slugaudit").mkdir(parents=True)
            pg_conn = _new_connection()

            self.assertEqual(migrate_sqlite_findings(root, "project-1", pg_conn), 0)
            # No legacy db, no .migrated marker either — nothing to do, twice.
            self.assertEqual(migrate_sqlite_findings(root, "project-1", pg_conn), 0)


if __name__ == "__main__":
    unittest.main()
