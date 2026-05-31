"""Schema service — database schema initialization."""

import os
from typing import Optional


class SchemaService:
    """Handles database schema initialization (idempotent).

    Used by both CLI and MCP server to ensure the schema exists.
    """

    def __init__(self, schema_path: Optional[str] = None):
        """Create a schema service.

        Args:
            schema_path: Path to schema.sql. If None, looks in the
                         same directory as this module.
        """
        if schema_path:
            self.schema_path = schema_path
        else:
            self.schema_path = os.path.join(
                os.path.dirname(os.path.abspath(__file__)),
                "..",
                "schema.sql",
            )

    def initialize(self, conn, logger=None) -> bool:
        """Initialize the database schema (idempotent).

        Args:
            conn: Database connection.
            logger: Optional logger for warnings.

        Returns:
            True if schema was initialized, False if already existed.
        """
        if not os.path.exists(self.schema_path):
            raise FileNotFoundError(f"schema.sql not found at {self.schema_path}")

        with open(self.schema_path, "r") as f:
            schema_sql = f.read()

        statements = [s.strip() for s in schema_sql.split(";") if s.strip()]
        cur = conn.cursor()
        for stmt in statements:
            try:
                cur.execute(stmt)
            except Exception as e:
                err_str = str(e).lower()
                if "already exists" not in err_str and "duplicate" not in err_str:
                    if logger:
                        logger.warning(f"Schema init warning: {e}")
                    else:
                        print(f"  Warning: {e}")
        conn.commit()
        cur.close()
        if logger:
            logger.info("Database schema initialized successfully")
        return True


__all__ = ["SchemaService"]
