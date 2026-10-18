use super::normalize::convert_span;
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, LineIndex};
use serde_json::json;
use tree_sitter_language_pack::{
    CommentInfo, DocstringInfo, ExportInfo, ImportInfo, StructureItem, SymbolInfo,
};

/// Flattens one pack structure tree in pre-order. A loop over a heap stack,
/// not recursion: the tree's nesting follows file content, and this walk
/// must not be the one that overflows the thread stack on it.
pub(super) fn flatten_structure(
    root: &StructureItem,
    lines: &LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) {
    let mut pending = vec![root];
    while let Some(item) = pending.pop() {
        let key = format!("structure:{counter}");
        *counter += 1;
        items.push(EvidenceItem {
            key,
            kind: EvidenceKind::Structure,
            origin: EvidenceOrigin::PackStructure,
            span: convert_span(&item.span, lines),
            payload: json!({
                "kind": format!("{:?}", item.kind),
                "name": item.name,
                "visibility": item.visibility,
                "decorators": item.decorators,
                "signature": item.signature,
            }),
        });
        pending.extend(item.children.iter().rev());
    }
}

pub(super) fn import_item(index: usize, item: &ImportInfo, lines: &LineIndex<'_>) -> EvidenceItem {
    EvidenceItem {
        key: format!("import:{index}"),
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines),
        payload: json!({
            "source": item.source,
            "items": item.items,
            "alias": item.alias,
            "is_wildcard": item.is_wildcard,
        }),
    }
}

pub(super) fn export_item(index: usize, item: &ExportInfo, lines: &LineIndex<'_>) -> EvidenceItem {
    EvidenceItem {
        key: format!("export:{index}"),
        kind: EvidenceKind::Export,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines),
        payload: json!({"name": item.name, "kind": format!("{:?}", item.kind)}),
    }
}

pub(super) fn comment_item(
    index: usize,
    item: &CommentInfo,
    lines: &LineIndex<'_>,
) -> EvidenceItem {
    EvidenceItem {
        key: format!("comment:{index}"),
        kind: EvidenceKind::Comment,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines),
        payload: json!({
            "text": item.text,
            "kind": format!("{:?}", item.kind),
            "associated_node": item.associated_node,
        }),
    }
}

pub(super) fn docstring_item(
    index: usize,
    item: &DocstringInfo,
    lines: &LineIndex<'_>,
) -> EvidenceItem {
    EvidenceItem {
        key: format!("docstring:{index}"),
        kind: EvidenceKind::Docstring,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines),
        payload: json!({
            "text": item.text,
            "format": format!("{:?}", item.format),
            "associated_item": item.associated_item,
        }),
    }
}

pub(super) fn symbol_item(index: usize, item: &SymbolInfo, lines: &LineIndex<'_>) -> EvidenceItem {
    EvidenceItem {
        key: format!("symbol:{index}"),
        kind: EvidenceKind::Symbol,
        origin: EvidenceOrigin::PackSymbol,
        span: convert_span(&item.span, lines),
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
    lines: &LineIndex<'_>,
) -> EvidenceItem {
    EvidenceItem {
        key: format!("diagnostic:{index}"),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines),
        payload: json!({
            "message": item.message,
            "severity": super::normalize::severity_text(&item.severity),
        }),
    }
}
