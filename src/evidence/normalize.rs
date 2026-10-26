use super::Extraction;
use crate::model::{EvidenceItem, LineIndex, Position, Span, SpanAvailability, saturating_u32};
use crate::util::Deadline;
use tree_sitter_language_pack::{
    DiagnosticSeverity, Error as PackError, ProcessConfig, ProcessResult, Span as PackSpan, process,
};

use super::normalize_builders::{
    comment_item, diagnostic_item, docstring_item, export_item, flatten_structure, import_item,
    symbol_item,
};

/// Runs the language pack's generic `process()` intelligence and flattens
/// its result into SlugAudit's own evidence records. This is the only
/// place a pack type crosses into SlugAudit's model — payloads are built
/// field-by-field so our evidence contract never silently follows
/// upstream's own wire format. Unbounded in time; the sync path uses
/// [`extract_within`].
///
/// # Errors
///
/// Returns an error if `language` is not known to the pack or parsing
/// fails outright; a tree with syntax-error nodes is not an error here, it
/// surfaces as `Diagnostic` evidence instead.
pub fn extract(language: &str, source: &str) -> Result<(Vec<EvidenceItem>, bool), PackError> {
    // With no deadline no parse can time out, so `timed_out` is always false.
    extract_within(language, source, None)
        .map(|extraction| (extraction.items, extraction.truncated))
}

/// [`extract`] bounded by `deadline` (`None`: unbounded). Every parse of the
/// file — the pack's `process()` and each walker re-parse — gets what is
/// left of the same deadline, through the pack's parse timeout (which
/// cancels from tree-sitter's parse progress callback), so the whole file
/// shares one budget.
///
/// # Errors
///
/// As [`extract`], plus [`PackError::ParseTimeout`] when `process()` was
/// cancelled at the deadline, or was not started because less than a
/// millisecond was left (then `timeout_ms` is 0: the budget it had). A
/// timeout in a later re-parse is not an error: the pack's evidence is kept
/// and [`Extraction::timed_out`] is set.
pub(crate) fn extract_within(
    language: &str,
    source: &str,
    deadline: Option<&Deadline>,
) -> Result<Extraction, PackError> {
    let mut config = ProcessConfig::new(language).all();
    let result = if let Some(deadline) = deadline {
        let Some(budget) = deadline.remaining_millis() else {
            return Err(PackError::ParseTimeout { timeout_ms: 0 });
        };
        config = config.with_parse_timeout_ms(Some(budget.get()));
        process_on_fresh_thread(source, &config)?
    } else {
        process(source, &config)?
    };

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
    let mut extraction = Extraction {
        items,
        ..Extraction::default()
    };
    extraction.absorb(extract_extra_walkers(
        language,
        source,
        &lines,
        result.imports.is_empty(),
        deadline,
    ));
    extraction.absorb(super::calls::extract(language, source, &lines, deadline));

    Ok(extraction)
}

/// Runs the pack's `process()` on a thread of its own, so a bounded parse
/// cannot poison a parser another file will use.
///
/// The pack keeps one tree-sitter parser per language per thread and reuses it
/// for every later `process()`. A parse cancelled by the progress callback
/// leaves that parser to resume the cancelled document on its next use unless
/// `reset()` is called, which `process()` does not do: the next file parsed on
/// that thread would then be read against the wrong source (a slice-out-of-
/// bounds panic inside the pack, in the field). The cache is thread-local, so
/// a thread that ends takes any poisoned parser with it; the cost is one
/// short-lived thread and one parser setup per file, small beside the parse.
/// An unbounded `process()` cannot be cancelled and so needs no isolation.
///
/// A panic inside the pack is returned as [`PackError::ParseFailed`] (logged
/// once, without the payload) instead of unwinding into the sync.
fn process_on_fresh_thread(
    source: &str,
    config: &ProcessConfig,
) -> Result<ProcessResult, PackError> {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("slugaudit-parse".to_owned())
            .spawn_scoped(scope, || process(source, config))
            .map_err(|error| {
                tracing::error!(
                    operation = "parse_thread",
                    error_kind = ?error.kind(),
                    "could not start the thread for a bounded parse",
                );
                PackError::Config("could not start the thread for a bounded parse".to_owned())
            })?;
        match worker.join() {
            Ok(result) => result,
            Err(_panic_payload) => {
                tracing::error!(
                    operation = "parse_thread",
                    "the language pack panicked while parsing; recording the parse as failed",
                );
                Err(PackError::ParseFailed)
            }
        }
    })
}

/// Parses `source` once and runs both the variable-binding walker and
/// (when `pack_imports_was_empty`) the generic import walker on the
/// same `Tree`. The result is truncated when a walker stopped early at
/// [`crate::evidence::MAX_WALK_DEPTH`], a node range did not slice the
/// source, or `language` has no parser or the source did not parse (then no
/// items at all); it is timed out when that parse hit `deadline`. A
/// truncated extraction that a caller cannot tell apart from a complete one
/// would claim completeness it does not have.
fn extract_extra_walkers(
    language: &str,
    source: &str,
    lines: &LineIndex<'_>,
    pack_imports_was_empty: bool,
    deadline: Option<&Deadline>,
) -> Extraction {
    let tree = match super::parse_tree(language, source, deadline) {
        Ok(tree) => tree,
        Err(why) => return Extraction::without_tree(why),
    };
    let root = tree.root_node();

    let mut items = Vec::new();
    let mut counter = 0usize;
    let mut capped = super::bindings::walk_bindings(&root, source, lines, &mut items, &mut counter);
    if pack_imports_was_empty {
        capped |=
            super::generic_imports::walk_imports(&root, source, lines, &mut items, &mut counter);
    }
    Extraction {
        items,
        truncated: capped,
        timed_out: false,
    }
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
