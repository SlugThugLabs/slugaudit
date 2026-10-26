//! Neutral evidence normalization: turns the language pack's generic
//! `process()` output into SlugAudit's own typed evidence records.

mod bindings;
mod calls;
mod generic_imports;
mod normalize;
mod normalize_builders;
mod sql;

/// How deep SlugAudit's own tree walkers descend before they stop.
///
/// Tree depth is a property of untrusted file content (6 000 nested parens
/// parse to a 6 000-deep tree), and a stack overflow ABORTS the process — it
/// is not a catchable panic — so the tool call never returns and the caller's
/// session dies with it. Every walker in this module (`calls::walk`,
/// `macro_calls::scan` including macro re-entry, `walk_bindings`,
/// `walk_imports`, `flatten_structure`, `simple_name`) is therefore a loop
/// over a heap stack ([`push_children`]), never recursion; the only recursion
/// left is `first_identifier_descendant`, fixed at 3 levels. This bound no
/// longer guards the stack: it keeps the walkers' truncation aligned with the
/// language pack's (`ts_pack::intel` truncates at `MAX_TREE_DEPTH = 512`), and
/// a cut is reported as `ExtractionCompleteness::Partial`, never silent.
///
/// Stack budget, measured 2026-10-18 (Rust 1.97.1, pack 1.18.0) by bisecting
/// the smallest thread stack `normalize::extract` completes on, 20 000-deep
/// inputs: SlugAudit's walkers need no stack proportional to depth — Rust
/// paren/call/macro shapes (incl. 64 macro levels x 505 parens) finish in
/// 31 KiB debug / 15 KiB release. The deepest user is the pack's own
/// bounded recursion: JSON value extraction, 512 levels, 1646 KiB debug /
/// 455 KiB release (~3.2 / ~0.9 KiB per level). The smallest stack production
/// extracts on is 2 MiB (tokio's blocking pool and `std::thread` defaults —
/// `server_runner.rs` `spawn_blocking`, `sync/sample_batch.rs`
/// `thread::scope`). Before this, the walkers recursed with ~2.2 KiB debug
/// frames and macro re-entry multiplied 64 macro levels by 512 tree levels.
pub(super) const MAX_WALK_DEPTH: u32 = 512;

/// Pushes one `entry` per named child of `node` onto a walker's explicit
/// stack, in reverse, so popping visits the children in source order — the
/// same pre-order a recursive walk produces. Every tree walker in this module
/// is a loop over such a heap stack rather than recursion: tree depth is a
/// property of untrusted file content, and the heap grows where the thread
/// stack would abort.
pub(super) fn push_children<T>(
    pending: &mut Vec<T>,
    node: &tree_sitter_language_pack::Node,
    entry: impl Fn(tree_sitter_language_pack::Node) -> T,
) {
    let first = pending.len();
    pending.extend(
        (0..node.named_child_count() as u32)
            .filter_map(|index| node.named_child(index))
            .map(entry),
    );
    if let Some(added) = pending.get_mut(first..) {
        added.reverse();
    }
}

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

