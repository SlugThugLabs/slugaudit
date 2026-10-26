// slugaudit-line-exception: approved-by=slugthug; reason=The per-file analysis decision in one place: the language, prose and parse-size gates, pack-error classification, and one ParseResult constructor per outcome (without content, content-only, withheld, timeout, cool-down, succeeded, load-failed, parse-failed).
use super::analyze_seam::run_extraction;
use crate::evidence::Extraction;
use crate::model::{
    EvidenceItem, EvidenceKind, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
use crate::util::Deadline;
use tree_sitter_language_pack::Error as PackError;

pub(super) struct ParseResult {
    pub language: Option<String>,
    pub language_detected: bool,
    pub run: ParserRun,
    pub evidence: Vec<EvidenceItem>,
}

impl ParseResult {
    /// There was no content to describe (binary, or otherwise never
    /// sampled), so nothing was extracted and there is nothing to read.
    fn without_content() -> Self {
        Self {
            language: None,
            language_detected: false,
            run: ParserRun {
                availability: ParserAvailability::Unavailable,
                outcome: ParseOutcome::NotAttempted,
                completeness: ExtractionCompleteness::Unavailable,
            },
            evidence: Vec::new(),
        }
    }

    /// The file's content *was* sampled and is indexed in `files.content`,
    /// but no parser ran over it. Reporting `Unavailable` here reads as
    /// "no content" while the text sits in the DB — a fabricated verdict
    /// about a file an agent can still read.
    fn content_only(language: Option<String>, language_detected: bool) -> Self {
        Self {
            language,
            language_detected,
            run: ParserRun {
                availability: ParserAvailability::Unavailable,
                outcome: ParseOutcome::NotAttempted,
                completeness: ExtractionCompleteness::ContentOnly,
            },
            evidence: Vec::new(),
        }
    }
}

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

/// The pack error as a static, path-free category.
///
/// `DynamicLoad` and `LanguageNotFound` format the grammar-library path into
/// their `Display` (the runtime download cache under the user's home, or the
/// installed-library directory; tree-sitter-language-pack 1.18.0,
/// `registry.rs:326-337`), so a parser failure is never logged by formatting
/// the error itself.
fn pack_error_kind(error: &PackError) -> &'static str {
    match error {
        PackError::LanguageNotFound(_) => "language_not_found",
        PackError::DynamicLoad(_) => "dynamic_load",
        PackError::NullLanguagePointer(_) => "null_language_pointer",
        PackError::ParserSetup(_) => "parser_setup",
        PackError::LockPoisoned(_) => "lock_poisoned",
        PackError::Config(_) => "config",
        PackError::ParseFailed => "parse_failed",
        PackError::ParseTimeout { .. } => "parse_timeout",
        PackError::QueryError(_) => "query_error",
        PackError::InvalidRange(_) => "invalid_range",
        PackError::Io(_) => "io",
        PackError::Json(_) => "json",
        PackError::Download(_) => "download",
        PackError::ChecksumMismatch { .. } => "checksum_mismatch",
        PackError::CacheLock(_) => "cache_lock",
        // `Error` is `#[non_exhaustive]`: a future pack release can add a
        // variant, which must not make the failure log fall back to Display.
        _ => "other",
    }
}

/// Detects a file's language and runs pack extraction when possible. Never
/// invents completeness: a file the pack can't identify or doesn't support
/// gets `ContentOnly`/`NotAttempted` — its text is indexed — and only a file
/// with no content at all is `Unavailable`. Content longer than
/// `max_parse_bytes` is never handed to the pack (no grammar fetch, load or
/// parse): it is `ContentOnly` with the `oversize_for_parse` reason. Every
/// parse of the file runs within what is left of the sync `deadline`; one
/// cut at it is recorded with the `parse_timeout` reason (see
/// [`timeout_result`]) and the sync goes on to its own deadline check.
pub(super) fn analyze(
    relative_path: &str,
    content: Option<&str>,
    max_parse_bytes: u64,
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
    match run_extraction(&language, content, deadline) {
        Ok(extraction) if extraction.timed_out => {
            timeout_result(relative_path, language, extraction.items)
        }
        Ok(extraction) => succeeded_result(relative_path, language, extraction),
        Err(error) if is_load_failure(&error) => {
            if is_fetch_failure(&error) {
                crate::parse::fetch_gate::record_fetch_failure();
            }
            load_failed_result(relative_path, language, &error)
        }
        Err(PackError::ParseTimeout { .. }) => timeout_result(relative_path, language, Vec::new()),
        Err(error) => parse_failed_result(relative_path, language, &error),
    }
}

/// A file whose grammar was deliberately not run: its content is stored and
/// searchable (`ContentOnly`), and `reason` (a stable code) is stored in
/// `parse_error_reason`, where `report`, `leads.files_not_parsed` and
/// `query` see it. No grammar is fetched, so availability is `Unavailable`
/// (unknown, not claimed), like prose.
fn withheld_result(relative_path: &str, language: String, reason: &str) -> ParseResult {
    tracing::debug!(
        path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
        language = %language,
        reason,
        "grammar not run on this file; recording ContentOnly with the reason",
    );
    ParseResult {
        language: Some(language),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::Unavailable,
            outcome: ParseOutcome::Withheld {
                reason: reason.to_owned(),
            },
            completeness: ExtractionCompleteness::ContentOnly,
        },
        evidence: Vec::new(),
    }
}

