// Tests for `src/store/connection.rs`. Extracted to a sibling file so the
// production source stays under the 300-line hard cap while the test suite
// still has room to grow. The `evidence.kind` CHECK tests live here too:
// this is the store's only test surface that opens a migrated connection,
// and the constraint is only observable through one.

use crate::evidence::to_row;
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, SpanAvailability};
use crate::store::test_capture::capture_warns;
use crate::store::*;

#[test]
fn foreign_keys_are_enabled_on_a_read_write_connection() {
    let directory = tempfile::tempdir().expect("temp dir");
    let connection = open_read_write(&directory.path().join("project.db")).expect("open");
    let enabled: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read pragma");
    assert_eq!(enabled, 1);
}

#[test]
fn a_read_only_connection_cannot_write() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    open_read_write(&path).expect("create database");

    let read_only = open_read_only(&path).expect("open read-only");
    let result = read_only.execute("DELETE FROM findings", []);
    let err = result.expect_err("read-only connection must reject writes");
    assert!(
        err.to_string().to_lowercase().contains("readonly"),
        "expected readonly error, got: {err}"
    );
}

#[test]
fn read_only_open_fails_against_a_missing_database() {
    let directory = tempfile::tempdir().expect("temp dir");
    let result = open_read_only(&directory.path().join("missing.db"));
    assert!(matches!(result, Err(StoreError::Open(_))));
}

/// The documented safety boundary for the `query` tool is "correctness
/// comes from the connection itself" (ARCHITECTURE.md invariant #1) —
/// and `SQLITE_OPEN_READ_ONLY` alone does NOT provide it: a read-only
/// flag blocks *writes* but not *other reads*, because
/// `ATTACH DATABASE '<file>'` opens a second database inside the same
/// connection. The `guard_against_attach` authorizer is what makes the
/// "project's own database" claim true at the connection layer,
/// independent of the query tool's subquery wrapping (which currently
/// hides the gap at the tool layer). This test exercises the connection
/// directly — no wrapper — so it pins the real invariant, not the
/// accident.
#[test]
fn neither_connection_can_attach_a_foreign_database() {
    let directory = tempfile::tempdir().expect("temp dir");
    let project_path = directory.path().join("project.db");
    let foreign_path = directory.path().join("foreign.db");
    open_read_write(&project_path).expect("create project database");

    // A foreign SQLite file with data the query tool must never reach.
    let foreign = rusqlite::Connection::open(&foreign_path).expect("create foreign db");
    foreign
        .execute_batch(
            "CREATE TABLE secrets (v TEXT); \
             INSERT INTO secrets VALUES ('top-secret-value');",
        )
        .expect("seed foreign db");
    drop(foreign);

    for (label, connection) in [
        (
            "read-only",
            open_read_only(&project_path).expect("open read-only"),
        ),
        (
            "read-write",
            open_read_write(&project_path).expect("open read-write"),
        ),
    ] {
        let attach = connection.execute_batch(&format!(
            "ATTACH DATABASE '{}' AS other",
            foreign_path.display()
        ));
        assert!(
            attach.is_err(),
            "{label} connection must deny ATTACH of a foreign database file"
        );

        // The attached alias must never become usable.
        let attached_read: rusqlite::Result<String> =
            connection.query_row("SELECT v FROM other.secrets", [], |row| row.get(0));
        assert!(
            attached_read.is_err(),
            "{label} connection must not reach data through an attached alias"
        );

        // Positive control: normal reads on the guarded connection are
        // unaffected — the guard must not over-block.
        let count: i64 = connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
            .expect("{label} connection still serves ordinary reads");
        assert!(count > 0, "{label} connection should see its own schema");
    }
}

/// A corrupted/non-SQLite file at the database path must fail closed
/// with a typed error, not panic and not silently treat garbage bytes
/// as an empty database (SQLite validates lazily on first real access,
/// not at `open_with_flags` itself, so this only surfaces once
/// `configure` issues its first pragma).
#[test]
fn a_corrupted_database_file_fails_closed_with_a_typed_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    std::fs::write(&path, b"not a sqlite database, just garbage bytes").expect("write garbage");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("protect garbage fixture");
    }

    let result = open_read_write(&path);
    assert!(
        matches!(result, Err(StoreError::Configure(_))),
        "expected a typed Configure error, got {result:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_db_path_is_rejected_for_both_read_write_and_read_only() {
    let directory = tempfile::tempdir().expect("temp dir");
    let real_target = directory.path().join("elsewhere.db");
    let link_path = directory.path().join("project.db");
    std::os::unix::fs::symlink(&real_target, &link_path).expect("create symlink");

    assert!(matches!(
        open_read_write(&link_path),
        Err(StoreError::Symlink)
    ));
    assert!(matches!(
        open_read_only(&link_path),
        Err(StoreError::Symlink)
    ));
    assert!(
        !real_target.exists(),
        "the symlink target must never be created/opened"
    );
}

