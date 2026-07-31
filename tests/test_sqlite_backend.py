"""Tests for the zero-config SQLite backend.

Unlike tests/test_integration_db.py (gated behind SLUGAUDIT_RUN_DB_TESTS
because it needs a real network Postgres), these run unconditionally: SQLite
is a Python stdlib module, so real (non-mocked) database behavior can be
tested for free, with no live server and no gating.
"""

import sqlite3
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from infrastructure.sqlite_db import _regexp, connect, sqlite_db_path
from repositories import (
    make_file_repository,
    make_finding_repository,
    make_import_repository,
    make_project_repository,
    make_risk_pattern_repository,
    repository_transaction,
)
from repositories.sqlite import (
    SqliteFileRepository,
    SqliteFindingRepository,
    SqliteImportRepository,
    SqliteProjectRepository,
    SqliteRiskPatternRepository,
)
from services.sqlite_schema_service import SqliteSchemaService


def _connection() -> sqlite3.Connection:
    conn = sqlite3.connect(":memory:")
    conn.execute("PRAGMA foreign_keys = ON")
    conn.create_function("REGEXP", 2, _regexp)
    SqliteSchemaService().initialize(conn)
    return conn


class TestSqliteDbPath(unittest.TestCase):
    def test_path_lives_inside_activation_directory(self) -> None:
        path = sqlite_db_path("/some/project")
        self.assertEqual(path, Path("/some/project/.planning/slugaudit/audit.db"))

    def test_connect_requires_no_pre_existing_file(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".planning" / "slugaudit").mkdir(parents=True)
            conn = connect(root)
            try:
                conn.execute("SELECT 1")
            finally:
                conn.close()
            self.assertTrue(sqlite_db_path(root).exists())


class TestRegexpFunction(unittest.TestCase):
    def test_matches(self) -> None:
        self.assertTrue(_regexp(r"ev\w+", "eval(x)"))

    def test_no_match(self) -> None:
        self.assertFalse(_regexp(r"^nomatch$", "eval(x)"))

    def test_none_text_is_safe(self) -> None:
        self.assertFalse(_regexp(r"anything", None))

    def test_catastrophic_pattern_times_out_instead_of_hanging(self) -> None:
        # Mirrors the timeout backstop in app/handlers.py:handle_search.
        result = _regexp(r"(a+)+$", "a" * 40 + "!")
        self.assertFalse(result)


class TestSqliteSchemaService(unittest.TestCase):
    def test_initialize_creates_all_seven_core_tables_plus_migrations(self) -> None:
        conn = _connection()
        cur = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
        )
        tables = {row[0] for row in cur.fetchall()}
        self.assertEqual(
            tables,
            {
                "dependency_edges",
                "file_imports",
                "files",
                "findings",
                "project_revisions",
                "projects",
                "risk_patterns",
                "schema_migrations",
            },
        )
        self.assertTrue(SqliteSchemaService().is_current(conn))

    def test_initialize_is_idempotent(self) -> None:
        conn = _connection()
        self.assertTrue(SqliteSchemaService().initialize(conn))
        self.assertTrue(SqliteSchemaService().initialize(conn))


class TestFactoryDispatch(unittest.TestCase):
    def test_sqlite_connection_selects_sqlite_repositories(self) -> None:
        conn = _connection()
        self.assertIsInstance(make_project_repository(conn), SqliteProjectRepository)
        self.assertIsInstance(make_file_repository(conn), SqliteFileRepository)
        self.assertIsInstance(make_import_repository(conn), SqliteImportRepository)
        self.assertIsInstance(make_finding_repository(conn), SqliteFindingRepository)
        self.assertIsInstance(
            make_risk_pattern_repository(conn), SqliteRiskPatternRepository
        )

    def test_non_sqlite_connection_selects_postgres_repositories(self) -> None:
        from repositories import FileRepository, ProjectRepository

        fake_pg_conn = object()
        self.assertIsInstance(make_project_repository(fake_pg_conn), ProjectRepository)
        self.assertIsInstance(make_file_repository(fake_pg_conn), FileRepository)


