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
}