#[cfg(unix)]
#[test]
fn a_newly_created_database_gets_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    open_read_write(&path).expect("create database");
    let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    // Literal `0o600` (owner-only) instead of importing `PRIVATE_MODE` —
    // the constant stays module-private, and `0o600` is the canonical
    // mode for owner-only file permissions.
    assert_eq!(mode, 0o600);
}

/// The `-wal` and `-shm` sidecars sit beside the database and can hold the same
/// source-derived content, so they must be as private as the database itself.
/// SQLite derives their mode from the database file's, which this crate pins to
/// `0600`; this test fails if that derivation ever stops holding — the sidecars
/// are created by SQLite, not by `create_with_private_permissions_if_missing`.
#[cfg(unix)]
#[test]
fn the_wal_and_shm_sidecars_are_owner_only_like_the_database() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    let connection = open_read_write(&path).expect("create database");
    connection
        .execute_batch("CREATE TABLE probe (v INTEGER); INSERT INTO probe VALUES (1);")
        .expect("a write materialises the write-ahead log");

    for name in ["project.db-wal", "project.db-shm"] {
        let sidecar = directory.path().join(name);
        let mode = std::fs::metadata(&sidecar)
            .unwrap_or_else(|error| panic!("{name} must exist while the log is live: {error}"))
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{name} must be owner-only, got {mode:o}");
    }
}

/// The file is created with `O_CREAT | O_EXCL` and the private mode in
/// one syscall (see `create_with_private_permissions_if_missing`), so
/// there is no separate "create, then chmod" step for a single-threaded
/// test to catch mid-window. What we can prove here: losing the create
/// race to another process (simulated by pre-creating the file with a
/// wider mode, as a concurrent creator might under a permissive umask)
/// doesn't panic or get chmod'd out from under its actual owner; the
/// security boundary rejects the wider mode without modifying it.
#[cfg(unix)]
#[test]
fn a_database_created_with_wider_permissions_is_rejected_without_touching_permissions() {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");

    // Simulate another process winning the create race with a wider,
    // umask-derived mode.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&path)
        .expect("simulate a concurrent creator");
    // `OpenOptionsExt::mode` is filtered by the process umask (the managed
    // runner uses 0077), so explicitly widen the fixture after creation.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("widen simulated concurrent creator");

    assert!(matches!(
        open_read_write(&path),
        Err(StoreError::Permissions(_))
    ));

    let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o644,
        "must never chmod a file this call didn't create"
    );
}

/// Distinct from `a_symlinked_db_path_is_rejected_for_both_read_write_and_read_only`,
/// where the path is a symlink from the very first open: this proves the
/// TOCTOU case, where a path that was legitimately a real file at one
/// point in time gets replaced by a symlink later (e.g. a compromised or
/// misbehaving process racing the legitimate owner). Both `open_flags`'
/// `SQLITE_OPEN_NOFOLLOW` and the `reject_symlink` pre-check must catch
/// this on the *next* open, not just the first.
#[cfg(unix)]
#[test]
fn a_db_path_replaced_by_a_symlink_after_a_successful_open_is_rejected_on_the_next_open() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");

    // A real, legitimate first open succeeds and migrates the schema.
    let connection = open_read_write(&path).expect("first open on a real file succeeds");
    drop(connection);
    assert!(
        !path
            .symlink_metadata()
            .expect("symlink metadata readable")
            .file_type()
            .is_symlink()
    );

    // The path is now replaced by a symlink pointing elsewhere, as a
    // TOCTOU attacker (or a racing process) might.
    let elsewhere = directory.path().join("elsewhere.db");
    std::fs::remove_file(&path).expect("remove the real file");
    std::os::unix::fs::symlink(&elsewhere, &path).expect("replace it with a symlink");

    assert!(matches!(open_read_write(&path), Err(StoreError::Symlink)));
    assert!(matches!(open_read_only(&path), Err(StoreError::Symlink)));
    assert!(
        !elsewhere.exists(),
        "the symlink target must never be created/opened through the replaced path"
    );
}

