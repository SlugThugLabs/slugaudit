"""Briefing package — structured audit briefing assembly."""

from .providers import (
    GhostContextProvider,
    TargetFileProvider,
    FindingsProvider,
    OverviewProvider,
    ArchitectureProvider,
)
from .formatter import (
    fmt_sig,
    format_header,
    format_overview,
    format_architecture,
    format_ghost_context,
    format_target_files,
    format_findings,
    format_output_contract,
)
from .assembler import BriefingAssembler, assemble_briefing

__all__ = [
    # providers
    "GhostContextProvider",
    "TargetFileProvider",
    "FindingsProvider",
    "OverviewProvider",
    "ArchitectureProvider",
    # formatters
    "fmt_sig",
    "format_header",
    "format_overview",
    "format_architecture",
    "format_ghost_context",
    "format_target_files",
    "format_findings",
    "format_output_contract",
    # assembler
    "BriefingAssembler",
    "assemble_briefing",
]
