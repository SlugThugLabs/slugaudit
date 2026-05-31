"""Base repository with common patterns."""


class BaseRepository:
    """Base class for repositories providing common database operations.

    Each repository wraps a single database connection and manages
    cursor lifecycle automatically.
    """

    def __init__(self, conn):
        """Create a repository wrapping a database connection.

        Args:
            conn: A psycopg2 connection.
        """
        self.conn = conn

    def _cursor(self):
        """Create and return a new cursor."""
        return self.conn.cursor()