/// Verifies that rejecting a symlink emits a `tracing::warn!` event
/// tagged with the path and `reason="symlink"`. The reject path
/// emits a typed `StoreError::Symlink`, but the typed error's
/// `Display` doesn't include the path — only the warn site captures
/// it. Without this test, a future refactor could silently drop the
/// warn site and an operator tracing an open failure would see
/// "refusing to open a database path that is a symlink" without
/// knowing *which* path.
#[cfg(unix)]
#[test]
fn a_symlinked_db_open_emits_a_warning_for_auditing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let real_target = directory.path().join("elsewhere.db");
    let link_path = directory.path().join("project.db");
    std::os::unix::fs::symlink(&real_target, &link_path).expect("create symlink");

    let (result, logs) = capture_warns(|| open_read_write(&link_path));
    assert!(matches!(result, Err(StoreError::Symlink)));
    assert!(
        logs.contains("symlink"),
        "the warn event must be tagged with reason=symlink, captured logs: {logs}"
    );
    assert!(
        logs.contains(link_path.display().to_string().as_str()),
        "the warn event must include the symlinked path so an operator can \
         locate which file triggered the rejection, captured logs: {logs}"
    );
}

/// Verified rejected with broader-than-owner permissions emit a
/// `tracing::warn!` event tagged with the path, the observed mode,
/// and `reason="insecure_permissions"`. The mode field is optional
/// (operators sometimes just want the `path`), but having it makes
/// "what was the wider mode?" a 30-second investigation rather than
/// a 30-minute one.
#[cfg(unix)]
#[test]
fn an_insecure_permissions_rejection_emits_a_warning_for_auditing() {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");

    // Pre-create the file with mode 0o644 to trip the rejection,
    // exactly how `a_database_created_with_wider_permissions_is_rejected_without_touching_permissions`
    // simulates a concurrent creator. The shared setup ensures the
    // warning test exercises the same code path the security test
    // proves correct.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&path)
        .expect("create with wide mode");
    // `OpenOptionsExt::mode` is filtered by the process umask; make the
    // fixture's intended mode explicit so this test is portable across CI.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("widen permissions fixture");

    let (result, logs) = capture_warns(|| open_read_write(&path));
    assert!(matches!(result, Err(StoreError::Permissions(_))));
    assert!(
        logs.contains("insecure_permissions"),
        "the warn event must be tagged with reason=insecure_permissions, captured logs: {logs}"
    );
    assert!(
        logs.contains(path.display().to_string().as_str()),
        "the warn event must include the path, captured logs: {logs}"
    );
    assert!(
        logs.contains("644"),
        "the warn event must include the actual mode so an operator doesn't \
         have to `stat` the file separately to see what mode tripped the \
         rejection, captured logs: {logs}"
    );
}

/// C12: when the network-filesystem check itself fails (inspection
/// directory unreadable, fstatfs failure, unsupported platform), the
/// error must surface the underlying cause AND a non-admin fallback
/// message — an operator must be able to tell "the check failed" from
/// "this is on NFS" and must not read it as a permissions problem.
#[test]
fn network_filesystem_check_error_surfaces_the_underlying_command_error() {
    let error = StoreError::NetworkFilesystemCheck(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "stat: no such file or directory",
    ));
    let message = error.to_string();
    assert!(
        message.contains("stat: no such file or directory"),
        "the underlying inspection error must be visible: {message}"
    );
    assert!(
        message.contains("not a permissions problem"),
        "the message must say this is not an admin-fixable-permissions issue: {message}"
    );
    assert!(
        !error.is_corruption(),
        "a check failure is not database corruption and must not trigger a discard"
    );
}

