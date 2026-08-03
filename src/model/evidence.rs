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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub key: String,
    pub kind: EvidenceKind,
    pub origin: EvidenceOrigin,
    pub span: SpanAvailability,
    pub payload: Value,
}
