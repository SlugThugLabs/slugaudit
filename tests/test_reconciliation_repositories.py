"""Repository regressions that prevent derived facts surviving reconciliation."""

from __future__ import annotations

import unittest

from repositories.file_repo import FileRepository
from repositories.import_repo import ImportRepository
from repositories.risk_repo import RiskPatternRepository


class RecordingCursor:
    def __init__(self, rows: list[tuple[str, str]] | None = None) -> None:
        self.rows = rows or []
        self.executions: list[tuple[str, object]] = []
        self.rowcount = 0

    def execute(self, query: str, params: object = None) -> None:
        self.executions.append((" ".join(query.split()), params))

    def fetchall(self) -> list[tuple[str, str]]:
        return self.rows

    def close(self) -> None:
        pass


class RecordingConnection:
    def __init__(self, rows: list[tuple[str, str]] | None = None) -> None:
        self.recording_cursor = RecordingCursor(rows)
        self.commits = 0

    def cursor(self) -> RecordingCursor:
        return self.recording_cursor

    def commit(self) -> None:
        self.commits += 1


class TestDerivedFactReplacement(unittest.TestCase):
    """Changed and deleted files cannot retain obsolete derived evidence."""

    def test_removing_all_imports_still_deletes_previous_import_facts(self) -> None:
        conn = RecordingConnection()

        inserted = ImportRepository(conn).insert("project-1", "file-1", [])

        self.assertEqual(inserted, 0)
        self.assertEqual(conn.commits, 1)
        self.assertEqual(
            conn.recording_cursor.executions,
            [("DELETE FROM file_imports WHERE file_id = %s", ("file-1",))],
        )

    def test_removing_all_risks_still_deletes_previous_risk_facts(self) -> None:
        conn = RecordingConnection()

        inserted = RiskPatternRepository(conn).upsert("project-1", "file-1", [])

        self.assertEqual(inserted, 0)
        self.assertEqual(conn.commits, 1)
        self.assertEqual(
            conn.recording_cursor.executions,
            [("DELETE FROM risk_patterns WHERE file_id = %s", ("file-1",))],
        )

    def test_deleted_file_purges_source_and_every_derived_evidence_family(self) -> None:
        conn = RecordingConnection(
            [("keep-id", "src/keep.py"), ("remove-id", "src/remove.py")]
        )

        deleted = FileRepository(conn).delete_removed(
            "project-1", {"src/keep.py"}
        )

        self.assertEqual(deleted, 1)
        self.assertEqual(conn.commits, 1)
        statements = [query for query, _ in conn.recording_cursor.executions]
        self.assertTrue(any("DELETE FROM dependency_edges" in query for query in statements))
        self.assertTrue(any("DELETE FROM file_imports" in query for query in statements))
        self.assertTrue(any("DELETE FROM file_staleness" in query for query in statements))
        self.assertTrue(any("DELETE FROM findings" in query for query in statements))
        self.assertTrue(any("DELETE FROM files" in query for query in statements))
        self.assertIn(
            (["remove-id"],),
            [
                params
                for query, params in conn.recording_cursor.executions
                if query.startswith("DELETE FROM files")
            ],
        )


if __name__ == "__main__":
    unittest.main()
