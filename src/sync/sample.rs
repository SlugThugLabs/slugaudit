use super::analyze::analyze;
use super::discovery::{DiscoveredFile, FileKind};
use super::hash;
use super::revision::FileRecord;
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, ResourceLimits, SourceIdentity, SpanAvailability,
};
use serde_json::json;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
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
pub struct Sample {
    pub relative_path: String,
    pub is_binary: bool,
    pub content: Option<String>,
    /// True when `content` required lossy UTF-8 conversion (invalid byte
    /// sequences replaced with U+FFFD). Recorded as evidence rather than
    /// silently accepted — see `to_file_record`.
    utf8_lossy: bool,
    pub identity: SourceIdentity,
    pub byte_len: u64,
}

/// # Errors
///
/// Returns an error if `file.absolute_path` can't be read, or if its size
/// exceeds `limits.max_file_bytes`.
pub fn sample_file(file: &DiscoveredFile, limits: &ResourceLimits) -> Result<Sample, SampleError> {
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
    })
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
pub fn to_file_record(sample: Sample, limits: &ResourceLimits) -> FileRecord {
    let mut parsed = analyze(&sample.relative_path, sample.content.as_deref());
    if sample.utf8_lossy {
        parsed.evidence.push(utf8_lossy_evidence());
    }
    apply_evidence_limits(&mut parsed.evidence, limits);
    FileRecord {
        relative_path: sample.relative_path,
        is_binary: sample.is_binary,
        content: sample.content,
        identity: sample.identity,
        byte_len: sample.byte_len,
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
pub(crate) fn apply_evidence_limits(items: &mut Vec<EvidenceItem>, limits: &ResourceLimits) {
    let cap = limits.evidence;
    let original_len = items.len();
    let max_items = cap.max_items_per_file.saturating_sub(1);
    let max_bytes = cap
        .max_payload_bytes_per_file
        .saturating_sub(TRUNCATION_MARKER_RESERVE_BYTES);

    let mut kept = Vec::with_capacity(original_len.min(cap.max_items_per_file));
    let mut cumulative_bytes = 0_usize;
    let mut oversized_dropped = 0_usize;
    let mut limit_truncated = false;

    for item in items.drain(..) {
        if kept.len() >= max_items {
            limit_truncated = true;
            break;
        }
        let payload_bytes =
            serde_json::to_vec(&item.payload).map_or(usize::MAX, |bytes| bytes.len());
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
    if limit_truncated || oversized_dropped > 0 {
        items.push(truncation_evidence(
            original_len,
            kept_len,
            oversized_dropped,
        ));
    }
}

fn truncation_evidence(
    original_count: usize,
    kept_count: usize,
    oversized_dropped: usize,
) -> EvidenceItem {
    let dropped_for_cap = original_count
        .saturating_sub(kept_count)
        .saturating_sub(oversized_dropped);
    EvidenceItem {
        key: "evidence:truncated".to_owned(),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::SourceContent,
        span: SpanAvailability::DerivedEvidence,
        payload: json!({
            "message": format!(
                "evidence limits discarded records: {oversized_dropped} item(s) exceeded the \
                 per-item byte cap, {dropped_for_cap} item(s) dropped by the item-count or \
                 cumulative-byte cap"
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
