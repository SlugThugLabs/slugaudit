"""Tests for the dollar-quote/string-aware SQL statement splitter."""

import os
import unittest

from services.schema_service import SchemaService, _split_sql_statements


class TestSplitSqlStatements(unittest.TestCase):
    def test_splits_plain_statements(self) -> None:
        sql = "CREATE TABLE a (id int);\nCREATE TABLE b (id int);"
        self.assertEqual(
            _split_sql_statements(sql),
            ["CREATE TABLE a (id int)", "CREATE TABLE b (id int)"],
        )

    def test_ignores_semicolon_inside_single_quoted_string(self) -> None:
        sql = "INSERT INTO t (msg) VALUES ('a;b');\nSELECT 1;"
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 2)
        self.assertIn("'a;b'", statements[0])

    def test_handles_escaped_single_quote_inside_string(self) -> None:
        sql = "INSERT INTO t (msg) VALUES ('it''s; fine');\nSELECT 1;"
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 2)
        self.assertIn("it''s; fine", statements[0])

    def test_ignores_semicolon_inside_double_quoted_identifier(self) -> None:
        sql = 'CREATE TABLE "weird;name" (id int);\nSELECT 1;'
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 2)
        self.assertIn('"weird;name"', statements[0])

    def test_ignores_semicolon_inside_line_comment(self) -> None:
        sql = "-- do not split; this is one comment\nSELECT 1;"
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 1)
        self.assertIn("SELECT 1", statements[0])

    def test_ignores_semicolon_inside_block_comment(self) -> None:
        sql = "/* a; b; c */\nSELECT 1;\nSELECT 2;"
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 2)

    def test_handles_nested_block_comments(self) -> None:
        sql = "/* outer /* inner; */ still-comment; */\nSELECT 1;"
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 1)
        self.assertIn("SELECT 1", statements[0])

    def test_ignores_semicolon_inside_plain_dollar_quoted_body(self) -> None:
        sql = (
            "CREATE FUNCTION f() RETURNS void AS $$\n"
            "BEGIN\n"
            "  PERFORM 1; PERFORM 2;\n"
            "END;\n"
            "$$ LANGUAGE plpgsql;\n"
            "SELECT 1;"
        )
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 2)
        self.assertIn("PERFORM 1; PERFORM 2;", statements[0])
        self.assertTrue(statements[0].rstrip().endswith("LANGUAGE plpgsql"))

    def test_ignores_semicolon_inside_tagged_dollar_quoted_body(self) -> None:
        sql = (
            "CREATE FUNCTION f() RETURNS void AS $body$\n"
            "BEGIN PERFORM 1; END;\n"
            "$body$ LANGUAGE plpgsql;\n"
            "SELECT 1;"
        )
        statements = _split_sql_statements(sql)
        self.assertEqual(len(statements), 2)
        self.assertIn("PERFORM 1;", statements[0])

    def test_trailing_statement_without_final_semicolon_is_kept(self) -> None:
        sql = "SELECT 1;\nSELECT 2"
        statements = _split_sql_statements(sql)
        self.assertEqual(statements, ["SELECT 1", "SELECT 2"])

    def test_empty_and_whitespace_only_input_yields_no_statements(self) -> None:
        self.assertEqual(_split_sql_statements(""), [])
        self.assertEqual(_split_sql_statements("   \n\n  ;  ; "), [])


class TestRealSchemaFileSplitsCleanly(unittest.TestCase):
    """The shipped schema.sql must actually round-trip through the splitter."""

    def test_shipped_schema_produces_nonempty_statements(self) -> None:
        service = SchemaService()
        self.assertTrue(os.path.exists(service.schema_path))
        with open(service.schema_path) as f:
            schema_sql = f.read()

        statements = _split_sql_statements(schema_sql)
        self.assertGreater(len(statements), 0)
        for statement in statements:
            self.assertTrue(statement.strip())
        # Every statement the splitter emits must have balanced parens —
        # a real mis-split (e.g. cutting a string/comment/dollar-quote in
        # half) would very likely produce an unbalanced fragment.
        for statement in statements:
            self.assertEqual(
                statement.count("("), statement.count(")"),
                msg=f"Unbalanced parens, likely a bad split: {statement[:120]!r}",
            )


if __name__ == "__main__":
    unittest.main()
