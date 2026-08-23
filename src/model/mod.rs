//! SlugAudit-owned contracts shared by parsing, storage, and tools.

mod evidence;
mod limits;
mod parser;
mod source;
mod span;

pub use evidence::{EvidenceItem, EvidenceKind, SpanAvailability};
pub use limits::{EvidenceLimits, ResourceLimits, process_limits};
pub use parser::{
    EvidenceOrigin, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
pub use source::{FileMetadata, LanguageSelection, SourceIdentity, SourceSnapshot};
pub use span::{Position, Span, SpanError, char_column, saturating_u32};
