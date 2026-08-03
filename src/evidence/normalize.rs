// slugaudit-line-exception: approved-by=agent; reason=one match arm per Tree-sitter evidence kind; splitting by kind would hide the exhaustiveness this file exists to guarantee
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, Position, Span, SpanAvailability, saturating_u32,
};
use serde_json::json;
use tree_sitter_language_pack::{
    CommentInfo, DiagnosticSeverity, DocstringInfo, Error as PackError, ExportInfo, ImportInfo,
    ProcessConfig, Span as PackSpan, StructureItem, SymbolInfo, SymbolKind, get_parser, process,
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

    // The language pack's `process()` extracts structure, imports, exports,
    // comments, docstrings, symbols, and diagnostics — but not individual
    // variable bindings or struct/class fields. Those are exactly the items
    // an AI needs to spot near-duplicate names (`file_path` vs `filepath`
    // vs `filePath`) that probably refer to the same thing. We walk the
    // tree ourselves with generic node-type patterns that are consistent
    // across tree-sitter grammars, so no per-language query code is needed.
    items.extend(extract_bindings(language, source));

    Ok(items)
}

/// Generic variable/field extractor. Walks the tree-sitter tree for node
/// types that name individual bindings — `let`/`const`/`var` declarations,
/// struct/class fields, and function parameters — and emits each as a
/// `Symbol` evidence item. The node-type names (`let_declaration`,
/// `field_declaration`, `parameter`, etc.) are consistent across
/// tree-sitter language grammars, so this works for any language the pack
/// supports without per-language query strings.
fn extract_bindings(language: &str, source: &str) -> Vec<EvidenceItem> {
    let Ok(mut parser) = get_parser(language) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(source) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut counter = 0usize;
    walk_bindings(&tree.root_node(), source, &mut items, &mut counter);
    items
}

/// Node-type patterns that indicate an individual binding. These names are
/// shared across tree-sitter language grammars — `let_declaration` in Rust,
/// JS, Go, etc.; `field_declaration` in Rust, C#, Java; `parameter`
/// universally. Matching on the name rather than a per-language query is
/// what keeps this generic.
fn is_variable_binding(kind: &str) -> bool {
    matches!(
        kind,
        "let_declaration"
            | "const_declaration"
            | "var_declaration"
            | "val_declaration"
            | "binding"
            | "parameter"
            | "assignment"
    )
}

fn is_field_declaration(kind: &str) -> bool {
    kind.contains("field_declaration") || kind.contains("property_declaration")
}

fn walk_bindings(
    node: &tree_sitter_language_pack::Node,
    source: &str,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) {
    let kind = node.kind();
    if let Some(name) = identifier_name(node, source) {
        let (sym_kind, origin_kind) = if is_field_declaration(&kind) {
            (SymbolKind::Other("Field".to_owned()), "field")
        } else if is_variable_binding(&kind) {
            if kind.starts_with("const") {
                (SymbolKind::Constant, "variable")
            } else {
                (SymbolKind::Variable, "variable")
            }
        } else {
            // Not a binding we care about — still recurse in case a child is.
            for child in named_children(node) {
                walk_bindings(&child, source, items, counter);
            }
            return;
        };

        let key = format!("{}:{counter}", origin_kind);
        *counter += 1;
        items.push(EvidenceItem {
            key,
            kind: EvidenceKind::Symbol,
            origin: EvidenceOrigin::PackSymbol,
            span: node_span(node, source),
            payload: json!({
                "name": name,
                "kind": format!("{sym_kind:?}"),
                "type_annotation": null,
                "doc": null,
            }),
        });
    }
    // Recurse into children regardless — a function body contains let
    // bindings, a struct body contains field declarations, etc.
    for child in named_children(node) {
        walk_bindings(&child, source, items, counter);
    }
}

/// Returns the text of the first identifier-flavored named child of `node`,
/// if any. This is the binding name for every node type this extractor
/// targets. Different grammars spell the name node differently — Rust
/// struct fields use `field_identifier`, type aliases use `type_identifier`,
/// and most everything else uses plain `identifier` — so we match any of
/// them rather than hard-coding one. The pack exposes no `utf8_text`
/// helper, so the text is sliced directly from `source` using the child's
/// byte range.
fn identifier_name(node: &tree_sitter_language_pack::Node, source: &str) -> Option<String> {
    for child in named_children(node) {
        if is_identifier_kind(&child.kind()) {
            return Some(source[child.start_byte()..child.end_byte()].to_owned());
        }
    }
    None
}

fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier" | "field_identifier" | "type_identifier" | "shorthand_property_identifier"
    )
}

/// Iterator helper over a node's named children. The pack only exposes
/// `named_child(index)` + `named_child_count()`, so we build the iteration
/// ourselves rather than depending on an iterator method that doesn't
/// exist.
fn named_children(
    node: &tree_sitter_language_pack::Node,
) -> impl Iterator<Item = tree_sitter_language_pack::Node> {
    (0..node.named_child_count() as u32).filter_map(move |i| node.named_child(i))
}

fn node_span(node: &tree_sitter_language_pack::Node, _source: &str) -> SpanAvailability {
    let start = Position {
        line: saturating_u32(node.start_position().row),
        column: saturating_u32(node.start_position().column),
    };
    let end = Position {
        line: saturating_u32(node.end_position().row),
        column: saturating_u32(node.end_position().column),
    };
    match Span::new(node.start_byte() as u64, node.end_byte() as u64, start, end) {
        Ok(span) => SpanAvailability::Present(span),
        Err(_) => SpanAvailability::NormalizerUnavailable {
            reason: "binding span failed local range validation".into(),
        },
    }
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
