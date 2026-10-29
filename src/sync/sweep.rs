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

use crate::ignore_rules::indexable_walker;
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
    /// The walker could not list a directory (or the root), or could not
    /// apply an ignore file. Paths under the unverified subtree are unknown,
    /// so `stored - on_disk` would publish still-existing rows as deleted;
    /// the sweep fails closed instead.
    #[error("stat sweep could not verify the tree ({kind}); no rows were deleted")]
    IncompleteWalk {
        /// Static `ignore::Error` category, never its path-bearing Display.
        kind: &'static str,
    },
}

impl SweepError {
    /// The elapsed budget when this is a budget exhaustion, else `None`.
    #[must_use]
    pub(crate) fn budget_elapsed_ms(&self) -> Option<u128> {
        match self {
            SweepError::TimeBudgetExceeded { elapsed_ms } => Some(*elapsed_ms),
            SweepError::Database(_) | SweepError::IncompleteWalk { .. } => None,
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
/// [`SweepError::TimeBudgetExceeded`] when `deadline` is spent mid-walk, or
/// [`SweepError::IncompleteWalk`] when the walker could not list a directory
/// or apply an ignore file — failing closed, because a partial sweep that
/// silently skipped part of the tree would both serve staleness and publish
/// still-existing rows as deleted.
pub(super) fn sweep(
    connection: &Connection,
    root: &Path,
    deadline: &Deadline,
) -> Result<SweepReport, SweepError> {
    note_walk(root);
    let stored = stored_signatures(connection)?;
    let mut findings = SweepFindings::default();

    for entry in indexable_walker(root) {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(SweepError::TimeBudgetExceeded {
                elapsed_ms: elapsed.as_millis(),
            });
        }
        classify_entry(root, entry, &stored, &mut findings)?;
    }

    // `stored.keys() - on_disk`: a database row whose file vanished
    // mid-walk, was deleted, or is no longer yielded by the indexable
    // walker (gitignored, excluded) all converge out uniformly.
    let deleted: HashSet<String> = stored
        .keys()
        .filter(|path| !findings.on_disk.contains(*path))
        .cloned()
        .collect();

    Ok(SweepReport {
        candidates: findings.candidates,
        deleted,
        on_disk: findings.on_disk,
        unchanged: findings.unchanged,
        unrepresentable: findings.unrepresentable,
    })
}

/// What one walk has found so far. Kept out of [`sweep`] so the walk loop
/// and the per-entry classification each read as one stage.
#[derive(Default)]
struct SweepFindings {
    /// Every file the walk could stat successfully, by relative path.
    on_disk: HashSet<String>,
    candidates: HashSet<String>,
    unrepresentable: HashSet<Vec<u8>>,
    unchanged: usize,
}

/// Classifies one walked entry against the stored stat signatures and folds
/// it into `findings`. A walker or ignore-file failure aborts the whole
/// sweep: an unreadable subtree makes every path under it unknown, so no
/// deletion may be published from this walk.
fn classify_entry(
    root: &Path,
    entry: Result<ignore::DirEntry, ignore::Error>,
    stored: &HashMap<String, StoredSignature>,
    findings: &mut SweepFindings,
) -> Result<(), SweepError> {
    let (relative, metadata) = match walked_file(root, entry, &mut findings.unrepresentable) {
        Ok(Walked::Stat(relative, metadata)) => (relative, metadata),
        Ok(Walked::Unverifiable(relative)) => {
            // The file exists but could not be stat'd: keep its stored
            // row (on disk, neither a candidate nor proven unchanged)
            // rather than classing it deleted.
            if stored.contains_key(&relative) {
                findings.on_disk.insert(relative);
            }
            return Ok(());
        }
        Ok(Walked::Skipped) => return Ok(()),
        Err(kind) => return Err(SweepError::IncompleteWalk { kind }),
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
        findings.unchanged += 1;
    } else {
        findings.candidates.insert(relative.clone());
    }
    findings.on_disk.insert(relative);
    Ok(())
}

/// Stored `(mtime, byte_len)`; `byte_len` is `None` for an unknown size.
type StoredSignature = (Option<i64>, Option<i64>);

/// `(mtime, byte_len)` per stored path — the two signals a stat can
/// disprove without reading the file. A NULL `byte_len` (unknown size, a
/// `stat_failed` skipped row) never equals a real size, so that path is
/// always a candidate for re-examination. A `LoadFailed` row (its grammar
/// could not be fetched or loaded) reports a NULL size for the same reason
/// while a retry is due (see `fetch_gate::load_failed_retry_due`): its
/// content is unchanged, but the parse must be retried. During the fetch
/// cool-down it is compared like any unchanged file.
fn stored_signatures(
    connection: &Connection,
) -> Result<HashMap<String, StoredSignature>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT path, modified_unix_seconds, \
         CASE WHEN ?1 AND parser_availability = 'LoadFailed' THEN NULL ELSE byte_len END FROM files",
    )?;
    let stored = statement
        .query_map([crate::parse::fetch_gate::load_failed_retry_due()], |row| {
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
    /// Nothing to compare: non-files, excluded or non-UTF-8 paths, and
    /// files that vanished between walk and stat. Walk-level failures are
    /// returned as errors instead, so a skipped entry can never authorize a
    /// deletion under a subtree the walk could not read.
    Skipped,
}

/// One walked entry, reduced to what the comparison needs. The error is the
/// static `ignore::Error` category for a directory or ignore-file failure;
/// its path-bearing `Display` is never returned or logged.
fn walked_file(
    root: &Path,
    entry: Result<ignore::DirEntry, ignore::Error>,
    unrepresentable: &mut HashSet<Vec<u8>>,
) -> Result<Walked, &'static str> {
    let entry = match entry {
        Ok(entry) => entry,
        Err(error) => return Err(super::sweep_reconcile::log_walk_failure(&error)),
    };
    if let Some(error) = entry.error() {
        return Err(super::sweep_reconcile::log_walk_failure(error));
    }
    if !entry.file_type().is_some_and(|kind| kind.is_file()) {
        return Ok(Walked::Skipped);
    }
    let absolute_path = entry.path();
    let Some(relative) =
        super::sweep_reconcile::walked_relative_path(root, absolute_path, unrepresentable)
    else {
        return Ok(Walked::Skipped);
    };

    // Only a stat that fails with NotFound means the file vanished between
    // the walk and the stat; leaving it out of `on_disk` classifies it as
    // deleted. Any other stat error (permissions, I/O) leaves the file
    // unverifiable, not gone: its stored row is kept and the cause is logged.
    match std::fs::metadata(absolute_path) {
        Ok(metadata) => Ok(Walked::Stat(relative, metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                path_id = %crate::util::redacted_path(absolute_path),
                "sweep walked a file that vanished before it could be stat'd",
            );
            Ok(Walked::Skipped)
        }
        Err(error) => {
            tracing::warn!(
                path = %absolute_path.display(),
                error = %error,
                "sweep could not stat a file it walked; its stored row is kept unverified",
            );
            Ok(Walked::Unverifiable(relative))
        }
    }
}

#[cfg(test)]
mod walks {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    static WALK_COUNTERS: Mutex<Option<HashMap<PathBuf, Arc<AtomicUsize>>>> = Mutex::new(None);

    pub(super) fn count_walks(root: &Path) -> Arc<AtomicUsize> {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut guard = crate::util::lock_or_recover(&WALK_COUNTERS, "walk_counters");
        guard
            .get_or_insert_with(HashMap::new)
            .insert(crate::util::canonical_or_self(root), Arc::clone(&counter));
        counter
    }

    pub(super) fn note_walk(root: &Path) {
        let guard = crate::util::lock_or_recover(&WALK_COUNTERS, "walk_counters");
        if let Some(counter) = guard
            .as_ref()
            .and_then(|counters| counters.get(&crate::util::canonical_or_self(root)))
        {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }
}
#[cfg(test)]
use walks::note_walk;
/// Release builds count nothing.
#[cfg(not(test))]
fn note_walk(_root: &Path) {}

#[cfg(test)]
#[path = "sweep_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sweep_walk_tests.rs"]
mod walk_tests;