/// A parse cut at the sync deadline (cancelled by the pack's parse timeout,
/// or not started because under a millisecond was left): the outcome is
/// `Failed` with the stable `parse_timeout` code rather than the pack's
/// message. `evidence` is what was extracted before the cut — empty when
/// `process()` itself was cut (`ContentOnly`), the pack's evidence when a
/// later walker re-parse was (`Partial`: real but incomplete, so it is kept
/// rather than dropped). The pack logs a cancellation itself.
fn timeout_result(
    relative_path: &str,
    language: String,
    evidence: Vec<EvidenceItem>,
) -> ParseResult {
    tracing::debug!(
        path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
        language = %language,
        evidence_items = evidence.len(),
        "parse cut at the deadline; recording Failed with parse_timeout",
    );
    let completeness = if evidence.is_empty() {
        ExtractionCompleteness::ContentOnly
    } else {
        ExtractionCompleteness::Partial
    };
    ParseResult {
        language: Some(language),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::Available,
            outcome: ParseOutcome::Failed {
                reason: ParseOutcome::PARSE_TIMEOUT.to_owned(),
            },
            completeness,
        },
        evidence,
    }
}

/// The reason stored for a file whose grammar fetch was not attempted
/// because an earlier fetch failed. Static: the original failure was logged
/// once, when it happened.
const COOLDOWN_REASON: &str = "grammar fetch skipped: an earlier fetch failed and is in cool-down; it is retried after the cool-down or on the next server start";

/// A file whose grammar is not on this host while the fetch gate is closed:
/// the pack is not called, so an offline host does not pay the fetch timeout
/// again. Recorded `LoadFailed` like the failure that closed the gate.
fn cooldown_result(relative_path: &str, language: String) -> ParseResult {
    tracing::debug!(
        path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
        language = %language,
        "grammar fetch in cool-down; recording LoadFailed without calling the pack",
    );
    ParseResult {
        language: Some(language),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::LoadFailed {
                reason: COOLDOWN_REASON.to_owned(),
            },
            outcome: ParseOutcome::NotAttempted,
            completeness: ExtractionCompleteness::ContentOnly,
        },
        evidence: Vec::new(),
    }
}

/// A run that extracted. Diagnostics make the tree degraded
/// (`SyntaxErrors` + `Partial`). A walk that stopped at `MAX_WALK_DEPTH` is
/// the third case — the tree parsed cleanly but SlugAudit's own walkers did
/// not reach the bottom of it, so the extraction is truncated even though
/// the outcome is `Succeeded`. Reporting that as `Full` would claim a
/// completeness the walk did not achieve. (A timed-out extraction goes to
/// [`timeout_result`] instead.)
fn succeeded_result(relative_path: &str, language: String, extraction: Extraction) -> ParseResult {
    let Extraction {
        items,
        truncated: depth_capped,
        ..
    } = extraction;
    let diagnostic_count = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Diagnostic)
        .count();
    let outcome = if diagnostic_count > 0 {
        ParseOutcome::SyntaxErrors {
            // A per-file diagnostic count past `u32::MAX` would need more
            // than 100 GB of in-memory evidence items, so it cannot occur
            // within any configured limit; if it ever did, the saturation is
            // logged with the cause rather than reported as an exact count.
            count: u32::try_from(diagnostic_count).unwrap_or_else(|error| {
                tracing::warn!(
                    path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
                    diagnostic_count,
                    error = %error,
                    "diagnostic count exceeds u32; the recorded syntax-error count saturates",
                );
                u32::MAX
            }),
        }
    } else {
        ParseOutcome::Succeeded
    };
    let completeness = if diagnostic_count > 0 || depth_capped {
        ExtractionCompleteness::Partial
    } else {
        ExtractionCompleteness::Full
    };
    tracing::debug!(
        path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
        language = %language,
        diagnostic_count,
        outcome = ?outcome,
        "parser load + extraction succeeded",
    );
    ParseResult {
        language: Some(language),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::Available,
            outcome,
            completeness,
        },
        evidence: items,
    }
}

/// A parser that failed to *load*: the run never attempted a parse, so the
/// text is still `ContentOnly`-indexed and the availability carries the
/// reason (see [`is_load_failure`] on why this stays distinct from `Failed`).
fn load_failed_result(relative_path: &str, language: String, error: &PackError) -> ParseResult {
    tracing::warn!(
        operation = "parser_load",
        path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
        language = %language,
        error_kind = pack_error_kind(error),
        "parser failed to load; recording LoadFailed",
    );
    ParseResult {
        language: Some(language),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::LoadFailed {
                reason: error.to_string(),
            },
            outcome: ParseOutcome::NotAttempted,
            completeness: ExtractionCompleteness::ContentOnly,
        },
        evidence: Vec::new(),
    }
}

/// A parser that loaded, ran, and failed extraction — availability stays
/// `Available`; the outcome carries the failure.
fn parse_failed_result(relative_path: &str, language: String, error: &PackError) -> ParseResult {
    tracing::debug!(
        operation = "parser_extract",
        path_id = %crate::util::redacted_path(std::path::Path::new(relative_path)),
        language = %language,
        error_kind = pack_error_kind(error),
        "parser ran but extraction failed; recording Failed",
    );
    ParseResult {
        language: Some(language),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::Available,
            outcome: ParseOutcome::Failed {
                reason: error.to_string(),
            },
            completeness: ExtractionCompleteness::ContentOnly,
        },
        evidence: Vec::new(),
    }
}

#[cfg(test)]
#[path = "analyze_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "analyze_bounds_tests.rs"]
mod bounds_tests;
