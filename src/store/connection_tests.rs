// Tests for `src/store/connection.rs`. Extracted to a sibling file so the
// production source stays under the 300-line hard cap while the test suite
// still has room to grow.
// slugaudit-line-exception: approved-by=agent; reason=audit-trail tests covering each rejection guard, paired with their happy-path negative; keeping them co-located lets future regressions cross-compare symlink / permissions / network paths at a glance

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
    assert!(result.is_err());
}

#[test]
fn read_only_open_fails_against_a_missing_database() {
    let directory = tempfile::tempdir().expect("temp dir");
    let result = open_read_only(&directory.path().join("missing.db"));
    assert!(matches!(result, Err(StoreError::Open(_))));
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
    assert!(!path.symlink_metadata().unwrap().file_type().is_symlink());

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
/// command unavailable, mountinfo unreadable, unsupported platform), the
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
    let (result, logs) = capture_warns(|| open_read_write(&path));
    assert!(result.is_ok(), "sanity: the open itself must succeed");
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
