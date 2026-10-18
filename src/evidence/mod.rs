//! Neutral evidence normalization: turns the language pack's generic
//! `process()` output into SlugAudit's own typed evidence records.

mod calls;
mod generic_imports;
mod normalize;
mod normalize_builders;
mod sql;

/// How deep SlugAudit's own tree walkers descend before they stop.
///
/// The walkers recurse over the raw parse tree, whose depth is a property of
/// the file's content: a file nesting 6 000 parentheses parses to a
/// 6 000-deep tree, and unbounded recursion over it overflows the thread
/// stack — which ABORTS the process rather than raising a catchable panic, so
/// the tool call never returns and the caller's session dies with it.
///
/// 512 matches the bound the language pack already applies to its own
/// traversal (`ts_pack::intel` truncates at `max_depth=512` and reports
/// `skipped_nodes`), so any file the pack considers safe is one these walkers
/// finish. Exceeding it yields a truncated extraction, reported as
/// `ExtractionCompleteness::Partial` — never a crash.
pub(super) const MAX_WALK_DEPTH: u32 = 512;

/// The one constructor for `NormalizerUnavailable`: keeps the real
/// `SpanError` in the reason instead of a static string, and warns, because a
/// span that fails validation means a walker computed offsets wrong — a
/// defect, not a property of the file. Only the context and the error kind are
/// logged, never source text or a path.
pub(super) fn span_unavailable(
    context: &'static str,
    error: crate::model::SpanError,
) -> crate::model::SpanAvailability {
    tracing::warn!(context, %error, "evidence span failed local range validation");
    crate::model::SpanAvailability::NormalizerUnavailable {
        reason: format!("{context} span failed local range validation: {error}"),
    }
}

/// A node byte range that does not slice the source: it is out of bounds or
/// lands inside a UTF-8 code point. A well-formed parse never produces one,
/// so seeing it means a grammar or scanner defect; callers report the
/// extraction as truncated instead of panicking.
#[derive(Debug, thiserror::Error)]
#[error("node byte range {start}..{end} does not slice the source text")]
pub(super) struct TextRangeError {
    start: usize,
    end: usize,
}

/// Slices `source[start..end]` without panicking.
pub(super) fn node_text(source: &str, start: usize, end: usize) -> Result<&str, TextRangeError> {
    source.get(start..end).ok_or(TextRangeError { start, end })
}

/// Parses `source` as `language`. A parser that cannot be built or a parse
/// that yields no tree is logged once and returned as `None`; the caller
/// must then report its extraction as truncated (`ExtractionCompleteness::
/// Partial`), never as complete.
pub(super) fn parse_tree(language: &str, source: &str) -> Option<tree_sitter_language_pack::Tree> {
    let mut parser = match tree_sitter_language_pack::get_parser(language) {
        Ok(parser) => parser,
        Err(error) => {
            tracing::warn!(language, %error, "evidence walker could not build a parser; extraction is partial");
            return None;
        }
    };
    let tree = parser.parse(source);
    if tree.is_none() {
        tracing::warn!(
            language,
            "evidence walker parse produced no tree; extraction is partial"
        );
    }
    tree
}

pub use normalize::extract;

pub use sql::{EvidenceRow, to_row};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unavailable_span_keeps_the_real_span_error_in_its_reason() {
        let availability = span_unavailable("binding", crate::model::SpanError::ReversedBytes);

        let crate::model::SpanAvailability::NormalizerUnavailable { reason } = availability else {
            panic!("expected NormalizerUnavailable, got {availability:?}");
        };
        assert!(reason.starts_with("binding span"), "{reason}");
        assert!(
            reason.contains("span end byte precedes start byte"),
            "{reason}"
        );
    }

    #[test]
    fn a_node_range_inside_a_code_point_is_an_error_not_a_panic() {
        let source = "a\u{e9}b";

        assert!(node_text(source, 0, 2).is_err());
        assert!(node_text(source, 0, 99).is_err());
        assert_eq!(node_text(source, 1, 3).expect("whole code point"), "\u{e9}");
    }

    #[test]
    fn a_parser_that_cannot_be_built_is_logged_and_reported_as_none() {
        let (tree, logs) = crate::store::test_capture::capture_warns(|| {
            parse_tree("no-such-language-for-evidence-test", "x")
        });

        assert!(tree.is_none());
        assert!(logs.contains("could not build a parser"), "{logs}");
    }
}
