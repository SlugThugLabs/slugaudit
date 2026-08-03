use crate::{parse, project, store, sync};
use rmcp::ErrorData;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// If a project was successfully synced within this window, `ensure_synced`
/// skips the full publish (discovery + sample + diff) and returns the
/// current revision id directly. The caller's `with_verified_read` /
/// `with_verified_write` revision check catches any concurrent publish
/// that landed in the window, so correctness is preserved — this is purely
/// a performance optimization for rapid successive tool calls against a
/// quiescent project.
const RECENCY_WINDOW: Duration = Duration::from_secs(5);

/// Thread-safe cache of (project root → last successful sync time). One
/// instance lives on `SlugAuditServer` and is cloned into each tool call's
/// `spawn_blocking` closure. Lost on process restart (first call after
/// restart does a full sync — the current behaviour).
#[derive(Clone, Default)]
pub struct SyncRecencyCache {
    inner: Arc<Mutex<HashMap<PathBuf, Instant>>>,
}

impl SyncRecencyCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if `root` was synced within the recency window.
    fn is_recent(&self, root: &Path) -> bool {
        let guard = self.inner.lock().unwrap();
        guard
            .get(root)
            .is_some_and(|instant| instant.elapsed() < RECENCY_WINDOW)
    }

    /// Records that `root` was just synced successfully.
    fn record(&self, root: PathBuf) {
        let mut guard = self.inner.lock().unwrap();
        guard.insert(root, Instant::now());
    }
}

/// A project brought fully up to date, ready for a tool to query. Every
/// tool calls this first — there is no separate sync/rebuild entry point
/// for a human or an AI to remember to call.
pub struct SyncedProject {
    pub database_path: PathBuf,
    pub revision_id: String,
    #[allow(dead_code)]
    pub root: PathBuf,
}

/// Resolves the active project from `path`, publishes a fresh revision,
/// and returns where its database lives. If a sync is effectively already
/// current (nothing changed on disk), this still verifies that rather than
/// trusting a cached assumption.
///
/// When `cache` reports a successful sync for this project within the
/// recency window, the full publish is skipped and the current revision
/// id is read directly from the database instead. This avoids paying a
/// filesystem walk + re-hash on every tool call when the project is
/// quiescent. The caller's revision check in `with_verified_read` /
/// `with_verified_write` remains the correctness boundary.
///
/// # Errors
///
/// Returns an error if `path` isn't inside an active project, or if sync
/// itself fails.
pub fn ensure_synced(path: &str, cache: &SyncRecencyCache) -> Result<SyncedProject, ErrorData> {
    let root = project::find_project_root(Path::new(path))
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let database_path = project::database_path(&root);

    // Recency short-circuit: skip the full publish (discovery + sample +
    // diff) when this project was synced recently. A concurrent publish
    // that landed inside the window is caught by the revision check in
    // with_verified_read / with_verified_write, so this is safe.
    if cache.is_recent(root.as_path()) {
        let connection = store::open_read_only(&database_path)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let revision_id = current_revision_id(&connection)?;
        return Ok(SyncedProject {
            database_path,
            revision_id,
            root: root.as_path().to_path_buf(),
        });
    }

    let mut connection = store::open_read_write(&database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    ensure_project_row(&connection, root.as_path())?;
    let report = sync::publish(&mut connection, root.as_path(), parse::PACK_VERSION)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    drop(connection);

    cache.record(root.as_path().to_path_buf());

    Ok(SyncedProject {
        database_path,
        revision_id: report.revision_id,
        root: root.as_path().to_path_buf(),
    })
}

/// Runs `f` inside one deferred read transaction that first pins the
/// revision `synced` claimed. Verification and every subsequent read share
/// that snapshot, so a concurrent publish cannot change what the tool sees
/// mid-response — the call either observes the expected revision entirely
/// or fails the revision check before any tool data is read.
///
/// # Errors
///
/// Returns an error if the connection can't be opened, the revision no
/// longer matches, or `f` itself fails.
pub fn with_verified_read<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_read_only(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(result)
}

/// Write-side counterpart: one immediate transaction holds the revision
/// check, any lookups, validation, and the write together.
///
/// # Errors
///
/// Returns an error if the connection can't be opened, the revision no
/// longer matches, or `f` itself fails.
pub fn with_verified_write<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_read_write(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(result)
}

/// Creates the (sole) project row on first sync, and on every later sync
/// verifies the stored `root_path` still matches the canonical root this
/// process resolved. `INSERT OR IGNORE` alone would silently accept a
/// database file copied in from a different project (e.g. its
/// `.planning/slugaudit/project.db` copied over another project's) and
/// happily serve stale, unrelated data through it — fail closed instead.
///
/// # Errors
///
/// Returns an error if the insert/select fails, or if the database's
/// stored `root_path` doesn't match `root`.
fn ensure_project_row(connection: &Connection, root: &Path) -> Result<(), ErrorData> {
    let root_path = root.to_string_lossy();
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });
    connection
        .execute(
            "INSERT OR IGNORE INTO project (\
                id, project_id, root_path, contract_version, schema_version, created_at_unix\
             ) VALUES (1, 'default', ?1, 1, 1, ?2)",
            rusqlite::params![root_path.as_ref(), created_at],
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    let stored_root_path: String = connection
        .query_row("SELECT root_path FROM project WHERE id = 1", [], |row| {
            row.get(0)
        })
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    if stored_root_path != root_path {
        return Err(ErrorData::invalid_params(
            format!(
                "database at this location belongs to a different project root \
                 (expected {root_path}, found {stored_root_path})"
            ),
            None,
        ));
    }
    Ok(())
}

fn current_revision_id(connection: &Connection) -> Result<String, ErrorData> {
    connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
}

fn verify_revision_matches(
    connection: &Connection,
    expected_revision_id: &str,
) -> Result<(), ErrorData> {
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    if current != expected_revision_id {
        return Err(ErrorData::internal_error(
            format!(
                "revision changed concurrently (expected {expected_revision_id}, now {current}); retry the call"
            ),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "context_race_tests.rs"]
mod race_tests;
