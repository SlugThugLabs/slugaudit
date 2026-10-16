use super::{EvidenceOrigin, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Whether an evidence item's span was computed — or the honest reason it
/// was not. An absent span is reported as absence, never invented.
pub enum SpanAvailability {
    Present(Span),
    PackOmitted,
    NormalizerUnavailable { reason: String },
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
    Structure,
    Import,
    Export,
    Comment,
    Docstring,
    Symbol,
    Diagnostic,
    Call,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One extracted evidence item before SQL conversion: kind, extraction origin,
/// span availability, and the JSON payload the `evidence` table stores.
pub struct EvidenceItem {
    pub key: String,
    pub kind: EvidenceKind,
    pub origin: EvidenceOrigin,
    pub span: SpanAvailability,
    pub payload: Value,
}
