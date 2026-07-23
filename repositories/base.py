"""Base repository with common transaction patterns."""

from contextlib import contextmanager
from collections.abc import Iterator
from typing import Any


class BaseRepository:
    """Base class for repositories providing common database operations.

    Each repository wraps a single database connection and manages
    cursor lifecycle automatically.
    """

    def __init__(self, conn: Any, *, auto_commit: bool = True) -> None:
        """Create a repository wrapping a database connection.

        Args:
            conn: A psycopg2 connection.
            auto_commit: Commit after repository writes. Set this to ``False``
                when several repository operations must publish atomically.
        """
        self.conn = conn
        self.auto_commit = auto_commit

    def _cursor(self) -> Any:
        """Create and return a new cursor."""
        return self.conn.cursor()

    def _commit(self) -> None:
        """Commit a write when this repository owns the transaction boundary."""
        if self.auto_commit:
            self.conn.commit()


@contextmanager
def repository_transaction(conn: Any) -> Iterator[None]:
    """Commit a group of repository writes atomically, or roll it all back.

    Repositories used inside this context must be constructed with
    ``auto_commit=False``. Keeping the boundary explicit prevents a helper
    repository from exposing a half-built index revision.
    """
    try:
        yield
        conn.commit()
    except BaseException:
        conn.rollback()
        raise


__all__ = ["BaseRepository", "repository_transaction"]
