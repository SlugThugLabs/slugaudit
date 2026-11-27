// slugaudit-line-exception: approved-by=split-judge; reason=single entry point and typed pack-to-evidence builders for file extraction
//! Evidence extraction entry points (`extract` and `extract_within`), routing
//! Rust through single-parse CST extraction and other languages through the
//! language pack's `process()` plus shared CST walkers.

use super::{ExtractError, Extraction, WalkInput};
use crate::model::{
    CommentPayload, DiagnosticPayload, DocstringPayload, EvidenceItem, EvidenceKind,
    EvidenceOrigin, EvidencePayload, ExportPayload, ImportPayload, LineIndex, PositionOverflow,
    SpanAvailability, StructurePayload, SymbolPayload,
};
use crate::parse::budget::{Parsed, parse_within};
use crate::util::Deadline;
use tree_sitter_language_pack::{
    CommentInfo, DiagnosticSeverity, DocstringInfo, Error as PackError, ExportInfo, ImportInfo,
    ProcessConfig, Span as PackSpan, StructureItem, SymbolInfo, process,
};

/// Extracts typed evidence items from `source` in `language` with no time limit.
///
/// # Errors
///
/// Returns an error if `language` is not known to the pack, parsing fails
/// outright, or a line or column exceeds `u32::MAX`; syntax-error nodes surface
/// as `Diagnostic` evidence instead.
#[cfg(test)]
pub(crate) fn extract(
    language: &str,
    source: &str,
) -> Result<(Vec<EvidenceItem>, bool), ExtractError> {
    extract_within(language, source, None)
        .map(|extraction| (extraction.items, extraction.truncated))
}

/// [`extract`] bounded by `deadline` (`None`: unbounded).
///
/// For `"rust"`, parses `source` **once** with `tree_sitter::Parser` under
/// `deadline` and walks that single CST for structure, symbols, bindings,
/// imports, comments, diagnostics, and calls. For other languages, loads the
/// grammar before starting the per-file parse timer, runs `process()`, and
/// runs the secondary binding/import/call walkers over one shared `Tree`.
///
/// # Errors
///
/// As [`extract`], plus [`PackError::ParseTimeout`] when the primary parse was
/// cancelled at `deadline` or had less than a millisecond left before starting.
pub(crate) fn extract_within(
    language: &str,
    source: &str,
    deadline: Option<&Deadline>,
) -> Result<Extraction, ExtractError> {
    if super::deadline_spent(deadline) {
        return Err(PackError::ParseTimeout { timeout_ms: 0 }.into());
    }
    if language == "rust" {
        return extract_rust_single_parse(source, deadline);
    }

    // Load (and if needed, download) the grammar before starting the per-file
    // bounded parse timer on the worker pool.
    let grammar = tree_sitter_language_pack::get_language(language)?;
    let mut config = ProcessConfig::new(language).all();
    let result = if let Some(deadline) = deadline {
        let Some(budget) = deadline.remaining_millis() else {
            return Err(PackError::ParseTimeout { timeout_ms: 0 }.into());
        };
        let timeout_ms = budget.get();
        config = config.with_parse_timeout_ms(Some(timeout_ms));
        super::parse_worker::process_bounded(source, config, timeout_ms)?
    } else {
        process(source, &config)?
    };

    let lines = LineIndex::new(source)?;
    let mut items = Vec::new();
    let mut counter = 0usize;
    for item in &result.structure {
        flatten_structure(item, &lines, &mut items, &mut counter)?;
    }
    for (index, item) in result.imports.iter().enumerate() {
        items.push(import_item(index, item, &lines)?);
    }
    for (index, item) in result.exports.iter().enumerate() {
        items.push(export_item(index, item, &lines)?);
    }
    for (index, item) in result.comments.iter().enumerate() {
        items.push(comment_item(index, item, &lines)?);
    }
    for (index, item) in result.docstrings.iter().enumerate() {
        items.push(docstring_item(index, item, &lines)?);
    }
    for (index, item) in result.symbols.iter().enumerate() {
        items.push(symbol_item(index, item, &lines)?);
    }
    for (index, diagnostic) in result.diagnostics.iter().enumerate() {
        items.push(diagnostic_item(index, diagnostic, &lines)?);
    }

    let input = WalkInput {
        source,
        lines: &lines,
        deadline,
    };
    let extra = extract_non_rust_walkers(language, &grammar, &input, result.imports.is_empty())?;
    let mut extraction = Extraction {
        items,
        ..Extraction::default()
    };
    extraction.absorb(extra);
    Ok(extraction)
}

