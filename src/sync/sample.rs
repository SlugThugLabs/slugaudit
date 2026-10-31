// slugaudit-line-exception: approved-by=slugthug; reason=one per-file sampling pipeline: the bounded read, the skip and UTF-8 classification, and folding that sample's analysis into its FileRecord
use super::analyze::analyze;
use super::discovery::{DiscoveredFile, FileKind};
use super::hash;
use super::revision::FileRecord;
use super::revision_skipped::{SKIP_REASON_STAT_FAILED, SkippedRecord};
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, ExtractionCompleteness, ResourceLimits,
    SourceIdentity, SpanAvailability,
};
use crate::util::{self, Deadline};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

// The evidence-capping loop lives beside the evidence writes in
// `revision_files`; re-exported here so `to_file_record` and its tests keep
// this module's path.
pub(crate) use super::revision_files::apply_evidence_limits;

// The parallel batch loop lives in `sample_batch` — it outgrew this
// file's source-size budget once it gained per-file skip handling.
// Re-exported here so existing importers
// (`crate::sync::sample::sample_all_with_deadline`) keep resolving
// through this module's path.
pub(super) use super::sample_batch::sample_all_with_deadline;

#[derive(Debug, Error)]
/// Per-file sampling failures. `TooLarge` skips that file with a reported
/// reason; read/hash failures abort the publish.
pub enum SampleError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `observed` is the exact stat size when the stat itself exceeded the
    /// cap, or the bounded read length (`limit + 1`) when the file grew in
    /// the stat/read window. It is a lower bound on the file's size in the
    /// growth case and never an exact size there, so the message says
    /// "at least" rather than claiming a size the read did not establish.
    #[error("{path} exceeds the {limit}-byte per-file import ceiling (at least {observed} bytes)")]
    TooLarge {
        path: PathBuf,
        observed: u64,
        limit: u64,
    },
    /// The file's bytes are not valid UTF-8, so its exact content cannot be
    /// stored; it becomes a skipped existence row instead of a lossy copy.
    #[error("{path} is not valid UTF-8; recorded as a skipped file")]
    NotUtf8 {
        /// Absolute path of the non-UTF-8 file.
        path: PathBuf,
    },
}

/// One discovered file's on-disk state: content, hash, and whatever it
/// takes to classify it (binary vs. text; text that is not valid UTF-8 is an error,
/// not a converted copy).
#[derive(Debug)]
pub(super) struct Sample {
    pub relative_path: String,
    pub is_binary: bool,
    pub content: Option<String>,
    pub identity: SourceIdentity,
    pub byte_len: u64,
    /// File mtime in unix seconds, captured from the same stat that
    /// enforced the size ceiling. Persisted for reporting only; changes are
    /// detected by content hash, never by mtime.
    pub modified_unix_seconds: Option<i64>,
}

/// # Errors
///
/// Returns an error if `file.absolute_path` can't be read, or if its size
/// exceeds `limits.max_file_bytes`.
pub(super) fn sample_file(
    file: &DiscoveredFile,
    limits: &ResourceLimits,
) -> Result<Sample, SampleError> {
    let metadata = std::fs::metadata(&file.absolute_path).map_err(|source| SampleError::Read {
        path: file.absolute_path.clone(),
        source,
    })?;
    let size = metadata.len();
    if size > limits.max_file_bytes {
        return Err(SampleError::TooLarge {
            path: file.absolute_path.clone(),
            observed: size,
            limit: limits.max_file_bytes,
        });
    }

    // The helper stats first (fast rejection) and bounds the read with
    // `take(max + 1)`, so a file that grows between stat and read cannot
    // allocate an unbounded buffer. Its observed length is reported, never
    // truncated.
    let bytes = util::read_file_capped_bytes(&file.absolute_path, limits.max_file_bytes).map_err(
        |error| match error {
            util::CappedReadError::Io(source) => SampleError::Read {
                path: file.absolute_path.clone(),
                source,
            },
            util::CappedReadError::TooLarge { limit, observed } => SampleError::TooLarge {
                path: file.absolute_path.clone(),
                observed,
                limit,
            },
        },
    )?;

    let is_binary = file.kind == FileKind::Binary;
    let content = if is_binary {
        None
    } else {
        match std::str::from_utf8(&bytes) {
            Ok(text) => Some(text.to_owned()),
            // Stored content must be byte-exact: a lossy re-encoding would
            // no longer hash to the file's identity, so the file is skipped
            // with a reason instead of stored wrong.
            Err(_) => {
                return Err(SampleError::NotUtf8 {
                    path: file.absolute_path.clone(),
                });
            }
        }
    };
    Ok(Sample {
        relative_path: file.relative_path.clone(),
        is_binary,
        byte_len: bytes.len() as u64,
        identity: hash::hash_bytes(&file.relative_path, &bytes),
        content,
        modified_unix_seconds: util::mtime_unix_seconds(&metadata),
    })
}

