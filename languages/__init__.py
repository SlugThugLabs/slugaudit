"""Language registry and auto-detection."""

import os

from .base import BaseExtractor
from .rust import RustExtractor
from .python import PythonExtractor
from .typescript import TypeScriptExtractor
from .go import GoExtractor
from .java import JavaExtractor
from .c import CExtractor
from .cpp import CppExtractor
from .ruby import RubyExtractor


# Language name → Extractor class mapping
ExtractorType = (
    type[RustExtractor]
    | type[PythonExtractor]
    | type[TypeScriptExtractor]
    | type[GoExtractor]
    | type[JavaExtractor]
    | type[CExtractor]
    | type[CppExtractor]
    | type[RubyExtractor]
)


LANG_MAP: dict[str, ExtractorType] = {
    "rust": RustExtractor,
    "python": PythonExtractor,
    "typescript": TypeScriptExtractor,
    "go": GoExtractor,
    "java": JavaExtractor,
    "c": CExtractor,
    "cpp": CppExtractor,
    "ruby": RubyExtractor,
}


_EXTENSION_LANGUAGE = {
    ".rs": "rust",
    ".py": "python",
    ".pyi": "python",
    ".ts": "typescript",
    ".tsx": "typescript",
    ".js": "typescript",
    ".jsx": "typescript",
    ".mjs": "typescript",
    ".cjs": "typescript",
    ".go": "go",
    ".java": "java",
    ".c": "c",
    ".h": "c",
    ".cpp": "cpp",
    ".hpp": "cpp",
    ".cc": "cpp",
    ".cxx": "cpp",
    ".hh": "cpp",
    ".hxx": "cpp",
    ".ixx": "cpp",
    ".tpp": "cpp",
    ".rb": "ruby",
    ".rake": "ruby",
    ".gemspec": "ruby",
}


def language_for_path(path: str) -> str | None:
    """Return the supported language for a source path."""
    return _EXTENSION_LANGUAGE.get(os.path.splitext(path)[1].lower())


def supported_extensions() -> frozenset[str]:
    """Return every source extension handled by the parser registry."""
    return frozenset(_EXTENSION_LANGUAGE)


def get_extractor_for_path(project_root: str, path: str) -> BaseExtractor:
    """Construct the language extractor appropriate for one source path."""
    language = language_for_path(path)
    if language is None:
        raise ValueError(f"Unsupported source file: {path}")
    return LANG_MAP[language](project_root)


def get_extractor(project_root: str, language: str | None) -> BaseExtractor:
    """Resolve the extractor class for the given language and project root.

    Args:
        project_root: Absolute path to the project directory.
        language: Language name ("rust", "python", "typescript") or "auto".

    Returns:
        An extractor instance.

    Raises:
        ValueError: If the language cannot be detected or is unsupported.
    """
    if language and language != "auto":
        cls = LANG_MAP.get(language)
        if not cls:
            raise ValueError(
                f"Unsupported language: {language}. Supported: {list_languages()}"
            )
        return cls(project_root)

    detected = detect_language(project_root)
    if detected:
        return detected(project_root)

    raise ValueError(
        "Could not detect language. Specify with language parameter: "
        f"{list_languages()}"
    )


def detect_language(project_root: str) -> type[BaseExtractor] | None:
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
    ext_counts: dict[str, int] = {}
    for _dirpath, dirnames, filenames in os.walk(project_root):
        dirnames[:] = [
            directory
            for directory in dirnames
            if not directory.startswith(".")
            and directory not in {"target", "node_modules"}
        ]
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


__all__ = [
    "BaseExtractor",
    "detect_language",
    "list_languages",
    "get_extractor",
    "LANG_MAP",
    "get_extractor_for_path",
    "language_for_path",
    "supported_extensions",
    "RustExtractor",
    "PythonExtractor",
    "TypeScriptExtractor",
    "GoExtractor",
    "JavaExtractor",
    "CExtractor",
    "CppExtractor",
    "RubyExtractor",
]

# ── Re-exports for backward compatibility (used by tests) ──
