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

/// What the sweep found, ready to feed the reconcile pipeline. `candidates`
/// are paths needing a re-hash check (new on disk, or stat mismatch against
/// the stored mtime/byte_len); `deleted` are database rows whose file is no
/// longer on disk — including files that became gitignored, which converge
/// out of the index exactly as a full publish would converge them.
pub(super) struct SweepReport {
    pub candidates: HashSet<String>,
    pub deleted: HashSet<String>,
    pub unchanged: usize,
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
    let mut statement =
        connection.prepare("SELECT path, modified_unix_seconds, byte_len FROM files")?;
    let stored: HashMap<String, (Option<i64>, i64)> = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, Option<i64>>(1)?, row.get::<_, i64>(2)?),
            ))
        })?
        .collect::<Result<_, _>>()?;
    drop(statement);

    // Every file the walk could stat successfully, by relative path. The
    // deleted set is then `stored.keys() - on_disk`: a database row whose
    // file vanished mid-walk, was deleted, or is no longer yielded by the
    // indexable walker (gitignored, excluded) all converge out uniformly.
    let mut on_disk: HashSet<String> = HashSet::new();
    let mut candidates: HashSet<String> = HashSet::new();
    let mut unchanged = 0_usize;

    for entry in indexable_walker(root) {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(SweepError::TimeBudgetExceeded {
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let Ok(entry) = entry else {
            continue; // per-entry walk errors: discovery skips these too
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let absolute_path = entry.path();
        let Ok(relative) = absolute_path.strip_prefix(root) else {
            continue;
        };
        if is_excluded_path(&relative.to_string_lossy()) {
            continue;
        }
        // Non-UTF-8 paths are never in the database (discovery rejects
        // them), so there is nothing to compare and nothing to delete.
        let Some(relative) = relative.to_str() else {
            continue;
        };

        // A stat that fails with NotFound means the file vanished between
        // the walk and the stat: leaving it out of `on_disk` classifies it
        // as deleted. Any other stat error means the file can't be verified
        // — skipping it matches discovery, which skips unreadable files and
        // would not index them on a full publish either.
        let Ok(metadata) = std::fs::metadata(absolute_path) else {
            continue;
        };
        on_disk.insert(relative.to_owned());

        let mtime = crate::util::mtime_unix_seconds(&metadata);
        let byte_len = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        match stored.get(relative) {
            Some((stored_mtime, stored_len))
                if *stored_mtime == mtime && *stored_len == byte_len =>
            {
                unchanged += 1;
            }
            _ => {
                candidates.insert(relative.to_owned());
            }
        }
    }

    let deleted: HashSet<String> = stored
        .keys()
        .filter(|path| !on_disk.contains(*path))
        .cloned()
        .collect();

    Ok(SweepReport {
        candidates,
        deleted,
        unchanged,
    })
}

#[cfg(test)]
#[path = "sweep_tests.rs"]
mod tests;
