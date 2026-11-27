//! SlugAudit-owned contracts shared by parsing, storage, and tools.

mod evidence;
mod limit_overrides;
mod limits;
mod parser;
mod profile;
mod span;
mod system_ram;

pub use evidence::{
    CallPayload, CommentPayload, DiagnosticPayload, DocstringPayload, EvidenceItem, EvidenceKind,
    EvidencePayload, ExportPayload, ImportPayload, RustModPathPayload, SourceIdentity,
    SpanAvailability, StructurePayload, SymbolPayload,
};
#[cfg(test)]
pub(crate) use limit_overrides::fresh_ledger;
pub use limit_overrides::{EnvConfigError, LimitOverrides};
pub(crate) use limit_overrides::{EnvOverrides, OverrideSource, env_overrides};
pub(crate) use limits::SQLITE_MAX_VALUE_BYTES;
pub use limits::{EvidenceLimits, ResourceLimits};
pub use parser::{
    EvidenceOrigin, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
pub(crate) use profile::adaptive_limits;
pub use profile::{AuditProfile, limits_for_profile};
pub use span::{LineIndex, Position, PositionOverflow, Span, SpanError, position_u32};
pub use system_ram::{MemoryDetectError, SystemMemoryInfo, detect_system_memory};
