"""Briefing package — structured audit briefing assembly."""

from .providers import (
    TargetFileProvider,
    FindingsProvider,
    OverviewProvider,
    ArchitectureProvider,
    RiskPatternsProvider,
)
from .formatter import (
    fmt_sig,
    format_header,
    format_overview,
    format_architecture,
    format_target_files,
    format_findings,
    format_risk_patterns,
)
from .assembler import BriefingAssembler, assemble_briefing

__all__ = [
    # providers
    "TargetFileProvider",
    "FindingsProvider",
    "OverviewProvider",
    "ArchitectureProvider",
    "RiskPatternsProvider",
    # formatters
    "fmt_sig",
    "format_header",
    "format_overview",
    "format_architecture",
    "format_target_files",
    "format_findings",
    "format_risk_patterns",
    # assembler
    "BriefingAssembler",
    "assemble_briefing",
]