/// The one constructor for a tree-sitter node's
/// [`crate::model::SpanAvailability`]: derives the byte range and the
/// start/end positions, validates the range, and reports a failure through
/// [`span_unavailable`] under `context`. Every walker that emits a node span
/// (`normalize`, `generic_imports`, `calls`) shares it, so byte, row, and
/// column behavior cannot drift between them.
pub(super) fn node_span(
    node: &tree_sitter_language_pack::Node,
    lines: &crate::model::LineIndex<'_>,
    context: &'static str,
) -> crate::model::SpanAvailability {
    let start_byte = node.start_byte();
    let end_byte = node.end_byte();
    let start = crate::model::Position {
        line: crate::model::saturating_u32(node.start_position().row),
        column: lines.column(start_byte),
    };
    let end = crate::model::Position {
        line: crate::model::saturating_u32(node.end_position().row),
        column: lines.column(end_byte),
    };
    match crate::model::Span::new(start_byte as u64, end_byte as u64, start, end) {
        Ok(span) => crate::model::SpanAvailability::Present(span),
        Err(error) => span_unavailable(context, error),
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

/// What one bounded extraction produced, and how it was cut short.
#[derive(Debug, Default)]
pub(crate) struct Extraction {
    /// The evidence extracted, in extraction order.
    pub(crate) items: Vec<crate::model::EvidenceItem>,
    /// A walk bound was hit, a node range did not slice the source, or a
    /// walker's parse failed with time left: report `Partial`.
    pub(crate) truncated: bool,
    /// A re-parse after the pack's `process()` was cut at the deadline, so
    /// the walker that needed it saw nothing: `items` holds what was
    /// extracted before, and the file is recorded with `parse_timeout`.
    pub(crate) timed_out: bool,
}

impl Extraction {
    /// No items: the parse a walker needed produced no tree, for `why`.
    pub(super) fn without_tree(why: NoTree) -> Self {
        Self {
            items: Vec::new(),
            truncated: why == NoTree::Failed,
            timed_out: why == NoTree::TimedOut,
        }
    }

    /// Appends a later walker's items and keeps both cut-short flags.
    pub(super) fn absorb(&mut self, other: Self) {
        self.items.extend(other.items);
        self.truncated |= other.truncated;
        self.timed_out |= other.timed_out;
    }
}

/// Why [`parse_tree`] returned no tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoTree {
    /// The call's deadline left less than a millisecond, so the parse was not
    /// started, or the pack's parse timeout (its tree-sitter progress
    /// callback) cancelled it at the deadline. The caller records the file
    /// with the `parse_timeout` reason.
    TimedOut,
    /// The parser could not be built, or returned no tree while time was
    /// left. Logged here; the caller reports its extraction as truncated
    /// (`ExtractionCompleteness::Partial`), never as complete.
    Failed,
}

/// Parses `source` as `language` within what is left of `deadline` (`None`:
/// unbounded). Every re-parse of one file passes the same deadline, so the
/// file's parses share one budget instead of each getting a fresh timeout.
/// The pack's `Parser::parse` returns `None` for a timeout and for any other
/// failure alike, so the two are told apart by checking the deadline after
/// the call: a timeout is only reported once the deadline has passed.
pub(super) fn parse_tree(
    language: &str,
    source: &str,
    deadline: Option<&crate::util::Deadline>,
) -> Result<tree_sitter_language_pack::Tree, NoTree> {
    let budget_ms = match deadline {
        Some(deadline) => Some(deadline.remaining_millis().ok_or(NoTree::TimedOut)?.get()),
        None => None,
    };
    let mut parser = match tree_sitter_language_pack::get_parser(language) {
        Ok(parser) => parser,
        Err(error) => {
            tracing::warn!(language, %error, "evidence walker could not build a parser; extraction is partial");
            return Err(NoTree::Failed);
        }
    };
    parser.set_parse_timeout_ms(budget_ms);
    if let Some(tree) = parser.parse(source) {
        return Ok(tree);
    }
    if deadline.is_some_and(|deadline| deadline.remaining_millis().is_none()) {
        // The pack logs the cancellation itself.
        return Err(NoTree::TimedOut);
    }
    tracing::warn!(
        language,
        "evidence walker parse produced no tree; extraction is partial"
    );
    Err(NoTree::Failed)
}

pub use normalize::extract;
pub(crate) use normalize::extract_within;

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
            parse_tree("no-such-language-for-evidence-test", "x", None)
        });

        assert_eq!(tree.err(), Some(NoTree::Failed));
        assert!(logs.contains("could not build a parser"), "{logs}");
    }

    #[test]
    fn a_parse_under_a_spent_deadline_is_a_timeout_and_never_starts() {
        let deadline = crate::util::Deadline::after(std::time::Duration::ZERO);

        let (tree, logs) = crate::store::test_capture::capture_warns(|| {
            parse_tree("no-such-language-for-evidence-test", "x", Some(&deadline))
        });

        assert_eq!(tree.err(), Some(NoTree::TimedOut));
        assert!(
            !logs.contains("could not build a parser"),
            "the parser is never built: {logs}"
        );
    }

    #[test]
    fn a_parse_with_time_left_returns_its_tree() {
        let deadline = crate::util::Deadline::after(std::time::Duration::from_secs(60));

        let tree = parse_tree("rust", "fn a() {}", Some(&deadline)).expect("a tree");

        assert_eq!(tree.root_node().kind(), "source_file");
    }
}
