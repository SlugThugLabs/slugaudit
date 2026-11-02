//! The caps that bound the evidence stored with one file: item count, bytes
//! per item and cumulative bytes, applied in one pass before the file's record
//! is written, with an explicit truncation marker whenever anything is dropped.

use super::sample::truncation_evidence;
use crate::model::{EvidenceItem, ResourceLimits};

/// Reserved out of `max_payload_bytes_per_file` so the truncation marker
/// itself never pushes a file's evidence over the cap it is reporting on.
const TRUNCATION_MARKER_RESERVE_BYTES: usize = 512;

/// Enforces all three evidence caps in one pass: per-item count, per-item
/// payload bytes, and cumulative payload bytes across every kept item. A file
/// with many small-but-not-tiny items (each under the per-item cap) could
/// still serialize to a multi-megabyte evidence blob, so the cumulative cap
/// is checked too. Whenever anything is discarded, an
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
    let original_len = items.len();
    let mut budget = EvidenceBudget::new(original_len, limits);
    for item in items.drain(..) {
        if matches!(budget.admit(item), Admission::Stop) {
            break;
        }
    }
    let kept_len = budget.kept.len();
    let discarded = budget.anything_discarded();
    *items = budget.kept;
    if discarded {
        items.push(truncation_evidence(
            original_len,
            kept_len,
            budget.oversized_dropped,
            budget.unmeasurable_dropped,
        ));
    }
    discarded
}

/// One pass of the three evidence caps; `apply_evidence_limits` drains the
/// source list into `kept`, stopping as soon as a cap is reached.
struct EvidenceBudget {
    max_items: usize,
    max_bytes: usize,
    max_payload_bytes_per_item: usize,
    kept: Vec<EvidenceItem>,
    cumulative_bytes: usize,
    oversized_dropped: usize,
    unmeasurable_dropped: usize,
    limit_truncated: bool,
}

/// What one item's cap check decided: keep it, drop it under its own
/// counter, or stop the pass because an item-count or cumulative-byte cap
/// was reached.
enum Admission {
    Keep,
    Drop,
    Stop,
}

impl EvidenceBudget {
    fn new(original_len: usize, limits: &ResourceLimits) -> Self {
        let cap = limits.evidence;
        Self {
            max_items: cap.max_items_per_file.saturating_sub(1),
            max_bytes: cap
                .max_payload_bytes_per_file
                .saturating_sub(TRUNCATION_MARKER_RESERVE_BYTES),
            max_payload_bytes_per_item: cap.max_payload_bytes_per_item,
            kept: Vec::with_capacity(original_len.min(cap.max_items_per_file)),
            cumulative_bytes: 0,
            oversized_dropped: 0,
            unmeasurable_dropped: 0,
            limit_truncated: false,
        }
    }

    fn admit(&mut self, item: EvidenceItem) -> Admission {
        if self.kept.len() >= self.max_items {
            self.limit_truncated = true;
            return Admission::Stop;
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
                self.unmeasurable_dropped += 1;
                return Admission::Drop;
            }
        };
        if payload_bytes > self.max_payload_bytes_per_item {
            self.oversized_dropped += 1;
            return Admission::Drop;
        }
        if self.cumulative_bytes.saturating_add(payload_bytes) > self.max_bytes {
            self.limit_truncated = true;
            return Admission::Stop;
        }
        self.cumulative_bytes += payload_bytes;
        self.kept.push(item);
        Admission::Keep
    }

    fn anything_discarded(&self) -> bool {
        self.limit_truncated || self.oversized_dropped > 0 || self.unmeasurable_dropped > 0
    }
}
