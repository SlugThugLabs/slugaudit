use super::normalize::convert_span;
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin};
use serde_json::json;
use tree_sitter_language_pack::{
    CommentInfo, DocstringInfo, ExportInfo, ImportInfo, StructureItem, SymbolInfo,
};

pub(super) fn flatten_structure(
    item: &StructureItem,
    source: &str,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) {
    let key = format!("structure:{counter}");
    *counter += 1;
    items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, source),
        payload: json!({
            "kind": format!("{:?}", item.kind),
            "name": item.name,
            "visibility": item.visibility,
            "decorators": item.decorators,
            "signature": item.signature,
        }),
    });
    for child in &item.children {
        flatten_structure(child, source, items, counter);
    }
}

pub(super) fn import_item(index: usize, item: &ImportInfo, source: &str) -> EvidenceItem {
    EvidenceItem {
        key: format!("import:{index}"),
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, source),
        payload: json!({
            "source": item.source,
            "items": item.items,
            "alias": item.alias,
            "is_wildcard": item.is_wildcard,
        }),
    }
}

pub(super) fn export_item(index: usize, item: &ExportInfo, source: &str) -> EvidenceItem {
    EvidenceItem {
        key: format!("export:{index}"),
        kind: EvidenceKind::Export,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, source),
        payload: json!({"name": item.name, "kind": format!("{:?}", item.kind)}),
    }
}

pub(super) fn comment_item(index: usize, item: &CommentInfo, source: &str) -> EvidenceItem {
    EvidenceItem {
        key: format!("comment:{index}"),
        kind: EvidenceKind::Comment,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, source),
        payload: json!({
            "text": item.text,
            "kind": format!("{:?}", item.kind),
            "associated_node": item.associated_node,
        }),
    }
}

pub(super) fn docstring_item(index: usize, item: &DocstringInfo, source: &str) -> EvidenceItem {
    EvidenceItem {
        key: format!("docstring:{index}"),
        kind: EvidenceKind::Docstring,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, source),
        payload: json!({
            "text": item.text,
            "format": format!("{:?}", item.format),
            "associated_item": item.associated_item,
        }),
    }
}

pub(super) fn symbol_item(index: usize, item: &SymbolInfo, source: &str) -> EvidenceItem {
    EvidenceItem {
        key: format!("symbol:{index}"),
        kind: EvidenceKind::Symbol,
        origin: EvidenceOrigin::PackSymbol,
        span: convert_span(&item.span, source),
        payload: json!({
            "name": item.name,
            "kind": format!("{:?}", item.kind),
            "type_annotation": item.type_annotation,
            "doc": item.doc,
        }),
    }
}

pub(super) fn diagnostic_item(
    index: usize,
    item: &tree_sitter_language_pack::Diagnostic,
    source: &str,
) -> EvidenceItem {
    EvidenceItem {
        key: format!("diagnostic:{index}"),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, source),
        payload: json!({
            "message": item.message,
            "severity": super::normalize::severity_text(&item.severity),
        }),
    }
}
