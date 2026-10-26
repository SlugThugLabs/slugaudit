use super::analyze_results::{
    ParseResult, cooldown_result, load_failed_result, parse_failed_result, succeeded_result,
    timeout_result, withheld_result,
};
use super::analyze_seam::run_extraction;
use crate::model::ParseOutcome;
use crate::util::Deadline;
use std::time::Duration;
use tree_sitter_language_pack::Error as PackError;

/// A parser that never loaded is a different failure than one that loaded
/// and then failed to parse — `ParserRun::validate` requires `LoadFailed`
/// to pair with `NotAttempted`, so these can't be conflated into one
/// `LoadFailed` bucket the way an untyped string representation could.
///
/// The grammar fetch is part of loading: the pack downloads a grammar bundle
/// on first use (`Download`), verifies it (`ChecksumMismatch`), serialises
/// writers on a cache lock (`CacheLock`) and reads or writes the cache
/// (`Io`). An offline host fails there, so those variants are load failures
/// too; the sweep, reconcile and publish treat a stored `LoadFailed` row as
/// stale and retry it once the fetch gate is open (see
/// `parse::fetch_gate`), a `Failed` parse is not retried.
fn is_load_failure(error: &PackError) -> bool {
    is_fetch_failure(error)
        || matches!(
            error,
            PackError::LanguageNotFound(_)
                | PackError::DynamicLoad(_)
                | PackError::NullLanguagePointer(_)
                | PackError::ParserSetup(_)
        )
}

/// The failures of the grammar fetch itself (network, checksum, cache lock,
/// cache I/O). These are not specific to one language, so they close the
/// process-wide fetch gate; a grammar that fetched but would not load does not.
fn is_fetch_failure(error: &PackError) -> bool {
    matches!(
        error,
        PackError::Download(_)
            | PackError::ChecksumMismatch { .. }
            | PackError::CacheLock(_)
            | PackError::Io(_)
    )
}

/// Detects a file's language and runs pack extraction when possible. Never
/// invents completeness: a file the pack can't identify or doesn't support
/// gets `ContentOnly`/`NotAttempted` — its text is indexed — and only a file
/// with no content at all is `Unavailable`. Content longer than
/// `max_parse_bytes` is never handed to the pack (no grammar fetch, load or
/// parse): it is `ContentOnly` with the `oversize_for_parse` reason. Every
/// parse of the file shares one budget: `parse_wall_clock`, or what is left
/// of the sync `deadline` when that is sooner. A parse cut at it is recorded
/// with the `parse_timeout` reason (see [`timeout_result`]). If the per-file
/// allowance was what ran out the sync goes on with the next file; if the
/// sync's did, the caller's own deadline check ends the sync.
pub(super) fn analyze(
    relative_path: &str,
    content: Option<&str>,
    max_parse_bytes: u64,
    parse_wall_clock: Duration,
    deadline: &Deadline,
) -> ParseResult {
    let Some(content) = content else {
        tracing::trace!(path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)), "no content; parser not invoked");
        return ParseResult::without_content();
    };
    let Some(language) = crate::parse::detect_language(relative_path) else {
        tracing::trace!(
            path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
            "no language detected; parser not invoked"
        );
        return ParseResult::content_only(None, false);
    };
    if language == crate::parse::PROSE_LANGUAGE {
        // Prose is stored and searchable but never parsed: no grammar is
        // fetched or loaded, so it yields no Diagnostic/Structure rows.
        tracing::trace!(
            path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
            "prose file; parser not invoked"
        );
        return ParseResult::content_only(Some(language), true);
    }
    if !crate::parse::language_available(&language) {
        tracing::debug!(
            path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
            language = %language,
            "detected language is not known to the pack; recording ContentOnly",
        );
        return ParseResult::content_only(Some(language), true);
    }
    if content.len() as u64 > max_parse_bytes {
        return withheld_result(relative_path, language, ParseOutcome::OVERSIZE_FOR_PARSE);
    }
    if crate::parse::fetch_gate::blocks_fetch(&language) {
        return cooldown_result(relative_path, language);
    }
    // One budget for every parse of this file: the per-file allowance, or the
    // sync's own remainder when that is sooner. `bound` says which it was.
    let (file_deadline, bound) = deadline.capped(parse_wall_clock);
    match run_extraction(&language, content, &file_deadline) {
        Ok(extraction) if extraction.timed_out => {
            timeout_result(relative_path, language, extraction.items, bound)
        }
        Ok(extraction) => succeeded_result(relative_path, language, extraction),
        Err(error) if is_load_failure(&error) => {
            if is_fetch_failure(&error) {
                crate::parse::fetch_gate::record_fetch_failure();
            }
            load_failed_result(relative_path, language, &error)
        }
        Err(PackError::ParseTimeout { .. }) => {
            timeout_result(relative_path, language, Vec::new(), bound)
        }
        Err(error) => parse_failed_result(relative_path, language, &error),
    }
}

#[cfg(test)]
#[path = "analyze_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "analyze_bounds_tests.rs"]
mod bounds_tests;
