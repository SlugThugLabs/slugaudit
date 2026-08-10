//! Reconciliation logic for SlugAudit's watch-based sync.
//!
//! When the filesystem watcher reports dirty or deleted paths, this module
//! reconciles them against the database: hashing dirty files, comparing
//! with stored hashes, and re-indexing only files that actually changed.
//! The barrier synchronization loop ensures no events are lost even if
//! new events arrive during reconciliation.

use super::discovery::{DiscoveredFile, FileKind};
use super::hash;
use super::revision;
use super::sample;
use crate::model::ResourceLimits;
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Hash(#[from] super::hash::HashError),
    #[error(transparent)]
    Sample(#[from] super::sample::SampleError),
    #[error(transparent)]
    Revision(#[from] super::revision::RevisionError),
}

/// Report of a reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    /// Files that were re-indexed because their hash differed from the
    /// stored hash or because they were new to the database.
    pub reconciled: usize,
    /// Files whose hash matched the stored hash and were skipped.
    pub unchanged: usize,
    /// Files that were removed from the database.
    pub deleted: usize,
}

/// Reconciles dirty and deleted paths against the database.
///
/// For each dirty path, reads the file, computes its BLAKE3 hash, and
/// queries the database for the previously stored hash. If the hash
/// matches, the file is skipped (unchanged since last index). If the hash
/// differs or the file is new, it is sampled, parsed, analyzed, and added
/// to the upsert set. Deleted paths are added to the deletion set.
///
/// After processing all paths, the function calls `publish_revision` to
/// atomically commit the upserts and deletions under compare-and-swap.
///
/// # Errors
///
/// Returns an error if any file read, hash, sample, analysis, or database
/// operation fails.
pub fn reconcile_dirty_paths(
    connection: &mut Connection,
    root: &Path,
    dirty: HashSet<String>,
    deleted: HashSet<String>,
    expected_revision: Option<&str>,
) -> Result<ReconcileReport, ReconcileError> {
    let limits = ResourceLimits::default();
    let mut upserts = Vec::new();
    let mut deletions = Vec::new();
    let mut reconciled = 0usize;
    let mut unchanged = 0usize;

    // Query existing hashes for dirty paths so we can skip files whose
    // content hasn't changed since they were last indexed.
    let existing_hashes = query_existing_hashes(connection, &dirty)?;

    for path in dirty {
        let absolute_path = root.join(&path);

        // If the file no longer exists on disk, treat it as a deletion.
        // This handles the race where a file was marked dirty and then
        // deleted before we could reconcile it.
        if !absolute_path.exists() {
            deletions.push(path);
            continue;
        }

        let identity = hash::hash_file(&path, &absolute_path)?;

        if let Some(existing_hash) = existing_hashes.get(&path)
            && existing_hash == &identity.content_hash {
            unchanged += 1;
            continue;
        }

        // Hash differs or file is new — sample, parse, analyze, and add
        // to the upsert set.
        let discovered = DiscoveredFile {
            relative_path: path.clone(),
            absolute_path: absolute_path.clone(),
            kind: FileKind::Indexed,
        };
        let sample = sample::sample_file(&discovered, &limits)?;
        let record = sample::to_file_record(sample, &limits);
        upserts.push(record);
        reconciled += 1;
    }

    for path in deleted {
        deletions.push(path);
    }

    // Nothing to commit — avoid churning out a no-op revision.
    if upserts.is_empty() && deletions.is_empty() {
        return Ok(ReconcileReport {
            reconciled,
            unchanged,
            deleted: 0,
        });
    }

    let manifest_hash = compute_manifest_hash(connection, &upserts, &deletions)?;
    let parser_pack_version = query_current_parser_pack_version(connection)?;

    let _revision_id = revision::publish_revision(
        connection,
        expected_revision,
        &manifest_hash,
        &parser_pack_version,
        &upserts,
        &deletions,
    )?;

    Ok(ReconcileReport {
        reconciled,
        unchanged,
        deleted: deletions.len(),
    })
}

/// Implements the barrier synchronization loop.
///
/// Takes a barrier (current watcher sequence), reconciles all dirty/deleted
/// paths up to that barrier, and then checks if new events arrived during
/// reconciliation. If so, loops and reconciles the new events. Continues
/// until the watcher sequence stabilizes.
pub fn sync_with_barrier(
    state: &crate::watch::WatchState,
    reconcile_fn: impl FnMut(HashSet<String>, HashSet<String>, u64) -> Result<(), ReconcileError>,
) -> Result<(), ReconcileError> {
    let mut reconcile_fn = reconcile_fn;
    loop {
        let _barrier = state.current_sequence();
        let (seq, dirty, deleted) = state.take_dirty();

        if dirty.is_empty() && deleted.is_empty() {
            break Ok(());
        }

        reconcile_fn(dirty, deleted, seq)?;

        // Check if more events arrived during reconciliation.
        if state.current_sequence() == seq {
            break Ok(());
        }
        // Otherwise, loop and reconcile the new events.
    }
}

/// Queries the stored content hashes for the given paths.
fn query_existing_hashes(
    connection: &Connection,
    paths: &HashSet<String>,
) -> Result<HashMap<String, String>, rusqlite::Error> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; paths.len()].join(", ");
    let sql = format!("SELECT path, content_hash FROM files WHERE path IN ({placeholders})");
    let params: Vec<&dyn rusqlite::ToSql> =
        paths.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
    let mut stmt = connection.prepare(&sql)?;
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut result = HashMap::new();
    for row in rows {
        let (path, hash) = row?;
        result.insert(path, hash);
    }
    Ok(result)
}

/// Computes the aggregate manifest hash from the post-reconciliation file
/// set: existing DB state, updated with upserts, minus deletions.
///
/// Avoids reading rows that will be replaced or removed: upserted paths are
/// overwritten in-memory, and deleted paths are excluded at the SQL level so
/// we don't pay to read rows we'll immediately discard.
fn compute_manifest_hash(
    connection: &Connection,
    upserts: &[revision::FileRecord],
    deletions: &[String],
) -> Result<String, rusqlite::Error> {
    let current_refs: BTreeMap<String, String> = {
        let exclude_count = upserts.len() + deletions.len();
        if exclude_count == 0 {
            let rows: Vec<(String, String)> = connection
                .prepare("SELECT path, content_hash FROM files")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter().collect()
        } else {
            let placeholders = vec!["?"; exclude_count].join(", ");
            let sql = format!(
                "SELECT path, content_hash FROM files WHERE path NOT IN ({placeholders})"
            );
            let params: Vec<&dyn rusqlite::ToSql> = upserts
                .iter()
                .map(|r| &r.relative_path as &dyn rusqlite::ToSql)
                .chain(deletions.iter().map(|p| p as &dyn rusqlite::ToSql))
                .collect();
            let rows: Vec<(String, String)> = connection
                .prepare(&sql)?
                .query_map(params.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter().collect()
        }
    };

    let mut current_refs = current_refs;
    for record in upserts {
        current_refs.insert(
            record.relative_path.clone(),
            record.identity.content_hash.clone(),
        );
    }

    Ok(hash::aggregate_manifest_hash(
        current_refs.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    ))
}

/// Reads the parser pack version of the current revision, falling back to
/// `"1.0"` when no revision has been published yet.
fn query_current_parser_pack_version(connection: &Connection) -> Result<String, rusqlite::Error> {
    let version: Option<String> = connection
        .query_row(
            "SELECT parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(version.unwrap_or_else(|| "1.0".to_owned()))
}

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;
