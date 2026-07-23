"""File system abstraction with path traversal protection.

Provides a clean interface for file operations with built-in
security checks for path traversal and file size limits.
"""

import os
from abc import ABC, abstractmethod

from .validators import validate_path_within


MAX_FILE_SIZE = 1 * 1024 * 1024  # 1 MB


class IFileSystem(ABC):
    """Abstract file system interface for testability and decoupling."""

    @abstractmethod
    def read_file(self, path: str, root: str, max_size: int = MAX_FILE_SIZE) -> str:
        """Read a file's contents with path traversal protection and size limit.

        Args:
            path: Relative path from root.
            root: Root directory that path must be within.
            max_size: Maximum file size in bytes (default 1MB).

        Returns:
            File contents as string.

        Raises:
            ValueError: If path escapes root or file exceeds max_size.
            FileNotFoundError: If file doesn't exist.
            OSError: On I/O errors.
        """
        ...

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
    def get_file_size(self, path: str) -> int:
        """Get file size in bytes.

        Args:
            path: Absolute file path.

        Returns:
            File size in bytes.
        """
        ...

    @abstractmethod
    def exists(self, path: str) -> bool:
        """Check if a path exists.

        Args:
            path: Absolute file path.

        Returns:
            True if the path exists.
        """
        ...

    @abstractmethod
    def is_file(self, path: str) -> bool:
        """Check if a path is a regular file.

        Args:
            path: Absolute file path.

        Returns:
            True if the path is a regular file.
        """
        ...

    @abstractmethod
    def is_dir(self, path: str) -> bool:
        """Check if a path is a directory.

        Args:
            path: Absolute file path.

        Returns:
            True if the path is a directory.
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

    @abstractmethod
    def validate_path_within(self, path: str, root: str) -> str:
        """Validate that a path is within a root directory.

        Args:
            path: Relative path from root.
            root: Root directory that path must be within.

        Returns:
            Canonical absolute path.

        Raises:
            ValueError: If path escapes root.
        """
        ...


class LocalFileSystem(IFileSystem):
    """Local file system implementation."""

    def read_file(self, path: str, root: str, max_size: int = MAX_FILE_SIZE) -> str:
        abs_path = self.validate_path_within(path, root)

        file_size = os.path.getsize(abs_path)
        if file_size > max_size:
            raise ValueError(
                f"File size {file_size} bytes exceeds limit of {max_size} bytes"
            )

        with open(abs_path, encoding='utf-8', errors='replace') as f:
            return f.read()

    def read_file_bytes(self, path: str) -> bytes:
        with open(path, 'rb') as f:
            return f.read()

    def get_file_size(self, path: str) -> int:
        return os.path.getsize(path)

    def exists(self, path: str) -> bool:
        return os.path.exists(path)

    def is_file(self, path: str) -> bool:
        return os.path.isfile(path)

    def is_dir(self, path: str) -> bool:
        return os.path.isdir(path)

    def get_mtime(self, path: str) -> float:
        return os.path.getmtime(path)

    def validate_path_within(self, path: str, root: str) -> str:
        return validate_path_within(path, root)


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
    "MAX_FILE_SIZE",
]
