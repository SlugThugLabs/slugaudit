// slugaudit-line-exception: approved-by=agent; reason=one match arm per Tree-sitter evidence kind; splitting by kind would hide the exhaustiveness this file exists to guarantee
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, Position, Span, SpanAvailability, char_column,
    saturating_u32,
};
use serde_json::json;
use tree_sitter_language_pack::{
    DiagnosticSeverity, Error as PackError, ProcessConfig, Span as PackSpan, SymbolKind,
    get_parser, process,
};

use super::normalize_builders::{
    comment_item, diagnostic_item, docstring_item, export_item, flatten_structure, import_item,
    symbol_item,
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
        flatten_structure(item, source, &mut items, &mut counter);
    }
    for (index, item) in result.imports.iter().enumerate() {
        items.push(import_item(index, item, source));
    }
    for (index, item) in result.exports.iter().enumerate() {
        items.push(export_item(index, item, source));
    }
    for (index, item) in result.comments.iter().enumerate() {
        items.push(comment_item(index, item, source));
    }
    for (index, item) in result.docstrings.iter().enumerate() {
        items.push(docstring_item(index, item, source));
    }
    for (index, item) in result.symbols.iter().enumerate() {
        items.push(symbol_item(index, item, source));
    }
    items.extend(
        result
            .diagnostics
            .iter()
            .enumerate()
            .map(|(index, diagnostic)| diagnostic_item(index, diagnostic, source)),
    );

    // The language pack's `process()` extracts structure, imports, exports,
    // comments, docstrings, symbols, and diagnostics — but not individual
    // variable bindings or struct/class fields. Those are exactly the items
    // an AI needs to spot near-duplicate names (`file_path` vs `filepath`
    // vs `filePath`) that probably refer to the same thing. We walk the
    // tree ourselves with generic node-type patterns that are consistent
    // across tree-sitter grammars, so no per-language query code is needed.
    //
    // The pack's own import pass only covers a hardcoded handful of
    // languages; when it found nothing, fall back to our generic import
    // walker so languages like kotlin/swift/csharp/dart still get
    // dependency edges. Gated on `imports.is_empty()` so pack-covered
    // languages never double-report and never pay an extra parse.
    //
    // Both walkers now share a single parse via `extract_extra_walkers`
    // rather than each calling `get_parser().parse` independently. The
    // prior shape triple-parsed every file (`process` internally +
    // `extract_bindings` + `extract_generic_imports`); the second parse
    // was redundant because both walkers want the same `Tree`. The
    // pack's `process` parse is unavoidable — `tree-sitter-language-pack`
    // exposes no `process_with_tree(source, tree)` overload — but the
    // two extra ones collapse to one.
    items.extend(extract_extra_walkers(
        language,
        source,
        result.imports.is_empty(),
    ));

    Ok(items)
}

