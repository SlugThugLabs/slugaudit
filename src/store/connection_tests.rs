// Tests for `src/store/connection.rs`, in a sibling file so the
// production source stays under the 300-line hard cap while the test suite
// has room to grow. The `evidence.kind` CHECK tests live here too:
// this is the store's only test surface that opens a migrated connection,
// and the constraint is only observable through one.

use crate::evidence::to_row;
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, SpanAvailability};
use crate::store::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn foreign_keys_are_enabled_on_a_read_write_connection() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;
    let enabled: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .check("read pragma")?;
    ensure_eq(&enabled, &1, "enabled == 1")?;
    Ok(())
}

#[test]
fn a_read_only_connection_cannot_write() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    open_read_write(&path).check("create database")?;

    let read_only = open_existing(&path).check("open read-only")?;
    let result = read_only.execute("DELETE FROM files", []);
    let err = result.check_err("read-only connection must reject writes")?;
    ensure(
        err.to_string().to_lowercase().contains("readonly"),
        format!("expected readonly error, got: {err}"),
    )?;
    Ok(())
}

#[test]
fn read_only_open_fails_against_a_missing_database() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let result = open_existing(&directory.path().join("missing.db"));
    ensure(
        matches!(result, Err(StoreError::Open(_))),
        "matches!(result, Err(StoreError::Open(_)))",
    )?;
    Ok(())
}

/// A corrupted/non-SQLite file at the database path must fail closed
/// with a typed error, not panic and not silently treat garbage bytes
/// as an empty database (SQLite validates lazily on first real access,
/// not at `open_with_flags` itself, so this only surfaces once
/// `configure` issues its first pragma).
#[test]
fn a_corrupted_database_file_fails_closed_with_a_typed_error() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    std::fs::write(&path, b"not a sqlite database, just garbage bytes").check("write garbage")?;

    let result = open_read_write(&path);
    ensure(
        matches!(result, Err(StoreError::Configure(_))),
        format!("expected a typed Configure error, got {result:?}"),
    )?;
    Ok(())
}

/// Every variant of `model::EvidenceKind`, derived rather than restated:
/// `EvidenceKind` carries a test-build-only `JsonSchema` derive, so the
/// compiler regenerates this list from the enum definition itself. The schema
/// lists one entry per variant in either shape schemars emits: a flat `enum`
/// of names when no variant is documented, or, once variants carry doc
/// comments, `oneOf` entries naming the variant in `const` (or a
/// single-element `enum`) beside an ignored `description`. A variant added to
/// the enum appears here with no hand-kept copy to forget — and the tests
/// below then ask `evidence::to_row` for that variant's text and compare it
/// against the CHECK SQLite actually installed, which is what forces
/// `schema.sql` to gain the text too.
fn every_evidence_kind() -> TestResult<Vec<EvidenceKind>> {
    let schema = serde_json::to_value(schemars::schema_for!(EvidenceKind)).check("schema JSON")?;
    schema
        .get("oneOf")
        .or_else(|| schema.get("enum"))
        .and_then(serde_json::Value::as_array)
        .check("the EvidenceKind schema lists its variants as oneOf entries or a flat enum")?
        .iter()
        .map(|v| {
            let enum_name = v.get("enum").and_then(|n| n.get(0));
            match v.get("const").or(enum_name) {
                Some(name) => name,
                None => v,
            }
        })
        .map(|name| serde_json::from_value(name.clone()).check("each name is an EvidenceKind"))
        .collect()
}

/// The `CHECK (kind IN (...))` allow-list, read back out of the schema's own
/// DDL so the comparison is against the constraint SQLite actually installed
/// rather than a restatement of what it should be.
fn kind_check_allowed_values(connection: &rusqlite::Connection) -> TestResult<Vec<String>> {
    let ddl: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'evidence'",
            [],
            |row| row.get(0),
        )
        .check("read the evidence table DDL")?;
    let list = ddl
        .split_once("kind IN (")
        .check("evidence.kind must carry a CHECK (kind IN (...))")?
        .1
        .split_once(')')
        .check("the kind CHECK's closing paren")?
        .0;

    Ok(list
        .split(',')
        .map(|value| value.trim().trim_matches('\'').to_owned())
        .collect())
}

/// The parent rows an `evidence` insert needs: `revisions` backs
/// `files.last_revision_id`, and every parser column on `files` has a CHECK
/// constraint, so the fixture supplies modeled values.
fn insert_a_file_row(connection: &rusqlite::Connection) -> TestResult<i64> {
    connection
        .execute(
            "INSERT INTO revisions \
                (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('rev-1', 'manifest-hash', 'parser-pack', 0, 1)",
            [],
        )
        .check("insert the parent revision")?;
    connection
        .execute(
            "INSERT INTO files \
                (path, content_hash, hash_algorithm, byte_len, parser_availability, \
                 parse_outcome, extraction_completeness, last_revision_id) \
             VALUES ('src/main.rs', 'content-hash', 'sha256', 0, 'Available', 'Succeeded', \
                     'Full', (SELECT id FROM revisions WHERE revision_id = 'rev-1'))",
            [],
        )
        .check("insert the parent file")?;

    Ok(connection.last_insert_rowid())
}

