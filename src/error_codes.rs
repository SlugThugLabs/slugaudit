// slugaudit-line-exception: approved-by=slugthug; reason=the closed set of wire error codes: the enum, its wire strings, remedies, payload builders and the one store-error mapping must stay in one table so a code cannot drift from its remedy
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
//! `ErrorCode::ALL` in both directions by the inline test below: a code
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
    /// An auditor-registered mechanic's scan spent the call's
    /// `max_structure_execution_time` before every file was read, so no
    /// count is returned. Permanent for the same scan, like the sync budget.
    MechanicBudgetExhausted,
    /// SQLite could not compile or run the statement passed to `query` — a
    /// syntax error, an unknown table or column, a function or type error.
    /// Not a refusal of the statement's kind (those carry no code) and not a
    /// spent query budget.
    SqlError,
    /// A project-wide `structure` call named a language no indexed file in
    /// this project has. The message lists the languages the index holds;
    /// the remedy stays static.
    UnknownLanguage,
    /// A `cursor` that is malformed, was issued by another process or
    /// server run, belongs to another tool, or points past its result. A
    /// cursor does not survive a server restart.
    UnknownCursor,
    /// A well-formed cursor whose stored result was reclaimed to make room
    /// for a newer one.
    ResultExpired,
    /// The result store has no room for a new result because every byte is
    /// held by results still being paged.
    ResultStoreFull,
    /// One result alone is larger than the whole result store budget.
    ResultTooLarge,
    /// The result store cannot hold or serve results on this machine: its
    /// disk is full or unwritable, or the platform lacks the file semantics
    /// it needs. Other tools keep working.
    ResultStoreUnavailable,
    /// A request carried `offset` or `expected_revision`, the pre-cursor
    /// paging parameters that were removed when the tools moved to cursors.
    PagingParameterRemoved,
}

