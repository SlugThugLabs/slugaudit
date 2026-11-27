// slugaudit-line-exception: approved-by=split-judge; reason=per-file bounded byte sampling, skip record construction, and evidence item/byte cap enforcement
//! Per-file bounded sampling, skip classification, pack analysis, and
//! evidence-budget enforcement (`apply_evidence_limits`).

use super::analyze::analyze;
use super::discovery::{DiscoveredFile, is_binary_bytes, is_vanished};
use super::hash;
use super::publish::PublishError;
use super::revision::{FileRecord, SKIP_REASON_STAT_FAILED, SkippedRecord};
use crate::model::{
    DiagnosticPayload, EvidenceItem, EvidenceKind, EvidenceOrigin, EvidencePayload,
    ExtractionCompleteness, ResourceLimits, SourceIdentity, SpanAvailability,
};
use crate::util::{self, Deadline};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Reserved out of `max_payload_bytes_per_file` so the truncation marker
/// itself never pushes a file's evidence over the cap it reports on.
const TRUNCATION_MARKER_RESERVE_BYTES: usize = 512;

#[derive(Debug, Error)]
/// Per-file sampling failures. `TooLarge` and `NotUtf8` skip that file with
/// a reported reason; a read failure aborts the publish attempt.
pub enum SampleError {
    /// Reading the file failed.
    #[error("failed to read {path}: {source}")]
    Read {
        /// Absolute path of the unreadable file.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// File exceeded `max_file_bytes` (`observed` is a lower bound on its size).
    #[error("{path} exceeds the {limit}-byte per-file import ceiling (at least {observed} bytes)")]
    TooLarge {
        /// Absolute path of the oversized file.
        path: PathBuf,
        /// Observed size or bounded read length (`limit + 1`).
        observed: u64,
        /// Configured per-file ceiling.
        limit: u64,
    },
    /// File bytes are not valid UTF-8 and become a `not_utf8` skipped row.
    #[error("{path} is not valid UTF-8; recorded as a skipped file")]
    NotUtf8 {
        /// Absolute path of the non-UTF-8 file.
        path: PathBuf,
        /// BLAKE3 hex hash of the bytes read during sampling.
        content_hash: String,
        /// Where the bytes stop being valid UTF-8.
        #[source]
        source: std::str::Utf8Error,
    },
}

impl SampleError {
    fn read(path: &Path, source: std::io::Error) -> Self {
        Self::Read {
            path: path.to_path_buf(),
            source,
        }
    }

