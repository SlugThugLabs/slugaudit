//! Stable, machine-readable failure codes and static tool-level remedies for
//! `rmcp::ErrorData.data`.

use std::borrow::Cow;

use serde_json::{Value, json};

macro_rules! define_error_codes {
    ($( $(#[$doc:meta])* $variant:ident => ($wire:literal, $remedy:expr) ),+ $(,)?) => {
        /// The closed set of machine-readable failure codes carried in `ErrorData.data`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum ErrorCode {
            $( $(#[$doc])* $variant, )+
        }

        impl ErrorCode {
            #[cfg(test)]
            pub(crate) const ALL: &'static [ErrorCode] = &[
                $( ErrorCode::$variant, )+
            ];

            /// The stable lowercase `snake_case` wire code.
            #[must_use]
            pub(crate) const fn as_str(self) -> &'static str {
                match self {
                    $( ErrorCode::$variant => $wire, )+
                }
            }

            /// Static one-sentence remedy naming the next tool action.
            #[must_use]
            pub(crate) const fn remedy(self) -> &'static str {
                match self {
                    $( ErrorCode::$variant => $remedy, )+
                }
            }
        }
    };
}

macro_rules! audit_profile_remedy {
    ($scope:literal, $middle:literal) => {
        concat!(
            "a permanent limit for this ",
            $scope,
            ", not a transient failure — retrying unchanged fails identically; ",
            $middle,
            "switch this session to the audit profile via project_control with ",
            "{\"action\":\"on\",\"profile\":\"audit\"}"
        )
    };
}

define_error_codes! {
    /// Sync wall-clock budget (`max_sync_wall_clock`) exhausted.
    SyncBudgetExhausted => (
        "sync_budget_exhausted",
        "a permanent limit for this profile, not a transient failure — retrying unchanged fails \
         identically; switch this session to the audit profile (90 s) instead of the lean profile \
         (30 s) via project_control with {\"action\":\"on\",\"profile\":\"audit\"}"
    ),
    /// No `.planning/slugaudit` directory exists on any ancestor of `path`.
    ProjectNotEnabled => (
        "project_not_enabled",
        "call project_control with {\"action\":\"on\",\"path\":\"<project directory>\"} to create \
         .planning/slugaudit and index the project"
    ),
    /// Concurrent publish or disk edit invalidated the pinned revision.
    StaleRevision => (
        "stale_revision",
        "retry the call — the sync in front of it re-reads the index and pins the new revision"
    ),
    /// Unknown mechanic id passed to `leads` or `mechanic`.
    UnknownMechanic => (
        "unknown_mechanic",
        "call leads with only `path` to list every valid mechanic id with its count, then retry \
         with one of them"
    ),
    /// SQL text exceeded `max_query_sql_bytes`.
    SqlTooLarge => (
        "sql_too_large",
        "Shorten the SQL statement: inline fewer literals or split the query into smaller SELECT \
         calls"
    ),
    /// Requested file has no indexed text content (missing, skipped, or binary).
    FileNotIndexed => (
        "file_not_indexed",
        "check what is indexed with SELECT path FROM files WHERE path LIKE … — a skipped \
         (oversize/unreadable) or binary file has no content to read"
    ),
    /// SQLite failed to compile or execute a `query` statement.
    SqlError => (
        "sql_error",
        "fix the statement from SQLite's message; list tables with SELECT name FROM sqlite_master \
         WHERE type='table', columns with SELECT name FROM pragma_table_info('<table>'), or read \
         slugaudit://schema.sql"
    ),
    /// Project-wide `structure` named a language no indexed file has.
    UnknownLanguage => (
        "unknown_language",
        "retry with one of the languages the message lists as indexed in this project, spelled \
         exactly as stored (report lists them under languages)"
    ),
    /// Malformed, foreign-tool, foreign-session, or out-of-bounds cursor.
    UnknownCursor => (
        "unknown_cursor",
        "copy the cursor exactly as returned, or repeat the request without one; a cursor does \
         not survive a server restart"
    ),
    /// Cursor's stored result was reclaimed to make room for newer results.
    ResultExpired => (
        "result_expired",
        "the result was reclaimed to make room: repeat the request without a cursor"
    ),
    /// All result-store budget bytes are held by active paged results.
    ResultStoreFull => (
        "result_store_full",
        "finish or abandon open paged reads, or narrow the request (LIMIT, filters)"
    ),
    /// Single result exceeds the total result-store byte budget.
    ResultTooLarge => (
        "result_too_large",
        "narrow the request: add LIMIT or filters; the whole result must fit the store budget"
    ),
    /// Result store cannot be opened or written on this machine.
    ResultStoreUnavailable => (
        "result_store_unavailable",
        "the result store cannot be used on this machine or its disk is full or unwritable; other \
         tools keep working"
    ),
    /// Removed `offset` or `expected_revision` parameter supplied.
    PagingParameterRemoved => (
        "paging_parameter_removed",
        "offset and expected_revision were removed; pass the cursor from the previous page's \
         page.cursor, or omit it for a new request"
    ),
    /// `query` exceeded its VM-step or wall-clock time budget.
    QueryBudgetExhausted => (
        "query_budget_exhausted",
        audit_profile_remedy!(
            "query",
            "narrow the query with WHERE or LIMIT, avoid cross joins, or "
        )
    ),
    /// `structure` exceeded `max_structure_execution_time`.
    StructureBudgetExhausted => (
        "structure_budget_exhausted",
        audit_profile_remedy!("scan", "narrow `query` or `pattern`, target one `file`, or ")
    ),
    /// `mechanic add`'s trial scan ended `partial`.
    MechanicPatternTooBroad => (
        "mechanic_pattern_too_broad",
        "Narrow the Tree-sitter query pattern with a concrete node kind, field name, or \
         `#eq?`/`#match?` predicate so its trial scan completes before registering it"
    ),
    /// Tree-sitter grammar is in fetch cool-down or failed to load.
    GrammarUnavailable => (
        "grammar_unavailable",
        "check the language name against `report`'s `languages` list, query `files.content` via \
         `query` while offline, or retry once the 10-minute grammar fetch cool-down elapses"
    ),
    /// Tree-sitter query failed to compile or violated capture-count rules.
    InvalidTreeSitterQuery => (
        "invalid_tree_sitter_query",
        "fix the S-expression from the compiler's message and name the required `@capture` \
         (`structure` needs at least one capture; `mechanic` `add` needs exactly one)"
    ),
    /// Parameter failed shape, presence, range, or byte-cap validation.
    InvalidInput => (
        "invalid_input",
        "adjust the named parameter to satisfy the tool's bounds and schema described in the \
         error message, then call again"
    ),
}

