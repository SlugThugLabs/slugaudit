//! The adversarial half of the `query` tool's boundary. Every test here
//! asserts *rejection* — the plan's wording, "asserting rejection, not just
//! 'no rows'". A silently empty page and a statement refused before
//! execution are indistinguishable to a caller that never inspects the
//! error, so nothing here is satisfied by an `Ok` with no rows, and every
//! test that claims nothing changed re-reads the index through the same
//! tool rather than trusting the refusal. Split out of `query_tests.rs`
//! (one property per test, and the payload tables are the bulk of it) to
//! keep that file inside the test-file code-line cap.
use super::*;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use std::time::Duration;

/// The fixture every payload below attacks: one indexed `lib.rs`.
fn project() -> tempfile::TempDir {
    activated_project("lib.rs", b"pub fn a() {}\n")
}

fn ask(project: &tempfile::TempDir, sql: &str) -> Result<QueryResponse, ErrorData> {
    query(
        &Parameters(QueryRequest {
            path: project.path().to_string_lossy().into_owned(),
            sql: sql.to_owned(),
            offset: 0,
            expected_revision: None,
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

/// Demands that `sql` be refused. The panic names the false pass: an `Ok`
/// here would mean a tool advertising a read-only connection executed the
/// payload anyway.
fn refused(project: &tempfile::TempDir, sql: &str, what: &str) -> ErrorData {
    match ask(project, sql) {
        Err(error) => error,
        Ok(response) => panic!(
            "`{sql}` was accepted and returned {} row(s) instead of an error; a false pass here \
             means {what} was not actually refused",
            response.rows.len()
        ),
    }
}

/// Refuses every payload, then re-reads the fixture through the same tool:
/// a refusal response alone cannot prove the index is unchanged.
fn all_refused(project: &tempfile::TempDir, payloads: &[&str], what: &str) {
    for sql in payloads {
        refused(project, sql, what);
    }
    let after = ask(project, "SELECT path, language FROM files").expect("the index still reads");
    assert_eq!(
        after.rows,
        vec![serde_json::json!({"path": "lib.rs", "language": "rust"})],
        "{what}: the index changed, so a refused payload still had an effect"
    );
}

/// Two shapes of CTE write, both refused and both pinned by re-reading the
/// index: a data-modifying CTE referenced by a `SELECT`, and a CTE feeding
/// a top-level `INSERT`/`UPDATE`/`DELETE ... RETURNING`. The second shape
/// parses on its own, so it is refused by the tool's wrapper — and, with
/// the wrapper experimentally removed, by the read-only connection itself
/// ("attempt to write a readonly database"), which is the guard the
/// contract claims writes rest on.
#[test]
fn writes_through_a_data_modifying_cte_are_refused() {
    let project = project();

    all_refused(
        &project,
        &[
            "WITH x AS (INSERT INTO files(path, language) VALUES ('planted.rs', 'rust') RETURNING path) SELECT * FROM x",
            "WITH x AS (UPDATE files SET path = 'planted.rs' RETURNING path) SELECT * FROM x",
            "WITH x AS (DELETE FROM files RETURNING path) SELECT * FROM x",
            "WITH RECURSIVE x AS (DELETE FROM files RETURNING path) SELECT * FROM x",
            "WITH x AS (SELECT 1 AS a) INSERT INTO files(path, language) SELECT 'planted.rs', 'rust' FROM x RETURNING path",
            "WITH x AS (SELECT 1 AS a) UPDATE files SET path = 'planted.rs' RETURNING path",
            "WITH x AS (SELECT 1 AS a) DELETE FROM files RETURNING path",
        ],
        "a write through a data-modifying CTE",
    );
}

/// The same write attempt one nesting level deeper: a scalar subquery and
/// an `EXISTS` subquery are both read-shaped positions, and a write must
/// not become reachable by wrapping it in one.
#[test]
fn writes_through_a_scalar_or_exists_subquery_are_refused() {
    let project = project();

    all_refused(
        &project,
        &[
            "SELECT (WITH x AS (DELETE FROM files RETURNING path) SELECT path FROM x) AS smuggled",
            "SELECT EXISTS (SELECT 1 FROM (WITH x AS (UPDATE files SET path = 'planted.rs' RETURNING path) SELECT path FROM x)) AS smuggled",
            "SELECT (SELECT (WITH x AS (INSERT INTO files(path, language) VALUES ('p.rs', 'rust') RETURNING path) SELECT path FROM x)) AS smuggled",
        ],
        "a write through a scalar or EXISTS subquery",
    );
}

/// `PRAGMA` is a statement kind, not a `SELECT`, so the wrapper refuses
/// both its read and write forms — as it refuses every other statement
/// kind the tool refuses to express. The knob values are re-read through
/// the tool afterwards, so a write form that somehow took effect before
/// the error is a failure, not a shrug.
#[test]
fn pragma_and_other_non_select_statements_smuggled_into_the_subquery_are_refused() {
    let project = project();
    let before = writable_knobs(&project);

    all_refused(
        &project,
        &[
            "PRAGMA table_info(files)",
            "PRAGMA schema_version",
            "PRAGMA main.table_info(files)",
            "PRAGMA user_version = 7",
            "PRAGMA journal_mode = DELETE",
            "PRAGMA writable_schema = ON",
            "PRAGMA query_only = OFF",
            "VACUUM",
            "VACUUM INTO '/tmp/slugaudit-vacuum.db'",
            "ANALYZE",
            "REINDEX",
            "CREATE TABLE planted (x)",
            "DROP TABLE files",
            "BEGIN",
            "COMMIT",
            "SAVEPOINT s",
            "EXPLAIN SELECT 1",
        ],
        "a non-SELECT statement smuggled into the subquery",
    );

    // `VACUUM INTO` is the one payload here that would leave a whole
    // database file behind at an arbitrary path, so the refusal is checked
    // against the filesystem too.
    assert!(
        !std::path::Path::new("/tmp/slugaudit-vacuum.db").exists(),
        "a refused VACUUM INTO still wrote a database file to /tmp"
    );

    assert_eq!(
        writable_knobs(&project),
        before,
        "a refused write-form PRAGMA still changed a knob"
    );
}

/// `user_version` and `journal_mode`, read on a plain connection without the
/// read-only authorizer (which denies both pragmas, so the tool cannot read
/// them any more). A first `query` syncs the project so the file exists.
fn writable_knobs(project: &tempfile::TempDir) -> (i64, String) {
    ask(project, "SELECT 1 AS synced").expect("sync the project");
    let database = project.path().join(".planning/slugaudit/project.db");
    let connection = rusqlite::Connection::open(database).expect("open without the authorizer");
    connection
        .query_row(
            "SELECT (SELECT * FROM pragma_user_version), (SELECT * FROM pragma_journal_mode)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read the knobs")
}

/// `load_extension` is an ordinary `SELECT`, so the wrapper cannot refuse
/// it: what stops it is SQLite's own disabled-by-default extension
/// loading. The filesystem functions the CLI builds ship are not compiled
/// into this library at all. `writefile` is the one payload here that
/// could leave a trace, so the absence of that file is asserted directly.
#[test]
fn functions_that_could_leave_the_sandbox_are_refused_or_inert() {
    let project = project();
    let escaped = std::path::Path::new("/tmp/slugaudit-escape.txt");

    for sql in [
        "SELECT load_extension('/tmp/slugaudit-escape.so') AS x",
        "SELECT load_extension('/tmp/slugaudit-escape.so', 'sqlite3_extension_init') AS x",
        "SELECT readfile('/etc/passwd') AS x",
        "SELECT writefile('/tmp/slugaudit-escape.txt', 'escaped') AS x",
        "SELECT edit('/etc/passwd') AS x",
    ] {
        refused(
            &project,
            sql,
            "a function that reads or writes the filesystem",
        );
    }
    assert!(
        !escaped.exists(),
        "a refused writefile() payload still created {escaped:?}"
    );

    // `fts3_tokenizer` is compiled in but disabled, which is a boundary in
    // both directions: the read form is an inert NULL rather than a raw
    // pointer, and the form that installs a pointer errors. If a build ever
    // re-enables it, the first assertion fails on a leaked pointer.
    let reader = ask(&project, "SELECT fts3_tokenizer('simple') AS t").expect("reader form runs");
    assert_eq!(
        reader.rows[0]["t"],
        serde_json::Value::Null,
        "fts3_tokenizer returned a value instead of NULL: {:?}",
        reader.rows[0]
    );
    let installer = ask(
        &project,
        "SELECT fts3_tokenizer('simple', zeroblob(8)) AS t",
    );
    assert!(
        installer.is_err(),
        "the pointer-installing fts3_tokenizer form must be refused, got: {:?}",
        installer.ok()
    );
}

/// Statement splicing. A real top-level `;` is caught by the separator
/// scanner with the friendly typed message — even when a string literal or
/// a comment sits immediately before it. A `;` *inside* a comment hides
/// the splice from that scanner, so the whole text reaches the wrapper as
/// one statement: the wrapper is then the guard that refuses it.
#[test]
fn a_second_statement_cannot_be_spliced_past_the_single_statement_guard() {
    let project = project();

    for sql in [
        "SELECT 1; DELETE FROM files",
        "SELECT 'literal;text' AS x; DELETE FROM files",
        "SELECT 1 -- comment\n; DELETE FROM files",
        "SELECT 1 /* comment */; DELETE FROM files",
    ] {
        let error = refused(&project, sql, "a second statement spliced after the first");
        assert!(
            error.message.contains("more than one statement"),
            "{sql}: expected the separator-scanner refusal, got: {}",
            error.message
        );
    }

    all_refused(
        &project,
        &[
            "SELECT 1 -- ;\nDELETE FROM files",
            "SELECT 1 /* ; */ DELETE FROM files",
            "DELETE FROM files -- trailing comment",
            "DELETE FROM files;",
        ],
        "a write spliced past the separator scanner",
    );
}

/// The other half of the boundary, and the reason this suite's rejections
/// are not vacuous: `sqlite_master` and the `pragma_*` table-valued
/// functions are exactly what the server instructions tell the agent to
/// run, and an empty result is a *successful* read, not a refusal. If this
/// test fails, the suite above is passing because everything fails.
#[test]
fn legal_reads_of_the_schema_and_pragma_table_functions_are_allowed() {
    let project = project();

    let tables = ask(
        &project,
        "SELECT name FROM sqlite_master WHERE type = 'table'",
    )
    .expect("sqlite_master reads");
    assert!(
        tables.rows.iter().any(|row| row["name"] == "files"),
        "legal schema introspection must work; a false pass here means ordinary reads are \
         being refused too: {:?}",
        tables.rows
    );

    let columns = ask(&project, "SELECT name FROM pragma_table_info('files')")
        .expect("pragma table-valued functions read");
    assert!(
        columns.rows.iter().any(|row| row["name"] == "path"),
        "pragma table-valued functions must work; a false pass here means every SELECT is \
         being refused: {:?}",
        columns.rows
    );

    let empty = ask(&project, "SELECT path FROM files WHERE 0").expect("an empty read succeeds");
    assert!(
        empty.rows.is_empty(),
        "an empty read returns Ok, not an error; a false pass here means this suite cannot \
         tell 'no rows' from 'refused': {:?}",
        empty.rows
    );
}

/// The request schema says only SELECT is accepted. `PRAGMA table_info` and
/// `EXPLAIN SELECT 1` are read-only, but neither is a SELECT, so the wrapper
/// refuses each before execution, saying what `query` accepts and keeping
/// the compile error SQLite reports for the wrapped statement.
#[test]
fn read_only_non_select_statements_are_refused_by_the_wrapper() {
    let project = project();
    let refusal = "sql was refused: `query` accepts a single read-only SELECT/WITH statement, \
                   and this is another kind of statement (PRAGMA, EXPLAIN, a write, DDL or a \
                   transaction command), so it cannot run as a subquery (SQLite: ";

    for (sql, expected) in [
        (
            "PRAGMA table_info(files)",
            format!(
                "{refusal}near \"(\": syntax error in SELECT * FROM (\nPRAGMA \
                 table_info(files)\n) LIMIT -1 OFFSET 0 at offset 33)"
            ),
        ),
        (
            "EXPLAIN SELECT 1",
            format!(
                "{refusal}near \"SELECT\": syntax error in SELECT * FROM (\nEXPLAIN SELECT \
                 1\n) LIMIT -1 OFFSET 0 at offset 24)"
            ),
        ),
    ] {
        let error = refused(&project, sql, "a read-only non-SELECT statement");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{sql}");
        assert_eq!(error.message.as_ref(), expected.as_str(), "{sql}");
        assert!(
            error.data.is_none(),
            "{sql}: a wrapper refusal carries no machine-readable code"
        );
    }
}

#[test]
fn a_comments_only_statement_is_refused_rather_than_run_as_an_empty_query() {
    let project = project();

    let error = refused(
        &project,
        "-- only a comment, no statement",
        "a comments-only statement",
    );

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message.as_ref(),
        "sql must contain a statement, not only comments"
    );
}

#[test]
fn a_query_path_over_the_cap_is_refused_before_any_sync() {
    let error = query_with_limits(
        &Parameters(QueryRequest {
            path: "x".repeat(32 * 1024 + 1),
            sql: "SELECT 1".to_owned(),
            offset: 0,
            expected_revision: None,
            page_bytes: None,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("an oversized path must be refused");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message.as_ref(), "path exceeds 32768 bytes");
}

#[test]
fn an_expected_revision_over_the_query_cap_is_refused_before_any_sync() {
    let directory = tempfile::tempdir().expect("project dir");
    let error = query_with_limits(
        &Parameters(QueryRequest {
            path: directory.path().to_string_lossy().into_owned(),
            sql: "SELECT 1".to_owned(),
            offset: 0,
            expected_revision: Some("x".repeat(257)),
            page_bytes: None,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("an oversized expected_revision must be refused");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message.as_ref(),
        "expected_revision exceeds 256 bytes"
    );
}
/// Demands the clear refusal: `invalid_params`, saying what `query`
/// accepts, never the comments-only message.
fn assert_refused_as_not_a_select(error: &ErrorData, sql: &str) {
    let opening = "sql was refused: `query` accepts a single read-only SELECT/WITH statement";
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{sql}");
    assert!(
        error.message.starts_with(opening) && !error.message.contains("only comments"),
        "{sql}: unexpected message: {}",
        error.message
    );
}

/// A process-wide pragma as a second, freshly opened connection reads it
/// (`None` when it has no value).
fn fresh_pragma(pragma: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    rusqlite::Connection::open_in_memory()
        .expect("a freshly opened connection")
        .query_row(&format!("PRAGMA {pragma}"), [], |row| {
            row.get::<_, rusqlite::types::Value>(0)
        })
        .optional()
        .expect("read the pragma")
        .map(|value| format!("{value:?}"))
}

/// SQLite applies these pragmas while *preparing* the statement and each is
/// process-wide: a refusal that came after the prepare would leave, for
/// `hard_heap_limit=1`, every later database open in the process failing.
/// A fresh connection reads each value unchanged and a `report` still runs.
#[test]
fn refused_prepare_time_pragmas_leave_process_wide_state_unchanged() {
    let project = project();
    let elsewhere = tempfile::tempdir().expect("a directory the pragma would point at");

    for (pragma, setter) in [
        ("hard_heap_limit", "PRAGMA hard_heap_limit=1".to_owned()),
        (
            "soft_heap_limit",
            "PRAGMA soft_heap_limit=123456789".to_owned(),
        ),
        (
            "temp_store_directory",
            format!(
                "PRAGMA temp_store_directory='{}'",
                elsewhere.path().display()
            ),
        ),
    ] {
        let before = fresh_pragma(pragma);
        let error = refused(&project, &setter, "a prepare-time PRAGMA");
        assert_refused_as_not_a_select(&error, &setter);
        assert_eq!(fresh_pragma(pragma), before, "{setter} took effect");
    }

    assert_eq!(
        fresh_pragma("hard_heap_limit").as_deref(),
        Some("Integer(0)")
    );
    let report = crate::tools::report::report(
        &Parameters(crate::tools::report::ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );
    assert!(
        report.is_ok(),
        "report after the refusals: {:?}",
        report.err()
    );
}

/// The schema-introspection `pragma_*` table functions pass the authorizer's
/// allowlist and return the column rows; a setter-capable one is refused.
#[test]
fn pragma_table_info_returns_rows_while_other_pragma_functions_are_refused() {
    let project = project();

    let columns =
        ask(&project, "SELECT * FROM pragma_table_info('files')").expect("introspection reads");
    let knob = refused(
        &project,
        "SELECT * FROM pragma_user_version",
        "a non-introspection pragma function",
    );

    assert!(
        columns
            .rows
            .iter()
            .any(|row| row["name"] == "path" && row["cid"].is_i64()),
        "{:?}",
        columns.rows
    );
    assert_refused_as_not_a_select(&knob, "pragma_user_version");
    assert!(
        knob.message.contains("pragma_table_info"),
        "{}",
        knob.message
    );
}

/// `query` under `limits`, whose `max_file_bytes` and `max_query_value_bytes`
/// are both `file_bytes`, and the value-length bound that gives.
fn ask_bounded(
    project: &tempfile::TempDir,
    sql: &str,
    file_bytes: usize,
) -> (Result<QueryResponse, ErrorData>, i32) {
    let limits = ResourceLimits {
        max_file_bytes: file_bytes as u64,
        max_query_value_bytes: file_bytes,
        ..ResourceLimits::default()
    };
    let bound = crate::tools::query_exec::engine_limits(&limits)
        .expect("engine limits")
        .value_length;
    let request = Parameters(QueryRequest {
        path: project.path().to_string_lossy().into_owned(),
        sql: sql.to_owned(),
        offset: 0,
        expected_revision: None,
        page_bytes: None,
    });
    let manager = crate::sync::SourceSyncManager::default();
    let result = query_with_limits(
        &request,
        &limits,
        &crate::progress::NoopProgressSink,
        &manager,
    );
    (result.map(|Json(response)| response), bound)
}

/// The deep-dive amplifier peaked at 2.1 GB. Under a 3 MiB value-length bound
/// it, and each step of the same chain just above the bound, fails quickly
/// with SQLite's "too big" as `invalid_params`; the bound itself evaluates.
#[test]
fn the_zeroblob_hex_replace_amplifier_fails_fast_with_a_too_big_error() {
    let project = project();
    let bound = ask_bounded(&project, "SELECT 1 AS warm", 1024 * 1024).1;
    let started = std::time::Instant::now();

    for sql in [
        "SELECT length(replace(replace(hex(zeroblob(250000000)),'0','00'),'00','000')) AS n"
            .to_owned(),
        format!("SELECT length(zeroblob({})) AS n", bound + 1),
        format!("SELECT length(hex(zeroblob({}))) AS n", bound / 2 + 1),
    ] {
        let error = ask_bounded(&project, &sql, 1024 * 1024)
            .0
            .expect_err("over the bound");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{sql}");
        assert!(
            error.message.contains("too big"),
            "{sql}: {}",
            error.message
        );
    }
    let at_bound = ask_bounded(
        &project,
        &format!("SELECT length(zeroblob({bound})) AS n"),
        1024 * 1024,
    )
    .0;

    assert_eq!(
        at_bound.expect("exactly the bound").rows[0]["n"],
        i64::from(bound)
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// SQLite checks the value-length bound when it reads a stored column too.
/// A text file of invalid UTF-8 at `max_file_bytes` is stored lossy-decoded,
/// three bytes of U+FFFD per input byte, which is exactly the bound: it must
/// stay readable with `substr`, and one byte more must not.
#[test]
fn a_stored_value_of_exactly_the_length_bound_is_still_readable_with_substr() {
    let file_bytes = 64 * 1024;
    let project = activated_project("latin.txt", &vec![0xFF; file_bytes]);
    let over = 3 * file_bytes + 1;
    std::fs::write(project.path().join("over.txt"), "x".repeat(over)).expect("over fixture");
    let read = |path: &str| {
        let sql = format!(
            "SELECT substr(content, 1, 1) AS head, length(CAST(content AS BLOB)) AS n \
             FROM files WHERE path = '{path}'"
        );
        ask_bounded(&project, &sql, file_bytes)
    };

    let (at, bound) = read("latin.txt");
    let (beyond, _) = read("over.txt");

    assert_eq!(usize::try_from(bound).expect("bound"), 3 * file_bytes);
    let expected = serde_json::json!({"head": "\u{FFFD}", "n": 3 * file_bytes});
    assert_eq!(at.expect("the bound reads").rows, vec![expected]);
    let error = beyond.expect_err("one byte over the bound");
    assert!(error.message.contains("too big"), "{}", error.message);
}

/// The value bound is the larger of the query value cap and three bytes per
/// file byte, clamped to SQLite's maximum; the SQL bound is the tool's SQL
/// cap plus the wrapper overhead.
#[test]
fn engine_limits_cover_stored_values_and_the_wrapped_sql() {
    use crate::tools::query_exec::{EngineLimits, engine_limits};
    let with = |file: u64, value: usize, sql: usize| ResourceLimits {
        max_file_bytes: file,
        max_query_value_bytes: value,
        max_query_sql_bytes: sql,
        ..ResourceLimits::default()
    };

    for (limits, value_length, sql_length) in [
        (with(1000, 1000, 100), 3000, 164),
        (with(1000, 10, 100), 3000, 164),
        (with(1000, 5000, 100), 5000, 164),
        (
            with(1_000_000_000, 1, 2_000_000_000),
            1_000_000_000,
            1_000_000_000,
        ),
    ] {
        let expected = EngineLimits {
            value_length,
            sql_length,
        };
        assert_eq!(
            engine_limits(&limits).expect("computable"),
            expected,
            "{limits:?}"
        );
    }
}
