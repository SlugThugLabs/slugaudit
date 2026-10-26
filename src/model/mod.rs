//! SlugAudit-owned contracts shared by parsing, storage, and tools.

mod evidence;
mod limit_overrides;
mod limits;
mod parser;
mod profile;
mod source;
mod span;
mod system_ram;

pub use evidence::{EvidenceItem, EvidenceKind, SpanAvailability};
pub(crate) use limits::SQLITE_MAX_VALUE_BYTES;
pub use limits::{EvidenceLimits, ResourceLimits, process_limits};
pub use parser::{
    EvidenceOrigin, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
pub use profile::{AuditProfile, limits_for_profile};
pub use source::SourceIdentity;
pub use span::{LineIndex, Position, Span, SpanError, saturating_u32};
pub use system_ram::{SystemMemoryInfo, detect_system_memory};