/// The `ErrorData.data` payload for `code`.
#[must_use]
pub(crate) fn payload(code: ErrorCode) -> Value {
    json!({ "code": code.as_str(), "remedy": code.remedy() })
}

/// Builds an `INVALID_PARAMS` [`rmcp::ErrorData`] carrying `code` and its static `remedy`.
#[must_use]
pub(crate) fn invalid_params_with_code(
    message: impl Into<Cow<'static, str>>,
    code: ErrorCode,
) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(message, Some(payload(code)))
}

/// [`payload`] for [`ErrorCode::SyncBudgetExhausted`] when `elapsed_ms` is `Some`.
#[must_use]
pub(crate) fn budget_payload(elapsed_ms: Option<u128>) -> Option<Value> {
    elapsed_ms.map(|_| payload(ErrorCode::SyncBudgetExhausted))
}

/// Maps a result store failure to its wire [`rmcp::ErrorData`].
#[must_use]
pub(crate) fn store_error(error: &crate::store::ResultStoreError) -> rmcp::ErrorData {
    use crate::store::ResultStoreError as E;
    let message = error.to_string();
    match error {
        E::Unavailable { .. } => rmcp::ErrorData::internal_error(
            message,
            Some(payload(ErrorCode::ResultStoreUnavailable)),
        ),
        E::Full { .. } => invalid_params_with_code(message, ErrorCode::ResultStoreFull),
        E::TooLarge => invalid_params_with_code(message, ErrorCode::ResultTooLarge),
        E::Expired => invalid_params_with_code(message, ErrorCode::ResultExpired),
        E::Unknown => invalid_params_with_code(message, ErrorCode::UnknownCursor),
        E::Lock(poisoned) => rmcp::ErrorData::from(poisoned.clone()),
        E::Clock(clock) => rmcp::ErrorData::from(clock.clone()),
        E::Invalid { .. } | E::Sqlite { .. } | E::Json { .. } | E::Corrupt { .. } => {
            rmcp::ErrorData::internal_error(message, None)
        }
    }
}

#[cfg(test)]
#[path = "error_codes_tests.rs"]
mod tests;
