//! Shared Tree-sitter grammar validation, fetch-gated loading, and budgeted
//! `QueryCursor` execution for `structure` and `mechanic`.

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::parse::PACK_ALIASES;
use crate::parse::fetch_gate::{FetchSlot, record_fetch_failure, with_fetch_slot};
use crate::parse::local_load_failures::record_local_load_failure;
use crate::tools::context::internal_error;
use crate::util::LockPoisoned;
use rmcp::ErrorData;
use std::ops::ControlFlow;
use std::time::Instant;
use tree_sitter::{
    Node, Parser, Query, QueryCursor, QueryCursorOptions, QueryCursorState, QueryMatch,
    StreamingIterator,
};
use tree_sitter_language_pack::Error as PackError;

/// Bound on in-progress Tree-sitter `QueryCursor` match states per tree scan so
/// pathological wildcard patterns degrade cleanly instead of growing unboundedly.
pub(crate) const QUERY_CURSOR_MATCH_LIMIT: u32 = 65_536;

/// Failure modes when loading a Tree-sitter grammar through the fetch gate.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GrammarLoadError {
    /// The grammar is not cached on disk and an earlier download failed within
    /// the 10-minute process-wide cool-down window.
    #[error("grammar download is blocked by the fetch cooldown")]
    CooldownBlocked,
    /// `tree_sitter_language_pack::get_language` failed.
    #[error(transparent)]
    Pack(PackError),
    /// The fetch gate's state mutex was poisoned.
    #[error(transparent)]
    Lock(LockPoisoned),
}

/// Outcome of running a Tree-sitter [`Query`] against one syntax tree under
/// [`QUERY_CURSOR_MATCH_LIMIT`] and an optional wall-clock deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorScanOutcome {
    /// True when the wall-clock `deadline` fired inside `QueryCursor`.
    pub(crate) timed_out: bool,
    /// True when `QueryCursor::did_exceed_match_limit()` fired (`65,536` states).
    pub(crate) match_limit_exceeded: bool,
}

/// Formats the shared refusal reason when a caller asks to run a Tree-sitter
/// query on `language` that the index stores as prose.
pub(in crate::tools) fn prose_refusal_reason(language: &str) -> String {
    format!(
        "language `{language}` is prose: prose files (Markdown, reStructuredText, AsciiDoc, \
         Org, plain text) are stored and searchable via `query` but are not parsed, so \
         Tree-sitter queries cannot run on them"
    )
}

/// Rejects language aliases (`PACK_ALIASES`) and prose languages with a uniform
/// human-readable reason shared by `structure` and `mechanic`.
pub(in crate::tools) fn check_non_alias_non_prose_language(language: &str) -> Result<(), String> {
    if let Some((_, canonical)) = PACK_ALIASES.iter().find(|(alias, _)| *alias == language) {
        return Err(format!(
            "language `{language}` is an alias the index never stores; use `{canonical}`"
        ));
    }
    if crate::parse::is_prose_language_name(language) {
        return Err(prose_refusal_reason(language));
    }
    Ok(())
}

/// Maps a [`GrammarLoadError`] into a structured [`ErrorData`] (`grammar_unavailable`
/// or lock-poisoned internal error), optionally prefixed (e.g. `"mechanic not added"`).
pub(in crate::tools) fn map_grammar_load_error(
    language: &str,
    prefix: Option<&str>,
    error: GrammarLoadError,
) -> ErrorData {
    let with_prefix = |body: String| match prefix {
        Some(head) => format!("{head}: {body}"),
        None => body,
    };
    match error {
        GrammarLoadError::CooldownBlocked => invalid_params_with_code(
            with_prefix(format!(
                "grammar `{language}` is temporarily unavailable: an earlier grammar download \
                 failed and the 10-minute offline cool-down is active; retry once network \
                 access is restored or restart the server, or query `files.content` via `query`"
            )),
            ErrorCode::GrammarUnavailable,
        ),
        GrammarLoadError::Pack(pack_err) => invalid_params_with_code(
            with_prefix(format!(
                "grammar `{language}` could not be loaded: {pack_err}; check network access or \
                 query `files.content` via `query`"
            )),
            ErrorCode::GrammarUnavailable,
        ),
        GrammarLoadError::Lock(poisoned) => poisoned.into(),
    }
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

fn is_local_load_failure(error: &PackError) -> bool {
    matches!(
        error,
        PackError::LanguageNotFound(_)
            | PackError::DynamicLoad(_)
            | PackError::NullLanguagePointer(_)
            | PackError::ParserSetup(_)
    )
}

/// Loads `language`'s Tree-sitter grammar under [`with_fetch_slot`], recording
/// fetch or local-load failures on the process-wide gate identically to `sync`.
pub(crate) fn load_grammar(language: &str) -> Result<tree_sitter::Language, GrammarLoadError> {
    let slot = with_fetch_slot(language, || {
        match tree_sitter_language_pack::get_language(language) {
            Ok(grammar) => Ok(Ok(grammar)),
            Err(error) => {
                if is_fetch_failure(&error) {
                    record_fetch_failure()?;
                } else if is_local_load_failure(&error) {
                    record_local_load_failure(language)?;
                }
                Ok(Err(error))
            }
        }
    });
    match slot {
        Ok(FetchSlot::Ran(Ok(grammar))) => Ok(grammar),
        Ok(FetchSlot::Ran(Err(error))) => Err(GrammarLoadError::Pack(error)),
        Ok(FetchSlot::Blocked) => Err(GrammarLoadError::CooldownBlocked),
        Err(poisoned) => Err(GrammarLoadError::Lock(poisoned)),
    }
}

/// Loads `language`'s Tree-sitter grammar through [`load_grammar`] and configures
/// a [`Parser`] ready to parse source files.
pub(in crate::tools) fn parser_for(language: &str, context: &str) -> Result<Parser, ErrorData> {
    let grammar =
        load_grammar(language).map_err(|error| map_grammar_load_error(language, None, error))?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|error| internal_error(context, error))?;
    Ok(parser)
}

/// Executes `query` captures over `root` under [`QUERY_CURSOR_MATCH_LIMIT`]
/// (`65,536` states) and `deadline`, invoking `on_capture` per capture and
/// reporting whether the wall-clock deadline or `did_exceed_match_limit()` fired.
pub(crate) fn run_budgeted_captures<'tree, E>(
    cursor: &mut QueryCursor,
    query: &Query,
    root: Node<'tree>,
    source: &'tree [u8],
    deadline: Option<Instant>,
    mut on_capture: impl FnMut(&QueryMatch<'_, 'tree>, usize) -> Result<(), E>,
) -> Result<CursorScanOutcome, E> {
    cursor.set_match_limit(QUERY_CURSOR_MATCH_LIMIT);
    let mut timed_out = false;
    let mut check_deadline = |_state: &QueryCursorState| -> ControlFlow<()> {
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            timed_out = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = QueryCursorOptions::new().progress_callback(&mut check_deadline);
    let mut captures = cursor.captures_with_options(query, root, source, options);
    while let Some((query_match, capture_index)) = captures.next() {
        on_capture(query_match, *capture_index)?;
    }
    drop(captures);
    Ok(CursorScanOutcome {
        timed_out,
        match_limit_exceeded: cursor.did_exceed_match_limit(),
    })
}

#[cfg(test)]
#[path = "grammar_tests.rs"]
mod tests;
