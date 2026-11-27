// slugaudit-line-exception: approved-by=split-judge; reason=single-file language gating, pack extraction, error classification, and worker test-seam propagation
//! Language detection, pack extraction, and `ParseResult` construction for
//! one sampled file, plus the test-only pack seam and worker-thread context
//! propagation.

use crate::evidence::{ExtractError, Extraction};
use crate::model::{
    EvidenceItem, EvidenceKind, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
use crate::util::{BudgetBound, Deadline};
use std::time::Duration;
use tree_sitter_language_pack::Error as PackError;

/// Stored reason for a file whose grammar fetch was skipped during cool-down.
pub(super) const COOLDOWN_REASON: &str = "grammar fetch skipped: an earlier fetch failed and is in cool-down; it is retried after the cool-down or on the next server start";

/// Stable `error_kind` (and `Failed` reason prefix) for a diagnostic count
/// exceeding `u32::MAX`.
const DIAGNOSTIC_COUNT_OVERFLOW: &str = "diagnostic_count_overflow";

pub(super) struct ParseResult {
    pub language: Option<String>,
    pub language_detected: bool,
    pub run: ParserRun,
    pub evidence: Vec<EvidenceItem>,
}

impl ParseResult {
    /// Binary or unsampled file with no content to describe.
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

    /// Sampled text indexed in `files.content` without running a parser.
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

    fn load_failed(language: String, reason: String) -> Self {
        Self {
            language: Some(language),
            language_detected: true,
            run: ParserRun {
                availability: ParserAvailability::LoadFailed { reason },
                outcome: ParseOutcome::NotAttempted,
                completeness: ExtractionCompleteness::ContentOnly,
            },
            evidence: Vec::new(),
        }
    }
}

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

fn is_fetch_failure(error: &PackError) -> bool {
    matches!(
        error,
        PackError::Download(_)
            | PackError::ChecksumMismatch { .. }
            | PackError::CacheLock(_)
            | PackError::Io(_)
    )
}

/// Static, path-free category for a `PackError`.
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
        _ => "other",
    }
}

fn extract_error_kind(error: &ExtractError) -> &'static str {
    match error {
        ExtractError::Pack(pack) => pack_error_kind(pack),
        ExtractError::Position(_) => "position_overflow",
    }
}

fn withheld_result(relative_path: &str, language: String, reason: &str) -> ParseResult {
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

fn timeout_result(
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

fn cooldown_result(relative_path: &str, language: String) -> ParseResult {
    tracing::debug!(
        path = %relative_path,
        language = %language,
        "grammar fetch in cool-down; recording LoadFailed without calling the pack",
    );
    ParseResult::load_failed(language, COOLDOWN_REASON.to_owned())
}

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
            count: match u32::try_from(diagnostic_count) {
                Ok(count) => count,
                Err(error) => {
                    tracing::warn!(
                        operation = "parser_extract",
                        path = %relative_path,
                        language = %language,
                        error_kind = DIAGNOSTIC_COUNT_OVERFLOW,
                        diagnostic_count,
                        error = %error,
                        "diagnostic count exceeds u32; recording Failed",
                    );
                    return failed_result(
                        language,
                        format!("{DIAGNOSTIC_COUNT_OVERFLOW}: {diagnostic_count} diagnostics"),
                    );
                }
            },
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

fn load_failed_result(relative_path: &str, language: String, error: &PackError) -> ParseResult {
    tracing::warn!(
        operation = "parser_load",
        path = %relative_path,
        language = %language,
        error_kind = pack_error_kind(error),
        "parser failed to load; recording LoadFailed",
    );
    ParseResult::load_failed(language, error.to_string())
}

fn parse_failed_result(relative_path: &str, language: String, error: &ExtractError) -> ParseResult {
    tracing::debug!(
        operation = "parser_extract",
        path = %relative_path,
        language = %language,
        error_kind = extract_error_kind(error),
        "parser ran but extraction failed; recording Failed",
    );
    failed_result(language, error.to_string())
}

fn failed_result(language: String, reason: String) -> ParseResult {
    ParseResult {
        language: Some(language),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::Available,
            outcome: ParseOutcome::Failed { reason },
            completeness: ExtractionCompleteness::ContentOnly,
        },
        evidence: Vec::new(),
    }
}

#[cfg(not(test))]
fn run_extraction(
    language: &str,
    content: &str,
    deadline: &Deadline,
) -> Result<Extraction, ExtractError> {
    crate::evidence::extract_within(language, content, Some(deadline))
}

#[cfg(test)]
fn run_extraction(
    language: &str,
    content: &str,
    deadline: &Deadline,
) -> Result<Extraction, ExtractError> {
    forced_failure::count_call();
    if let Some(make_error) = forced_failure::current() {
        return Err(make_error().into());
    }
    crate::evidence::extract_within(language, content, Some(deadline))
}

fn eligible_language<'a>(
    relative_path: &str,
    content: Option<&'a str>,
    max_parse_bytes: u64,
) -> Result<(&'a str, String), ParseResult> {
    let Some(content) = content else {
        tracing::trace!(path = %relative_path, "no content; parser not invoked");
        return Err(ParseResult::without_content());
    };
    let Some(language) = crate::parse::detect_language(relative_path) else {
        tracing::trace!(
            path = %relative_path,
            "no language detected; parser not invoked"
        );
        return Err(ParseResult::content_only(None, false));
    };
    if language == crate::parse::PROSE_LANGUAGE {
        tracing::trace!(path = %relative_path, "prose file; parser not invoked");
        return Err(ParseResult::content_only(Some(language), true));
    }
    if !crate::parse::language_available(&language) {
        tracing::debug!(
            path = %relative_path,
            language = %language,
            "detected language is not known to the pack; recording ContentOnly",
        );
        return Err(ParseResult::content_only(Some(language), true));
    }
    if content.len() as u64 > max_parse_bytes {
        return Err(withheld_result(
            relative_path,
            language,
            ParseOutcome::OVERSIZE_FOR_PARSE,
        ));
    }
    Ok((content, language))
}

