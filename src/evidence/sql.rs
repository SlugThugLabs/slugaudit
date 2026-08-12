use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, SpanAvailability};

/// One evidence record shaped for storage. The enum-to-text mapping here is
/// the single implementation other code must reuse — never re-derive these
/// strings ad hoc when writing or reading evidence rows.
pub struct EvidenceRow {
    pub key: String,
    pub kind: &'static str,
    pub origin: &'static str,
    pub span_availability: &'static str,
    pub start_byte: Option<i64>,
    pub end_byte: Option<i64>,
    pub start_line: Option<i64>,
    pub start_column: Option<i64>,
    pub end_line: Option<i64>,
    pub end_column: Option<i64>,
    pub payload: String,
}

#[must_use]
pub fn to_row(item: &EvidenceItem) -> EvidenceRow {
    // SQLite has no unsigned 64-bit column type; byte offsets past i64::MAX
    // are not a real file size, so saturating here is a documented choice,
    // not a silent truncation risk.
    let byte_to_i64 = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let (start_byte, end_byte, start_line, start_column, end_line, end_column) = match &item.span {
        SpanAvailability::Present(span) => (
            Some(byte_to_i64(span.start_byte)),
            Some(byte_to_i64(span.end_byte)),
            Some(i64::from(span.start.line)),
            Some(i64::from(span.start.column)),
            Some(i64::from(span.end.line)),
            Some(i64::from(span.end.column)),
        ),
        _ => (None, None, None, None, None, None),
    };
    EvidenceRow {
        key: item.key.clone(),
        kind: kind_text(&item.kind),
        origin: origin_text(&item.origin),
        span_availability: span_availability_text(&item.span),
        start_byte,
        end_byte,
        start_line,
        start_column,
        end_line,
        end_column,
        payload: item.payload.to_string(),
    }
}

fn kind_text(kind: &EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::Structure => "Structure",
        EvidenceKind::Import => "Import",
        EvidenceKind::Export => "Export",
        EvidenceKind::Comment => "Comment",
        EvidenceKind::Docstring => "Docstring",
        EvidenceKind::Symbol => "Symbol",
        EvidenceKind::Diagnostic => "Diagnostic",
        EvidenceKind::Chunk => "Chunk",
        EvidenceKind::RawNode => "RawNode",
        EvidenceKind::Metric => "Metric",
    }
}

fn origin_text(origin: &EvidenceOrigin) -> &'static str {
    match origin {
        EvidenceOrigin::PackStructure => "PackStructure",
        EvidenceOrigin::PackSymbol => "PackSymbol",
        EvidenceOrigin::RawTree => "RawTree",
        EvidenceOrigin::SourceContent => "SourceContent",
        EvidenceOrigin::DerivedRelationship => "DerivedRelationship",
        EvidenceOrigin::GenericWalker => "GenericWalker",
    }
}

fn span_availability_text(span: &SpanAvailability) -> &'static str {
    match span {
        SpanAvailability::Present(_) => "Present",
        SpanAvailability::PackOmitted => "PackOmitted",
        SpanAvailability::NormalizerUnavailable { .. } => "NormalizerUnavailable",
        SpanAvailability::DerivedEvidence => "DerivedEvidence",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Position, Span};

    #[test]
    fn a_present_span_carries_all_six_coordinates() {
        let item = EvidenceItem {
            key: "symbol:0".into(),
            kind: EvidenceKind::Symbol,
            origin: EvidenceOrigin::PackSymbol,
            span: SpanAvailability::Present(
                Span::new(
                    0,
                    3,
                    Position { line: 0, column: 0 },
                    Position { line: 0, column: 3 },
                )
                .expect("valid span"),
            ),
            payload: serde_json::json!({"name": "x"}),
        };
        let row = to_row(&item);
        assert_eq!(row.kind, "Symbol");
        assert_eq!(row.span_availability, "Present");
        assert_eq!(row.start_byte, Some(0));
        assert_eq!(row.end_byte, Some(3));
    }

    #[test]
    fn a_missing_span_carries_no_coordinates() {
        let item = EvidenceItem {
            key: "chunk:0".into(),
            kind: EvidenceKind::Chunk,
            origin: EvidenceOrigin::PackStructure,
            span: SpanAvailability::PackOmitted,
            payload: serde_json::json!({}),
        };
        let row = to_row(&item);
        assert_eq!(row.span_availability, "PackOmitted");
        assert_eq!(row.start_byte, None);
    }
}
