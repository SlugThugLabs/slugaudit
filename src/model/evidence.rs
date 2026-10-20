use super::{EvidenceOrigin, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Whether an evidence item's span was computed — or the honest reason it
/// was not. An absent span is reported as absence, never invented.
pub enum SpanAvailability {
    /// A span was computed and is available.
    Present(Span),
    /// The language pack supplied no span for the item.
    PackOmitted,
    /// A span was computed by a walker but failed local range validation.
    NormalizerUnavailable {
        /// The validation failure that made the span unavailable.
        reason: String,
    },
    /// The item is derived from other evidence and has no source span of its own.
    DerivedEvidence,
}

// `JsonSchema` is derived in test builds only: it is the one derive that
// turns the variant list into machine-readable data, which is how the store
// test enumerates every kind without a hand-kept copy that a new variant
// could silently miss (see `store::connection_tests`). Production code never
// needs the impl, so the crate's public API does not gain one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
/// The categories of evidence SlugAudit extracts from parsed source.
/// Observed syntax only — no category carries a judgment.
pub enum EvidenceKind {
    /// A structural syntax element reported by a language pack.
    Structure,
    /// An import, include, or equivalent dependency statement.
    Import,
    /// An exported symbol or declaration.
    Export,
    /// A source comment.
    Comment,
    /// A documentation string attached to a declaration.
    Docstring,
    /// A symbol definition or reference.
    Symbol,
    /// A parser- or extractor-reported problem.
    Diagnostic,
    /// A call site.
    Call,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One extracted evidence item before SQL conversion: kind, extraction origin,
/// span availability, and the JSON payload the `evidence` table stores.
pub struct EvidenceItem {
    /// Per-file item key of the form `<kind>:<index>`, unique within the file.
    pub key: String,
    /// Evidence category.
    pub kind: EvidenceKind,
    /// Extractor that produced the item.
    pub origin: EvidenceOrigin,
    /// Span coordinates, or the reason none are available.
    pub span: SpanAvailability,
    /// Category-specific JSON payload persisted in the `evidence` table.
    pub payload: Value,
}