/// Detects a file's language and runs pack extraction within `parse_wall_clock`
/// capped by the sync `deadline`.
pub(super) fn analyze(
    relative_path: &str,
    content: Option<&str>,
    max_parse_bytes: u64,
    parse_wall_clock: Duration,
    deadline: &Deadline,
) -> Result<ParseResult, crate::util::LockPoisoned> {
    let (content, language) = match eligible_language(relative_path, content, max_parse_bytes) {
        Ok(eligible) => eligible,
        Err(early_result) => return Ok(early_result),
    };
    match crate::parse::fetch_gate::with_fetch_slot(&language, || {
        let (file_deadline, bound) = deadline.capped(parse_wall_clock);
        let result = match run_extraction(&language, content, &file_deadline) {
            Ok(extraction) if extraction.timed_out => {
                timeout_result(relative_path, language.clone(), extraction.items, bound)
            }
            Ok(extraction) => succeeded_result(relative_path, language.clone(), extraction),
            Err(ExtractError::Pack(error)) if is_load_failure(&error) => {
                if is_fetch_failure(&error) {
                    crate::parse::fetch_gate::record_fetch_failure()?;
                } else {
                    crate::parse::local_load_failures::record_local_load_failure(&language)?;
                }
                load_failed_result(relative_path, language.clone(), &error)
            }
            Err(ExtractError::Pack(PackError::ParseTimeout { .. })) => {
                timeout_result(relative_path, language.clone(), Vec::new(), bound)
            }
            Err(error) => parse_failed_result(relative_path, language.clone(), &error),
        };
        Ok(result)
    })? {
        crate::parse::fetch_gate::FetchSlot::Blocked => {
            Ok(cooldown_result(relative_path, language))
        }
        crate::parse::fetch_gate::FetchSlot::Ran(result) => Ok(result),
    }
}

/// Test-only injection of a pack error into [`run_extraction`], scoped to the
/// calling test thread and propagated onto `file_pool` workers.
#[cfg(test)]
pub(super) mod forced_failure {
    use super::PackError;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    pub(crate) struct State {
        forced: Mutex<Option<fn() -> PackError>>,
        calls: AtomicUsize,
    }

    impl State {
        fn new() -> Self {
            Self {
                forced: Mutex::new(None),
                calls: AtomicUsize::new(0),
            }
        }

        fn lock_forced(&self) -> MutexGuard<'_, Option<fn() -> PackError>> {
            match self.forced.lock() {
                Ok(guard) => guard,
                Err(poisoned) => PoisonError::into_inner(poisoned),
            }
        }
    }

    thread_local! {
        static STATE: RefCell<Arc<State>> = RefCell::new(Arc::new(State::new()));
    }

    fn current_state() -> Arc<State> {
        STATE.with(|cell| Arc::clone(&cell.borrow()))
    }

    pub(crate) fn snapshot_for_worker() -> Arc<State> {
        current_state()
    }

    pub(crate) fn install_on_worker(state: Arc<State>) {
        STATE.with(|cell| *cell.borrow_mut() = state);
    }

    pub(super) fn current() -> Option<fn() -> PackError> {
        *current_state().lock_forced()
    }

    pub(super) fn count_call() {
        let state = current_state();
        let mut current = state.calls.load(Ordering::Acquire);
        loop {
            // KEEP saturating: a counter with a designed ceiling reads "at least this many" at usize::MAX and never wraps to a small number
            let next = current.saturating_add(1);
            match state.calls.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    pub(crate) fn calls() -> usize {
        current_state().calls.load(Ordering::Acquire)
    }

    pub(crate) struct Guard(Arc<State>);

    impl Drop for Guard {
        fn drop(&mut self) {
            *self.0.lock_forced() = None;
        }
    }

    pub(crate) fn force(make_error: fn() -> PackError) -> Guard {
        let state = current_state();
        *state.lock_forced() = Some(make_error);
        Guard(state)
    }
}

/// Propagates thread-scoped `#[cfg(test)]` parse gates and extraction seams
/// from the calling test thread onto `file_pool` worker threads during Pass 2.
#[cfg(test)]
#[derive(Clone)]
pub(super) struct TestWorkerContext {
    gate: std::sync::Arc<crate::parse::fetch_gate::GateState>,
    local_failures: std::sync::Arc<crate::parse::local_load_failures::LocalLoadFailures>,
    seam: std::sync::Arc<forced_failure::State>,
}

#[cfg(test)]
impl TestWorkerContext {
    pub(super) fn capture() -> Self {
        Self {
            gate: crate::parse::fetch_gate::test_controls::current_gate(),
            local_failures: crate::parse::local_load_failures::current_test_set(),
            seam: forced_failure::snapshot_for_worker(),
        }
    }

    pub(super) fn install(&self) {
        crate::parse::fetch_gate::test_controls::install_gate(std::sync::Arc::clone(&self.gate));
        crate::parse::local_load_failures::install_test_set(std::sync::Arc::clone(
            &self.local_failures,
        ));
        forced_failure::install_on_worker(std::sync::Arc::clone(&self.seam));
    }
}

#[cfg(test)]
#[path = "analyze_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "analyze_bounds_tests.rs"]
mod bounds_tests;

#[cfg(test)]
#[path = "analyze_results_tests.rs"]
mod results_tests;
