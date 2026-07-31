"""Circular imports don't crash or hang the dependency-graph pipeline.

Structural note on why this is even safe to test quickly: nothing in the
sync/reconciliation path does a recursive multi-hop graph walk that a cycle
could spin forever on. ImportRepository.build_dependency_edges does one pass
over each file's *direct* imports and calls resolve_import once per import —
it never chases A -> B -> C -> A transitively. The one place that *does*
recurse (RustExtractor._follow_reexport, chasing re-exports to find where an
item is really defined) already has an explicit depth cap
(_MAX_REEXPORT_DEPTH). So a real import cycle can only ever produce two
direct edges (A->B and B->A); this test proves that's actually what happens,
through the real reconciliation pipeline, not just an assertion that it
*should* be safe.
"""

import sqlite3
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from app.manifest import build_manifest
from infrastructure.sqlite_db import _regexp
from repositories import make_file_repository
from services.import_service import ImportService
from services.sqlite_schema_service import SqliteSchemaService


def _sqlite_conn() -> sqlite3.Connection:
    conn = sqlite3.connect(":memory:")
    conn.execute("PRAGMA foreign_keys = ON")
    conn.create_function("REGEXP", 2, _regexp)
    SqliteSchemaService().initialize(conn)
    return conn


class TestCircularImportsThroughRealPipeline(unittest.TestCase):
    def test_two_python_files_importing_each_other_both_resolve(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text(
                "from b import b_func\n\ndef a_func():\n    return b_func()\n",
                encoding="utf-8",
            )
            (root / "b.py").write_text(
                "from a import a_func\n\ndef b_func():\n    return 1\n",
                encoding="utf-8",
            )

            manifest = build_manifest(root)
            conn = _sqlite_conn()

            result = ImportService().reconcile_project(
                str(root), manifest, conn=conn, force_full=True
            )

            self.assertEqual(result.files_processed, 2)
            self.assertEqual(
                result.dependency_edges, 2,
                "expected exactly the two direct edges (a->b, b->a), no hang, no crash",
            )

            file_repo = make_file_repository(conn)
            project_id = result.project_id
            self.assertEqual(
                file_repo.get_dependents(project_id, "a.py", "incoming"), ["b.py"]
            )
            self.assertEqual(
                file_repo.get_dependents(project_id, "a.py", "outgoing"), ["b.py"]
            )
            self.assertEqual(
                file_repo.get_dependents(project_id, "b.py", "incoming"), ["a.py"]
            )
            self.assertEqual(
                file_repo.get_dependents(project_id, "b.py", "outgoing"), ["a.py"]
            )

    def test_three_file_cycle_resolves_without_hanging(self) -> None:
        # a -> b -> c -> a. If anything ever chased imports transitively
        # instead of one hop at a time, this is the shape that would hang.
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("from c import c_func\n", encoding="utf-8")
            (root / "b.py").write_text("from a import a_func\n", encoding="utf-8")
            (root / "c.py").write_text("from b import b_func\n", encoding="utf-8")
            for name in ("a", "b", "c"):
                (root / f"{name}.py").write_text(
                    (root / f"{name}.py").read_text(encoding="utf-8")
                    + f"\ndef {name}_func():\n    return 1\n",
                    encoding="utf-8",
                )

            manifest = build_manifest(root)
            conn = _sqlite_conn()

            result = ImportService().reconcile_project(
                str(root), manifest, conn=conn, force_full=True
            )

            self.assertEqual(result.files_processed, 3)
            self.assertEqual(result.dependency_edges, 3)

            file_repo = make_file_repository(conn)
            project_id = result.project_id
            self.assertEqual(
                file_repo.get_dependents(project_id, "a.py", "incoming"), ["b.py"]
            )
            self.assertEqual(
                file_repo.get_dependents(project_id, "b.py", "incoming"), ["c.py"]
            )
            self.assertEqual(
                file_repo.get_dependents(project_id, "c.py", "incoming"), ["a.py"]
            )

    def test_file_that_imports_itself_does_not_create_a_self_edge(self) -> None:
        # Unusual, but not impossible (e.g. a package's __init__.py doing
        # `from . import something` that resolves back to itself). Confirm
        # this degenerate case doesn't crash and doesn't record a nonsensical
        # self-loop edge — import_repo.build_dependency_edges explicitly
        # skips edges where target_id == src_file_id.
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "pkg").mkdir()
            (root / "pkg" / "__init__.py").write_text(
                "from . import __init__ as _self\n\ndef do_thing():\n    return 1\n",
                encoding="utf-8",
            )

            manifest = build_manifest(root)
            conn = _sqlite_conn()

            result = ImportService().reconcile_project(
                str(root), manifest, conn=conn, force_full=True
            )
            self.assertEqual(result.files_processed, 1)
            self.assertEqual(result.dependency_edges, 0)

            file_repo = make_file_repository(conn)
            project_id = result.project_id
            incoming = file_repo.get_dependents(project_id, "pkg/__init__.py", "incoming")
            self.assertEqual(incoming, [])


if __name__ == "__main__":
    unittest.main()
