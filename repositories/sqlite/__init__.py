"""SQLite sibling repositories — the zero-config local backend.

Same method names/signatures as their repositories/*.py PostgreSQL
counterparts, scoped to only what the current tool surface calls. Selected
at runtime by repositories/__init__.py's factory functions based on which
connection type a caller has (sqlite3.Connection vs a psycopg2 connection) —
callers never import these classes directly.
"""

from .file_repo import SqliteFileRepository
from .finding_repo import SqliteFindingRepository
from .import_repo import SqliteImportRepository
from .project_repo import SqliteProjectRepository
from .risk_repo import SqliteRiskPatternRepository

__all__ = [
    "SqliteFileRepository",
    "SqliteFindingRepository",
    "SqliteImportRepository",
    "SqliteProjectRepository",
    "SqliteRiskPatternRepository",
]
