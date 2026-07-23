"""Project path and name validation.

Provides secure validation of project paths and names to prevent
directory traversal, system directory access, and other injection attacks.
"""

import os


# System directories that should never be imported as projects
DANGEROUS_PREFIXES = frozenset([
    "/etc/", "/usr/", "/boot/", "/sys/", "/proc/", "/dev/",
    "/bin/", "/sbin/", "/lib/", "/lib64/",
])

# Exact paths that are always rejected
DANGEROUS_EXACT = frozenset(["/", "/bin", "/sbin", "/lib", "/lib64"])

MAX_PROJECT_NAME_LENGTH = 255


def validate_project_name(name: str) -> str:
    """Validate and sanitize a project name.

    Args:
        name: The project name to validate.

    Returns:
        The validated project name.

    Raises:
        ValueError: If the name is empty, too long, or contains unsafe characters.
    """
    if not name:
        raise ValueError("Project name cannot be empty")

    if len(name) > MAX_PROJECT_NAME_LENGTH:
        raise ValueError(
            f"Project name too long: {len(name)} chars (max {MAX_PROJECT_NAME_LENGTH})"
        )

    # Reject null bytes
    if "\x00" in name:
        raise ValueError("Project name contains null byte")

    # Reject path separators
    if "/" in name or "\\" in name:
        raise ValueError(f"Project name contains path separators: {name}")

    # Reject '..' sequences (directory traversal)
    if ".." in name:
        raise ValueError(f"Project name contains '..': {name}")

    # Reject control characters
    if any(ord(c) < 32 for c in name):
        raise ValueError("Project name contains control characters")

    return name


def validate_project_path(path: str) -> str:
    """Validate a project path to prevent directory traversal and system access.

    Args:
        path: The project path to validate (absolute or relative).

    Returns:
        The validated absolute path.

    Raises:
        ValueError: If the path is empty, points to a system directory,
                    contains traversal components, or is not a directory.
    """
    if not path:
        raise ValueError("Project path cannot be empty")

    # Reject null bytes
    if "\x00" in path:
        raise ValueError("Project path contains null byte")

    # Resolve to absolute path first
    abs_path = os.path.abspath(path)

    # Canonicalize (resolve symlinks)
    try:
        canonical = os.path.realpath(abs_path)
    except (OSError, ValueError) as e:
        raise ValueError(f"Cannot resolve project path: {path}") from e

    # Reject exact system paths
    if canonical in DANGEROUS_EXACT:
        raise ValueError(f"Project path points to system directory: {canonical}")

    # Reject system directory prefixes
    for prefix in DANGEROUS_PREFIXES:
        if canonical.startswith(prefix):
            raise ValueError(f"Project path points to system directory: {canonical}")

    # Verify it's actually a directory
    if not os.path.isdir(abs_path):
        raise ValueError(f"Not a directory: {abs_path}")

    return abs_path


def validate_path_within(path: str, root: str) -> str:
    """Validate that a path is within a root directory (path traversal protection).

    Args:
        path: The path to validate (relative to root).
        root: The root directory that path must be within.

    Returns:
        The canonical absolute path.

    Raises:
        ValueError: If the path escapes the root directory.
    """
    # Construct absolute path
    abs_path = os.path.abspath(os.path.join(root, path))

    # Canonicalize both paths
    canonical_abs = os.path.realpath(abs_path)
    canonical_root = os.path.realpath(root)

    # Check containment
    if not canonical_abs.startswith(canonical_root + os.sep) and canonical_abs != canonical_root:
        raise ValueError(f"Path traversal detected: {path} escapes {root}")

    return canonical_abs


__all__ = [
    "validate_project_name",
    "validate_project_path",
    "validate_path_within",
]
