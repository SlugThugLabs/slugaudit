use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, Position, Span, SpanAvailability, saturating_u32,
};
use serde_json::json;
use tree_sitter_language_pack::{
    CommentInfo, DiagnosticSeverity, DocstringInfo, Error as PackError, ExportInfo, ImportInfo,
    ProcessConfig, Span as PackSpan, StructureItem, SymbolInfo, process,
};

/// Runs the language pack's generic `process()` intelligence and flattens
/// its result into SlugAudit's own evidence records. This is the only
/// place a pack type crosses into SlugAudit's model — payloads are built
/// field-by-field so our evidence contract never silently follows
/// upstream's own wire format.
///
/// # Errors
///
/// Returns an error if `language` is not known to the pack or parsing
/// fails outright; a tree with syntax-error nodes is not an error here, it
/// surfaces as `Diagnostic` evidence instead.
pub fn extract(language: &str, source: &str) -> Result<Vec<EvidenceItem>, PackError> {
    let config = ProcessConfig::new(language).all();
    let result = process(source, &config)?;

    let mut items = Vec::new();
    let mut counter = 0usize;
    for item in &result.structure {
        flatten_structure(item, &mut items, &mut counter);
    }
    for (index, item) in result.imports.iter().enumerate() {
        items.push(import_item(index, item));
    }
    for (index, item) in result.exports.iter().enumerate() {
        items.push(export_item(index, item));
    }
    for (index, item) in result.comments.iter().enumerate() {
        items.push(comment_item(index, item));
    }
    for (index, item) in result.docstrings.iter().enumerate() {
        items.push(docstring_item(index, item));
    }
    for (index, item) in result.symbols.iter().enumerate() {
        items.push(symbol_item(index, item));
    }
    for (index, diagnostic) in result.diagnostics.iter().enumerate() {
        items.push(EvidenceItem {
            key: format!("diagnostic:{index}"),
            kind: EvidenceKind::Diagnostic,
            origin: EvidenceOrigin::PackStructure,
            span: convert_span(&diagnostic.span),
            payload: json!({
                "message": diagnostic.message,
                "severity": severity_text(&diagnostic.severity),
            }),
        });
    }
    Ok(items)
}

fn convert_span(span: &PackSpan) -> SpanAvailability {
    let start = Position {
        line: saturating_u32(span.start_line),
        column: saturating_u32(span.start_column),
    };
    let end = Position {
        line: saturating_u32(span.end_line),
        column: saturating_u32(span.end_column),
    };
    match Span::new(span.start_byte as u64, span.end_byte as u64, start, end) {
        Ok(converted) => SpanAvailability::Present(converted),
        Err(_) => SpanAvailability::NormalizerUnavailable {
            reason: "pack span failed local range validation".into(),
        },
    }
}

fn severity_text(severity: &DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Info => "info",
    }
}

fn flatten_structure(item: &StructureItem, items: &mut Vec<EvidenceItem>, counter: &mut usize) {
    let key = format!("structure:{counter}");
    *counter += 1;
    items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span),
        payload: json!({
            "kind": format!("{:?}", item.kind),
            "name": item.name,
            "visibility": item.visibility,
            "decorators": item.decorators,
            "signature": item.signature,
        }),
    });
    for child in &item.children {
        flatten_structure(child, items, counter);
    }
}

fn import_item(index: usize, item: &ImportInfo) -> EvidenceItem {
    EvidenceItem {
        key: format!("import:{index}"),
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span),
        payload: json!({
            "source": item.source,
            "items": item.items,
            "alias": item.alias,
            "is_wildcard": item.is_wildcard,
        }),
    }
}

fn export_item(index: usize, item: &ExportInfo) -> EvidenceItem {
    EvidenceItem {
        key: format!("export:{index}"),
        kind: EvidenceKind::Export,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span),
        payload: json!({
            "name": item.name,
            "kind": format!("{:?}", item.kind),
        }),
    }
}

fn comment_item(index: usize, item: &CommentInfo) -> EvidenceItem {
    EvidenceItem {
        key: format!("comment:{index}"),
        kind: EvidenceKind::Comment,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span),
        payload: json!({
            "text": item.text,
            "kind": format!("{:?}", item.kind),
            "associated_node": item.associated_node,
        }),
    }
}

fn docstring_item(index: usize, item: &DocstringInfo) -> EvidenceItem {
    EvidenceItem {
        key: format!("docstring:{index}"),
        kind: EvidenceKind::Docstring,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span),
        payload: json!({
            "text": item.text,
            "format": format!("{:?}", item.format),
            "associated_item": item.associated_item,
        }),
    }
}

fn symbol_item(index: usize, item: &SymbolInfo) -> EvidenceItem {
    EvidenceItem {
        key: format!("symbol:{index}"),
        kind: EvidenceKind::Symbol,
        origin: EvidenceOrigin::PackSymbol,
        span: convert_span(&item.span),
        payload: json!({
            "name": item.name,
            "kind": format!("{:?}", item.kind),
            "type_annotation": item.type_annotation,
            "doc": item.doc,
        }),
    }
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;