impl ErrorCode {
    /// Every code, in documentation order. The `ARCHITECTURE.md` table is
    /// checked against this list in both directions, so adding a code means
    /// adding its row in the same change. Test-only: the closed set's
    /// caller-facing contract is the enum and its payloads, not this Rust
    /// enumeration.
    #[cfg(test)]
    pub(crate) const ALL: &'static [ErrorCode] = &[
        ErrorCode::SyncBudgetExhausted,
        ErrorCode::ProjectNotEnabled,
        ErrorCode::StaleRevision,
        ErrorCode::UnknownMechanic,
        ErrorCode::SqlTooLarge,
        ErrorCode::FileNotIndexed,
        ErrorCode::MechanicBudgetExhausted,
        ErrorCode::SqlError,
        ErrorCode::UnknownLanguage,
        ErrorCode::UnknownCursor,
        ErrorCode::ResultExpired,
        ErrorCode::ResultStoreFull,
        ErrorCode::ResultTooLarge,
        ErrorCode::ResultStoreUnavailable,
        ErrorCode::PagingParameterRemoved,
    ];

    /// The stable wire string: lowercase `snake_case`, never renamed.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::SyncBudgetExhausted => "sync_budget_exhausted",
            ErrorCode::ProjectNotEnabled => "project_not_enabled",
            ErrorCode::StaleRevision => "stale_revision",
            ErrorCode::UnknownMechanic => "unknown_mechanic",
            ErrorCode::SqlTooLarge => "sql_too_large",
            ErrorCode::FileNotIndexed => "file_not_indexed",
            ErrorCode::MechanicBudgetExhausted => "mechanic_budget_exhausted",
            ErrorCode::SqlError => "sql_error",
            ErrorCode::UnknownLanguage => "unknown_language",
            ErrorCode::UnknownCursor => "unknown_cursor",
            ErrorCode::ResultExpired => "result_expired",
            ErrorCode::ResultStoreFull => "result_store_full",
            ErrorCode::ResultTooLarge => "result_too_large",
            ErrorCode::ResultStoreUnavailable => "result_store_unavailable",
            ErrorCode::PagingParameterRemoved => "paging_parameter_removed",
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
            ErrorCode::MechanicBudgetExhausted => {
                "a permanent limit for this scan, not a transient failure — retrying unchanged \
                 fails identically; remove the named entry with mechanic action remove, narrow \
                 leads `pattern`, or raise SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS (seconds)"
            }
            ErrorCode::SqlError => {
                "fix the statement from SQLite's message; list tables with SELECT name FROM \
                 sqlite_master WHERE type='table', columns with SELECT name FROM \
                 pragma_table_info('<table>'), or read slugaudit://schema.sql"
            }
            ErrorCode::UnknownLanguage => {
                "retry with one of the languages the message lists as indexed in this project, \
                 spelled exactly as stored (report lists them under languages)"
            }
            ErrorCode::UnknownCursor => {
                "copy the cursor exactly as returned, or repeat the request without one; a \
                 cursor does not survive a server restart"
            }
            ErrorCode::ResultExpired => {
                "the result was reclaimed to make room: repeat the request without a cursor"
            }
            ErrorCode::ResultStoreFull => {
                "finish or abandon open paged reads, or narrow the request (LIMIT, filters)"
            }
            ErrorCode::ResultTooLarge => {
                "narrow the request: add LIMIT or filters; the whole result must fit the \
                 store budget"
            }
            ErrorCode::ResultStoreUnavailable => {
                "the result store cannot be used on this machine or its disk is full or \
                 unwritable; see docs/TROUBLESHOOTING.md; other tools keep working"
            }
            ErrorCode::PagingParameterRemoved => {
                "offset and expected_revision were removed; pass the cursor from the previous \
                 page's page.cursor, or omit it for a new request"
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

/// The one mapping from a result store failure to the wire. The store logged
/// the failure where it decided it, so this does not log again. Messages come
/// from the error's own `Display`, which holds only static contexts and the
/// active-result count, never a path, SQL or row content. Failures that are
/// neither a caller mistake nor an unusable store (`Invalid`, `Io`, `Sqlite`,
/// `Json`, `Corrupt`) are internal errors with no code.
#[must_use]
pub fn store_error(error: &crate::store::ResultStoreError) -> rmcp::ErrorData {
    use crate::store::ResultStoreError as E;
    let message = error.to_string();
    let invalid = |code| rmcp::ErrorData::invalid_params(message.clone(), payload(code));
    match error {
        E::Unavailable { .. } => rmcp::ErrorData::internal_error(
            message.clone(),
            payload(ErrorCode::ResultStoreUnavailable),
        ),
        E::Full { .. } => invalid(ErrorCode::ResultStoreFull),
        E::TooLarge => invalid(ErrorCode::ResultTooLarge),
        E::Expired => invalid(ErrorCode::ResultExpired),
        E::Unknown => invalid(ErrorCode::UnknownCursor),
        E::Invalid { .. }
        | E::Io { .. }
        | E::Sqlite { .. }
        | E::Json { .. }
        | E::Corrupt { .. } => rmcp::ErrorData::internal_error(message.clone(), None),
    }
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

    /// Each store failure maps to exactly one wire code (or none for the
    /// internal faults), with the static remedy and a message that carries
    /// no more than the error's own text.
    #[test]
    fn store_errors_map_to_their_wire_codes_and_static_remedies() {
        use crate::store::ResultStoreError as E;
        let unavailable = E::Unavailable {
            context: "open the result store",
            source: Box::new(std::io::Error::other("secret path /x")),
        };
        let cases = [
            (E::Unknown, Some(ErrorCode::UnknownCursor), false),
            (E::Expired, Some(ErrorCode::ResultExpired), false),
            (
                E::Full { active_count: 3 },
                Some(ErrorCode::ResultStoreFull),
                false,
            ),
            (E::TooLarge, Some(ErrorCode::ResultTooLarge), false),
            (unavailable, Some(ErrorCode::ResultStoreUnavailable), true),
            (E::Corrupt { context: "x" }, None, true),
        ];

        for (error, code, internal) in cases {
            let mapped = store_error(&error);

            let expected = code.map(|code| json!({"code": code.as_str(), "remedy": code.remedy()}));
            assert_eq!(mapped.data, expected, "{error:?}");
            assert_eq!(
                mapped.code == rmcp::model::ErrorCode::INTERNAL_ERROR,
                internal,
                "{error:?}"
            );
            assert_eq!(mapped.message, error.to_string());
            assert!(!mapped.message.contains("secret"), "{}", mapped.message);
        }
        assert_eq!(
            store_error(&crate::store::ResultStoreError::Full { active_count: 3 }).message,
            "the result store is full: 3 active results hold the space"
        );
    }
}