/// The existence row for a file discovery or sampling deliberately did not
/// index. Never reads the file: the identity is stat-derived
/// (`hash::stat_identity`) precisely so that skipping keeps its no-read
/// cost. A failed stat (the file vanished, was never readable, or is a
/// broken symlink) records an unknown size (`None`, stored as NULL, never a
/// fake 0), no mtime, and the distinct [`SKIP_REASON_STAT_FAILED`] reason
/// instead of `reason`, rather than failing the publish; the failure is
/// logged once here.
#[must_use]
pub(super) fn skipped_record(
    relative_path: &str,
    absolute_path: &Path,
    reason: &str,
) -> SkippedRecord {
    let (byte_len, modified_unix_seconds, reason) = match std::fs::metadata(absolute_path) {
        Ok(metadata) => (
            Some(metadata.len()),
            util::mtime_unix_seconds(&metadata),
            reason,
        ),
        Err(error) => {
            tracing::warn!(
                path = %absolute_path.display(),
                requested_reason = reason,
                error = %error,
                "stat failed for a skipped file; recording its size as unknown"
            );
            (None, None, SKIP_REASON_STAT_FAILED)
        }
    };
    SkippedRecord {
        relative_path: relative_path.to_owned(),
        reason: reason.to_owned(),
        identity: hash::stat_identity(relative_path, byte_len, modified_unix_seconds),
        byte_len,
        modified_unix_seconds,
    }
}

/// Runs pack analysis on a sampled file and folds the result into a
/// storable `FileRecord`, capping evidence to `limits.evidence`. The file's
/// parses run within what is left of the sync `deadline`.
pub(super) fn to_file_record(
    sample: Sample,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> FileRecord {
    let content = sample.content.as_deref();
    let mut parsed = analyze(
        &sample.relative_path,
        content,
        limits.max_parse_bytes,
        Duration::from_millis(limits.max_parse_wall_clock_ms),
        deadline,
    );
    // `analyze` decides completeness from the pack's own diagnostics, before
    // the evidence caps run. A run it called `Full` whose items are then
    // discarded by the caps must not be stored as `Full` beside the "records
    // were dropped" marker the caps append — that is the same class of
    // overclaim `ContentOnly` exists to remove. `ContentOnly`/`Unavailable`
    // are left alone: they already describe a file whose structural evidence
    // was never available.
    let discarded = apply_evidence_limits(&mut parsed.evidence, limits);
    if discarded && parsed.run.completeness == ExtractionCompleteness::Full {
        parsed.run.completeness = ExtractionCompleteness::Partial;
    }
    FileRecord {
        relative_path: sample.relative_path,
        is_binary: sample.is_binary,
        content: sample.content,
        identity: sample.identity,
        byte_len: sample.byte_len,
        modified_unix_seconds: sample.modified_unix_seconds,
        language: parsed.language,
        language_detected: parsed.language_detected,
        run: parsed.run,
        evidence: parsed.evidence,
    }
}

pub(super) fn truncation_evidence(
    original_count: usize,
    kept_count: usize,
    oversized_dropped: usize,
    unmeasurable_dropped: usize,
) -> EvidenceItem {
    let dropped_for_cap = original_count
        .saturating_sub(kept_count)
        .saturating_sub(oversized_dropped)
        .saturating_sub(unmeasurable_dropped);
    let unmeasurable_note = if unmeasurable_dropped > 0 {
        format!(", {unmeasurable_dropped} item(s) failed to serialize")
    } else {
        String::new()
    };
    EvidenceItem {
        key: "evidence:truncated".to_owned(),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::SourceContent,
        span: SpanAvailability::DerivedEvidence,
        payload: json!({
            "message": format!(
                "evidence limits discarded records: {oversized_dropped} item(s) exceeded the \
                 per-item byte cap, {dropped_for_cap} item(s) dropped by the item-count or \
                 cumulative-byte cap{unmeasurable_note}"
            ),
            "severity": "warning",
            "original_item_count": original_count,
            "kept_item_count": kept_count,
        }),
    }
}

#[cfg(test)]
#[path = "sample_tests.rs"]
mod tests;