    fn too_large(path: &Path, observed: u64, limit: u64) -> Self {
        Self::TooLarge {
            path: path.to_path_buf(),
            observed,
            limit,
        }
    }
}

/// One discovered file's sampled on-disk state.
#[derive(Debug)]
pub(super) struct Sample {
    pub relative_path: String,
    pub is_binary: bool,
    pub content: Option<String>,
    pub identity: SourceIdentity,
    pub byte_len: u64,
    /// File mtime in unix seconds, captured for reporting only.
    pub modified_unix_seconds: Option<i64>,
}

/// Reads and UTF-8-validates `file` within `limits.max_file_bytes`.
pub(super) fn sample_file(
    file: &DiscoveredFile,
    limits: &ResourceLimits,
) -> Result<Sample, SampleError> {
    let metadata = std::fs::metadata(&file.absolute_path)
        .map_err(|source| SampleError::read(&file.absolute_path, source))?;
    let size = metadata.len();
    if size > limits.max_file_bytes {
        return Err(SampleError::too_large(
            &file.absolute_path,
            size,
            limits.max_file_bytes,
        ));
    }

    let bytes = util::read_file_capped_bytes(&file.absolute_path, limits.max_file_bytes).map_err(
        |error| match error {
            util::CappedReadError::Io(source) => SampleError::read(&file.absolute_path, source),
            util::CappedReadError::TooLarge { limit, observed } => {
                SampleError::too_large(&file.absolute_path, observed, limit)
            }
        },
    )?;

    let identity = hash::hash_bytes(&file.relative_path, &bytes);
    let is_binary = is_binary_bytes(&bytes);
    let content = if is_binary {
        None
    } else {
        match std::str::from_utf8(&bytes) {
            Ok(text) => Some(text.to_owned()),
            Err(source) => {
                return Err(SampleError::NotUtf8 {
                    path: file.absolute_path.clone(),
                    content_hash: identity.content_hash,
                    source,
                });
            }
        }
    };
    Ok(Sample {
        relative_path: file.relative_path.clone(),
        is_binary,
        byte_len: bytes.len() as u64,
        identity,
        content,
        modified_unix_seconds: util::mtime_unix_seconds(&metadata),
    })
}

/// Builds a stat-derived existence record for a skipped file without reading
/// its content.
pub(super) fn skipped_record(
    relative_path: &str,
    absolute_path: &Path,
    reason: &str,
) -> Result<SkippedRecord, PublishError> {
    let (byte_len, modified_unix_seconds, reason) = match std::fs::metadata(absolute_path) {
        Ok(metadata) => (
            Some(metadata.len()),
            util::mtime_unix_seconds(&metadata),
            reason,
        ),
        Err(error) if is_vanished(&error) => {
            return Err(PublishError::read_after_walk(
                relative_path,
                absolute_path.to_path_buf(),
                error,
            ));
        }
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
    Ok(SkippedRecord {
        relative_path: relative_path.to_owned(),
        reason: reason.to_owned(),
        identity: hash::stat_identity(relative_path, byte_len, modified_unix_seconds),
        byte_len,
        modified_unix_seconds,
    })
}

/// Runs pack analysis on a sampled file and folds the result into a storable
/// [`FileRecord`], capping evidence to `limits.evidence`.
pub(super) fn to_file_record(
    sample: Sample,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<FileRecord, crate::util::LockPoisoned> {
    let content = sample.content.as_deref();
    let mut parsed = analyze(
        &sample.relative_path,
        content,
        limits.max_parse_bytes,
        Duration::from_millis(limits.max_parse_wall_clock_ms),
        deadline,
    )?;
    let discarded = apply_evidence_limits(&mut parsed.evidence, limits);
    if discarded && parsed.run.completeness == ExtractionCompleteness::Full {
        parsed.run.completeness = ExtractionCompleteness::Partial;
    }
    Ok(FileRecord {
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
    })
}

/// Enforces item-count, per-item byte, and cumulative per-file byte caps on
/// `items`, appending an explicit `Diagnostic` truncation item if anything is
/// dropped.
#[must_use]
pub(crate) fn apply_evidence_limits(
    items: &mut Vec<EvidenceItem>,
    limits: &ResourceLimits,
) -> bool {
    EvidenceBudget::new(items.len(), limits).enforce(items)
}

struct EvidenceBudget {
    original_len: usize,
    max_items: usize,
    max_bytes: usize,
    max_payload_bytes_per_item: usize,
    kept: Vec<EvidenceItem>,
    cumulative_bytes: usize,
    oversized_dropped: usize,
    unmeasurable_dropped: usize,
    dropped_for_cap: usize,
}

enum Admission {
    Keep,
    Drop,
    Stop,
}

fn tally(count: &mut usize) {
    // KEEP saturating: a report tally has a designed ceiling; at usize::MAX it reads "at least this many" and never wraps to a small number
    *count = count.saturating_add(1);
}

impl EvidenceBudget {
    fn new(original_len: usize, limits: &ResourceLimits) -> Self {
        let cap = limits.evidence;
        Self {
            original_len,
            // KEEP saturating: one slot is held back for the truncation marker; a cap of zero leaves no slot for kept items
            max_items: cap.max_items_per_file.saturating_sub(1),
            // KEEP saturating: the marker's reserve comes off the byte cap; a cap below the reserve leaves zero bytes for kept items
            max_bytes: cap
                .max_payload_bytes_per_file
                .saturating_sub(TRUNCATION_MARKER_RESERVE_BYTES),
            max_payload_bytes_per_item: cap.max_payload_bytes_per_item,
            kept: Vec::with_capacity(original_len.min(cap.max_items_per_file)),
            cumulative_bytes: 0,
            oversized_dropped: 0,
            unmeasurable_dropped: 0,
            dropped_for_cap: 0,
        }
    }

    fn enforce(mut self, items: &mut Vec<EvidenceItem>) -> bool {
        {
            let mut source = items.drain(..);
            for item in source.by_ref() {
                if matches!(self.admit(item), Admission::Stop) {
                    self.dropped_for_cap = std::iter::once(()).chain(source.map(|_| ())).count();
                    break;
                }
            }
        }
        let discarded =
            self.dropped_for_cap > 0 || self.oversized_dropped > 0 || self.unmeasurable_dropped > 0;
        let kept_len = self.kept.len();
        *items = self.kept;
        if discarded {
            items.push(truncation_evidence(
                self.original_len,
                kept_len,
                self.oversized_dropped,
                self.unmeasurable_dropped,
                self.dropped_for_cap,
            ));
        }
        discarded
    }

    fn admit(&mut self, item: EvidenceItem) -> Admission {
        if self.kept.len() >= self.max_items {
            return Admission::Stop;
        }
        let payload_bytes = match util::serialized_len(&item.payload) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(
                    key = %item.key,
                    error = %error,
                    "evidence payload failed to serialize; item dropped",
                );
                tally(&mut self.unmeasurable_dropped);
                return Admission::Drop;
            }
        };
        if payload_bytes > self.max_payload_bytes_per_item {
            tally(&mut self.oversized_dropped);
            return Admission::Drop;
        }
        let Some(cumulative) = self
            .cumulative_bytes
            .checked_add(payload_bytes)
            .filter(|total| *total <= self.max_bytes)
        else {
            return Admission::Stop;
        };
        self.cumulative_bytes = cumulative;
        self.kept.push(item);
        Admission::Keep
    }
}

pub(super) fn truncation_evidence(
    original_count: usize,
    kept_count: usize,
    oversized_dropped: usize,
    unmeasurable_dropped: usize,
    dropped_for_cap: usize,
) -> EvidenceItem {
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
        payload: EvidencePayload::Diagnostic(DiagnosticPayload {
            message: format!(
                "evidence limits discarded records: {oversized_dropped} item(s) exceeded the \
                 per-item byte cap, {dropped_for_cap} item(s) dropped by the item-count or \
                 cumulative-byte cap{unmeasurable_note}"
            ),
            severity: "warning".to_owned(),
            original_item_count: Some(original_count),
            kept_item_count: Some(kept_count),
        }),
    }
}

#[cfg(test)]
#[path = "sample_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "evidence_limits_tests.rs"]
mod evidence_limits_tests;
