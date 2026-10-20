//! The stat sweep: the freshness backstop that makes evidence independent
//! of the filesystem watcher.
//!
//! The watcher is a latency optimization; it must never be the only thing
//! standing between a tool call and stale evidence. Events can be dropped
//! (`WatchManager::handle_event` skips an event when a tool thread holds
//! the manager lock), a watcher can silently die, and a project's ignore
//! rules can change without any event surviving to mark the tree dirty.
//! The sweep closes all three holes with one cheap pass:
//!
//! 1. Walk the project with the same walker discovery uses.
//! 2. Stat every file — one `stat`, no read.
//! 3. A file whose stored `modified_unix_seconds` + `byte_len` both match
//!    the stat is proven unchanged. Everything else (stat mismatch, new on
//!    disk, present in the database but gone from disk) becomes a reconcile
//!    candidate or a deletion.
//!
//! Candidates are only *nominees*: the reconcile pipeline re-hashes each
//! one and skips files whose content hash still matches, so a false
//! positive (same-second edits, mtime-preserving writes with identical
//! size) costs one hash, never a parse. The residual blind spot — a
//! same-size in-place edit that lands in the same wall-clock second as the
//! stored stat *and* produces no watcher event — is documented in
//! ARCHITECTURE.md as accepted, and is covered whenever either layer fires.

use super::discovery::unrepresentable_relative_bytes;
use crate::ignore_rules::{indexable_walker, is_excluded_path};
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum SweepError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("stat sweep exceeded its wall-clock time budget after {elapsed_ms} ms")]
    TimeBudgetExceeded { elapsed_ms: u128 },
}

impl SweepError {
    /// The elapsed budget when this is a budget exhaustion, else `None`.
    #[must_use]
    pub(crate) fn budget_elapsed_ms(&self) -> Option<u128> {
        match self {
            SweepError::TimeBudgetExceeded { elapsed_ms } => Some(*elapsed_ms),
            SweepError::Database(_) => None,
        }
    }
}

/// What the sweep found, ready to feed the reconcile pipeline. `candidates`
/// are paths needing a re-hash check (new on disk, or stat mismatch against
/// the stored mtime/byte_len); `deleted` are database rows whose file is no
/// longer on disk — including files that became gitignored, which converge
/// out of the index exactly as a full publish would converge them.
/// `on_disk` is every path the walk stat'd successfully: the set that
/// exists, including files sampling skips — edge resolution resolves
/// imports against it so an import of an oversize-skipped file is not
/// misreported as `Unresolved`.
pub(super) struct SweepReport {
    pub candidates: HashSet<String>,
    pub deleted: HashSet<String>,
    pub on_disk: HashSet<String>,
    pub unchanged: usize,
    /// Raw project-relative bytes of the walked files whose path is not
    /// valid UTF-8. They have no `files` row, so this is the exact set the
    /// persisted unrepresentable-path table must hold after this walk.
    pub unrepresentable: HashSet<Vec<u8>>,
}

/// Walks `root`, stats every indexable file, and classifies it against the
/// database's stored stat state. Read-only: publishing what it finds is the
/// caller's job (`reconcile_dirty_paths_with_deadline`), under the caller's
/// compare-and-swap revision check.
///
/// # Errors
///
/// Returns [`SweepError::Database`] if the stored stat state can't be read,
/// or [`SweepError::TimeBudgetExceeded`] when `deadline` is spent mid-walk —
/// failing closed, because a partial sweep that silently skipped the rest of
/// the tree would serve exactly the staleness the sweep exists to prevent.
pub(super) fn sweep(
    connection: &Connection,
    root: &Path,
    deadline: &Deadline,
) -> Result<SweepReport, SweepError> {
    note_walk(root);
    let stored = stored_signatures(connection)?;

    // Every file the walk could stat successfully, by relative path. The
    // deleted set is then `stored.keys() - on_disk`: a database row whose
    // file vanished mid-walk, was deleted, or is no longer yielded by the
    // indexable walker (gitignored, excluded) all converge out uniformly.
    let mut on_disk: HashSet<String> = HashSet::new();
    let mut candidates: HashSet<String> = HashSet::new();
    let mut unrepresentable: HashSet<Vec<u8>> = HashSet::new();
    let mut unchanged = 0_usize;

    for entry in indexable_walker(root) {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(SweepError::TimeBudgetExceeded {
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let (relative, metadata) = match walked_file(root, entry, &mut unrepresentable) {
            Walked::Stat(relative, metadata) => (relative, metadata),
            Walked::Unverifiable(relative) => {
                // The file exists but could not be stat'd: keep its stored
                // row (on disk, neither a candidate nor proven unchanged)
                // rather than classing it deleted.
                if stored.contains_key(&relative) {
                    on_disk.insert(relative);
                }
                continue;
            }
            Walked::Skipped => continue,
        };
        let mtime = crate::util::mtime_unix_seconds(&metadata);
        // A size beyond `i64::MAX` cannot equal any stored size, so it is
        // never proven unchanged; it stays a candidate and reconcile skips it.
        let unchanged_match = i64::try_from(metadata.len()).is_ok_and(|byte_len| {
            matches!(
                stored.get(&relative),
                Some((stored_mtime, Some(stored_len))) if *stored_mtime == mtime && *stored_len == byte_len
            )
        });
        if unchanged_match {
            unchanged += 1;
        } else {
            candidates.insert(relative.clone());
        }
        on_disk.insert(relative);
    }

    let deleted: HashSet<String> = stored
        .keys()
        .filter(|path| !on_disk.contains(*path))
        .cloned()
        .collect();

    Ok(SweepReport {
        candidates,
        deleted,
        on_disk,
        unchanged,
        unrepresentable,
    })
}

/// Stored `(mtime, byte_len)`; `byte_len` is `None` for an unknown size.
type StoredSignature = (Option<i64>, Option<i64>);

/// `(mtime, byte_len)` per stored path — the two signals a stat can
/// disprove without reading the file. A NULL `byte_len` (unknown size, a
/// `stat_failed` skipped row) never equals a real size, so that path is
/// always a candidate for re-examination.
fn stored_signatures(
    connection: &Connection,
) -> Result<HashMap<String, StoredSignature>, rusqlite::Error> {
    let mut statement =
        connection.prepare("SELECT path, modified_unix_seconds, byte_len FROM files")?;
    let stored = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, Option<i64>>(1)?, row.get::<_, Option<i64>>(2)?),
            ))
        })?
        .collect::<Result<_, _>>()?;
    drop(statement);
    Ok(stored)
}

