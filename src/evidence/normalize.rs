use crate::model::{EvidenceItem, LineIndex, Position, Span, SpanAvailability, saturating_u32};
use tree_sitter_language_pack::{
    DiagnosticSeverity, Error as PackError, ProcessConfig, Span as PackSpan, process,
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
pub fn extract(language: &str, source: &str) -> Result<(Vec<EvidenceItem>, bool), PackError> {
    let config = ProcessConfig::new(language).all();
    let result = process(source, &config)?;

    let lines = LineIndex::new(source);
    let mut items = Vec::new();
    let mut counter = 0usize;
    for item in &result.structure {
        flatten_structure(item, &lines, &mut items, &mut counter);
    }
    for (index, item) in result.imports.iter().enumerate() {
        items.push(import_item(index, item, &lines));
    }
    for (index, item) in result.exports.iter().enumerate() {
        items.push(export_item(index, item, &lines));
    }
    for (index, item) in result.comments.iter().enumerate() {
        items.push(comment_item(index, item, &lines));
    }
    for (index, item) in result.docstrings.iter().enumerate() {
        items.push(docstring_item(index, item, &lines));
    }
    for (index, item) in result.symbols.iter().enumerate() {
        items.push(symbol_item(index, item, &lines));
    }
    items.extend(
        result
            .diagnostics
            .iter()
            .enumerate()
            .map(|(index, diagnostic)| diagnostic_item(index, diagnostic, &lines)),
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
    let (walker_items, mut depth_capped) =
        extract_extra_walkers(language, source, &lines, result.imports.is_empty());
    items.extend(walker_items);
    if language == "rust" {
        let (call_items, calls_capped) = super::calls::extract_rust(source, &lines);
        items.extend(call_items);
        depth_capped |= calls_capped;
    }

    Ok((items, depth_capped))
}

/// Parses `source` once and runs both the variable-binding walker and
/// (when `pack_imports_was_empty`) the generic import walker on the
/// same `Tree`. Returns the items and whether the extraction is truncated:
/// a walker stopped early at [`crate::evidence::MAX_WALK_DEPTH`], a node
/// range did not slice the source, or `language` has no parser or the source
/// did not parse (then no items at all). The flag is the whole reason this
/// returns a pair: a truncated extraction that a caller cannot tell apart
/// from a complete one would claim completeness it does not have.
fn extract_extra_walkers(
    language: &str,
    source: &str,
    lines: &LineIndex<'_>,
    pack_imports_was_empty: bool,
) -> (Vec<EvidenceItem>, bool) {
    let Some(tree) = super::parse_tree(language, source) else {
        return (Vec::new(), true);
    };
    let root = tree.root_node();

    let mut items = Vec::new();
    let mut counter = 0usize;
    let mut capped = super::bindings::walk_bindings(&root, source, lines, &mut items, &mut counter);
    if pack_imports_was_empty {
        capped |=
            super::generic_imports::walk_imports(&root, source, lines, &mut items, &mut counter);
    }
    (items, capped)
}

pub(super) fn convert_span(span: &PackSpan, lines: &LineIndex<'_>) -> SpanAvailability {
    let start = Position {
        line: saturating_u32(span.start_line),
        column: lines.column(span.start_byte),
    };
    let end = Position {
        line: saturating_u32(span.end_line),
        column: lines.column(span.end_byte),
    };
    match Span::new(span.start_byte as u64, span.end_byte as u64, start, end) {
        Ok(converted) => SpanAvailability::Present(converted),
        Err(error) => super::span_unavailable("pack", error),
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
