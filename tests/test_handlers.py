"""Tests for MCP tool handlers."""

import unittest
from types import SimpleNamespace
from unittest.mock import patch

from app.handlers import (
    HANDLERS,
    handle_file_tree,
    handle_finding,
    handle_raw_sql,
    handle_read_file,
)


class TestHandlerRegistry(unittest.TestCase):
    """Handler dispatch table contains all expected handlers."""

    def test_all_expected_handlers_present(self) -> None:
        expected = {
            "audit_overview",
            "audit_search",
            "audit_read_file",
            "audit_dependents",
            "audit_brief",
            "audit_finding",
            "audit_raw_sql",
            "audit_file_tree",
        }
        self.assertEqual(set(HANDLERS.keys()), expected)

    def test_all_handlers_are_callable(self) -> None:
        for name, handler in HANDLERS.items():
            self.assertTrue(callable(handler), f"Handler '{name}' is not callable")

    def test_all_handlers_are_async(self) -> None:
        import inspect
        for name, handler in HANDLERS.items():
            self.assertTrue(
                inspect.iscoroutinefunction(handler),
                f"Handler '{name}' is not async",
            )

    def test_each_handler_takes_three_args(self) -> None:
        import inspect
        for name, handler in HANDLERS.items():
            sig = inspect.signature(handler)
            params = list(sig.parameters.keys())
            self.assertEqual(
                params[:3],
                ["conn", "state", "args"],
                f"Handler '{name}' has unexpected signature: {params}",
            )

    def test_handler_names_match_tool_names(self) -> None:
        from app.tools import TOOLS
        tool_names = {t.name for t in TOOLS}
        self.assertEqual(set(HANDLERS.keys()), tool_names)


class _RawSQLCursor:
    def __init__(self) -> None:
        self.calls: list[tuple[str, object | None]] = []
        self.description = [("path",)]
        self.closed = False

    def execute(self, query: str, params: object | None = None) -> None:
        self.calls.append((query, params))

    def fetchmany(self, limit: int) -> list[tuple[str]]:
        return [("src/main.py",)]

    def close(self) -> None:
        self.closed = True


class _RawSQLConnection:
    def __init__(self) -> None:
        self.raw_cursor = _RawSQLCursor()
        self.commits = 0
        self.rollbacks = 0

    def cursor(self) -> _RawSQLCursor:
        return self.raw_cursor

    def commit(self) -> None:
        self.commits += 1

    def rollback(self) -> None:
        self.rollbacks += 1


class TestRawSQLHandler(unittest.IsolatedAsyncioTestCase):
    async def test_uses_read_only_transaction_and_forced_scope(self) -> None:
        conn = _RawSQLConnection()
        state = SimpleNamespace(project_id="project-1", project_name="demo")

        result = await handle_raw_sql(conn, state, {"query": "SELECT path FROM files"})

        self.assertEqual(conn.commits, 1)
        self.assertEqual(conn.rollbacks, 1)
        self.assertTrue(conn.raw_cursor.closed)
        self.assertEqual(conn.raw_cursor.calls[0], ("BEGIN READ ONLY", None))
        self.assertEqual(
            conn.raw_cursor.calls[1],
            ("SET LOCAL statement_timeout = '3000ms'", None),
        )
        query, params = conn.raw_cursor.calls[2]
        self.assertIn("files.project_id = %s", query)
        self.assertEqual(params, ("project-1",))
        self.assertIn("Scoped to project `demo`", result[0].text)

    async def test_rejects_writable_cte_before_opening_cursor(self) -> None:
        conn = _RawSQLConnection()
        state = SimpleNamespace(project_id="project-1", project_name="demo")
        result = await handle_raw_sql(
            conn,
            state,
            {"query": "WITH gone AS (DELETE FROM files RETURNING *) SELECT * FROM gone"},
        )
        self.assertEqual(conn.raw_cursor.calls, [])
        self.assertIn("single-table SELECT", result[0].text)


class TestFindingHandler(unittest.IsolatedAsyncioTestCase):
    async def test_records_finding_against_current_file_hash(self) -> None:
        state = SimpleNamespace(project_id="project-1", revision_id="revision-1")
        with (
            patch(
                "app.handlers.FileRepository.get_file_identity",
                return_value=("file-1", "hash-1"),
            ),
            patch(
                "app.handlers.FindingRepository.record",
                return_value=("finding-1", True),
            ) as record,
        ):
            result = await handle_finding(
                object(),
                state,
                {
                    "path": "src/main.py",
                    "line_start": 10,
                    "line_end": 12,
                    "severity": "high",
                    "category": "correctness",
                    "title": "Incorrect state transition",
                    "description": "The failure path publishes partial state.",
                },
            )

        self.assertIn('"finding_id":"finding-1"', result[0].text)
        self.assertIn('"evidence_revision":"revision-1"', result[0].text)
        self.assertEqual(record.call_count, 1)

    async def test_rejects_unknown_file(self) -> None:
        state = SimpleNamespace(project_id="project-1", revision_id="revision-1")
        with patch(
            "app.handlers.FileRepository.get_file_identity", return_value=None
        ):
            result = await handle_finding(
                object(),
                state,
                {
                    "path": "missing.py",
                    "line_start": 1,
                    "severity": "medium",
                    "category": "correctness",
                    "title": "Missing",
                    "description": "Not indexed.",
                },
            )
        self.assertIn("Indexed file not found", result[0].text)


class TestBoundedRetrieval(unittest.IsolatedAsyncioTestCase):
    async def test_read_file_applies_line_bounds(self) -> None:
        state = SimpleNamespace(project_id="project-1")
        with patch(
            "app.handlers.FileRepository.get_file_contents",
            return_value=[("src/main.py", "one\ntwo\nthree\nfour")],
        ):
            result = await handle_read_file(
                object(),
                state,
                {"paths": ["src/main.py"], "start_line": 2, "end_line": 3},
            )
        self.assertIn("2: two", result[0].text)
        self.assertIn("3: three", result[0].text)
        self.assertNotIn("one", result[0].text)
        self.assertNotIn("four", result[0].text)

    async def test_file_tree_renders_lines_not_python_list(self) -> None:
        state = SimpleNamespace(project_id="project-1", project_name="demo")
        with patch(
            "app.handlers.FileRepository.get_all_paths_ordered",
            return_value=["src/main.py", "tests/test_main.py"],
        ):
            result = await handle_file_tree(object(), state, {})
        self.assertIn("src/\n  main.py", result[0].text)
        self.assertNotIn("['src/", result[0].text)


if __name__ == "__main__":
    unittest.main()
