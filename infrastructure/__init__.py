"""Infrastructure layer — connection management, validation, file I/O."""

from .db import parse_connection_string, get_connection, ConnectionPool
from .validators import (
    validate_project_name,
    validate_project_path,
    validate_path_within,
)
from .filesystem import (
    IFileSystem,
    LocalFileSystem,
    get_file_system,
    set_file_system,
    MAX_FILE_SIZE,
)

__all__ = [
    # db
    "parse_connection_string",
    "get_connection",
    "ConnectionPool",
    # validators
    "validate_project_name",
    "validate_project_path",
    "validate_path_within",
    # filesystem
    "IFileSystem",
    "LocalFileSystem",
    "get_file_system",
    "set_file_system",
    "MAX_FILE_SIZE",
]
