//! SlugAudit-owned contracts shared by parsing, storage, and tools.

mod evidence;
mod limits;
mod parser;
mod profile;
mod source;
mod span;
mod system_ram;

pub use evidence::{EvidenceItem, EvidenceKind, SpanAvailability};
pub use limits::{EvidenceLimits, ResourceLimits, process_limits};
pub use parser::{
    EvidenceOrigin, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
pub use profile::{AuditProfile, limits_for_profile};
pub use source::{FileMetadata, LanguageSelection, SourceIdentity, SourceSnapshot};
pub use span::{Position, Span, SpanError, char_column, saturating_u32};
pub use system_ram::{SystemMemoryInfo, detect_system_memory};