/// What the sweep makes of one walked entry.
enum Walked {
    /// A file with a successful stat, ready for comparison.
    Stat(String, std::fs::Metadata),
    /// A file whose stat failed for a reason other than it being gone
    /// (permissions, I/O error): it exists, so a stored row must stay.
    Unverifiable(String),
    /// Nothing to compare: unreadable directories, non-files, excluded or
    /// non-UTF-8 paths, and files that vanished between walk and stat. The
    /// cases that are otherwise invisible warn.
    Skipped,
}

/// One walked entry, reduced to what the comparison needs.
fn walked_file(
    root: &Path,
    entry: Result<ignore::DirEntry, ignore::Error>,
    unrepresentable: &mut HashSet<Vec<u8>>,
) -> Walked {
    let entry = match entry {
        Ok(entry) => entry,
        Err(error) => {
            // Discovery turns these into skipped rows; the sweep has no
            // row to write, so the log is the only place the gap shows.
            tracing::warn!(
                error = %error,
                "sweep could not read a directory entry; its subtree is not part of \
                 this comparison and files under it cannot be verified",
            );
            return Walked::Skipped;
        }
    };
    if !entry.file_type().is_some_and(|kind| kind.is_file()) {
        return Walked::Skipped;
    }
    let absolute_path = entry.path();
    let relative = match absolute_path.strip_prefix(root) {
        Ok(relative) => relative,
        Err(error) => {
            // The walker yields paths under `root`, so this cannot happen;
            // if it did, the entry is omitted and its stored row (if any)
            // would be classed deleted, so say so with the cause.
            tracing::warn!(
                path_id = %crate::util::redacted_path(absolute_path),
                error = %error,
                "sweep walked a path outside the project root; it is left out of the comparison",
            );
            return Walked::Skipped;
        }
    };
    if is_excluded_path(&relative.to_string_lossy()) {
        return Walked::Skipped;
    }
    // Non-UTF-8 paths are never in the database (discovery rejects
    // them), so there is nothing to compare and nothing to delete — but
    // they must still reach the persisted set `report` counts, or the
    // count would go stale between full publishes.
    let Some(relative) = relative.to_str().map(str::to_owned) else {
        unrepresentable.extend(unrepresentable_relative_bytes(root, absolute_path));
        return Walked::Skipped;
    };

    // Only a stat that fails with NotFound means the file vanished between
    // the walk and the stat; leaving it out of `on_disk` classifies it as
    // deleted. Any other stat error (permissions, I/O) leaves the file
    // unverifiable, not gone: its stored row is kept and the cause is logged.
    match std::fs::metadata(absolute_path) {
        Ok(metadata) => Walked::Stat(relative, metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                path_id = %crate::util::redacted_path(absolute_path),
                "sweep walked a file that vanished before it could be stat'd",
            );
            Walked::Skipped
        }
        Err(error) => {
            tracing::warn!(
                path_id = %crate::util::redacted_path(absolute_path),
                error = %error,
                "sweep could not stat a file it walked; its stored row is kept unverified",
            );
            Walked::Unverifiable(relative)
        }
    }
}

/// Test-only per-root walk counters, so the Healthy-path regression can
/// prove one whole-tree walk per call. Keyed by canonical root (the same
/// per-root keying `race_hook` uses) so tests running in parallel never
/// share a counter.
#[cfg(test)]
static WALK_COUNTERS: std::sync::Mutex<
    Option<
        std::collections::HashMap<
            std::path::PathBuf,
            std::sync::Arc<std::sync::atomic::AtomicUsize>,
        >,
    >,
> = std::sync::Mutex::new(None);

/// Arms walk counting for `root` and returns the counter [`sweep`] bumps on
/// every walk of that root.
#[cfg(test)]
pub(super) fn count_walks(root: &Path) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut guard = crate::util::lock_or_recover(&WALK_COUNTERS);
    guard
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            crate::util::canonical_or_self(root),
            std::sync::Arc::clone(&counter),
        );
    counter
}

/// Bumps the armed counter for `root`, if any; a no-op for any root nobody
/// armed a counter for. Counting exists only in test builds.
#[cfg(test)]
fn note_walk(root: &Path) {
    let guard = crate::util::lock_or_recover(&WALK_COUNTERS);
    // Canonical like the watcher's keys, so a counter armed with a tempdir
    // path matches the walk's resolved root.
    if let Some(counter) = guard
        .as_ref()
        .and_then(|counters| counters.get(&crate::util::canonical_or_self(root)))
    {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Release builds count nothing.
#[cfg(not(test))]
fn note_walk(_root: &Path) {}

#[cfg(test)]
#[path = "sweep_tests.rs"]
mod tests;
