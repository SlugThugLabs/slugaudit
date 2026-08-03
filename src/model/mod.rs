//! SlugAudit-owned contracts shared by parsing, storage, and tools.

mod capability;
mod evidence;
mod freshness;
mod limits;
mod parser;
mod source;
mod span;

pub use capability::{
    EvidenceCapabilities, GrammarQueryCapabilities, LanguageCapabilityReport, SupportState,
};
pub use evidence::{EvidenceItem, EvidenceKind, EvidenceSet, SpanAvailability};
pub use freshness::{FreshnessInput, VerifiedRevision};
pub use limits::{EvidenceLimits, ResourceLimits};
pub use parser::{
    EvidenceOrigin, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
pub use source::{FileMetadata, LanguageSelection, SourceIdentity, SourceSnapshot};
pub use span::{Position, Span, SpanError, saturating_u32};