class TestSqliteRepositoryRoundTrip(unittest.TestCase):
    """End-to-end through the repository layer against a real SQLite db."""

    def setUp(self) -> None:
        self.conn = _connection()

    def tearDown(self) -> None:
        self.conn.close()

    def test_full_project_lifecycle(self) -> None:
        project_repo = make_project_repository(self.conn, auto_commit=False)
        file_repo = make_file_repository(self.conn, auto_commit=False)
        import_repo = make_import_repository(self.conn, auto_commit=False)
        risk_repo = make_risk_pattern_repository(self.conn, auto_commit=False)

        with repository_transaction(self.conn):
            project_id = project_repo.get_or_create(
                name="demo", language="python", repo_path="/tmp/demo"
            )
            file_id, created = file_repo.upsert(
                project_id=project_id,
                relpath="a.py",
                file_hash="hash1",
                file_size=42,
                mtime="2026-01-01T00:00:00Z",
                signatures=[{"type": "fn", "name": "foo"}],
                content="def foo(): pass\n",
                force=True,
            )
            self.assertTrue(created)
            import_repo.insert(
                project_id,
                file_id,
                [{"import_text": "import os", "import_type": "external"}],
                force=True,
            )
            risk_repo.upsert(project_id, file_id, [{"pattern_type": "eval", "count": 2}])
            revision_id = project_repo.begin_revision(
                project_id, manifest_hash="m1", file_count=1, signature_count=1,
                parser_version="v1",
            )
            project_repo.publish_revision(project_id, revision_id)

        status = project_repo.get_status(project_id)
        self.assertEqual(status["file_count"], 1)
        self.assertEqual(status["total_size"], 42)
        self.assertEqual(status["files_with_sigs"], 1)
        self.assertEqual(status["imports_count"], 1)

        self.assertEqual(file_repo.get_manifest(project_id), {"a.py": "hash1"})

        stats = file_repo.get_file_stats(project_id)
        self.assertEqual(stats["total_files"], 1)
        self.assertEqual(stats["total_bytes"], 42)
        self.assertIs(type(stats["total_bytes"]), int)  # never Decimal-shaped

        revision = project_repo.get_current_revision(project_id)
        assert revision is not None
        self.assertEqual(revision["manifest_hash"], "m1")

        self.assertEqual(
            file_repo.search_by_pattern(project_id, "foo", False, 10),
            [("a.py", "def foo(): pass\n")],
        )
        self.assertEqual(
            file_repo.search_by_pattern(project_id, r"def\s+\w+", True, 10),
            [("a.py", "def foo(): pass\n")],
        )
        self.assertEqual(
            file_repo.get_file_contents(project_id, ["a.py"]),
            [("a.py", "def foo(): pass\n")],
        )
        self.assertEqual(
            file_repo.get_file_identity(project_id, "a.py"), (file_id, "hash1")
        )
        self.assertEqual(risk_repo.get_pattern_summary(project_id), {"eval": 2})

    def test_dependency_edges_resolve_incoming_and_outgoing(self) -> None:
        project_repo = make_project_repository(self.conn, auto_commit=False)
        file_repo = make_file_repository(self.conn, auto_commit=False)
        import_repo = make_import_repository(self.conn, auto_commit=False)

        class _StubResolver:
            def resolve_import(self, import_text, source_file, path_to_id):
                if "b" in import_text:
                    return "b.py"
                return None

        with repository_transaction(self.conn):
            project_id = project_repo.get_or_create(
                name="demo", language="python", repo_path="/tmp/demo2"
            )
            a_id, _ = file_repo.upsert(
                project_id=project_id, relpath="a.py", file_hash="ha", file_size=1,
                mtime="2026-01-01T00:00:00Z", signatures=[], content="", force=True,
            )
            b_id, _ = file_repo.upsert(
                project_id=project_id, relpath="b.py", file_hash="hb", file_size=1,
                mtime="2026-01-01T00:00:00Z", signatures=[], content="", force=True,
            )
            import_repo.insert(
                project_id, a_id,
                [{"import_text": "import b", "import_type": "internal"}], force=True,
            )
            edges = import_repo.build_dependency_edges(project_id, _StubResolver(), force=True)
            self.assertEqual(edges, 1)

        self.assertEqual(file_repo.get_dependents(project_id, "b.py", "incoming"), ["a.py"])
        self.assertEqual(file_repo.get_dependents(project_id, "a.py", "outgoing"), ["b.py"])

    def test_finding_record_upserts_on_matching_identity(self) -> None:
        project_repo = make_project_repository(self.conn, auto_commit=False)
        file_repo = make_file_repository(self.conn, auto_commit=False)
        finding_repo = make_finding_repository(self.conn, auto_commit=False)

        with repository_transaction(self.conn):
            project_id = project_repo.get_or_create(
                name="demo", language="python", repo_path="/tmp/demo3"
            )
            file_id, _ = file_repo.upsert(
                project_id=project_id, relpath="a.py", file_hash="ha", file_size=1,
                mtime="2026-01-01T00:00:00Z", signatures=[], content="", force=True,
            )

        finding_id, created = finding_repo.record(
            project_id=project_id, file_id=file_id, identity_hash="ih1",
            severity="high", category="security", message="v1", line_start=1, line_end=1,
        )
        self.assertTrue(created)

        finding_id2, created2 = finding_repo.record(
            project_id=project_id, file_id=file_id, identity_hash="ih1",
            severity="high", category="security", message="v2", line_start=1, line_end=1,
        )
        self.assertEqual(finding_id, finding_id2)
        self.assertFalse(created2)

        findings = finding_repo.get_open_findings(project_id)
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0][5], "v2")  # message was updated, not duplicated


if __name__ == "__main__":
    unittest.main()
