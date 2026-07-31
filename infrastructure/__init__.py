"""Infrastructure layer — connection pooling and file I/O."""

from .db import ConnectionPool
from .filesystem import (
    IFileSystem,
    LocalFileSystem,
    get_file_system,
    set_file_system,
)
from .sqlite_db import connect as sqlite_connect
from .sqlite_db import sqlite_db_path

__all__ = [
    # db
    "ConnectionPool",
    # sqlite_db
    "sqlite_connect",
    "sqlite_db_path",
    # filesystem
    "IFileSystem",
    "LocalFileSystem",
    "get_file_system",
    "set_file_system",
]
