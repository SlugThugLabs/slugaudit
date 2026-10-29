// slugaudit-line-exception: approved-by=slugthug; reason=One responsibility: the ParseResult a file's analysis ends in and one constructor per outcome (without content, content-only, withheld, timeout, cool-down, succeeded, load-failed, parse-failed), each setting availability, outcome and completeness together so the combination stays valid.
//! The outcomes `analyze` can end in. Each constructor sets availability,
//! outcome and completeness together (see `ParserRun::validate`).
use crate::evidence::Extraction;
use crate::model::{
    EvidenceItem, EvidenceKind, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
use crate::util::BudgetBound;
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
    pub(super) fn without_content() -> Self {
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
    pub(super) fn content_only(language: Option<String>, language_detected: bool) -> Self {
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

/// The pack error as a static, path-free category.
///
/// `DynamicLoad` and `LanguageNotFound` format the grammar-library path into
/// their `Display` (the runtime download cache under the user's home, or the
/// installed-library directory; tree-sitter-language-pack 1.18.0,
/// `registry.rs:326-337`), so a parser failure is never logged by formatting
/// the error itself.
pub(super) fn pack_error_kind(error: &PackError) -> &'static str {
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

/// A file whose grammar was deliberately not run: its content is stored and
/// searchable (`ContentOnly`), and `reason` (a stable code) is stored in
/// `parse_error_reason`, where `report`, `leads.files_not_parsed` and
/// `query` see it. No grammar is fetched, so availability is `Unavailable`
/// (unknown, not claimed), like prose.
pub(super) fn withheld_result(relative_path: &str, language: String, reason: &str) -> ParseResult {
    tracing::debug!(
        path = %relative_path,
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

/// A parse cut at the file's deadline (cancelled by the pack's parse timeout,
/// or not started because under a millisecond was left): the outcome is
/// `Failed` with the stable `parse_timeout` code rather than the pack's
/// message. `bound` is which budget the file ran under; it only decides what
/// is logged here, because the file is recorded the same way either way and
/// the sync's own deadline check decides whether the sync goes on. `evidence` is what was extracted before the cut — empty when
/// `process()` itself was cut (`ContentOnly`), the pack's evidence when a
/// later walker re-parse was (`Partial`: real but incomplete, so it is kept
/// rather than dropped). The pack logs a cancellation itself.
pub(super) fn timeout_result(
    relative_path: &str,
    language: String,
    evidence: Vec<EvidenceItem>,
    bound: BudgetBound,
) -> ParseResult {
    tracing::debug!(
        path = %relative_path,
        language = %language,
        budget = ?bound,
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
pub(super) const COOLDOWN_REASON: &str = "grammar fetch skipped: an earlier fetch failed and is in cool-down; it is retried after the cool-down or on the next server start";

/// A file whose grammar is not on this host while the fetch gate is closed:
/// the pack is not called, so an offline host does not pay the fetch timeout
/// again. Recorded `LoadFailed` like the failure that closed the gate.
pub(super) fn cooldown_result(relative_path: &str, language: String) -> ParseResult {
    tracing::debug!(
        path = %relative_path,
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
pub(super) fn succeeded_result(
    relative_path: &str,
    language: String,
    extraction: Extraction,
) -> ParseResult {
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
                    path = %relative_path,
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
        path = %relative_path,
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
pub(super) fn load_failed_result(
    relative_path: &str,
    language: String,
    error: &PackError,
) -> ParseResult {
    tracing::warn!(
        operation = "parser_load",
        path = %relative_path,
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
pub(super) fn parse_failed_result(
    relative_path: &str,
    language: String,
    error: &PackError,
) -> ParseResult {
    tracing::debug!(
        operation = "parser_extract",
        path = %relative_path,
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
