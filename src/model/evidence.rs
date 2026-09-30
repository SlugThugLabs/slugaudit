use super::{EvidenceOrigin, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub enum EvidenceKind {
    Structure,
    Import,
    Export,
    Comment,
    Docstring,
    Symbol,
    Diagnostic,
    Chunk,
    RawNode,
    Metric,
    Call,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub key: String,
    pub kind: EvidenceKind,
    pub origin: EvidenceOrigin,
    pub span: SpanAvailability,
    pub payload: Value,
}
