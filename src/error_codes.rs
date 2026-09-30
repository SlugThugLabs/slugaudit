//! Stable, machine-readable failure codes.
//!
//! rmcp's `ErrorData` carries a free-form `data` value beside its human
//! message, and that value is the only machine-readable channel a tool
//! response has. This module owns the closed set of codes that may go in
//! it, so a caller can branch on *what failed* instead of parsing prose,
//! and so the set cannot grow one call site at a time.
//!
//! Three conventions, all load-bearing:
//!
//! - A code is lowercase `snake_case`, names the *condition* rather than
//!   the tool or the call site that detected it, and is stable once
//!   shipped: renaming one breaks every caller that branches on it.
//! - The payload is always `{"code": …, "remedy": …}`. `remedy` is the
//!   caller's next action — the client call to make, the environment knob
//!   to set, or "retry" where retrying is genuinely the right move. It
//!   exists so a failure is self-healing instead of something the caller
//!   re-issues unchanged and concludes is a broken tool.
//! - Both strings are static. Never interpolate agent-supplied text (a
//!   path, SQL, a query string, a finding body, file content) into a code
//!   or a remedy: a remedy is surfaced verbatim to the caller and into the
//!   server log, and the redaction contract in `tools/redaction_tests.rs`
//!   keeps that text out of both.
//!
//! The codes are documented in `.planning/ARCHITECTURE.md` (the table
//! between the `error-codes` markers) and checked against
//! [`ErrorCode::ALL`] in both directions by the inline test below: a code
//! cannot be added without its row, and a row cannot outlive its code.

use serde_json::{Value, json};

/// The closed set of machine-readable failure codes. See the module docs
/// for the conventions; each variant documents its own condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The sync wall-clock budget (`ResourceLimits::max_sync_wall_clock`)
    /// was exhausted while discovering, hashing, parsing, or publishing
    /// the project. On a tree that is simply too big for the budget this
    /// is permanent, not transient, so "retry" is the wrong advice and
    /// the remedy names the knob instead.
    SyncBudgetExhausted,
    /// No ancestor of the requested path has a SlugAudit activation
    /// marker (`.planning/slugaudit`), so no tool can serve that path.
    ProjectNotEnabled,
    /// The revision this call was pinned to is no longer current — another
    /// publish won the compare-and-swap race, or a sampled file changed on
    /// disk before the write that would have recorded it. The caller's own
    /// snapshot is stale, and retrying (which re-syncs) is correct.
    StaleRevision,
    /// `leads` was asked for a mechanic id that is not in the catalogue.
    UnknownMechanic,
    /// The SQL passed to `query` exceeded `max_query_sql_bytes`.
    SqlTooLarge,
    /// The file named by the request has no indexed source content — never
    /// discovered, skipped as oversize/unreadable, or binary — so it
    /// cannot be read or parsed out of the index.
    FileNotIndexed,
}

impl ErrorCode {
    /// Every code, in documentation order. The `ARCHITECTURE.md` table is
    /// checked against this list in both directions, so adding a code
    /// means adding its row in the same change.
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::SyncBudgetExhausted,
        ErrorCode::ProjectNotEnabled,
        ErrorCode::StaleRevision,
        ErrorCode::UnknownMechanic,
        ErrorCode::SqlTooLarge,
        ErrorCode::FileNotIndexed,
    ];

    /// The stable wire string: lowercase `snake_case`, never renamed.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::SyncBudgetExhausted => "sync_budget_exhausted",
            ErrorCode::ProjectNotEnabled => "project_not_enabled",
            ErrorCode::StaleRevision => "stale_revision",
            ErrorCode::UnknownMechanic => "unknown_mechanic",
            ErrorCode::SqlTooLarge => "sql_too_large",
            ErrorCode::FileNotIndexed => "file_not_indexed",
        }
    }

    /// The caller's next action, in one sentence. Static by contract —
    /// never agent-supplied text (see the module docs).
    #[must_use]
    pub const fn remedy(self) -> &'static str {
        match self {
            ErrorCode::SyncBudgetExhausted => {
                "a permanent limit, not a transient failure — retrying unchanged fails \
                 identically; raise SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS (seconds), or run \
                 SLUGAUDIT_PROFILE=audit (1200 s) instead of the lean profile (120 s)"
            }
            ErrorCode::ProjectNotEnabled => {
                "call project_control with {\"action\":\"on\",\"path\":\"<project \
                 directory>\"} to create .planning/slugaudit and index the project"
            }
            ErrorCode::StaleRevision => {
                "retry the call — the sync in front of it re-reads the index and pins the \
                 new revision"
            }
            ErrorCode::UnknownMechanic => {
                "call leads with only `path` to list every valid mechanic id with its count, \
                 then retry with one of them"
            }
            ErrorCode::SqlTooLarge => {
                "shorten the SQL, or raise SLUGAUDIT_MAX_QUERY_SQL_BYTES (bytes)"
            }
            ErrorCode::FileNotIndexed => {
                "check what is indexed with SELECT path FROM files WHERE path LIKE … — a \
                 skipped (oversize/unreadable) or binary file has no content to read"
            }
        }
    }
}

