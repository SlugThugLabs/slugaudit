use super::analyze::analyze;
use super::discovery::{self, DiscoveredFile, FileKind};
use super::hash::{self, aggregate_manifest_hash};
use super::manifest::{self, ChangeStatus};
use super::revision::{self, FileRecord};
use crate::model::SourceIdentity;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PublishError {
    #[error(transparent)]
    Discovery(#[from] discovery::DiscoveryError),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Revision(#[from] revision::RevisionError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub revision_id: String,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub unchanged: usize,
}

struct Sample {
    relative_path: String,
    is_binary: bool,
    content: Option<String>,
    identity: SourceIdentity,
    byte_len: u64,
}

fn sample_file(file: &DiscoveredFile) -> Result<Sample, PublishError> {
    let bytes = std::fs::read(&file.absolute_path).map_err(|source| PublishError::Read {
        path: file.absolute_path.clone(),
        source,
    })?;
    let is_binary = file.kind == FileKind::Binary;
    let content = (!is_binary).then(|| String::from_utf8_lossy(&bytes).into_owned());
    Ok(Sample {
        relative_path: file.relative_path.clone(),
        is_binary,
        byte_len: bytes.len() as u64,
        identity: hash::hash_bytes(&file.relative_path, &bytes),
        content,
    })
}

fn to_file_record(sample: Sample) -> FileRecord {
    let parsed = analyze(&sample.relative_path, sample.content.as_deref());
    FileRecord {
        relative_path: sample.relative_path,
        is_binary: sample.is_binary,
        content: sample.content,
        identity: sample.identity,
        byte_len: sample.byte_len,
        language: parsed.language,
        language_detected: parsed.language_detected,
        parser_availability: parsed.parser_availability,
        parse_outcome: parsed.parse_outcome,
        parse_error_reason: parsed.parse_error_reason,
        extraction_completeness: parsed.extraction_completeness,
        evidence: parsed.evidence,
    }
}

struct CurrentRevision {
    revision_id: String,
    parser_pack_version: String,
}

fn current_revision(connection: &Connection) -> Result<Option<CurrentRevision>, PublishError> {
    connection
        .query_row(
            "SELECT revision_id, parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| {
                Ok(CurrentRevision {
                    revision_id: row.get(0)?,
                    parser_pack_version: row.get(1)?,
                })
            },
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(PublishError::Database(other)),
        })
}

/// Discovers, hashes, and diffs the project's current disk state against
/// what the database has stored, then publishes exactly the delta as one
/// atomic revision. A disk state identical to the current revision skips
/// publishing entirely rather than churning out a no-op revision.
///
/// # Errors
///
/// Returns an error if discovery, reading a discovered file, or any
/// database operation in the publish transaction fails.
pub fn publish(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
) -> Result<PublishReport, PublishError> {
    let discovered = discovery::discover(root)?;
    let samples = discovered
        .iter()
        .map(sample_file)
        .collect::<Result<Vec<_>, _>>()?;

    let mut stored_statement = connection.prepare("SELECT path, content_hash FROM files")?;
    let stored_rows = stored_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stored_statement);

    let stored_refs: Vec<(&str, &str)> = stored_rows
        .iter()
        .map(|(path, hash)| (path.as_str(), hash.as_str()))
        .collect();
    let current_refs: Vec<(&str, &str)> = samples
        .iter()
        .map(|sample| {
            (
                sample.relative_path.as_str(),
                sample.identity.content_hash.as_str(),
            )
        })
        .collect();
    let changes = manifest::compare(stored_refs, current_refs.clone());
    let manifest_hash = aggregate_manifest_hash(current_refs);

    let added = changes
        .iter()
        .filter(|c| c.status == ChangeStatus::Added)
        .count();
    let modified = changes
        .iter()
        .filter(|c| c.status == ChangeStatus::Modified)
        .count();
    let deleted = changes
        .iter()
        .filter(|c| c.status == ChangeStatus::Deleted)
        .count();
    let unchanged = discovered.len() - (added + modified);

    let current = current_revision(connection)?;
    // A file's content not changing doesn't mean its evidence is still
    // valid — if the parser itself was upgraded, everything needs
    // re-analysis even though every hash still matches.
    let parser_version_changed = current
        .as_ref()
        .is_none_or(|revision| revision.parser_pack_version != parser_pack_version);

    if changes.is_empty()
        && !parser_version_changed
        && let Some(current) = current
    {
        return Ok(PublishReport {
            revision_id: current.revision_id,
            added: 0,
            modified: 0,
            deleted: 0,
            unchanged,
        });
    }

    let changed_paths: HashSet<String> = if parser_version_changed {
        samples
            .iter()
            .map(|sample| sample.relative_path.clone())
            .collect()
    } else {
        changes
            .iter()
            .filter(|change| change.status != ChangeStatus::Deleted)
            .map(|change| change.relative_path.clone())
            .collect()
    };
    let deletions: Vec<String> = changes
        .iter()
        .filter(|change| change.status == ChangeStatus::Deleted)
        .map(|change| change.relative_path.clone())
        .collect();
    let upserts: Vec<FileRecord> = samples
        .into_iter()
        .filter(|sample| changed_paths.contains(sample.relative_path.as_str()))
        .map(to_file_record)
        .collect();

    let revision_id = revision::publish_revision(
        connection,
        &manifest_hash,
        parser_pack_version,
        &upserts,
        &deletions,
    )?;

    Ok(PublishReport {
        revision_id,
        added,
        modified,
        deleted,
        unchanged,
    })
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