fn extract_rust_single_parse(
    source: &str,
    deadline: Option<&Deadline>,
) -> Result<Extraction, ExtractError> {
    let grammar = tree_sitter_language_pack::get_language("rust")?;
    let budget_ms = match deadline {
        Some(deadline) => match deadline.remaining_millis() {
            Some(ms) => Some(ms.get()),
            None => return Err(PackError::ParseTimeout { timeout_ms: 0 }.into()),
        },
        None => None,
    };
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|err| PackError::ParserSetup(err.to_string()))?;
    let tree = match parse_within(&mut parser, source, || super::deadline_spent(deadline)) {
        Parsed::Tree(tree) => tree,
        Parsed::Cancelled => {
            let mut timeout_ms = 0;
            if let Some(ms) = budget_ms {
                timeout_ms = ms;
            }
            return Err(PackError::ParseTimeout { timeout_ms }.into());
        }
        Parsed::NoTree => return Err(PackError::ParseFailed.into()),
    };

    let lines = LineIndex::new(source)?;
    let input = WalkInput {
        source,
        lines: &lines,
        deadline,
    };
    let mut extraction = super::rust_items::extract_rust_cst(&tree, &input)?;
    if !extraction.timed_out {
        extraction.absorb(super::calls::extract("rust", &tree, &input)?);
    }
    Ok(extraction)
}

fn extract_non_rust_walkers(
    language: &str,
    grammar: &tree_sitter::Language,
    input: &WalkInput<'_>,
    pack_imports_was_empty: bool,
) -> Result<Extraction, PositionOverflow> {
    let tree = match super::parse_tree_with_grammar(language, grammar, input.source, input.deadline)
    {
        Ok(tree) => tree,
        Err(why) => return Ok(Extraction::without_tree(why)),
    };
    let root = tree.root_node();
    let mut out = Extraction::default();
    let mut counter = 0usize;
    super::bindings::walk_bindings(root, input, &mut out, &mut counter)?;
    if pack_imports_was_empty && !out.timed_out {
        super::generic_imports::walk_imports(root, input, &mut out, &mut counter)?;
    }
    if !out.timed_out {
        out.absorb(super::calls::extract(language, &tree, input)?);
    }
    Ok(out)
}

#[cfg(test)]
fn extract_extra_walkers(
    language: &str,
    input: &WalkInput<'_>,
    pack_imports_was_empty: bool,
) -> Result<Extraction, PositionOverflow> {
    let tree = match super::parse_tree(language, input.source, input.deadline) {
        Ok(tree) => tree,
        Err(why) => return Ok(Extraction::without_tree(why)),
    };
    if language == "rust" {
        let mut out = super::rust_items::extract_rust_cst(&tree, input)?;
        if !out.timed_out {
            out.absorb(super::calls::extract("rust", &tree, input)?);
        }
        return Ok(out);
    }
    let root = tree.root_node();
    let mut out = Extraction::default();
    let mut counter = 0usize;
    super::bindings::walk_bindings(root, input, &mut out, &mut counter)?;
    if pack_imports_was_empty && !out.timed_out {
        super::generic_imports::walk_imports(root, input, &mut out, &mut counter)?;
    }
    if !out.timed_out {
        out.absorb(super::calls::extract(language, &tree, input)?);
    }
    Ok(out)
}

