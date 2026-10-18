#![no_main]
//! Target 2 — SQL fed to `query`, including authorizer-bypass attempts.
//!
//! `query` executes agent-supplied SQL against the project's own database.
//! Three guards stand between the text and the store, and all three are
//! downstream of this one public function:
//!
//! 1. the read-only connection (`store::connection::open_read_only`, opened
//!    with `SQLITE_OPEN_READ_ONLY` via the tool's verified-read transaction),
//! 2. the authorizer installed on every connection, which denies
//!    `ATTACH`/`DETACH` (`store::connection::guard_against_attach`),
//! 3. the subquery wrapper (`SELECT * FROM (\n…\n) LIMIT … OFFSET …`), which
//!    is what refuses a statement that is not a `SELECT`,
//!
//! A second statement is refused by SQLite's own parser (rusqlite's
//! `MultipleStatement`), not by any lexer in this crate.
//!
//! This target attacks all three from outside: the corpus carries the payload
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
//! Known gap: the `offset` contract error (`offset` above `i64::MAX`) is not
//! fuzzed here — it is a scalar bound already pinned by
//! `query_tests::an_offset_past_the_integer_range_is_refused`. The SQL text is
//! the subject.

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
    let fixture = &*support::FIXTURE;
    let request = Parameters(QueryRequest {
        path: fixture.path(),
        sql: String::from_utf8_lossy(data).into_owned(),
        offset: 0,
        expected_revision: None,
        page_bytes: None,
    });

    // A refusal is the expected outcome for most hostile inputs and is not
    // the target's subject; what a success must satisfy is the paging
    // contract: `truncated` and `next_offset` always travel together. (No
    // row or byte ceiling is asserted: the first row is always kept, even
    // over the byte budget.)
    if let Ok(Json(response)) = query(&request, &NoopProgressSink, &fixture.manager) {
        assert_eq!(
            response.truncated,
            response.next_offset.is_some(),
            "truncated and next_offset disagree"
        );
    }
});