/// Parses `source` once and runs both the variable-binding walker and
/// (when `pack_imports_was_empty`) the generic import walker on the
/// same `Tree`. Returns an empty `Vec` when `language` has no parser
/// or the source doesn't parse — same contract as the two individual
/// extractors this replaces, so callers see the same emptiness on the
/// same failure modes.
fn extract_extra_walkers(
    language: &str,
    source: &str,
    pack_imports_was_empty: bool,
) -> Vec<EvidenceItem> {
    let Ok(mut parser) = get_parser(language) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(source) else {
        return Vec::new();
    };
    let root = tree.root_node();

    let mut items = Vec::new();
    let mut counter = 0usize;
    walk_bindings(&root, source, &mut items, &mut counter);
    if pack_imports_was_empty {
        super::generic_imports::walk_imports(&root, source, &mut items, &mut counter);
    }
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

/// Exact node kinds for a *field* declaration, verified against a fixed
/// grammar matrix (C7): rust/go/c/cpp/csharp/ocaml/java use
/// `field_declaration`; swift/kotlin use `property_declaration`;
/// javascript uses `field_definition`; typescript uses
/// `public_field_definition` and (for interface members) `property_signature`.
///
/// Exact match only — never `contains`: a substring test on
/// `field_declaration` also matches the *container* node
/// `field_declaration_list` (and could match unrelated kinds in an
/// unforeseen grammar), which would emit a duplicate Field symbol for the
/// same member. `field_identifier` (the name child) is deliberately not a
/// declaration and is not listed.
fn is_field_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "field_declaration"
            | "property_declaration"
            | "field_definition"
            | "public_field_definition"
            | "property_signature"
    )
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
///
/// When no direct child is an identifier, falls back to a bounded
/// recursive search of the declaration node's subtree (C7 grammar-matrix
/// finding): swift/kotlin nest the name under `pattern` /
/// `variable_declaration` wrappers, so `property_declaration`'s direct
/// children are not identifiers. The search returns the *first*
/// identifier-kind descendant — for the probed grammars (rust, go, c,
/// cpp, csharp, java, ocaml, swift, kotlin, js/ts) the member name
/// precedes the type, so this yields the name, not the type.
fn identifier_name(node: &tree_sitter_language_pack::Node, source: &str) -> Option<String> {
    for child in named_children(node) {
        if is_identifier_kind(&child.kind()) {
            return Some(source[child.start_byte()..child.end_byte()].to_owned());
        }
    }
    first_identifier_descendant(node, source, 0)
}

/// Depth-first search for the first identifier-kind descendant. `depth`
/// bounds the descent so a pathological grammar can't turn a single
/// declaration into an unbounded traversal — two levels of wrapper
/// (e.g. `property_declaration → pattern → simple_identifier`) is the
/// deepest any probed grammar nests a member name.
fn first_identifier_descendant(
    node: &tree_sitter_language_pack::Node,
    source: &str,
    depth: u8,
) -> Option<String> {
    if depth >= 3 {
        return None;
    }
    for child in named_children(node) {
        if is_identifier_kind(&child.kind()) {
            return Some(source[child.start_byte()..child.end_byte()].to_owned());
        }
    }
    for child in named_children(node) {
        if let Some(name) = first_identifier_descendant(&child, source, depth + 1) {
            return Some(name);
        }
    }
    None
}

/// Node kinds that name a *binding or member*. Deliberately excludes
/// `type_identifier`/`type_name`: in Java (and other grammars) a
/// `field_declaration` lists the type before the name, so treating the
/// type node as a name candidate would emit the type as the member name
/// (a C7 grammar-matrix finding). Type-name nodes are never the *name* of
/// a binding this walker targets.
fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "field_name"
            | "shorthand_property_identifier"
            | "property_identifier"
            | "simple_identifier"
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

fn node_span(node: &tree_sitter_language_pack::Node, source: &str) -> SpanAvailability {
    let start_byte = node.start_byte();
    let end_byte = node.end_byte();
    let start = Position {
        line: saturating_u32(node.start_position().row),
        column: char_column(source, start_byte),
    };
    let end = Position {
        line: saturating_u32(node.end_position().row),
        column: char_column(source, end_byte),
    };
    match Span::new(start_byte as u64, end_byte as u64, start, end) {
        Ok(span) => SpanAvailability::Present(span),
        Err(_) => SpanAvailability::NormalizerUnavailable {
            reason: "binding span failed local range validation".into(),
        },
    }
}

pub(super) fn convert_span(span: &PackSpan, source: &str) -> SpanAvailability {
    let start = Position {
        line: saturating_u32(span.start_line),
        column: char_column(source, span.start_byte),
    };
    let end = Position {
        line: saturating_u32(span.end_line),
        column: char_column(source, span.end_byte),
    };
    match Span::new(span.start_byte as u64, span.end_byte as u64, start, end) {
        Ok(converted) => SpanAvailability::Present(converted),
        Err(_) => SpanAvailability::NormalizerUnavailable {
            reason: "pack span failed local range validation".into(),
        },
    }
}

pub(super) fn severity_text(severity: &DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Info => "info",
    }
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "normalize_grammar_tests.rs"]
mod grammar_tests;
