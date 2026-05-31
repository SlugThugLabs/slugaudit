"""Briefing formatters — Markdown generation for each section."""

from datetime import datetime, timezone
from typing import List, Optional, Tuple

from infrastructure import (
    IFileSystem,
    get_file_system,
    validate_path_within,
    MAX_FILE_SIZE,
)


def fmt_sig(sig: dict) -> str:
    """Format a single signature entry into a compact line."""
    vis = sig.get("visibility", "")
    vis_str = f"{vis} " if vis else ""
    name = sig.get("name", "")
    sig_text = sig.get("signature", "")
    if sig_text:
        return f"- {vis_str}{sig_text}"
    return f"- {vis_str}{sig.get('type', '?')} {name}"


def format_header(name: str, changed_paths: list, language: str) -> List[str]:
    """Format the briefing header section."""
    lines = []
    timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    scope = "full" if not changed_paths else "incremental"
    lines.append(f"# Audit Briefing — {name} (Phase 1, {scope.capitalize()})")
    lines.append("")
    lines.append(f"Generated: {timestamp}")
    lines.append(f"Project: {name} ({language})")
    lines.append("")
    return lines


def format_overview(
    language: str,
    stats: dict,
    changed_count: int,
    blast_radius_count: int,
    findings_count: int,
) -> List[str]:
    """Format the project overview section."""
    lines = []
    lines.append("## Project Overview")
    lines.append("")
    lines.append("| Metric | Value |")
    lines.append("|--------|-------|")
    lines.append(f"| Language | {language} |")
    lines.append(f"| Total files | {stats['total_files']} |")
    lines.append(f"| Total size | {stats['total_bytes']/1024:.0f} KB |")
    lines.append(f"| Files with signatures | {stats['files_with_sigs']} |")
    lines.append(f"| Total signatures | {stats['total_sigs']} |")
    lines.append(f"| Imports tracked | {stats['imports_count']} |")
    lines.append(f"| Dependency edges | {stats['edge_count']} |")
    lines.append(f"| Changed files | {changed_count} |")
    lines.append(f"| Blast radius files | {blast_radius_count} |")
    lines.append(f"| Open findings | {findings_count} |")
    lines.append("")
    return lines


def format_architecture(architecture_text: str, language: str) -> List[str]:
    """Format the architecture section."""
    lines = []
    lines.append("## Architecture")
    lines.append("")
    if architecture_text:
        lines.append(architecture_text)
    else:
        lines.append(f"Hexagonal Architecture (Ports and Adapters). {language} project.")
    lines.append("")
    return lines


def format_ghost_context(
    unchanged_with_sigs: List[Tuple[str, list]],
    total_sigs: int,
    max_ghost_lines: int,
) -> List[str]:
    """Format the ghost context section."""
    lines = []
    lines.append(
        f"## GHOST CONTEXT — Unchanged Files "
        f"({len(unchanged_with_sigs)} files, {total_sigs} signatures)"
    )
    lines.append("")
    lines.append(
        "The following files are unchanged. Their public API signatures are provided "
        "for reference. **Do NOT read these files** — use the signatures below."
    )
    lines.append("")

    ghost_line_count = 0
    file_index = 0
    for fpath, sig_cache in unchanged_with_sigs:
        if ghost_line_count >= max_ghost_lines:
            remaining = len(unchanged_with_sigs) - file_index
            lines.append("")
            lines.append(f"*[... {remaining} more files omitted for brevity ...]*")
            break
        file_index += 1
        crate = fpath.split("/")[0] if "/" in fpath else "root"
        lines.append("")
        lines.append(f"### {fpath}")
        for sig in sig_cache:
            s = fmt_sig(sig)
            lines.append(f"{s}")
            ghost_line_count += 1
            if ghost_line_count >= max_ghost_lines:
                break

    lines.append("")
    return lines


def format_target_files(
    changed_paths: List[str],
    blast_radius_paths: set,
    target_paths: List[str],
    repo_path: str,
    language: str,
    file_system: Optional[IFileSystem] = None,
) -> List[str]:
    """Format the target files section with full source code."""
    lines = []
    lines.append("## TARGET FILES — Audit These")
    lines.append("")
    lines.append(f"{len(target_paths)} file(s) to audit:")
    lines.append("")

    if changed_paths:
        lines.append("**Changed files:**")
        for fp in changed_paths:
            lines.append(f"- `{fp}` (CHANGED)")
        lines.append("")

    if blast_radius_paths:
        lines.append("**Blast radius (dependents):**")
        for fp in sorted(blast_radius_paths):
            lines.append(f"- `{fp}` (BLAST RADIUS)")
        lines.append("")

    # Canonicalize repo_path once for path traversal checks
    canonical_repo = repo_path  # Already validated by caller

    # Full source code of target files
    fs = file_system or get_file_system()
    for fp in target_paths:
        lines.append(f"### {fp}")
        label = "CHANGED" if fp in changed_paths else "BLAST RADIUS"
        lines.append(f"*Status: {label}*")
        lines.append("")
        lines.append(f"```{language}")
        try:
            # Path traversal protection
            abs_path = validate_path_within(fp, repo_path)
            file_size = fs.get_file_size(abs_path)
            if file_size > MAX_FILE_SIZE:
                lines.append(
                    f"// File skipped: size {file_size} bytes exceeds "
                    f"{MAX_FILE_SIZE} byte limit"
                )
            else:
                content = fs.read_file_bytes(abs_path)
                lines.append(content.decode("utf-8", errors="replace").rstrip())
        except ValueError as e:
            lines.append(f"*Skipped: {e}*")
        except Exception as e:
            lines.append(f"// Error reading file: {e}")
        lines.append(f"```")
        lines.append("")

    return lines


def format_findings(
    findings_list: List[Tuple[str, Optional[int], Optional[int], str, str, str]]
) -> List[str]:
    """Format the historical findings section."""
    lines = []
    lines.append("## Historical Findings")
    lines.append("")
    if findings_list:
        for fpath, ls, le, sev, cat, msg in findings_list:
            short_title = msg[:100] if msg else "No message"
            if len(msg) > 100:
                short_title += "..."
            lines.append(
                f"- **[{sev.upper()}]** `{fpath}:{ls}-{le}` — {short_title}"
            )
            lines.append("")
    else:
        lines.append("No open findings on record.")
        lines.append("")
    return lines


def format_output_contract() -> List[str]:
    """Format the output contract section."""
    lines = []
    lines.append("## Output Contract")
    lines.append("")
    lines.append("Return findings in this exact JSON format:")
    lines.append("")
    lines.append("```json")
    lines.append("{")
    lines.append('  "findings": [')
    lines.append("    {")
    lines.append('      "file": "path/to/file.rs",')
    lines.append('      "line_start": 42,')
    lines.append('      "line_end": 55,')
    lines.append('      "severity": "high",')
    lines.append('      "category": "correctness",')
    lines.append('      "title": "Short descriptive title",')
    lines.append('      "description": "Full explanation of the issue..."')
    lines.append("    }")
    lines.append("  ]")
    lines.append("}")
    lines.append("```")
    lines.append("")
    return lines


__all__ = [
    "fmt_sig",
    "format_header",
    "format_overview",
    "format_architecture",
    "format_ghost_context",
    "format_target_files",
    "format_findings",
    "format_output_contract",
]
