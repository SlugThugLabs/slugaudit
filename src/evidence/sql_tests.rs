use super::*;
use crate::model::{EvidencePayload, Position, Span, SymbolPayload};
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[test]
fn a_present_span_carries_all_six_coordinates_and_serializes_typed_payload() -> TestResult {
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
            .check("build a valid span")?,
        ),
        payload: EvidencePayload::Symbol(SymbolPayload {
            name: "x".into(),
            kind: "Variable".into(),
            visibility: None,
            type_annotation: Some("u32".into()),
            doc: None,
        }),
    };
    let row = to_row(&item);
    ensure_eq(&row.kind, &"Symbol", "row.kind == \"Symbol\"")?;
    ensure_eq(
        &row.span_availability,
        &"Present",
        "row.span_availability == \"Present\"",
    )?;
    ensure_eq(&row.start_byte, &Some(0), "row.start_byte == Some(0)")?;
    ensure_eq(&row.end_byte, &Some(3), "row.end_byte == Some(3)")?;
    ensure_eq(&row.start_line, &Some(1), "row.start_line == Some(1)")?;
    ensure_eq(&row.end_line, &Some(1), "row.end_line == Some(1)")?;
    let decoded: SymbolPayload = serde_json::from_str(&row.payload).check("decode row payload")?;
    ensure_eq(&decoded.name, &"x", "decoded.name == \"x\"")?;
    ensure_eq(
        &decoded.type_annotation.as_deref(),
        &Some("u32"),
        "decoded.type_annotation == Some(\"u32\")",
    )?;
    Ok(())
}

#[test]
fn a_span_past_i64_max_is_stored_unavailable_not_clamped() -> TestResult {
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
            .check("build a valid span")?,
        ),
        payload: EvidencePayload::sample_for_kind(&EvidenceKind::Symbol),
    };

    let row = to_row(&item);

    ensure_eq(
        &row.span_availability,
        &"NormalizerUnavailable",
        "row.span_availability == \"NormalizerUnavailable\"",
    )?;
    ensure_eq(&row.start_byte, &None, "row.start_byte == None")?;
    ensure_eq(&row.end_byte, &None, "row.end_byte == None")?;
    Ok(())
}

#[test]
fn a_missing_span_carries_no_coordinates() -> TestResult {
    let item = EvidenceItem {
        key: "structure:0".into(),
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: EvidencePayload::sample_for_kind(&EvidenceKind::Structure),
    };
    let row = to_row(&item);
    ensure_eq(
        &row.span_availability,
        &"PackOmitted",
        "row.span_availability == \"PackOmitted\"",
    )?;
    ensure_eq(&row.start_byte, &None, "row.start_byte == None")?;
    Ok(())
}
