// slugaudit-line-exception: approved-by=slugthug; reason=single-file read/hash/classify, evidence caps, and to_file_record are one cohesive per-file pipeline; the parallel batch loop lives in sample_batch.rs next to the loop that feeds it
use super::analyze::analyze;
use super::discovery::{DiscoveredFile, FileKind};
use super::hash;
use super::revision::FileRecord;
use super::revision_skipped::{SKIP_REASON_STAT_FAILED, SkippedRecord};
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, ExtractionCompleteness, ResourceLimits,
    SourceIdentity, SpanAvailability,
};
use crate::util;
use serde_json::json;
use std::path::{Path, PathBuf};
use thiserror::Error;

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
    #[error("{path} is {size} bytes, exceeding the {limit}-byte per-file import ceiling")]
    TooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
}

/// One discovered file's on-disk state: content, hash, and whatever it
/// takes to classify it (binary vs. text, and whether reading it as text
/// required lossy UTF-8 conversion).
#[derive(Debug)]
pub(super) struct Sample {
    pub relative_path: String,
    pub is_binary: bool,
    pub content: Option<String>,
    /// True when `content` required lossy UTF-8 conversion (invalid byte
    /// sequences replaced with U+FFFD). Recorded as evidence rather than
    /// silently accepted — see `to_file_record`.
    utf8_lossy: bool,
    pub identity: SourceIdentity,
    pub byte_len: u64,
    /// File mtime in unix seconds, captured from the same stat that
    /// enforced the size ceiling. Persisted so the sync sweep can detect
    /// changes with one stat instead of a full read+hash.
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
            size,
            limit: limits.max_file_bytes,
        });
    }

    let bytes = std::fs::read(&file.absolute_path).map_err(|source| SampleError::Read {
        path: file.absolute_path.clone(),
        source,
    })?;
    // Re-check after the read: the file may have grown between stat and
    // read (TOCTOU). Prefer failing closed over indexing a huge buffer.
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(SampleError::TooLarge {
            path: file.absolute_path.clone(),
            size: bytes.len() as u64,
            limit: limits.max_file_bytes,
        });
    }

    let is_binary = file.kind == FileKind::Binary;
    let (content, utf8_lossy) = if is_binary {
        (None, false)
    } else {
        match std::str::from_utf8(&bytes) {
            Ok(text) => (Some(text.to_owned()), false),
            Err(_) => (Some(String::from_utf8_lossy(&bytes).into_owned()), true),
        }
    };
    Ok(Sample {
        relative_path: file.relative_path.clone(),
        is_binary,
        byte_len: bytes.len() as u64,
        identity: hash::hash_bytes(&file.relative_path, &bytes),
        content,
        utf8_lossy,
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
                path_id = %util::redacted_path(absolute_path),
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

fn utf8_lossy_evidence() -> EvidenceItem {
    EvidenceItem {
        key: "encoding:0".to_owned(),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::SourceContent,
        span: SpanAvailability::DerivedEvidence,
        payload: json!({
            "message": "file contains invalid UTF-8 byte sequences; replaced with U+FFFD during indexing",
            "severity": "warning",
        }),
    }
}

/// Runs pack analysis on a sampled file and folds the result into a
/// storable `FileRecord`, appending an encoding diagnostic when the
/// content required lossy UTF-8 conversion and capping evidence to
/// `limits.evidence`.
pub(super) fn to_file_record(sample: Sample, limits: &ResourceLimits) -> FileRecord {
    let mut parsed = analyze(&sample.relative_path, sample.content.as_deref());
    if sample.utf8_lossy {
        parsed.evidence.push(utf8_lossy_evidence());
    }
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

/// Reserved out of `max_payload_bytes_per_file` so the truncation marker
/// itself never pushes a file's evidence over the cap it is reporting on.
const TRUNCATION_MARKER_RESERVE_BYTES: usize = 512;

/// Enforces all three evidence caps in one pass: per-item count, per-item
/// payload bytes, and — the one the old truncate-then-retain version never
/// checked — cumulative payload bytes across every kept item. A file with
/// many small-but-not-tiny items (each under the per-item cap) could still
/// serialize to a multi-megabyte evidence blob; this is what actually
/// caught the 5,326,990-byte failure. Whenever anything is discarded, an
/// explicit `Diagnostic` evidence item records it — the AI sees a "this
/// file's evidence was truncated" signal instead of silently incomplete data.
///
/// Returns `true` when anything was discarded, so `to_file_record` can
/// downgrade a would-be `Full` run to `Partial` instead of storing a
/// completeness claim that contradicts the marker sitting next to it.
#[must_use]
pub(crate) fn apply_evidence_limits(
    items: &mut Vec<EvidenceItem>,
    limits: &ResourceLimits,
) -> bool {
    let cap = limits.evidence;
    let original_len = items.len();
    let max_items = cap.max_items_per_file.saturating_sub(1);
    let max_bytes = cap
        .max_payload_bytes_per_file
        .saturating_sub(TRUNCATION_MARKER_RESERVE_BYTES);

    let mut kept = Vec::with_capacity(original_len.min(cap.max_items_per_file));
    let mut cumulative_bytes = 0_usize;
    let mut oversized_dropped = 0_usize;
    let mut unmeasurable_dropped = 0_usize;
    let mut limit_truncated = false;

    for item in items.drain(..) {
        if kept.len() >= max_items {
            limit_truncated = true;
            break;
        }
        let payload_bytes = match serde_json::to_vec(&item.payload) {
            Ok(bytes) => bytes.len(),
            Err(error) => {
                // A `Value` serializes infallibly today; if that ever stops
                // being true the item is dropped under its own count, never
                // passed off as an over-the-cap item.
                tracing::warn!(
                    key = %item.key,
                    error = %error,
                    "evidence payload failed to serialize; item dropped",
                );
                unmeasurable_dropped += 1;
                continue;
            }
        };
        if payload_bytes > cap.max_payload_bytes_per_item {
            oversized_dropped += 1;
            continue;
        }
        if cumulative_bytes.saturating_add(payload_bytes) > max_bytes {
            limit_truncated = true;
            break;
        }
        cumulative_bytes += payload_bytes;
        kept.push(item);
    }

    let kept_len = kept.len();
    *items = kept;
    let discarded = limit_truncated || oversized_dropped > 0 || unmeasurable_dropped > 0;
    if discarded {
        items.push(truncation_evidence(
            original_len,
            kept_len,
            oversized_dropped,
            unmeasurable_dropped,
        ));
    }
    discarded
}

fn truncation_evidence(
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
