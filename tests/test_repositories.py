"""Transaction and revision contract tests for the repository layer."""

# ruff: noqa: S101 - pytest assertions provide the clearest contract failures.

import json
from decimal import Decimal
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


def test_file_stats_total_bytes_is_a_real_int_not_decimal() -> None:
    # files.size is BIGINT; PostgreSQL's SUM(bigint) returns numeric, which
    # psycopg2 decodes as Decimal regardless of the actual value. A raw
    # Decimal here broke audit_brief's json.dumps in production
    # ("Object of type Decimal is not JSON serializable") the first time it
    # was ever called with real file-size data — get_file_stats must return
    # a plain int so every caller, JSON-serializing or not, gets one.
    conn, cursor = _connection()
    cursor.fetchone.side_effect = [
        (3, Decimal("1024")),  # COUNT(*), COALESCE(SUM(size), 0)
        (2,),  # files_with_sigs
        (Decimal("7"),),  # COALESCE(SUM(jsonb_array_length(...)), 0)
    ]

    stats = FileRepository(conn).get_file_stats("project-1")

    assert stats["total_bytes"] == 1024
    assert type(stats["total_bytes"]) is int
    json.dumps(stats)  # must not raise


def test_project_status_total_size_is_a_real_int_not_decimal() -> None:
    conn, cursor = _connection()
    cursor.fetchone.side_effect = [
        (3, Decimal("2048"), 2),  # COUNT(*), COALESCE(SUM(size), 0), with_sigs
        (Decimal("9"),),  # SUM(jsonb_array_length(...))
        (5,),  # file_imports COUNT(*)
        (4,),  # dependency_edges COUNT(*)
    ]

    status = ProjectRepository(conn).get_status("project-1")

    assert status["total_size"] == 2048
    assert type(status["total_size"]) is int
    json.dumps(status)  # must not raise


# The five tests below cover methods that were, until now, only ever called
# from production code (app/sync.py, app/activation.py, services/
# import_service.py, app/handlers.py) and never exercised by a single test —
# not mocked-and-verified, not real-DB-tested, nothing. This is exactly the
# kind of gap that let the audit_brief Decimal bug ship undetected.


def test_get_by_path_finds_project_used_by_the_freshness_gate() -> None:
    conn, cursor = _connection()
    cursor.fetchone.return_value = ("project-1", "demo", "python", "/repo")

    row = ProjectRepository(conn).get_by_path("/repo")

    assert row == ("project-1", "demo", "python", "/repo")
    cursor.execute.assert_called_once_with(
        "SELECT id, name, primary_language, repo_path "
        "FROM projects WHERE repo_path = %s",
        ("/repo",),
    )


def test_get_by_path_returns_none_for_an_unknown_project() -> None:
    conn, cursor = _connection()
    cursor.fetchone.return_value = None

    assert ProjectRepository(conn).get_by_path("/nowhere") is None


def test_purge_by_path_deletes_the_project_it_finds() -> None:
    conn, cursor = _connection()
    cursor.fetchone.side_effect = [("project-1",), ("project-1",)]

    deleted = ProjectRepository(conn).purge_by_path("/repo")

    assert deleted is True
    statements = [call.args[0] for call in cursor.execute.call_args_list]
    assert any("DELETE FROM projects WHERE id = %s" in s for s in statements)


def test_purge_by_path_is_a_noop_for_an_unknown_project() -> None:
    conn, cursor = _connection()
    cursor.fetchone.return_value = None

    assert ProjectRepository(conn).purge_by_path("/nowhere") is False
    # Never even looks for evidence to delete if the project doesn't exist.
    delete_calls = [
        call for call in cursor.execute.call_args_list if "DELETE" in call.args[0]
    ]
    assert delete_calls == []


def test_purge_obsolete_findings_scopes_to_project_and_file() -> None:
    conn, cursor = _connection()
    cursor.rowcount = 3

    deleted = FileRepository(conn).purge_obsolete_findings("project-1", "file-1")

    assert deleted == 3
    cursor.execute.assert_called_once_with(
        "DELETE FROM findings WHERE project_id = %s AND file_id = %s",
        ("project-1", "file-1"),
    )
    conn.commit.assert_called_once_with()


def test_update_audit_timestamps_syncs_hash_for_the_whole_project() -> None:
    conn, cursor = _connection()

    FileRepository(conn).update_audit_timestamps("project-1")

    cursor.execute.assert_called_once_with(
        "UPDATE files SET last_audited_hash = hash WHERE project_id = %s",
        ("project-1",),
    )
    conn.commit.assert_called_once_with()


def test_get_all_paths_ordered_returns_sorted_paths_only() -> None:
    conn, cursor = _connection()
    cursor.fetchall.return_value = [("b.py",), ("a.py",)]

    paths = FileRepository(conn).get_all_paths_ordered("project-1")

    assert paths == ["b.py", "a.py"]  # trusts the DB's ORDER BY, doesn't re-sort
    query = cursor.execute.call_args.args[0]
    assert "ORDER BY path" in query
