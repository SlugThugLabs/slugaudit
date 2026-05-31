"""Language registry and auto-detection."""

import os
from typing import Optional

from .base import BaseExtractor
from .rust import RustExtractor
from .python import PythonExtractor
from .typescript import TypeScriptExtractor
from .go import GoExtractor
from .java import JavaExtractor
from .c import CExtractor
from .cpp import CppExtractor
from .ruby import RubyExtractor


def detect_language(project_root: str) -> Optional[type[BaseExtractor]]:
    """Auto-detect the programming language for a project.
    
    Checks for project files in order of specificity.
    """
    # Check for toolchain file first (most specific)
    if os.path.exists(os.path.join(project_root, "rust-toolchain.toml")) or \
       os.path.exists(os.path.join(project_root, "rust-toolchain")):
        return RustExtractor

    # Check for project files
    checks = [
        ("Cargo.toml", RustExtractor),
        ("pyproject.toml", PythonExtractor),
        ("setup.py", PythonExtractor),
        ("requirements.txt", PythonExtractor),
        ("package.json", TypeScriptExtractor),
        ("tsconfig.json", TypeScriptExtractor),
        ("go.mod", GoExtractor),
        ("go.sum", GoExtractor),
        ("pom.xml", JavaExtractor),
        ("build.gradle", JavaExtractor),
        ("build.gradle.kts", JavaExtractor),
        ("Gemfile", RubyExtractor),
        ("Rakefile", RubyExtractor),
    ]
    for filename, extractor_cls in checks:
        if os.path.exists(os.path.join(project_root, filename)):
            return extractor_cls

    # Fallback: check file extensions in the project
    ext_counts = {}
    for dirpath, dirnames, filenames in os.walk(project_root):
        dirnames[:] = [d for d in dirnames if not d.startswith(".") and d != "target" and d != "node_modules"]
        for f in filenames:
            _, ext = os.path.splitext(f)
            ext_counts[ext] = ext_counts.get(ext, 0) + 1
        if sum(ext_counts.values()) > 100:
            break

    # Map extensions to extractors
    ext_map = {
        ".rs": RustExtractor,
        ".py": PythonExtractor,
        ".ts": TypeScriptExtractor,
        ".tsx": TypeScriptExtractor,
        ".js": TypeScriptExtractor,
        ".go": GoExtractor,
        ".java": JavaExtractor,
        ".c": CExtractor,
        ".h": CExtractor,
        ".cpp": CppExtractor,
        ".hpp": CppExtractor,
        ".cc": CppExtractor,
        ".cxx": CppExtractor,
        ".rb": RubyExtractor,
    }
    for ext, cls in ext_map.items():
        if ext_counts.get(ext, 0) > 3:
            return cls

    return None


def list_languages() -> list[str]:
    """Return list of supported language names."""
    return ["rust", "python", "typescript", "go", "java", "c", "cpp", "ruby"]


__all__ = ["BaseExtractor", "detect_language", "list_languages",
           "RustExtractor", "PythonExtractor", "TypeScriptExtractor",
           "GoExtractor", "JavaExtractor", "CExtractor", "CppExtractor",
           "RubyExtractor"]