/// A successful local-filesystem open must NOT emit any of the
/// rejection warnings. Paired regression test for the warning tests:
/// if any of the rejection paths were silently tripping on the
/// happy path, this would catch it.
#[cfg(unix)]
#[test]
fn a_local_filesystem_open_does_not_emit_any_rejection_warnings() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    let (result, logs) = capture_warns(|| {
        // Liveness control: proves the absence checks below cannot pass
        // because the capture went blind.
        tracing::warn!("capture liveness control");
        open_read_write(&path)
    });
    assert!(result.is_ok(), "sanity: the open itself must succeed");
    assert!(
        logs.contains("capture liveness control"),
        "the capture must see this thread's events, or the absence checks \
         below prove nothing, captured logs: {logs}"
    );
    assert!(
        !logs.contains("\"symlink\""),
        "a successful local open must not log the symlink-rejection warning, captured logs: {logs}"
    );
    assert!(
        !logs.contains("\"insecure_permissions\""),
        "a successful local open must not log the insecure-permissions warning, captured logs: {logs}"
    );
    assert!(
        !logs.contains("\"network_filesystem\""),
        "a successful local open must not log the network-filesystem warning, captured logs: {logs}"
    );
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
fn every_evidence_kind() -> Vec<EvidenceKind> {
    let schema = serde_json::to_value(schemars::schema_for!(EvidenceKind)).expect("schema JSON");
    schema
        .get("oneOf")
        .or_else(|| schema.get("enum"))
        .and_then(serde_json::Value::as_array)
        .expect("the EvidenceKind schema lists its variants as oneOf entries or a flat enum")
        .iter()
        .map(|v| {
            let enum_name = v.get("enum").and_then(|n| n.get(0));
            v.get("const").or(enum_name).unwrap_or(v)
        })
        .map(|name| serde_json::from_value(name.clone()).expect("each name is an EvidenceKind"))
        .collect()
}

/// The `CHECK (kind IN (...))` allow-list, read back out of the schema's own
/// DDL so the comparison is against the constraint SQLite actually installed
/// rather than a restatement of what it should be.
fn kind_check_allowed_values(connection: &rusqlite::Connection) -> Vec<String> {
    let ddl: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'evidence'",
            [],
            |row| row.get(0),
        )
        .expect("read the evidence table DDL");
    let list = ddl
        .split_once("kind IN (")
        .expect("evidence.kind must carry a CHECK (kind IN (...))")
        .1
        .split_once(')')
        .expect("the kind CHECK's closing paren")
        .0;

    list.split(',')
        .map(|value| value.trim().trim_matches('\'').to_owned())
        .collect()
}

/// The parent rows an `evidence` insert needs: `revisions` backs
/// `files.last_revision_id`, and every parser column on `files` is CHECKed,
/// so the fixture supplies modeled values.
fn insert_a_file_row(connection: &rusqlite::Connection) -> i64 {
    connection
        .execute(
            "INSERT INTO revisions \
                (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('rev-1', 'manifest-hash', 'parser-pack', 0, 1)",
            [],
        )
        .expect("insert the parent revision");
    connection
        .execute(
            "INSERT INTO files \
                (path, content_hash, hash_algorithm, byte_len, parser_availability, \
                 parse_outcome, extraction_completeness, last_revision_id) \
             VALUES ('src/main.rs', 'content-hash', 'sha256', 0, 'Available', 'Succeeded', \
                     'Full', (SELECT id FROM revisions WHERE revision_id = 'rev-1'))",
            [],
        )
        .expect("insert the parent file");

    connection.last_insert_rowid()
}

/// An evidence kind this build does not model must be refused by SQLite
/// itself. Without the CHECK the row is stored, and a later reader sees a
/// kind no producer claims — an unmodeled verdict dressed as evidence.
#[test]
fn evidence_kind_check_rejects_an_unmodeled_kind() {
    let directory = tempfile::tempdir().expect("temp dir");
    let connection = open_read_write(&directory.path().join("project.db")).expect("open");
    let file_id = insert_a_file_row(&connection);

    let result = connection.execute(
        "INSERT INTO evidence (file_id, key, kind, origin, span_availability, payload) \
         VALUES (?1, 'unmodeled:0', 'NotAnEvidenceKind', 'PackSymbol', 'PackOmitted', '{}')",
        [file_id],
    );

    let err =
        result.expect_err("an unmodeled evidence kind must be rejected by the CHECK constraint");
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "expected constraint error, got: {err}"
    );
}

/// `files.byte_len` is nullable: NULL is an unknown size (a skipped file
/// whose stat failed), and a real empty file's 0 must stay distinguishable
/// from it.
#[test]
fn a_null_byte_len_is_storable_and_distinct_from_an_empty_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let connection = open_read_write(&directory.path().join("project.db")).expect("open");
    let file_id = insert_a_file_row(&connection);
    let size = |connection: &rusqlite::Connection| -> Option<i64> {
        connection
            .query_row(
                "SELECT byte_len FROM files WHERE id = ?1",
                [file_id],
                |row| row.get(0),
            )
            .expect("read the size")
    };
    assert_eq!(size(&connection), Some(0), "an empty file stores 0");

    connection
        .execute(
            "UPDATE files SET byte_len = NULL, file_kind = 'skipped', \
             skip_reason = 'stat_failed' WHERE id = ?1",
            [file_id],
        )
        .expect("a skipped row may carry an unknown size");

    assert_eq!(size(&connection), None, "an unknown size stays NULL");
}

