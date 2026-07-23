"""Transaction and revision contract tests for the repository layer."""

# ruff: noqa: S101 - pytest assertions provide the clearest contract failures.

from unittest.mock import MagicMock

import pytest

from repositories import (
    FileRepository,
    ImportRepository,
    ProjectRepository,
    RiskPatternRepository,
    repository_transaction,
)


def _connection() -> tuple[MagicMock, MagicMock]:
    conn = MagicMock()
    cursor = conn.cursor.return_value
    return conn, cursor


def test_repository_transaction_commits_once() -> None:
    conn, _ = _connection()

    with repository_transaction(conn):
        pass

    conn.commit.assert_called_once_with()
    conn.rollback.assert_not_called()


def test_repository_transaction_rolls_back_on_failure() -> None:
    conn, _ = _connection()

    with pytest.raises(RuntimeError, match="index failed"):
        with repository_transaction(conn):
            raise RuntimeError("index failed")

    conn.rollback.assert_called_once_with()
    conn.commit.assert_not_called()


def test_revision_publish_does_not_commit_inside_caller_transaction() -> None:
    conn, cursor = _connection()
    cursor.fetchone.side_effect = [("revision-1",), ("project-1",)]
    repo = ProjectRepository(conn, auto_commit=False)

    repo.publish_revision("project-1", "revision-1")

    conn.commit.assert_not_called()
    statements = [call.args[0] for call in cursor.execute.call_args_list]
    assert any("status = 'ready'" in statement for statement in statements)
    assert any("current_revision_id" in statement for statement in statements)
    assert any("DELETE FROM project_revisions" in statement for statement in statements)


def test_current_revision_exposes_freshness_metadata() -> None:
    conn, cursor = _connection()
    cursor.fetchone.return_value = (
        "revision-1",
        "manifest-1",
        12,
        34,
        "tree-sitter-1",
        "2026-07-22T00:00:00Z",
    )

    revision = ProjectRepository(conn).get_current_revision("project-1")

    assert revision == {
        "revision_id": "revision-1",
        "manifest_hash": "manifest-1",
        "file_count": 12,
        "signature_count": 34,
        "parser_version": "tree-sitter-1",
        "published_at": "2026-07-22T00:00:00Z",
    }


def test_project_purge_is_scoped_and_honors_caller_transaction() -> None:
    conn, cursor = _connection()
    cursor.fetchone.return_value = ("project-1",)

    deleted = ProjectRepository(conn, auto_commit=False).purge_project("project-1")

    assert deleted is True
    assert cursor.execute.call_args_list[-1].args == (
        "DELETE FROM projects WHERE id = %s RETURNING id", ("project-1",)
    )
    conn.commit.assert_not_called()


def test_file_manifest_is_path_to_hash_map() -> None:
    conn, cursor = _connection()
    cursor.fetchall.return_value = [("a.py", "hash-a"), ("b.py", "hash-b")]

    manifest = FileRepository(conn).get_manifest("project-1")

    assert manifest == {"a.py": "hash-a", "b.py": "hash-b"}


def test_empty_import_replacement_deletes_stale_imports() -> None:
    conn, cursor = _connection()

    count = ImportRepository(conn, auto_commit=False).insert(
        "project-1", "file-1", []
    )

    assert count == 0
    cursor.execute.assert_called_once_with(
        "DELETE FROM file_imports WHERE file_id = %s", ("file-1",)
    )
    conn.commit.assert_not_called()


def test_empty_risk_replacement_deletes_stale_patterns() -> None:
    conn, cursor = _connection()

    count = RiskPatternRepository(conn, auto_commit=False).upsert(
        "project-1", "file-1", []
    )

    assert count == 0
    cursor.execute.assert_called_once_with(
        "DELETE FROM risk_patterns WHERE file_id = %s", ("file-1",)
    )
    conn.commit.assert_not_called()
