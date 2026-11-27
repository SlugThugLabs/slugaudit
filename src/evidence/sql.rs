use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, SpanAvailability};

/// One evidence record shaped for storage. The enum-to-text mapping here is
/// the single implementation other code must reuse — never re-derive these
/// strings ad hoc when writing or reading evidence rows.
pub(crate) struct EvidenceRow {
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
    /// One-based start line, or `None` when no span is available.
    pub start_line: Option<i64>,
    /// Zero-based character start column, or `None` when no span is available.
    pub start_column: Option<i64>,
    /// One-based, inclusive end line, or `None` when no span is available.
    pub end_line: Option<i64>,
    /// Zero-based character end column, or `None` when no span is available.
    pub end_column: Option<i64>,
    /// Category-specific JSON payload serialized as text.
    pub payload: String,
}

#[must_use]
/// Maps one normalized `EvidenceItem` onto an `evidence` table row: key, kind,
/// origin, span columns (NULL when no span is available), and the payload JSON.
pub(crate) fn to_row(item: &EvidenceItem) -> EvidenceRow {
    // SQLite has no unsigned 64-bit column type. A byte offset past i64::MAX
    // is not a real file size, so such a span is stored as unavailable (NULL
    // coordinates, logged) rather than as a clamped offset quoted as fact.
    let mut span_availability = span_availability_text(&item.span);
    let (start_byte, end_byte, start_line, start_column, end_line, end_column) =
        if let SpanAvailability::Present(span) = &item.span {
            if let (Ok(start_byte), Ok(end_byte), Some(start_line), Some(end_line)) = (
                i64::try_from(span.start_byte),
                i64::try_from(span.end_byte),
                one_based_line(span.start.line),
                one_based_line(span.end.line),
            ) {
                (
                    Some(start_byte),
                    Some(end_byte),
                    Some(start_line),
                    Some(i64::from(span.start.column)),
                    Some(end_line),
                    Some(i64::from(span.end.column)),
                )
            } else {
                tracing::warn!(
                    key = %item.key,
                    "evidence span coordinate exceeds the storable range; stored as unavailable"
                );
                span_availability = "NormalizerUnavailable";
                (None, None, None, None, None, None)
            }
        } else {
            (None, None, None, None, None, None)
        };
    let payload = match serde_json::to_string(&item.payload) {
        Ok(json) => json,
        Err(error) => {
            tracing::warn!(
                key = %item.key,
                %error,
                "evidence payload serialization failed; storing empty JSON object"
            );
            "{}".to_owned()
        }
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
        payload,
    }
}

/// The single zero-based to one-based line conversion on the write path:
/// `model::span` lines are zero-based, the `evidence` table stores them
/// one-based and inclusive. `None` if the sum does not fit (it always does:
/// a `u32` plus one fits an `i64`), which the caller stores as unavailable.
fn one_based_line(zero_based: u32) -> Option<i64> {
    i64::from(zero_based).checked_add(1)
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
        SpanAvailability::NormalizerUnavailable => "NormalizerUnavailable",
        SpanAvailability::DerivedEvidence => "DerivedEvidence",
    }
}

#[cfg(test)]
#[path = "sql_tests.rs"]
mod tests;
