"""File system abstraction for the import pipeline.

Only the two operations the reconciliation path actually performs
(reading raw bytes and reading mtime) are part of this interface.
"""

import os
from abc import ABC, abstractmethod


class IFileSystem(ABC):
    """Abstract file system interface for testability and decoupling."""

    @abstractmethod
    def read_file_bytes(self, path: str) -> bytes:
        """Read a file as bytes (no path validation, absolute path).

        Args:
            path: Absolute file path.

        Returns:
            File contents as bytes.
        """
        ...

    @abstractmethod
    def get_mtime(self, path: str) -> float:
        """Get file modification time as Unix timestamp.

        Args:
            path: Absolute file path.

        Returns:
            Modification time as seconds since epoch.
        """
        ...


class LocalFileSystem(IFileSystem):
    """Local file system implementation."""

    def read_file_bytes(self, path: str) -> bytes:
        with open(path, 'rb') as f:
            return f.read()

    def get_mtime(self, path: str) -> float:
        return os.path.getmtime(path)


# Default global instance
_default_fs: IFileSystem | None = None


def get_file_system() -> IFileSystem:
    """Get the default file system instance.

    Returns:
        A LocalFileSystem instance.
    """
    global _default_fs
    if _default_fs is None:
        _default_fs = LocalFileSystem()
    return _default_fs


def set_file_system(fs: IFileSystem) -> None:
    """Set the default file system instance (for testing).

    Args:
        fs: The file system to use as default.
    """
    global _default_fs
    _default_fs = fs


__all__ = [
    "IFileSystem",
    "LocalFileSystem",
    "get_file_system",
    "set_file_system",
]
