use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, SpanAvailability};

/// One evidence record shaped for storage. The enum-to-text mapping here is
/// the single implementation other code must reuse — never re-derive these
/// strings ad hoc when writing or reading evidence rows.
pub struct EvidenceRow {
    /// Per-file item key, unique within the file.
    pub key: String,
    /// `EvidenceKind` rendered as its SQL text tag.
    pub kind: &'static str,
    /// `EvidenceOrigin` rendered as its SQL text tag.
    pub origin: &'static str,
    /// `SpanAvailability` rendered as its SQL text tag.
    pub span_availability: &'static str,
    /// Zero-based inclusive start byte, or `None` when no span is available.
    pub start_byte: Option<i64>,
    /// Zero-based exclusive end byte, or `None` when no span is available.
    pub end_byte: Option<i64>,
    /// Zero-based start line, or `None` when no span is available.
    pub start_line: Option<i64>,
    /// Zero-based character start column, or `None` when no span is available.
    pub start_column: Option<i64>,
    /// Zero-based end line, or `None` when no span is available.
    pub end_line: Option<i64>,
    /// Zero-based character end column, or `None` when no span is available.
    pub end_column: Option<i64>,
    /// Category-specific JSON payload serialized as text.
    pub payload: String,
}

#[must_use]
/// Maps one normalized `EvidenceItem` onto an `evidence` table row: key, kind,
/// origin, span columns (NULL when no span is available), and the payload JSON.
pub fn to_row(item: &EvidenceItem) -> EvidenceRow {
    // SQLite has no unsigned 64-bit column type. A byte offset past i64::MAX
    // is not a real file size, so such a span is stored as unavailable (NULL
    // coordinates, logged) rather than as a clamped offset quoted as fact.
    let mut span_availability = span_availability_text(&item.span);
    let (start_byte, end_byte, start_line, start_column, end_line, end_column) = match &item.span {
        SpanAvailability::Present(span) => {
            match (i64::try_from(span.start_byte), i64::try_from(span.end_byte)) {
                (Ok(start_byte), Ok(end_byte)) => (
                    Some(start_byte),
                    Some(end_byte),
                    Some(i64::from(span.start.line)),
                    Some(i64::from(span.start.column)),
                    Some(i64::from(span.end.line)),
                    Some(i64::from(span.end.column)),
                ),
                _ => {
                    tracing::warn!(
                        key = %item.key,
                        "evidence span byte offset exceeds i64::MAX; stored as unavailable"
                    );
                    span_availability = "NormalizerUnavailable";
                    (None, None, None, None, None, None)
                }
            }
        }
        _ => (None, None, None, None, None, None),
    };
    EvidenceRow {
        key: item.key.clone(),
        kind: kind_text(&item.kind),
        origin: origin_text(&item.origin),
        span_availability,
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
        EvidenceKind::Call => "Call",
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
        EvidenceOrigin::RustCallWalker => "RustCallWalker",
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
    fn a_span_past_i64_max_is_stored_unavailable_not_clamped() {
        let item = EvidenceItem {
            key: "symbol:1".into(),
            kind: EvidenceKind::Symbol,
            origin: EvidenceOrigin::PackSymbol,
            span: SpanAvailability::Present(
                Span::new(
                    0,
                    u64::MAX,
                    Position { line: 0, column: 0 },
                    Position { line: 0, column: 3 },
                )
                .expect("valid span"),
            ),
            payload: serde_json::json!({"name": "x"}),
        };

        let row = to_row(&item);

        assert_eq!(row.span_availability, "NormalizerUnavailable");
        assert_eq!(row.start_byte, None);
        assert_eq!(row.end_byte, None);
    }

    #[test]
    fn a_missing_span_carries_no_coordinates() {
        let item = EvidenceItem {
            key: "structure:0".into(),
            kind: EvidenceKind::Structure,
            origin: EvidenceOrigin::PackStructure,
            span: SpanAvailability::PackOmitted,
            payload: serde_json::json!({}),
        };
        let row = to_row(&item);
        assert_eq!(row.span_availability, "PackOmitted");
        assert_eq!(row.start_byte, None);
    }
}