/// The `ErrorData.data` payload for `code`. Always `Some` — the `Option`
/// mirrors `ErrorData`'s constructor parameter so a call site forwards it
/// without wrapping.
#[must_use]
pub fn payload(code: ErrorCode) -> Option<Value> {
    Some(json!({ "code": code.as_str(), "remedy": code.remedy() }))
}

/// [`payload`] for [`ErrorCode::SyncBudgetExhausted`] when the sync
/// actually ran out of wall-clock budget (`elapsed_ms` is `Some`), else
/// `None`. The single predicate behind every budget map site: the sync
/// error types carry the elapsed time in their own variant, so the code is
/// attached where that typed error is still in hand — discovery, publish,
/// reconcile, barrier, sweep — rather than guessed at from a message at
/// the outermost tool.
#[must_use]
pub fn budget_payload(elapsed_ms: Option<u128>) -> Option<Value> {
    if elapsed_ms.is_some() {
        payload(ErrorCode::SyncBudgetExhausted)
    } else {
        None
    }
}

/// [`payload`] for [`ErrorCode::StaleRevision`] when a *paged* read is
/// resumed against a revision that is no longer current — the caller
/// supplied an `expected_revision` and the index moved on since.
///
/// The code is the same condition as [`ErrorCode::StaleRevision`], but the
/// remedy is not: re-issuing a resumed page at the same `offset` continues
/// a sweep whose row order no longer matches the rows already returned, so
/// "retry" would quietly skip or repeat rows. The remedy restarts the sweep
/// instead. This mirrors [`budget_payload`], which likewise selects a
/// different payload for one code depending on what actually happened.
#[must_use]
pub fn stale_page_payload() -> Option<Value> {
    Some(json!({
        "code": ErrorCode::StaleRevision.as_str(),
        "remedy": "restart the sweep from offset 0 — the revision changed since the page you \
                   would be resuming, so the same offset can skip or repeat rows; the call \
                   you are making already re-synced the index"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The wire strings are what a caller branches on, so they must be
    /// unique, non-empty, lowercase `snake_case`, and each must carry a
    /// non-empty remedy.
    #[test]
    fn every_code_is_unique_non_empty_and_lowercase_snake_case() {
        let mut seen = BTreeSet::new();
        for code in ErrorCode::ALL {
            let text = code.as_str();
            assert!(!text.is_empty(), "{code:?} has an empty wire string");
            assert!(
                text.chars()
                    .all(|character| character.is_ascii_lowercase() || character == '_'),
                "{text} must be lowercase snake_case"
            );
            assert!(seen.insert(text), "{text} is not unique");
            assert!(!code.remedy().is_empty(), "{text} has no remedy");
        }
    }

    /// The documented table and the code set cannot drift apart: a code
    /// added without its row fails here, and a row whose code was removed
    /// fails here too. This is the whole point of the closed set.
    #[test]
    fn the_documented_table_matches_the_code_set_exactly() {
        let architecture = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/.planning/ARCHITECTURE.md"
        ))
        .expect("read ARCHITECTURE.md");
        let documented = documented_codes(&architecture);
        let implemented: Vec<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(
            documented, implemented,
            "the error-code table in ARCHITECTURE.md must list exactly ErrorCode::ALL, in \
             the same order"
        );
    }

    /// The first backticked cell of every table row between the
    /// `error-codes` markers — the table's code column, in document order.
    fn documented_codes(architecture: &str) -> Vec<String> {
        let begin = architecture
            .find("<!-- error-codes:begin -->")
            .expect("the error-codes table needs its begin marker");
        let end = architecture
            .find("<!-- error-codes:end -->")
            .expect("the error-codes table needs its end marker");
        architecture[begin..end]
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("| `"))
            .map(|row| row.split('`').next().unwrap_or_default().to_owned())
            .collect()
    }

    /// The budget case is the one that must not read as "retry": its
    /// payload names the knob and the profiles, and a non-budget failure
    /// gets no budget code.
    #[test]
    fn the_budget_payload_names_the_knob_and_only_the_budget_gets_it() {
        let payload = budget_payload(Some(120_000)).expect("a spent budget carries a payload");
        assert_eq!(payload["code"], "sync_budget_exhausted");
        let remedy = payload["remedy"].as_str().expect("remedy is a string");
        assert!(
            remedy.contains("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS"),
            "the remedy must name the knob: {remedy}"
        );
        assert!(
            remedy.contains("audit") && remedy.contains("lean"),
            "the remedy must name the profiles: {remedy}"
        );
        assert!(
            budget_payload(None).is_none(),
            "a non-budget failure must not carry the budget code"
        );
    }

    /// A resumed page's refusal carries the same code as the read-path
    /// stale-revision guard but not the same remedy: "retry" would resume
    /// the sweep at an offset that no longer means what it did.
    #[test]
    fn the_stale_page_payload_keeps_the_code_and_restarts_the_sweep() {
        let payload = stale_page_payload().expect("a stale page carries a payload");
        assert_eq!(payload["code"], "stale_revision");
        let remedy = payload["remedy"].as_str().expect("remedy is a string");
        assert!(
            remedy.contains("offset 0"),
            "the remedy must restart the sweep: {remedy}"
        );
        assert_ne!(
            remedy,
            ErrorCode::StaleRevision.remedy(),
            "a resumed page must not be told to simply retry"
        );
    }
}