/// Flattens one pack structure tree in pre-order using an explicit heap stack.
fn flatten_structure(
    root: &StructureItem,
    lines: &LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) -> Result<(), PositionOverflow> {
    let mut pending = vec![root];
    while let Some(item) = pending.pop() {
        let key = format!("structure:{counter}");
        items.push(EvidenceItem {
            key,
            kind: EvidenceKind::Structure,
            origin: EvidenceOrigin::PackStructure,
            span: convert_span(&item.span, lines)?,
            payload: EvidencePayload::Structure(StructurePayload {
                kind: format!("{:?}", item.kind),
                name: item.name.clone(),
                visibility: item.visibility.clone(),
                decorators: item.decorators.clone(),
                signature: item.signature.clone(),
                doc_comment: item.doc_comment.clone(),
                rust_mod_path: None,
            }),
        });
        *counter = items.len();
        pending.extend(item.children.iter().rev());
    }
    Ok(())
}

fn import_item(
    index: usize,
    item: &ImportInfo,
    lines: &LineIndex<'_>,
) -> Result<EvidenceItem, PositionOverflow> {
    Ok(EvidenceItem {
        key: format!("import:{index}"),
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines)?,
        payload: EvidencePayload::Import(ImportPayload {
            source: item.source.clone(),
            items: item.items.clone(),
            alias: item.alias.clone(),
            is_wildcard: item.is_wildcard,
        }),
    })
}

fn export_item(
    index: usize,
    item: &ExportInfo,
    lines: &LineIndex<'_>,
) -> Result<EvidenceItem, PositionOverflow> {
    Ok(EvidenceItem {
        key: format!("export:{index}"),
        kind: EvidenceKind::Export,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines)?,
        payload: EvidencePayload::Export(ExportPayload {
            name: item.name.clone(),
            kind: format!("{:?}", item.kind),
        }),
    })
}

fn comment_item(
    index: usize,
    item: &CommentInfo,
    lines: &LineIndex<'_>,
) -> Result<EvidenceItem, PositionOverflow> {
    Ok(EvidenceItem {
        key: format!("comment:{index}"),
        kind: EvidenceKind::Comment,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines)?,
        payload: EvidencePayload::Comment(CommentPayload {
            text: item.text.clone(),
            kind: format!("{:?}", item.kind),
            associated_node: item.associated_node.clone(),
        }),
    })
}

fn docstring_item(
    index: usize,
    item: &DocstringInfo,
    lines: &LineIndex<'_>,
) -> Result<EvidenceItem, PositionOverflow> {
    Ok(EvidenceItem {
        key: format!("docstring:{index}"),
        kind: EvidenceKind::Docstring,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines)?,
        payload: EvidencePayload::Docstring(DocstringPayload {
            text: item.text.clone(),
            format: format!("{:?}", item.format),
            associated_item: item.associated_item.clone(),
        }),
    })
}

fn symbol_item(
    index: usize,
    item: &SymbolInfo,
    lines: &LineIndex<'_>,
) -> Result<EvidenceItem, PositionOverflow> {
    Ok(EvidenceItem {
        key: format!("symbol:{index}"),
        kind: EvidenceKind::Symbol,
        origin: EvidenceOrigin::PackSymbol,
        span: convert_span(&item.span, lines)?,
        payload: EvidencePayload::Symbol(SymbolPayload {
            name: item.name.clone(),
            kind: format!("{:?}", item.kind),
            visibility: None,
            type_annotation: item.type_annotation.clone(),
            doc: item.doc.clone(),
        }),
    })
}

fn diagnostic_item(
    index: usize,
    item: &tree_sitter_language_pack::Diagnostic,
    lines: &LineIndex<'_>,
) -> Result<EvidenceItem, PositionOverflow> {
    Ok(EvidenceItem {
        key: format!("diagnostic:{index}"),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::PackStructure,
        span: convert_span(&item.span, lines)?,
        payload: EvidencePayload::Diagnostic(DiagnosticPayload {
            message: item.message.clone(),
            severity: severity_text(&item.severity).to_owned(),
            original_item_count: None,
            kept_item_count: None,
        }),
    })
}

fn convert_span(
    span: &PackSpan,
    lines: &LineIndex<'_>,
) -> Result<SpanAvailability, PositionOverflow> {
    super::range_span(
        (span.start_byte, span.end_byte),
        (span.start_line, span.end_line),
        lines,
        "pack",
    )
}

fn severity_text(severity: &DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Info => "info",
    }
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;