/// An evidence kind this build does not model must be refused by SQLite
/// itself. Without the CHECK the row is stored, and a later reader sees a
/// kind no producer claims — an unmodeled verdict dressed as evidence.
#[test]
fn evidence_kind_check_rejects_an_unmodeled_kind() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;
    let file_id = insert_a_file_row(&connection)?;

    let result = connection.execute(
        "INSERT INTO evidence (file_id, key, kind, origin, span_availability, payload) \
         VALUES (?1, 'unmodeled:0', 'NotAnEvidenceKind', 'PackSymbol', 'PackOmitted', '{}')",
        [file_id],
    );

    let err =
        result.check_err("an unmodeled evidence kind must be rejected by the CHECK constraint")?;
    ensure(
        err.to_string().to_lowercase().contains("constraint"),
        format!("expected constraint error, got: {err}"),
    )?;
    Ok(())
}

/// `files.byte_len` is nullable: NULL is an unknown size (a skipped file
/// whose stat failed), and a real empty file's 0 must stay distinguishable
/// from it.
#[test]
fn a_null_byte_len_is_storable_and_distinct_from_an_empty_file() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;
    let file_id = insert_a_file_row(&connection)?;
    let size = |connection: &rusqlite::Connection| -> TestResult<Option<i64>> {
        connection
            .query_row(
                "SELECT byte_len FROM files WHERE id = ?1",
                [file_id],
                |row| row.get(0),
            )
            .check("read the size")
    };
    ensure_eq(&size(&connection)?, &Some(0), "an empty file stores 0")?;

    connection
        .execute(
            "UPDATE files SET byte_len = NULL, file_kind = 'skipped', \
             skip_reason = 'stat_failed' WHERE id = ?1",
            [file_id],
        )
        .check("a skipped row may carry an unknown size")?;

    ensure_eq(&size(&connection)?, &None, "an unknown size stays NULL")?;
    Ok(())
}

/// Every kind `kind_text` can produce must be accepted, and the CHECK must
/// list those strings and no others — the SQL mirror has to be exact in both
/// directions. The insert set is driven through `evidence::to_row`, the one
/// production enum→text mapping, so this test cannot carry a stale copy of
/// the strings.
#[test]
fn every_evidence_kind_text_passes_the_kind_check() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;
    let file_id = insert_a_file_row(&connection)?;

    let mut inserted_kinds = Vec::new();
    for (index, kind) in every_evidence_kind()?.into_iter().enumerate() {
        let row = to_row(&EvidenceItem {
            key: format!("kind:{index}"),
            kind,
            origin: EvidenceOrigin::PackSymbol,
            span: SpanAvailability::PackOmitted,
            payload: serde_json::json!({}),
        });
        let inserted = connection.execute(
            "INSERT INTO evidence (file_id, key, kind, origin, span_availability, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                file_id,
                row.key,
                row.kind,
                row.origin,
                row.span_availability,
                row.payload
            ],
        );

        inserted.as_ref().check("`inserted` is ok")?;
        inserted_kinds.push(row.kind.to_owned());
    }

    let mut allowed = kind_check_allowed_values(&connection)?;
    inserted_kinds.sort();
    allowed.sort();
    ensure_eq(
        &allowed,
        &inserted_kinds,
        "left is the evidence.kind allow-list SQLite installed from src/store/schema.sql, \
         right is the text src/evidence/sql.rs::kind_text produces: the CHECK must list \
         exactly these strings",
    )?;
    Ok(())
}

#[test]
fn the_line_functions_run_on_both_connection_modes() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let project_path = directory.path().join("project.db");
    open_read_write(&project_path).check("create project database")?;

    for (label, connection) in [
        (
            "read-only",
            open_existing(&project_path).check("open read-only")?,
        ),
        (
            "read-write",
            open_read_write(&project_path).check("open read-write")?,
        ),
    ] {
        let lines: (i64, String) = connection
            .query_row(
                "SELECT line_count('a\nb\nc'), line_range('a\nb\nc', 2, 3)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .check(&format!("{label}: the functions must be registered"))?;
        ensure_eq(&lines, &(3, "b\nc".to_owned()), label.to_string())?;
    }
    Ok(())
}

#[test]
fn a_connection_that_cannot_enter_wal_mode_fails_configuration() -> TestResult {
    let connection = rusqlite::Connection::open_in_memory().check("in-memory db")?;

    let result = super::configure(&connection);

    ensure(
        matches!(&result, Err(StoreError::WalUnavailable { granted }) if granted == "memory"),
        "matches!(&result, Err(StoreError::WalUnavailable { granted }) if granted == \"memory\")",
    )?;
    Ok(())
}
