"""Briefing formatters — Markdown generation for each section."""

from typing import Any

from infrastructure import (
    IFileSystem,
    get_file_system,
    validate_path_within,
    MAX_FILE_SIZE,
)


def fmt_sig(sig: dict[str, Any]) -> str:
    """Format a single signature entry into a compact line."""
    vis = sig.get("visibility", "")
    vis_str = f"{vis} " if vis else ""
    name = sig.get("name", "")
    sig_text = sig.get("signature", "")
    if sig_text:
        return f"- {vis_str}{sig_text}"
    return f"- {vis_str}{sig.get('type', '?')} {name}"


def format_header(name: str, changed_paths: list[str], language: str) -> list[str]:
    """Format the briefing header — dense, no fluff."""
    return [
        f"# SIGNALS — {name} ({language})",
        "",
    ]


def format_overview(
    language: str,
    stats: dict[str, Any],
    changed_count: int,
    blast_radius_count: int,
    findings_count: int,
) -> list[str]:
    """Format the project stats as a single dense line."""
    lines: list[Any] = []
    lines.append("## STATS")
    lines.append("")
    lines.append(
        f"files={stats['total_files']}  "
        f"sigs={stats['total_sigs']}  "
        f"imports={stats['imports_count']}  "
        f"edges={stats['edge_count']}  "
        f"bytes={stats['total_bytes']}  "
        f"changed={changed_count}  "
        f"blast_radius={blast_radius_count}  "
        f"findings={findings_count}"
    )
    lines.append("")
    return lines


def format_architecture(architecture_text: str, language: str) -> list[str]:
    """Format the architecture section."""
    lines: list[Any] = []
    lines.append("## Architecture")
    lines.append("")
    if architecture_text:
        lines.append(architecture_text)
    else:
        lines.append("No architecture summary has been recorded.")
    lines.append("")
    return lines


def format_target_files(
    changed_paths: list[str],
    blast_radius_paths: set[str],
    target_paths: list[str],
    repo_path: str,
    language: str,
    file_system: IFileSystem | None = None,
) -> list[str]:
    """Format the target files section with full source code."""
    lines: list[Any] = []
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
        lines.append("```")
        lines.append("")

    return lines


def format_findings(
    findings_list: list[tuple[str, int | None, int | None, str, str, str]]
) -> list[str]:
    """Format the historical findings section."""
    lines: list[Any] = []
    lines.append("## Historical Findings")
    lines.append("")
    if findings_list:
        for fpath, ls, le, sev, _cat, msg in findings_list:
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


def format_risk_patterns(
    file_patterns: list[tuple[str, list[tuple[str, int]]]],
    summary: dict[str, int],
) -> list[str]:
    """Format the risk patterns section as dense structured data.

    Output is optimized for LLM consumption — no prose, just signals.
    """
    lines: list[Any] = []

    if not file_patterns:
        return lines

    lines.append("## RISK_PATTERNS")
    lines.append("")

    # Summary line: total counts per pattern type
    if summary:
        summary_parts = "  ".join(f"{k}={v}" for k, v in summary.items())
        lines.append(f"<summary>{summary_parts}</summary>")
        lines.append("")

    # Per-file patterns, sorted by risk (highest first)
    for fpath, patterns in file_patterns:
        pattern_str = " ".join(f"{name}={count}" for name, count in patterns)
        lines.append(f"  {fpath}: {pattern_str}")

    lines.append("")
    return lines


__all__ = [
    "fmt_sig",
    "format_header",
    "format_overview",
    "format_architecture",
    "format_target_files",
    "format_findings",
]
