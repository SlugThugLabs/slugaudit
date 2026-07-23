"""Tests for input validation and tool definitions."""

import unittest

from app.tools import (
    TOOLS,
    validate_paths,
    validate_pattern,
    validate_sql_query,
    MAX_SEARCH_RESULTS,
    MAX_SQL_ROWS,
    MAX_READ_PATHS,
)

from app.handlers import _scope_sql_query, _SCOPED_TABLES



class TestValidatePaths(unittest.TestCase):
    """Path validation for security."""

    def test_accepts_relative_paths(self) -> None:
        result = validate_paths(["src/main.rs", "lib/core.py", "README.md"])
        self.assertEqual(len(result), 3)

    def test_rejects_directory_traversal(self) -> None:
        result = validate_paths(["../../etc/passwd", "src/../secret"])
        self.assertEqual(len(result), 0)

    def test_rejects_absolute_paths(self) -> None:
        result = validate_paths(["/etc/passwd", "/root/.ssh/id_rsa"])
        self.assertEqual(len(result), 0)

    def test_rejects_home_paths(self) -> None:
        result = validate_paths(["~/secret", "~other/file"])
        self.assertEqual(len(result), 0)

    def test_rejects_empty_input(self) -> None:
        result = validate_paths(["", "   "])
        self.assertEqual(len(result), 0)

    def test_rejects_long_paths(self) -> None:
        from app.tools import MAX_PATH_LENGTH
        long_path = "a" * (MAX_PATH_LENGTH + 1)
        result = validate_paths([long_path])
        self.assertEqual(len(result), 0)

    def test_handles_none_and_non_list(self) -> None:
        result = validate_paths(None)
        self.assertEqual(len(result), 0)
        result = validate_paths("not a list")
        self.assertEqual(len(result), 0)

    def test_limits_path_count(self) -> None:
        many_paths = [f"file{i}.py" for i in range(MAX_READ_PATHS * 2)]
        result = validate_paths(many_paths)
        self.assertLessEqual(len(result), MAX_READ_PATHS)

    def test_strips_whitespace(self) -> None:
        result = validate_paths(["  src/main.rs  ", "\tlib/core.py"])
        self.assertEqual(len(result), 2)
        self.assertEqual(result[0], "src/main.rs")


class TestValidatePattern(unittest.TestCase):
    """Search pattern validation."""

    def test_accepts_valid_pattern(self) -> None:
        self.assertEqual(validate_pattern("def foo"), "def foo")

    def test_rejects_empty_pattern(self) -> None:
        self.assertIsNone(validate_pattern(""))
        self.assertIsNone(validate_pattern("   "))

    def test_rejects_none(self) -> None:
        self.assertIsNone(validate_pattern(None))

    def test_rejects_long_pattern(self) -> None:
        self.assertIsNone(validate_pattern("x" * 300))


class TestValidateSQLQuery(unittest.TestCase):
    """SQL query validation for read-only safety."""

    def test_accepts_select(self) -> None:
        q = "SELECT * FROM files"
        self.assertEqual(validate_sql_query(q), q)

    def test_rejects_with_cte(self) -> None:
        q = "WITH recent AS (SELECT * FROM files) SELECT * FROM recent"
        self.assertIsNone(validate_sql_query(q))

    def test_rejects_writable_cte(self) -> None:
        q = "WITH gone AS (DELETE FROM files RETURNING *) SELECT * FROM gone"
        self.assertIsNone(validate_sql_query(q))

    def test_rejects_insert(self) -> None:
        self.assertIsNone(validate_sql_query("INSERT INTO files VALUES (1)"))

    def test_rejects_update(self) -> None:
        self.assertIsNone(validate_sql_query("UPDATE files SET path='x'"))

    def test_rejects_delete(self) -> None:
        self.assertIsNone(validate_sql_query("DELETE FROM files"))

    def test_rejects_drop(self) -> None:
        self.assertIsNone(validate_sql_query("DROP TABLE files"))

    def test_rejects_empty_query(self) -> None:
        self.assertIsNone(validate_sql_query(""))
        self.assertIsNone(validate_sql_query("   "))

    def test_rejects_long_query(self) -> None:
        self.assertIsNone(validate_sql_query("SELECT " + "x" * 6000))

    def test_rejects_multi_statement(self) -> None:
        self.assertIsNone(validate_sql_query("SELECT 1; DROP TABLE files"))
        self.assertIsNone(validate_sql_query(
            "WITH cte AS (SELECT 1) SELECT * FROM cte; UPDATE files SET path='x'"
        ))

    def test_allows_trailing_semicolon(self) -> None:
        self.assertEqual(validate_sql_query("SELECT * FROM files;"), "SELECT * FROM files;")

    def test_rejects_select_without_project_table(self) -> None:
        self.assertIsNone(validate_sql_query("SELECT 1"))
        self.assertIsNone(validate_sql_query("SELECT * FROM projects"))

    def test_rejects_joins_subqueries_and_set_operations(self) -> None:
        self.assertIsNone(validate_sql_query(
            "SELECT * FROM files f JOIN findings x ON x.file_id = f.id"
        ))
        self.assertIsNone(validate_sql_query(
            "SELECT * FROM files WHERE id IN (SELECT file_id FROM findings)"
        ))
        self.assertIsNone(validate_sql_query(
            "SELECT path FROM files UNION SELECT import_text FROM file_imports"
        ))

    def test_rejects_comments_and_arbitrary_functions(self) -> None:
        self.assertIsNone(validate_sql_query("SELECT * FROM files -- bypass"))
        self.assertIsNone(validate_sql_query("SELECT pg_sleep(10) FROM files"))

    def test_accepts_allowlisted_aggregates(self) -> None:
        q = "SELECT COUNT(*), MAX(size) FROM files WHERE path LIKE '%.py'"
        self.assertEqual(validate_sql_query(q), q)

    def test_is_case_insensitive(self) -> None:
        q = "select * from files"
        self.assertEqual(validate_sql_query(q), q)


