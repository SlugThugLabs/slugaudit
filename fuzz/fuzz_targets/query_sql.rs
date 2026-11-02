#![no_main]
//! Target 2 — SQL fed to `query`, including statements that are not a plain
//! `SELECT`.
//!
//! `query` executes agent-supplied SQL against the project's own database.
//! Two things stand between the text and the store, and both are downstream
//! of this one public function:
//!
//! 1. the read-only connection (`store::connection::open_existing`, opened
//!    with `SQLITE_OPEN_READ_ONLY` via the tool's verified-read transaction),
//!    a connection mode rather than a trust boundary,
//! 2. the subquery wrapper (`SELECT * FROM (\n…\n) LIMIT -1 OFFSET 0`), which
//!    is what refuses a statement that is not a `SELECT`,
//!
//! A second statement is refused by SQLite's own parser (rusqlite's
//! `MultipleStatement`), not by any lexer in this crate.
//!
//! This target attacks both from outside: the corpus carries the payload
//! tables from `src/tools/query_safety_tests.rs` (`ATTACH`/`DETACH`,
//! `PRAGMA` write forms, `VACUUM INTO`, `load_extension`, filesystem
//! functions, data-modifying CTEs, `;` splices past string literals and
//! comments) beside the real queries in `examples/queries/` and the schema in
//! `src/store/schema.sql`.
//!
//! The project fixture is built once per process (see `support.rs`), so each
//! iteration is one `query` call and the index is never rebuilt between
//! inputs.
//!
//! The SQL text is the subject; paging is the result store's and is not
//! fuzzed here.

use libfuzzer_sys::fuzz_target;
use rmcp::handler::server::wrapper::{Json, Parameters};
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::tools::{QueryRequest, query};

#[path = "support.rs"]
mod support;

/// `ResourceLimits::default().max_query_sql_bytes` is 50_000; a longer input is
/// refused before the wrapper is built.
const MAX_SQL_BYTES: usize = 50_000;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_SQL_BYTES {
        return;
    }
    let Some(fixture) = support::fixture() else {
        return;
    };
    let request = Parameters(QueryRequest {
        path: Some(fixture.path()),
        sql: String::from_utf8_lossy(data).into_owned(),
        cursor: None,
        legacy_paging: Default::default(),
        page_bytes: None,
    });

    // A refusal is the expected outcome for most hostile inputs and is not
    // the target's subject; what a success must satisfy is the paging
    // contract: `page.complete` is false exactly when `page.cursor` is
    // present. (No row or byte ceiling is asserted: the first row is always
    // kept, even over the byte budget.)
    if let Ok(Json(response)) = query(&request, &NoopProgressSink, &fixture.manager) {
        assert_eq!(
            !response.page.complete,
            response.page.cursor.is_some(),
            "page.complete and page.cursor disagree"
        );
    }
});