/// Every kind `kind_text` can produce must be accepted, and the CHECK must
/// list those strings and no others — the SQL mirror has to be exact in both
/// directions. The insert set is driven through `evidence::to_row`, the one
/// production enum→text mapping, so this test cannot carry a stale copy of
/// the strings.
#[test]
fn every_evidence_kind_text_passes_the_kind_check() {
    let directory = tempfile::tempdir().expect("temp dir");
    let connection = open_read_write(&directory.path().join("project.db")).expect("open");
    let file_id = insert_a_file_row(&connection);

    let mut inserted_kinds = Vec::new();
    for (index, kind) in every_evidence_kind().into_iter().enumerate() {
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

        assert!(
            inserted.is_ok(),
            "kind {} must be modeled by the CHECK: {inserted:?}",
            row.kind
        );
        inserted_kinds.push(row.kind.to_owned());
    }

    let mut allowed = kind_check_allowed_values(&connection);
    inserted_kinds.sort();
    allowed.sort();
    assert_eq!(
        allowed, inserted_kinds,
        "left is the evidence.kind allow-list SQLite installed from src/store/schema.sql, \
         right is the text src/evidence/sql.rs::kind_text produces: the CHECK must list \
         exactly these strings"
    );
}

/// A symlinked *ancestor* directory is not a reason to refuse the open: on
/// macOS every temp and `/var` path runs through `/var -> /private/var`.
/// Only the database file itself being a symlink is rejected.
#[cfg(unix)]
#[test]
fn a_database_under_a_symlinked_parent_directory_opens_for_both_modes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let real = directory.path().join("real");
    std::fs::create_dir(&real).expect("real dir");
    let linked = directory.path().join("linked");
    std::os::unix::fs::symlink(&real, &linked).expect("dir symlink");
    let path = linked.join("project.db");

    drop(open_read_write(&path).expect("read-write open through a symlinked parent"));
    let read_only = open_read_only(&path);

    assert!(
        read_only.is_ok(),
        "read-only open through a symlinked parent: {read_only:?}"
    );
    assert!(
        real.join("project.db").is_file(),
        "the database lands in the real directory"
    );
}

#[test]
fn the_line_functions_run_on_both_guarded_connections_while_the_guard_still_denies_attach() {
    let directory = tempfile::tempdir().expect("temp dir");
    let project_path = directory.path().join("project.db");
    open_read_write(&project_path).expect("create project database");

    for (label, connection) in [
        (
            "read-only",
            open_read_only(&project_path).expect("open read-only"),
        ),
        (
            "read-write",
            open_read_write(&project_path).expect("open read-write"),
        ),
    ] {
        let lines: (i64, String) = connection
            .query_row(
                "SELECT line_count('a\nb\nc'), line_range('a\nb\nc', 2, 3)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or_else(|error| panic!("{label}: the functions must pass the guard: {error}"));
        assert_eq!(lines, (3, "b\nc".to_owned()), "{label}");

        assert!(
            connection
                .execute_batch("ATTACH DATABASE ':memory:' AS other")
                .is_err(),
            "{label} must still deny ATTACH"
        );
        assert!(
            connection.execute_batch("DETACH DATABASE main").is_err(),
            "{label} must still deny DETACH"
        );
        assert!(
            connection
                .query_row("SELECT load_extension('libnothing')", [], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .is_err(),
            "{label} must not load extensions"
        );
    }
}

#[test]
fn a_connection_that_cannot_enter_wal_mode_fails_configuration() {
    let connection = rusqlite::Connection::open_in_memory().expect("in-memory db");

    let result = super::configure(&connection);

    assert!(matches!(&result, Err(StoreError::WalUnavailable { granted }) if granted == "memory"));
}

#[test]
fn an_uninspectable_database_path_fails_instead_of_skipping_the_symlink_guard() {
    let path = std::path::Path::new("/tmp")
        .join("a".repeat(300))
        .join("project.db");

    assert!(matches!(
        super::reject_symlink(&path),
        Err(StoreError::Inspect(_))
    ));
}

/// R6: the remedy names what the code actually does — an `fstatfs` call on
/// the database directory — not commands or mount tables it never reads.
#[test]
fn the_network_filesystem_check_error_names_the_real_inspection_path() {
    let message = StoreError::NetworkFilesystemCheck(std::io::Error::other("x")).to_string();
    assert!(message.contains("fstatfs"), "{message}");
    assert!(!message.contains("fsutil"), "{message}");
}
