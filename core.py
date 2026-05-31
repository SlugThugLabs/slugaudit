"""
core.py — Shared audit logic used by both CLI and MCP server.

This module re-exports key functions from the new package structure
for backward compatibility. New code should import from services/,
repositories/, and languages/ directly.
"""

import os
import sys
from typing import List, Dict, Any, Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# Re-export ImportResult from domain
from domain import ImportResult

# Re-export import_project from services
from services.import_service import import_project

# Re-export language detection and extractor resolution
from languages import detect_language, list_languages, get_extractor

# Re-export the LANG_MAP for backward compatibility
from languages import LANG_MAP


def _build_sig_cache(sigs: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Convert raw signature dicts into the JSON-cacheable format."""
    result = []
    for sig in sigs:
        entry = {
            "type": sig.get("type", "unknown"),
            "name": sig.get("name", ""),
            "visibility": sig.get("visibility", ""),
            "signature": sig.get("signature", ""),
        }
        if sig.get("generic_params"):
            entry["generic_params"] = sig["generic_params"]
        result.append(entry)
    return result


def _build_import_records(imps: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Convert raw import dicts into database import records."""
    return [
        {
            "import_text": imp["import_text"],
            "resolved_path": None,
            "import_type": imp.get("import_type", "internal"),
            "line_start": imp.get("line_start"),
            "line_end": imp.get("line_end"),
        }
        for imp in imps
    ]


__all__ = [
    "ImportResult",
    "import_project",
    "get_extractor",
    "detect_language",
    "list_languages",
    "LANG_MAP",
    "_build_sig_cache",
    "_build_import_records",
]
