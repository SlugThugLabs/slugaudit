"""Atomic import-service reconciliation tests."""

# ruff: noqa: S101 - pytest-style assertions are not used, but unittest failure
# helpers are clearer than production-style guards in these contracts.

from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import MagicMock, patch

from app.manifest import build_manifest
from services.import_service import ImportService


class PythonExtractor:
    @classmethod
    def source_extensions(cls) -> set[str]:
        return {".py"}

    def extract_signatures(
        self, file_path: str, content: bytes
    ) -> list[dict[str, object]]:
        return [{"type": "function", "name": "new_value"}]

    def extract_imports(
        self, file_path: str, content: bytes
    ) -> list[dict[str, object]]:
        return []

    def extract_risk_patterns(
        self, file_path: str, content: bytes
    ) -> list[dict[str, object]]:
        return []

    def resolve_import(
        self,
        import_text: str,
        source_file: str,
        path_to_id: dict[str, object],
    ) -> None:
        return None


class FailingExtractor(PythonExtractor):
    def extract_signatures(
        self, file_path: str, content: bytes
    ) -> list[dict[str, object]]:
        raise RuntimeError("Tree-sitter parse failed")


def _repository_mocks() -> tuple[MagicMock, MagicMock, MagicMock, MagicMock]:
    project_repo = MagicMock()
    project_repo.get_or_create.return_value = "project-1"
    project_repo.get_status.return_value = {
        "file_count": 1,
        "signatures_count": 1,
        "imports_count": 0,
    }
    project_repo.begin_revision.return_value = "revision-1"

    file_repo = MagicMock()
    file_repo.upsert.return_value = ("file-1", True)
    file_repo.delete_removed.return_value = 0

    import_repo = MagicMock()
    import_repo.build_dependency_edges.return_value = 0

    risk_repo = MagicMock()
    risk_repo.get_pattern_summary.return_value = {}
    return project_repo, file_repo, import_repo, risk_repo


class TestAtomicImportReconciliation(unittest.TestCase):
    def test_modified_file_replaces_all_derived_facts_before_publish(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            source = project / "src" / "main.py"
            source.parent.mkdir(parents=True)
            source.write_text("def new_value(): return 2\n", encoding="utf-8")
            manifest = build_manifest(project)
            conn = MagicMock()
            project_repo, file_repo, import_repo, risk_repo = _repository_mocks()
            file_repo.get_manifest.return_value = {"src/main.py": "old-hash"}
            service = ImportService()

            with (
                patch.object(
                    service,
                    "_extractors",
                    return_value={"python": PythonExtractor()},
                ),
                patch(
                    "services.import_service.ProjectRepository",
                    return_value=project_repo,
                ),
                patch(
                    "services.import_service.FileRepository",
                    return_value=file_repo,
                ),
                patch(
                    "services.import_service.ImportRepository",
                    return_value=import_repo,
                ),
                patch(
                    "services.import_service.RiskPatternRepository",
                    return_value=risk_repo,
                ),
            ):
                result = service.reconcile_project(
                    str(project), manifest, conn=conn, force_full=False
                )

            file_repo.purge_obsolete_findings.assert_called_once_with(
                "project-1", "file-1"
            )
            import_repo.insert.assert_called_once_with(
                "project-1", "file-1", [], force=True
            )
            risk_repo.upsert.assert_called_once_with("project-1", "file-1", [])
            import_repo.build_dependency_edges.assert_called_once()
            self.assertTrue(
                import_repo.build_dependency_edges.call_args.kwargs["force"]
            )
            project_repo.publish_revision.assert_called_once_with(
                "project-1", "revision-1"
            )
            conn.commit.assert_called_once_with()
            conn.rollback.assert_not_called()
            self.assertEqual(result.revision_id, "revision-1")

    def test_parse_failure_rolls_back_and_never_publishes_partial_revision(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            source = project / "src" / "main.py"
            source.parent.mkdir(parents=True)
            source.write_text("def broken(): pass\n", encoding="utf-8")
            manifest = build_manifest(project)
            conn = MagicMock()
            project_repo, file_repo, import_repo, risk_repo = _repository_mocks()
            file_repo.get_manifest.return_value = {"src/main.py": "old-hash"}
            service = ImportService()

            with (
                patch.object(
                    service,
                    "_extractors",
                    return_value={"python": FailingExtractor()},
                ),
                patch(
                    "services.import_service.ProjectRepository",
                    return_value=project_repo,
                ),
                patch(
                    "services.import_service.FileRepository",
                    return_value=file_repo,
                ),
                patch(
                    "services.import_service.ImportRepository",
                    return_value=import_repo,
                ),
                patch(
                    "services.import_service.RiskPatternRepository",
                    return_value=risk_repo,
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "Tree-sitter parse failed"):
                    service.reconcile_project(str(project), manifest, conn=conn)

            conn.rollback.assert_called_once_with()
            conn.commit.assert_not_called()
            project_repo.begin_revision.assert_not_called()
            project_repo.publish_revision.assert_not_called()


if __name__ == "__main__":
    unittest.main()
