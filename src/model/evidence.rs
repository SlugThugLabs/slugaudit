use super::{EvidenceLimits, EvidenceOrigin, Span};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EvidenceSet {
    pub items: Vec<EvidenceItem>,
    pub truncated: bool,
}

impl EvidenceSet {
    #[must_use]
    pub fn within_limits(&self, limits: EvidenceLimits) -> bool {
        self.items.len() <= limits.max_items_per_file
            && self.items.iter().all(|item| {
                serde_json::to_vec(&item.payload)
                    .is_ok_and(|bytes| bytes.len() <= limits.max_payload_bytes_per_item)
            })
            && serde_json::to_vec(self)
                .is_ok_and(|bytes| bytes.len() <= limits.max_payload_bytes_per_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_checked_without_mutating_evidence() {
        let evidence = EvidenceSet {
            items: vec![EvidenceItem {
                key: "file:metric:0".into(),
                kind: EvidenceKind::Metric,
                origin: EvidenceOrigin::PackStructure,
                span: SpanAvailability::PackOmitted,
                payload: serde_json::json!({"lines": 3}),
            }],
            truncated: false,
        };
        assert!(evidence.within_limits(EvidenceLimits {
            max_items_per_file: 1,
            max_payload_bytes_per_item: 100,
            max_payload_bytes_per_file: 500,
        }));
    }
}