class TestScopeSQLQuery(unittest.TestCase):
    """Project-ID scoping for raw SQL queries."""

    def test_scopes_select_on_scoped_table(self) -> None:
        result = _scope_sql_query("SELECT * FROM files", "proj-1")
        self.assertIn("WHERE files.project_id = %s", result)
        self.assertNotIn("proj-1", result)  # should use placeholder, not value

    def test_scopes_with_where(self) -> None:
        result = _scope_sql_query(
            "SELECT * FROM files WHERE path LIKE '%.rs'", "proj-1"
        )
        self.assertIn("WHERE files.project_id = %s AND path LIKE", result)

    def test_scopes_qualified_alias(self) -> None:
        result = _scope_sql_query("SELECT f.path FROM files AS f", "proj-1")
        self.assertIn("WHERE f.project_id = %s", result)

    def test_rejects_non_scoped_table(self) -> None:
        with self.assertRaises(ValueError):
            _scope_sql_query("SELECT * FROM pg_catalog", "proj-1")

    def test_forces_scope_when_project_id_already_present(self) -> None:
        q = "SELECT * FROM files WHERE project_id = 'abc'"
        result = _scope_sql_query(q, "proj-1")
        self.assertIn("files.project_id = %s AND project_id = 'abc'", result)

    def test_scopes_before_group_by(self) -> None:
        result = _scope_sql_query(
            "SELECT severity, COUNT(*) FROM findings GROUP BY severity", "proj-1"
        )
        self.assertIn("WHERE findings.project_id = %s GROUP BY", result)

    def test_scopes_before_order_by(self) -> None:
        result = _scope_sql_query(
            "SELECT * FROM files ORDER BY path", "proj-1"
        )
        self.assertIn("WHERE files.project_id = %s ORDER BY", result)

    def test_scopes_before_limit(self) -> None:
        result = _scope_sql_query(
            "SELECT * FROM dependency_edges LIMIT 10", "proj-1"
        )
        self.assertIn("WHERE dependency_edges.project_id = %s LIMIT", result)

    def test_rejects_union(self) -> None:
        with self.assertRaises(ValueError):
            _scope_sql_query(
                "SELECT path FROM files UNION SELECT path FROM file_imports", "proj-1"
            )

    def test_rejects_join(self) -> None:
        with self.assertRaises(ValueError):
            _scope_sql_query(
                "SELECT f.path, fi.import_text FROM files f JOIN file_imports fi "
                "ON f.id = fi.file_id", "proj-1"
            )

    def test_rejects_with_queries(self) -> None:
        with self.assertRaises(ValueError):
            _scope_sql_query(
                "WITH recent AS (SELECT * FROM files) SELECT * FROM recent", "proj-1"
            )

    def test_rejects_insert(self) -> None:
        with self.assertRaises(ValueError):
            _scope_sql_query("INSERT INTO files VALUES (1)", "proj-1")

    def test_scoped_tables_are_documented(self) -> None:
        self.assertIn("files", _SCOPED_TABLES)
        self.assertIn("findings", _SCOPED_TABLES)
        self.assertIn("risk_patterns", _SCOPED_TABLES)
        self.assertNotIn("file_staleness", _SCOPED_TABLES)


class TestToolDefinitions(unittest.TestCase):
    """Tool registry contains expected tools."""

    def test_all_tools_present(self) -> None:
        names = {t.name for t in TOOLS}
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
        self.assertEqual(names, expected)

    def test_all_tools_have_descriptions(self) -> None:
        for t in TOOLS:
            self.assertTrue(t.description, f"Tool {t.name} has no description")

    def test_search_tool_has_pattern_required(self) -> None:
        search = next(t for t in TOOLS if t.name == "audit_search")
        self.assertIn("pattern", search.inputSchema.get("required", []))

    def test_file_tree_tool_has_max_depth_default(self) -> None:
        tree_tool = next(t for t in TOOLS if t.name == "audit_file_tree")
        props = tree_tool.inputSchema.get("properties", {})
        max_depth = props.get("max_depth", {})
        self.assertEqual(max_depth.get("default", None), 3)
        self.assertEqual(max_depth.get("minimum", None), 1)
        self.assertEqual(max_depth.get("maximum", None), 10)
        self.assertEqual(max_depth.get("type", None), "integer")

    def test_max_constants_are_sane(self) -> None:
        self.assertGreater(MAX_SEARCH_RESULTS, 0)
        self.assertGreater(MAX_SQL_ROWS, 0)
        self.assertGreater(MAX_READ_PATHS, 0)


if __name__ == "__main__":
    unittest.main()
